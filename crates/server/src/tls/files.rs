//! `ROEDY_TLS=files`: the certificate chain and key from PEM files (any key
//! type rustls supports), checked at start – the start fails without a
//! usable pair – and reloaded when the files change.
//!
//! [`Files::watch`] reads both files every [`POLL`] (asynchronously; a few
//! KiB) and compares a SHA-256 of their content with the pair it installed
//! last, so a replacement is found whatever its size and modification time.
//! A pair is installed only once it is usable: it fits together, names the
//! host and is valid now. Until then the previous certificate stays and the
//! pair is tried again at every poll – a certificate whose `notBefore` lies
//! ahead is installed when that time comes; a broken pair is logged once.

use std::{
    net::IpAddr,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Duration,
};

use anyhow::{Context as _, anyhow, bail};
use jiff::Timestamp;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, pem::PemObject as _};
use sha2::{Digest as _, Sha256};
use tokio::task::JoinHandle;

use super::{CertResolver, certified_key_from, x509::CertInfo};

/// How often the files are checked for changes.
pub const POLL: Duration = Duration::from_secs(10);

/// A certificate this close to its end is reported at every load.
const WARN_BEFORE: jiff::SignedDuration = jiff::SignedDuration::from_hours(14 * 24);

/// The content of a pair of files.
type Fingerprint = [u8; 32];

#[derive(Debug, Clone)]
pub struct Files {
    pub cert: PathBuf,
    pub key: PathBuf,
    /// What is served now.
    installed: Arc<Mutex<Option<Fingerprint>>>,
}

impl Files {
    pub fn new(cert: PathBuf, key: PathBuf) -> Self {
        Self {
            cert,
            key,
            installed: Arc::default(),
        }
    }

    /// Reads and installs the pair into `resolver` (at start).
    pub fn load(&self, resolver: &CertResolver) -> anyhow::Result<CertInfo> {
        let cert = std::fs::read(&self.cert)
            .with_context(|| format!("{} not readable", self.cert.display()))?;
        let key = std::fs::read(&self.key)
            .with_context(|| format!("{} not readable", self.key.display()))?;
        self.install(resolver, &cert, &key, Timestamp::now())
    }

    /// Checks a pair and serves it; nothing changes if it is not usable.
    fn install(
        &self,
        resolver: &CertResolver,
        cert_pem: &[u8],
        key_pem: &[u8],
        now: Timestamp,
    ) -> anyhow::Result<CertInfo> {
        let (key, info) = parse(cert_pem, key_pem)
            .with_context(|| format!("{} and {}", self.cert.display(), self.key.display()))?;
        let host = resolver.host();
        if !covers(&info, host) {
            let mut names: Vec<String> = info.dns_names.clone();
            names.extend(info.ip_addresses.iter().map(ToString::to_string));
            bail!(
                "{} is not for {host} (names: {})",
                self.cert.display(),
                if names.is_empty() {
                    "none".to_owned()
                } else {
                    names.join(", ")
                }
            );
        }
        if now < info.not_before {
            bail!(
                "{} is not valid before {}",
                self.cert.display(),
                info.not_before
            );
        }
        if info.not_after <= now {
            bail!("{} expired on {}", self.cert.display(), info.not_after);
        }
        if info.not_after - WARN_BEFORE < now {
            tracing::warn!(not_after = %info.not_after, file = %self.cert.display(), "TLS certificate expires soon");
        }
        resolver.set(key);
        *self.installed.lock().unwrap_or_else(|e| e.into_inner()) =
            Some(fingerprint(cert_pem, key_pem));
        Ok(info)
    }

    fn installed(&self) -> Option<Fingerprint> {
        *self.installed.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Reloads the pair whenever its content changes, until the task is
    /// aborted.
    pub fn watch(self, resolver: Arc<CertResolver>, every: Duration) -> JoinHandle<()> {
        tokio::spawn(async move {
            // The content last reported as unusable, so it is logged once.
            let mut reported: Option<Fingerprint> = None;
            loop {
                tokio::time::sleep(every).await;
                let (cert, key) =
                    match tokio::try_join!(tokio::fs::read(&self.cert), tokio::fs::read(&self.key))
                    {
                        Ok(pair) => pair,
                        Err(error) => {
                            // Missing for a moment while being replaced, or
                            // not readable: the current certificate stays.
                            tracing::debug!("TLS files not read: {error}");
                            continue;
                        }
                    };
                let current = fingerprint(&cert, &key);
                if self.installed() == Some(current) {
                    continue;
                }
                match self.install(&resolver, &cert, &key, Timestamp::now()) {
                    Ok(info) => {
                        reported = None;
                        tracing::info!(
                            not_after = %info.not_after,
                            file = %self.cert.display(),
                            "TLS certificate reloaded"
                        );
                    }
                    Err(error) if reported != Some(current) => {
                        reported = Some(current);
                        tracing::warn!(
                            "TLS certificate not reloaded, the previous one stays: {error:#}"
                        );
                    }
                    Err(error) => tracing::debug!("TLS certificate still not usable: {error:#}"),
                }
            }
        })
    }
}

fn fingerprint(cert: &[u8], key: &[u8]) -> Fingerprint {
    let mut hash = Sha256::new();
    hash.update(Sha256::digest(cert));
    hash.update(Sha256::digest(key));
    hash.finalize().into()
}

/// Whether the certificate names `host`: an IP host among its IP addresses,
/// a DNS host among its DNS names (a `*.` wildcard covers one label).
fn covers(info: &CertInfo, host: &str) -> bool {
    match host.parse::<IpAddr>() {
        Ok(ip) => info.ip_addresses.contains(&ip),
        Err(_) => info.dns_names.iter().any(|name| matches(name, host)),
    }
}

fn matches(name: &str, host: &str) -> bool {
    let name = name.to_ascii_lowercase();
    match name.strip_prefix("*.") {
        Some(parent) => host
            .split_once('.')
            .is_some_and(|(label, rest)| !label.is_empty() && rest == parent),
        None => name == host,
    }
}

fn parse(
    cert_pem: &[u8],
    key_pem: &[u8],
) -> anyhow::Result<(Arc<rustls::sign::CertifiedKey>, CertInfo)> {
    let chain = CertificateDer::pem_slice_iter(cert_pem)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| anyhow!("not a PEM certificate chain: {e}"))?;
    let leaf = chain
        .first()
        .ok_or_else(|| anyhow!("no certificate in the chain file"))?;
    let info = CertInfo::parse(leaf).context("certificate")?;
    let private =
        PrivateKeyDer::from_pem_slice(key_pem).map_err(|e| anyhow!("no PEM private key: {e}"))?;
    Ok((certified_key_from(chain, &private)?, info))
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;
    use crate::tls::{Challenges, key::P256Key, x509};

    #[test]
    fn names_cover_the_host() {
        assert!(matches("hub.example.test", "hub.example.test"));
        assert!(matches("HUB.example.test", "hub.example.test"));
        assert!(matches("*.example.test", "hub.example.test"));
        assert!(!matches("*.example.test", "a.hub.example.test"));
        assert!(!matches("*.example.test", "example.test"));
        assert!(!matches("other.example.test", "hub.example.test"));
    }

    pub(crate) fn pem(label: &str, der: &[u8]) -> String {
        use base64::Engine as _;
        let body = base64::engine::general_purpose::STANDARD.encode(der);
        let lines: Vec<&str> = body
            .as_bytes()
            .chunks(64)
            .map(|c| std::str::from_utf8(c).unwrap())
            .collect();
        format!(
            "-----BEGIN {label}-----\n{}\n-----END {label}-----\n",
            lines.join("\n")
        )
    }

    /// A self-signed pair with these names, valid from `not_before` for 90
    /// days.
    fn pair(dns: &[&str], ips: &[IpAddr], not_before: Timestamp) -> (Vec<u8>, String, String) {
        let key = P256Key::generate().unwrap();
        let der = x509::CertificateParams {
            serial: crate::random::bytes::<8>().unwrap().to_vec(),
            issuer: x509::name(None),
            subject: x509::name(None),
            not_before,
            not_after: not_before + jiff::SignedDuration::from_hours(24 * 90),
            spki: key.spki(),
            extensions: vec![x509::Extension::alt_names(dns, ips)],
        }
        .sign(&key)
        .unwrap();
        let cert = pem("CERTIFICATE", &der);
        (der, cert, pem("PRIVATE KEY", key.pkcs8()))
    }

    fn write(dir: &Path, name: &str) -> (Vec<u8>, Files) {
        let ago = Timestamp::now() - jiff::SignedDuration::from_hours(1);
        let (der, cert, key) = pair(&[name], &[], ago);
        let files = Files::new(dir.join("cert.pem"), dir.join("key.pem"));
        std::fs::write(&files.cert, cert).unwrap();
        std::fs::write(&files.key, key).unwrap();
        (der, files)
    }

    fn resolver(host: &str) -> Arc<CertResolver> {
        Arc::new(CertResolver::new(host, Arc::new(Challenges::default())))
    }

    fn served(resolver: &CertResolver) -> Vec<u8> {
        resolver.current().unwrap().cert[0].to_vec()
    }

    async fn until_served(resolver: &CertResolver, der: &[u8]) {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        while served(resolver) != der {
            assert!(tokio::time::Instant::now() < deadline, "not reloaded");
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    #[test]
    fn checks_names_and_validity() {
        let now = Timestamp::now();
        let ago = now - jiff::SignedDuration::from_hours(1);
        let files = Files::new("cert.pem".into(), "key.pem".into());

        // An IP host needs the address among the IP names.
        let ip: IpAddr = "192.0.2.7".parse().unwrap();
        let (_, cert, key) = pair(&["hub.example.test"], &[ip], ago);
        let by_ip = resolver("192.0.2.7");
        files
            .install(&by_ip, cert.as_bytes(), key.as_bytes(), now)
            .unwrap();
        assert!(by_ip.current().is_some());
        let err = files
            .install(&resolver("192.0.2.8"), cert.as_bytes(), key.as_bytes(), now)
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("is not for 192.0.2.8 (names: hub.example.test, 192.0.2.7)"),
            "{err}"
        );

        // Not valid yet, or expired: the previous one stays.
        let host = resolver("hub.example.test");
        let (first, cert, key) = pair(&["hub.example.test"], &[], ago);
        files
            .install(&host, cert.as_bytes(), key.as_bytes(), now)
            .unwrap();
        let later = now + jiff::SignedDuration::from_hours(2);
        let (_, cert, key) = pair(&["hub.example.test"], &[], later);
        let err = files
            .install(&host, cert.as_bytes(), key.as_bytes(), now)
            .unwrap_err()
            .to_string();
        assert!(err.contains("is not valid before"), "{err}");
        let err = files
            .install(
                &host,
                cert.as_bytes(),
                key.as_bytes(),
                later + jiff::SignedDuration::from_hours(24 * 91),
            )
            .unwrap_err()
            .to_string();
        assert!(err.contains("expired on"), "{err}");
        assert_eq!(served(&host), first);
        // Once its time has come, it is installed.
        files
            .install(&host, cert.as_bytes(), key.as_bytes(), later)
            .unwrap();
        assert_ne!(served(&host), first);
    }

    #[tokio::test]
    async fn reloads_changed_content() {
        let tmp = tempfile::tempdir().unwrap();
        let resolver = resolver("hub.example.test");
        let (first, files) = write(tmp.path(), "hub.example.test");
        files.load(&resolver).unwrap();
        assert_eq!(served(&resolver), first);

        // Another name is refused, also at start; a missing file too.
        let other = tempfile::tempdir().unwrap();
        let (_, wrong) = write(other.path(), "other.example.test");
        let err = wrong.load(&resolver).unwrap_err().to_string();
        assert!(err.contains("is not for hub.example.test"), "{err}");
        let missing = Files::new(tmp.path().join("missing.pem"), files.key.clone());
        assert!(missing.load(&resolver).is_err());

        // Replaced before the watcher's first look: still picked up, as the
        // watcher compares with what was installed.
        let (second, _) = write(tmp.path(), "hub.example.test");
        let watch = files
            .clone()
            .watch(resolver.clone(), Duration::from_millis(20));
        until_served(&resolver, &second).await;

        // A broken replacement: the previous certificate stays.
        std::fs::write(&files.cert, "garbage").unwrap();
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert_eq!(served(&resolver), second);

        // Same size and modification time, other content: found by content.
        // (Text around the PEM blocks is ignored, so both pairs are padded
        // to the same length.)
        let padded = |pem: String| format!("{pem:\n<2048}");
        let ago = Timestamp::now() - jiff::SignedDuration::from_hours(1);
        let (third, cert, key) = pair(&["hub.example.test"], &[], ago);
        std::fs::write(&files.cert, padded(cert)).unwrap();
        std::fs::write(&files.key, padded(key)).unwrap();
        until_served(&resolver, &third).await;
        let stamp = |path: &Path| {
            let meta = std::fs::metadata(path).unwrap();
            (meta.len(), meta.modified().unwrap())
        };
        let before = (stamp(&files.cert), stamp(&files.key));
        let (fourth, cert, key) = pair(&["hub.example.test"], &[], ago);
        std::fs::write(&files.cert, padded(cert)).unwrap();
        std::fs::write(&files.key, padded(key)).unwrap();
        for (path, (_, modified)) in [(&files.cert, before.0), (&files.key, before.1)] {
            std::fs::File::options()
                .write(true)
                .open(path)
                .unwrap()
                .set_modified(modified)
                .unwrap();
        }
        assert_eq!((stamp(&files.cert), stamp(&files.key)), before);
        until_served(&resolver, &fourth).await;
        watch.abort();
    }
}
