//! Built-in TLS (`ROEDY_TLS`, docs/ops.md › TLS): the hub terminates TLS
//! itself and gets its certificate itself, without a proxy in front
//! (decided on 2026-11-27).
//!
//! - [`CertResolver`] hands rustls the current certificate – swapped in
//!   place on renewal or reload, open connections keep theirs – and, for
//!   the ACME server's TLS-ALPN-01 validation (`acme-tls/1`), the challenge
//!   certificate. Only the public host's name (SNI) gets a certificate.
//! - [`listener`] accepts TLS connections for axum; [`redirect`] is the
//!   plain HTTP listener with HTTP-01 challenges and the redirect.
//! - [`issuer`] runs ACME ([`acme`]: the hub's own RFC 8555 client with
//!   [`jose`], [`x509`] and [`der`]); with `ROEDY_ACME_CHALLENGE=dns-01` it
//!   validates through a TXT record at deSEC ([`dns01`], [`desec`], [`dns`]).
//!   [`files`] reads and reloads a certificate from files.
//!
//! ALPN offers `http/1.1` only: axum serves HTTP/1.1 here (no `http2`
//! feature), and the gateway's WebSocket tunnel uses HTTP/1.1's `Upgrade`;
//! over HTTP/2 it would need RFC 8441, which neither side speaks.

pub mod acme;
pub mod der;
pub mod desec;
pub mod dns;
pub mod dns01;
pub mod files;
pub mod issuer;
pub mod jose;
pub mod key;
pub mod listener;
pub mod redirect;
pub mod x509;

use std::{
    collections::HashMap,
    fmt,
    sync::{Arc, Mutex, RwLock},
};

use rustls::{
    ServerConfig,
    crypto::ring::{default_provider, sign::any_supported_type},
    pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer},
    server::{ClientHello, ResolvesServerCert},
    sign::CertifiedKey,
};

use crate::{
    config::{AcmeChallenge, Config, TlsMode},
    crypto::SecretKey,
    db::Db,
};

/// ALPN of the TLS-ALPN-01 validation (RFC 8737 § 6.2).
pub const ACME_TLS_ALPN: &[u8] = b"acme-tls/1";
/// The only application protocol over TLS.
pub const HTTP_1_1: &[u8] = b"http/1.1";

/// Open ACME challenges: HTTP-01 key authorizations by token, TLS-ALPN-01
/// certificates by domain.
#[derive(Default)]
pub struct Challenges {
    http: Mutex<HashMap<String, String>>,
    alpn: Mutex<HashMap<String, Arc<CertifiedKey>>>,
}

impl fmt::Debug for Challenges {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Challenges(…)")
    }
}

impl Challenges {
    pub fn add_http(&self, token: &str, key_authorization: &str) {
        lock(&self.http).insert(token.to_owned(), key_authorization.to_owned());
    }

    pub fn http(&self, token: &str) -> Option<String> {
        lock(&self.http).get(token).cloned()
    }

    pub fn remove_http(&self, token: &str) {
        lock(&self.http).remove(token);
    }

    pub fn add_alpn(&self, domain: &str, key: Arc<CertifiedKey>) {
        lock(&self.alpn).insert(domain.to_owned(), key);
    }

    pub fn alpn(&self, domain: &str) -> Option<Arc<CertifiedKey>> {
        lock(&self.alpn).get(domain).cloned()
    }

    pub fn remove_alpn(&self, domain: &str) {
        lock(&self.alpn).remove(domain);
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|e| e.into_inner())
}

/// Picks the certificate for a TLS handshake.
pub struct CertResolver {
    /// The public host, lower case (a DNS name or an IP address).
    host: String,
    /// An IP address as host: clients send no SNI for it (RFC 6066 § 3).
    host_is_ip: bool,
    current: RwLock<Option<Arc<CertifiedKey>>>,
    challenges: Arc<Challenges>,
}

impl fmt::Debug for CertResolver {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CertResolver")
            .field("host", &self.host)
            .finish_non_exhaustive()
    }
}

impl CertResolver {
    pub fn new(host: &str, challenges: Arc<Challenges>) -> Self {
        Self {
            host: host.to_ascii_lowercase(),
            host_is_ip: host.parse::<std::net::IpAddr>().is_ok(),
            current: RwLock::new(None),
            challenges,
        }
    }

    pub fn host(&self) -> &str {
        &self.host
    }

    /// Serves `key` from the next handshake on.
    pub fn set(&self, key: Arc<CertifiedKey>) {
        *self.current.write().unwrap_or_else(|e| e.into_inner()) = Some(key);
    }

    pub fn current(&self) -> Option<Arc<CertifiedKey>> {
        self.current
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// The decision, apart from rustls' types: the SNI sent (if any) and
    /// what the client offers in ALPN.
    fn pick(&self, server_name: Option<&str>, offer: Offer) -> Option<Arc<CertifiedKey>> {
        let name = server_name.map(str::to_ascii_lowercase);
        match offer {
            // RFC 8737 § 3: the validation sends the domain as SNI.
            Offer::AcmeOnly => return self.challenges.alpn(name.as_deref()?),
            // `acme-tls/1` is for the validation only (RFC 8737 § 6.2): mixed
            // with other protocols the handshake fails, as for any protocol
            // the hub does not speak.
            Offer::AcmeMixed => return None,
            Offer::Other => {}
        }
        match name.as_deref() {
            Some(name) if name == self.host => self.current(),
            None if self.host_is_ip => self.current(),
            // Another name, or none for a DNS host: no certificate, the
            // handshake fails. The hub answers only under its own name.
            _ => None,
        }
    }
}

impl ResolvesServerCert for CertResolver {
    fn resolve(&self, client_hello: ClientHello<'_>) -> Option<Arc<CertifiedKey>> {
        let offer = match client_hello.alpn() {
            Some(protocols) => {
                let protocols: Vec<&[u8]> = protocols.collect();
                match protocols.iter().filter(|p| **p == ACME_TLS_ALPN).count() {
                    0 => Offer::Other,
                    _ if protocols.len() == 1 => Offer::AcmeOnly,
                    _ => Offer::AcmeMixed,
                }
            }
            None => Offer::Other,
        };
        self.pick(client_hello.server_name(), offer)
    }
}

/// What a client offers in ALPN, as far as the resolver cares.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Offer {
    /// No ALPN, or protocols without `acme-tls/1`.
    Other,
    /// `acme-tls/1` alone: a TLS-ALPN-01 validation.
    AcmeOnly,
    /// `acme-tls/1` together with other protocols.
    AcmeMixed,
}

/// The server configuration: TLS 1.3 and 1.2 with ring's default suites
/// (AEAD only, forward secrecy), ALPN `http/1.1` and `acme-tls/1`, the
/// certificate from `resolver`.
pub fn server_config(resolver: Arc<CertResolver>) -> anyhow::Result<Arc<ServerConfig>> {
    let mut config = ServerConfig::builder_with_provider(Arc::new(default_provider()))
        .with_protocol_versions(&[&rustls::version::TLS13, &rustls::version::TLS12])?
        .with_no_client_auth()
        .with_cert_resolver(resolver);
    config.alpn_protocols = vec![HTTP_1_1.to_vec(), ACME_TLS_ALPN.to_vec()];
    Ok(Arc::new(config))
}

/// A certified key from a DER chain and a PKCS#8 key; checks that they
/// belong together.
pub fn certified_key(
    chain: Vec<CertificateDer<'static>>,
    pkcs8: &[u8],
) -> anyhow::Result<Arc<CertifiedKey>> {
    certified_key_from(
        chain,
        &PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(pkcs8.to_vec())),
    )
}

/// The TLS-ALPN-01 certificate with its key. Without [`certified_key`]'s
/// check: rustls checks with webpki, which (rightly, for a browser) rejects
/// the unknown critical `acmeIdentifier`; a server never parses its own
/// certificate in the handshake.
pub fn challenge_key(der: Vec<u8>, pkcs8: &[u8]) -> anyhow::Result<Arc<CertifiedKey>> {
    let key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(pkcs8.to_vec()));
    let signer =
        any_supported_type(&key).map_err(|e| anyhow::anyhow!("unsupported private key: {e}"))?;
    Ok(Arc::new(CertifiedKey::new(vec![der.into()], signer)))
}

pub fn certified_key_from(
    chain: Vec<CertificateDer<'static>>,
    key: &PrivateKeyDer<'_>,
) -> anyhow::Result<Arc<CertifiedKey>> {
    anyhow::ensure!(!chain.is_empty(), "the certificate chain is empty");
    let signer =
        any_supported_type(key).map_err(|e| anyhow::anyhow!("unsupported private key: {e}"))?;
    let certified = CertifiedKey::new(chain, signer);
    certified
        .keys_match()
        .map_err(|e| anyhow::anyhow!("certificate and key do not match: {e}"))?;
    Ok(Arc::new(certified))
}

/// TLS of a running hub: the resolver, the challenges and, for `acme`, the
/// issuer. Part of `AppState` (`None` with `ROEDY_TLS=off`).
pub struct Tls {
    pub mode: TlsMode,
    pub resolver: Arc<CertResolver>,
    pub challenges: Arc<Challenges>,
    pub server_config: Arc<ServerConfig>,
    pub issuer: Option<Arc<issuer::Issuer>>,
    pub files: Option<files::Files>,
}

impl fmt::Debug for Tls {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Tls")
            .field("mode", &self.mode)
            .field("host", &self.resolver.host())
            .finish_non_exhaustive()
    }
}

impl Tls {
    /// TLS for `host` without a certificate yet.
    pub fn new(mode: TlsMode, host: &str) -> anyhow::Result<Self> {
        let challenges = Arc::new(Challenges::default());
        let resolver = Arc::new(CertResolver::new(host, challenges.clone()));
        Ok(Self {
            mode,
            server_config: server_config(resolver.clone())?,
            resolver,
            challenges,
            issuer: None,
            files: None,
        })
    }

    /// Starts what keeps the certificate current: the first issuance and
    /// the retries of failed renewals (`acme`), or watching the files
    /// (`files`). Abort the task on shutdown.
    pub fn start(&self) -> Option<tokio::task::JoinHandle<()>> {
        if let Some(issuer) = &self.issuer {
            return Some(tokio::spawn(issuer.clone().keep_valid()));
        }
        self.files
            .clone()
            .map(|files| files.watch(self.resolver.clone(), files::POLL))
    }
}

/// TLS as configured, with the stored or the files' certificate loaded;
/// `None` with `ROEDY_TLS=off`. For `files` an unusable pair fails the start.
pub async fn prepare(
    config: &Config,
    db: &Db,
    secrets: Arc<SecretKey>,
) -> anyhow::Result<Option<Tls>> {
    if !config.tls.enabled() {
        return Ok(None);
    }
    let host = config
        .tls_host()
        .ok_or_else(|| anyhow::anyhow!("ROEDY_PUBLIC_URL is not set"))?;
    let mut tls = Tls::new(config.tls, &host)?;
    match config.tls {
        TlsMode::Off => {}
        TlsMode::Files => {
            let (Some(cert), Some(key)) = (&config.tls_cert_file, &config.tls_key_file) else {
                anyhow::bail!("ROEDY_TLS=files needs ROEDY_TLS_CERT_FILE and ROEDY_TLS_KEY_FILE");
            };
            let files = files::Files::new(cert.clone(), key.clone());
            let info = files.load(&tls.resolver).map_err(|e| {
                anyhow::anyhow!("TLS certificate not usable (ROEDY_TLS=files): {e:#}")
            })?;
            tracing::info!(not_after = %info.not_after, file = %cert.display(), "TLS certificate loaded");
            tls.files = Some(files);
        }
        TlsMode::Acme => {
            let dns01 = match config.acme_challenge {
                AcmeChallenge::TlsAlpn01 => None,
                AcmeChallenge::Dns01 => {
                    // Reading the token file blocks.
                    let config = config.clone();
                    let token = tokio::task::spawn_blocking(move || config.desec_token()).await??;
                    Some(dns01::Settings::desec(token))
                }
            };
            let issuer = issuer::Issuer::new(
                issuer::Settings {
                    directory: config.acme_directory.to_string(),
                    contact: config
                        .acme_email
                        .iter()
                        .map(|email| format!("mailto:{email}"))
                        .collect(),
                    domain: host,
                    dns01,
                },
                issuer::http_client()?,
                db.clone(),
                secrets,
                tls.resolver.clone(),
                tls.challenges.clone(),
            );
            match config.acme_challenge {
                AcmeChallenge::TlsAlpn01 => {
                    tracing::info!("ACME validation: TLS-ALPN-01, HTTP-01 after a failure");
                }
                AcmeChallenge::Dns01 => {
                    tracing::info!("ACME validation: DNS-01 with a TXT record at deSEC");
                }
            }
            match issuer.load().await? {
                Some(not_after) => {
                    tracing::info!(%not_after, "stored TLS certificate loaded");
                }
                None => tracing::info!(
                    directory = %config.acme_directory,
                    "no TLS certificate yet; HTTPS starts once the CA has issued one"
                ),
            }
            tls.issuer = Some(Arc::new(issuer));
        }
    }
    Ok(Some(tls))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tls::key::P256Key;

    fn key() -> Arc<CertifiedKey> {
        let key = P256Key::generate().unwrap();
        let der = x509::CertificateParams {
            serial: vec![1],
            issuer: x509::name(None),
            subject: x509::name(None),
            not_before: jiff::Timestamp::now(),
            not_after: jiff::Timestamp::now() + jiff::SignedDuration::from_hours(1),
            spki: key.spki(),
            extensions: vec![x509::Extension::dns_names(&["hub.example.test"])],
        }
        .sign(&key)
        .unwrap();
        certified_key(vec![der.into()], key.pkcs8()).unwrap()
    }

    #[test]
    fn serves_only_the_public_name() {
        let challenges = Arc::new(Challenges::default());
        let resolver = CertResolver::new("Hub.Example.test", challenges.clone());
        assert!(
            resolver
                .pick(Some("hub.example.test"), Offer::Other)
                .is_none(),
            "no cert yet"
        );
        let current = key();
        resolver.set(current.clone());
        let picked = resolver
            .pick(Some("HUB.example.test"), Offer::Other)
            .unwrap();
        assert!(Arc::ptr_eq(&picked, &current));
        assert!(
            resolver
                .pick(Some("other.example.test"), Offer::Other)
                .is_none()
        );
        assert!(resolver.pick(None, Offer::Other).is_none(), "no SNI");

        // `acme-tls/1` alone: the challenge certificate, never the current one.
        assert!(
            resolver
                .pick(Some("hub.example.test"), Offer::AcmeOnly)
                .is_none()
        );
        let challenge = key();
        challenges.add_alpn("hub.example.test", challenge.clone());
        let picked = resolver
            .pick(Some("hub.example.test"), Offer::AcmeOnly)
            .unwrap();
        assert!(Arc::ptr_eq(&picked, &challenge));
        assert!(resolver.pick(None, Offer::AcmeOnly).is_none());
        // Offered with other protocols: no certificate at all.
        assert!(
            resolver
                .pick(Some("hub.example.test"), Offer::AcmeMixed)
                .is_none()
        );
        challenges.remove_alpn("hub.example.test");
        assert!(
            resolver
                .pick(Some("hub.example.test"), Offer::AcmeOnly)
                .is_none()
        );

        // An IP address as host: clients send no SNI.
        let resolver = CertResolver::new("192.0.2.7", challenges);
        resolver.set(current);
        assert!(resolver.pick(None, Offer::Other).is_some());
    }

    #[test]
    fn rejects_a_key_of_another_certificate() {
        let one = P256Key::generate().unwrap();
        let other = P256Key::generate().unwrap();
        let chain = key().cert.clone();
        let err = certified_key(chain, other.pkcs8()).unwrap_err();
        assert!(err.to_string().contains("do not match"), "{err}");
        assert!(certified_key(Vec::new(), one.pkcs8()).is_err());

        // The challenge certificate goes without that check.
        let der = x509::alpn_challenge_certificate(
            "hub.example.test",
            &[0; 32],
            &one,
            jiff::Timestamp::now(),
        )
        .unwrap();
        assert!(certified_key(vec![der.clone().into()], one.pkcs8()).is_err());
        challenge_key(der, one.pkcs8()).unwrap();
    }

    #[test]
    fn http_challenges() {
        let challenges = Challenges::default();
        assert_eq!(challenges.http("t"), None);
        challenges.add_http("t", "t.thumb");
        assert_eq!(challenges.http("t").as_deref(), Some("t.thumb"));
        challenges.remove_http("t");
        assert_eq!(challenges.http("t"), None);
    }
}
