//! The gateway (docs/plan.md › Gateway, docs/app-contract.md › Routing):
//! every request whose first path segment is an app's slug goes to that
//! app's internal address, path and query unchanged.
//!
//! Per request, [`serve`]:
//!
//! 1. finds the app in the in-memory `apps::AppTable` (a first segment that
//!    cannot be a slug, or a reserved name, stays with the hub's web app);
//!    unknown slugs and `/<slug>/_roedy/…` get `404 not_found`, an app an
//!    administrator switched off `503 app_disabled` – for everyone, before
//!    the session is read;
//! 2. reads the hub session from its cookie through the [`SessionCache`]
//!    (10 s; ending a session clears it at once) and decides: a paired app
//!    gets the request without identity when signed out, with
//!    `X-Roedy-Identity` when the user has access (`app_access`), else
//!    `403 no_app_access`; a pending app only from hub administrators,
//!    without identity, else `503 app_not_ready`;
//! 3. sends it with hyper's pooled client over HTTP/1.1, bodies streamed in
//!    both directions, the headers per [`headers`]; WebSocket upgrades from
//!    the hub's origin are relayed with `copy_bidirectional` after the
//!    app's `101`;
//! 4. answers `502 app_unreachable` when the app does not take the
//!    connection (5 s to connect) and `504 app_timeout` without response
//!    headers within 300 s ([`Limits`]).
//!
//! Errors the gateway answers itself are JSON `{"error": "<code>"}` with
//! `Cache-Control: no-store`, or for page loads the web app with
//! `<meta name="roedy-error">` (contract § 12, `static_files::error_page`).
//! The app's responses are marked [`Proxied`]: the hub's own security
//! headers stay off them.
//!
//! Every request is registered in [`Streams`] before it goes to the app,
//! and stays registered with its response body or WebSocket; ending its
//! session closes it, and switching the app off closes all of the app's
//! ([`Streams::end_app`]) – also a request still uploading or waiting for
//! the app's response headers, which then gets `503 app_disabled`. A
//! request registered while the app is being switched off is stopped by
//! the check right after its registration (the app table is reloaded
//! before the streams end).

pub mod client;
pub mod headers;
pub mod sessions;

use std::{
    collections::HashMap,
    net::SocketAddr,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
    time::Duration,
};

use axum::{
    Json,
    body::{Body, Bytes},
    extract::{ConnectInfo, Request, State},
    http::{HeaderMap, HeaderValue, StatusCode, Uri, Version, header, uri::PathAndQuery},
    response::{IntoResponse as _, Response},
};
use hyper::body::{Body as HttpBody, Frame, Incoming, SizeHint};
use hyper_util::rt::TokioIo;
use percent_encoding::percent_decode_str;
use roedy_hub_contract::pairing::{is_reserved_slug, is_valid_slug};
use serde_json::json;
use url::Url;

use self::{
    client::{AppClient, Upload},
    headers::Forward,
    sessions::{Owner, SessionCache, Signed, Streams, Watch},
};
use crate::{
    app::AppState,
    apps::App,
    auth::session::{self, Token},
    db::{
        self,
        repo::{app_access, apps::Status, sessions as repo},
    },
    identity::Subject,
    settings,
    web::{security_headers::Proxied, static_files},
};

/// The limits of contract § 3.4.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// To connect to the app, then `502 app_unreachable`.
    pub connect: Duration,
    /// Until the app's response headers, then `504 app_timeout`.
    pub response: Duration,
    /// Request headers in total (names and values), then `431`.
    pub request_headers: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            connect: Duration::from_secs(5),
            response: Duration::from_secs(300),
            request_headers: 64 * 1024,
        }
    }
}

/// Client, limits and session state of the gateway.
pub struct Gateway {
    client: AppClient,
    limits: Limits,
    /// Public host (with port, if not the scheme's) for `Host` and
    /// `X-Forwarded-Host`.
    host: HeaderValue,
    /// `https` or `http`, for `X-Forwarded-Proto`.
    proto: HeaderValue,
    /// The hub's public origin, for the `Origin` of WebSocket upgrades.
    origin: String,
    pub sessions: SessionCache,
    pub streams: Arc<Streams>,
}

impl Gateway {
    /// For the hub at `public_url`, with the contract's limits.
    pub fn new(public_url: &Url) -> anyhow::Result<Self> {
        Self::with_limits(public_url, Limits::default())
    }

    pub fn with_limits(public_url: &Url, limits: Limits) -> anyhow::Result<Self> {
        let host = public_url
            .host_str()
            .ok_or_else(|| anyhow::anyhow!("public URL without host"))?;
        let host = match public_url.port() {
            Some(port) => format!("{host}:{port}"),
            None => host.to_owned(),
        };
        Ok(Self {
            client: client::client(limits.connect)?,
            limits,
            host: HeaderValue::try_from(host)?,
            proto: HeaderValue::try_from(public_url.scheme())?,
            origin: public_url.origin().ascii_serialization(),
            sessions: SessionCache::default(),
            streams: Arc::new(Streams::default()),
        })
    }

    pub fn limits(&self) -> Limits {
        self.limits
    }

    /// A session ended: its cache entry and its streams go.
    pub fn end_session(&self, sid: &str) {
        self.sessions.forget_session(sid);
        self.streams.end_session(sid);
    }

    /// The sessions of a user ended, except `keep`.
    pub fn end_user(&self, user_id: &str, keep: Option<&str>) {
        self.sessions.forget_user(user_id, keep);
        self.streams.end_user(user_id, keep);
    }

    /// The server shuts down: every stream ends, so that the graceful
    /// shutdown does not wait for them.
    pub fn shutdown(&self) {
        self.streams.end_all();
    }
}

/// Errors the gateway answers itself (contract § 12).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GatewayError {
    NotFound,
    NoAccess,
    OriginMismatch,
    Unreachable,
    NotReady,
    Disabled,
    Timeout,
    HeadersTooLarge,
    Internal,
}

impl GatewayError {
    pub fn status(self) -> StatusCode {
        match self {
            Self::NotFound => StatusCode::NOT_FOUND,
            Self::NoAccess | Self::OriginMismatch => StatusCode::FORBIDDEN,
            Self::Unreachable => StatusCode::BAD_GATEWAY,
            Self::NotReady | Self::Disabled => StatusCode::SERVICE_UNAVAILABLE,
            Self::Timeout => StatusCode::GATEWAY_TIMEOUT,
            Self::HeadersTooLarge => StatusCode::REQUEST_HEADER_FIELDS_TOO_LARGE,
            Self::Internal => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    pub fn code(self) -> &'static str {
        match self {
            Self::NotFound => "not_found",
            Self::NoAccess => "no_app_access",
            Self::OriginMismatch => "origin_mismatch",
            Self::Unreachable => "app_unreachable",
            Self::NotReady => "app_not_ready",
            Self::Disabled => "app_disabled",
            Self::Timeout => "app_timeout",
            Self::HeadersTooLarge => "headers_too_large",
            Self::Internal => "internal",
        }
    }

    /// Whether the web app has a page for it (`apps/web/src/lib/errors.ts`).
    fn has_page(self) -> bool {
        !matches!(self, Self::HeadersTooLarge)
    }
}

/// The slug a path addresses: its first segment, when it has a slug's
/// format and is not reserved. Everything else belongs to the hub.
pub fn app_slug(path: &str) -> Option<&str> {
    let rest = path.strip_prefix('/')?;
    let first = rest.split('/').next().unwrap_or(rest);
    (is_valid_slug(first) && !is_reserved_slug(first)).then_some(first)
}

/// Whether `path` addresses an app's reserved `/<slug>/_roedy/` (contract
/// § 3.1) – also in the spellings an app's server may normalize to it:
/// percent-encoded, with empty, `.` and `..` segments, backslashes or path
/// parameters, and whichever slug it normalizes to.
pub fn is_reserved_app_path(path: &str) -> bool {
    let decoded = percent_decode_str(path).decode_utf8_lossy();
    let mut segments: Vec<&str> = Vec::new();
    for segment in decoded.split(['/', '\\']) {
        let segment = segment.split(';').next().unwrap_or(segment);
        match segment {
            "" | "." => {}
            ".." => {
                segments.pop();
            }
            _ => segments.push(segment),
        }
    }
    matches!(segments.as_slice(), [_, "_roedy", ..])
}

/// Whether the browser loads a page (contract § 12): `Sec-Fetch-Mode:
/// navigate`, or without `Sec-Fetch-Mode` an `Accept` with `text/html`.
fn is_page_load(headers: &HeaderMap) -> bool {
    match headers.get("sec-fetch-mode") {
        Some(mode) => mode.as_bytes() == b"navigate",
        None => headers
            .get_all(header::ACCEPT)
            .iter()
            .flat_map(|accept| media_types(accept.as_bytes()))
            .any(|media| media.eq_ignore_ascii_case(b"text/html")),
    }
}

/// The media types of an `Accept` value, without their parameters: the
/// list is split at commas outside quoted parameter values (RFC 9110
/// § 5.6.4), each range at its first `;`.
fn media_types(accept: &[u8]) -> Vec<&[u8]> {
    let mut types = Vec::new();
    let (mut start, mut quoted, mut escaped) = (0, false, false);
    for (at, &byte) in accept.iter().enumerate() {
        match byte {
            _ if escaped => escaped = false,
            b'\\' if quoted => escaped = true,
            b'"' => quoted = !quoted,
            b',' if !quoted => {
                types.push(&accept[start..at]);
                start = at + 1;
            }
            _ => {}
        }
    }
    types.push(&accept[start..]);
    types
        .into_iter()
        .map(|range| {
            range
                .split(|&b| b == b';')
                .next()
                .unwrap_or_default()
                .trim_ascii()
        })
        .collect()
}

/// The gateway's own error: the web app's page for page loads, else JSON.
/// `page_load`: [`is_page_load`] of the request.
pub async fn error_response(state: &AppState, page_load: bool, error: GatewayError) -> Response {
    if error.has_page()
        && page_load
        && let Some(page) = static_files::error_page(state, error.status(), error.code()).await
    {
        return page;
    }
    (
        error.status(),
        [(header::CACHE_CONTROL, HeaderValue::from_static("no-store"))],
        Json(json!({ "error": error.code() })),
    )
        .into_response()
}

/// The hub session of a request, as far as the gateway needs it.
struct Session {
    signed: Option<Arc<Signed>>,
    /// `Set-Cookie` for the hub's session cookie: renewed or cleared.
    cookie: Option<HeaderValue>,
}

impl Session {
    fn add_cookie(&self, response: &mut Response) {
        if let Some(cookie) = &self.cookie {
            response
                .headers_mut()
                .append(header::SET_COOKIE, cookie.clone());
        }
    }
}

/// Reads the session from its cookie, through the cache. As the API's
/// `auth::extract::resolve`: an unknown, expired or revoked session or a
/// locked user clears the cookie, an extended session renews it.
async fn session(state: &AppState, headers: &HeaderMap) -> Result<Session, GatewayError> {
    let none = |cookie| Session {
        signed: None,
        cookie,
    };
    let Some(value) = session::cookie_value(headers) else {
        return Ok(none(None));
    };
    let Some(token) = Token::parse(value) else {
        return Ok(none(Some(session::clear_cookie())));
    };
    let hash = token.hash();
    let cache = &state.gateway.sessions;
    if let Some(signed) = cache.get(&hash) {
        return Ok(Session {
            signed: Some(signed),
            cookie: None,
        });
    }
    let generation = cache.generation();
    let found = db::call(&state.db, move |conn| {
        let Some(found) = repo::authenticate(conn, &hash)? else {
            return Ok(None);
        };
        let locale = settings::lang_of(conn, &found.user.id)?;
        let roles = app_access::roles(conn, &found.user.id)?;
        Ok(Some((found, locale, roles)))
    })
    .await
    .map_err(|error| {
        tracing::error!("session not checked: {error:#}");
        GatewayError::Internal
    })?;
    let Some((found, locale, roles)) = found else {
        return Ok(none(Some(session::clear_cookie())));
    };
    let signed = Arc::new(Signed {
        user: found.user,
        locale,
        roles: roles.into_iter().collect::<HashMap<_, _>>(),
    });
    cache.insert(hash, signed.clone(), generation);
    Ok(Session {
        signed: Some(signed),
        cookie: found.touched.then(|| token.cookie(found.persistent)),
    })
}

/// How a request reaches the app.
#[derive(Debug, PartialEq, Eq)]
enum Access<'a> {
    /// Without identity.
    Anonymous,
    /// With an identity carrying this role.
    As(&'a str),
}

/// The decision of contract § 9 (pending apps) and § 11.
fn access<'a>(app: &App, signed: Option<&'a Signed>) -> Result<Access<'a>, GatewayError> {
    match (app.status, signed) {
        (Status::Pending, Some(signed)) if signed.user.is_admin() => Ok(Access::Anonymous),
        (Status::Pending, _) => Err(GatewayError::NotReady),
        (Status::Paired, None) => Ok(Access::Anonymous),
        (Status::Paired, Some(signed)) => signed
            .roles
            .get(&app.id)
            .map(|role| Access::As(role))
            .ok_or(GatewayError::NoAccess),
    }
}

fn header_bytes(headers: &HeaderMap) -> usize {
    headers
        .iter()
        .map(|(name, value)| name.as_str().len() + value.len())
        .sum()
}

/// Router fallback of the public listener: an app's path goes to the app,
/// everything else to the hub's web app.
pub async fn serve(State(state): State<AppState>, request: Request) -> Response {
    let Some(slug) = app_slug(request.uri().path()).map(str::to_owned) else {
        let (parts, _) = request.into_parts();
        return static_files::serve(State(state), parts.method, parts.uri, parts.headers).await;
    };
    let page_load = is_page_load(request.headers());
    if header_bytes(request.headers()) > state.gateway.limits.request_headers {
        return error_response(&state, page_load, GatewayError::HeadersTooLarge).await;
    }
    let app = match state.apps.get(&slug) {
        Some(app) if !is_reserved_app_path(request.uri().path()) => app,
        _ => return error_response(&state, page_load, GatewayError::NotFound).await,
    };
    if !app.enabled {
        return error_response(&state, page_load, GatewayError::Disabled).await;
    }
    // Before the session is read: a revocation from here on until the
    // stream is registered makes `forward` check the session again.
    let generation = state.gateway.sessions.generation();
    let session = match session(&state, request.headers()).await {
        Ok(session) => session,
        Err(error) => return error_response(&state, page_load, error).await,
    };
    let mut response = match forward(&state, &app, &session, generation, request).await {
        Ok(response) => response,
        Err(error) => error_response(&state, page_load, error).await,
    };
    session.add_cookie(&mut response);
    response
}

/// Everything after the session: the decision, the request to the app and
/// its response. `generation`: the session cache's, taken before the
/// session was read.
async fn forward(
    state: &AppState,
    app: &App,
    session: &Session,
    generation: u64,
    mut request: Request,
) -> Result<Response, GatewayError> {
    let slug = app.slug.as_str();
    let gateway = &state.gateway;
    let signed = session.signed.as_deref();
    let role = match access(app, signed)? {
        Access::Anonymous => None,
        Access::As(role) => Some(role),
    };
    let websocket = headers::is_websocket(request.method(), request.headers());
    if websocket
        && request
            .headers()
            .get(header::ORIGIN)
            .is_some_and(|origin| origin.as_bytes() != gateway.origin.as_bytes())
    {
        return Err(GatewayError::OriginMismatch);
    }

    let identity = match (role, signed) {
        (Some(role), Some(signed)) => {
            let subject = Subject::new(&signed.user, role, signed.locale);
            match state
                .identities
                .issue(&state.keys, slug, &subject, jiff::Timestamp::now())
                .and_then(|token| Ok(HeaderValue::try_from(token.as_ref())?))
            {
                Ok(token) => Some(token),
                Err(error) => {
                    tracing::error!(slug, "identity not issued: {error:#}");
                    return Err(GatewayError::Internal);
                }
            }
        }
        _ => None,
    };
    let Some(ConnectInfo(peer)) = request
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .copied()
    else {
        tracing::error!("connection address missing (ConnectInfo)");
        return Err(GatewayError::Internal);
    };
    let Some(request_id) = db::new_id()
        .ok()
        .and_then(|id| HeaderValue::try_from(id).ok())
    else {
        return Err(GatewayError::Internal);
    };
    let forwarded_for = headers::forwarded_for(peer.ip(), request.headers(), state.trust_proxy);
    let forward = Forward {
        slug,
        host: &gateway.host,
        proto: &gateway.proto,
        // Addresses only: always a valid value.
        forwarded_for: HeaderValue::try_from(forwarded_for)
            .unwrap_or_else(|_| HeaderValue::from_static("")),
        request_id: request_id.clone(),
        identity,
        websocket,
    };
    let mut watch = gateway.streams.watch(signed.map_or_else(
        || Owner::anonymous(&app.id),
        |signed| Owner {
            sid: &signed.user.session_id,
            user_id: &signed.user.id,
            app_id: &app.id,
        },
    ));
    // Until the app's response headers: the app alone, so that switching
    // it off still stops the request once its session has ended.
    let mut app_watch = gateway.streams.watch(Owner::anonymous(&app.id));
    // Switched off (or removed) since the app was looked up: its streams
    // were ended before this one was registered, and the table says so.
    let gone = || match state.apps.get(slug) {
        None => Some(GatewayError::NotFound),
        Some(now) if !now.enabled => Some(GatewayError::Disabled),
        Some(_) => None,
    };
    if let Some(error) = gone() {
        return Err(error);
    }
    let outgoing = headers::request_headers(request.headers(), &forward);
    let uri = app_uri(app, request.uri()).ok_or(GatewayError::Internal)?;
    let client_upgrade = websocket.then(|| hyper::upgrade::on(&mut request));
    let (parts, body) = request.into_parts();
    let (body, sent) = Upload::new(if websocket { Body::empty() } else { body });
    let mut to_app = axum::http::Request::new(body);
    *to_app.method_mut() = parts.method;
    *to_app.uri_mut() = uri;
    *to_app.version_mut() = Version::HTTP_11;
    *to_app.headers_mut() = outgoing;

    let respond = client::respond(
        gateway.client.request(to_app),
        sent,
        gateway.limits.response,
    );
    tokio::pin!(respond);
    // Switched off or removed while the request was on its way: dropping it
    // ends the upload and the connection to the app. Checked before a
    // response that is ready at the same moment.
    let stopped = |error: GatewayError| {
        tracing::debug!(
            slug,
            request_id = request_id.to_str().unwrap_or_default(),
            "request stopped: app switched off or removed"
        );
        Err(error)
    };
    let mut session_ended = false;
    let answer = loop {
        tokio::select! {
            biased;
            () = app_watch.ended() => match gone() {
                Some(error) => return stopped(error),
                // The server shuts down: the response still goes out, and
                // its body ends at once.
                None => break (&mut respond).await,
            },
            () = watch.ended(), if !session_ended => match gone() {
                Some(error) => return stopped(error),
                // The session ended: the response still goes out, and its
                // body ends at once – unless the app is switched off first.
                None => session_ended = true,
            },
            answer = &mut respond => break answer,
        }
    };
    drop(app_watch);
    let mut from_app = match answer {
        Some(Ok(response)) => response,
        Some(Err(error)) => {
            tracing::warn!(
                slug,
                request_id = request_id.to_str().unwrap_or_default(),
                "app not reachable: {}",
                chain(&error)
            );
            return Err(GatewayError::Unreachable);
        }
        None => {
            tracing::warn!(
                slug,
                request_id = request_id.to_str().unwrap_or_default(),
                timeout = ?gateway.limits.response,
                "app sent no response in time"
            );
            return Err(GatewayError::Timeout);
        }
    };

    let switching = from_app.status() == StatusCode::SWITCHING_PROTOCOLS;
    if switching && !websocket {
        tracing::warn!(
            slug,
            request_id = request_id.to_str().unwrap_or_default(),
            "app switched protocols without an upgrade"
        );
        return Err(GatewayError::Unreachable);
    }
    // A session that ended, or an access that went, after it was read and
    // before this request was registered had no stream to close yet.
    if let Some(signed) = signed
        && gateway.sessions.generation() != generation
    {
        still_valid(state, app, signed).await;
    }
    let app_upgrade = switching.then(|| hyper::upgrade::on(&mut from_app));
    let (mut parts, incoming) = from_app.into_parts();
    let body = match (client_upgrade, app_upgrade) {
        (Some(client_upgrade), Some(app_upgrade)) => {
            tokio::spawn(tunnel(client_upgrade, app_upgrade, watch, slug.to_owned()));
            Body::empty()
        }
        _ => Body::new(Watched {
            inner: incoming,
            watch,
        }),
    };
    let (headers, dropped) = headers::response_headers(
        std::mem::take(&mut parts.headers),
        slug,
        switching,
        state.hsts(),
    );
    for cookie in dropped {
        tracing::warn!(slug, cookie, "cookie outside the app's namespace dropped");
    }
    let mut response = Response::new(body);
    *response.status_mut() = parts.status;
    *response.headers_mut() = headers;
    response.extensions_mut().insert(Proxied);
    Ok(response)
}

/// A session ended, or the user lost the access to the app, while its
/// request waited for the app – before its stream was registered, so
/// nothing closed it: checks the session and the access and ends the
/// session (and the new stream), or the user's streams to the app, if they
/// no longer hold. If the check fails, the stream ends too.
async fn still_valid(state: &AppState, app: &App, signed: &Signed) {
    let sid = signed.user.session_id.as_str();
    let ids = vec![sid.to_owned()];
    let (user_id, app_id) = (signed.user.id.clone(), app.id.clone());
    // A pending app has no access entries: hub administrators reach it.
    let paired = app.status == Status::Paired;
    let valid = db::call(&state.db, move |conn| {
        let session = repo::valid_among(conn, &ids)?;
        let access = !paired || app_access::role_of(conn, &user_id, &app_id)?.is_some();
        Ok((session, access))
    })
    .await;
    match valid {
        Ok((valid, access)) if valid.contains(sid) => {
            if !access {
                state.gateway.streams.end_user_app(&signed.user.id, &app.id);
            }
        }
        Ok(_) => state.end_session(sid),
        Err(error) => {
            tracing::warn!("session not checked again, stream closed: {error:#}");
            state.end_session(sid);
        }
    }
}

/// `<internal address><path and query>`, both unchanged.
fn app_uri(app: &App, uri: &Uri) -> Option<Uri> {
    let path = uri
        .path_and_query()
        .map_or_else(|| uri.path(), PathAndQuery::as_str);
    format!("{}{path}", app.internal_url.as_str().trim_end_matches('/'))
        .parse()
        .ok()
}

/// The error and its sources in one line.
fn chain(error: &dyn std::error::Error) -> String {
    let mut text = error.to_string();
    let mut source = error.source();
    while let Some(cause) = source {
        text.push_str(": ");
        text.push_str(&cause.to_string());
        source = cause.source();
    }
    text
}

/// Relays a WebSocket in both directions until one side closes, the
/// session ends or the server shuts down. The hub does not read the frames.
async fn tunnel(
    client: hyper::upgrade::OnUpgrade,
    app: hyper::upgrade::OnUpgrade,
    mut watch: Watch,
    slug: String,
) {
    let (client, app) = match tokio::try_join!(client, app) {
        Ok(upgraded) => upgraded,
        Err(error) => {
            tracing::debug!(slug, "WebSocket not upgraded: {error}");
            return;
        }
    };
    let (mut client, mut app) = (TokioIo::new(client), TokioIo::new(app));
    tokio::select! {
        // The end of the session first: a session that ended before the
        // tunnel started relays nothing.
        biased;
        () = watch.ended() => tracing::debug!(slug, "WebSocket closed: session ended"),
        relayed = tokio::io::copy_bidirectional(&mut client, &mut app) => {
            if let Err(error) = relayed {
                tracing::debug!(slug, "WebSocket ended: {error}");
            }
        }
    }
}

/// An app's response body as it arrives, frame by frame; it breaks off when
/// the session ends (or the server shuts down), which closes the
/// connection to the browser.
struct Watched {
    inner: Incoming,
    watch: Watch,
}

#[derive(Debug)]
struct SessionEnded;

impl std::fmt::Display for SessionEnded {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("session ended")
    }
}

impl std::error::Error for SessionEnded {}

impl HttpBody for Watched {
    type Data = Bytes;
    type Error = axum::BoxError;

    fn poll_frame(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Bytes>, Self::Error>>> {
        let this = self.get_mut();
        if this.watch.poll_ended(cx).is_ready() {
            return Poll::Ready(Some(Err(Box::new(SessionEnded))));
        }
        Pin::new(&mut this.inner)
            .poll_frame(cx)
            .map_err(axum::BoxError::from)
    }

    fn is_end_stream(&self) -> bool {
        self.inner.is_end_stream()
    }

    fn size_hint(&self) -> SizeHint {
        self.inner.size_hint()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugs_from_paths() {
        let table: &[(&str, Option<&str>)] = &[
            ("/mail", Some("mail")),
            ("/mail/", Some("mail")),
            ("/mail/api/v1/messages", Some("mail")),
            ("/mail-archive/x", Some("mail-archive")),
            ("/", None),
            ("", None),
            // Reserved names and what cannot be a slug stay with the hub.
            ("/login", None),
            ("/admin/apps", None),
            ("/api/v1/session", None),
            ("/_hub/immutable/app.js", None),
            ("/favicon.ico", None),
            ("/manifest.webmanifest", None),
            ("/Mail/", None),
            ("/%6Dail/", None),
            ("/m/", None),
            ("/a.b/", None),
        ];
        for (path, expected) in table {
            assert_eq!(app_slug(path), *expected, "{path}");
        }
    }

    #[test]
    fn reserved_app_paths() {
        let table: &[(&str, bool)] = &[
            ("/mail/_roedy/v1/events", true),
            ("/mail/_roedy", true),
            ("/mail/_roedy/", true),
            ("/mail//_roedy/v1/events", true),
            ("/mail/./_roedy/v1/events", true),
            ("/mail/x/../_roedy/v1/events", true),
            ("/mail/%5Froedy/v1/events", true),
            ("/mail/%5froedy/v1/events", true),
            ("/mail\\_roedy\\v1\\events", true),
            ("/mail/_roedy;x=1/v1/events", true),
            ("/mail/../mail/_roedy/v1/events", true),
            // Normalized into another prefix: refused as well.
            ("/mail/../other/_roedy/x", true),
            ("/mail/x/../../other/_roedy", true),
            ("/mail/_roedyx", false),
            ("/mail/_ROEDY/v1/events", false),
            ("/mail/api/_roedy", false),
            ("/mail/x/_roedy", false),
            ("/mail/", false),
            ("/mail", false),
        ];
        for (path, expected) in table {
            assert_eq!(is_reserved_app_path(path), *expected, "{path}");
        }
    }

    #[test]
    fn page_loads() {
        let table: &[(&[(&str, &str)], bool)] = &[
            (&[("sec-fetch-mode", "navigate")], true),
            (&[("sec-fetch-mode", "navigate"), ("accept", "*/*")], true),
            (
                &[("sec-fetch-mode", "cors"), ("accept", "text/html")],
                false,
            ),
            (&[("accept", "text/html,application/xhtml+xml")], true),
            (&[("accept", "application/json")], false),
            (&[("accept", "application/json, TEXT/HTML;q=0.9")], true),
            (
                &[("accept", "application/json;profile=\"text/html\"")],
                false,
            ),
            (&[("accept", "text/html-fragment")], false),
            // A comma inside a quoted parameter does not start a new type.
            (
                &[("accept", "application/json;profile=\"foo,text/html;bar\"")],
                false,
            ),
            (
                &[(
                    "accept",
                    "application/json;p=\"a\\\",text/html\", text/html",
                )],
                true,
            ),
            (
                &[("accept", "application/json;p=\"a\\\",text/html\"")],
                false,
            ),
            (&[], false),
        ];
        for (pairs, expected) in table {
            let mut headers = HeaderMap::new();
            for (name, value) in *pairs {
                headers.append(*name, HeaderValue::from_static(value));
            }
            assert_eq!(is_page_load(&headers), *expected, "{pairs:?}");
        }
    }

    fn app(status: Status) -> App {
        App {
            id: "app1".into(),
            slug: "mail".into(),
            internal_url: "http://roedy-mail:8080".parse().unwrap(),
            status,
            manifest: None,
            access_default: crate::db::repo::apps::AccessDefault::All,
            health: None,
            enabled: true,
            icon_version: None,
        }
    }

    fn signed(role: crate::db::repo::users::Role, app_role: Option<&str>) -> Signed {
        Signed {
            user: crate::auth::session::AuthUser {
                id: "u1".into(),
                username: "max".into(),
                email: "max@example.com".into(),
                display_name: "Max Example".into(),
                role,
                session_id: "s1".into(),
                auth_time: 1_795_600_000,
            },
            locale: crate::settings::Language::English,
            roles: app_role
                .map(|role| HashMap::from([("app1".to_owned(), role.to_owned())]))
                .unwrap_or_default(),
        }
    }

    #[test]
    fn access_per_app_state_and_user() {
        use crate::db::repo::users::Role::{Admin, User};
        let admin_with = signed(Admin, Some("admin"));
        let admin_without = signed(Admin, None);
        let user_with = signed(User, Some("user"));
        let user_without = signed(User, None);
        let table: &[(Status, Option<&Signed>, Result<Access<'_>, GatewayError>)] = &[
            (Status::Paired, None, Ok(Access::Anonymous)),
            (Status::Paired, Some(&user_with), Ok(Access::As("user"))),
            (Status::Paired, Some(&admin_with), Ok(Access::As("admin"))),
            (
                Status::Paired,
                Some(&user_without),
                Err(GatewayError::NoAccess),
            ),
            // The hub role alone gives no access.
            (
                Status::Paired,
                Some(&admin_without),
                Err(GatewayError::NoAccess),
            ),
            (Status::Pending, None, Err(GatewayError::NotReady)),
            (
                Status::Pending,
                Some(&user_with),
                Err(GatewayError::NotReady),
            ),
            (Status::Pending, Some(&admin_without), Ok(Access::Anonymous)),
            (Status::Pending, Some(&admin_with), Ok(Access::Anonymous)),
        ];
        for (status, signed, expected) in table {
            assert_eq!(
                access(&app(*status), *signed),
                *expected,
                "{status:?} {:?}",
                signed.map(|s| (s.user.role, s.roles.len()))
            );
        }
    }

    #[test]
    fn errors_and_their_codes() {
        let table = [
            (GatewayError::NotFound, 404, "not_found"),
            (GatewayError::NoAccess, 403, "no_app_access"),
            (GatewayError::OriginMismatch, 403, "origin_mismatch"),
            (GatewayError::Unreachable, 502, "app_unreachable"),
            (GatewayError::NotReady, 503, "app_not_ready"),
            (GatewayError::Disabled, 503, "app_disabled"),
            (GatewayError::Timeout, 504, "app_timeout"),
            (GatewayError::HeadersTooLarge, 431, "headers_too_large"),
            (GatewayError::Internal, 500, "internal"),
        ];
        for (error, status, code) in table {
            assert_eq!((error.status().as_u16(), error.code()), (status, code));
        }
    }
}
