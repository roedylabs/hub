//! The hub's own client for deSEC's DNS API (<https://desec.readthedocs.io/>),
//! as far as the DNS-01 challenge needs it: the zone that owns a name, its
//! `NS` records, and the TXT record set of the challenge.
//!
//! - `GET domains/?owns_qname=<name>`: the account's domain responsible for
//!   the name (a list with at most one entry), with its `minimum_ttl`.
//! - `GET domains/<zone>/rrsets/<subname>/<type>/`: a record set; `404` when
//!   there is none. Any token may read every record set of its account.
//! - `PUT domains/<zone>/rrsets/` with a list of one record set (a bulk
//!   request): creates or replaces it; empty `records` delete it. Only the
//!   record set in the list changes. TXT values go in double quotes.
//!
//! As deSEC's certbot plugin does, the hub reads the record set first and
//! adds its value to the others (or removes only its own), keeping the set's
//! TTL, so it does not take away values another client put there
//! ([`Desec::modify_txt`]). deSEC has no conditional update: a client that
//! writes the same set between the hub's read and its write loses its change.
//! The set belongs to the hub (docs/ops.md › DNS-01 with deSEC); other values
//! there are only kept, not coordinated with.
//!
//! The token goes in `Authorization: Token …` (a header marked sensitive)
//! and nowhere else: not in URLs, logs, errors or the audit log. Throttled
//! requests (`429`) are sent again after `Retry-After` seconds, up to three
//! times as long as each wait is at most a minute – a throttled write from a
//! fresh read. A longer wait ends the request: when adding the value, the
//! issuer's backoff takes over; when removing it, the value stays in the
//! record set and the issuer logs a warning (docs/ops.md).

use std::{fmt, time::Duration};

use anyhow::{Context as _, anyhow, bail};
use jiff::Timestamp;
use reqwest::{Method, StatusCode, header};
use serde::Deserialize;
use serde_json::{Value, json};
use url::Url;

use super::acme::retry_after;
use crate::config::Secret;

/// deSEC's API.
pub const API: &str = "https://desec.io/api/v1/";
/// How often a throttled request is sent again.
const THROTTLE_RETRIES: usize = 3;
/// Longest `Retry-After` waited for in place; longer waits (deSEC's hourly
/// and daily limits) end the attempt and the issuer waits.
const THROTTLE_MAX_WAIT: Duration = Duration::from_secs(60);
/// Waiting after a `429` without `Retry-After`.
const THROTTLE_DEFAULT_WAIT: Duration = Duration::from_secs(5);
/// Largest response body read.
const MAX_BODY: usize = 256 * 1024;
/// The TTL when deSEC names no minimum (it always does).
const DEFAULT_TTL: u32 = 3600;

/// An error answer from deSEC.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DesecError {
    pub status: u16,
    /// deSEC's `detail`, or the start of the body.
    pub detail: String,
    /// From `Retry-After` (with `429`).
    pub retry_after: Option<Duration>,
}

impl fmt::Display for DesecError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "deSEC answered {}", self.status)?;
        match self.status {
            401 => f.write_str(" (the token is not valid)")?,
            403 => f.write_str(" (the token may not do this)")?,
            429 => f.write_str(" (throttled)")?,
            _ => {}
        }
        if !self.detail.is_empty() {
            write!(f, ": {}", self.detail)?;
        }
        Ok(())
    }
}

impl std::error::Error for DesecError {}

impl DesecError {
    fn from_response(status: StatusCode, headers: &header::HeaderMap, body: &[u8]) -> Self {
        // DRF's errors: `{"detail": "…", "code": "…"}`; validation errors are
        // other JSON.
        let detail = serde_json::from_slice::<Value>(body)
            .ok()
            .and_then(|doc| doc.get("detail").and_then(Value::as_str).map(str::to_owned))
            .unwrap_or_else(|| String::from_utf8_lossy(body).into_owned());
        Self {
            status: status.as_u16(),
            detail: detail
                .chars()
                .map(|c| if c.is_control() { ' ' } else { c })
                .take(300)
                .collect::<String>()
                .trim()
                .to_owned(),
            retry_after: retry_after(headers, Timestamp::now()),
        }
    }
}

/// A domain of the account (`GET domains/…`).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Zone {
    pub name: String,
    /// The lowest TTL deSEC accepts for this domain's records.
    #[serde(default)]
    pub minimum_ttl: Option<u32>,
}

impl Zone {
    pub fn ttl(&self) -> u32 {
        self.minimum_ttl.unwrap_or(DEFAULT_TTL)
    }

    /// `name`'s part before the zone (`_acme-challenge.hub` for
    /// `_acme-challenge.hub.example.com` in `example.com`); `None` if the
    /// name is not in the zone.
    pub fn subname(&self, name: &str) -> Option<String> {
        let name = name.trim_end_matches('.').to_ascii_lowercase();
        let zone = self.name.trim_end_matches('.').to_ascii_lowercase();
        if name == zone {
            return Some(String::new());
        }
        name.strip_suffix(&zone)?
            .strip_suffix('.')
            .filter(|subname| !subname.is_empty())
            .map(str::to_owned)
    }
}

/// A record set as deSEC returns it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct RrSet {
    #[serde(default)]
    pub ttl: Option<u32>,
    #[serde(default)]
    pub records: Vec<String>,
}

/// A TXT value as deSEC takes it: in double quotes. The challenge's value
/// (base64url) needs no escaping.
pub fn quote(value: &str) -> String {
    format!("\"{value}\"")
}

/// The body of a bulk `PUT` that sets one TXT record set; empty `records`
/// delete it.
pub fn txt_body(subname: &str, ttl: u32, records: &[String]) -> Value {
    json!([{
        "subname": subname,
        "type": "TXT",
        "ttl": ttl,
        "records": records,
    }])
}

/// A deSEC account, through one token.
pub struct Desec {
    http: reqwest::Client,
    api: Url,
    authorization: header::HeaderValue,
}

impl fmt::Debug for Desec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Desec")
            .field("api", &self.api.as_str())
            .finish_non_exhaustive()
    }
}

impl Desec {
    /// A client for the API at `api` (HTTPS; plain HTTP only on loopback,
    /// for tests) with `token`.
    pub fn new(http: reqwest::Client, api: &Url, token: &Secret) -> anyhow::Result<Self> {
        let loopback = match api.host() {
            Some(url::Host::Domain(host)) => host == "localhost",
            Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
            Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
            None => false,
        };
        match api.scheme() {
            "https" if api.host().is_some() => {}
            "http" if loopback => {}
            _ => bail!("refusing the deSEC API at {api} – it must be reached over HTTPS"),
        }
        let mut api = api.clone();
        if !api.path().ends_with('/') {
            api.set_path(&format!("{}/", api.path()));
        }
        // Never the token itself in the message.
        let mut authorization = header::HeaderValue::from_str(&format!("Token {}", token.expose()))
            .map_err(|_| anyhow!("the deSEC token contains invalid characters"))?;
        authorization.set_sensitive(true);
        Ok(Self {
            http,
            api,
            authorization,
        })
    }

    /// The domain of the account that owns `name`.
    pub async fn zone_for(&self, name: &str) -> anyhow::Result<Zone> {
        let mut url = self.url(&["domains", ""]);
        url.query_pairs_mut()
            .append_pair("owns_qname", name.trim_end_matches('.'));
        let body = self
            .send(Method::GET, url, None, true)
            .await?
            .unwrap_or_default();
        let zones: Vec<Zone> =
            serde_json::from_slice(&body).context("deSEC domains: unexpected answer")?;
        let zone = zones.into_iter().next().ok_or_else(|| {
            anyhow!(
                "no domain in the deSEC account owns {name} – add the domain at deSEC, or use a \
                 token of the account that has it"
            )
        })?;
        if zone.subname(name).is_none() {
            bail!("deSEC named {} as the domain of {name}", zone.name);
        }
        Ok(zone)
    }

    /// A record set; `None` if there is none.
    pub async fn rrset(
        &self,
        zone: &str,
        subname: &str,
        kind: &str,
    ) -> anyhow::Result<Option<RrSet>> {
        // `@` stands for the zone's apex in URLs.
        let subname = if subname.is_empty() { "@" } else { subname };
        let url = self.url(&["domains", zone, "rrsets", subname, kind, ""]);
        let Some(body) = self.send(Method::GET, url, None, true).await? else {
            return Ok(None);
        };
        let set = serde_json::from_slice(&body)
            .with_context(|| format!("deSEC record set {subname}/{kind}: unexpected answer"))?;
        Ok(Some(set))
    }

    /// The records of a record set; empty if there is none.
    pub async fn records(
        &self,
        zone: &str,
        subname: &str,
        kind: &str,
    ) -> anyhow::Result<Vec<String>> {
        Ok(self
            .rrset(zone, subname, kind)
            .await?
            .map(|set| set.records)
            .unwrap_or_default())
    }

    /// Reads the TXT record set `subname` of `zone` and writes what `change`
    /// makes of its records (`None`: nothing to write; empty: delete the
    /// set), with the set's TTL, or the zone's minimum for a new set.
    /// Throttled, it starts again from a fresh read. Whether it wrote.
    pub async fn modify_txt(
        &self,
        zone: &Zone,
        subname: &str,
        change: impl Fn(&[String]) -> Option<Vec<String>>,
    ) -> anyhow::Result<bool> {
        let mut throttled = 0;
        loop {
            let set = self.rrset(&zone.name, subname, "TXT").await?;
            let current = set.as_ref().map_or(&[][..], |set| &set.records[..]);
            let Some(records) = change(current) else {
                return Ok(false);
            };
            let ttl = set.and_then(|set| set.ttl).unwrap_or_else(|| zone.ttl());
            let url = self.url(&["domains", &zone.name, "rrsets", ""]);
            match self
                .send(
                    Method::PUT,
                    url,
                    Some(txt_body(subname, ttl, &records)),
                    false,
                )
                .await
            {
                Ok(_) => return Ok(true),
                Err(error) => match throttle_wait(&error, &mut throttled) {
                    Some(wait) => {
                        tracing::info!(?wait, "deSEC throttles, waiting");
                        tokio::time::sleep(wait).await;
                    }
                    None => return Err(error),
                },
            }
        }
    }

    /// `api` + the segments, each percent-encoded; `""` last for the
    /// trailing slash every endpoint has.
    fn url(&self, segments: &[&str]) -> Url {
        let mut url = self.api.clone();
        if let Ok(mut path) = url.path_segments_mut() {
            path.pop_if_empty().extend(segments);
        }
        url
    }

    /// Sends a request, with `retry_throttled` again after short
    /// throttling; the body of a `2xx` answer, `None` for `404` on a `GET`.
    async fn send(
        &self,
        method: Method,
        url: Url,
        body: Option<Value>,
        retry_throttled: bool,
    ) -> anyhow::Result<Option<Vec<u8>>> {
        let what = format!("deSEC {method} {}", url.path());
        let mut throttled = 0;
        loop {
            let mut request = self
                .http
                .request(method.clone(), url.clone())
                .header(header::AUTHORIZATION, self.authorization.clone())
                .header(header::ACCEPT, "application/json");
            if let Some(body) = &body {
                request = request
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(body.to_string());
            }
            // reqwest's error names the URL, which holds no secret.
            let response = request
                .send()
                .await
                .map_err(|error| anyhow!("{what}: {}", without_url(&error)))?;
            let status = response.status();
            let headers = response.headers().clone();
            let body = match read_body(response).await {
                Ok(body) => body,
                Err(error) if status.is_success() => {
                    return Err(error.context(format!("{what}: answer not read")));
                }
                // An error answer's body is only detail; its status and
                // `Retry-After` still count.
                Err(_) => Vec::new(),
            };
            if status.is_success() {
                return Ok(Some(body));
            }
            if status == StatusCode::NOT_FOUND && method == Method::GET {
                return Ok(None);
            }
            let error = anyhow::Error::new(DesecError::from_response(status, &headers, &body))
                .context(what.clone());
            match throttle_wait(&error, &mut throttled).filter(|_| retry_throttled) {
                Some(wait) => {
                    tracing::info!(?wait, "deSEC throttles, waiting");
                    tokio::time::sleep(wait).await;
                }
                None => return Err(error),
            }
        }
    }
}

/// How long to wait before trying a throttled request again; `None` if the
/// error is no `429`, the retries are used up or deSEC asks for longer than
/// [`THROTTLE_MAX_WAIT`].
fn throttle_wait(error: &anyhow::Error, throttled: &mut usize) -> Option<Duration> {
    let error = error.downcast_ref::<DesecError>()?;
    if error.status != 429 || *throttled >= THROTTLE_RETRIES {
        return None;
    }
    let wait = error.retry_after.unwrap_or(THROTTLE_DEFAULT_WAIT);
    if wait > THROTTLE_MAX_WAIT {
        return None;
    }
    *throttled += 1;
    Some(wait)
}

/// reqwest's message without the URL (the path is in the context already).
fn without_url(error: &reqwest::Error) -> String {
    let mut message = error.to_string();
    if let Some(url) = error.url() {
        message = message.replace(&format!(" ({url})"), "");
    }
    let mut source = std::error::Error::source(error);
    while let Some(inner) = source {
        message.push_str(": ");
        message.push_str(&inner.to_string());
        source = inner.source();
    }
    message
}

async fn read_body(mut response: reqwest::Response) -> anyhow::Result<Vec<u8>> {
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        if body.len() + chunk.len() > MAX_BODY {
            bail!("answer larger than {MAX_BODY} bytes");
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bodies_and_quoting() {
        assert_eq!(quote("abc-_1"), "\"abc-_1\"");
        assert_eq!(
            txt_body("_acme-challenge.hub", 3600, &[quote("v1"), quote("v2")]),
            json!([{
                "subname": "_acme-challenge.hub",
                "type": "TXT",
                "ttl": 3600,
                "records": ["\"v1\"", "\"v2\""],
            }])
        );
        assert_eq!(
            txt_body("_acme-challenge", 60, &[]).to_string(),
            r#"[{"records":[],"subname":"_acme-challenge","ttl":60,"type":"TXT"}]"#
        );
    }

    #[test]
    fn subnames_within_the_zone() {
        let zone = Zone {
            name: "example.test".into(),
            minimum_ttl: Some(60),
        };
        assert_eq!(
            zone.subname("_acme-challenge.hub.Example.test.").as_deref(),
            Some("_acme-challenge.hub")
        );
        assert_eq!(zone.subname("example.test").as_deref(), Some(""));
        assert_eq!(zone.subname("hub.otherexample.test"), None);
        assert_eq!(zone.subname("other.test"), None);
        assert_eq!(zone.ttl(), 60);
        let zone: Zone = serde_json::from_str(r#"{"name":"example.test"}"#).unwrap();
        assert_eq!(zone.ttl(), DEFAULT_TTL);
    }

    #[test]
    fn urls_end_with_a_slash() {
        let token = Secret::new("t0ken");
        let http = crate::tls::issuer::http_client().unwrap();
        let desec = Desec::new(http.clone(), &Url::parse(API).unwrap(), &token).unwrap();
        assert_eq!(
            desec
                .url(&[
                    "domains",
                    "example.test",
                    "rrsets",
                    "_acme-challenge.hub",
                    "TXT",
                    ""
                ])
                .as_str(),
            "https://desec.io/api/v1/domains/example.test/rrsets/_acme-challenge.hub/TXT/"
        );
        assert_eq!(
            desec.url(&["domains", ""]).as_str(),
            "https://desec.io/api/v1/domains/"
        );
        let without = Desec::new(
            http.clone(),
            &Url::parse("http://127.0.0.1:1/api/v1").unwrap(),
            &token,
        )
        .unwrap();
        assert_eq!(
            without.url(&["domains", ""]).as_str(),
            "http://127.0.0.1:1/api/v1/domains/"
        );
        assert!(format!("{desec:?}").contains("desec.io"));
        assert!(!format!("{desec:?}").contains("t0ken"));

        // HTTPS only (plain HTTP on loopback), and a token fit for a header.
        assert!(
            Desec::new(
                http.clone(),
                &Url::parse("http://desec.io/api/v1/").unwrap(),
                &token
            )
            .is_err()
        );
        let err =
            Desec::new(http, &Url::parse(API).unwrap(), &Secret::new("bad\ntoken")).unwrap_err();
        assert!(!err.to_string().contains("bad"), "{err}");
    }

    #[test]
    fn error_documents() {
        let mut headers = header::HeaderMap::new();
        headers.insert(header::RETRY_AFTER, "7".parse().unwrap());
        let error = DesecError::from_response(
            StatusCode::TOO_MANY_REQUESTS,
            &headers,
            br#"{"detail":"Request was throttled.\nExpected available in 7 seconds.","code":"throttled"}"#,
        );
        assert_eq!(error.status, 429);
        assert_eq!(error.retry_after, Some(Duration::from_secs(7)));
        assert_eq!(
            error.to_string(),
            "deSEC answered 429 (throttled): Request was throttled. Expected available in 7 seconds."
        );
        let error = DesecError::from_response(
            StatusCode::BAD_REQUEST,
            &header::HeaderMap::new(),
            br#"[{"ttl":["Ensure this value is greater than or equal to 3600."]}]"#,
        );
        assert!(
            error.detail.contains("greater than or equal to 3600"),
            "{error}"
        );
        assert_eq!(error.retry_after, None);
        let error = DesecError::from_response(
            StatusCode::FORBIDDEN,
            &header::HeaderMap::new(),
            br#"{"detail":"Insufficient token permissions.","code":"forbidden"}"#,
        );
        assert_eq!(
            error.to_string(),
            "deSEC answered 403 (the token may not do this): Insufficient token permissions."
        );
    }
}
