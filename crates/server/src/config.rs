//! Configuration via `ROEDY_*` environment variables (docs/ops.md ›
//! Configuration).
//!
//! Unknown `ROEDY_*` variables are ignored so that tools such as the Git hooks
//! (`ROEDY_PUSH_MAIN`) can use their own variables. Empty values count as unset
//! (e.g. `ROEDY_LOG=${ROEDY_LOG:-}` in Compose), except for
//! `ROEDY_INTERNAL_LISTEN`, where empty turns the internal listener off.
//! Values are text, without surrounding whitespace; each field parses what it
//! needs.
//!
//! Taken over from Roedy Mail (`crates/server/src/config.rs`): the same names
//! and meaning, without `ROEDY_GEOIP_DB`, plus `ROEDY_INTERNAL_LISTEN`.
//! `ROEDY_PUBLIC_URL` must not have a path, because the hub owns the root of
//! its origin.
//!
//! Built-in TLS (`ROEDY_TLS`, `tls`) is the hub's own: off by default (plain
//! HTTP as before, for development and setups behind a proxy), `acme` for
//! certificates from an ACME CA or `files` for a certificate from files.
//! With TLS on, `ROEDY_LISTEN` is the HTTPS listener and defaults to
//! `0.0.0.0:8443`, and `ROEDY_HTTP_LISTEN` serves the ACME HTTP-01
//! challenges and redirects everything else to `ROEDY_PUBLIC_URL`.

use std::{
    collections::BTreeMap,
    net::{IpAddr, Ipv4Addr, SocketAddr},
    path::PathBuf,
};

use anyhow::{anyhow, bail};
use figment::{
    Figment,
    providers::{Env, Serialized},
};
use serde::{Deserialize, Deserializer, de::Error as _};
use url::Url;

/// Prefix of all environment variables.
pub const ENV_PREFIX: &str = "ROEDY_";

/// Field of `ROEDY_INTERNAL_LISTEN`, whose empty value means "off".
const INTERNAL_LISTEN: &str = "internal_listen";

/// `ROEDY_LISTEN` with TLS on, unless set.
pub const TLS_LISTEN: SocketAddr = SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 8443);

/// Let's Encrypt's production directory, the default CA.
pub const LETS_ENCRYPT: &str = "https://acme-v02.api.letsencrypt.org/directory";

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Address of the public server (`ROEDY_LISTEN`): hub, API, health and
    /// the gateway; HTTPS with TLS on ([`TLS_LISTEN`] then by default).
    pub listen: SocketAddr,
    /// Address of the internal HTTP server for the apps (`ROEDY_INTERNAL_LISTEN`,
    /// `/internal/v1/…`); `None` when set to empty. Must never be published.
    #[serde(deserialize_with = "internal_listen")]
    pub internal_listen: Option<SocketAddr>,
    /// Directory for database and key (`ROEDY_DATA_DIR`).
    pub data_dir: PathBuf,
    /// Public address of the hub (`ROEDY_PUBLIC_URL`), the root of its origin.
    /// `serve` requires it ([`Self::require_public_url`]).
    #[serde(deserialize_with = "public_url")]
    pub public_url: Option<Url>,
    /// Log filter in the format of `tracing_subscriber::EnvFilter` (`ROEDY_LOG`).
    pub log: String,
    /// Log format (`ROEDY_LOG_FORMAT`).
    pub log_format: LogFormat,
    /// Directory with the web app build (`ROEDY_WEB_DIR`, e.g.
    /// `apps/web/build`). Takes precedence over the embedded web app.
    pub web_dir: Option<PathBuf>,
    /// Reverse proxies in front of the server (`ROEDY_TRUST_PROXY`): `false`,
    /// `true` (one proxy) or their number. Determines the client IP from
    /// `X-Forwarded-For`.
    #[serde(deserialize_with = "trust_proxy")]
    pub trust_proxy: TrustProxy,
    /// Key for secrets in the database (`ROEDY_SECRET_KEY`): 64 hex characters
    /// or Base64 for 32 bytes.
    pub secret_key: Option<Secret>,
    /// File with this key (`ROEDY_SECRET_KEY_FILE`, e.g. a Docker secret). If
    /// both are missing, `<ROEDY_DATA_DIR>/secret.key` applies.
    pub secret_key_file: Option<PathBuf>,
    /// Built-in TLS (`ROEDY_TLS`).
    pub tls: TlsMode,
    /// Plain HTTP with TLS on (`ROEDY_HTTP_LISTEN`): ACME HTTP-01 challenges
    /// and a redirect to `ROEDY_PUBLIC_URL`; unused with TLS off.
    pub http_listen: SocketAddr,
    /// The ACME CA's directory (`ROEDY_ACME_DIRECTORY`).
    #[serde(deserialize_with = "acme_directory")]
    pub acme_directory: Url,
    /// Contact address for the ACME account (`ROEDY_ACME_EMAIL`).
    pub acme_email: Option<String>,
    /// The operator accepts the CA's terms of service
    /// (`ROEDY_ACME_AGREE_TOS`); required for `acme`.
    #[serde(deserialize_with = "flag")]
    pub acme_agree_tos: bool,
    /// Certificate chain in PEM for `files` (`ROEDY_TLS_CERT_FILE`).
    pub tls_cert_file: Option<PathBuf>,
    /// Its private key in PEM for `files` (`ROEDY_TLS_KEY_FILE`).
    pub tls_key_file: Option<PathBuf>,
}

/// Built-in TLS.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TlsMode {
    /// Plain HTTP on `ROEDY_LISTEN` (default): development, or TLS in a
    /// proxy in front.
    #[default]
    Off,
    /// Certificates from an ACME CA (Let's Encrypt by default).
    Acme,
    /// A certificate from `ROEDY_TLS_CERT_FILE` and `ROEDY_TLS_KEY_FILE`,
    /// reloaded when they change.
    Files,
}

impl TlsMode {
    pub fn enabled(self) -> bool {
        self != Self::Off
    }
}

/// Value that never appears in `Debug` output or logs.
#[derive(Clone, PartialEq, Eq, Deserialize)]
#[serde(transparent)]
pub struct Secret(String);

impl Secret {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Secret(…)")
    }
}

/// How many reverse proxies append to `X-Forwarded-For`. With 0 only the
/// address of the TCP connection counts; otherwise the client is as many
/// entries before the end of the list as there are proxies in front.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TrustProxy(pub u8);

impl TrustProxy {
    /// At most this many proxies; more would almost certainly be a typo.
    pub const MAX: u8 = 10;

    pub fn hops(self) -> usize {
        usize::from(self.0)
    }

    pub fn enabled(self) -> bool {
        self.0 > 0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LogFormat {
    /// One JSON line per event (default, for container logs).
    Json,
    /// Readable text for development.
    Text,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            listen: SocketAddr::from(([0, 0, 0, 0], 8080)),
            internal_listen: Some(SocketAddr::from(([0, 0, 0, 0], 8081))),
            // Relative to the working directory; the image sets `ROEDY_DATA_DIR=/data`.
            data_dir: PathBuf::from("data"),
            public_url: None,
            log: "info".into(),
            log_format: LogFormat::Json,
            web_dir: None,
            trust_proxy: TrustProxy::default(),
            secret_key: None,
            secret_key_file: None,
            tls: TlsMode::Off,
            http_listen: SocketAddr::from(([0, 0, 0, 0], 8080)),
            acme_directory: Url::parse(LETS_ENCRYPT).unwrap_or_else(|_| unreachable!()),
            acme_email: None,
            acme_agree_tos: false,
            tls_cert_file: None,
            tls_key_file: None,
        }
    }
}

impl Config {
    /// Reads the configuration from the environment.
    pub fn load() -> anyhow::Result<Self> {
        Self::from_vars(
            Env::prefixed(ENV_PREFIX)
                .iter()
                .map(|(key, value)| (key.to_string(), value)),
        )
    }

    /// Reads the configuration from variables named without the prefix
    /// (`secret_key` for `ROEDY_SECRET_KEY`).
    ///
    /// The values go to figment as strings on purpose: its `Env` provider
    /// parses each value as data, so a key made only of digits would arrive as
    /// a number (without its leading zeros), `true` as a bool and `[a]` as an
    /// array, and text fields would reject them.
    fn from_vars(vars: impl IntoIterator<Item = (String, String)>) -> anyhow::Result<Self> {
        let vars: BTreeMap<String, String> = vars
            .into_iter()
            .filter_map(|(key, value)| {
                let key = key.to_ascii_lowercase();
                let value = value.trim();
                (!value.is_empty() || key == INTERNAL_LISTEN).then(|| (key, value.to_owned()))
            })
            .collect();
        Self::from_figment(Figment::from(Serialized::defaults(vars)))
    }

    pub fn from_figment(figment: Figment) -> anyhow::Result<Self> {
        let mut config: Self = figment.extract().map_err(|errors| {
            // Report each error with the full variable name, e.g. `ROEDY_LISTEN: …`.
            let details: Vec<String> = errors
                .into_iter()
                .map(|e| match e.path.join("_").to_uppercase() {
                    key if key.is_empty() => e.kind.to_string(),
                    key => format!("{ENV_PREFIX}{key}: {}", e.kind),
                })
                .collect();
            anyhow!("Invalid configuration – {}", details.join("; "))
        })?;
        if config.tls.enabled() && !figment.contains("listen") {
            config.listen = TLS_LISTEN;
        }
        config.check_tls()?;
        config.check_listeners()?;
        Ok(config)
    }

    /// The name the certificate is for: the host of `ROEDY_PUBLIC_URL`
    /// (checked by [`Self::check_tls`] when TLS is on).
    pub fn tls_host(&self) -> Option<String> {
        let url = self.public_url.as_ref()?;
        Some(match url.host()? {
            url::Host::Domain(domain) => domain.to_ascii_lowercase(),
            url::Host::Ipv4(ip) => ip.to_string(),
            url::Host::Ipv6(ip) => ip.to_string(),
        })
    }

    /// TLS needs an `https` public address; `acme` a DNS name and the
    /// operator's consent to the CA's terms, `files` both files.
    fn check_tls(&self) -> anyhow::Result<()> {
        if !self.tls.enabled() {
            return Ok(());
        }
        let invalid = |message: String| anyhow!("Invalid configuration – {message}");
        let url = self.public_url.as_ref().ok_or_else(|| {
            invalid(
                "ROEDY_PUBLIC_URL: required with ROEDY_TLS, e.g. https://app.example.com"
                    .to_owned(),
            )
        })?;
        if url.scheme() != "https" {
            return Err(invalid(format!(
                "ROEDY_PUBLIC_URL: must start with https:// when ROEDY_TLS is on (is {url})"
            )));
        }
        match self.tls {
            TlsMode::Off => {}
            TlsMode::Acme => {
                match url.host() {
                    Some(url::Host::Domain(domain)) => {
                        crate::tls::x509::check_dns_name(domain).map_err(|e| {
                            invalid(format!("ROEDY_PUBLIC_URL: {e} (ROEDY_TLS=acme)"))
                        })?;
                    }
                    _ => {
                        return Err(invalid(
                            "ROEDY_PUBLIC_URL: ROEDY_TLS=acme needs a DNS name, not an IP \
                             address"
                                .to_owned(),
                        ));
                    }
                }
                if !self.acme_agree_tos {
                    return Err(invalid(format!(
                        "ROEDY_ACME_AGREE_TOS: ROEDY_TLS=acme gets certificates from {} – read \
                         the CA's terms of service and set ROEDY_ACME_AGREE_TOS=true to accept \
                         them",
                        self.acme_directory
                    )));
                }
                if let Some(email) = &self.acme_email {
                    let plausible = email.split_once('@').is_some_and(|(local, domain)| {
                        !local.is_empty() && domain.contains('.') && !domain.contains('@')
                    }) && !email.chars().any(|c| {
                        c.is_whitespace() || c.is_control() || matches!(c, ',' | '<' | '>' | '?')
                    });
                    if !plausible {
                        return Err(invalid(format!(
                            "ROEDY_ACME_EMAIL: {email} is not an email address"
                        )));
                    }
                }
            }
            TlsMode::Files => {
                for (name, value) in [
                    ("ROEDY_TLS_CERT_FILE", &self.tls_cert_file),
                    ("ROEDY_TLS_KEY_FILE", &self.tls_key_file),
                ] {
                    if value.is_none() {
                        return Err(invalid(format!("{name}: required with ROEDY_TLS=files")));
                    }
                }
            }
        }
        Ok(())
    }

    /// The public address, which `serve` needs for links, the gateway's
    /// `X-Forwarded-*` headers and the apps' `aud`.
    pub fn require_public_url(&self) -> anyhow::Result<Url> {
        self.public_url.clone().ok_or_else(|| {
            anyhow!(
                "ROEDY_PUBLIC_URL is not set – set it to the public address of the hub, \
                 e.g. https://app.example.com (docs/ops.md › Configuration)"
            )
        })
    }

    /// Both listeners on the same port would fail at startup with "address in
    /// use"; this message says which variables to change.
    fn check_listeners(&self) -> anyhow::Result<()> {
        let overlaps = |a: SocketAddr, b: SocketAddr| {
            a.port() == b.port()
                && (a.ip() == b.ip() || a.ip().is_unspecified() || b.ip().is_unspecified())
        };
        if let Some(internal) = self.internal_listen {
            if overlaps(internal, self.listen) {
                bail!(
                    "Invalid configuration – ROEDY_INTERNAL_LISTEN: {internal} overlaps with \
                     ROEDY_LISTEN ({}); use another port, or set it to empty to turn the \
                     internal listener off",
                    self.listen
                );
            }
            if self.tls.enabled() && overlaps(internal, self.http_listen) {
                bail!(
                    "Invalid configuration – ROEDY_INTERNAL_LISTEN: {internal} overlaps with \
                     ROEDY_HTTP_LISTEN ({}); use another port, or set it to empty to turn the \
                     internal listener off",
                    self.http_listen
                );
            }
        }
        if self.tls.enabled() && overlaps(self.http_listen, self.listen) {
            bail!(
                "Invalid configuration – ROEDY_HTTP_LISTEN: {} overlaps with ROEDY_LISTEN ({}); \
                 with ROEDY_TLS on, ROEDY_LISTEN is the HTTPS port (default 0.0.0.0:8443)",
                self.http_listen,
                self.listen
            );
        }
        Ok(())
    }
}

fn internal_listen<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<SocketAddr>, D::Error> {
    let raw = String::deserialize(deserializer)?;
    let raw = raw.trim();
    if raw.is_empty() {
        return Ok(None);
    }
    raw.parse().map(Some).map_err(|_| {
        D::Error::custom(
            "expects an address with port, e.g. 0.0.0.0:8081, or empty to turn the internal \
             listener off",
        )
    })
}

fn public_url<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<Url>, D::Error> {
    let Some(raw) = Option::<String>::deserialize(deserializer)? else {
        return Ok(None);
    };
    let url =
        Url::parse(raw.trim()).map_err(|e| D::Error::custom(format!("not a valid URL: {e}")))?;
    if !matches!(url.scheme(), "http" | "https") || url.host().is_none() {
        return Err(D::Error::custom("expects an http(s) address with a host"));
    }
    if url.query().is_some() || url.fragment().is_some() {
        return Err(D::Error::custom("must contain neither query nor fragment"));
    }
    // Credentials have no place in links or in the log.
    if !url.username().is_empty() || url.password().is_some() {
        return Err(D::Error::custom("must not contain credentials"));
    }
    // The hub serves the root of its origin; the apps live under `/<slug>/`.
    if url.path() != "/" {
        return Err(D::Error::custom(
            "must not contain a path – the hub serves the root of its origin, \
             e.g. https://app.example.com",
        ));
    }
    Ok(Some(url))
}

fn acme_directory<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Url, D::Error> {
    let raw = String::deserialize(deserializer)?;
    let url =
        Url::parse(raw.trim()).map_err(|e| D::Error::custom(format!("not a valid URL: {e}")))?;
    // RFC 8555 § 6.1: HTTPS only; plain HTTP for a test server on loopback.
    let loopback = match url.host() {
        Some(url::Host::Domain(host)) => host == "localhost",
        Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
        Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
        None => false,
    };
    match url.scheme() {
        "https" if url.host().is_some() => Ok(url),
        "http" if loopback => Ok(url),
        _ => Err(D::Error::custom(
            "expects the https:// address of an ACME directory",
        )),
    }
}

/// `true`/`false` (also `yes`/`no`, `on`/`off`, `1`/`0`).
fn flag<'de, D: Deserializer<'de>>(deserializer: D) -> Result<bool, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Raw {
        Bool(bool),
        Text(String),
    }
    match Raw::deserialize(deserializer)? {
        Raw::Bool(value) => Ok(value),
        Raw::Text(text) => match text.trim().to_ascii_lowercase().as_str() {
            "true" | "yes" | "on" | "1" => Ok(true),
            "false" | "no" | "off" | "0" => Ok(false),
            _ => Err(D::Error::custom("expects true or false")),
        },
    }
}

fn trust_proxy<'de, D: Deserializer<'de>>(deserializer: D) -> Result<TrustProxy, D::Error> {
    // The environment delivers text; other figment providers may deliver
    // `true`/`2` as bool or number.
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Raw {
        Bool(bool),
        Hops(u64),
        Text(String),
    }
    let hops = match Raw::deserialize(deserializer)? {
        Raw::Bool(on) => u64::from(on),
        Raw::Hops(hops) => hops,
        Raw::Text(text) => match text.trim().to_ascii_lowercase().as_str() {
            "true" | "yes" | "on" => 1,
            "false" | "no" | "off" => 0,
            other => other
                .parse()
                .map_err(|_| D::Error::custom("expects true, false or the number of proxies"))?,
        },
    };
    u8::try_from(hops)
        .ok()
        .filter(|&hops| hops <= TrustProxy::MAX)
        .map(TrustProxy)
        .ok_or_else(|| {
            D::Error::custom(format!(
                "at most {} proxies (expects true, false or a number)",
                TrustProxy::MAX
            ))
        })
}

#[cfg(test)]
#[allow(
    clippy::result_large_err,
    reason = "figment::Jail requires closures returning figment::Result"
)]
mod tests {
    use super::*;
    use figment::Jail;

    /// Every variable the hub reads.
    const ALL: [&str; 17] = [
        "ROEDY_LISTEN",
        "ROEDY_INTERNAL_LISTEN",
        "ROEDY_DATA_DIR",
        "ROEDY_PUBLIC_URL",
        "ROEDY_LOG",
        "ROEDY_LOG_FORMAT",
        "ROEDY_WEB_DIR",
        "ROEDY_TRUST_PROXY",
        "ROEDY_SECRET_KEY",
        "ROEDY_SECRET_KEY_FILE",
        "ROEDY_TLS",
        "ROEDY_HTTP_LISTEN",
        "ROEDY_ACME_DIRECTORY",
        "ROEDY_ACME_EMAIL",
        "ROEDY_ACME_AGREE_TOS",
        "ROEDY_TLS_CERT_FILE",
        "ROEDY_TLS_KEY_FILE",
    ];

    #[test]
    fn defaults_without_environment() {
        let config = Config::from_figment(Figment::new()).unwrap();
        assert_eq!(config, Config::default());
        assert_eq!(config.listen.to_string(), "0.0.0.0:8080");
        assert_eq!(
            config.internal_listen.map(|a| a.to_string()).as_deref(),
            Some("0.0.0.0:8081")
        );
        assert_eq!(config.data_dir, PathBuf::from("data"));
        assert_eq!(config.public_url, None);
        assert_eq!(config.log, "info");
        assert_eq!(config.log_format, LogFormat::Json);
        assert_eq!(config.web_dir, None);
        assert_eq!(config.trust_proxy, TrustProxy(0));
        assert_eq!(config.secret_key, None);
        assert_eq!(config.secret_key_file, None);
        assert_eq!(config.tls, TlsMode::Off);
        assert_eq!(config.http_listen.to_string(), "0.0.0.0:8080");
        assert_eq!(config.acme_directory.as_str(), LETS_ENCRYPT);
        assert_eq!(config.acme_email, None);
        assert!(!config.acme_agree_tos);
        assert_eq!(config.tls_cert_file, None);
        assert_eq!(config.tls_key_file, None);
    }

    fn vars(pairs: &[(&str, &str)]) -> anyhow::Result<Config> {
        Config::from_vars(
            pairs
                .iter()
                .map(|(key, value)| ((*key).to_owned(), (*value).to_owned())),
        )
    }

    const ACME: [(&str, &str); 3] = [
        ("tls", "acme"),
        ("public_url", "https://hub.example.com"),
        ("acme_agree_tos", "true"),
    ];

    #[test]
    fn tls_moves_the_public_listener_to_8443() {
        let config = vars(&ACME).unwrap();
        assert_eq!(config.tls, TlsMode::Acme);
        assert_eq!(config.listen, TLS_LISTEN);
        assert_eq!(config.listen.to_string(), "0.0.0.0:8443");
        assert_eq!(config.http_listen.to_string(), "0.0.0.0:8080");
        assert_eq!(config.tls_host().as_deref(), Some("hub.example.com"));

        // An explicit ROEDY_LISTEN stays, also on the default port.
        let mut explicit = ACME.to_vec();
        explicit.push(("listen", "0.0.0.0:8080"));
        explicit.push(("http_listen", "0.0.0.0:8000"));
        let config = vars(&explicit).unwrap();
        assert_eq!(config.listen.to_string(), "0.0.0.0:8080");
        assert_eq!(config.http_listen.to_string(), "0.0.0.0:8000");

        // Off: ROEDY_LISTEN keeps 8080, ROEDY_HTTP_LISTEN is not used (and
        // not checked for overlaps).
        let config = vars(&[("tls", "off"), ("http_listen", "0.0.0.0:8080")]).unwrap();
        assert_eq!(config.listen.to_string(), "0.0.0.0:8080");
    }

    #[test]
    fn acme_settings() {
        let mut all = ACME.to_vec();
        all.extend([
            (
                "acme_directory",
                "https://acme-staging-v02.api.letsencrypt.org/directory",
            ),
            ("acme_email", "ops@example.com"),
        ]);
        let config = vars(&all).unwrap();
        assert_eq!(
            config.acme_directory.as_str(),
            "https://acme-staging-v02.api.letsencrypt.org/directory"
        );
        assert_eq!(config.acme_email.as_deref(), Some("ops@example.com"));
        assert!(config.acme_agree_tos);
        for (value, agreed) in [("TRUE", true), ("yes", true), ("1", true), ("off", false)] {
            let mut flag = vec![("tls", "off"), ("acme_agree_tos", value)];
            flag.push(("public_url", "https://hub.example.com"));
            assert_eq!(vars(&flag).unwrap().acme_agree_tos, agreed, "{value}");
        }
        // A test CA on loopback may speak plain HTTP.
        for directory in [
            "http://127.0.0.1:14000/dir",
            "http://localhost:14000/dir",
            "http://[::1]:1/d",
        ] {
            let mut test = ACME.to_vec();
            test.push(("acme_directory", directory));
            vars(&test).unwrap();
        }
    }

    #[test]
    fn files_settings() {
        let config = vars(&[
            ("tls", "files"),
            ("public_url", "https://192.0.2.7"),
            ("tls_cert_file", "/run/secrets/cert.pem"),
            ("tls_key_file", "/run/secrets/key.pem"),
        ])
        .unwrap();
        assert_eq!(config.tls, TlsMode::Files);
        assert_eq!(config.tls_host().as_deref(), Some("192.0.2.7"));
        assert_eq!(
            config.tls_cert_file,
            Some(PathBuf::from("/run/secrets/cert.pem"))
        );
    }

    #[test]
    fn rejects_incomplete_tls_settings() {
        for (pairs, message) in [
            (
                vec![("tls", "acme"), ("acme_agree_tos", "true")],
                "ROEDY_PUBLIC_URL: required with ROEDY_TLS",
            ),
            (
                vec![
                    ("tls", "acme"),
                    ("acme_agree_tos", "true"),
                    ("public_url", "http://hub.example.com"),
                ],
                "ROEDY_PUBLIC_URL: must start with https:// when ROEDY_TLS is on",
            ),
            (
                vec![("tls", "acme"), ("public_url", "https://hub.example.com")],
                "ROEDY_ACME_AGREE_TOS: ROEDY_TLS=acme gets certificates from \
                 https://acme-v02.api.letsencrypt.org/directory – read the CA's terms of service \
                 and set ROEDY_ACME_AGREE_TOS=true to accept them",
            ),
            (
                vec![
                    ("tls", "acme"),
                    ("public_url", "https://hub.example.com"),
                    ("acme_agree_tos", "false"),
                ],
                "ROEDY_ACME_AGREE_TOS: ROEDY_TLS=acme gets certificates",
            ),
            (
                vec![
                    ("tls", "acme"),
                    ("acme_agree_tos", "true"),
                    ("public_url", "https://192.0.2.7"),
                ],
                "ROEDY_PUBLIC_URL: ROEDY_TLS=acme needs a DNS name",
            ),
            (
                vec![
                    ("tls", "acme"),
                    ("acme_agree_tos", "true"),
                    ("public_url", "https://localhost"),
                ],
                "ROEDY_PUBLIC_URL: localhost is not a DNS name",
            ),
            (
                vec![
                    ("tls", "acme"),
                    ("acme_agree_tos", "true"),
                    ("public_url", "https://hub.example.com"),
                    ("acme_email", "not an address"),
                ],
                "ROEDY_ACME_EMAIL: not an address is not an email address",
            ),
            (
                vec![
                    ("tls", "files"),
                    ("public_url", "https://hub.example.com"),
                    ("tls_key_file", "/k.pem"),
                ],
                "ROEDY_TLS_CERT_FILE: required with ROEDY_TLS=files",
            ),
            (
                vec![
                    ("tls", "files"),
                    ("public_url", "https://hub.example.com"),
                    ("tls_cert_file", "/c.pem"),
                ],
                "ROEDY_TLS_KEY_FILE: required with ROEDY_TLS=files",
            ),
            (vec![("tls", "on")], "ROEDY_TLS: unknown variant"),
            (
                vec![("acme_agree_tos", "maybe")],
                "ROEDY_ACME_AGREE_TOS: expects true or false",
            ),
            (
                vec![("acme_directory", "http://ca.example.com/dir")],
                "ROEDY_ACME_DIRECTORY: expects the https:// address of an ACME directory",
            ),
        ] {
            let err = vars(&pairs).unwrap_err().to_string();
            assert!(
                err.starts_with("Invalid configuration – "),
                "{pairs:?}: {err}"
            );
            assert!(err.contains(message), "{pairs:?}: {err}");
        }
    }

    #[test]
    fn rejects_overlapping_tls_listeners() {
        let mut same = ACME.to_vec();
        same.push(("http_listen", "0.0.0.0:8443"));
        assert_eq!(
            vars(&same).unwrap_err().to_string(),
            "Invalid configuration – ROEDY_HTTP_LISTEN: 0.0.0.0:8443 overlaps with ROEDY_LISTEN \
             (0.0.0.0:8443); with ROEDY_TLS on, ROEDY_LISTEN is the HTTPS port (default \
             0.0.0.0:8443)"
        );
        let mut internal = ACME.to_vec();
        internal.push(("internal_listen", "0.0.0.0:8080"));
        assert_eq!(
            vars(&internal).unwrap_err().to_string(),
            "Invalid configuration – ROEDY_INTERNAL_LISTEN: 0.0.0.0:8080 overlaps with \
             ROEDY_HTTP_LISTEN (0.0.0.0:8080); use another port, or set it to empty to turn the \
             internal listener off"
        );
    }

    #[test]
    fn reads_prefixed_environment() {
        Jail::expect_with(|jail| {
            jail.set_env("ROEDY_LISTEN", "127.0.0.1:9000");
            jail.set_env("ROEDY_INTERNAL_LISTEN", "10.0.0.5:9001");
            jail.set_env("ROEDY_DATA_DIR", "/srv/roedy");
            jail.set_env("ROEDY_PUBLIC_URL", "https://app.example.com");
            jail.set_env("ROEDY_LOG", "debug,tower_http=trace");
            jail.set_env("ROEDY_LOG_FORMAT", "text");
            jail.set_env("ROEDY_WEB_DIR", "apps/web/build");
            jail.set_env("ROEDY_TRUST_PROXY", "true");
            jail.set_env("ROEDY_SECRET_KEY", "abc");
            jail.set_env("ROEDY_SECRET_KEY_FILE", "/run/secrets/roedy");
            jail.set_env("ROEDY_PUSH_MAIN", "1");

            let config = Config::load().unwrap();
            assert_eq!(config.listen.to_string(), "127.0.0.1:9000");
            assert_eq!(
                config.internal_listen.map(|a| a.to_string()).as_deref(),
                Some("10.0.0.5:9001")
            );
            assert_eq!(config.data_dir, PathBuf::from("/srv/roedy"));
            assert_eq!(
                config.public_url.as_ref().map(Url::as_str),
                Some("https://app.example.com/")
            );
            assert_eq!(config.log, "debug,tower_http=trace");
            assert_eq!(config.log_format, LogFormat::Text);
            assert_eq!(config.web_dir, Some(PathBuf::from("apps/web/build")));
            assert_eq!(config.trust_proxy, TrustProxy(1));
            assert_eq!(config.secret_key, Some(Secret::new("abc")));
            assert_eq!(format!("{:?}", config.secret_key), "Some(Secret(…))");
            assert_eq!(
                config.secret_key_file,
                Some(PathBuf::from("/run/secrets/roedy"))
            );
            Ok(())
        });
    }

    #[test]
    fn empty_values_are_unset_except_the_internal_listener() {
        Jail::expect_with(|jail| {
            for key in ALL {
                jail.set_env(key, "  ");
            }
            assert_eq!(
                Config::load().unwrap(),
                Config {
                    internal_listen: None,
                    ..Config::default()
                }
            );
            Ok(())
        });
        Jail::expect_with(|jail| {
            jail.set_env("ROEDY_INTERNAL_LISTEN", "");
            assert_eq!(Config::load().unwrap().internal_listen, None);
            Ok(())
        });
    }

    #[test]
    fn internal_listener_may_share_the_port_on_another_address() {
        let config = Config::from_vars([
            ("listen".to_owned(), "127.0.0.1:8080".to_owned()),
            ("internal_listen".to_owned(), "10.0.0.5:8080".to_owned()),
        ])
        .unwrap();
        assert_eq!(config.internal_listen.unwrap().port(), 8080);
        // Off: then the port does not matter.
        let config = Config::from_vars([
            ("listen".to_owned(), "0.0.0.0:8081".to_owned()),
            ("internal_listen".to_owned(), String::new()),
        ])
        .unwrap();
        assert_eq!(config.internal_listen, None);
    }

    #[test]
    fn rejects_overlapping_listeners() {
        for (listen, internal) in [
            ("0.0.0.0:8080", "0.0.0.0:8080"),
            ("0.0.0.0:8080", "10.0.0.5:8080"),
            ("127.0.0.1:9000", "[::]:9000"),
            ("0.0.0.0:8081", ""),
        ] {
            let vars = [
                ("listen".to_owned(), listen.to_owned()),
                ("internal_listen".to_owned(), internal.to_owned()),
            ];
            let vars: Vec<_> = vars.into_iter().filter(|(_, v)| !v.is_empty()).collect();
            let err = Config::from_vars(vars).unwrap_err().to_string();
            assert_eq!(
                err,
                format!(
                    "Invalid configuration – ROEDY_INTERNAL_LISTEN: {} overlaps with \
                     ROEDY_LISTEN ({listen}); use another port, or set it to empty to turn the \
                     internal listener off",
                    if internal.is_empty() {
                        "0.0.0.0:8081"
                    } else {
                        internal
                    }
                ),
                "{listen} / {internal}"
            );
        }
    }

    #[test]
    fn secret_key_made_only_of_digits_is_text() {
        // Regression from Roedy Mail: the key arrived as the number 0 and was
        // rejected.
        let digits = "0".repeat(64);
        Jail::expect_with(|jail| {
            jail.set_env("ROEDY_SECRET_KEY", &digits);
            let config = Config::load().unwrap();
            assert_eq!(config.secret_key, Some(Secret::new(digits.clone())));
            let (_, source) = crate::crypto::SecretKey::load(&config).unwrap();
            assert_eq!(source, crate::crypto::KeySource::Env);
            Ok(())
        });
    }

    /// Values that look like a number, a bool or structured data.
    const LOOKALIKES: [&str; 14] = [
        "0",
        "007",
        "42",
        "-1",
        "1.50",
        "1e3",
        "inf",
        "true",
        "false",
        "[a]",
        "[1, 2]",
        "{a=1}",
        "\"quoted\"",
        "a=b",
    ];

    #[test]
    fn text_settings_are_read_verbatim() {
        for value in LOOKALIKES {
            Jail::expect_with(|jail| {
                for key in [
                    "ROEDY_DATA_DIR",
                    "ROEDY_LOG",
                    "ROEDY_WEB_DIR",
                    "ROEDY_SECRET_KEY",
                ] {
                    jail.set_env(key, value);
                }
                let config = Config::load().unwrap_or_else(|e| panic!("{value}: {e}"));
                assert_eq!(config.data_dir, PathBuf::from(value), "{value}");
                assert_eq!(config.log, value);
                assert_eq!(config.web_dir, Some(PathBuf::from(value)), "{value}");
                assert_eq!(config.secret_key, Some(Secret::new(value)), "{value}");
                Ok(())
            });
            // Separately: both key sources together are rejected later on.
            Jail::expect_with(|jail| {
                jail.set_env("ROEDY_SECRET_KEY_FILE", value);
                let config = Config::load().unwrap_or_else(|e| panic!("{value}: {e}"));
                assert_eq!(config.secret_key_file, Some(PathBuf::from(value)));
                Ok(())
            });
        }
    }

    /// Property: every text setting keeps any value as text, without the
    /// surrounding whitespace. Random values from a fixed seed (no property
    /// test crate in the workspace), weighted towards digits and the syntax of
    /// figment's value parser.
    #[test]
    fn text_settings_keep_any_value() {
        const CHARS: &[u8] = b"0123456789000000abcdefABCDEF+/=-_.,:[]{}\"' \txyz";
        let mut state: u64 = 0x5EED_0019;
        let mut next = |bound: usize| {
            // xorshift64
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            usize::try_from(state % bound as u64).unwrap()
        };
        for _ in 0..2_000 {
            let len = next(70);
            let raw: String = (0..len)
                .map(|_| char::from(CHARS[next(CHARS.len())]))
                .collect();
            let vars = ["data_dir", "log", "web_dir", "secret_key"]
                .map(|key| (key.to_owned(), raw.clone()));
            let config = Config::from_vars(vars).unwrap_or_else(|e| panic!("{raw:?}: {e}"));
            let value = raw.trim();
            if value.is_empty() {
                assert_eq!(config, Config::default(), "{raw:?}");
                continue;
            }
            assert_eq!(config.data_dir, PathBuf::from(value), "{raw:?}");
            assert_eq!(config.log, value, "{raw:?}");
            assert_eq!(config.web_dir, Some(PathBuf::from(value)), "{raw:?}");
            assert_eq!(config.secret_key, Some(Secret::new(value)), "{raw:?}");
        }
    }

    #[test]
    fn trust_proxy_accepts_bool_and_count() {
        for (value, hops) in [
            ("false", 0),
            ("true", 1),
            ("TRUE", 1),
            ("yes", 1),
            ("on", 1),
            ("off", 0),
            ("no", 0),
            ("0", 0),
            ("2", 2),
            ("10", 10),
        ] {
            Jail::expect_with(|jail| {
                jail.set_env("ROEDY_TRUST_PROXY", value);
                assert_eq!(
                    Config::load().unwrap().trust_proxy,
                    TrustProxy(hops),
                    "{value}"
                );
                Ok(())
            });
        }
    }

    #[test]
    fn public_url_is_the_root_of_an_origin() {
        for (value, stored) in [
            ("https://app.example.com", "https://app.example.com/"),
            ("https://app.example.com/", "https://app.example.com/"),
            ("http://localhost:5173", "http://localhost:5173/"),
            (
                "  https://APP.example.com:8443 ",
                "https://app.example.com:8443/",
            ),
        ] {
            Jail::expect_with(|jail| {
                jail.set_env("ROEDY_PUBLIC_URL", value);
                let config = Config::load().unwrap();
                assert_eq!(config.require_public_url().unwrap().as_str(), stored);
                Ok(())
            });
        }
    }

    #[test]
    fn serve_requires_the_public_url() {
        let err = Config::default().require_public_url().unwrap_err();
        assert_eq!(
            err.to_string(),
            "ROEDY_PUBLIC_URL is not set – set it to the public address of the hub, \
             e.g. https://app.example.com (docs/ops.md › Configuration)"
        );
    }

    #[test]
    fn rejects_invalid_values_naming_the_variable() {
        for (key, value, message) in [
            (
                "ROEDY_LISTEN",
                "localhost",
                "ROEDY_LISTEN: invalid socket address syntax",
            ),
            (
                "ROEDY_INTERNAL_LISTEN",
                "8081",
                "ROEDY_INTERNAL_LISTEN: expects an address with port, e.g. 0.0.0.0:8081, or \
                 empty to turn the internal listener off",
            ),
            (
                "ROEDY_PUBLIC_URL",
                "app.example.com",
                "ROEDY_PUBLIC_URL: not a valid URL: relative URL without a base",
            ),
            (
                "ROEDY_PUBLIC_URL",
                "ftp://app.example.com",
                "ROEDY_PUBLIC_URL: expects an http(s) address with a host",
            ),
            (
                "ROEDY_PUBLIC_URL",
                "https://app.example.com/?a=1",
                "ROEDY_PUBLIC_URL: must contain neither query nor fragment",
            ),
            (
                "ROEDY_PUBLIC_URL",
                "https://app.example.com/#top",
                "ROEDY_PUBLIC_URL: must contain neither query nor fragment",
            ),
            (
                "ROEDY_PUBLIC_URL",
                "https://admin:secret@app.example.com",
                "ROEDY_PUBLIC_URL: must not contain credentials",
            ),
            (
                "ROEDY_PUBLIC_URL",
                "https://example.com/hub",
                "ROEDY_PUBLIC_URL: must not contain a path – the hub serves the root of its \
                 origin, e.g. https://app.example.com",
            ),
            (
                "ROEDY_PUBLIC_URL",
                "https://example.com/hub/",
                "ROEDY_PUBLIC_URL: must not contain a path",
            ),
            (
                "ROEDY_LOG_FORMAT",
                "xml",
                "ROEDY_LOG_FORMAT: unknown variant",
            ),
            (
                "ROEDY_TRUST_PROXY",
                "maybe",
                "ROEDY_TRUST_PROXY: expects true, false or the number of proxies",
            ),
            (
                "ROEDY_TRUST_PROXY",
                "11",
                "ROEDY_TRUST_PROXY: at most 10 proxies (expects true, false or a number)",
            ),
            (
                "ROEDY_TRUST_PROXY",
                "-1",
                "ROEDY_TRUST_PROXY: expects true, false or the number of proxies",
            ),
        ] {
            Jail::expect_with(|jail| {
                jail.set_env(key, value);
                let err = Config::load().unwrap_err().to_string();
                assert!(
                    err.starts_with("Invalid configuration – "),
                    "{key}={value}: {err}"
                );
                assert!(err.contains(message), "{key}={value}: {err}");
                Ok(())
            });
        }
    }
}
