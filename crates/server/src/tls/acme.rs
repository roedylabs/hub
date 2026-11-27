//! The hub's own ACME client (RFC 8555), as far as one certificate for one
//! DNS name needs it: directory, nonces, account, order, authorizations
//! with HTTP-01 or TLS-ALPN-01 (RFC 8737), finalize, download, and ARI
//! renewal information (RFC 9773). Requests are JWS with the account key
//! ([`jose`]); every response's `Replay-Nonce` is kept for the next request.
//!
//! Errors from the CA are problem documents ([`Problem`]); a `badNonce` is
//! retried at once with the nonce that came with it (§ 6.5), everything else
//! goes to the caller, which waits as long as `Retry-After` says or its own
//! backoff (`tls::issuer`).

use std::{fmt, time::Duration};

use anyhow::{Context as _, anyhow, bail};
use jiff::Timestamp;
use reqwest::{
    StatusCode,
    header::{self, HeaderMap},
};
use serde::Deserialize;
use serde_json::{Value, json};

use super::{
    jose::{self, Signer},
    key::P256Key,
};

/// How often a request is sent again after `badNonce`.
const BAD_NONCE_RETRIES: usize = 5;
/// Waiting between polls when the CA sends no `Retry-After`.
const POLL_DEFAULT: Duration = Duration::from_secs(2);
/// Longest wait between two polls, whatever `Retry-After` says.
const POLL_MAX: Duration = Duration::from_secs(60);
/// How long an authorization or order may stay pending or processing.
const POLL_DEADLINE: Duration = Duration::from_secs(5 * 60);
/// Largest response body read from the CA.
const MAX_BODY: usize = 1024 * 1024;

pub const HTTP_01: &str = "http-01";
pub const TLS_ALPN_01: &str = "tls-alpn-01";

/// An error document from the CA (RFC 8555 § 6.7, RFC 7807).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Problem {
    /// The part after `urn:ietf:params:acme:error:` (`badNonce`,
    /// `rateLimited` …), or the whole type for other namespaces.
    pub kind: String,
    pub detail: String,
    pub status: u16,
    /// From the response's `Retry-After`.
    pub retry_after: Option<Duration>,
}

impl fmt::Display for Problem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "the CA answered {} ({})", self.status, self.kind)?;
        if !self.detail.is_empty() {
            write!(f, ": {}", self.detail)?;
        }
        Ok(())
    }
}

impl std::error::Error for Problem {}

impl Problem {
    fn from_response(status: StatusCode, headers: &HeaderMap, body: &[u8]) -> Self {
        #[derive(Deserialize, Default)]
        struct Doc {
            #[serde(rename = "type", default)]
            kind: String,
            #[serde(default)]
            detail: String,
        }
        let doc: Doc = serde_json::from_slice(body).unwrap_or_default();
        let kind = doc
            .kind
            .strip_prefix("urn:ietf:params:acme:error:")
            .unwrap_or(&doc.kind)
            .to_owned();
        Self {
            kind: if kind.is_empty() {
                "unknown".into()
            } else {
                kind
            },
            detail: doc.detail.chars().take(500).collect(),
            status: status.as_u16(),
            retry_after: retry_after(headers, Timestamp::now()),
        }
    }
}

/// `Retry-After` as seconds or HTTP date, relative to `now`.
pub fn retry_after(headers: &HeaderMap, now: Timestamp) -> Option<Duration> {
    let value = headers.get(header::RETRY_AFTER)?.to_str().ok()?.trim();
    if let Ok(seconds) = value.parse::<u64>() {
        return Some(Duration::from_secs(seconds));
    }
    let at = jiff::fmt::rfc2822::parse(value).ok()?.timestamp();
    Some(Duration::try_from(at.duration_since(now)).unwrap_or(Duration::ZERO))
}

/// The URLs of the CA (§ 7.1.1).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Directory {
    pub new_nonce: String,
    pub new_account: String,
    pub new_order: String,
    #[serde(default)]
    pub renewal_info: Option<String>,
    #[serde(default)]
    pub meta: DirectoryMeta,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DirectoryMeta {
    #[serde(default)]
    pub terms_of_service: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Order {
    pub status: String,
    #[serde(default)]
    pub authorizations: Vec<String>,
    pub finalize: String,
    #[serde(default)]
    pub certificate: Option<String>,
    #[serde(default)]
    pub error: Option<Value>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Authorization {
    pub status: String,
    pub identifier: Identifier,
    #[serde(default)]
    pub challenges: Vec<Challenge>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Identifier {
    #[serde(rename = "type")]
    pub kind: String,
    pub value: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Challenge {
    #[serde(rename = "type")]
    pub kind: String,
    pub url: String,
    #[serde(default)]
    pub token: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub error: Option<Value>,
}

/// ARI's suggested window (RFC 9773 § 4.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RenewalWindow {
    pub start: Timestamp,
    pub end: Timestamp,
}

/// A response with what the client reads from it.
struct Reply {
    status: StatusCode,
    location: Option<String>,
    retry_after: Option<Duration>,
    body: Vec<u8>,
}

impl Reply {
    fn json<T: for<'de> Deserialize<'de>>(&self, what: &str) -> anyhow::Result<T> {
        serde_json::from_slice(&self.body).with_context(|| format!("{what}: unexpected answer"))
    }

    fn location(&self, what: &str) -> anyhow::Result<String> {
        self.location
            .clone()
            .ok_or_else(|| anyhow!("{what}: the CA sent no Location"))
    }
}

/// A session with one CA and one account key.
pub struct Client {
    http: reqwest::Client,
    pub directory: Directory,
    key: P256Key,
    /// The account URL once known (`kid`).
    account: Option<String>,
    nonce: Option<String>,
}

impl Client {
    /// Reads the directory at `directory_url`.
    pub async fn connect(
        http: reqwest::Client,
        directory_url: &str,
        key: P256Key,
    ) -> anyhow::Result<Self> {
        let directory = directory(&http, directory_url).await?;
        Ok(Self {
            http,
            directory,
            key,
            account: None,
            nonce: None,
        })
    }

    pub fn key(&self) -> &P256Key {
        &self.key
    }

    pub fn account_url(&self) -> Option<&str> {
        self.account.as_deref()
    }

    /// Creates the account, or finds the existing one for this key (§ 7.3,
    /// § 7.3.1), and brings its contact up to date. Returns its URL.
    pub async fn account(&mut self, contact: &[String]) -> anyhow::Result<String> {
        let payload = json!({ "termsOfServiceAgreed": true, "contact": contact });
        let url = self.directory.new_account.clone();
        let reply = self.post_with(&url, Some(&payload), Signer::Jwk).await?;
        let account = reply.location("new account")?;
        self.account = Some(account.clone());
        #[derive(Deserialize)]
        struct Account {
            #[serde(default)]
            contact: Vec<String>,
        }
        // An existing account keeps its old contact; update it.
        if reply.status == StatusCode::OK
            && reply
                .json::<Account>("account")
                .is_ok_and(|existing| existing.contact != contact)
        {
            self.post(&account, Some(&json!({ "contact": contact })))
                .await
                .context("account contact not updated")?;
        }
        Ok(account)
    }

    /// Uses a known account URL without asking the CA.
    pub fn use_account(&mut self, url: String) {
        self.account = Some(url);
    }

    /// A new order for `domain` (§ 7.4), optionally replacing a certificate
    /// by its ARI ID.
    pub async fn new_order(
        &mut self,
        domain: &str,
        replaces: Option<&str>,
    ) -> anyhow::Result<(String, Order)> {
        let mut payload = json!({ "identifiers": [{ "type": "dns", "value": domain }] });
        if let Some(replaces) = replaces {
            payload["replaces"] = replaces.into();
        }
        let url = self.directory.new_order.clone();
        let reply = self.post(&url, Some(&payload)).await?;
        Ok((reply.location("new order")?, reply.json("order")?))
    }

    pub async fn authorization(&mut self, url: &str) -> anyhow::Result<Authorization> {
        self.post(url, None).await?.json("authorization")
    }

    pub async fn order(&mut self, url: &str) -> anyhow::Result<Order> {
        self.post(url, None).await?.json("order")
    }

    /// Tells the CA to validate a challenge (§ 7.5.1).
    pub async fn respond(&mut self, challenge_url: &str) -> anyhow::Result<()> {
        self.post(challenge_url, Some(&json!({}))).await.map(drop)
    }

    /// Polls an authorization until it is no longer pending.
    pub async fn wait_for_authorization(&mut self, url: &str) -> anyhow::Result<Authorization> {
        let deadline = tokio::time::Instant::now() + POLL_DEADLINE;
        loop {
            let reply = self.post(url, None).await?;
            let authorization: Authorization = reply.json("authorization")?;
            if authorization.status != "pending" {
                return Ok(authorization);
            }
            pause(reply.retry_after, deadline, "authorization").await?;
        }
    }

    /// Polls an order while it is `pending` or `processing`.
    pub async fn wait_for_order(&mut self, url: &str) -> anyhow::Result<Order> {
        let deadline = tokio::time::Instant::now() + POLL_DEADLINE;
        loop {
            let reply = self.post(url, None).await?;
            let order: Order = reply.json("order")?;
            if !matches!(order.status.as_str(), "pending" | "processing") {
                return Ok(order);
            }
            pause(reply.retry_after, deadline, "order").await?;
        }
    }

    /// Sends the CSR (§ 7.4).
    pub async fn finalize(&mut self, order: &Order, csr: &[u8]) -> anyhow::Result<()> {
        let payload = json!({ "csr": jose::b64(csr) });
        self.post(&order.finalize, Some(&payload)).await.map(drop)
    }

    /// The certificate chain as PEM (§ 7.4.2).
    pub async fn download(&mut self, url: &str) -> anyhow::Result<String> {
        let reply = self.post(url, None).await?;
        String::from_utf8(reply.body).context("certificate chain is not text")
    }

    async fn post(&mut self, url: &str, payload: Option<&Value>) -> anyhow::Result<Reply> {
        let account = self
            .account
            .clone()
            .ok_or_else(|| anyhow!("ACME: no account yet"))?;
        self.post_with(url, payload, Signer::Kid(&account)).await
    }

    /// A signed POST; a `badNonce` is sent again with a fresh nonce.
    async fn post_with(
        &mut self,
        url: &str,
        payload: Option<&Value>,
        signer: Signer<'_>,
    ) -> anyhow::Result<Reply> {
        let mut attempt = 0;
        loop {
            let nonce = match self.nonce.take() {
                Some(nonce) => nonce,
                None => self.fresh_nonce().await?,
            };
            check_url(url)?;
            let body = jose::sign(&self.key, signer, &nonce, url, payload)?;
            let response = self
                .http
                .post(url)
                .header(header::CONTENT_TYPE, "application/jose+json")
                .body(body.to_string())
                .send()
                .await
                .with_context(|| format!("ACME request to {url} failed"))?;
            let status = response.status();
            let headers = response.headers().clone();
            self.keep_nonce(&headers);
            let body = read_body(response).await?;
            if status.is_success() {
                return Ok(Reply {
                    status,
                    location: headers
                        .get(header::LOCATION)
                        .and_then(|v| v.to_str().ok())
                        .map(str::to_owned),
                    retry_after: retry_after(&headers, Timestamp::now()),
                    body,
                });
            }
            let problem = Problem::from_response(status, &headers, &body);
            if problem.kind == "badNonce" && attempt < BAD_NONCE_RETRIES {
                attempt += 1;
                tracing::debug!(url, attempt, "ACME: bad nonce, retrying");
                continue;
            }
            return Err(problem.into());
        }
    }

    async fn fresh_nonce(&mut self) -> anyhow::Result<String> {
        let response = self
            .http
            .head(&self.directory.new_nonce)
            .send()
            .await
            .context("ACME nonce not fetched")?;
        let headers = response.headers().clone();
        if !response.status().is_success() {
            return Err(Problem::from_response(response.status(), &headers, &[]).into());
        }
        self.keep_nonce(&headers);
        self.nonce
            .take()
            .ok_or_else(|| anyhow!("the CA sent no Replay-Nonce"))
    }

    fn keep_nonce(&mut self, headers: &HeaderMap) {
        if let Some(nonce) = headers
            .get("replay-nonce")
            .and_then(|v| v.to_str().ok())
            .filter(|nonce| {
                !nonce.is_empty()
                    && nonce
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
            })
        {
            self.nonce = Some(nonce.to_owned());
        }
    }
}

/// Only HTTPS to the CA (RFC 8555 § 6.1), also for every URL it hands out;
/// plain HTTP only to a test CA on loopback (`config` allows such a
/// directory nowhere else).
pub fn check_url(url: &str) -> anyhow::Result<()> {
    let parsed = url::Url::parse(url).with_context(|| format!("ACME: {url} is not a URL"))?;
    let loopback = match parsed.host() {
        Some(url::Host::Domain(host)) => host == "localhost",
        Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
        Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
        None => false,
    };
    match parsed.scheme() {
        "https" if parsed.host().is_some() => Ok(()),
        "http" if loopback => Ok(()),
        _ => bail!("ACME: refusing {url} – the CA must be reached over HTTPS"),
    }
}

/// Reads the CA's directory (§ 7.1.1).
pub async fn directory(http: &reqwest::Client, url: &str) -> anyhow::Result<Directory> {
    check_url(url)?;
    let response = http
        .get(url)
        .send()
        .await
        .with_context(|| format!("ACME directory {url} not reachable"))?;
    let status = response.status();
    let headers = response.headers().clone();
    let body = read_body(response).await?;
    if !status.is_success() {
        return Err(Problem::from_response(status, &headers, &body).into());
    }
    serde_json::from_slice(&body).with_context(|| format!("{url} is not an ACME directory"))
}

/// ARI for a certificate (RFC 9773 § 4.1); `None` if the CA has no renewal
/// info.
pub async fn renewal_info(
    http: &reqwest::Client,
    directory: &Directory,
    cert_id: &str,
) -> anyhow::Result<Option<RenewalWindow>> {
    let Some(base) = &directory.renewal_info else {
        return Ok(None);
    };
    let url = format!("{}/{cert_id}", base.trim_end_matches('/'));
    check_url(&url)?;
    let response = http.get(&url).send().await?;
    let status = response.status();
    let headers = response.headers().clone();
    let body = read_body(response).await?;
    if !status.is_success() {
        return Err(Problem::from_response(status, &headers, &body).into());
    }
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Info {
        suggested_window: Window,
    }
    #[derive(Deserialize)]
    struct Window {
        start: String,
        end: String,
    }
    let info: Info = serde_json::from_slice(&body).context("renewal info: unexpected answer")?;
    let parse = |text: &str| -> anyhow::Result<Timestamp> {
        text.parse()
            .with_context(|| format!("renewal info: {text} is not a time"))
    };
    let (start, end) = (
        parse(&info.suggested_window.start)?,
        parse(&info.suggested_window.end)?,
    );
    if end < start {
        bail!("renewal info: window ends before it starts");
    }
    Ok(Some(RenewalWindow { start, end }))
}

async fn read_body(mut response: reqwest::Response) -> anyhow::Result<Vec<u8>> {
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        if body.len() + chunk.len() > MAX_BODY {
            bail!("ACME response larger than {MAX_BODY} bytes");
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

/// Waits before the next poll; fails once the deadline would pass.
async fn pause(
    retry_after: Option<Duration>,
    deadline: tokio::time::Instant,
    what: &str,
) -> anyhow::Result<()> {
    let wait = retry_after.unwrap_or(POLL_DEFAULT).min(POLL_MAX);
    if tokio::time::Instant::now() + wait > deadline {
        bail!("the {what} is still not done after {POLL_DEADLINE:?}");
    }
    tokio::time::sleep(wait).await;
    Ok(())
}

/// A short reason from a challenge's or order's `error`.
pub fn describe_error(error: Option<&Value>) -> String {
    let Some(error) = error else {
        return "no reason given".into();
    };
    let kind = error["type"]
        .as_str()
        .unwrap_or("unknown")
        .trim_start_matches("urn:ietf:params:acme:error:");
    match error["detail"].as_str() {
        Some(detail) => format!("{kind}: {}", detail.chars().take(300).collect::<String>()),
        None => kind.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use reqwest::header::HeaderValue;

    use super::*;

    #[test]
    fn retry_after_in_seconds_or_as_date() {
        let now: Timestamp = "2026-11-27T12:00:00Z".parse().unwrap();
        let mut headers = HeaderMap::new();
        assert_eq!(retry_after(&headers, now), None);
        headers.insert(header::RETRY_AFTER, HeaderValue::from_static("120"));
        assert_eq!(retry_after(&headers, now), Some(Duration::from_secs(120)));
        headers.insert(
            header::RETRY_AFTER,
            HeaderValue::from_static("Fri, 27 Nov 2026 13:00:00 GMT"),
        );
        assert_eq!(retry_after(&headers, now), Some(Duration::from_secs(3600)));
        headers.insert(
            header::RETRY_AFTER,
            HeaderValue::from_static("Fri, 27 Nov 2026 11:00:00 GMT"),
        );
        assert_eq!(retry_after(&headers, now), Some(Duration::ZERO));
        headers.insert(header::RETRY_AFTER, HeaderValue::from_static("soon"));
        assert_eq!(retry_after(&headers, now), None);
    }

    #[test]
    fn problem_documents() {
        let mut headers = HeaderMap::new();
        headers.insert(header::RETRY_AFTER, HeaderValue::from_static("30"));
        let problem = Problem::from_response(
            StatusCode::TOO_MANY_REQUESTS,
            &headers,
            br#"{"type":"urn:ietf:params:acme:error:rateLimited","detail":"too many certificates"}"#,
        );
        assert_eq!(
            problem,
            Problem {
                kind: "rateLimited".into(),
                detail: "too many certificates".into(),
                status: 429,
                retry_after: Some(Duration::from_secs(30)),
            }
        );
        assert_eq!(
            problem.to_string(),
            "the CA answered 429 (rateLimited): too many certificates"
        );
        let problem = Problem::from_response(StatusCode::BAD_GATEWAY, &HeaderMap::new(), b"<html>");
        assert_eq!(problem.kind, "unknown");
        assert_eq!(problem.to_string(), "the CA answered 502 (unknown)");
    }

    #[test]
    fn talks_to_the_ca_over_https_only() {
        for good in [
            "https://acme-v02.api.letsencrypt.org/acme/new-order",
            "http://127.0.0.1:14000/dir",
            "http://[::1]:14000/dir",
            "http://localhost/dir",
        ] {
            check_url(good).unwrap();
        }
        for bad in [
            "http://ca.example.test/acme/order/1",
            "http://192.0.2.7/dir",
            "ftp://ca.example.test/dir",
            "not a url",
        ] {
            assert!(check_url(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn describes_challenge_errors() {
        assert_eq!(describe_error(None), "no reason given");
        assert_eq!(
            describe_error(Some(&json!({
                "type": "urn:ietf:params:acme:error:connection",
                "detail": "Timeout during connect"
            }))),
            "connection: Timeout during connect"
        );
    }
}
