//! Built-in TLS end to end: the hub gets its certificate from an in-process
//! ACME server (`fake_acme`) over its own listeners – TLS-ALPN-01 and
//! HTTP-01 –, serves HTTPS that a rustls client trusting the fake CA
//! accepts, renews and swaps certificates without dropping connections,
//! survives `badNonce`, rate limits and error documents, and serves a
//! certificate from files.

mod fake_acme;

use std::{net::SocketAddr, path::Path, sync::Arc, time::Duration};

use fake_acme::{FakeAcme, PATIENCE};
use jiff::{SignedDuration, Timestamp};
use roedy_hub_server::{
    app::{AppState, Listeners, run_http},
    config::{Config, TlsMode},
    crypto::SecretKey,
    db::{self, Db},
    healthcheck,
    tls::{
        self,
        issuer::{self, Issuer, Outcome},
        key::P256Key,
        x509::{self, CertInfo, CertificateParams, Extension},
    },
    web::static_files::WebAssets,
};
use rustls::{
    ClientConfig, RootCertStore,
    crypto::ring::default_provider,
    pki_types::{CertificateDer, ServerName},
};
use tokio::{
    io::{AsyncBufReadExt as _, AsyncReadExt as _, AsyncWriteExt as _, BufReader},
    net::{TcpListener, TcpStream},
    task::JoinHandle,
};
use tokio_rustls::{TlsConnector, client::TlsStream};
use url::Url;

const HOST: &str = "hub.example.test";

struct Hub {
    state: AppState,
    https: SocketAddr,
    http: SocketAddr,
    stop: Option<tokio::sync::oneshot::Sender<()>>,
    server: JoinHandle<anyhow::Result<()>>,
    certificates: Option<JoinHandle<()>>,
}

impl Hub {
    fn tls(&self) -> &tls::Tls {
        self.state.tls.as_ref().unwrap()
    }

    fn issuer(&self) -> Arc<Issuer> {
        self.tls().issuer.clone().unwrap()
    }

    async fn wait_for_certificate(&self) {
        let deadline = tokio::time::Instant::now() + PATIENCE;
        while self.tls().resolver.current().is_none() {
            assert!(
                tokio::time::Instant::now() < deadline,
                "no certificate in time"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    async fn stop(mut self) {
        if let Some(task) = self.certificates.take() {
            task.abort();
        }
        let _ = self.stop.take().unwrap().send(());
        tokio::time::timeout(Duration::from_secs(10), self.server)
            .await
            .expect("stops")
            .unwrap()
            .unwrap();
    }

    async fn audit(&self) -> Vec<(String, String)> {
        db::call(&self.state.db, |conn| {
            let mut stmt =
                conn.prepare("SELECT action, COALESCE(details, '') FROM audit_log ORDER BY id")?;
            let rows = stmt
                .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        })
        .await
        .unwrap()
    }
}

/// The stored retry state, if any.
async fn retry_state(hub: &Hub) -> Option<serde_json::Value> {
    db::call(&hub.state.db, |conn| {
        roedy_hub_server::db::repo::app_settings::get(conn, "tls_retry")
    })
    .await
    .unwrap()
}

fn config(fake: &FakeAcme) -> Config {
    Config {
        tls: TlsMode::Acme,
        public_url: Some(Url::parse(&format!("https://{HOST}")).unwrap()),
        acme_directory: Url::parse(&fake.directory_url()).unwrap(),
        acme_email: Some("ops@example.com".into()),
        acme_agree_tos: true,
        ..Config::default()
    }
}

/// A hub with `ROEDY_TLS=acme` against `fake`, its data in `data`.
async fn start(fake: &FakeAcme, data: &Path, secrets: SecretKey) -> Hub {
    let config = config(fake);
    let db = Db::from(db::open(data).unwrap());
    let mut state = AppState::new(
        db.clone(),
        Arc::new(WebAssets::None),
        &config,
        config.public_url.clone().unwrap(),
        secrets,
    )
    .unwrap();
    let mut tls = tls::prepare(&config, &db, state.secrets.clone())
        .await
        .unwrap()
        .unwrap();
    // Short waits after failures.
    let settings = tls.issuer.take().unwrap().settings().clone();
    tls.issuer = Some(Arc::new(
        Issuer::new(
            settings,
            issuer::http_client().unwrap(),
            db,
            state.secrets.clone(),
            tls.resolver.clone(),
            tls.challenges.clone(),
        )
        .with_backoff(vec![Duration::from_millis(50)]),
    ));
    let public = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let http = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let (https_addr, http_addr) = (public.local_addr().unwrap(), http.local_addr().unwrap());
    fake.targets(Some(http_addr), Some(https_addr));
    state.tls = Some(Arc::new(tls));
    let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
    let server = tokio::spawn(run_http(
        Listeners {
            public,
            internal: None,
            http: Some(http),
        },
        state.clone(),
        async {
            let _ = stopped.await;
        },
    ));
    let certificates = state.tls.as_ref().unwrap().start();
    Hub {
        state,
        https: https_addr,
        http: http_addr,
        stop: Some(stop),
        server,
        certificates,
    }
}

/// A TLS connection trusting only `ca`.
async fn connect(addr: SocketAddr, name: &str, ca: &[u8]) -> std::io::Result<TlsStream<TcpStream>> {
    let mut roots = RootCertStore::empty();
    roots.add(CertificateDer::from(ca.to_vec())).unwrap();
    let mut config = ClientConfig::builder_with_provider(Arc::new(default_provider()))
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_root_certificates(roots)
        .with_no_client_auth();
    config.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
    // A hub that never finishes the handshake fails the test instead of
    // hanging it.
    tokio::time::timeout(PATIENCE, async {
        let tcp = TcpStream::connect(addr).await?;
        TlsConnector::from(Arc::new(config))
            .connect(ServerName::try_from(name.to_owned()).unwrap(), tcp)
            .await
    })
    .await
    .unwrap_or_else(|_| panic!("TLS connection to {addr} ({name}): no handshake in time"))
}

/// One request on a kept-alive connection; the status line and headers.
async fn get<S>(stream: &mut S, path: &str) -> String
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    tokio::time::timeout(PATIENCE, exchange(stream, path))
        .await
        .unwrap_or_else(|_| panic!("GET {path}: no complete response in time"))
}

async fn exchange<S>(stream: &mut S, path: &str) -> String
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    stream
        .write_all(format!("GET {path} HTTP/1.1\r\nHost: {HOST}\r\n\r\n").as_bytes())
        .await
        .unwrap();
    let mut reader = BufReader::new(stream);
    let mut head = String::new();
    let mut length = 0;
    loop {
        let mut line = String::new();
        reader.read_line(&mut line).await.unwrap();
        if line == "\r\n" || line.is_empty() {
            break;
        }
        if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
            length = value.trim().parse().unwrap();
        }
        head.push_str(&line);
    }
    let mut body = vec![0; length];
    reader.read_exact(&mut body).await.unwrap();
    head
}

fn serial(stream: &TlsStream<TcpStream>) -> Vec<u8> {
    let cert = &stream.get_ref().1.peer_certificates().unwrap()[0];
    CertInfo::parse(cert).unwrap().serial
}

#[tokio::test]
async fn issues_with_tls_alpn_01_and_serves_https() {
    let fake = FakeAcme::start().await;
    fake.knobs(|k| {
        k.bad_nonces = 2;
        k.processing_polls = 1;
    });
    let tmp = tempfile::tempdir().unwrap();
    let secrets = SecretKey::random().unwrap();
    let hub = start(&fake, tmp.path(), secrets.clone()).await;
    hub.wait_for_certificate().await;

    let log = fake.log();
    // TLS-ALPN-01 first, although the CA lists HTTP-01 first.
    assert_eq!(
        log.validations,
        [("tls-alpn-01".to_owned(), true)],
        "{:?}",
        hub.audit().await
    );
    assert_eq!(log.bad_nonces_sent, 2, "retried with fresh nonces");
    assert_eq!(log.accounts.len(), 1);
    assert_eq!(log.contacts[0], ["mailto:ops@example.com"]);
    assert_eq!(log.orders, [None]);
    assert_eq!(log.issued, 1);

    // HTTPS with a certificate the fake CA's root verifies, HSTS on.
    let mut stream = connect(hub.https, HOST, &fake.ca_cert).await.unwrap();
    assert_eq!(stream.get_ref().1.alpn_protocol(), Some(&b"http/1.1"[..]));
    let head = get(&mut stream, "/healthz").await;
    assert!(head.starts_with("HTTP/1.1 200 OK"), "{head}");
    assert!(
        head.contains("strict-transport-security: max-age=31536000\r\n"),
        "{head}"
    );
    // Only under its own name.
    assert!(
        connect(hub.https, "other.example.test", &fake.ca_cert)
            .await
            .is_err()
    );
    // `roedy-hub healthcheck` over TLS.
    healthcheck::check_tls(hub.https, HOST, healthcheck::DEFAULT_TIMEOUT)
        .await
        .unwrap();

    // Plain HTTP only redirects.
    let mut plain = TcpStream::connect(hub.http).await.unwrap();
    let head = get(&mut plain, "/login?next=%2Fmail").await;
    assert!(head.starts_with("HTTP/1.1 308"), "{head}");
    assert!(
        head.contains(&format!("location: https://{HOST}/login?next=%2Fmail\r\n")),
        "{head}"
    );

    let audit = hub.audit().await;
    assert_eq!(audit.len(), 1, "{audit:?}");
    assert_eq!(audit[0].0, "tls.issue");
    assert!(audit[0].1.contains(HOST));
    hub.stop().await;

    // A restart serves the stored certificate at once, without the CA
    // validating anything; the daily check finds it valid (ARI window
    // later).
    let hub = start(&fake, tmp.path(), secrets).await;
    fake.targets(None, None);
    let mut stream = connect(hub.https, HOST, &fake.ca_cert).await.unwrap();
    assert!(
        get(&mut stream, "/healthz")
            .await
            .starts_with("HTTP/1.1 200")
    );
    assert!(matches!(
        hub.issuer().run_once().await.unwrap(),
        Outcome::Valid { .. }
    ));
    let log = fake.log();
    assert_eq!((log.issued, log.accounts.len()), (1, 1));
    assert!(log.ari_requests >= 1);
    hub.stop().await;
}

#[tokio::test]
async fn falls_back_to_http_01_after_a_failed_validation() {
    let fake = FakeAcme::start().await;
    fake.knobs(|k| k.fail = vec!["tls-alpn-01"]);
    let tmp = tempfile::tempdir().unwrap();
    let hub = start(&fake, tmp.path(), SecretKey::random().unwrap()).await;
    hub.wait_for_certificate().await;
    assert_eq!(
        fake.log().validations,
        [
            ("tls-alpn-01".to_owned(), false),
            ("http-01".to_owned(), true)
        ]
    );
    let actions: Vec<String> = hub.audit().await.into_iter().map(|(a, _)| a).collect();
    assert_eq!(actions, ["tls.fail", "tls.issue"]);
    let fail = &hub.audit().await[0].1;
    assert!(
        fail.contains("tls-alpn-01 validation of hub.example.test failed"),
        "{fail}"
    );
    let mut stream = connect(hub.https, HOST, &fake.ca_cert).await.unwrap();
    assert!(
        get(&mut stream, "/healthz")
            .await
            .starts_with("HTTP/1.1 200")
    );
    // The retry state is cleared once the certificate is in place.
    let deadline = tokio::time::Instant::now() + PATIENCE;
    while retry_state(&hub).await.is_some() {
        assert!(
            tokio::time::Instant::now() < deadline,
            "retry state not cleared"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    hub.stop().await;
}

#[tokio::test]
async fn waits_as_the_ca_says_and_records_one_failure() {
    let fake = FakeAcme::start().await;
    fake.knobs(|k| {
        k.rate_limited = 1;
        k.retry_after = 2;
        k.finalize_error = Some("badCSR");
        // The next attempt waits until the checks without a certificate
        // are done, however slow the runner.
        k.hold_orders = true;
    });
    let tmp = tempfile::tempdir().unwrap();
    let started = tokio::time::Instant::now();
    let hub = start(&fake, tmp.path(), SecretKey::random().unwrap()).await;

    // No certificate yet: no TLS handshake, the healthcheck fails.
    // Wait for the rate limit to be recorded.
    let deadline = tokio::time::Instant::now() + PATIENCE;
    let retry = loop {
        if let Some(retry) = retry_state(&hub).await {
            break retry;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "no failure recorded"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    };
    // Stored, so that a restart waits as well.
    assert_eq!(retry["failures"], 1);
    assert!(hub.tls().resolver.current().is_none());
    assert!(connect(hub.https, HOST, &fake.ca_cert).await.is_err());
    let err = healthcheck::check_tls(hub.https, HOST, healthcheck::DEFAULT_TIMEOUT)
        .await
        .unwrap_err();
    assert!(err.to_string().contains("no TLS connection"), "{err}");
    fake.knobs(|k| k.hold_orders = false);

    hub.wait_for_certificate().await;
    assert!(
        started.elapsed() >= Duration::from_secs(2),
        "Retry-After waited: {:?}",
        started.elapsed()
    );
    let log = fake.log();
    assert_eq!(log.rate_limits_sent, 1);
    // The next order came no earlier than `Retry-After` allowed.
    let gap = log.order_at[0] - log.rate_limited_at[0];
    assert!(gap >= Duration::from_secs(2), "next order after {gap:?}");
    // The rate-limited attempt, the refused finalize, the success.
    assert_eq!(log.orders.len(), 2);
    let audit = hub.audit().await;
    let actions: Vec<&str> = audit.iter().map(|(a, _)| a.as_str()).collect();
    assert_eq!(actions, ["tls.fail", "tls.issue"], "one entry per series");
    assert!(audit[0].1.contains("rateLimited"), "{}", audit[0].1);
    hub.stop().await;
}

#[tokio::test]
async fn renews_and_swaps_without_dropping_connections() {
    let fake = FakeAcme::start().await;
    // First certificate: 80 of 90 days over, the CA without ARI.
    fake.knobs(|k| {
        k.ari = None;
        k.backdate = SignedDuration::from_hours(80 * 24);
    });
    let tmp = tempfile::tempdir().unwrap();
    let hub = start(&fake, tmp.path(), SecretKey::random().unwrap()).await;
    hub.wait_for_certificate().await;
    let mut kept = connect(hub.https, HOST, &fake.ca_cert).await.unwrap();
    let first = serial(&kept);
    assert!(get(&mut kept, "/healthz").await.starts_with("HTTP/1.1 200"));

    // Due by the 30-day rule: renewed, without `replaces` (no ARI).
    fake.knobs(|k| k.backdate = SignedDuration::from_hours(1));
    let outcome = hub.issuer().run_once().await.unwrap();
    assert!(
        matches!(outcome, Outcome::Issued { renewed: true, .. }),
        "{outcome:?}"
    );
    let second_conn = connect(hub.https, HOST, &fake.ca_cert).await.unwrap();
    let second = serial(&second_conn);
    assert_ne!(first, second);
    // The open connection keeps working.
    assert!(get(&mut kept, "/healthz").await.starts_with("HTTP/1.1 200"));
    assert!(matches!(
        hub.issuer().run_once().await.unwrap(),
        Outcome::Valid { .. }
    ));

    // The CA suggests renewing now (ARI): renewed with `replaces`.
    fake.knobs(|k| {
        k.ari = Some((
            SignedDuration::from_hours(-1),
            SignedDuration::from_hours(1),
        ));
    });
    let outcome = hub.issuer().run_once().await.unwrap();
    assert!(
        matches!(outcome, Outcome::Issued { renewed: true, .. }),
        "{outcome:?}"
    );
    let third = serial(&connect(hub.https, HOST, &fake.ca_cert).await.unwrap());
    assert_ne!(third, second);
    let row = db::call(&hub.state.db, |conn| {
        Ok(roedy_hub_server::db::repo::tls::certificate(conn, HOST)?)
    })
    .await
    .unwrap()
    .unwrap();
    assert_eq!(serial_of_pem(&row.chain_pem), third);
    let log = fake.log();
    assert_eq!(log.orders.len(), 3);
    assert_eq!(log.orders[1], None);
    // `replaces` names the second certificate as the CA knows it.
    assert_eq!(log.orders[2].as_ref(), Some(&log.issued_ids[1]));
    let second_info =
        CertInfo::parse(&second_conn.get_ref().1.peer_certificates().unwrap()[0]).unwrap();
    assert_eq!(second_info.ari_id().as_ref(), Some(&log.issued_ids[1]));
    assert!(get(&mut kept, "/healthz").await.starts_with("HTTP/1.1 200"));

    let actions: Vec<String> = hub.audit().await.into_iter().map(|(a, _)| a).collect();
    assert_eq!(actions, ["tls.issue", "tls.renew", "tls.renew"]);

    // The CA refuses `replaces` (replaced already): ordered again without.
    fake.knobs(|k| k.refuse_replaces = true);
    let outcome = hub.issuer().run_once().await.unwrap();
    assert!(
        matches!(outcome, Outcome::Issued { renewed: true, .. }),
        "{outcome:?}"
    );
    assert_eq!(fake.log().orders.len(), 4);
    assert_eq!(fake.log().orders[3], None);
    hub.stop().await;
}

#[tokio::test]
async fn a_failed_daily_renewal_is_retried_in_the_background() {
    let fake = FakeAcme::start().await;
    let tmp = tempfile::tempdir().unwrap();
    let hub = start(&fake, tmp.path(), SecretKey::random().unwrap()).await;
    hub.wait_for_certificate().await;
    // The daily job's run fails (rate limit for a second).
    fake.knobs(|k| {
        k.ari = Some((
            SignedDuration::from_hours(-1),
            SignedDuration::from_hours(1),
        ));
        k.rate_limited = 1;
        k.retry_after = 1;
    });
    assert!(hub.issuer().run_once().await.is_err());
    // Nobody calls it again: the background task retries once the wait is
    // over, not a day later.
    let deadline = tokio::time::Instant::now() + PATIENCE;
    while fake.log().issued < 2 {
        assert!(
            tokio::time::Instant::now() < deadline,
            "failed renewal not retried"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let log = fake.log();
    assert!(log.order_at[1] - log.rate_limited_at[0] >= Duration::from_secs(1));
    let deadline = tokio::time::Instant::now() + PATIENCE;
    while hub.audit().await.last().map(|(a, _)| a.as_str()) != Some("tls.renew") {
        assert!(tokio::time::Instant::now() < deadline, "no tls.renew");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    hub.stop().await;
}

#[tokio::test]
async fn another_ca_does_not_wait_for_the_previous_cas_backoff() {
    // A certificate from CA A, then a failed renewal there that would wait
    // an hour.
    let a = FakeAcme::start().await;
    let tmp = tempfile::tempdir().unwrap();
    let secrets = SecretKey::random().unwrap();
    let hub = start(&a, tmp.path(), secrets.clone()).await;
    hub.wait_for_certificate().await;
    a.knobs(|k| {
        k.ari = Some((
            SignedDuration::from_hours(-1),
            SignedDuration::from_hours(1),
        ));
        k.rate_limited = 1;
        k.retry_after = 3600;
    });
    // A rate limit on the order with `replaces` is no reason to order
    // again without it.
    let outcome = hub.issuer().run_once().await;
    assert!(outcome.is_err(), "{outcome:?}");
    assert_eq!(a.log().orders.len(), 1, "no second order");
    assert_eq!(retry_state(&hub).await.unwrap()["failures"], 1);
    hub.stop().await;

    // Switched to CA B: it issues at once, not after A's hour.
    let b = FakeAcme::start().await;
    let hub = start(&b, tmp.path(), secrets).await;
    // Until then the previous CA's certificate is served.
    let deadline = tokio::time::Instant::now() + PATIENCE;
    while b.log().issued == 0 {
        assert!(
            tokio::time::Instant::now() < deadline,
            "CA B issued nothing"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let deadline = tokio::time::Instant::now() + PATIENCE;
    while connect(hub.https, HOST, &b.ca_cert).await.is_err() {
        assert!(
            tokio::time::Instant::now() < deadline,
            "CA B's certificate not served"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(connect(hub.https, HOST, &a.ca_cert).await.is_err());
    let actions: Vec<String> = hub.audit().await.into_iter().map(|(a, _)| a).collect();
    assert_eq!(actions, ["tls.issue", "tls.fail", "tls.issue"]);
    hub.stop().await;
}

fn serial_of_pem(pem: &str) -> Vec<u8> {
    let chain = issuer::parse_chain(pem).unwrap();
    CertInfo::parse(&chain[0]).unwrap().serial
}

#[tokio::test]
async fn another_instance_key_gets_a_new_account_and_certificate() {
    let fake = FakeAcme::start().await;
    let tmp = tempfile::tempdir().unwrap();
    let hub = start(&fake, tmp.path(), SecretKey::random().unwrap()).await;
    hub.wait_for_certificate().await;
    hub.stop().await;

    let hub = start(&fake, tmp.path(), SecretKey::random().unwrap()).await;
    hub.wait_for_certificate().await;
    let log = fake.log();
    assert_eq!(log.accounts.len(), 2, "a new account for the new key");
    assert_eq!(log.issued, 2);
    let actions: Vec<String> = hub.audit().await.into_iter().map(|(a, _)| a).collect();
    assert_eq!(actions, ["tls.issue", "tls.issue"]);
    hub.stop().await;
}

/// A CA and a certificate for `HOST` from it, written as PEM files.
fn write_files(
    dir: &Path,
    ca_key: &P256Key,
    serial: u8,
) -> (std::path::PathBuf, std::path::PathBuf) {
    let key = P256Key::generate().unwrap();
    let now = Timestamp::now();
    let leaf = CertificateParams {
        serial: vec![serial],
        issuer: x509::name(Some("Files CA")),
        subject: x509::name(None),
        not_before: now - SignedDuration::from_hours(1),
        not_after: now + SignedDuration::from_hours(24 * 30),
        spki: key.spki(),
        extensions: vec![Extension::dns_names(&[HOST]), Extension::server_auth()],
    }
    .sign(ca_key)
    .unwrap();
    let (cert, private) = (dir.join("cert.pem"), dir.join("key.pem"));
    std::fs::write(&cert, fake_acme::pem("CERTIFICATE", &leaf)).unwrap();
    std::fs::write(&private, fake_acme::pem("PRIVATE KEY", key.pkcs8())).unwrap();
    (cert, private)
}

#[tokio::test]
async fn serves_a_certificate_from_files_and_reloads_it() {
    let ca_key = P256Key::generate().unwrap();
    let now = Timestamp::now();
    let ca_name = x509::name(Some("Files CA"));
    let ca = CertificateParams {
        serial: vec![1],
        issuer: ca_name.clone(),
        subject: ca_name,
        not_before: now - SignedDuration::from_hours(1),
        not_after: now + SignedDuration::from_hours(24 * 365),
        spki: ca_key.spki(),
        extensions: vec![Extension::ca()],
    }
    .sign(&ca_key)
    .unwrap();
    let files = tempfile::tempdir().unwrap();
    let (cert, key) = write_files(files.path(), &ca_key, 7);
    let config = Config {
        tls: TlsMode::Files,
        public_url: Some(Url::parse(&format!("https://{HOST}")).unwrap()),
        tls_cert_file: Some(cert.clone()),
        tls_key_file: Some(key.clone()),
        ..Config::default()
    };
    let data = tempfile::tempdir().unwrap();
    let db = Db::from(db::open(data.path()).unwrap());
    let secrets = Arc::new(SecretKey::random().unwrap());
    let tls = tls::prepare(&config, &db, secrets).await.unwrap().unwrap();
    assert!(tls.issuer.is_none());
    let mut state = AppState::new(
        db,
        Arc::new(WebAssets::None),
        &config,
        config.public_url.clone().unwrap(),
        SecretKey::random().unwrap(),
    )
    .unwrap();
    let watch = tls
        .files
        .clone()
        .unwrap()
        .watch(tls.resolver.clone(), Duration::from_millis(20));
    state.tls = Some(Arc::new(tls));
    let public = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let http = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let (addr, http_addr) = (public.local_addr().unwrap(), http.local_addr().unwrap());
    let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
    let server = tokio::spawn(run_http(
        Listeners {
            public,
            internal: None,
            http: Some(http),
        },
        state,
        async {
            let _ = stopped.await;
        },
    ));

    let mut stream = connect(addr, HOST, &ca).await.unwrap();
    assert_eq!(serial(&stream), [7]);
    assert!(
        get(&mut stream, "/readyz")
            .await
            .starts_with("HTTP/1.1 200")
    );
    let mut plain = TcpStream::connect(http_addr).await.unwrap();
    assert!(get(&mut plain, "/").await.starts_with("HTTP/1.1 308"));

    // New files: served without a restart, the open connection stays.
    write_files(files.path(), &ca_key, 8);
    let deadline = tokio::time::Instant::now() + PATIENCE;
    while serial(&connect(addr, HOST, &ca).await.unwrap()) != [8] {
        assert!(tokio::time::Instant::now() < deadline, "not reloaded");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(
        get(&mut stream, "/healthz")
            .await
            .starts_with("HTTP/1.1 200")
    );

    watch.abort();
    stop.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(10), server)
        .await
        .unwrap()
        .unwrap()
        .unwrap();

    // At start, a pair for another name is refused with a clear message.
    let other = tempfile::tempdir().unwrap();
    let config = Config {
        public_url: Some(Url::parse("https://other.example.test").unwrap()),
        ..config
    };
    let data = tempfile::tempdir().unwrap();
    let db = Db::from(db::open(data.path()).unwrap());
    let err = tls::prepare(&config, &db, Arc::new(SecretKey::random().unwrap()))
        .await
        .err()
        .unwrap()
        .to_string();
    assert!(
        err.starts_with("TLS certificate not usable (ROEDY_TLS=files):")
            && err.contains("is not for other.example.test"),
        "{err}"
    );
    drop(other);
}
