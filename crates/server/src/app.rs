//! HTTP server: routers, state and lifecycle of `roedy-hub serve`.
//!
//! Two listeners (docs/plan.md › Server design):
//!
//! - **public** (`ROEDY_LISTEN`): the web app, `/api/v1`, `/healthz` and
//!   `/readyz`, and the gateway for every app's path (`gateway`).
//!   `/internal/…` is 404 here.
//! - **internal** (`ROEDY_INTERNAL_LISTEN`, off when empty): only
//!   `/internal/…` for the apps (docs/app-contract.md › Internal API,
//!   [`internal`]); everything else is 404 there.
//!
//! Taken over from Roedy Mail (`crates/server/src/app.rs`) without the mail
//! specific state and jobs (the hub's are in `jobs`); the graceful shutdown
//! covers both listeners.

use std::{
    future::{Future, IntoFuture as _},
    net::SocketAddr,
    sync::Arc,
    time::Duration,
};

use anyhow::Context as _;
use axum::{
    Router,
    http::{HeaderValue, header},
    middleware,
    response::{IntoResponse as _, Response},
    routing::{any, get},
};
use tokio::{net::TcpListener, sync::watch, task::JoinSet};
use tower_http::trace::TraceLayer;
use url::Url;

use crate::{
    api::{self, health},
    apps::{self, AppTable},
    auth::{challenge::Challenges, password::Hasher},
    config::{Config, TrustProxy},
    crypto::SecretKey,
    db::{self, Db},
    events::{self, Events},
    gateway,
    gateway::Gateway,
    identity::Identities,
    instance::Instance,
    internal, jobs,
    locks::KeyedLocks,
    logging::{self, SETUP_TARGET},
    logo::IMAGE_PATH,
    mailer::{
        self, Mailer,
        templates::{Accent, Brand, MAIL_LOGO_PATH, Plate},
    },
    rate_limit::RateLimiter,
    setup_token,
    signing::{self, KeyStore},
    web::{
        app_icons,
        branding::{
            self, APP_ICON_PATH, MANIFEST_PATH, MASKABLE_ICON_PATH, SMALL_ICON_PATH,
            TOUCH_ICON_PATH,
        },
        security_headers,
        static_files::WebAssets,
    },
};

/// Grace period for running requests after SIGTERM; Docker kills after 10 s.
const SHUTDOWN_GRACE: Duration = Duration::from_secs(8);
/// Then for background tasks (audit entries of failed sign-ins, rehashing).
const BACKGROUND_GRACE: Duration = Duration::from_secs(2);

/// First path segment of the internal API, on both listeners.
const INTERNAL: &str = "/internal";

#[derive(Clone)]
pub struct AppState {
    pub db: Db,
    pub web: Arc<WebAssets>,
    pub hasher: Arc<Hasher>,
    pub limiter: Arc<RateLimiter>,
    /// Sign-ins between password and second factor (`auth::challenge`).
    pub challenges: Arc<Challenges>,
    pub trust_proxy: TrustProxy,
    /// Public address of the hub (`ROEDY_PUBLIC_URL`).
    pub public_url: Arc<Url>,
    /// Key for secrets in the database.
    pub secrets: Arc<SecretKey>,
    /// Sending of the system mails (`mailer`).
    pub mailer: Arc<Mailer>,
    /// Email address changes run one after another per user (`api::email`).
    pub email_locks: Arc<KeyedLocks>,
    /// Requests with the same password reset link run one after another,
    /// until the link is used up (`api::password`).
    pub reset_locks: Arc<KeyedLocks>,
    /// Organization and branding (`instance`).
    pub instance: Arc<Instance>,
    /// Signing keys for identities and events (`signing`); filled at start.
    pub keys: Arc<KeyStore>,
    /// Identity tokens for the apps (`identity`).
    pub identities: Arc<Identities>,
    /// The apps by slug (`apps`); filled at start, reloaded on every change.
    pub apps: Arc<AppTable>,
    /// HTTP client for the apps' health checks (`apps::check_health`).
    pub probe: reqwest::Client,
    /// The gateway's client, session cache and open streams (`gateway`).
    pub gateway: Arc<Gateway>,
    /// Wake-up and timing of the event delivery (`events`); [`events::start`]
    /// runs it.
    pub events: Arc<Events>,
    pub background: Background,
}

/// Tasks that keep running after the response. On shutdown `serve` waits
/// briefly for them ([`Self::finish`]) before the database closes. Tasks can
/// also be started from the database thread (e.g. the mails of a committed
/// change): they run on the runtime that created the set.
#[derive(Clone)]
pub struct Background {
    tasks: Arc<std::sync::Mutex<Option<JoinSet<()>>>>,
    runtime: Option<tokio::runtime::Handle>,
}

impl Default for Background {
    fn default() -> Self {
        Self {
            tasks: Arc::new(std::sync::Mutex::new(Some(JoinSet::new()))),
            runtime: tokio::runtime::Handle::try_current().ok(),
        }
    }
}

impl Background {
    /// Starts `task`; once [`Self::finish`] has begun, the task is dropped
    /// (logged): nothing would wait for it before the database closes.
    pub fn spawn(&self, task: impl Future<Output = ()> + Send + 'static) {
        let mut tasks = self.tasks.lock().unwrap_or_else(|e| e.into_inner());
        let Some(tasks) = tasks.as_mut() else {
            tracing::warn!("background task dropped during shutdown");
            return;
        };
        // Clean up finished ones so the list does not grow.
        while tasks.try_join_next().is_some() {}
        match &self.runtime {
            Some(runtime) => tasks.spawn_on(task, runtime),
            None => tasks.spawn(task),
        };
    }

    /// Closes admission, waits at most `within` for running tasks and aborts
    /// the rest.
    pub async fn finish(&self, within: Duration) {
        let taken = self.tasks.lock().unwrap_or_else(|e| e.into_inner()).take();
        let Some(mut tasks) = taken else {
            return;
        };
        let drained =
            tokio::time::timeout(within, async { while tasks.join_next().await.is_some() {} })
                .await;
        if drained.is_err() {
            tracing::warn!(open = tasks.len(), "background tasks aborted");
        }
    }
}

impl AppState {
    pub fn new(
        db: Db,
        web: Arc<WebAssets>,
        config: &Config,
        public_url: Url,
        secrets: SecretKey,
    ) -> anyhow::Result<Self> {
        Ok(Self {
            db,
            web,
            hasher: Arc::new(Hasher::default()),
            limiter: Arc::new(RateLimiter::default()),
            challenges: Arc::new(Challenges::default()),
            trust_proxy: config.trust_proxy,
            identities: Arc::new(Identities::new(&public_url)),
            gateway: Arc::new(Gateway::new(&public_url)?),
            keys: Arc::new(KeyStore::default()),
            public_url: Arc::new(public_url),
            secrets: Arc::new(secrets),
            mailer: Arc::new(Mailer::new(mailer::http_client()?)),
            email_locks: Arc::new(KeyedLocks::default()),
            reset_locks: Arc::new(KeyedLocks::default()),
            instance: Arc::new(Instance::default()),
            apps: Arc::new(AppTable::default()),
            probe: apps::probe_client()?,
            events: Arc::new(Events::new()?),
            background: Background::default(),
        })
    }
}

impl AppState {
    /// A session ended (sign-out, revoked): the gateway forgets it and
    /// closes its streams, its identity tokens go. Call after the commit.
    pub fn end_session(&self, sid: &str) {
        self.gateway.end_session(sid);
        self.identities.forget_session(sid);
    }

    /// The sessions of a user ended (locked, removed, password changed,
    /// "Sign out all others"), except `keep`. Call after the commit.
    pub fn end_user_sessions(&self, user_id: &str, keep: Option<&str>) {
        self.gateway.end_user(user_id, keep);
        self.identities.forget_user(user_id, keep);
    }

    /// Issuer of the authenticator apps (the name the app shows): the hub's
    /// name from the organization.
    pub fn issuer(&self) -> String {
        self.instance.name()
    }

    /// Branding of the mails from organization and branding, with the drawn
    /// logos – so that a mail's `<img>` and `/branding/mail-logo.png` match.
    pub fn brand(&self) -> Brand {
        let org = self.instance.org();
        let branding = self.instance.branding();
        Brand {
            app: org.hub_name().to_owned(),
            accent: Accent::named(&branding.branding.color),
            plate: Plate::named(&branding.branding.plate),
            logos: branding.logos.clone(),
            org: org.name,
            support: org.email,
            imprint: org.imprint,
            privacy: org.privacy,
        }
    }
}

/// Router of the public listener.
pub fn router(state: AppState) -> Router {
    let api = api::router(state.clone());
    Router::new()
        .route("/healthz", get(health::healthz))
        .route("/readyz", get(health::readyz))
        // Branding files for mails, the sign-in page and installed apps
        // (`web::branding`), without sign-in.
        .route(MAIL_LOGO_PATH, get(branding::mail_logo))
        .route(&format!("{IMAGE_PATH}/{{file}}"), get(branding::image))
        .route(MANIFEST_PATH, get(branding::manifest))
        .route(SMALL_ICON_PATH, get(branding::small_icon))
        .route(APP_ICON_PATH, get(branding::app_icon))
        .route(MASKABLE_ICON_PATH, get(branding::maskable_icon))
        .route(TOUCH_ICON_PATH, get(branding::touch_icon))
        // The apps' icons: public and cacheable images under the API's
        // path, without its middleware (`web::app_icons`).
        .route(app_icons::ICON_PATH, get(app_icons::icon))
        .nest(api::PREFIX, api.clone())
        // `nest` does not cover `/api/v1/` (axum 0.8); the API answers there too.
        .route_service(&format!("{}/", api::PREFIX), api.with_state(state.clone()))
        // The internal API never answers on the public listener.
        .route(INTERNAL, any(internal_not_found))
        .route(&format!("{INTERNAL}/"), any(internal_not_found))
        .route(&format!("{INTERNAL}/{{*path}}"), any(internal_not_found))
        // The apps' paths go to the gateway, everything else is the web app
        // (SPA fallback).
        .fallback(gateway::serve)
        .layer(middleware::from_fn_with_state(
            state.clone(),
            security_headers::apply,
        ))
        .layer(TraceLayer::new_for_http().make_span_with(logging::request_span))
        .with_state(state)
}

/// Router of the internal listener: only the internal API
/// (docs/app-contract.md › Internal API), everything else 404.
pub fn internal_router(state: AppState) -> Router {
    internal::router()
        .fallback(internal_not_found)
        .layer(TraceLayer::new_for_http().make_span_with(logging::request_span))
        .with_state(state)
}

/// `404 {"error": "not_found"}` with `Cache-Control: no-store`.
async fn internal_not_found() -> Response {
    (
        [(header::CACHE_CONTROL, HeaderValue::from_static("no-store"))],
        api::not_found().await,
    )
        .into_response()
}

/// Opens the database, logs the setup token while the initial setup is
/// pending and serves HTTP until SIGTERM or SIGINT.
pub async fn serve(config: Config) -> anyhow::Result<()> {
    // First: as PID 1 in a container the process ignores signals without a handler.
    let shutdown = shutdown_signal()?;
    // Before the database: without an address nothing is created.
    let public_url = config.require_public_url()?;
    let web = Arc::new(WebAssets::new(config.web_dir.as_deref())?);

    let blocking_config = config.clone();
    let (conn, token, (secrets, key_source)) = db::blocking(move || {
        let conn = db::open(&blocking_config.data_dir)?;
        let token = setup_token::ensure(&conn)?;
        // After `open`: the data directory exists by then.
        let key = SecretKey::load(&blocking_config)?;
        Ok((conn, token, key))
    })
    .await?;
    if let Some(token) = token {
        tracing::info!(
            target: SETUP_TARGET,
            "Setup-Token: {token} (valid until the initial setup is complete; \
             show again with `roedy-hub setup-token`)"
        );
    }
    tracing::info!(source = %key_source, "secret key loaded");
    let db = Db::from(conn);

    let state = AppState::new(db.clone(), web, &config, public_url, secrets)?;
    let instance = state.instance.clone();
    db::call(&db, move |conn| instance.load(conn))
        .await
        .context("organization and branding not loaded")?;
    let table = state.apps.clone();
    db::call(&db, move |conn| table.reload(conn))
        .await
        .context("apps not loaded")?;
    // Before the listeners: a request may need an identity at once.
    signing::rotate_now(&db, state.secrets.clone(), &state.keys)
        .await
        .context("signing keys not loaded")?;
    let background = state.background.clone();
    let result = async {
        // Placeholder hash before the first request, so that even the first
        // sign-in of an unknown user takes no longer than that of a known one.
        // It fails only without a random source; then nothing else works
        // either, so the start fails.
        state
            .hasher
            .prepare()
            .await
            .context("placeholder password hash not created")?;
        let listeners = Listeners::bind(&config).await?;
        let delivery = events::start(&state).await?;
        let scheduler = jobs::start(&state);
        let served = run_http(listeners, state, shutdown).await;
        scheduler.shutdown().await;
        delivery.shutdown().await;
        // Before closing the database: audit entries after the last responses.
        background.finish(BACKGROUND_GRACE).await;
        served
    }
    .await;
    // Close even after errors so that SQLite checkpoints the WAL file.
    let closed = db.close().await.context("Database not closed cleanly");
    result?;
    closed?;
    tracing::info!("Roedy Hub stopped");
    Ok(())
}

/// The bound sockets of both listeners.
pub struct Listeners {
    pub public: TcpListener,
    pub internal: Option<TcpListener>,
}

impl Listeners {
    async fn bind(config: &Config) -> anyhow::Result<Self> {
        let public = TcpListener::bind(config.listen)
            .await
            .with_context(|| format!("Address {} not available (ROEDY_LISTEN)", config.listen))?;
        let internal = match config.internal_listen {
            Some(addr) => Some(TcpListener::bind(addr).await.with_context(|| {
                format!("Address {addr} not available (ROEDY_INTERNAL_LISTEN)")
            })?),
            None => None,
        };
        Ok(Self { public, internal })
    }
}

/// Serves both listeners until `shutdown`, then lets open requests finish for
/// at most [`SHUTDOWN_GRACE`].
pub async fn run_http(
    listeners: Listeners,
    state: AppState,
    shutdown: impl Future<Output = ()> + Send + 'static,
) -> anyhow::Result<()> {
    let Listeners { public, internal } = listeners;
    let internal_addr = internal.as_ref().map(TcpListener::local_addr).transpose()?;
    tracing::info!(
        version = env!("CARGO_PKG_VERSION"),
        listen = %public.local_addr()?,
        internal_listen = internal_addr.map_or_else(|| "off".to_owned(), |a| a.to_string()),
        public_url = state.public_url.as_str(),
        web = state.web.describe(),
        trust_proxy = state.trust_proxy.0,
        "Roedy Hub started"
    );
    if internal_addr.is_none() {
        tracing::warn!("internal listener off (ROEDY_INTERNAL_LISTEN is empty): no app can pair");
    }

    // One signal for both servers; `true` once shutting down.
    let (stopping, stopped) = watch::channel(false);
    let graceful = |mut stopped: watch::Receiver<bool>| async move {
        // An error means the sender is gone, which also ends the server.
        let _ = stopped.wait_for(|stop| *stop).await;
    };
    // `client_ip::ClientIp` needs the address of the connection.
    let public_server = axum::serve(
        public,
        router(state.clone()).into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(graceful(stopped.clone()))
    .into_future();
    let gateway = state.gateway.clone();
    let internal_router = internal_router(state);
    let internal_stopped = stopped.clone();
    let internal_server = async move {
        match internal {
            Some(listener) => {
                axum::serve(
                    listener,
                    internal_router.into_make_service_with_connect_info::<SocketAddr>(),
                )
                .with_graceful_shutdown(graceful(internal_stopped))
                .await
            }
            None => Ok(()),
        }
    };
    let servers = async {
        tokio::try_join!(
            async { public_server.await.context("public HTTP server aborted") },
            async {
                internal_server
                    .await
                    .context("internal HTTP server aborted")
            },
        )
        .map(|_| ())
    };
    let deadline = async move {
        shutdown.await;
        let _ = stopping.send(true);
        // Open streams and WebSockets would hold the graceful shutdown up.
        gateway.shutdown();
        tokio::time::sleep(SHUTDOWN_GRACE).await;
    };
    tokio::select! {
        result = servers => result,
        () = deadline => {
            tracing::warn!(grace = ?SHUTDOWN_GRACE, "open requests aborted after the grace period");
            Ok(())
        }
    }
}

/// Installs the handlers right away; the future ends at the first signal.
#[cfg(unix)]
fn shutdown_signal() -> anyhow::Result<impl Future<Output = ()> + Send + 'static> {
    use tokio::signal::unix::{SignalKind, signal};
    let mut terminate = signal(SignalKind::terminate()).context("SIGTERM cannot be handled")?;
    let mut interrupt = signal(SignalKind::interrupt()).context("SIGINT cannot be handled")?;
    Ok(async move {
        tokio::select! {
            _ = terminate.recv() => {}
            _ = interrupt.recv() => {}
        }
        tracing::info!("shutting down, finishing open requests");
    })
}

#[cfg(not(unix))]
fn shutdown_signal() -> anyhow::Result<impl Future<Output = ()> + Send + 'static> {
    Ok(async {
        if tokio::signal::ctrl_c().await.is_err() {
            std::future::pending::<()>().await;
        }
        tracing::info!("shutting down, finishing open requests");
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use axum::{
        body::Body,
        http::{HeaderMap, Method, Request, StatusCode},
    };
    use http_body_util::BodyExt as _;
    use tokio::{
        io::{AsyncReadExt as _, AsyncWriteExt as _},
        net::TcpStream,
    };
    use tower::ServiceExt as _;

    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;
    use crate::{auth::password, healthcheck, web::security_headers::InlineScripts};

    /// State with a fresh database in a temporary directory (kept alive by
    /// the returned guard) and without a web app.
    pub(crate) async fn state() -> (AppState, tempfile::TempDir) {
        state_with_web(WebAssets::None).await
    }

    pub(crate) async fn state_with_web(web: WebAssets) -> (AppState, tempfile::TempDir) {
        let tmp = tempfile::tempdir().unwrap();
        let data_dir = tmp.path().to_path_buf();
        let conn = db::blocking(move || db::open(&data_dir)).await.unwrap();
        let mut state = AppState::new(
            Db::from(conn),
            Arc::new(web),
            &Config::default(),
            Url::parse("https://app.example.com").unwrap(),
            SecretKey::random().unwrap(),
        )
        .unwrap();
        // Low cost so that tests stay fast.
        state.hasher = Arc::new(password::tests::fast());
        signing::rotate_now(&state.db, state.secrets.clone(), &state.keys)
            .await
            .unwrap();
        (state, tmp)
    }

    /// `index.html` with SvelteKit's start script, as `pnpm build` writes it.
    const INDEX_HTML: &str = "<!doctype html><html><head>\
        <link href=\"/_hub/immutable/entry/start.js\" rel=\"modulepreload\"></head>\
        <body><script>start();</script></body></html>";

    /// A web app build in a temporary directory.
    fn web_build() -> (WebAssets, tempfile::TempDir) {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        std::fs::create_dir_all(root.join("_hub/immutable/entry")).unwrap();
        std::fs::write(root.join("index.html"), INDEX_HTML).unwrap();
        std::fs::write(root.join("_hub/immutable/entry/start.js"), "start();").unwrap();
        std::fs::write(root.join("_hub/version.json"), r#"{"version":"1"}"#).unwrap();
        std::fs::write(root.join("robots.txt"), "User-agent: *").unwrap();
        (WebAssets::new(Some(root)).unwrap(), tmp)
    }

    struct Reply {
        status: StatusCode,
        headers: HeaderMap,
        body: String,
    }

    impl Reply {
        fn header(&self, name: &str) -> Option<&str> {
            self.headers.get(name).map(|v| v.to_str().unwrap())
        }
    }

    async fn send(router: &Router, request: Request<Body>) -> Reply {
        let response = router.clone().oneshot(request).await.unwrap();
        let status = response.status();
        let headers = response.headers().clone();
        let body = response.into_body().collect().await.unwrap().to_bytes();
        Reply {
            status,
            headers,
            body: String::from_utf8(body.to_vec()).unwrap(),
        }
    }

    async fn get(router: &Router, uri: &str) -> Reply {
        send(router, Request::get(uri).body(Body::empty()).unwrap()).await
    }

    fn start_script_hash() -> String {
        InlineScripts::of_html(INDEX_HTML.as_bytes()).0.remove(0)
    }

    #[tokio::test(start_paused = true)]
    async fn background_waits_briefly_then_aborts() {
        let done = Arc::new(AtomicUsize::new(0));
        let background = Background::default();
        for secs in [1, 60] {
            let done = done.clone();
            background.spawn(async move {
                tokio::time::sleep(Duration::from_secs(secs)).await;
                done.fetch_add(1, Ordering::SeqCst);
            });
        }
        background.finish(Duration::from_secs(5)).await;
        assert_eq!(
            done.load(Ordering::SeqCst),
            1,
            "the short one, not the long one"
        );
        tokio::time::sleep(Duration::from_secs(120)).await;
        assert_eq!(done.load(Ordering::SeqCst), 1, "the long one is aborted");

        // After the start of `finish` nothing new is admitted.
        let late = done.clone();
        background.spawn(async move {
            late.fetch_add(1, Ordering::SeqCst);
        });
        tokio::time::sleep(Duration::from_secs(1)).await;
        assert_eq!(done.load(Ordering::SeqCst), 1, "dropped after shutdown");
        background.finish(Duration::from_secs(5)).await;
    }

    /// The database thread has no runtime context; its tasks still run.
    #[tokio::test]
    async fn background_accepts_tasks_from_other_threads() {
        let background = Background::default();
        let (done, finished) = tokio::sync::oneshot::channel();
        let from = background.clone();
        std::thread::spawn(move || {
            from.spawn(async move {
                let _ = done.send(());
            });
        })
        .join()
        .unwrap();
        tokio::time::timeout(Duration::from_secs(5), finished)
            .await
            .unwrap()
            .unwrap();
    }

    #[tokio::test]
    async fn health_endpoints() {
        let (state, _tmp) = state().await;
        let db = state.db.clone();
        let app = router(state);
        for path in ["/healthz", "/readyz"] {
            let reply = get(&app, path).await;
            assert_eq!(reply.status, StatusCode::OK, "{path}");
            assert_eq!(reply.body, r#"{"status":"ok"}"#, "{path}");
            assert_eq!(reply.header("cache-control"), Some("no-store"), "{path}");
            assert_eq!(reply.header("content-type"), Some("application/json"));
            assert!(reply.header("content-security-policy").is_some(), "{path}");
        }

        // Without a database the process still responds, but is not ready.
        db.close().await.unwrap();
        let reply = get(&app, "/readyz").await;
        assert_eq!(reply.status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(reply.body, r#"{"status":"unavailable"}"#);
        assert_eq!(reply.header("cache-control"), Some("no-store"));
        assert_eq!(get(&app, "/healthz").await.status, StatusCode::OK);
    }

    #[tokio::test]
    async fn spa_fallback_with_the_start_script_allowed() {
        let (web, _web_tmp) = web_build();
        let (state, _tmp) = state_with_web(web).await;
        let app = router(state);
        let hash = start_script_hash();
        // Pages of the hub.
        for path in [
            "/",
            "/login",
            "/admin/users/01J",
            "/invite/abc.def",
            "/account/",
        ] {
            let reply = get(&app, path).await;
            assert_eq!(reply.status, StatusCode::OK, "{path}");
            assert_eq!(reply.body, INDEX_HTML, "{path}");
            assert_eq!(
                reply.header("content-type"),
                Some("text/html; charset=utf-8")
            );
            assert_eq!(reply.header("cache-control"), Some("no-cache"), "{path}");
            let csp = reply.header("content-security-policy").unwrap();
            assert!(
                csp.contains(&format!("script-src 'self' {hash};")),
                "{path}: {csp}"
            );
            assert!(csp.contains("frame-ancestors 'none'"), "{csp}");
            assert_eq!(reply.header("x-content-type-options"), Some("nosniff"));
            assert_eq!(reply.header("referrer-policy"), Some("no-referrer"));
        }

        // A path an app could have belongs to the gateway: without that app
        // its 404, for a page load the web app showing it.
        let reply = get(&app, "/mail/inbox").await;
        assert_eq!(reply.status, StatusCode::NOT_FOUND);
        assert_eq!(reply.body, r#"{"error":"not_found"}"#);
        let request = Request::get("/mail/inbox")
            .header("sec-fetch-mode", "navigate")
            .body(Body::empty())
            .unwrap();
        let reply = send(&app, request).await;
        assert_eq!(reply.status, StatusCode::NOT_FOUND);
        assert!(
            reply
                .body
                .contains(r#"<meta name="roedy-error" content="not_found">"#),
            "{}",
            reply.body
        );
        assert!(
            reply
                .header("content-security-policy")
                .unwrap()
                .contains(&hash)
        );

        // Revalidation: 304 keeps the CSP with the hash, otherwise the browser
        // would store a CSP that blocks the start script.
        let etag = get(&app, "/login").await.header("etag").unwrap().to_owned();
        let request = Request::get("/login")
            .header("if-none-match", &etag)
            .body(Body::empty())
            .unwrap();
        let reply = send(&app, request).await;
        assert_eq!(reply.status, StatusCode::NOT_MODIFIED);
        assert_eq!(reply.body, "");
        assert!(
            reply
                .header("content-security-policy")
                .unwrap()
                .contains(&hash)
        );

        // Only GET and HEAD.
        let request = Request::post("/login").body(Body::empty()).unwrap();
        let reply = send(&app, request).await;
        assert_eq!(reply.status, StatusCode::METHOD_NOT_ALLOWED);
        assert_eq!(reply.header("allow"), Some("GET, HEAD"));
        let request = Request::head("/login").body(Body::empty()).unwrap();
        assert_eq!(send(&app, request).await.status, StatusCode::OK);
    }

    #[tokio::test]
    async fn hub_assets_are_cached_by_kind() {
        let (web, _web_tmp) = web_build();
        let (state, _tmp) = state_with_web(web).await;
        let app = router(state);

        let reply = get(&app, "/_hub/immutable/entry/start.js").await;
        assert_eq!(reply.status, StatusCode::OK);
        assert_eq!(reply.body, "start();");
        assert_eq!(
            reply.header("content-type"),
            Some("text/javascript; charset=utf-8")
        );
        assert_eq!(
            reply.header("cache-control"),
            Some("public, max-age=31536000, immutable")
        );
        assert!(reply.header("etag").is_some());

        for path in ["/_hub/version.json", "/robots.txt"] {
            let reply = get(&app, path).await;
            assert_eq!(reply.status, StatusCode::OK, "{path}");
            assert_eq!(reply.header("cache-control"), Some("no-cache"), "{path}");
        }

        // A missing asset is 404, never the SPA.
        for path in [
            "/_hub/immutable/entry/missing.js",
            "/_hub/missing",
            "/_hub/",
            "/favicon.ico",
            "/missing.css",
        ] {
            let reply = get(&app, path).await;
            assert_eq!(reply.status, StatusCode::NOT_FOUND, "{path}");
            assert_eq!(reply.header("cache-control"), Some("no-store"), "{path}");
            assert!(!reply.body.contains("<html"), "{path}");
        }
    }

    #[tokio::test]
    async fn api_and_internal_paths_never_get_the_spa() {
        let (web, _web_tmp) = web_build();
        let (state, _tmp) = state_with_web(web).await;
        let app = router(state);
        for path in [
            "/api",
            "/api/",
            "/api/v1",
            "/api/v1/",
            "/api/v1/missing",
            "/api/v2/x",
            "/internal",
            "/internal/",
            "/internal/v1/jwks",
            "/internal/v1/pair",
        ] {
            let reply = get(&app, path).await;
            assert_eq!(reply.status, StatusCode::NOT_FOUND, "{path}");
            assert_eq!(reply.header("cache-control"), Some("no-store"), "{path}");
            if path.starts_with("/internal") || path.starts_with("/api/v1") {
                assert_eq!(reply.body, r#"{"error":"not_found"}"#, "{path}");
            } else {
                assert_eq!(reply.body, "", "{path}");
            }
        }
        // Also for other methods on the internal API.
        let request = Request::builder()
            .method(Method::POST)
            .uri("/internal/v1/pair")
            .body(Body::empty())
            .unwrap();
        assert_eq!(send(&app, request).await.status, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn internal_router_serves_nothing_else() {
        let (web, _web_tmp) = web_build();
        let (state, _tmp) = state_with_web(web).await;
        let internal = internal_router(state);
        for path in [
            "/",
            "/login",
            "/healthz",
            "/readyz",
            "/api/v1/x",
            "/_hub/version.json",
            "/internal/v1/x",
            "/internal/v2/jwks",
            "/internal/v1/jwks/",
        ] {
            let reply = get(&internal, path).await;
            assert_eq!(reply.status, StatusCode::NOT_FOUND, "{path}");
            assert_eq!(reply.body, r#"{"error":"not_found"}"#, "{path}");
            assert_eq!(reply.header("cache-control"), Some("no-store"), "{path}");
        }
    }

    /// `GET path` over a real connection; returns the status line.
    async fn status_line(addr: SocketAddr, path: &str) -> String {
        let mut stream = TcpStream::connect(addr).await.unwrap();
        stream
            .write_all(
                format!("GET {path} HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\r\n")
                    .as_bytes(),
            )
            .await
            .unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).await.unwrap();
        response.lines().next().unwrap_or_default().to_owned()
    }

    #[tokio::test]
    async fn serves_both_listeners_until_shutdown() {
        let (state, _tmp) = state().await;
        let public = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let internal = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let (public_addr, internal_addr) =
            (public.local_addr().unwrap(), internal.local_addr().unwrap());
        let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
        let server = tokio::spawn(run_http(
            Listeners {
                public,
                internal: Some(internal),
            },
            state,
            async {
                let _ = stopped.await;
            },
        ));

        healthcheck::check(public_addr, healthcheck::DEFAULT_TIMEOUT)
            .await
            .unwrap();
        assert_eq!(
            status_line(public_addr, "/internal/v1/jwks").await,
            "HTTP/1.1 404 Not Found"
        );
        assert_eq!(
            status_line(internal_addr, "/healthz").await,
            "HTTP/1.1 404 Not Found"
        );
        assert_eq!(
            status_line(internal_addr, "/internal/v1/jwks").await,
            "HTTP/1.1 200 OK"
        );
        assert!(
            healthcheck::check(internal_addr, healthcheck::DEFAULT_TIMEOUT)
                .await
                .is_err()
        );

        stop.send(()).unwrap();
        tokio::time::timeout(Duration::from_secs(5), server)
            .await
            .expect("both servers stop")
            .unwrap()
            .unwrap();
        assert!(TcpStream::connect(public_addr).await.is_err());
        assert!(TcpStream::connect(internal_addr).await.is_err());
    }

    #[tokio::test]
    async fn serves_without_the_internal_listener() {
        let (state, _tmp) = state().await;
        let public = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = public.local_addr().unwrap();
        let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
        let server = tokio::spawn(run_http(
            Listeners {
                public,
                internal: None,
            },
            state,
            async {
                let _ = stopped.await;
            },
        ));
        healthcheck::check(addr, healthcheck::DEFAULT_TIMEOUT)
            .await
            .unwrap();
        stop.send(()).unwrap();
        tokio::time::timeout(Duration::from_secs(5), server)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
    }

    #[tokio::test]
    async fn binding_names_the_variable() {
        let taken = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = taken.local_addr().unwrap();
        let config = Config {
            listen: addr,
            internal_listen: None,
            ..Config::default()
        };
        let err = Listeners::bind(&config).await.err().unwrap();
        assert_eq!(
            err.to_string(),
            format!("Address {addr} not available (ROEDY_LISTEN)")
        );
        let config = Config {
            listen: "127.0.0.1:0".parse().unwrap(),
            internal_listen: Some(addr),
            ..Config::default()
        };
        let err = Listeners::bind(&config).await.err().unwrap();
        assert_eq!(
            err.to_string(),
            format!("Address {addr} not available (ROEDY_INTERNAL_LISTEN)")
        );
    }

    #[tokio::test]
    async fn serve_needs_the_public_url_before_creating_anything() {
        let tmp = tempfile::tempdir().unwrap();
        let data_dir = tmp.path().join("data");
        let config = Config {
            data_dir: data_dir.clone(),
            ..Config::default()
        };
        let err = serve(config).await.unwrap_err();
        assert!(
            err.to_string().starts_with("ROEDY_PUBLIC_URL is not set"),
            "{err}"
        );
        assert!(!data_dir.exists());
    }

    /// The build embedded with `embed-web` (in CI a placeholder page).
    #[cfg(feature = "embed-web")]
    #[tokio::test]
    async fn serves_the_embedded_web_app() {
        let (state, _tmp) = state_with_web(WebAssets::new(None).unwrap()).await;
        let app = router(state);
        let reply = get(&app, "/login").await;
        assert_eq!(reply.status, StatusCode::OK);
        assert!(
            reply
                .body
                .to_ascii_lowercase()
                .starts_with("<!doctype html>")
        );
        assert_eq!(reply.header("cache-control"), Some("no-cache"));
        assert!(reply.header("etag").unwrap().len() > 60, "SHA-256 ETag");
        assert!(reply.header("content-security-policy").is_some());
    }
}
