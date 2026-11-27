//! Roedy Hub's example app (docs/app-contract.md § 15): a minimal app that
//! implements the contract, for the hub's tests and, later, its end-to-end
//! suite. Not part of the image.
//!
//! It pairs with a code ([`Example::pair`]), sends its manifest
//! ([`Example::send_manifest`]), keeps the hub's JWKS and verifies the
//! identity header with `roedy-hub-contract` ([`Example::identity`],
//! refreshing the JWKS at most every 30 s for an unknown `kid`), reads the
//! users endpoint, accepts events idempotently and catches up on missed ones
//! ([`Example::catch_up`]: at start, and when a delivery skips a `seq`;
//! after `410 events_expired` it continues from `resume_after`). Everything
//! it serves is below its prefix `/<slug>` ([`Example::router`]):
//!
//! | Path | What it does |
//! |---|---|
//! | `/<slug>` | Redirects to `/<slug>/` |
//! | `/<slug>/` | A small page with the signed-in user and the link to the hub |
//! | `/<slug>/healthz`, `/<slug>/readyz` | `200` while running; `readyz` only once paired and while [`Example::set_ready`] allows it |
//! | `/<slug>/whoami` | The verified identity, otherwise `401` |
//! | `/<slug>/echo[/…]` | Any method: method, path, query, headers, identity and body size as JSON |
//! | `/<slug>/sse?count=&interval_ms=` | Server-Sent Events: the first at once, then one per interval; `count=0` runs until the client leaves |
//! | `/<slug>/ws` | WebSocket echo (text and binary); `403` for a foreign `Origin`; held before the `101` while [`Example::hold_websockets`] says so |
//! | `/<slug>/upload` | `POST`: reads the body as a stream, answers its size and SHA-256 |
//! | `/<slug>/download?bytes=&chunk=` | Streams `bytes` bytes (byte `i` is `i % 251`) |
//! | `/<slug>/slow?ms=&body_ms=` | Waits `ms` before the headers and `body_ms` before the body |
//! | `/<slug>/respond?status=&header=Name:%20Value&body=` | A response with this status, these headers (repeatable) and body |
//! | `/<slug>/_roedy/v1/events` | Event delivery (contract § 10.1), `204`; another status or a delay while [`Example::fail_events`] or [`Example::delay_events`] say so |
//!
//! The example keeps its connection to the hub in memory only; a real app
//! stores the secret encrypted with its instance key (contract § 9).

use std::{
    convert::Infallible,
    sync::{
        Arc, Mutex, PoisonError, RwLock,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use axum::{
    Json, Router,
    body::{Body, Bytes},
    extract::{
        DefaultBodyLimit, Query, Request, State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    http::{HeaderMap, HeaderName, HeaderValue, Method, StatusCode, header},
    response::{
        Html, IntoResponse, Redirect, Response,
        sse::{Event as SseEvent, KeepAlive, Sse},
    },
    routing::{any, get, post},
};
use futures_util::{Stream, StreamExt as _, stream};
use roedy_hub_contract::{
    Error as TokenError, Event, Expected, IDENTITY_HEADER, Identity, Jwks,
    pairing::{
        EventPage, ManifestAccepted, PairRequest, Paired, User, Users, is_valid_slug,
        normalize_pairing_code,
    },
    verify,
};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};
use url::Url;

/// The example's manifest id.
pub const MANIFEST_ID: &str = "roedy.example";

/// Unknown `kid`s refresh the JWKS at most this often (contract § 4.3).
const JWKS_REFRESH: Duration = Duration::from_secs(30);

/// The example's manifest (contract § 8).
pub fn manifest() -> Value {
    json!({
        "contract": 1,
        "id": MANIFEST_ID,
        "version": env!("CARGO_PKG_VERSION"),
        "name": { "en": "Example", "de": "Beispiel" },
        "description": {
            "en": "Test app that implements the app contract",
            "de": "Test-App, die den App-Vertrag umsetzt"
        },
        "icon": "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 24 24\"><path d=\"M4 4h16v16H4z\" fill=\"none\" stroke=\"#000\" stroke-width=\"2\"/></svg>",
        "home": "/",
        "roles": [
            { "id": "user", "name": { "en": "User", "de": "Benutzer" },
              "description": { "en": "Uses the example", "de": "Nutzt das Beispiel" },
              "default": true },
            { "id": "admin", "name": { "en": "Administrator", "de": "Administrator" },
              "description": { "en": "Manages the example", "de": "Verwaltet das Beispiel" },
              "admin": true }
        ]
    })
}

/// What pairing gave the app.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Connection {
    pub app_id: String,
    pub slug: String,
    pub issuer: String,
    pub secret: String,
    pub jwks: Jwks,
}

/// An error answer of the hub, or `status` 0 when it was not reachable.
#[derive(Debug, Clone, PartialEq)]
pub struct HubError {
    pub status: u16,
    pub body: Value,
}

impl HubError {
    /// The `error` code of the answer.
    pub fn code(&self) -> Option<&str> {
        self.body.get("error").and_then(Value::as_str)
    }

    fn local(code: &str, detail: impl ToString) -> Self {
        Self {
            status: 0,
            body: json!({ "error": code, "detail": detail.to_string() }),
        }
    }
}

impl std::fmt::Display for HubError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "hub answered {}: {}", self.status, self.body)
    }
}

impl std::error::Error for HubError {}

/// Why a request counts as not signed in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotSignedIn {
    /// No identity header.
    Missing,
    /// More than one identity header.
    Several,
    /// Not paired yet: no keys to verify with.
    Unpaired,
    /// The token failed a check (contract § 4.3).
    Invalid(TokenError),
}

impl NotSignedIn {
    pub fn code(self) -> &'static str {
        match self {
            Self::Missing => "missing",
            Self::Several => "several",
            Self::Unpaired => "unpaired",
            Self::Invalid(error) => error.code(),
        }
    }
}

/// The example app; clones share everything.
#[derive(Clone)]
pub struct Example(Arc<Inner>);

struct Inner {
    /// `ROEDY_PUBLIC_URL` without trailing slash.
    public_url: String,
    slug: String,
    /// Scheme, host and port of the public URL, for `Origin` checks.
    origin: String,
    /// Address of the hub's internal listener, without trailing slash.
    hub: String,
    client: reqwest::Client,
    connection: RwLock<Option<Connection>>,
    jwks_refreshed: Mutex<Option<Instant>>,
    ready: AtomicBool,
    /// Text and binary messages the WebSocket echo received.
    websocket_messages: AtomicUsize,
    /// While `true`, upgrades wait before their `101` ([`Example::hold_websockets`]).
    hold_websockets: tokio::sync::watch::Sender<bool>,
    /// One permit per upgrade that reached the handler while held.
    websocket_held: tokio::sync::Notify,
    events: Mutex<Events>,
    /// Deliveries and catch-ups process one at a time.
    processing: tokio::sync::Mutex<()>,
}

#[derive(Default)]
struct Events {
    /// The highest `seq` processed (a real app stores it with its data).
    highest_seq: u64,
    /// The events processed, in order.
    received: Vec<Event>,
    /// Deliveries of events processed before, acknowledged again.
    repeated: usize,
    /// `resume_after` of the last `410 events_expired`.
    resumed_after: Option<u64>,
    /// Answer deliveries with this status instead of processing them.
    fail_with: Option<u16>,
    /// Wait this long before answering a delivery.
    delay: Option<Duration>,
}

/// What [`Example::catch_up`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CatchUp {
    /// Events processed.
    pub processed: usize,
    /// `resume_after` if the hub no longer had every event the app missed.
    pub resumed_after: Option<u64>,
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}

impl Example {
    /// An unpaired app at `public_url` (`<hub public URL>/<slug>`) that
    /// reaches the hub's internal listener at `hub_internal`.
    pub fn new(public_url: &str, hub_internal: &str) -> anyhow::Result<Self> {
        let url = Url::parse(public_url)?;
        let slug = url.path().trim_matches('/').to_owned();
        anyhow::ensure!(
            is_valid_slug(&slug) && url.path().trim_end_matches('/') == format!("/{slug}"),
            "ROEDY_PUBLIC_URL must be <hub public URL>/<slug>, not {public_url}"
        );
        let hub = Url::parse(hub_internal)?;
        // As the hub: ring for the whole process if no provider is set yet.
        let _ = rustls::crypto::ring::default_provider().install_default();
        let client = reqwest::Client::builder()
            .user_agent(concat!("Roedy-Hub-Example/", env!("CARGO_PKG_VERSION")))
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(30))
            .redirect(reqwest::redirect::Policy::none())
            .build()?;
        Ok(Self(Arc::new(Inner {
            public_url: public_url.trim_end_matches('/').to_owned(),
            slug,
            origin: url.origin().ascii_serialization(),
            hub: hub.as_str().trim_end_matches('/').to_owned(),
            client,
            connection: RwLock::new(None),
            jwks_refreshed: Mutex::new(None),
            ready: AtomicBool::new(true),
            websocket_messages: AtomicUsize::new(0),
            hold_websockets: tokio::sync::watch::Sender::new(false),
            websocket_held: tokio::sync::Notify::new(),
            events: Mutex::new(Events::default()),
            processing: tokio::sync::Mutex::new(()),
        })))
    }

    pub fn slug(&self) -> &str {
        &self.0.slug
    }

    pub fn public_url(&self) -> &str {
        &self.0.public_url
    }

    /// The connection to the hub, once paired.
    pub fn connection(&self) -> Option<Connection> {
        self.0
            .connection
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Uses a connection stored earlier (a real app reads it from its
    /// encrypted storage at start).
    pub fn connect(&self, connection: Connection) {
        *self
            .0
            .connection
            .write()
            .unwrap_or_else(PoisonError::into_inner) = Some(connection);
    }

    /// Whether `readyz` answers `200` (once paired).
    /// Holds WebSocket upgrades before their `101` (`true`) or lets them go
    /// on (`false`), for tests of what happens meanwhile.
    pub fn hold_websockets(&self, hold: bool) {
        self.0.hold_websockets.send_replace(hold);
    }

    /// Waits until a held upgrade has reached the handler.
    pub async fn websocket_held(&self) {
        self.0.websocket_held.notified().await;
    }

    /// How many text and binary messages the WebSocket echo received.
    pub fn websocket_messages(&self) -> usize {
        self.0.websocket_messages.load(Ordering::SeqCst)
    }

    pub fn set_ready(&self, ready: bool) {
        self.0.ready.store(ready, Ordering::SeqCst);
    }

    fn event_state(&self) -> std::sync::MutexGuard<'_, Events> {
        self.0.events.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The events processed so far, in order.
    pub fn events(&self) -> Vec<Event> {
        self.event_state().received.clone()
    }

    /// The highest `seq` processed.
    pub fn highest_seq(&self) -> u64 {
        self.event_state().highest_seq
    }

    /// How many deliveries repeated an event processed before.
    pub fn repeated_events(&self) -> usize {
        self.event_state().repeated
    }

    /// `resume_after` of the last `410 events_expired`, if any.
    pub fn resumed_after(&self) -> Option<u64> {
        self.event_state().resumed_after
    }

    /// Answers deliveries with `status` instead of processing them (`None`:
    /// normal again), as an app does while it reconciles (contract § 10.3).
    pub fn fail_events(&self, status: Option<u16>) {
        self.event_state().fail_with = status;
    }

    /// Waits `delay` before answering a delivery (`None`: at once).
    pub fn delay_events(&self, delay: Option<Duration>) {
        self.event_state().delay = delay;
    }

    /// Processes `event` unless its `seq` was processed before; `true` if
    /// it was new.
    fn apply(&self, event: Event) -> bool {
        let mut state = self.event_state();
        if event.seq <= state.highest_seq {
            state.repeated += 1;
            return false;
        }
        state.highest_seq = event.seq;
        state.received.push(event);
        true
    }

    /// Reads the events after the highest `seq` processed
    /// (`GET /internal/v1/events`), as on every start (contract § 10.3);
    /// after `410 events_expired` it continues from `resume_after` – the
    /// example holds no user data, so reconciling needs nothing more.
    pub async fn catch_up(&self) -> Result<CatchUp, HubError> {
        let _processing = self.0.processing.lock().await;
        self.catch_up_now().await
    }

    async fn catch_up_now(&self) -> Result<CatchUp, HubError> {
        let mut done = CatchUp::default();
        loop {
            let after = self.highest_seq();
            let page = match self
                .call(
                    Method::GET,
                    &format!("/events?after={after}&limit=100"),
                    None,
                )
                .await
            {
                Err(error) if error.status == 410 && error.code() == Some("events_expired") => {
                    let resume = error
                        .body
                        .get("resume_after")
                        .and_then(Value::as_u64)
                        .filter(|resume| *resume > after)
                        .ok_or_else(|| HubError::local("unexpected_answer", &error))?;
                    let mut state = self.event_state();
                    state.highest_seq = resume;
                    state.resumed_after = Some(resume);
                    done.resumed_after = Some(resume);
                    continue;
                }
                result => Self::parse::<EventPage>(result?)?,
            };
            for token in &page.events {
                let event = self
                    .verified::<Event>(token)
                    .await
                    .map_err(|reason| HubError::local("invalid_event", reason.code()))?;
                if self.apply(event) {
                    done.processed += 1;
                }
            }
            if !page.more || page.events.is_empty() {
                return Ok(done);
            }
        }
    }

    async fn call(
        &self,
        method: Method,
        path: &str,
        body: Option<&Value>,
    ) -> Result<Value, HubError> {
        let mut request = self
            .0
            .client
            .request(method, format!("{}/internal/v1{path}", self.0.hub));
        if let Some(connection) = self.connection() {
            request = request.bearer_auth(connection.secret);
        }
        if let Some(body) = body {
            request = request
                .header(header::CONTENT_TYPE, "application/json")
                .body(body.to_string());
        }
        let response = request
            .send()
            .await
            .map_err(|error| HubError::local("unreachable", error))?;
        let status = response.status();
        let bytes = response
            .bytes()
            .await
            .map_err(|error| HubError::local("unreachable", error))?;
        let body: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        if status.is_success() {
            Ok(body)
        } else {
            Err(HubError {
                status: status.as_u16(),
                body,
            })
        }
    }

    fn parse<T: serde::de::DeserializeOwned>(value: Value) -> Result<T, HubError> {
        serde_json::from_value(value).map_err(|error| HubError::local("unexpected_answer", error))
    }

    /// Pairs with `code` (typed with or without spaces and hyphens) and
    /// sends the manifest with it; stores the connection.
    pub async fn pair(&self, code: &str) -> Result<Paired, HubError> {
        self.pair_with(code, &manifest()).await
    }

    /// Like [`Self::pair`] with another manifest (tests).
    pub async fn pair_with(&self, code: &str, manifest: &Value) -> Result<Paired, HubError> {
        let request = PairRequest {
            code: normalize_pairing_code(code).unwrap_or_else(|| code.to_owned()),
            public_url: self.0.public_url.clone(),
            manifest: manifest.clone(),
        };
        let body = serde_json::to_value(&request).map_err(|e| HubError::local("invalid", e))?;
        let paired: Paired = Self::parse(self.call(Method::POST, "/pair", Some(&body)).await?)?;
        if paired.public_url != self.0.public_url || paired.slug != self.0.slug {
            return Err(HubError::local(
                "public_url_mismatch",
                format!("the hub expects {}", paired.public_url),
            ));
        }
        self.connect(Connection {
            app_id: paired.app_id.clone(),
            slug: paired.slug.clone(),
            issuer: paired.issuer.clone(),
            secret: paired.app_secret.clone(),
            jwks: paired.jwks.clone(),
        });
        Ok(paired)
    }

    /// `PUT /internal/v1/manifest`, as on every start.
    pub async fn send_manifest(&self) -> Result<ManifestAccepted, HubError> {
        self.send_manifest_with(&manifest()).await
    }

    /// Like [`Self::send_manifest`] with another manifest (tests).
    pub async fn send_manifest_with(&self, manifest: &Value) -> Result<ManifestAccepted, HubError> {
        let accepted: ManifestAccepted =
            Self::parse(self.call(Method::PUT, "/manifest", Some(manifest)).await?)?;
        if accepted.public_url != self.0.public_url {
            return Err(HubError::local(
                "public_url_mismatch",
                format!("the hub expects {}", accepted.public_url),
            ));
        }
        Ok(accepted)
    }

    /// One page of `GET /internal/v1/users`.
    pub async fn users(
        &self,
        cursor: Option<&str>,
        limit: Option<usize>,
    ) -> Result<Users, HubError> {
        let mut query = url::form_urlencoded::Serializer::new(String::new());
        if let Some(cursor) = cursor {
            query.append_pair("cursor", cursor);
        }
        if let Some(limit) = limit {
            query.append_pair("limit", &limit.to_string());
        }
        let query = query.finish();
        let path = if query.is_empty() {
            "/users".to_owned()
        } else {
            format!("/users?{query}")
        };
        Self::parse(self.call(Method::GET, &path, None).await?)
    }

    /// `GET /internal/v1/users/<sub>`: `404 no_access` and `410
    /// user_deleted` come back as [`HubError`].
    pub async fn user(&self, sub: &str) -> Result<User, HubError> {
        let sub: String = url::form_urlencoded::byte_serialize(sub.as_bytes()).collect();
        Self::parse(
            self.call(Method::GET, &format!("/users/{sub}"), None)
                .await?,
        )
    }

    /// Fetches the JWKS from the hub and keeps it.
    pub async fn refresh_jwks(&self) -> Result<(), HubError> {
        let jwks: Jwks = Self::parse(self.call(Method::GET, "/jwks", None).await?)?;
        *self
            .0
            .jwks_refreshed
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(Instant::now());
        if let Some(connection) = self
            .0
            .connection
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .as_mut()
        {
            connection.jwks = jwks;
        }
        Ok(())
    }

    /// Whether an unknown `kid` may refresh the JWKS now; if so, the
    /// attempt is recorded at once, so that failing or concurrent refreshes
    /// stay within one per 30 s.
    fn may_refresh(&self) -> bool {
        let mut last = self
            .0
            .jwks_refreshed
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if last.is_some_and(|at| at.elapsed() < JWKS_REFRESH) {
            return false;
        }
        *last = Some(Instant::now());
        true
    }

    /// Verifies `token` as claims `C` with the stored keys; an unknown `kid`
    /// refreshes the JWKS once (at most every 30 s) and tries again.
    async fn verified<C: roedy_hub_contract::Claims>(&self, token: &str) -> Result<C, NotSignedIn> {
        let check = |connection: &Connection| {
            verify::<C>(
                token,
                &connection.jwks,
                &Expected {
                    issuer: &connection.issuer,
                    audience: &connection.slug,
                    now: now(),
                },
            )
        };
        let connection = self.connection().ok_or(NotSignedIn::Unpaired)?;
        match check(&connection) {
            Err(TokenError::UnknownKey) if self.may_refresh() => {
                // A failed refresh leaves the keys as they were.
                let _ = self.refresh_jwks().await;
                let connection = self.connection().ok_or(NotSignedIn::Unpaired)?;
                check(&connection).map_err(NotSignedIn::Invalid)
            }
            result => result.map_err(NotSignedIn::Invalid),
        }
    }

    /// The signed-in user of a request (contract § 4.3): exactly one
    /// identity header with a valid token.
    pub async fn identity(&self, headers: &HeaderMap) -> Result<Identity, NotSignedIn> {
        let mut values = headers.get_all(IDENTITY_HEADER).iter();
        let (Some(value), None) = (values.next(), values.next()) else {
            return Err(if headers.contains_key(IDENTITY_HEADER) {
                NotSignedIn::Several
            } else {
                NotSignedIn::Missing
            });
        };
        let token = value
            .to_str()
            .map_err(|_| NotSignedIn::Invalid(TokenError::Malformed))?;
        self.verified::<Identity>(token).await
    }

    /// Everything the example serves, below `/<slug>`.
    pub fn router(&self) -> Router {
        let p = format!("/{}", self.0.slug);
        Router::new()
            .route(&p, get(to_slash))
            .route(&format!("{p}/"), get(home))
            .route(&format!("{p}/healthz"), get(healthz))
            .route(&format!("{p}/readyz"), get(readyz))
            .route(&format!("{p}/whoami"), get(whoami))
            .route(&format!("{p}/echo"), any(echo))
            .route(&format!("{p}/echo/{{*rest}}"), any(echo))
            .route(&format!("{p}/sse"), get(sse))
            .route(&format!("{p}/ws"), get(ws))
            .route(
                &format!("{p}/upload"),
                post(upload).layer(DefaultBodyLimit::disable()),
            )
            .route(&format!("{p}/download"), get(download))
            .route(&format!("{p}/slow"), get(slow))
            .route(&format!("{p}/respond"), any(respond))
            .route(&format!("{p}/_roedy/v1/events"), post(events))
            .with_state(self.clone())
    }
}

/// Serves `example` on `listener` until `shutdown`.
pub async fn serve(
    listener: tokio::net::TcpListener,
    example: &Example,
    shutdown: impl std::future::Future<Output = ()> + Send + 'static,
) -> std::io::Result<()> {
    axum::serve(listener, example.router())
        .with_graceful_shutdown(shutdown)
        .await
}

fn json_error(status: StatusCode, code: &str) -> Response {
    (status, Json(json!({ "error": code }))).into_response()
}

async fn to_slash(State(example): State<Example>) -> Redirect {
    Redirect::permanent(&format!("/{}/", example.slug()))
}

/// The page: who is signed in, and the link to the hub's home page
/// (contract § 5.4).
async fn home(State(example): State<Example>, headers: HeaderMap) -> Response {
    let who = match example.identity(&headers).await {
        Ok(identity) => format!("Signed in as {}", escape(&identity.name)),
        Err(_) => "Not signed in".to_owned(),
    };
    let page = format!(
        "<!doctype html><html lang=\"en\"><meta charset=\"utf-8\"><title>Example</title>\
         <nav><a href=\"/\">Home</a></nav><h1>Example</h1><p>{who}</p></html>"
    );
    (
        [(
            header::CONTENT_SECURITY_POLICY,
            "default-src 'none'; frame-ancestors 'self'",
        )],
        Html(page),
    )
        .into_response()
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

async fn healthz() -> Json<Value> {
    Json(json!({ "status": "ok" }))
}

async fn readyz(State(example): State<Example>) -> Response {
    if example.0.ready.load(Ordering::SeqCst) && example.connection().is_some() {
        Json(json!({ "status": "ok" })).into_response()
    } else {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({ "status": "unavailable" })),
        )
            .into_response()
    }
}

async fn whoami(State(example): State<Example>, headers: HeaderMap) -> Response {
    match example.identity(&headers).await {
        Ok(identity) => Json(identity).into_response(),
        Err(reason) => (
            StatusCode::UNAUTHORIZED,
            Json(json!({ "error": "unauthorized", "reason": reason.code() })),
        )
            .into_response(),
    }
}

async fn echo(State(example): State<Example>, request: Request) -> Response {
    let (parts, body) = request.into_parts();
    let identity = example.identity(&parts.headers).await;
    let headers: Vec<Value> = parts
        .headers
        .iter()
        .map(|(name, value)| json!([name.as_str(), String::from_utf8_lossy(value.as_bytes())]))
        .collect();
    let mut stream = body.into_data_stream();
    let mut body_bytes = 0usize;
    while let Some(chunk) = stream.next().await {
        let Ok(chunk) = chunk else {
            return json_error(StatusCode::BAD_REQUEST, "body");
        };
        body_bytes += chunk.len();
    }
    Json(json!({
        "method": parts.method.as_str(),
        "path": parts.uri.path(),
        "query": parts.uri.query(),
        "headers": headers,
        "identity": identity.as_ref().ok(),
        "identity_error": identity.as_ref().err().map(|reason| reason.code()),
        "body_bytes": body_bytes,
    }))
    .into_response()
}

#[derive(Deserialize)]
struct SseQuery {
    #[serde(default = "three")]
    count: u64,
    #[serde(default = "hundred")]
    interval_ms: u64,
}

const fn three() -> u64 {
    3
}

const fn hundred() -> u64 {
    100
}

async fn sse(
    Query(query): Query<SseQuery>,
) -> Sse<impl Stream<Item = Result<SseEvent, Infallible>>> {
    let interval = Duration::from_millis(query.interval_ms);
    let events = stream::unfold(0u64, move |n| async move {
        if query.count != 0 && n >= query.count {
            return None;
        }
        if n > 0 {
            tokio::time::sleep(interval).await;
        }
        let event = SseEvent::default()
            .id(n.to_string())
            .event("tick")
            .data(format!("tick {n}"));
        Some((Ok(event), n + 1))
    });
    Sse::new(events).keep_alive(KeepAlive::new().interval(Duration::from_secs(15)))
}

async fn ws(
    State(example): State<Example>,
    headers: HeaderMap,
    upgrade: WebSocketUpgrade,
) -> Response {
    let mut hold = example.0.hold_websockets.subscribe();
    if *hold.borrow_and_update() {
        example.0.websocket_held.notify_one();
        // An error means the example is gone.
        let _ = hold.wait_for(|held| !*held).await;
    }
    // Contract § 3.5: the app checks `Origin` as well.
    if let Some(origin) = headers.get(header::ORIGIN)
        && origin.as_bytes() != example.0.origin.as_bytes()
    {
        return json_error(StatusCode::FORBIDDEN, "origin_mismatch");
    }
    upgrade.on_upgrade(move |socket| echo_socket(socket, example))
}

async fn echo_socket(mut socket: WebSocket, example: Example) {
    while let Some(Ok(message)) = socket.recv().await {
        let reply = match message {
            Message::Text(_) | Message::Binary(_) => {
                example.0.websocket_messages.fetch_add(1, Ordering::SeqCst);
                message
            }
            Message::Close(frame) => {
                let _ = socket.send(Message::Close(frame)).await;
                return;
            }
            Message::Ping(_) | Message::Pong(_) => continue,
        };
        if socket.send(reply).await.is_err() {
            return;
        }
    }
}

async fn upload(body: Body) -> Response {
    let mut stream = body.into_data_stream();
    let (mut bytes, mut hash) = (0u64, Sha256::new());
    while let Some(chunk) = stream.next().await {
        let Ok(chunk) = chunk else {
            return json_error(StatusCode::BAD_REQUEST, "body");
        };
        bytes += chunk.len() as u64;
        hash.update(&chunk);
    }
    let digest: String = hash.finalize().iter().map(|b| format!("{b:02x}")).collect();
    Json(json!({ "bytes": bytes, "sha256": digest })).into_response()
}

#[derive(Deserialize)]
struct DownloadQuery {
    #[serde(default = "mebibyte")]
    bytes: u64,
    #[serde(default = "chunk")]
    chunk: usize,
}

const fn mebibyte() -> u64 {
    1024 * 1024
}

const fn chunk() -> usize {
    64 * 1024
}

/// Byte `offset` of every download.
pub fn download_byte(offset: u64) -> u8 {
    u8::try_from(offset % 251).unwrap_or_default()
}

async fn download(Query(query): Query<DownloadQuery>) -> Response {
    let chunk = query.chunk.clamp(1, 1024 * 1024) as u64;
    let total = query.bytes;
    let chunks = stream::unfold(0u64, move |sent| async move {
        if sent >= total {
            return None;
        }
        let len = chunk.min(total - sent);
        let data: Vec<u8> = (sent..sent + len).map(download_byte).collect();
        Some((Ok::<_, Infallible>(Bytes::from(data)), sent + len))
    });
    (
        [(header::CONTENT_TYPE, "application/octet-stream")],
        Body::from_stream(chunks),
    )
        .into_response()
}

#[derive(Deserialize)]
struct SlowQuery {
    #[serde(default)]
    ms: u64,
    #[serde(default)]
    body_ms: u64,
}

async fn slow(Query(query): Query<SlowQuery>) -> Response {
    let cap = 600_000;
    tokio::time::sleep(Duration::from_millis(query.ms.min(cap))).await;
    let body_delay = Duration::from_millis(query.body_ms.min(cap));
    let ms = query.ms;
    let body = stream::once(async move {
        tokio::time::sleep(body_delay).await;
        Ok::<_, Infallible>(Bytes::from(
            json!({ "ms": ms, "body_ms": body_delay.as_millis() }).to_string(),
        ))
    });
    (
        [(header::CONTENT_TYPE, "application/json")],
        Body::from_stream(body),
    )
        .into_response()
}

async fn respond(request: Request) -> Response {
    let query = request.uri().query().unwrap_or_default();
    let mut response = Response::new(Body::empty());
    let mut body = String::new();
    for (key, value) in url::form_urlencoded::parse(query.as_bytes()) {
        match key.as_ref() {
            "status" => match value
                .parse::<u16>()
                .ok()
                .and_then(|s| StatusCode::from_u16(s).ok())
            {
                Some(status) => *response.status_mut() = status,
                None => return json_error(StatusCode::BAD_REQUEST, "status"),
            },
            "header" => {
                let Some((name, value)) = value.split_once(':') else {
                    return json_error(StatusCode::BAD_REQUEST, "header");
                };
                let (Ok(name), Ok(value)) = (
                    HeaderName::try_from(name.trim()),
                    HeaderValue::try_from(value.trim()),
                ) else {
                    return json_error(StatusCode::BAD_REQUEST, "header");
                };
                response.headers_mut().append(name, value);
            }
            "body" => body.push_str(&value),
            _ => {}
        }
    }
    *response.body_mut() = Body::from(body);
    response
}

/// Event delivery (contract § 10.1): verified like an identity with the
/// event `typ`; an event at or below the highest `seq` processed is
/// acknowledged again without acting. A `seq` beyond the next one means the
/// app missed events: it catches up first (`503` if that fails, so that the
/// hub tries again).
async fn events(State(example): State<Example>, body: Bytes) -> Response {
    let (fail_with, delay) = {
        let state = example.event_state();
        (state.fail_with, state.delay)
    };
    if let Some(delay) = delay {
        tokio::time::sleep(delay).await;
    }
    if let Some(status) = fail_with {
        return json_error(
            StatusCode::from_u16(status).unwrap_or(StatusCode::SERVICE_UNAVAILABLE),
            "failing",
        );
    }
    let Ok(token) = std::str::from_utf8(&body) else {
        return json_error(StatusCode::BAD_REQUEST, "malformed");
    };
    let event = match example.verified::<Event>(token.trim()).await {
        Ok(event) => event,
        Err(NotSignedIn::Unpaired) => {
            return json_error(StatusCode::SERVICE_UNAVAILABLE, "unpaired");
        }
        Err(reason) => return json_error(StatusCode::UNAUTHORIZED, reason.code()),
    };
    let _processing = example.0.processing.lock().await;
    if event.seq > example.highest_seq() + 1 && example.catch_up_now().await.is_err() {
        return json_error(StatusCode::SERVICE_UNAVAILABLE, "catching_up");
    }
    example.apply(event);
    StatusCode::NO_CONTENT.into_response()
}

#[cfg(test)]
mod tests {
    use axum::http::Request;
    use http_body_util::BodyExt as _;
    use roedy_hub_contract::{Manifest, SigningKey};
    use tower::ServiceExt as _;

    use super::*;

    const KID: &str = "01JDX3M8Q4V6S2T0R9P7N5K3H1";

    fn example() -> Example {
        Example::new("https://app.example.com/example", "http://127.0.0.1:9").unwrap()
    }

    fn key() -> SigningKey {
        SigningKey::from_seed(KID, &[7; 32]).unwrap()
    }

    fn paired() -> Example {
        let example = example();
        example.connect(Connection {
            app_id: "01JDX4AAAAAAAAAAAAAAAAAAAA".into(),
            slug: "example".into(),
            issuer: "https://app.example.com".into(),
            secret: "rhs_x".into(),
            jwks: Jwks {
                keys: vec![key().jwk()],
            },
        });
        // No refresh against the unreachable hub in these tests.
        *example.0.jwks_refreshed.lock().unwrap() = Some(Instant::now());
        example
    }

    fn identity(aud: &str) -> Identity {
        let now = now();
        Identity {
            iss: "https://app.example.com".into(),
            aud: aud.into(),
            sub: "01JDX2Q7B5W3Y1Z9X7V5T3R1P0".into(),
            username: "alex".into(),
            email: "alex@example.com".into(),
            name: "Alex <Example>".into(),
            role: "user".into(),
            app_role: "user".into(),
            locale: "de".into(),
            sid: "01JDX3A1C3E5G7J9K1M3N5P7Q9".into(),
            auth_time: now - 100,
            iat: now,
            exp: now + 60,
        }
    }

    async fn get(
        example: &Example,
        uri: &str,
        headers: &[(&str, &str)],
    ) -> (StatusCode, HeaderMap, Bytes) {
        let mut request = Request::get(uri);
        for (name, value) in headers {
            request = request.header(*name, *value);
        }
        let response = example
            .router()
            .oneshot(request.body(Body::empty()).unwrap())
            .await
            .unwrap();
        let (parts, body) = response.into_parts();
        (
            parts.status,
            parts.headers,
            body.collect().await.unwrap().to_bytes(),
        )
    }

    #[test]
    fn its_manifest_keeps_the_rules() {
        let manifest = Manifest::parse(&manifest()).unwrap();
        assert_eq!(manifest.id, MANIFEST_ID);
        assert_eq!(manifest.role_for_hub_admins().unwrap().id, "admin");
    }

    #[test]
    fn the_public_url_names_the_slug() {
        assert_eq!(example().slug(), "example");
        assert_eq!(
            Example::new("https://app.example.com/notes/", "http://hub.test")
                .unwrap()
                .public_url(),
            "https://app.example.com/notes"
        );
        for url in [
            "https://app.example.com",
            "https://app.example.com/a/b",
            "https://app.example.com/Mail",
            "not a url",
        ] {
            assert!(Example::new(url, "http://hub.test").is_err(), "{url}");
        }
    }

    #[tokio::test]
    async fn verifies_the_identity() {
        let example = paired();
        let token = key().sign(&identity("example")).unwrap();
        let (status, _, body) =
            get(&example, "/example/whoami", &[(IDENTITY_HEADER, &token)]).await;
        assert_eq!(status, StatusCode::OK);
        let claims: Identity = serde_json::from_slice(&body).unwrap();
        assert_eq!(claims.username, "alex");

        let other = key().sign(&identity("mail")).unwrap();
        for (headers, reason) in [
            (vec![], "missing"),
            (vec![(IDENTITY_HEADER, other.as_str())], "aud"),
            (vec![(IDENTITY_HEADER, "x.y.z")], "malformed"),
            (
                vec![
                    (IDENTITY_HEADER, token.as_str()),
                    (IDENTITY_HEADER, token.as_str()),
                ],
                "several",
            ),
        ] {
            let (status, _, body) = get(&example, "/example/whoami", &headers).await;
            assert_eq!(status, StatusCode::UNAUTHORIZED, "{reason}");
            let body: Value = serde_json::from_slice(&body).unwrap();
            assert_eq!(body["reason"], reason);
        }
        // Without pairing nobody is signed in.
        let unpaired =
            Example::new("https://app.example.com/example", "http://127.0.0.1:9").unwrap();
        let (status, _, body) =
            get(&unpaired, "/example/whoami", &[(IDENTITY_HEADER, &token)]).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert_eq!(
            serde_json::from_slice::<Value>(&body).unwrap()["reason"],
            "unpaired"
        );

        // The page escapes the name.
        let (_, headers, body) = get(&example, "/example/", &[(IDENTITY_HEADER, &token)]).await;
        let page = String::from_utf8(body.to_vec()).unwrap();
        assert!(
            page.contains("Alex &lt;Example&gt;") && page.contains("href=\"/\""),
            "{page}"
        );
        assert!(
            headers[header::CONTENT_SECURITY_POLICY]
                .to_str()
                .unwrap()
                .contains("frame-ancestors")
        );
    }

    #[tokio::test]
    async fn health_and_redirect() {
        let example = example();
        assert_eq!(
            get(&example, "/example/healthz", &[]).await.0,
            StatusCode::OK
        );
        assert_eq!(
            get(&example, "/example/readyz", &[]).await.0,
            StatusCode::SERVICE_UNAVAILABLE,
            "not paired yet"
        );
        let example = paired();
        assert_eq!(
            get(&example, "/example/readyz", &[]).await.0,
            StatusCode::OK
        );
        example.set_ready(false);
        assert_eq!(
            get(&example, "/example/readyz", &[]).await.0,
            StatusCode::SERVICE_UNAVAILABLE
        );
        let (status, headers, _) = get(&example, "/example", &[]).await;
        assert_eq!(status, StatusCode::PERMANENT_REDIRECT);
        assert_eq!(headers[header::LOCATION], "/example/");
        assert_eq!(
            get(&example, "/other/echo", &[]).await.0,
            StatusCode::NOT_FOUND
        );
    }

    #[tokio::test]
    async fn echoes_the_request() {
        let example = paired();
        let (status, _, body) = get(
            &example,
            "/example/echo/a%20b/c?x=1&y=%2F",
            &[("x-test", "one"), ("cookie", "__Host-roedy-example_t=1")],
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let echo: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(echo["method"], "GET");
        assert_eq!(echo["path"], "/example/echo/a%20b/c");
        assert_eq!(echo["query"], "x=1&y=%2F");
        assert!(
            echo["headers"]
                .as_array()
                .unwrap()
                .contains(&json!(["x-test", "one"]))
        );
        assert_eq!(echo["identity"], Value::Null);
        assert_eq!(echo["identity_error"], "missing");
    }

    #[tokio::test]
    async fn streams_and_custom_responses() {
        let example = example();
        let (status, headers, body) =
            get(&example, "/example/sse?count=3&interval_ms=1", &[]).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(headers[header::CONTENT_TYPE], "text/event-stream");
        let text = String::from_utf8(body.to_vec()).unwrap();
        assert_eq!(text.matches("event: tick").count(), 3, "{text}");
        assert!(text.contains("data: tick 2"));

        let (_, _, body) = get(&example, "/example/download?bytes=1000&chunk=300", &[]).await;
        assert_eq!(body.len(), 1000);
        assert!(
            body.iter()
                .enumerate()
                .all(|(i, b)| *b == download_byte(i as u64))
        );

        let (status, headers, body) = get(
            &example,
            "/example/respond?status=201&header=Set-Cookie:%20a%3D1&header=Set-Cookie:%20b%3D2&header=Clear-Site-Data:%20%22cache%22&body=hi",
            &[],
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
        assert_eq!(headers.get_all(header::SET_COOKIE).iter().count(), 2);
        assert_eq!(headers["clear-site-data"], "\"cache\"");
        assert_eq!(body, "hi");

        let (status, _, body) = get(&example, "/example/slow?ms=5&body_ms=5", &[]).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(serde_json::from_slice::<Value>(&body).unwrap()["ms"], 5);
    }

    #[tokio::test]
    async fn counts_uploads() {
        let example = example();
        let data = vec![7u8; 100_000];
        let response = example
            .router()
            .oneshot(
                Request::post("/example/upload")
                    .body(Body::from(data.clone()))
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let body: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(body["bytes"], 100_000);
        let digest: String = Sha256::digest(&data)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        assert_eq!(body["sha256"], digest);
    }

    #[tokio::test]
    async fn accepts_events_once() {
        let example = paired();
        let event = |seq| {
            let now = now();
            key()
                .sign(&Event {
                    iss: "https://app.example.com".into(),
                    aud: "example".into(),
                    jti: format!("01JDX5R2T4V6X8Z0B2D4F6H8J{seq}"),
                    seq,
                    kind: "user.locked".into(),
                    data: json!({ "sub": "01JDX2Q7B5W3Y1Z9X7V5T3R1P0" }),
                    iat: now,
                    exp: now + 300,
                })
                .unwrap()
        };
        let post = |body: String| {
            let router = example.router();
            async move {
                router
                    .oneshot(
                        Request::post("/example/_roedy/v1/events")
                            .header(header::CONTENT_TYPE, "application/jwt")
                            .body(Body::from(body))
                            .unwrap(),
                    )
                    .await
                    .unwrap()
                    .status()
            }
        };
        for seq in [1, 2, 2, 1] {
            assert_eq!(post(event(seq)).await, StatusCode::NO_CONTENT, "{seq}");
        }
        assert_eq!(
            example.events().iter().map(|e| e.seq).collect::<Vec<_>>(),
            [1, 2]
        );
        assert_eq!(example.repeated_events(), 2);
        // An identity is no event.
        let identity = key().sign(&identity("example")).unwrap();
        assert_eq!(post(identity).await, StatusCode::UNAUTHORIZED);
        // A gap needs a catch-up first; without the hub the delivery fails,
        // so that the hub tries again.
        assert_eq!(post(event(4)).await, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(example.highest_seq(), 2);
        // Failing on purpose, as while reconciling.
        example.fail_events(Some(500));
        assert_eq!(post(event(3)).await, StatusCode::INTERNAL_SERVER_ERROR);
        example.fail_events(None);
        assert_eq!(post(event(3)).await, StatusCode::NO_CONTENT);
        assert_eq!(example.highest_seq(), 3);
    }
}
