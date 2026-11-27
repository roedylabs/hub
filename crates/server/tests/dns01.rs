//! The DNS-01 challenge with deSEC end to end, without network: the hub's
//! issuer against the fake ACME server (`fake_acme`), an in-process deSEC
//! API (`fake_desec`) and two in-process authoritative name servers
//! (`fake_dns`). Covers the zone lookup, creating, updating and deleting
//! the TXT record set, the cleanup after a failure, deSEC's throttling,
//! waiting for the name servers (and giving up), renewal, and that the
//! token shows up nowhere but in the `Authorization` header.

mod fake_acme;
mod fake_desec;
mod fake_dns;

use std::{
    collections::HashMap,
    sync::{Arc, Mutex, OnceLock},
    time::{Duration, Instant},
};

use fake_acme::FakeAcme;
use fake_desec::FakeDesec;
use fake_dns::{FakeDns, Transport};
use jiff::{SignedDuration, Timestamp};
use roedy_hub_server::{
    config::{AcmeChallenge, Config, DnsProvider, TlsMode},
    crypto::SecretKey,
    db::{self, Db},
    tls::{
        self, CertResolver,
        desec::{quote, txt_body},
        dns, dns01,
        issuer::{self, Issuer, Outcome},
    },
};
use serde_json::Value;
use url::Url;

const HOST: &str = "hub.example.test";
const ZONE: &str = "example.test";
const SUBNAME: &str = "_acme-challenge.hub";
const NAME: &str = "_acme-challenge.hub.example.test";
const TOKEN: &str = "FakeDesecT0kenForTests1234";
const MINIMUM_TTL: u32 = 3600;

struct Setup {
    acme: FakeAcme,
    desec: FakeDesec,
    ns1: FakeDns,
    ns2: FakeDns,
    issuer: Issuer,
    resolver: Arc<CertResolver>,
    db: Db,
    /// When the CA looked the TXT record up.
    ca_lookups: Arc<Mutex<Vec<Instant>>>,
    _dir: tempfile::TempDir,
}

/// The issuer as `tls::prepare` makes it from the configuration (the
/// token read from a file), pointed at the fakes, with short waits.
async fn setup(change: impl FnOnce(&mut dns01::Settings, &FakeDesec)) -> Setup {
    let acme = FakeAcme::start().await;
    acme.knobs(|knobs| knobs.offer = vec!["http-01", "dns-01", "tls-alpn-01"]);
    let (ns1, ns2) = (FakeDns::start().await, FakeDns::start().await);
    let desec = FakeDesec::start(
        TOKEN,
        ZONE,
        MINIMUM_TTL,
        &["ns1.desec.test.", "ns2.desec.test."],
        vec![ns1.clone(), ns2.clone()],
    )
    .await;
    // The CA asks the first name server.
    let ca_lookups = Arc::new(Mutex::new(Vec::new()));
    let (lookups, server) = (ca_lookups.clone(), ns1.clone());
    acme.dns(move |name| {
        lookups.lock().unwrap().push(Instant::now());
        server.txt(name)
    });

    let dir = tempfile::tempdir().unwrap();
    let token_file = dir.path().join("desec_token");
    std::fs::write(&token_file, format!("{TOKEN}\n")).unwrap();
    let config = Config {
        tls: TlsMode::Acme,
        public_url: Some(Url::parse(&format!("https://{HOST}")).unwrap()),
        acme_directory: Url::parse(&acme.directory_url()).unwrap(),
        acme_agree_tos: true,
        acme_challenge: AcmeChallenge::Dns01,
        acme_dns_provider: Some(DnsProvider::Desec),
        desec_token_file: Some(token_file),
        ..Config::default()
    };
    let db = Db::from(db::open(dir.path()).unwrap());
    let secrets = Arc::new(SecretKey::random().unwrap());
    let mut tls = tls::prepare(&config, &db, secrets.clone())
        .await
        .unwrap()
        .unwrap();
    let mut settings = tls.issuer.take().unwrap().settings().clone();
    let dns = settings.dns01.as_mut().expect("DNS-01 configured");
    let dns01::Provider::Desec { api, .. } = &mut dns.provider;
    *api = desec.api();
    dns.poll = Duration::from_millis(50);
    dns.propagation_timeout = Duration::from_secs(10);
    dns.query_timeout = Duration::from_secs(1);
    dns.name_servers = HashMap::from([
        ("ns1.desec.test".to_owned(), vec![ns1.addr]),
        ("ns2.desec.test".to_owned(), vec![ns2.addr]),
    ]);
    change(dns, &desec);
    let issuer = Issuer::new(
        settings,
        issuer::http_client().unwrap(),
        db.clone(),
        secrets,
        tls.resolver.clone(),
        tls.challenges.clone(),
    )
    .with_backoff(vec![Duration::from_millis(50)]);
    Setup {
        acme,
        desec,
        ns1,
        ns2,
        issuer,
        resolver: tls.resolver.clone(),
        db,
        ca_lookups,
        _dir: dir,
    }
}

impl Setup {
    async fn audit(&self) -> Vec<(String, String)> {
        db::call(&self.db, |conn| {
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

    async fn retry_at(&self) -> Option<Timestamp> {
        let stored: Option<Value> = db::call(&self.db, |conn| {
            roedy_hub_server::db::repo::app_settings::get(conn, "tls_retry")
        })
        .await
        .unwrap();
        stored?["at"].as_str()?.parse().ok()
    }

    /// The bulk `PUT`s' bodies, in order.
    fn writes(&self) -> Vec<Value> {
        self.desec
            .log()
            .into_iter()
            .filter(|request| request.method == "PUT" && request.status == 200)
            .filter_map(|request| request.body)
            .collect()
    }
}

/// The one TXT value of a write.
fn written_records(body: &Value) -> Vec<String> {
    body[0]["records"]
        .as_array()
        .unwrap()
        .iter()
        .map(|record| record.as_str().unwrap().to_owned())
        .collect()
}

#[tokio::test]
async fn issues_with_dns_01_at_desec() {
    let setup = setup(|_, _| {}).await;
    let outcome = setup.issuer.run_once().await.unwrap();
    assert!(
        matches!(outcome, Outcome::Issued { renewed: false, .. }),
        "{outcome:?}"
    );
    assert!(setup.resolver.current().is_some(), "certificate served");
    assert_eq!(setup.acme.log().validations, [("dns-01".to_owned(), true)]);

    // deSEC: the zone, its name servers, the record set read, written,
    // read and deleted – all with the token, all with trailing slashes.
    let log = setup.desec.log();
    let requests: Vec<(String, String)> = log
        .iter()
        .map(|request| (request.method.clone(), request.uri.clone()))
        .collect();
    let rrset = format!("/api/v1/domains/{ZONE}/rrsets/{SUBNAME}/TXT/");
    let bulk = format!("/api/v1/domains/{ZONE}/rrsets/");
    assert_eq!(
        requests,
        [
            ("GET".into(), format!("/api/v1/domains/?owns_qname={NAME}")),
            ("GET".into(), format!("/api/v1/domains/{ZONE}/rrsets/@/NS/")),
            ("GET".into(), rrset.clone()),
            ("PUT".into(), bulk.clone()),
            ("GET".into(), rrset),
            ("PUT".into(), bulk),
        ]
    );
    assert!(log.iter().all(|request| request.authorized));
    let writes = setup.writes();
    let value = written_records(&writes[0]);
    assert_eq!(value.len(), 1);
    assert_eq!(value[0].len(), 45, "a quoted SHA-256 in base64url");
    assert_eq!(writes[0], txt_body(SUBNAME, MINIMUM_TTL, &value));
    assert_eq!(writes[1], txt_body(SUBNAME, MINIMUM_TTL, &[]));
    assert_eq!(setup.desec.rrset(ZONE, SUBNAME, "TXT"), None, "deleted");
    assert!(setup.ns1.txt(NAME).is_empty());

    // Both name servers were asked before the CA looked.
    for ns in [&setup.ns1, &setup.ns2] {
        assert!(ns.queries().iter().any(|(name, _)| name == NAME));
    }
    let audit = setup.audit().await;
    assert_eq!(audit.len(), 1);
    assert_eq!(audit[0].0, "tls.issue");
    assert!(audit[0].1.contains(HOST));
}

#[tokio::test]
async fn keeps_other_values_of_the_record_set() {
    let setup = setup(|_, desec| desec.preset(ZONE, SUBNAME, 7200, &["\"other client\""])).await;
    setup.issuer.run_once().await.unwrap();
    let writes = setup.writes();
    assert_eq!(writes.len(), 2);
    let added = written_records(&writes[0]);
    assert_eq!(added[0], "\"other client\"");
    assert_eq!(added.len(), 2, "{added:?}");
    assert_eq!(
        writes[1],
        txt_body(SUBNAME, MINIMUM_TTL, &[quote("other client")])
    );
    assert_eq!(
        setup.desec.rrset(ZONE, SUBNAME, "TXT").unwrap().records,
        ["\"other client\""]
    );
    assert_eq!(setup.ns1.txt(NAME), ["other client"]);
}

#[tokio::test]
async fn removes_the_record_after_a_failed_validation() {
    let setup = setup(|_, _| {}).await;
    setup.acme.knobs(|knobs| knobs.fail = vec!["dns-01"]);
    let error = setup.issuer.run_once().await.unwrap_err();
    let message = format!("{error:#}");
    assert!(
        message.contains("dns-01 validation of hub.example.test failed"),
        "{message}"
    );
    assert_eq!(setup.acme.log().validations, [("dns-01".to_owned(), false)]);
    assert_eq!(setup.writes().len(), 2, "set and removed");
    assert_eq!(setup.desec.rrset(ZONE, SUBNAME, "TXT"), None);
    assert!(setup.resolver.current().is_none());
    let audit = setup.audit().await;
    assert_eq!(audit.len(), 1);
    assert_eq!(audit[0].0, "tls.fail");
    assert!(setup.retry_at().await.is_some());

    // A refused write: nothing to clean up, and the error says why.
    let setup = setup_with_forbidden_writes().await;
    let error = format!("{:#}", setup.issuer.run_once().await.unwrap_err());
    assert!(
        error.contains("deSEC answered 403 (the token may not do this)"),
        "{error}"
    );
    assert!(setup.acme.log().validations.is_empty());
}

async fn setup_with_forbidden_writes() -> Setup {
    let setup = setup(|_, _| {}).await;
    setup.desec.knobs(|knobs| knobs.forbid_writes = true);
    setup
}

#[tokio::test]
async fn waits_until_every_name_server_has_the_record() {
    let delay = Duration::from_millis(600);
    let setup = setup(|_, desec| {
        desec.knobs(|knobs| knobs.publish = vec![Some(Duration::ZERO), Some(delay)]);
    })
    .await;
    setup.issuer.run_once().await.unwrap();
    // The CA (asking the first name server, which had it at once) only
    // looked after the second one had it as well.
    let published = setup.ns2.set_at(NAME).unwrap();
    let looked = setup.ca_lookups.lock().unwrap().clone();
    assert_eq!(looked.len(), 1);
    assert!(looked[0] >= published, "the CA looked before ns2 had it");
    let asked = setup
        .ns2
        .queries()
        .iter()
        .filter(|(name, _)| name == NAME)
        .count();
    assert!(asked > 2, "ns2 asked again and again ({asked})");
}

#[tokio::test]
async fn gives_up_when_a_name_server_never_has_the_record() {
    let setup = setup(|dns, desec| {
        dns.propagation_timeout = Duration::from_millis(400);
        desec.knobs(|knobs| knobs.publish = vec![Some(Duration::ZERO), None]);
    })
    .await;
    let error = format!("{:#}", setup.issuer.run_once().await.unwrap_err());
    assert!(
        error.contains("is not on all name servers after 400ms: ns2.desec.test (not there yet)"),
        "{error}"
    );
    assert!(
        setup.acme.log().validations.is_empty(),
        "the CA was not asked"
    );
    assert_eq!(setup.desec.rrset(ZONE, SUBNAME, "TXT"), None, "cleaned up");

    // A name server that does not answer at all.
    let setup = setup_with_silent_name_server().await;
    let error = format!("{:#}", setup.issuer.run_once().await.unwrap_err());
    assert!(
        error.contains("ns2.desec.test (no DNS answer from 127.0.0.1:"),
        "{error}"
    );
}

async fn setup_with_silent_name_server() -> Setup {
    // A bound UDP socket that never answers, and no TCP.
    let silent = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
    let addr = silent.local_addr().unwrap();
    std::mem::forget(silent);
    setup(move |dns, _| {
        dns.propagation_timeout = Duration::from_millis(300);
        dns.query_timeout = Duration::from_millis(100);
        dns.name_servers
            .insert("ns2.desec.test".to_owned(), vec![addr]);
    })
    .await
}

#[tokio::test]
async fn waits_as_desec_throttles() {
    let setup = setup(|_, _| {}).await;
    setup.desec.knobs(|knobs| {
        knobs.throttle = 2;
        knobs.retry_after = "1".into();
    });
    let started = Instant::now();
    setup.issuer.run_once().await.unwrap();
    assert!(started.elapsed() >= Duration::from_secs(2));
    let throttled = setup
        .desec
        .log()
        .iter()
        .filter(|request| request.status == 429)
        .count();
    assert_eq!(throttled, 2);
    assert_eq!(setup.acme.log().validations, [("dns-01".to_owned(), true)]);

    // An hour: not waited for in place; the next attempt waits as long.
    let setup = setup_throttled_for_an_hour().await;
    let before = Timestamp::now();
    let error = format!("{:#}", setup.issuer.run_once().await.unwrap_err());
    assert!(error.contains("deSEC answered 429 (throttled)"), "{error}");
    let at = setup.retry_at().await.unwrap();
    assert!(
        at >= before + SignedDuration::from_secs(3590),
        "retry at {at}"
    );
    let outcome = setup.issuer.run_once().await.unwrap();
    assert!(matches!(outcome, Outcome::Waiting { .. }), "{outcome:?}");
}

async fn setup_throttled_for_an_hour() -> Setup {
    let setup = setup(|_, _| {}).await;
    setup.desec.knobs(|knobs| {
        knobs.throttle = 1;
        knobs.retry_after = "3600".into();
    });
    setup
}

#[tokio::test]
async fn refuses_a_name_outside_the_account() {
    let setup = setup(|_, _| {}).await;
    // An account with another domain only.
    let other = FakeDesec::start(TOKEN, "other.test", 60, &["ns1.desec.test."], Vec::new()).await;
    let mut settings = setup.issuer.settings().clone();
    let dns01::Provider::Desec { api, .. } = &mut settings.dns01.as_mut().unwrap().provider;
    *api = other.api();
    let issuer = Issuer::new(
        settings,
        issuer::http_client().unwrap(),
        setup.db.clone(),
        Arc::new(SecretKey::random().unwrap()),
        setup.resolver.clone(),
        Arc::new(tls::Challenges::default()),
    );
    let error = format!("{:#}", issuer.run_once().await.unwrap_err());
    assert!(
        error.contains("no domain in the deSEC account owns _acme-challenge.hub.example.test"),
        "{error}"
    );
    assert!(other.log().iter().all(|request| request.method == "GET"));
}

#[tokio::test]
async fn renews_with_dns_01() {
    let setup = setup(|_, _| {}).await;
    setup.issuer.run_once().await.unwrap();
    let first = setup.resolver.current().unwrap();
    // The CA's renewal window has begun.
    setup.acme.knobs(|knobs| {
        knobs.ari = Some((
            SignedDuration::from_hours(-1),
            SignedDuration::from_hours(1),
        ));
    });
    let outcome = setup.issuer.run_once().await.unwrap();
    assert!(
        matches!(outcome, Outcome::Issued { renewed: true, .. }),
        "{outcome:?}"
    );
    assert!(!Arc::ptr_eq(&first, &setup.resolver.current().unwrap()));
    assert_eq!(
        setup.acme.log().validations,
        [("dns-01".to_owned(), true), ("dns-01".to_owned(), true)]
    );
    assert_eq!(setup.writes().len(), 4);
    assert_eq!(setup.desec.rrset(ZONE, SUBNAME, "TXT"), None);
    let actions: Vec<String> = setup.audit().await.into_iter().map(|(a, _)| a).collect();
    assert_eq!(actions, ["tls.issue", "tls.renew"]);
}

/// Collects everything logged, at every level.
#[derive(Clone, Default)]
struct Captured(Arc<Mutex<Vec<u8>>>);

impl std::io::Write for Captured {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().write(buf)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Everything the test binary logs from the first call on, at every
/// level. A global subscriber: with a thread-local one alone, tracing caches
/// the callsites of other threads as uninteresting.
fn capture_logs() -> Captured {
    static LOGS: OnceLock<Captured> = OnceLock::new();
    LOGS.get_or_init(|| {
        let captured = Captured::default();
        let writer = captured.clone();
        tracing_subscriber::fmt()
            .with_max_level(tracing::Level::TRACE)
            .with_ansi(false)
            .with_writer(move || writer.clone())
            .init();
        captured
    })
    .clone()
}

#[tokio::test]
async fn the_token_shows_up_nowhere() {
    let captured = capture_logs();

    // A successful issuance, a failed validation and a rejected token.
    let setup = setup(|_, _| {}).await;
    setup.issuer.run_once().await.unwrap();
    let settings = format!("{:?}", setup.issuer.settings());
    let mut errors = Vec::new();
    let failing = self::setup(|_, _| {}).await;
    failing.acme.knobs(|knobs| knobs.fail = vec!["dns-01"]);
    errors.push(format!(
        "{:?}",
        failing.issuer.run_once().await.unwrap_err()
    ));
    // A token deSEC does not know (revoked).
    let rejected = self::setup(|_, _| {}).await;
    let mut settings_wrong = rejected.issuer.settings().clone();
    let dns01::Provider::Desec { token, .. } = &mut settings_wrong.dns01.as_mut().unwrap().provider;
    *token = roedy_hub_server::config::Secret::new(format!("{TOKEN}-revoked"));
    let wrong = Issuer::new(
        settings_wrong,
        issuer::http_client().unwrap(),
        rejected.db.clone(),
        Arc::new(SecretKey::random().unwrap()),
        rejected.resolver.clone(),
        Arc::new(tls::Challenges::default()),
    );
    let error = wrong.run_once().await.unwrap_err();
    assert!(
        format!("{error:#}").contains("deSEC answered 401 (the token is not valid)"),
        "{error:#}"
    );
    errors.push(format!("{error:?}"));

    let logs = String::from_utf8(captured.0.lock().unwrap().clone()).unwrap();
    assert!(logs.contains("DNS-01: TXT record set"), "logging captured");
    for (what, text) in [("logs", &logs), ("settings", &settings)]
        .into_iter()
        .chain(errors.iter().map(|error| ("error", error)))
    {
        assert!(!text.contains(TOKEN), "token in the {what}: {text}");
    }
    for setup in [&setup, &failing, &rejected] {
        for (action, details) in setup.audit().await {
            assert!(!details.contains(TOKEN), "token in {action}: {details}");
        }
    }
}

#[tokio::test]
async fn dns_client_reads_strings_and_falls_back_to_tcp() {
    let server = FakeDns::start().await;
    let long = "a".repeat(255);
    server.set_strings(NAME, vec![vec![long.clone(), "b".into()], vec!["c".into()]]);
    let timeout = Duration::from_secs(2);
    let response = dns::lookup_txt(server.addr, NAME, timeout).await.unwrap();
    assert!(response.authoritative);
    assert_eq!(response.rcode, dns::RCODE_NOERROR);
    assert_eq!(response.txt, [format!("{long}b"), "c".into()]);
    assert_eq!(server.queries(), [(NAME.to_owned(), Transport::Udp)]);

    server.truncate_udp(true);
    let response = dns::lookup_txt(server.addr, "_ACME-challenge.hub.example.test.", timeout)
        .await
        .unwrap();
    assert!(!response.truncated);
    assert_eq!(response.txt.len(), 2);
    assert_eq!(
        server.queries()[1..],
        [
            (NAME.to_owned(), Transport::Udp),
            (NAME.to_owned(), Transport::Tcp)
        ]
    );

    let response = dns::lookup_txt(server.addr, "missing.example.test", timeout)
        .await
        .unwrap();
    assert_eq!(response.rcode, dns::RCODE_NXDOMAIN);
    assert!(response.txt.is_empty());
}
