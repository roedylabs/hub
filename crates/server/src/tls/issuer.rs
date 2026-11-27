//! `ROEDY_TLS=acme`: gets and renews the certificate for the public host.
//!
//! - **At start** [`Issuer::load`] serves the stored certificate, if there
//!   is one, and [`Issuer::keep_valid`] gets a first one (or renews a due
//!   one) in the background, retrying until it succeeds. Meanwhile the
//!   HTTPS listener answers only TLS-ALPN-01 validations. Afterwards it
//!   retries every failed renewal on the same schedule.
//! - **Daily** (`jobs::certificate_renewal`) [`Issuer::run_once`] renews
//!   30 days before the end (a third of the lifetime for shorter
//!   certificates), or earlier when the CA's renewal info (ARI, RFC 9773)
//!   suggests it. A new certificate is swapped in without dropping
//!   connections.
//! - **Failures** wait 1 min, 5 min, 15 min, 1 h, then 3 h between
//!   attempts, or longer if the CA says so (`Retry-After`, e.g. a rate
//!   limit). The time of the next attempt is stored (`app_settings`, with
//!   the CA it applies to), so a restart does not hammer the CA, while
//!   switching to another CA does not wait for the previous one's backoff.
//!   The first failure of a series goes to the audit log, as do issuance
//!   and renewal.
//! - **Validation:** TLS-ALPN-01 on the HTTPS port first (port 80 then is
//!   optional), HTTP-01 on `ROEDY_HTTP_LISTEN` after a failed validation,
//!   alternating.
//! - **Storage:** account key, certificate key and chain in the database,
//!   the keys sealed with the instance key; with another instance key they
//!   cannot be opened, and the hub creates a new account and certificate.

use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use anyhow::{Context as _, anyhow, bail};
use jiff::{SignedDuration, Timestamp};
use rustls::pki_types::{CertificateDer, pem::PemObject as _};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest as _, Sha256};

use super::{
    CertResolver, Challenges,
    acme::{self, Client, Problem},
    certified_key, jose,
    key::P256Key,
    x509::{self, CertInfo},
};
use crate::{
    crypto::SecretKey,
    db::{
        self, Db,
        repo::{
            app_settings,
            audit_log::{self, Category, Entry},
            tls as store,
        },
    },
};

/// Renew this long before the end at the latest …
const RENEW_BEFORE: SignedDuration = SignedDuration::from_hours(30 * 24);
/// … or after two thirds of a shorter certificate's lifetime.
const RENEW_FRACTION: i32 = 3;
/// Waits after the 1st, 2nd … failure; the last one repeats.
pub const BACKOFF: [Duration; 5] = [
    Duration::from_secs(60),
    Duration::from_secs(5 * 60),
    Duration::from_secs(15 * 60),
    Duration::from_secs(60 * 60),
    Duration::from_secs(3 * 60 * 60),
];
/// Longest wait, whatever the CA asks for.
const MAX_WAIT: Duration = Duration::from_secs(24 * 60 * 60);

const ACCOUNT_PURPOSE: &str = "acme_accounts.key";
const CERT_PURPOSE: &str = "tls_certificates.key";
/// `app_settings` key of the retry state.
const RETRY_KEY: &str = "tls_retry";

/// HTTP client for the CA: rustls with ring and the system's roots (as
/// for the mail servers), no redirects, a deadline per request.
pub fn http_client() -> anyhow::Result<reqwest::Client> {
    let _ = rustls::crypto::ring::default_provider().install_default();
    reqwest::Client::builder()
        .user_agent(concat!("Roedy-Hub/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(30))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|error| anyhow!("HTTP client not created: {error}"))
}

/// What the issuer works with.
#[derive(Debug, Clone)]
pub struct Settings {
    /// The CA's directory URL.
    pub directory: String,
    /// `mailto:` contacts for the account.
    pub contact: Vec<String>,
    /// The DNS name of the certificate.
    pub domain: String,
}

/// What a run did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// The certificate is good until it is due.
    Valid { not_after: Timestamp },
    /// A new certificate: the first, for another CA or after an unreadable
    /// key (`renewed: false`), or a renewal.
    Issued { not_after: Timestamp, renewed: bool },
    /// An earlier failure waits until then.
    Waiting { until: Timestamp },
}

/// Stored retry state.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct Retry {
    failures: u32,
    at: Option<Timestamp>,
}

/// [`Retry`] as stored in `app_settings`.
#[derive(Debug, Default, Serialize, Deserialize)]
struct StoredRetry {
    failures: u32,
    at: Option<String>,
    /// The CA the failures were with: another `ROEDY_ACME_DIRECTORY` starts
    /// afresh instead of waiting for the previous CA's backoff.
    #[serde(default)]
    directory: Option<String>,
}

pub struct Issuer {
    settings: Settings,
    http: reqwest::Client,
    db: Db,
    secrets: Arc<SecretKey>,
    resolver: Arc<CertResolver>,
    challenges: Arc<Challenges>,
    /// One run at a time (start and the daily job).
    running: tokio::sync::Mutex<()>,
    /// Waits after failures ([`BACKOFF`]; shorter in tests).
    backoff: Vec<Duration>,
    /// Try HTTP-01 first (after a failed TLS-ALPN-01, and the other way).
    prefer_http: AtomicBool,
    /// A run failed: [`Self::keep_valid`] takes over the retries.
    failed: tokio::sync::Notify,
}

impl Issuer {
    pub fn new(
        settings: Settings,
        http: reqwest::Client,
        db: Db,
        secrets: Arc<SecretKey>,
        resolver: Arc<CertResolver>,
        challenges: Arc<Challenges>,
    ) -> Self {
        Self {
            settings,
            http,
            db,
            secrets,
            resolver,
            challenges,
            running: tokio::sync::Mutex::new(()),
            backoff: BACKOFF.to_vec(),
            prefer_http: AtomicBool::new(false),
            failed: tokio::sync::Notify::new(),
        }
    }

    /// Other waits after failures (tests).
    #[must_use]
    pub fn with_backoff(mut self, backoff: Vec<Duration>) -> Self {
        if !backoff.is_empty() {
            self.backoff = backoff;
        }
        self
    }

    pub fn settings(&self) -> &Settings {
        &self.settings
    }

    /// Serves the stored certificate, if it is for this domain, can be
    /// opened and has not expired; returns its end.
    pub async fn load(&self) -> anyhow::Result<Option<Timestamp>> {
        let Some((info, _)) = self.stored().await? else {
            return Ok(None);
        };
        Ok(Some(info.not_after))
    }

    /// Runs for the life of the server: gets a valid certificate, waiting
    /// after failures, then sleeps until a run elsewhere (the daily job)
    /// fails and retries that on the backoff schedule as well.
    pub async fn keep_valid(self: Arc<Self>) {
        loop {
            match self.run_once().await {
                Ok(Outcome::Waiting { until }) => {
                    tracing::info!(%until, "next certificate attempt waits");
                    tokio::time::sleep(until_now(until)).await;
                }
                Ok(_) => self.failed.notified().await,
                // Usually `run_once` stored when to try again and the next
                // round waits for it; but it may have failed before that (the
                // database), so never retry at once.
                Err(_) => tokio::time::sleep(self.backoff[0]).await,
            }
        }
    }

    /// One run: unless an earlier failure still waits, checks the
    /// certificate and gets a new one if it is due.
    pub async fn run_once(&self) -> anyhow::Result<Outcome> {
        let _running = self.running.lock().await;
        let retry = self.retry().await?;
        if let Some(at) = retry.at.filter(|at| *at > Timestamp::now()) {
            return Ok(Outcome::Waiting { until: at });
        }
        match self.ensure().await {
            Ok(outcome) => {
                // Also removes a stale row of another CA.
                self.set_retry(Retry::default(), None).await?;
                Ok(outcome)
            }
            Err(error) => {
                let failures = retry.failures.saturating_add(1);
                let step = usize::try_from(failures - 1)
                    .unwrap_or(usize::MAX)
                    .min(self.backoff.len() - 1);
                let asked = error
                    .downcast_ref::<Problem>()
                    .and_then(|problem| problem.retry_after);
                let wait = self.backoff[step]
                    .max(asked.unwrap_or_default())
                    .min(MAX_WAIT);
                let at = Timestamp::now() + SignedDuration::try_from(wait).unwrap_or_default();
                tracing::warn!(
                    domain = self.settings.domain,
                    failures,
                    retry_at = %at,
                    "TLS certificate not obtained: {error:#}"
                );
                // Only the first failure of a series goes to the audit log.
                let audit = (failures == 1).then(|| {
                    Entry::new(Category::System, "tls.fail").details(json!({
                        "domain": self.settings.domain,
                        "error": format!("{error:#}").chars().take(300).collect::<String>(),
                    }))
                });
                self.set_retry(
                    Retry {
                        failures,
                        at: Some(at),
                    },
                    audit,
                )
                .await?;
                // `keep_valid` takes over the retry, also of a failed daily
                // renewal.
                self.failed.notify_one();
                Err(error)
            }
        }
    }

    /// The stored certificate for this domain, served; `None` if there is
    /// none or it cannot be used.
    async fn stored(&self) -> anyhow::Result<Option<(CertInfo, String)>> {
        let domain = self.settings.domain.clone();
        let Some(row) =
            db::call(&self.db, move |conn| Ok(store::certificate(conn, &domain)?)).await?
        else {
            return Ok(None);
        };
        match self.open(&row) {
            Ok((key, info)) if info.not_after > Timestamp::now() => {
                self.resolver.set(key);
                Ok(Some((info, row.directory)))
            }
            Ok(_) => {
                tracing::info!(domain = row.domain, "stored TLS certificate expired");
                Ok(None)
            }
            Err(error) => {
                tracing::warn!(
                    domain = row.domain,
                    "stored TLS certificate not usable, getting a new one: {error:#}"
                );
                Ok(None)
            }
        }
    }

    fn open(
        &self,
        row: &store::Certificate,
    ) -> anyhow::Result<(Arc<rustls::sign::CertifiedKey>, CertInfo)> {
        let pkcs8 = self.secrets.open(CERT_PURPOSE, &row.domain, &row.key_enc)?;
        let chain = parse_chain(&row.chain_pem)?;
        let info = CertInfo::parse(&chain[0])?;
        Ok((certified_key(chain, &pkcs8)?, info))
    }

    /// Checks the certificate and gets a new one if needed.
    async fn ensure(&self) -> anyhow::Result<Outcome> {
        let now = Timestamp::now();
        let (replaces, renewed) = match self.stored().await? {
            None => (None, false),
            Some((_, directory)) if directory != self.settings.directory => {
                tracing::info!(
                    directory = self.settings.directory,
                    "ACME directory changed, getting a certificate from the new CA"
                );
                (None, false)
            }
            Some((info, _)) => {
                let mut due = renew_at(&info) <= now;
                let ari_id = info.ari_id();
                if !due && let Some(id) = &ari_id {
                    due = self.ari_due(id, now).await;
                }
                if !due {
                    return Ok(Outcome::Valid {
                        not_after: info.not_after,
                    });
                }
                (ari_id, true)
            }
        };
        let not_after = self.issue(replaces.as_deref(), renewed).await?;
        Ok(Outcome::Issued { not_after, renewed })
    }

    /// Whether ARI's window has begun; `false` if the CA has none or it
    /// cannot be asked (the fixed rule applies then).
    async fn ari_due(&self, ari_id: &str, now: Timestamp) -> bool {
        let directory = match acme::directory(&self.http, &self.settings.directory).await {
            Ok(directory) => directory,
            Err(error) => {
                tracing::debug!("renewal info not checked: {error:#}");
                return false;
            }
        };
        match acme::renewal_info(&self.http, &directory, ari_id).await {
            Ok(Some(window)) => {
                tracing::debug!(start = %window.start, end = %window.end, "renewal info");
                window.start <= now
            }
            Ok(None) => false,
            Err(error) => {
                tracing::debug!("renewal info not read: {error:#}");
                false
            }
        }
    }

    /// The account key and a client signed in to the CA.
    async fn client(&self) -> anyhow::Result<Client> {
        let directory = self.settings.directory.clone();
        let stored = db::call(&self.db, move |conn| Ok(store::account(conn, &directory)?)).await?;
        let known = stored.and_then(|account| {
            match self
                .secrets
                .open(ACCOUNT_PURPOSE, &account.directory, &account.key_enc)
                .and_then(|pkcs8| P256Key::from_pkcs8(&pkcs8))
            {
                Ok(key) => Some((key, account.url)),
                Err(error) => {
                    tracing::warn!(
                        "ACME account key not usable, creating a new account: {error:#}"
                    );
                    None
                }
            }
        });
        let (key, known_url) = match known {
            Some((key, url)) => (key, Some(url)),
            None => (P256Key::generate()?, None),
        };
        let mut client = Client::connect(self.http.clone(), &self.settings.directory, key).await?;
        let url = client.account(&self.settings.contact).await?;
        if known_url.as_deref() != Some(url.as_str()) {
            let sealed = self.secrets.seal(
                ACCOUNT_PURPOSE,
                &self.settings.directory,
                client.key().pkcs8(),
            )?;
            let account = store::Account {
                directory: self.settings.directory.clone(),
                key_enc: sealed,
                url: url.clone(),
            };
            db::call(&self.db, move |conn| {
                Ok(store::save_account(conn, &account)?)
            })
            .await?;
            tracing::info!(
                account = url,
                terms_of_service = client
                    .directory
                    .meta
                    .terms_of_service
                    .as_deref()
                    .unwrap_or("-"),
                "ACME account registered; its terms of service were accepted with \
                 ROEDY_ACME_AGREE_TOS"
            );
        }
        Ok(client)
    }

    /// Orders, validates, finalizes and stores a certificate; serves it.
    async fn issue(&self, replaces: Option<&str>, renewed: bool) -> anyhow::Result<Timestamp> {
        let domain = self.settings.domain.as_str();
        let mut client = self.client().await?;
        // `replaces` only for a CA with ARI (RFC 9773 § 5).
        let replaces = replaces.filter(|_| client.directory.renewal_info.is_some());
        let thumbprint = jose::thumbprint(client.key());
        tracing::info!(domain, renewal = renewed, "requesting a TLS certificate");

        let (order_url, order) = match client.new_order(domain, replaces).await {
            Ok(order) => order,
            // The CA does not take the replaced certificate (RFC 9773 § 5:
            // replaced already, or an ID it does not know): order without.
            // Any other error – a rate limit above all – stays an error.
            Err(error)
                if replaces.is_some()
                    && error.downcast_ref::<Problem>().is_some_and(|problem| {
                        matches!(problem.kind.as_str(), "alreadyReplaced" | "malformed")
                    }) =>
            {
                tracing::debug!("order with `replaces` refused, ordering without: {error:#}");
                client.new_order(domain, None).await?
            }
            Err(error) => return Err(error),
        };
        for url in &order.authorizations {
            self.authorize(&mut client, url, &thumbprint).await?;
        }
        let order = client.wait_for_order(&order_url).await?;
        if order.status != "ready" {
            bail!(
                "order is {} instead of ready: {}",
                order.status,
                acme::describe_error(order.error.as_ref())
            );
        }

        let key = P256Key::generate()?;
        client.finalize(&order, &x509::csr(domain, &key)?).await?;
        let order = client.wait_for_order(&order_url).await?;
        let chain_url = match (order.status.as_str(), &order.chain_url) {
            ("valid", Some(url)) => url.clone(),
            _ => bail!(
                "order is {} after finalizing: {}",
                order.status,
                acme::describe_error(order.error.as_ref())
            ),
        };
        let pem = client.download(&chain_url).await?;
        let chain = parse_chain(&pem)?;
        let info = CertInfo::parse(&chain[0]).context("issued certificate")?;
        if !info.dns_names.iter().any(|name| name == domain) {
            bail!("the issued certificate is not for {domain}");
        }
        let certified = certified_key(chain, key.pkcs8())?;

        let row = store::Certificate {
            domain: domain.to_owned(),
            key_enc: self.secrets.seal(CERT_PURPOSE, domain, key.pkcs8())?,
            chain_pem: pem,
            not_before: info.not_before,
            not_after: info.not_after,
            directory: self.settings.directory.clone(),
        };
        let action = if renewed { "tls.renew" } else { "tls.issue" };
        let entry = Entry::new(Category::System, action).details(json!({
            "domain": domain,
            "not_after": info.not_after.to_string(),
        }));
        db::call(&self.db, move |conn| {
            let tx = conn.transaction()?;
            store::save_certificate(&tx, &row)?;
            audit_log::insert(&tx, &entry)?;
            tx.commit()?;
            Ok(())
        })
        .await?;
        self.resolver.set(certified);
        tracing::info!(domain, not_after = %info.not_after, renewal = renewed, "TLS certificate issued");
        Ok(info.not_after)
    }

    /// Completes one authorization with a challenge the hub answers.
    async fn authorize(
        &self,
        client: &mut Client,
        url: &str,
        thumbprint: &str,
    ) -> anyhow::Result<()> {
        let authorization = client.authorization(url).await?;
        match authorization.status.as_str() {
            "valid" => return Ok(()),
            "pending" => {}
            status => bail!("authorization is {status}"),
        }
        let domain = authorization.identifier.value.as_str();
        if authorization.identifier.kind != "dns" || domain != self.settings.domain {
            bail!("unexpected identifier {}", authorization.identifier.value);
        }
        let order = if self.prefer_http.load(Ordering::Relaxed) {
            [acme::HTTP_01, acme::TLS_ALPN_01]
        } else {
            [acme::TLS_ALPN_01, acme::HTTP_01]
        };
        let challenge = order
            .iter()
            .find_map(|kind| authorization.challenges.iter().find(|c| c.kind == *kind))
            .ok_or_else(|| anyhow!("the CA offers neither TLS-ALPN-01 nor HTTP-01"))?;
        let key_authorization = format!("{}.{thumbprint}", challenge.token);

        let _answer = self.answer(
            &challenge.kind,
            &challenge.token,
            domain,
            &key_authorization,
        )?;
        tracing::info!(domain, challenge = challenge.kind, "validating the domain");
        client.respond(&challenge.url).await?;
        let authorization = client.wait_for_authorization(url).await?;
        if authorization.status == "valid" {
            return Ok(());
        }
        // Try the other kind next time.
        self.prefer_http
            .store(challenge.kind == acme::TLS_ALPN_01, Ordering::Relaxed);
        let reason = authorization
            .challenges
            .iter()
            .find(|c| c.url == challenge.url)
            .and_then(|c| c.error.as_ref());
        bail!(
            "{} validation of {domain} failed: {}",
            challenge.kind,
            acme::describe_error(reason)
        )
    }

    /// Puts the answer up for the validation; removed when the guard drops.
    fn answer(
        &self,
        kind: &str,
        token: &str,
        domain: &str,
        key_authorization: &str,
    ) -> anyhow::Result<Answer> {
        if !token
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
            || token.is_empty()
        {
            bail!("the CA sent an invalid token");
        }
        if kind == acme::HTTP_01 {
            self.challenges.add_http(token, key_authorization);
            return Ok(Answer::Http(self.challenges.clone(), token.to_owned()));
        }
        let digest: [u8; 32] = Sha256::digest(key_authorization.as_bytes()).into();
        let key = P256Key::generate()?;
        let der = x509::alpn_challenge_certificate(domain, &digest, &key, Timestamp::now())?;
        self.challenges
            .add_alpn(domain, super::challenge_key(der, key.pkcs8())?);
        Ok(Answer::Alpn(self.challenges.clone(), domain.to_owned()))
    }

    async fn retry(&self) -> anyhow::Result<Retry> {
        let stored = db::call(&self.db, |conn| {
            Ok(app_settings::get::<StoredRetry>(conn, RETRY_KEY)?.unwrap_or_default())
        })
        .await?;
        if stored.directory.as_deref() != Some(self.settings.directory.as_str()) {
            return Ok(Retry::default());
        }
        Ok(Retry {
            failures: stored.failures,
            at: stored.at.and_then(|at| at.parse().ok()),
        })
    }

    async fn set_retry(&self, retry: Retry, audit: Option<Entry>) -> anyhow::Result<()> {
        let directory = self.settings.directory.clone();
        db::call(&self.db, move |conn| {
            let tx = conn.transaction()?;
            if retry == Retry::default() {
                app_settings::delete(&tx, RETRY_KEY)?;
            } else {
                let stored = StoredRetry {
                    failures: retry.failures,
                    at: retry.at.map(|at| at.to_string()),
                    directory: Some(directory),
                };
                app_settings::set(&tx, RETRY_KEY, &stored)?;
            }
            if let Some(entry) = &audit {
                audit_log::insert(&tx, entry)?;
            }
            tx.commit()?;
            Ok(())
        })
        .await
    }
}

/// An answer to a validation, taken down on drop.
enum Answer {
    Http(Arc<Challenges>, String),
    Alpn(Arc<Challenges>, String),
}

impl Drop for Answer {
    fn drop(&mut self) {
        match self {
            Self::Http(challenges, token) => challenges.remove_http(token),
            Self::Alpn(challenges, domain) => challenges.remove_alpn(domain),
        }
    }
}

/// When a certificate is due: 30 days before its end, or after two thirds
/// of a shorter lifetime.
pub fn renew_at(info: &CertInfo) -> Timestamp {
    let lifetime = info.not_after.duration_since(info.not_before);
    let before = RENEW_BEFORE.min(lifetime / RENEW_FRACTION);
    info.not_after - before
}

/// The DER certificates of a PEM chain, leaf first.
pub fn parse_chain(pem: &str) -> anyhow::Result<Vec<CertificateDer<'static>>> {
    let chain = CertificateDer::pem_slice_iter(pem.as_bytes())
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| anyhow!("certificate chain is not PEM: {e}"))?;
    if chain.is_empty() {
        bail!("certificate chain is empty");
    }
    Ok(chain)
}

fn until_now(at: Timestamp) -> Duration {
    Duration::try_from(at.duration_since(Timestamp::now())).unwrap_or(Duration::ZERO)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(not_before: &str, not_after: &str) -> CertInfo {
        CertInfo {
            serial: vec![1],
            not_before: not_before.parse().unwrap(),
            not_after: not_after.parse().unwrap(),
            dns_names: Vec::new(),
            ip_addresses: Vec::new(),
            authority_key_id: None,
            subject_key_id: None,
            acme_identifier: None,
            spki: Vec::new(),
            tbs: Vec::new(),
            signature: Vec::new(),
        }
    }

    #[test]
    fn renews_30_days_before_or_after_two_thirds() {
        let ninety = info("2026-11-01T00:00:00Z", "2027-01-30T00:00:00Z");
        assert_eq!(renew_at(&ninety), "2026-12-31T00:00:00Z".parse().unwrap());
        let six_days = info("2026-11-01T00:00:00Z", "2026-11-07T00:00:00Z");
        assert_eq!(renew_at(&six_days), "2026-11-05T00:00:00Z".parse().unwrap());
    }
}
