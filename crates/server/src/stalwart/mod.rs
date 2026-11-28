//! Users' Stalwart accounts and the servers they live on (`docs/plan.md`
//! § 9.3): diagnosis of a server ([`probe`]), registration of the hub's OAuth
//! client by DCR ([`dcr`]), the token endpoint and revocation ([`oauth`]),
//! which account a token belongs to ([`account`]), protection against
//! requests to internal networks for custom servers ([`outbound`]) and the
//! token broker ([`token`]).
//!
//! Taken over from Roedy Mail (`crates/server/src/broker`), where it served
//! one app; here every app on the hub's origin uses the same accounts.

pub mod account;
pub mod dcr;
pub mod oauth;
pub mod outbound;
pub mod probe;
pub mod token;
pub mod view;

use std::time::Duration;

use anyhow::Context as _;
// As in the JMAP client: credentials only to the same origin.
use roedy_jmap::same_origin;
use serde::{Deserialize, de::DeserializeOwned};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const TIMEOUT: Duration = Duration::from_secs(10);

/// Scopes of the hub's client and of every sign-in, with Stalwart's names:
/// one grant per person and server serves Mail, Calendar and Contacts, so it
/// asks for all of them. Without `scope` a DCR client would get none.
pub const SCOPE: &str = "offline_access urn:ietf:params:oauth:scope:mail \
                         urn:ietf:params:oauth:scope:contacts \
                         urn:ietf:params:oauth:scope:calendars";

/// Path of the redirect URI on the hub's origin (a page of the hub's web
/// app; `oauth` is a reserved first segment, contract § 3.1).
pub const CALLBACK_PATH: &str = "/oauth/callback";

/// The redirect URI of the hub's client: `<public URL>/oauth/callback`.
pub fn redirect_uri(public_url: &url::Url) -> String {
    format!(
        "{}{CALLBACK_PATH}",
        public_url.as_str().trim_end_matches('/')
    )
}

/// What the hub needs at run time for Stalwart (`AppState::stalwart`).
pub struct Stalwart {
    /// Client for registry servers ([`http_client`]).
    pub http: reqwest::Client,
    /// Client for custom servers: public addresses only
    /// ([`outbound::public_client`]).
    pub public_http: reqwest::Client,
    /// One refresh per account at a time ([`token`]); under `user:<id>`
    /// also adding and signing in again per user (`api::stalwart`).
    pub token_locks: std::sync::Arc<crate::locks::KeyedLocks>,
}

impl Stalwart {
    pub fn new() -> anyhow::Result<Self> {
        Ok(Self {
            http: http_client()?,
            public_http: outbound::public_client()?,
            token_locks: std::sync::Arc::default(),
        })
    }

    /// The client for an account's server: a registry server or a custom
    /// one.
    pub fn http(&self, registered: bool) -> &reqwest::Client {
        if registered {
            &self.http
        } else {
            &self.public_http
        }
    }
}

/// HTTP client for requests to registry servers. Follows no redirects –
/// where they are allowed, the caller follows them itself so that credentials
/// never go to another address.
pub fn http_client() -> anyhow::Result<reqwest::Client> {
    client_builder().build().context("HTTP client not created")
}

fn client_builder() -> reqwest::ClientBuilder {
    // With `rustls-no-provider` reqwest brings no cryptography; ring
    // applies to the whole process. If a provider is already set, it stays.
    let _ = rustls::crypto::ring::default_provider().install_default();
    reqwest::Client::builder()
        .user_agent(concat!("Roedy-Hub/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(CONNECT_TIMEOUT)
        .timeout(TIMEOUT)
        .redirect(reqwest::redirect::Policy::none())
}

/// Reads at most 256 KiB as JSON ([`roedy_jmap::read_json`]); diagnosis
/// and DCR do not need the reason.
async fn read_json<T: DeserializeOwned>(response: reqwest::Response) -> Option<T> {
    roedy_jmap::read_json(response).await.ok()
}

#[derive(Deserialize)]
struct ErrorResponse {
    error: String,
}

/// Whether `code` looks like an OAuth error code (RFC 6749, appendix A.7,
/// limited to what servers send): nothing arbitrary gets into the log or
/// the API.
pub fn is_error_code(code: &str) -> bool {
    !code.is_empty()
        && code.len() <= 64
        && code.bytes().all(|b| b.is_ascii_lowercase() || b == b'_')
}

/// Error code of an OAuth response (`{"error": …}`, RFC 6749 and RFC 7591).
async fn oauth_error(response: reqwest::Response) -> Option<String> {
    read_json::<ErrorResponse>(response)
        .await
        .map(|e| e.error)
        .filter(|code| is_error_code(code))
}

/// Base URL of a registry server: `https://` (for development also `http://`
/// on `localhost` or `127.0.0.1`), host as DNS name or IPv4 address, without
/// credentials, path, query and fragment. The result is the origin. The
/// CSP's `connect-src` cannot express IPv6 addresses. Taken over from Roedy
/// Mail (`api::servers::normalize_url`).
pub fn normalize_url(raw: &str) -> Result<url::Url, &'static str> {
    parse_origin(raw, true)
}

/// Address of a custom server ("Other server …"): like [`normalize_url`],
/// but only `https://` – without the exception for development, which only
/// administrators get. [`outbound`] checks whether the address is public.
pub fn normalize_custom_url(raw: &str) -> Result<url::Url, &'static str> {
    parse_origin(raw, false)
}

fn parse_origin(raw: &str, dev_loopback: bool) -> Result<url::Url, &'static str> {
    use url::{Host, Url};
    let raw = raw.trim();
    if raw.is_empty() {
        return Err("required");
    }
    let url = Url::parse(raw.trim_end_matches('/')).map_err(|_| "invalid")?;
    let loopback = match url.host() {
        // Only what the CSP's host source can express: labels of `a–z`,
        // `0–9`, `-`, no empty ones (so no trailing dot either).
        Some(Host::Domain(domain)) => {
            if !domain.split('.').all(is_dns_label) {
                return Err("invalid");
            }
            domain == "localhost"
        }
        // Exactly the addresses the page names, not all of 127/8.
        Some(Host::Ipv4(ip)) => ip == std::net::Ipv4Addr::LOCALHOST,
        Some(Host::Ipv6(_)) | None => return Err("invalid"),
    };
    let scheme_ok = url.scheme() == "https" || (dev_loopback && url.scheme() == "http" && loopback);
    let bare = url.username().is_empty()
        && url.password().is_none()
        && url.path() == "/"
        && url.query().is_none()
        && url.fragment().is_none();
    if !scheme_ok || !bare {
        return Err("invalid");
    }
    Url::parse(&url.origin().ascii_serialization()).map_err(|_| "invalid")
}

/// A label of a DNS name: `a–z`, `0–9`, `-`, not empty, no `-` at the edges.
pub fn is_dns_label(label: &str) -> bool {
    !label.is_empty()
        && label.len() <= 63
        && !label.starts_with('-')
        && !label.ends_with('-')
        && label
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

/// Whether an account without a registry server is usable as a custom
/// server: policy on and an address that would pass as a custom server
/// (`https://`, no internal IP address) – e.g. not `http://localhost` of a
/// removed development server. Internal DNS names are only detected by the
/// connection (`AppState::stalwart_public_http`).
fn usable_as_custom(base_url: &str, allow_custom: bool) -> bool {
    use url::Host;
    allow_custom
        && normalize_custom_url(base_url).is_ok_and(|url| match url.host() {
            Some(Host::Ipv4(ip)) => outbound::is_public(std::net::IpAddr::V4(ip)),
            _ => true,
        })
}

/// An account without a registry server that is not usable as a custom
/// server: no tokens, no signing in again (`blocked`).
pub fn blocked(base_url: &str, registered: bool, allow_custom: bool) -> bool {
    !registered && !usable_as_custom(base_url, allow_custom)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_urls() {
        for (raw, expected) in [
            ("https://mail.s1.test", "https://mail.s1.test/"),
            ("  https://Mail.S1.test/  ", "https://mail.s1.test/"),
            ("https://mail.s1.test:443", "https://mail.s1.test/"),
            ("https://mail.s1.test:8443", "https://mail.s1.test:8443/"),
            ("https://café.example", "https://xn--caf-dma.example/"),
            ("http://localhost:8080", "http://localhost:8080/"),
            ("http://127.0.0.1:8080/", "http://127.0.0.1:8080/"),
        ] {
            assert_eq!(
                normalize_url(raw).map(String::from),
                Ok(expected.into()),
                "{raw}"
            );
        }
        for raw in [
            "mail.s1.test",
            "http://mail.s1.test",
            "https://mail.s1.test/jmap",
            "https://user@mail.s1.test",
            "https://[2001:db8::1]",
            "https://mail.s1.test?x=1",
            "http://127.0.0.2",
        ] {
            assert_eq!(normalize_url(raw), Err("invalid"), "{raw}");
        }
        assert_eq!(normalize_url(" "), Err("required"));
        assert_eq!(
            normalize_custom_url("http://localhost:8080"),
            Err("invalid")
        );
        assert!(normalize_custom_url("https://mail.example.com").is_ok());
    }

    #[test]
    fn accounts_off_the_registry_need_the_policy_and_a_public_address() {
        assert!(!blocked("http://localhost:8080", true, false));
        assert!(blocked("https://mail.example.com", false, false));
        assert!(!blocked("https://mail.example.com", false, true));
        assert!(blocked("http://localhost:8080", false, true));
        assert!(blocked("https://10.0.0.1", false, true));
    }
}
