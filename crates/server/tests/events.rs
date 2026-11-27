//! Events over real connections with the example app: which change gives
//! which event to which app, delivery in order with backoff (app down, 5xx,
//! timeout), giving up and resuming, redelivery, catching up with `410`,
//! the example's checks of forged events and the removal of an app.
//!
//! The delivery here looks again only after 30 s without a wake-up, so
//! every event that arrives sooner shows the wake-up after the commit.

mod common;

use std::{net::SocketAddr, sync::Arc, time::Duration};

use axum::{
    Router,
    body::Body,
    extract::connect_info::MockConnectInfo,
    http::{Method, Request, StatusCode, header},
};
use common::{Env, PASSWORD, Reply, env};
use http_body_util::BodyExt as _;
use roedy_hub_contract::{Event, Identity, SigningKey, pairing::EventPage};
use roedy_hub_example::Example;
use roedy_hub_server::{
    app::internal_router,
    db::repo::{events as repo, users::Role},
    events::{self, Delivery, Events, Schedule},
    jobs::{event_retention, session_cleanup},
    signing,
};
use serde_json::{Value, json};
use tokio::net::TcpListener;
use tower::ServiceExt as _;

const ORIGIN: &str = "https://app.example.com";

fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}

/// Fast retries; a wake-up is the only way to see a fresh event quickly.
fn fast() -> Schedule {
    Schedule {
        delays: vec![ms(50), ms(100), ms(200)],
        give_up: Duration::from_secs(3600),
        connect_timeout: ms(500),
        timeout: ms(400),
        recheck: Duration::from_secs(30),
    }
}

/// A hub with its internal listener and the delivery running, the example
/// app paired at `/example` (access for everyone), `anna` an administrator
/// (signed in), `max` and `eva` users.
struct Hub {
    env: Env,
    internal: String,
    example: Example,
    app_id: String,
    anna: String,
    _delivery: Delivery,
}

async fn hub() -> Hub {
    hub_with(fast()).await
}

async fn hub_with(schedule: Schedule) -> Hub {
    let mut env = env().await;
    env.state.events = Arc::new(Events::with_schedule(schedule).unwrap());
    signing::rotate_now(&env.state.db, env.state.secrets.clone(), &env.state.keys)
        .await
        .unwrap();
    env.add_user("anna", "anna@example.com", Role::Admin).await;
    env.add_user("max", "max@example.com", Role::User).await;
    env.add_user("eva", "eva@example.com", Role::User).await;
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let internal = format!("http://{}", listener.local_addr().unwrap());
    let router = internal_router(env.state.clone());
    tokio::spawn(async move {
        axum::serve(
            listener,
            router.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
        .unwrap();
    });
    let delivery = events::start(&env.state).await.unwrap();
    let anna = env.session_for("anna").await;
    let mut hub = Hub {
        env,
        internal,
        example: Example::new(&format!("{ORIGIN}/placeholder"), "http://127.0.0.1:9").unwrap(),
        app_id: String::new(),
        anna,
        _delivery: delivery,
    };
    let (example, _) = hub.app("example", true).await;
    hub.app_id = example.connection().unwrap().app_id;
    hub.example = example;
    hub
}

/// Serves `example` on `listener`.
fn serve(example: &Example, listener: TcpListener) {
    let served = example.clone();
    tokio::spawn(async move {
        roedy_hub_example::serve(listener, &served, std::future::pending())
            .await
            .unwrap();
    });
}

impl Hub {
    /// Adds and pairs an example app at `/slug`; served on a free port or,
    /// without `serving`, at a port nothing listens on (returned, to serve
    /// later).
    async fn app(&self, slug: &str, serving: bool) -> (Example, SocketAddr) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let example = Example::new(&format!("{ORIGIN}/{slug}"), &self.internal).unwrap();
        if serving {
            serve(&example, listener);
        } else {
            drop(listener);
        }
        let code = self.add(slug, &format!("http://{addr}")).await;
        example.pair(&code).await.unwrap();
        (example, addr)
    }

    /// Adds an app (pending); returns the pairing code.
    async fn add(&self, slug: &str, internal_url: &str) -> String {
        let reply = self
            .admin(
                Method::POST,
                "/admin/apps",
                Some(json!({ "slug": slug, "internal_url": internal_url })),
            )
            .await;
        assert_eq!(reply.status, StatusCode::CREATED, "{}", reply.body);
        reply.json()["pairing"]["code"].as_str().unwrap().to_owned()
    }

    async fn admin(&self, method: Method, path: &str, body: Option<Value>) -> Reply {
        self.env.as_user(&self.anna, method, path, body).await
    }

    async fn user_id(&self, username: &'static str) -> String {
        self.env
            .sql(move |c| {
                c.query_row(
                    "SELECT id FROM users WHERE username = ?1",
                    [username],
                    |r| r.get(0),
                )
            })
            .await
    }

    async fn app_id(&self, slug: &'static str) -> String {
        self.env
            .sql(move |c| c.query_row("SELECT id FROM apps WHERE slug = ?1", [slug], |r| r.get(0)))
            .await
    }

    /// The stored events of an app as `(type, data)`, oldest first.
    async fn stored(&self, app_id: &str) -> Vec<(String, Value)> {
        let app_id = app_id.to_owned();
        self.env
            .sql(move |c| {
                c.prepare("SELECT type, data FROM events WHERE app_id = ?1 ORDER BY seq")?
                    .query_map([app_id], |r| {
                        let data: String = r.get(1)?;
                        Ok((r.get(0)?, serde_json::from_str(&data).unwrap()))
                    })?
                    .collect()
            })
            .await
    }

    async fn state(&self, app_id: &str) -> repo::State {
        let app_id = app_id.to_owned();
        self.env
            .sql(move |c| repo::state(c, &app_id))
            .await
            .expect("no delivery state")
    }

    /// `GET /internal/v1/events…` with the example's secret.
    async fn catch_up(&self, query: &str) -> Reply {
        let secret = self.example.connection().unwrap().secret;
        internal(&self.env, &format!("/events{query}"), Some(&secret)).await
    }
}

/// A request to the internal router without network.
async fn internal(env: &Env, path: &str, secret: Option<&str>) -> Reply {
    let mut builder = Request::get(format!("/internal/v1{path}"));
    if let Some(secret) = secret {
        builder = builder.header(header::AUTHORIZATION, format!("Bearer {secret}"));
    }
    let router: Router = internal_router(env.state.clone())
        .layer(MockConnectInfo(SocketAddr::from(([10, 0, 0, 7], 4711))));
    let response = router
        .oneshot(builder.body(Body::empty()).unwrap())
        .await
        .unwrap();
    let (status, headers) = (response.status(), response.headers().clone());
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    Reply {
        status,
        headers,
        body: String::from_utf8(bytes.to_vec()).unwrap(),
    }
}

/// Waits up to 5 s for `done`.
async fn until(what: &str, mut done: impl FnMut() -> bool) {
    for _ in 0..500 {
        if done() {
            return;
        }
        tokio::time::sleep(ms(10)).await;
    }
    panic!("timed out waiting for {what}");
}

/// Like [`until`] for a condition that needs the database.
async fn until_async<F: Future<Output = bool>>(what: &str, mut done: impl FnMut() -> F) {
    for _ in 0..500 {
        if done().await {
            return;
        }
        tokio::time::sleep(ms(10)).await;
    }
    panic!("timed out waiting for {what}");
}

/// The example's events after `seq` as `(type, data)`, once it has `n`.
async fn arrived(example: &Example, after: u64, n: usize) -> Vec<(String, Value)> {
    until(&format!("{n} events after {after}"), || {
        example.highest_seq() >= after + n as u64
    })
    .await;
    let events = example.events();
    let mut seqs: Vec<u64> = events.iter().map(|e| e.seq).collect();
    let sorted = {
        let mut s = seqs.clone();
        s.sort_unstable();
        s
    };
    assert_eq!(seqs, sorted, "in order");
    seqs.dedup();
    assert_eq!(seqs.len(), events.len(), "each once");
    events
        .into_iter()
        .filter(|e| e.seq > after)
        .map(|e| (e.kind, e.data))
        .collect()
}

/// `(type, data)` sorted, for events of several users in one change.
fn sorted(mut events: Vec<(String, Value)>) -> Vec<(String, Value)> {
    events.sort_by_key(|e| e.1.to_string());
    events
}

fn access(sub: &str, role: &str) -> (String, Value) {
    (
        "access.changed".into(),
        json!({ "sub": sub, "access": true, "app_role": role }),
    )
}

fn of_user(kind: &str, sub: &str) -> (String, Value) {
    (kind.into(), json!({ "sub": sub }))
}

/// `session.revoked` for `sub` with `reason`, whatever the `sid`.
fn revoked(event: &(String, Value), sub: &str, reason: &str) -> bool {
    event.0 == "session.revoked"
        && event.1["sub"] == sub
        && event.1["reason"] == reason
        && event.1["sid"].as_str().is_some_and(|sid| sid.len() == 26)
}

fn updated(sub: &str, username: &str, name: &str, role: &str, locale: &str) -> (String, Value) {
    (
        "user.updated".into(),
        json!({
            "sub": sub, "username": username, "email": format!("{username}@example.com"),
            "name": name, "role": role, "locale": locale,
        }),
    )
}

#[tokio::test]
async fn every_change_reaches_the_apps_it_concerns() {
    let hub = hub().await;
    let (anna, max, eva) = (
        hub.user_id("anna").await,
        hub.user_id("max").await,
        hub.user_id("eva").await,
    );
    let example = &hub.example;

    // Pairing: the defaults of contract § 11, each as `access.changed`.
    let events = arrived(example, 0, 3).await;
    assert_eq!(
        sorted(events),
        sorted(vec![
            access(&anna, "admin"),
            access(&max, "user"),
            access(&eva, "user")
        ])
    );

    // A second app for administrators only, which is down, and a pending one.
    let second_code = hub.add("second", "http://127.0.0.1:9").await;
    hub.env
        .sql(|c| {
            c.execute(
                "UPDATE apps SET access_default = 'selected' WHERE slug = 'second'",
                [],
            )
        })
        .await;
    let second = Example::new(&format!("{ORIGIN}/second"), &hub.internal).unwrap();
    second.pair(&second_code).await.unwrap();
    let second_id = hub.app_id("second").await;
    hub.add("later", "http://127.0.0.1:9").await;
    let later_id = hub.app_id("later").await;

    // A new user: access where the default is "everyone".
    let lena = hub
        .env
        .add_user("lena", "lena@example.com", Role::User)
        .await;
    assert_eq!(arrived(example, 3, 1).await, [access(&lena, "user")]);

    // Profile, role and language: `user.updated` with the new values; an
    // unchanged profile gives none.
    let max_session = hub.env.session_for("max").await;
    let patch = json!({ "display_name": "Max Miller" });
    for _ in 0..2 {
        let reply = hub
            .env
            .as_user(&max_session, Method::PATCH, "/auth/me", Some(patch.clone()))
            .await;
        assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    }
    assert_eq!(
        arrived(example, 4, 1).await,
        [updated(&max, "max", "Max Miller", "user", "en")]
    );
    let reply = hub
        .admin(
            Method::PATCH,
            &format!("/users/{max}"),
            Some(json!({ "role": "admin" })),
        )
        .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    let reply = hub
        .env
        .as_user(
            &max_session,
            Method::PUT,
            "/settings",
            Some(json!({ "version": 0, "settings": { "lang": "de" } })),
        )
        .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    assert_eq!(
        arrived(example, 5, 2).await,
        [
            updated(&max, "max", "Max Miller", "admin", "en"),
            updated(&max, "max", "Max Miller", "admin", "de"),
        ]
    );

    // Signing out, locking (with the session), unlocking, removing.
    let eva_session = hub.env.session_for("eva").await;
    let reply = hub
        .env
        .as_user(&eva_session, Method::POST, "/auth/logout", None)
        .await;
    assert_eq!(reply.status, StatusCode::NO_CONTENT, "{}", reply.body);
    let events = arrived(example, 7, 1).await;
    assert!(revoked(&events[0], &eva, "logout"), "{events:?}");

    let reply = hub
        .admin(
            Method::PATCH,
            &format!("/users/{max}"),
            Some(json!({ "role": "user" })),
        )
        .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    let reply = hub
        .admin(Method::POST, &format!("/users/{max}/lock"), None)
        .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    let events = arrived(example, 8, 3).await;
    assert_eq!(events[0], updated(&max, "max", "Max Miller", "user", "de"));
    assert_eq!(events[1], of_user("user.locked", &max));
    assert!(revoked(&events[2], &max, "locked"), "{events:?}");
    let reply = hub
        .admin(Method::POST, &format!("/users/{max}/unlock"), None)
        .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    let reply = hub
        .admin(Method::DELETE, &format!("/users/{eva}"), None)
        .await;
    assert_eq!(reply.status, StatusCode::NO_CONTENT, "{}", reply.body);
    assert_eq!(
        arrived(example, 11, 2).await,
        [
            of_user("user.unlocked", &max),
            of_user("user.deleted", &eva)
        ]
    );

    // A password change ends the other sessions; expired ones end in the
    // cleanup.
    let first = hub.env.session_for("max").await;
    let _second = hub.env.session_for("max").await;
    let reply = hub
        .env
        .as_user(
            &first,
            Method::PUT,
            "/auth/password",
            Some(json!({ "current_password": PASSWORD, "new_password": "Winter2026!" })),
        )
        .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    let events = arrived(example, 13, 1).await;
    assert!(revoked(&events[0], &max, "password_changed"), "{events:?}");
    hub.env
        .sql(|c| {
            c.execute(
                "UPDATE sessions SET expires_at = '2000-01-01T00:00:00.000Z'",
                [],
            )
        })
        .await;
    session_cleanup::run(&hub.env.state).await.unwrap();
    let events = arrived(example, 14, 2).await;
    let expired = |sub: &str| events.iter().filter(|e| revoked(e, sub, "expired")).count();
    assert_eq!((expired(&anna), expired(&max)), (1, 1), "{events:?}");

    // The manifest drops the admin role: it falls back to the default role,
    // only in this app.
    let mut manifest = roedy_hub_example::manifest();
    manifest["roles"][1]["id"] = json!("owner");
    example.send_manifest_with(&manifest).await.unwrap();
    assert_eq!(arrived(example, 16, 1).await, [access(&anna, "user")]);

    // The other paired app got every user event – also about users without
    // access to it – but `access.changed` only for its own access; the
    // pending app nothing.
    let all = hub.stored(&hub.app_id).await;
    let second_events = hub.stored(&second_id).await;
    let user_events = |events: &[(String, Value)]| -> Vec<(String, Value)> {
        events
            .iter()
            .filter(|e| e.0 != "access.changed")
            .cloned()
            .collect()
    };
    assert_eq!(second_events[0], access(&anna, "admin"));
    assert_eq!(
        second_events
            .iter()
            .filter(|e| e.0 == "access.changed")
            .count(),
        1
    );
    // The second app was paired after the example's first three events.
    assert_eq!(user_events(&second_events), user_events(&all));
    assert!(hub.stored(&later_id).await.is_empty());
    // It is down: its first event waits for a retry, the rest behind it.
    let state = hub.state(&second_id).await;
    assert_eq!(state.delivered_seq, 0);
    assert!(state.attempts > 0, "{state:?}");
    assert_eq!(state.last_error.as_deref(), Some("not reachable"));

    // Access per user in the administration (the cleanup above ended
    // Anna's session): a new role, then none – only to this app.
    let admin = hub.env.session_for("anna").await;
    let path = format!("/users/{lena}/apps/example");
    let reply = hub
        .env
        .as_user(
            &admin,
            Method::PUT,
            &path,
            Some(json!({ "app_role": "owner" })),
        )
        .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    assert_eq!(arrived(example, 17, 1).await, [access(&lena, "owner")]);
    let reply = hub.env.as_user(&admin, Method::DELETE, &path, None).await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    assert_eq!(
        arrived(example, 18, 1).await,
        [(
            "access.changed".to_owned(),
            json!({ "sub": lena, "access": false, "app_role": null })
        )]
    );
    assert_eq!(hub.stored(&second_id).await, second_events);
    let _ = second;
}

#[tokio::test]
async fn retries_with_backoff_in_order() {
    let hub = hub().await;
    let example = &hub.example;
    arrived(example, 0, 3).await;
    let eva = hub.user_id("eva").await;

    // 5xx: the same event again after the backoff, nothing overtakes it.
    example.fail_events(Some(500));
    let reply = hub
        .admin(Method::POST, &format!("/users/{eva}/lock"), None)
        .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    let reply = hub
        .admin(Method::POST, &format!("/users/{eva}/unlock"), None)
        .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    until_async("three failures", || async {
        hub.state(&hub.app_id).await.attempts >= 3
    })
    .await;
    let state = hub.state(&hub.app_id).await;
    assert_eq!((state.delivered_seq, state.last_seq), (3, 5));
    assert_eq!(state.last_error.as_deref(), Some("status 500"));
    assert!(state.retry_at.is_some());
    assert_eq!(example.highest_seq(), 3);
    example.fail_events(None);
    assert_eq!(
        arrived(example, 3, 2).await,
        [of_user("user.locked", &eva), of_user("user.unlocked", &eva)]
    );
    // The app answered before the hub recorded it.
    until_async("the acknowledgement", || async {
        hub.state(&hub.app_id).await.delivered_seq == 5
    })
    .await;
    let state = hub.state(&hub.app_id).await;
    assert_eq!((state.attempts, state.last_error), (0, None));

    // No answer within the time limit.
    example.delay_events(Some(ms(800)));
    hub.admin(Method::POST, &format!("/users/{eva}/lock"), None)
        .await;
    until_async("a timeout", || async {
        hub.state(&hub.app_id).await.last_error.as_deref() == Some("timeout")
    })
    .await;
    example.delay_events(None);
    assert_eq!(arrived(example, 5, 1).await, [of_user("user.locked", &eva)]);

    // The app is down: delivered once it is back.
    let (late, addr) = hub.app("late", false).await;
    let late_id = late.connection().unwrap().app_id;
    until_async("failures while down", || async {
        hub.state(&late_id).await.attempts >= 2
    })
    .await;
    assert_eq!(
        hub.state(&late_id).await.last_error.as_deref(),
        Some("not reachable")
    );
    serve(&late, TcpListener::bind(addr).await.unwrap());
    assert_eq!(arrived(&late, 0, 3).await.len(), 3);
}

#[tokio::test]
async fn gives_up_and_resumes_when_the_app_is_back() {
    let hub = hub_with(Schedule {
        give_up: ms(300),
        ..fast()
    })
    .await;
    let example = &hub.example;
    arrived(example, 0, 3).await;
    example.fail_events(Some(503));
    hub.env
        .add_user("lena", "lena@example.com", Role::User)
        .await;
    until_async("giving up", || async {
        let state = hub.state(&hub.app_id).await;
        state.attempts > 0 && state.retry_at.is_none()
    })
    .await;
    example.fail_events(None);
    tokio::time::sleep(ms(300)).await;
    assert_eq!(example.highest_seq(), 3, "no attempt after giving up");
    // Every start of the app sends its manifest.
    example.send_manifest().await.unwrap();
    assert_eq!(arrived(example, 3, 1).await.len(), 1);
}

#[tokio::test]
async fn redelivery_is_acknowledged_without_acting() {
    let hub = hub().await;
    let example = &hub.example;
    let first = arrived(example, 0, 3).await;
    // The app answered before the hub recorded it; a later record would
    // undo the reset below.
    until_async("the acknowledgement", || async {
        hub.state(&hub.app_id).await.delivered_seq == 3
    })
    .await;
    // As if the acknowledgements had been lost.
    hub.env
        .sql(|c| c.execute("UPDATE event_delivery SET delivered_seq = 0", []))
        .await;
    hub.env.state.events.wake();
    until("redelivery", || example.repeated_events() == 3).await;
    assert_eq!(arrived(example, 0, 3).await, first);
    until_async("acknowledged again", || async {
        hub.state(&hub.app_id).await.delivered_seq == 3
    })
    .await;
}

#[tokio::test]
async fn catching_up_pages_and_answers_410_after_the_retention() {
    let hub = hub().await;
    arrived(&hub.example, 0, 3).await;
    for name in ["lena", "otto"] {
        hub.env
            .add_user(name, &format!("{name}@example.com"), Role::User)
            .await;
    }
    arrived(&hub.example, 3, 2).await;

    // Paging with freshly signed events.
    let reply = hub.catch_up("?after=1&limit=2").await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    assert_eq!(reply.header("cache-control"), Some("no-store"));
    let page: EventPage = serde_json::from_str(&reply.body).unwrap();
    assert!(page.more);
    let key = hub.env.state.keys.signer(jiff::Timestamp::now()).unwrap();
    let jwks = roedy_hub_contract::Jwks {
        keys: vec![key.jwk()],
    };
    let expected = roedy_hub_contract::Expected {
        issuer: ORIGIN,
        audience: "example",
        now: jiff::Timestamp::now().as_second(),
    };
    let seqs: Vec<u64> = page
        .events
        .iter()
        .map(|token| {
            let event: Event = roedy_hub_contract::verify(token, &jwks, &expected).unwrap();
            assert!(event.exp - event.iat <= 300);
            event.seq
        })
        .collect();
    assert_eq!(seqs, [2, 3]);
    let reply = hub.catch_up("?after=3").await;
    let page: EventPage = serde_json::from_str(&reply.body).unwrap();
    assert_eq!((page.events.len(), page.more), (2, false));
    let reply = hub.catch_up("?after=5").await;
    assert_eq!(reply.json(), json!({ "events": [], "more": false }));

    // Bad requests and the secret.
    for query in ["?limit=0", "?limit=101", "?after=-1", "?after=x"] {
        let reply = hub.catch_up(query).await;
        assert_eq!(reply.status, StatusCode::BAD_REQUEST, "{query}");
        assert_eq!(reply.error(), "bad_request");
    }
    let reply = internal(&hub.env, "/events", None).await;
    assert_eq!(reply.status, StatusCode::UNAUTHORIZED);
    assert_eq!(reply.error(), "invalid_app_credentials");

    // An app that lost its data reads everything.
    let fresh = Example::new(&format!("{ORIGIN}/example"), &hub.internal).unwrap();
    fresh.connect(hub.example.connection().unwrap());
    let done = fresh.catch_up().await.unwrap();
    assert_eq!((done.processed, done.resumed_after), (5, None));
    let content = |example: &Example| -> Vec<(u64, String, Value)> {
        example
            .events()
            .into_iter()
            .map(|e| (e.seq, e.kind, e.data))
            .collect()
    };
    assert_eq!(content(&fresh), content(&hub.example));

    // The retention deleted the first two.
    hub.env
        .sql(|c| {
            c.execute(
                "UPDATE events SET created_at = '2000-01-01T00:00:00.000Z' WHERE seq <= 2",
                [],
            )
        })
        .await;
    assert_eq!(event_retention::run(&hub.env.state).await.unwrap(), 2);
    let reply = hub.catch_up("?after=0").await;
    assert_eq!(reply.status, StatusCode::GONE, "{}", reply.body);
    assert_eq!(
        reply.json(),
        json!({ "error": "events_expired", "resume_after": 2 })
    );
    assert_eq!(hub.catch_up("?after=1").await.status, StatusCode::GONE);
    assert_eq!(hub.catch_up("?after=2").await.status, StatusCode::OK);
    let reset = Example::new(&format!("{ORIGIN}/example"), &hub.internal).unwrap();
    reset.connect(hub.example.connection().unwrap());
    let done = reset.catch_up().await.unwrap();
    assert_eq!((done.processed, done.resumed_after), (3, Some(2)));
    assert_eq!(reset.resumed_after(), Some(2));
    assert_eq!(
        reset.events().iter().map(|e| e.seq).collect::<Vec<_>>(),
        [3, 4, 5]
    );
}

#[tokio::test]
async fn a_gap_in_the_delivery_makes_the_app_catch_up() {
    let hub = hub().await;
    let example = &hub.example;
    arrived(example, 0, 3).await;
    let eva = hub.user_id("eva").await;
    // The hub skips event 4 (as after a lost acknowledgement of a catch-up):
    // event 5 shows the app the gap.
    example.fail_events(Some(503));
    let reply = hub
        .admin(Method::POST, &format!("/users/{eva}/lock"), None)
        .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    let reply = hub
        .admin(Method::POST, &format!("/users/{eva}/unlock"), None)
        .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    hub.env
        .sql(|c| {
            c.execute(
                "UPDATE event_delivery SET delivered_seq = 4, attempts = 0, retry_at = NULL, \
                 failing_since = NULL",
                [],
            )
        })
        .await;
    example.fail_events(None);
    hub.env.state.events.wake();
    assert_eq!(
        arrived(example, 3, 2).await,
        [of_user("user.locked", &eva), of_user("user.unlocked", &eva)]
    );
}

#[tokio::test]
async fn the_example_rejects_forged_events() {
    let hub = hub().await;
    arrived(&hub.example, 0, 3).await;
    // The example's own address, as the hub reaches it.
    let url: String = hub
        .env
        .sql(|c| {
            c.query_row(
                "SELECT internal_url FROM apps WHERE slug = 'example'",
                [],
                |r| r.get(0),
            )
        })
        .await;
    let url = format!("{url}/example/_roedy/v1/events");
    let client = reqwest::Client::new();
    let post = |token: String| {
        let request = client
            .post(&url)
            .header(header::CONTENT_TYPE.as_str(), "application/jwt")
            .body(token);
        async move {
            let response = request.send().await.unwrap();
            let status = response.status().as_u16();
            let bytes = response.bytes().await.unwrap();
            let body: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
            (
                status,
                body["error"].as_str().unwrap_or_default().to_owned(),
            )
        }
    };
    let now = jiff::Timestamp::now().as_second();
    let event = |aud: &str| Event {
        iss: ORIGIN.into(),
        aud: aud.into(),
        jti: "01JDX5R2T4V6X8Z0B2D4F6H8J0".into(),
        seq: 4,
        kind: "user.deleted".into(),
        data: json!({ "sub": "01JDX2Q7B5W3Y1Z9X7V5T3R1P0" }),
        iat: now,
        exp: now + 300,
    };
    let hub_key = hub.env.state.keys.signer(jiff::Timestamp::now()).unwrap();
    // The same `kid` with another key.
    let forger = SigningKey::from_seed(hub_key.kid(), &[9; 32]).unwrap();
    assert_eq!(
        post(forger.sign(&event("example")).unwrap()).await,
        (401, "signature".into())
    );
    // The hub's key, but an identity: wrong `typ`.
    let identity = Identity {
        iss: ORIGIN.into(),
        aud: "example".into(),
        sub: "01JDX2Q7B5W3Y1Z9X7V5T3R1P0".into(),
        username: "alex".into(),
        email: "alex@example.com".into(),
        name: "Alex Example".into(),
        role: "user".into(),
        app_role: "user".into(),
        locale: "en".into(),
        sid: "01JDX3A1C3E5G7J9K1M3N5P7Q9".into(),
        auth_time: now,
        iat: now,
        exp: now + 60,
    };
    assert_eq!(
        post(hub_key.sign(&identity).unwrap()).await,
        (401, "typ".into())
    );
    // For another app, and too long-lived.
    assert_eq!(
        post(hub_key.sign(&event("mail")).unwrap()).await,
        (401, "aud".into())
    );
    let long = Event {
        exp: now + 301,
        ..event("example")
    };
    assert_eq!(
        post(hub_key.sign(&long).unwrap()).await,
        (401, "lifetime".into())
    );
    assert_eq!(hub.example.highest_seq(), 3, "nothing processed");
    // The real one.
    assert_eq!(
        post(hub_key.sign(&event("example")).unwrap()).await,
        (204, String::new())
    );
    assert_eq!(hub.example.highest_seq(), 4);
}

#[tokio::test]
async fn removing_an_app_stops_its_delivery_and_deletes_its_events() {
    let hub = hub().await;
    arrived(&hub.example, 0, 3).await;
    let reply = hub.admin(Method::DELETE, "/admin/apps/example", None).await;
    assert_eq!(reply.status, StatusCode::NO_CONTENT, "{}", reply.body);
    assert!(hub.stored(&hub.app_id).await.is_empty());
    let app_id = hub.app_id.clone();
    let state = hub.env.sql(move |c| repo::state(c, &app_id)).await;
    assert_eq!(state, None);
    hub.env
        .add_user("lena", "lena@example.com", Role::User)
        .await;
    let eva = hub.user_id("eva").await;
    hub.admin(Method::POST, &format!("/users/{eva}/lock"), None)
        .await;
    tokio::time::sleep(ms(300)).await;
    assert_eq!(hub.example.highest_seq(), 3);
    assert_eq!(hub.catch_up("").await.error(), "invalid_app_credentials");
}
