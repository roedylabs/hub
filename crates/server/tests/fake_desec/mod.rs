//! An in-process deSEC API for the DNS-01 tests, as far as the hub uses it
//! (<https://desec.readthedocs.io/>): `GET domains/?owns_qname=`, `GET
//! domains/<zone>/rrsets/<subname>/<type>/` (`@` for the apex) and the bulk
//! `PUT domains/<zone>/rrsets/`. It checks the token, the trailing slashes,
//! the record sets (type, quoted TXT values, TTL ≥ the domain's minimum)
//! and publishes TXT changes to fake name servers ([`FakeDns`]), each on its
//! own schedule ([`Publish`]); an older publication never overtakes a newer
//! one. Knobs throttle (`429` with `Retry-After`) requests or writes – with
//! another client's change while the hub waits – and refuse writes (`403`);
//! every request is logged.

#![allow(dead_code, reason = "each test uses its own part")]
#![allow(
    clippy::result_large_err,
    reason = "a handler's error is the response it sends"
)]

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Duration,
};

use axum::{
    Json, Router,
    body::Bytes,
    extract::{Path, Query, State},
    http::{HeaderMap, Method, StatusCode, Uri, header},
    response::{IntoResponse as _, Response},
    routing::get,
};
use serde_json::{Value, json};
use tokio::net::TcpListener;

use super::fake_dns::FakeDns;

/// A request as it arrived.
#[derive(Debug, Clone)]
pub struct Request {
    pub method: String,
    /// Path and query.
    pub uri: String,
    pub authorized: bool,
    pub body: Option<Value>,
    pub status: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RrSet {
    pub ttl: u32,
    pub records: Vec<String>,
}

/// When a change shows on a name server.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Publish {
    Now,
    After(Duration),
    /// Once the server was asked for the name after the change.
    AfterQuery,
    Never,
}

pub struct Knobs {
    /// The next this many requests get `429` with this `Retry-After`.
    pub throttle: usize,
    pub retry_after: String,
    /// The next this many writes get `429` …
    pub throttle_writes: usize,
    /// … while another client adds this value to the record set.
    pub concurrent_value: Option<String>,
    /// Writes get `403` (a token whose policy does not allow them).
    pub forbid_writes: bool,
    /// Per name server.
    pub publish: Vec<Publish>,
}

struct Inner {
    token: String,
    /// Domains of the account: name and minimum TTL.
    domains: Vec<(String, u32)>,
    name_servers: Vec<String>,
    rrsets: HashMap<(String, String, String), RrSet>,
    dns: Vec<FakeDns>,
    knobs: Knobs,
    log: Vec<Request>,
    /// Publications: the last one numbered, and the latest applied per
    /// server and name.
    revision: u64,
    applied: HashMap<(usize, String), u64>,
}

#[derive(Clone)]
pub struct FakeDesec {
    pub base: String,
    inner: Arc<Mutex<Inner>>,
}

impl FakeDesec {
    /// An account with `token` and one domain `zone` (minimum TTL
    /// `minimum_ttl`) served by `name_servers`, whose fake servers are `dns`.
    pub async fn start(
        token: &str,
        zone: &str,
        minimum_ttl: u32,
        name_servers: &[&str],
        dns: Vec<FakeDns>,
    ) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let publish = vec![Publish::Now; dns.len()];
        let mut rrsets = HashMap::new();
        rrsets.insert(
            (zone.to_owned(), String::new(), "NS".to_owned()),
            RrSet {
                ttl: 3600,
                records: name_servers.iter().map(|ns| (*ns).to_owned()).collect(),
            },
        );
        let fake = Self {
            base,
            inner: Arc::new(Mutex::new(Inner {
                token: token.to_owned(),
                domains: vec![(zone.to_owned(), minimum_ttl)],
                name_servers: name_servers.iter().map(|ns| (*ns).to_owned()).collect(),
                rrsets,
                dns,
                knobs: Knobs {
                    throttle: 0,
                    retry_after: "1".into(),
                    throttle_writes: 0,
                    concurrent_value: None,
                    forbid_writes: false,
                    publish,
                },
                log: Vec::new(),
                revision: 0,
                applied: HashMap::new(),
            })),
        };
        let router = Router::new()
            .route("/api/v1/domains/", get(domains))
            .route("/api/v1/domains/{zone}/rrsets/", axum::routing::put(bulk))
            .route(
                "/api/v1/domains/{zone}/rrsets/{subname}/{kind}/",
                get(rrset),
            )
            .fallback(|uri: Uri| async move {
                (
                    StatusCode::NOT_FOUND,
                    Json(json!({"detail": format!("no route {uri}")})),
                )
            })
            .with_state(fake.clone());
        tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        fake
    }

    /// The API's base, as `ROEDY_…` would name it.
    pub fn api(&self) -> url::Url {
        url::Url::parse(&format!("{}/api/v1/", self.base)).unwrap()
    }

    pub fn knobs(&self, change: impl FnOnce(&mut Knobs)) {
        change(&mut self.lock().knobs);
    }

    pub fn log(&self) -> Vec<Request> {
        self.lock().log.clone()
    }

    pub fn rrset(&self, zone: &str, subname: &str, kind: &str) -> Option<RrSet> {
        self.lock()
            .rrsets
            .get(&(zone.to_owned(), subname.to_owned(), kind.to_owned()))
            .cloned()
    }

    /// Puts a record set in place (and on the name servers at once).
    pub fn preset(&self, zone: &str, subname: &str, ttl: u32, records: &[&str]) {
        let records: Vec<String> = records.iter().map(|r| (*r).to_owned()).collect();
        let mut inner = self.lock();
        let name = fqdn(zone, subname);
        // Newer than any publication still pending.
        inner.revision += 1;
        let revision = inner.revision;
        for (index, dns) in inner.dns.clone().iter().enumerate() {
            inner.applied.insert((index, name.clone()), revision);
            dns.set(&name, &unquote(&records));
        }
        inner.rrsets.insert(
            (zone.to_owned(), subname.to_owned(), "TXT".to_owned()),
            RrSet { ttl, records },
        );
    }

    /// Another server that gets the zone's changes (at once).
    pub fn add_name_server(&self, dns: FakeDns) {
        let mut inner = self.lock();
        inner.dns.push(dns);
        inner.knobs.publish.push(Publish::Now);
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap()
    }

    /// Checks token and throttling and logs the request; the response if
    /// it ends here.
    fn admit(
        &self,
        method: &Method,
        uri: &Uri,
        headers: &HeaderMap,
        body: Option<Value>,
    ) -> Result<usize, Response> {
        let mut inner = self.lock();
        let expected = format!("Token {}", inner.token);
        let authorized = headers
            .get(header::AUTHORIZATION)
            .is_some_and(|value| value.as_bytes() == expected.as_bytes());
        let index = inner.log.len();
        inner.log.push(Request {
            method: method.to_string(),
            uri: uri.to_string(),
            authorized,
            body,
            status: 0,
        });
        let response = if !authorized {
            Some(
                (
                    StatusCode::UNAUTHORIZED,
                    Json(json!({"detail": "Invalid token."})),
                )
                    .into_response(),
            )
        } else if inner.knobs.throttle > 0 {
            inner.knobs.throttle -= 1;
            let wait = inner.knobs.retry_after.clone();
            Some(
                (
                    StatusCode::TOO_MANY_REQUESTS,
                    [(header::RETRY_AFTER, wait.clone())],
                    Json(json!({
                        "detail": format!("Request was throttled. Expected available in {wait} seconds."),
                    })),
                )
                    .into_response(),
            )
        } else {
            None
        };
        if let Some(response) = response {
            inner.log[index].status = response.status().as_u16();
            return Err(response);
        }
        Ok(index)
    }

    fn done(&self, index: usize, response: Response) -> Response {
        self.lock().log[index].status = response.status().as_u16();
        response
    }

    /// Shows `values` at `name` on each name server as its knob says.
    fn publish(&self, name: &str, values: Vec<String>) {
        let (revision, plan) = {
            let mut inner = self.lock();
            inner.revision += 1;
            let plan: Vec<(usize, FakeDns, Publish)> = inner
                .dns
                .iter()
                .cloned()
                .zip(inner.knobs.publish.iter().copied())
                .enumerate()
                .map(|(index, (server, publish))| (index, server, publish))
                .collect();
            (inner.revision, plan)
        };
        for (index, server, publish) in plan {
            let (fake, name, values) = (self.clone(), name.to_owned(), values.clone());
            let (watched, target) = (server.clone(), name.clone());
            let asked = move || {
                watched
                    .queries()
                    .iter()
                    .filter(|(asked, _)| *asked == target)
                    .count()
            };
            let before = asked();
            let show = move || fake.show(index, &server, &name, &values, revision);
            match publish {
                Publish::Now => show(),
                Publish::After(delay) => {
                    tokio::spawn(async move {
                        tokio::time::sleep(delay).await;
                        show();
                    });
                }
                Publish::AfterQuery => {
                    tokio::spawn(async move {
                        while asked() <= before {
                            tokio::time::sleep(Duration::from_millis(5)).await;
                        }
                        show();
                    });
                }
                Publish::Never => {}
            }
        }
    }

    /// Applies a publication unless a newer one is there already.
    fn show(&self, index: usize, server: &FakeDns, name: &str, values: &[String], revision: u64) {
        let mut inner = self.lock();
        let applied = inner.applied.entry((index, name.to_owned())).or_insert(0);
        if *applied > revision {
            return;
        }
        *applied = revision;
        server.set(name, values);
    }
}

fn fqdn(zone: &str, subname: &str) -> String {
    if subname.is_empty() {
        zone.to_owned()
    } else {
        format!("{subname}.{zone}")
    }
}

fn unquote(records: &[String]) -> Vec<String> {
    records
        .iter()
        .map(|record| record.trim_matches('"').to_owned())
        .collect()
}

async fn domains(
    State(fake): State<FakeDesec>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    Query(query): Query<HashMap<String, String>>,
) -> Response {
    let index = match fake.admit(&method, &uri, &headers, None) {
        Ok(index) => index,
        Err(response) => return response,
    };
    let qname = query.get("owns_qname").cloned().unwrap_or_default();
    let domains = fake.lock().domains.clone();
    // The longest domain that the name is in or equal to.
    let owner = domains
        .iter()
        .filter(|(name, _)| qname == *name || qname.ends_with(&format!(".{name}")))
        .max_by_key(|(name, _)| name.len());
    let body: Vec<Value> = owner
        .map(|(name, ttl)| {
            json!({
                "created": "2026-01-01T00:00:00Z",
                "minimum_ttl": ttl,
                "name": name,
                "published": "2026-01-01T00:00:00Z",
                "touched": "2026-01-01T00:00:00Z",
            })
        })
        .into_iter()
        .collect();
    fake.done(index, Json(body).into_response())
}

async fn rrset(
    State(fake): State<FakeDesec>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    Path((zone, subname, kind)): Path<(String, String, String)>,
) -> Response {
    let index = match fake.admit(&method, &uri, &headers, None) {
        Ok(index) => index,
        Err(response) => return response,
    };
    let subname = if subname == "@" {
        String::new()
    } else {
        subname
    };
    let set = fake
        .lock()
        .rrsets
        .get(&(zone.clone(), subname.clone(), kind.clone()))
        .cloned();
    let response = match set {
        Some(set) => Json(json!({
            "domain": zone,
            "subname": subname,
            "name": format!("{}.", fqdn(&zone, &subname)),
            "type": kind,
            "ttl": set.ttl,
            "records": set.records,
        }))
        .into_response(),
        None => (StatusCode::NOT_FOUND, Json(json!({"detail": "Not found."}))).into_response(),
    };
    fake.done(index, response)
}

async fn bulk(
    State(fake): State<FakeDesec>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    Path(zone): Path<String>,
    body: Bytes,
) -> Response {
    let parsed: Option<Value> = serde_json::from_slice(&body).ok();
    let index = match fake.admit(&method, &uri, &headers, parsed.clone()) {
        Ok(index) => index,
        Err(response) => return response,
    };
    let throttled = {
        let mut inner = fake.lock();
        (inner.knobs.throttle_writes > 0).then(|| {
            inner.knobs.throttle_writes -= 1;
            (
                inner.knobs.retry_after.clone(),
                inner.knobs.concurrent_value.take(),
            )
        })
    };
    if let Some((wait, concurrent)) = throttled {
        // Another client writes the set while the hub waits.
        if let Some(value) = concurrent {
            let subname = parsed
                .as_ref()
                .and_then(|body| body[0]["subname"].as_str())
                .unwrap_or_default()
                .to_owned();
            let key = (zone.clone(), subname.clone(), "TXT".to_owned());
            let records = {
                let mut inner = fake.lock();
                let set = inner.rrsets.entry(key).or_insert(RrSet {
                    ttl: 7200,
                    records: Vec::new(),
                });
                set.records.push(format!("\"{value}\""));
                set.records.clone()
            };
            fake.publish(&fqdn(&zone, &subname), unquote(&records));
        }
        let response = (
            StatusCode::TOO_MANY_REQUESTS,
            [(header::RETRY_AFTER, wait.clone())],
            Json(json!({"detail": format!("Request was throttled. Expected available in {wait} seconds.")})),
        )
            .into_response();
        return fake.done(index, response);
    }
    let response = match apply(&fake, &zone, parsed) {
        Ok(publications) => {
            for (name, values) in publications {
                fake.publish(&name, values);
            }
            Json(json!([])).into_response()
        }
        Err(response) => response,
    };
    fake.done(index, response)
}

/// Validates and applies a bulk write; the TXT changes to publish.
fn apply(
    fake: &FakeDesec,
    zone: &str,
    body: Option<Value>,
) -> Result<Vec<(String, Vec<String>)>, Response> {
    let bad = |detail: Value| Err((StatusCode::BAD_REQUEST, Json(detail)).into_response());
    let mut inner = fake.lock();
    let Some(&(_, minimum_ttl)) = inner.domains.iter().find(|(name, _)| name == zone) else {
        return Err((StatusCode::NOT_FOUND, Json(json!({"detail": "Not found."}))).into_response());
    };
    if inner.knobs.forbid_writes {
        return Err((
            StatusCode::FORBIDDEN,
            Json(json!({"detail": "Insufficient token permissions.", "code": "forbidden"})),
        )
            .into_response());
    }
    let Some(Value::Array(sets)) = body else {
        return bad(json!({"non_field_errors": ["Expected a list of items."]}));
    };
    let mut changes = Vec::new();
    for set in &sets {
        let (Some(subname), Some("TXT"), Some(ttl), Some(records)) = (
            set["subname"].as_str(),
            set["type"].as_str(),
            set["ttl"].as_u64(),
            set["records"].as_array(),
        ) else {
            return bad(json!([{"detail": "this fake takes full TXT record sets only"}]));
        };
        let records: Vec<String> = records
            .iter()
            .map(|r| r.as_str().unwrap_or_default().to_owned())
            .collect();
        if records
            .iter()
            .any(|r| r.len() < 2 || !r.starts_with('"') || !r.ends_with('"'))
        {
            return bad(json!([{"records": ["TXT values must be in quotes."]}]));
        }
        let Some(ttl) = u32::try_from(ttl).ok().filter(|ttl| *ttl <= 86400) else {
            return bad(json!([{"ttl": ["Ensure this value is less than or equal to 86400."]}]));
        };
        if ttl < minimum_ttl {
            return bad(json!([{"ttl": [format!(
                "Ensure this value is greater than or equal to {minimum_ttl}."
            )]}]));
        }
        changes.push((subname.to_owned(), ttl, records));
    }
    let mut publications = Vec::new();
    for (subname, ttl, records) in changes {
        let key = (zone.to_owned(), subname.clone(), "TXT".to_owned());
        if records.is_empty() {
            inner.rrsets.remove(&key);
        } else {
            inner.rrsets.insert(
                key,
                RrSet {
                    ttl,
                    records: records.clone(),
                },
            );
        }
        publications.push((fqdn(zone, &subname), unquote(&records)));
    }
    Ok(publications)
}
