//! `ROEDY_TLS=files`: the certificate chain and key from PEM files (any key
//! type rustls supports), checked at start – the start fails without a
//! usable pair – and reloaded when the files change: [`Files::watch`] looks
//! every [`POLL`] at their size and modification time. A changed pair that
//! cannot be used is logged and the previous certificate stays.

use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, SystemTime},
};

use anyhow::{Context as _, anyhow, bail};
use jiff::Timestamp;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, pem::PemObject as _};
use tokio::task::JoinHandle;

use super::{CertResolver, certified_key_from, x509::CertInfo};

/// How often the files are checked for changes.
pub const POLL: Duration = Duration::from_secs(10);

/// A certificate this close to its end is reported at every load.
const WARN_BEFORE: jiff::SignedDuration = jiff::SignedDuration::from_hours(14 * 24);

#[derive(Debug, Clone)]
pub struct Files {
    pub cert: PathBuf,
    pub key: PathBuf,
}

/// What identifies a version of the files.
type Stamp = Option<[(u64, Option<SystemTime>); 2]>;

impl Files {
    /// Loads the pair into `resolver`.
    pub fn load(&self, resolver: &CertResolver) -> anyhow::Result<CertInfo> {
        let (key, info) = read(&self.cert, &self.key)?;
        let host = resolver.host();
        if !info.dns_names.iter().any(|name| matches(name, host)) {
            bail!(
                "{} is not for {host} (names: {})",
                self.cert.display(),
                if info.dns_names.is_empty() {
                    "none".to_owned()
                } else {
                    info.dns_names.join(", ")
                }
            );
        }
        let now = Timestamp::now();
        if info.not_after <= now {
            bail!("{} expired on {}", self.cert.display(), info.not_after);
        }
        if info.not_after - WARN_BEFORE < now {
            tracing::warn!(not_after = %info.not_after, file = %self.cert.display(), "TLS certificate expires soon");
        }
        resolver.set(key);
        Ok(info)
    }

    fn stamp(&self) -> Stamp {
        let meta = |path: &Path| {
            let meta = std::fs::metadata(path).ok()?;
            Some((meta.len(), meta.modified().ok()))
        };
        Some([meta(&self.cert)?, meta(&self.key)?])
    }

    /// Reloads the pair whenever it changes, until the task is aborted.
    pub fn watch(self, resolver: Arc<CertResolver>, every: Duration) -> JoinHandle<()> {
        tokio::spawn(async move {
            let mut seen = self.stamp();
            loop {
                tokio::time::sleep(every).await;
                let stamp = self.stamp();
                if stamp == seen || stamp.is_none() {
                    // Unchanged, or missing for a moment while being replaced.
                    continue;
                }
                seen = stamp;
                match self.load(&resolver) {
                    Ok(info) => tracing::info!(
                        not_after = %info.not_after,
                        file = %self.cert.display(),
                        "TLS certificate reloaded"
                    ),
                    Err(error) => tracing::warn!(
                        "TLS certificate not reloaded, the previous one stays: {error:#}"
                    ),
                }
            }
        })
    }
}

/// Whether a certificate name covers `host` (a `*.` wildcard covers one
/// label).
fn matches(name: &str, host: &str) -> bool {
    let name = name.to_ascii_lowercase();
    match name.strip_prefix("*.") {
        Some(parent) => host
            .split_once('.')
            .is_some_and(|(label, rest)| !label.is_empty() && rest == parent),
        None => name == host,
    }
}

fn read(cert: &Path, key: &Path) -> anyhow::Result<(Arc<rustls::sign::CertifiedKey>, CertInfo)> {
    let chain = CertificateDer::pem_file_iter(cert)
        .with_context(|| format!("{} not readable", cert.display()))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| anyhow!("{}: not a PEM certificate chain: {e}", cert.display()))?;
    let leaf = chain
        .first()
        .ok_or_else(|| anyhow!("{} contains no certificate", cert.display()))?;
    let info = CertInfo::parse(leaf).with_context(|| format!("{}: certificate", cert.display()))?;
    let private = PrivateKeyDer::from_pem_file(key)
        .map_err(|e| anyhow!("{}: no PEM private key: {e}", key.display()))?;
    let certified = certified_key_from(chain, &private)
        .with_context(|| format!("{} and {}", cert.display(), key.display()))?;
    Ok((certified, info))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tls::{Challenges, key::P256Key, x509};

    #[test]
    fn wildcards_cover_one_label() {
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

    fn write_pair(dir: &Path, name: &str) -> (Vec<u8>, Files) {
        let key = P256Key::generate().unwrap();
        let der = x509::CertificateParams {
            serial: vec![1],
            issuer: x509::name(None),
            subject: x509::name(None),
            not_before: Timestamp::now() - jiff::SignedDuration::from_hours(1),
            not_after: Timestamp::now() + jiff::SignedDuration::from_hours(24 * 90),
            spki: key.spki(),
            extensions: vec![x509::Extension::dns_names(&[name])],
        }
        .sign(&key)
        .unwrap();
        let files = Files {
            cert: dir.join("cert.pem"),
            key: dir.join("key.pem"),
        };
        std::fs::write(&files.cert, pem("CERTIFICATE", &der)).unwrap();
        std::fs::write(&files.key, pem("PRIVATE KEY", key.pkcs8())).unwrap();
        (der, files)
    }

    fn served(resolver: &CertResolver) -> Vec<u8> {
        resolver.current().unwrap().cert[0].to_vec()
    }

    #[tokio::test]
    async fn loads_checks_and_reloads() {
        let tmp = tempfile::tempdir().unwrap();
        let resolver = Arc::new(CertResolver::new(
            "hub.example.test",
            Arc::new(Challenges::default()),
        ));
        let (first, files) = write_pair(tmp.path(), "hub.example.test");
        files.load(&resolver).unwrap();
        assert_eq!(served(&resolver), first);

        // Another name is refused, also at start.
        let other = tempfile::tempdir().unwrap();
        let (_, wrong) = write_pair(other.path(), "other.example.test");
        let err = wrong.load(&resolver).unwrap_err().to_string();
        assert!(err.contains("is not for hub.example.test"), "{err}");
        let missing = Files {
            cert: tmp.path().join("missing.pem"),
            key: files.key.clone(),
        };
        assert!(missing.load(&resolver).is_err());

        let watch = files
            .clone()
            .watch(resolver.clone(), Duration::from_millis(20));
        // A broken replacement: the previous certificate stays.
        std::fs::write(&files.cert, "garbage, and longer than before").unwrap();
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert_eq!(served(&resolver), first);
        // A new pair is served without a restart.
        let (second, _) = write_pair(tmp.path(), "hub.example.test");
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        while served(&resolver) != second {
            assert!(tokio::time::Instant::now() < deadline, "not reloaded");
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        watch.abort();
    }
}
