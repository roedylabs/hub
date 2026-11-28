//! The gateway against the example app over real connections: path and
//! query, header rules both ways, the identity at the app (including key
//! rotation), SSE, WebSocket, large bodies, timeouts, the permission cases,
//! the end of a session's streams and switching the app off and on.

mod common;

use std::{
    convert::Infallible,
    net::SocketAddr,
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use axum::http::{HeaderMap, Method, Request, Response, StatusCode, header};
use base64::Engine as _;
use common::{Env, env};
use futures_util::{SinkExt as _, StreamExt as _, stream};
use http_body_util::{BodyExt as _, Empty, Full, StreamBody, combinators::BoxBody};
use hyper::body::{Bytes, Frame, Incoming};
use hyper_util::{
    client::legacy::{Client, connect::HttpConnector},
    rt::TokioExecutor,
};
use roedy_hub_contract::{IDENTITY_HEADER, Identity};
use roedy_hub_example::{Example, download_byte};
use roedy_hub_server::{
    app::{internal_router, router},
    auth::session::COOKIE,
    crypto::SecretKey,
    db,
    db::repo::users::Role,
    gateway::{Gateway, Limits},
    jobs::gateway_streams,
    signing,
    web::static_files::WebAssets,
};
use serde_json::{Value, json};
use tokio::{
    io::{AsyncReadExt as _, AsyncWriteExt as _},
    net::{TcpListener, TcpStream},
};
use tokio_tungstenite::tungstenite::{self, Message, client::IntoClientRequest as _};

const ORIGIN: &str = "https://app.example.com";

type TestBody = BoxBody<Bytes, Infallible>;

/// A hub with the example app paired at `/example`, both on real ports.
/// `anna` is an administrator, `max` a user with the default role, `eva` a
/// user without access.
struct Hub {
    env: Env,
    public: SocketAddr,
    app: SocketAddr,
    example: Example,
    anna: String,
    max: String,
    eva: String,
    client: Client<HttpConnector, TestBody>,
}

async fn hub() -> Hub {
    hub_with(Limits::default(), None).await
}

async fn hub_with(limits: Limits, web: Option<&Path>) -> Hub {
    let mut env = env().await;
    env.state.gateway = Arc::new(Gateway::with_limits(&env.state.public_url, limits).unwrap());
    if let Some(web) = web {
        env.state.web = Arc::new(WebAssets::new(Some(web)).unwrap());
    }
    signing::rotate_now(&env.state.db, env.state.secrets.clone(), &env.state.keys)
        .await
        .unwrap();
    env.add_user("anna", "anna@example.com", Role::Admin).await;
    env.add_user("max", "max@example.com", Role::User).await;
    let eva_id = env.add_user("eva", "eva@example.com", Role::User).await;

    let internal = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let internal_addr = internal.local_addr().unwrap();
    let internal_app = internal_router(env.state.clone());
    tokio::spawn(async move {
        axum::serve(
            internal,
            internal_app.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
        .unwrap();
    });
    let example = Example::new(
        &format!("{ORIGIN}/example"),
        &format!("http://{internal_addr}"),
    )
    .unwrap();
    let app_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let app = app_listener.local_addr().unwrap();
    let served = example.clone();
    tokio::spawn(async move {
        roedy_hub_example::serve(app_listener, &served, std::future::pending())
            .await
            .unwrap();
    });

    let anna = env.session_for("anna").await;
    let reply = env
        .as_user(
            &anna,
            Method::POST,
            "/admin/apps",
            Some(json!({ "slug": "example", "internal_url": format!("http://{app}") })),
        )
        .await;
    assert_eq!(reply.status, StatusCode::CREATED, "{}", reply.body);
    let code = reply.json()["pairing"]["code"].as_str().unwrap().to_owned();
    example.pair(&code).await.unwrap();
    // Eva has no access.
    let reply = env
        .as_user(
            &anna,
            Method::DELETE,
            &format!("/users/{eva_id}/apps/example"),
            None,
        )
        .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    let max = env.session_for("max").await;
    let eva = env.session_for("eva").await;

    let public = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let public_addr = public.local_addr().unwrap();
    let public_app = router(env.state.clone());
    tokio::spawn(async move {
        axum::serve(
            public,
            public_app.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
        .unwrap();
    });
    Hub {
        env,
        public: public_addr,
        app,
        example,
        anna,
        max,
        eva,
        client: Client::builder(TokioExecutor::new()).build_http(),
    }
}

fn empty() -> TestBody {
    Empty::new().boxed()
}

fn cookie(session: &str) -> String {
    format!("{COOKIE}={session}")
}

impl Hub {
    /// A request to the public listener with `session` and `headers`.
    async fn send(
        &self,
        method: Method,
        path: &str,
        session: Option<&str>,
        headers: &[(&str, &str)],
        body: TestBody,
    ) -> Response<Incoming> {
        let mut request = Request::builder()
            .method(method)
            .uri(format!("http://{}{path}", self.public));
        if let Some(session) = session {
            request = request.header(header::COOKIE, cookie(session));
        }
        for (name, value) in headers {
            request = request.header(*name, *value);
        }
        self.client
            .request(request.body(body).unwrap())
            .await
            .unwrap()
    }

    async fn get(&self, path: &str, session: Option<&str>) -> Response<Incoming> {
        self.send(Method::GET, path, session, &[], empty()).await
    }

    /// `GET` and the whole body as JSON.
    async fn json(&self, path: &str, session: Option<&str>) -> (StatusCode, HeaderMap, Value) {
        json_of(self.get(path, session).await).await
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

    /// A WebSocket to `/example/ws` with `session` and `origin`.
    async fn websocket(
        &self,
        session: &str,
        origin: &str,
    ) -> Result<tokio_tungstenite::WebSocketStream<TcpStream>, tungstenite::Error> {
        let mut request = format!("ws://{}/example/ws", self.public)
            .into_client_request()
            .unwrap();
        request
            .headers_mut()
            .insert(header::COOKIE, cookie(session).parse().unwrap());
        request
            .headers_mut()
            .insert(header::ORIGIN, origin.parse().unwrap());
        let stream = TcpStream::connect(self.public).await.unwrap();
        tokio_tungstenite::client_async(request, stream)
            .await
            .map(|(socket, _)| socket)
    }
}

async fn json_of(response: Response<Incoming>) -> (StatusCode, HeaderMap, Value) {
    let (status, headers) = (response.status(), response.headers().clone());
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let json = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes)
            .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into_owned()))
    };
    (status, headers, json)
}

fn values(headers: &Value, name: &str) -> Vec<String> {
    headers
        .as_array()
        .unwrap()
        .iter()
        .filter(|pair| pair[0] == name)
        .map(|pair| pair[1].as_str().unwrap().to_owned())
        .collect()
}

fn all(headers: &HeaderMap, name: &str) -> Vec<String> {
    headers
        .get_all(name)
        .iter()
        .map(|v| v.to_str().unwrap().to_owned())
        .collect()
}

#[tokio::test]
async fn forwards_path_query_body_and_clean_headers() {
    let hub = hub().await;
    let max_id = hub.user_id("max").await;
    let forged = "eyJhbGciOiJub25lIn0.eyJzdWIiOiJ4In0.";
    let cookies = format!(
        "{}; __Host-roedy-example_pref=1; __Host-roedy-mail_token=2; theme=dark",
        cookie(&hub.max)
    );
    let response = hub
        .send(
            Method::POST,
            "/example/echo/a%20b/%2e%2e/c?x=1&y=%2F&z",
            None,
            &[
                (header::COOKIE.as_str(), &cookies),
                (IDENTITY_HEADER, forged),
                ("x-roedy-request-id", "forged"),
                ("x-roedy-other", "1"),
                ("x-forwarded-for", "198.51.100.1"),
                ("x-forwarded-host", "evil.example"),
                ("x-forwarded-proto", "http"),
                ("forwarded", "for=198.51.100.1"),
                ("connection", "x-hop"),
                ("x-hop", "1"),
                ("origin", ORIGIN),
                ("x-requested-with", "fetch"),
                ("authorization", "Bearer app-token"),
                ("content-type", "text/plain"),
            ],
            Full::new(Bytes::from_static(b"hello, app!!")).boxed(),
        )
        .await;
    let (status, headers, echo) = json_of(response).await;
    assert_eq!(status, StatusCode::OK, "{echo}");
    assert_eq!(echo["method"], "POST");
    assert_eq!(echo["path"], "/example/echo/a%20b/%2e%2e/c");
    assert_eq!(echo["query"], "x=1&y=%2F&z");
    assert_eq!(echo["body_bytes"], 12);
    // The hub's identity, not the forged one.
    assert_eq!(echo["identity"]["sub"], max_id.as_str(), "{echo}");
    let sent = &echo["headers"];
    assert_eq!(values(sent, "x-roedy-identity").len(), 1);
    assert_ne!(values(sent, "x-roedy-identity"), [forged]);
    let request_id = values(sent, "x-roedy-request-id");
    assert_eq!(request_id.len(), 1);
    assert_eq!(request_id[0].len(), 26, "a ULID");
    assert!(values(sent, "x-roedy-other").is_empty());
    assert_eq!(values(sent, "cookie"), ["__Host-roedy-example_pref=1"]);
    assert_eq!(values(sent, "host"), ["app.example.com"]);
    assert_eq!(values(sent, "x-forwarded-host"), ["app.example.com"]);
    assert_eq!(values(sent, "x-forwarded-proto"), ["https"]);
    // Without a trusted proxy only the peer.
    assert_eq!(values(sent, "x-forwarded-for"), ["127.0.0.1"]);
    assert!(values(sent, "forwarded").is_empty());
    assert!(values(sent, "x-hop").is_empty());
    assert_eq!(values(sent, "origin"), [ORIGIN]);
    assert_eq!(values(sent, "x-requested-with"), ["fetch"]);
    assert_eq!(values(sent, "authorization"), ["Bearer app-token"]);
    assert_eq!(values(sent, "content-type"), ["text/plain"]);
    // The app's response without the hub's own policies.
    assert_eq!(all(&headers, "x-content-type-options"), ["nosniff"]);
    for absent in [
        "content-security-policy",
        "referrer-policy",
        "permissions-policy",
        "cross-origin-opener-policy",
        "strict-transport-security",
    ] {
        assert!(headers.get(absent).is_none(), "{absent}");
    }

    // Signed out, a forged identity does not reach the app either.
    let response = hub
        .send(
            Method::GET,
            "/example/echo",
            None,
            &[(IDENTITY_HEADER, forged)],
            empty(),
        )
        .await;
    let (_, _, echo) = json_of(response).await;
    assert_eq!(echo["identity_error"], "missing");
}

#[tokio::test]
async fn response_headers_are_cleaned() {
    let hub = hub().await;
    let query = [
        ("status", "201"),
        (
            "header",
            "Set-Cookie: __Host-roedy-example_token=a; Path=/; Secure; HttpOnly",
        ),
        (
            "header",
            "Set-Cookie: __Host-roedy_hub_session=forged; Path=/; Secure",
        ),
        (
            "header",
            "Set-Cookie: __Host-roedy-mail_token=b; Path=/; Secure",
        ),
        ("header", "Set-Cookie: plain=c"),
        ("header", "Clear-Site-Data: \"*\""),
        ("header", "Service-Worker-Allowed: /"),
        ("header", "Strict-Transport-Security: max-age=0"),
        ("header", "Content-Security-Policy: default-src 'self'"),
        ("header", "Cache-Control: private, max-age=60"),
        ("body", "made"),
    ];
    let query: String = url::form_urlencoded::Serializer::new(String::new())
        .extend_pairs(query)
        .finish();
    let response = hub.get(&format!("/example/respond?{query}"), None).await;
    let (status, headers) = (response.status(), response.headers().clone());
    let body = response.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(body.as_ref(), b"made");
    assert_eq!(
        all(&headers, "set-cookie"),
        ["__Host-roedy-example_token=a; Path=/; Secure; HttpOnly"]
    );
    assert!(headers.get("clear-site-data").is_none());
    assert!(headers.get("service-worker-allowed").is_none());
    // Without a TLS proxy (`ROEDY_TRUST_PROXY`) no HSTS at all.
    assert!(headers.get("strict-transport-security").is_none());
    assert_eq!(
        all(&headers, "content-security-policy"),
        ["default-src 'self'"]
    );
    assert_eq!(all(&headers, "cache-control"), ["private, max-age=60"]);
    assert_eq!(all(&headers, "x-content-type-options"), ["nosniff"]);
}

/// A token signed with the hub's current key.
fn signed(hub: &Hub, claims: &Identity) -> String {
    hub.env
        .state
        .keys
        .signer(jiff::Timestamp::now())
        .unwrap()
        .sign(claims)
        .unwrap()
}

fn now() -> i64 {
    jiff::Timestamp::now().as_second()
}

fn claims(sub: &str) -> Identity {
    let iat = now();
    Identity {
        iss: ORIGIN.into(),
        aud: "example".into(),
        sub: sub.into(),
        username: "max".into(),
        email: "max@example.com".into(),
        name: "Max Example".into(),
        role: "user".into(),
        app_role: "user".into(),
        locale: "en".into(),
        sid: "01JDX3A1C3E5G7J9K1M3N5P7Q9".into(),
        auth_time: iat,
        iat,
        exp: iat + 60,
    }
}

#[tokio::test]
async fn the_app_verifies_the_identity() {
    let hub = hub().await;
    let (max_id, anna_id) = (hub.user_id("max").await, hub.user_id("anna").await);

    // Through the hub: the identity of the session, with the app role.
    let (status, _, me) = hub.json("/example/whoami", Some(&hub.max)).await;
    assert_eq!(status, StatusCode::OK, "{me}");
    assert_eq!(me["sub"], max_id.as_str());
    assert_eq!(me["aud"], "example");
    assert_eq!(me["iss"], ORIGIN);
    assert_eq!(me["role"], "user");
    assert_eq!(me["app_role"], "user");
    assert_eq!(me["locale"], "en");
    let (status, _, me) = hub.json("/example/whoami", Some(&hub.anna)).await;
    assert_eq!((status, &me["app_role"]), (StatusCode::OK, &json!("admin")));
    assert_eq!(me["sub"], anna_id.as_str());
    let (status, _, me) = hub.json("/example/whoami", None).await;
    assert_eq!(
        (status, &me["reason"]),
        (StatusCode::UNAUTHORIZED, &json!("missing"))
    );

    // Directly at the app (as if the private network were open): it
    // rejects every flawed token, signed with the hub's real key or not.
    let valid = signed(&hub, &claims(&anna_id));
    let parts: Vec<&str> = valid.split('.').collect();
    let mut signature = parts[2].as_bytes().to_vec();
    signature[0] = if signature[0] == b'A' { b'B' } else { b'A' };
    let forged_signature = format!(
        "{}.{}.{}",
        parts[0],
        parts[1],
        String::from_utf8(signature).unwrap()
    );
    let b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD;
    let kid = hub
        .env
        .state
        .keys
        .signer(jiff::Timestamp::now())
        .unwrap()
        .kid()
        .to_owned();
    let alg_none = format!(
        "{}.{}.",
        b64.encode(json!({ "alg": "none", "typ": "roedy-id+jwt", "kid": kid }).to_string()),
        parts[1]
    );
    let wrong_aud = signed(
        &hub,
        &Identity {
            aud: "mail".into(),
            ..claims(&anna_id)
        },
    );
    let expired = signed(
        &hub,
        &Identity {
            iat: now() - 120,
            exp: now() - 60,
            ..claims(&anna_id)
        },
    );
    let future = signed(
        &hub,
        &Identity {
            iat: now() + 60,
            exp: now() + 120,
            ..claims(&anna_id)
        },
    );
    let direct = reqwest::Client::new();
    let table = [
        (valid.as_str(), None),
        (forged_signature.as_str(), Some("signature")),
        (alg_none.as_str(), Some("alg")),
        (wrong_aud.as_str(), Some("aud")),
        (expired.as_str(), Some("expired")),
        (future.as_str(), Some("iat")),
    ];
    for (token, reason) in table {
        let response = direct
            .get(format!("http://{}/example/whoami", hub.app))
            .header(IDENTITY_HEADER, token)
            .send()
            .await
            .unwrap();
        let status = response.status().as_u16();
        let body: Value = serde_json::from_slice(&response.bytes().await.unwrap()).unwrap();
        match reason {
            None => assert_eq!((status, &body["sub"]), (200, &json!(anna_id))),
            Some(reason) => assert_eq!((status, &body["reason"]), (401, &json!(reason)), "{token}"),
        }
        // Through the hub, none of them arrives: signed out the app sees
        // none, signed in the session's own.
        let response = hub
            .send(
                Method::GET,
                "/example/whoami",
                None,
                &[(IDENTITY_HEADER, token)],
                empty(),
            )
            .await;
        let (status, _, me) = json_of(response).await;
        assert_eq!(
            (status, &me["reason"]),
            (StatusCode::UNAUTHORIZED, &json!("missing"))
        );
        let response = hub
            .send(
                Method::GET,
                "/example/whoami",
                Some(&hub.max),
                &[(IDENTITY_HEADER, token)],
                empty(),
            )
            .await;
        let (status, _, me) = json_of(response).await;
        assert_eq!((status, &me["sub"]), (StatusCode::OK, &json!(max_id)));
    }

    // Key rotation: the hub can no longer decrypt its key (a new instance
    // key) and signs with a new one at once. The app meets an unknown
    // `kid`, refreshes the JWKS and accepts it.
    let before: Vec<String> = hub
        .example
        .connection()
        .unwrap()
        .jwks
        .keys
        .iter()
        .filter_map(|k| k.kid.clone())
        .collect();
    signing::rotate_now(
        &hub.env.state.db,
        Arc::new(SecretKey::random().unwrap()),
        &hub.env.state.keys,
    )
    .await
    .unwrap();
    let new_kid = hub
        .env
        .state
        .keys
        .signer(jiff::Timestamp::now())
        .unwrap()
        .kid()
        .to_owned();
    assert!(!before.contains(&new_kid));
    let (status, _, me) = hub.json("/example/whoami", Some(&hub.max)).await;
    assert_eq!(status, StatusCode::OK, "{me}");
    assert_eq!(me["sub"], max_id.as_str());
    let after: Vec<String> = hub
        .example
        .connection()
        .unwrap()
        .jwks
        .keys
        .iter()
        .filter_map(|k| k.kid.clone())
        .collect();
    assert!(after.contains(&new_kid), "JWKS refreshed");
}

#[tokio::test]
async fn permissions() {
    let hub = hub().await;
    // No app with this slug: 404 from the hub, JSON for API calls.
    let (status, headers, body) = hub.json("/nothing/here", Some(&hub.max)).await;
    assert_eq!(
        (status, &body),
        (StatusCode::NOT_FOUND, &json!({ "error": "not_found" }))
    );
    assert_eq!(all(&headers, "cache-control"), ["no-store"]);
    // The app's reserved path never reaches it, in no spelling.
    for path in [
        "/example/_roedy/v1/events",
        "/example/%5Froedy/v1/events",
        "/example//_roedy/v1/events",
        "/example/x/../_roedy/v1/events",
    ] {
        let response = hub
            .send(
                Method::POST,
                path,
                None,
                &[],
                Full::new(Bytes::from_static(b"x")).boxed(),
            )
            .await;
        let (status, _, body) = json_of(response).await;
        assert_eq!(
            (status, &body["error"]),
            (StatusCode::NOT_FOUND, &json!("not_found")),
            "{path}"
        );
    }
    assert!(hub.example.events().is_empty());
    // Signed out: forwarded without identity; the app decides.
    let (status, _, body) = hub.json("/example/healthz", None).await;
    assert_eq!((status, &body["status"]), (StatusCode::OK, &json!("ok")));
    // Signed in without access: 403 for every path, even public ones.
    for path in ["/example/healthz", "/example/", "/example"] {
        let (status, headers, body) = hub.json(path, Some(&hub.eva)).await;
        assert_eq!(
            (status, &body),
            (StatusCode::FORBIDDEN, &json!({ "error": "no_app_access" })),
            "{path}"
        );
        assert_eq!(all(&headers, "cache-control"), ["no-store"]);
    }
    // A session cookie that is no longer valid is cleared, the request
    // goes on signed out.
    let response = hub.get("/example/whoami", Some(&"0".repeat(64))).await;
    let set_cookies = all(response.headers(), "set-cookie");
    assert!(
        set_cookies
            .iter()
            .any(|c| c.starts_with(&format!("{COOKIE}=;"))),
        "{set_cookies:?}"
    );
    let (status, _, body) = json_of(response).await;
    assert_eq!(
        (status, &body["reason"]),
        (StatusCode::UNAUTHORIZED, &json!("missing"))
    );

    // A pending app (pointing at the example's port): only hub
    // administrators get through, without identity.
    let reply = hub
        .env
        .as_user(
            &hub.anna,
            Method::POST,
            "/admin/apps",
            Some(json!({ "slug": "pending", "internal_url": format!("http://{}", hub.app) })),
        )
        .await;
    assert_eq!(reply.status, StatusCode::CREATED, "{}", reply.body);
    for session in [None, Some(hub.max.as_str()), Some(hub.eva.as_str())] {
        let (status, _, body) = hub.json("/pending/", session).await;
        assert_eq!(
            (status, &body),
            (
                StatusCode::SERVICE_UNAVAILABLE,
                &json!({ "error": "app_not_ready" })
            )
        );
    }
    // The example serves nothing under `/pending`: its own empty 404 shows
    // that the request reached it.
    let response = hub.get("/pending/", Some(&hub.anna)).await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    assert!(response.headers().get("cache-control").is_none());
    let body = response.into_body().collect().await.unwrap().to_bytes();
    assert!(body.is_empty());

    // A WebSocket from another origin: 403 from the hub.
    let refused = hub
        .websocket(&hub.max, "https://evil.example")
        .await
        .unwrap_err();
    let tungstenite::Error::Http(response) = refused else {
        panic!("{refused:?}");
    };
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let body: Value = serde_json::from_slice(response.body().as_deref().unwrap()).unwrap();
    assert_eq!(body, json!({ "error": "origin_mismatch" }));
}

#[tokio::test]
async fn error_pages_for_page_loads() {
    let web = tempfile::tempdir().unwrap();
    std::fs::write(
        web.path().join("index.html"),
        "<!doctype html><html><head><title>Roedy Hub</title>\
         <script>start();</script></HEAD><body></body></html>",
    )
    .unwrap();
    let hub = hub_with(Limits::default(), Some(web.path())).await;
    let page_load = [("sec-fetch-mode", "navigate"), ("accept", "text/html")];
    let response = hub
        .send(
            Method::GET,
            "/nothing/here",
            Some(&hub.eva),
            &page_load,
            empty(),
        )
        .await;
    let (status, headers) = (response.status(), response.headers().clone());
    let body = String::from_utf8(
        response
            .into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes()
            .to_vec(),
    )
    .unwrap();
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(
        body.contains("<meta name=\"roedy-error\" content=\"not_found\"></HEAD>"),
        "{body}"
    );
    assert_eq!(all(&headers, "content-type"), ["text/html; charset=utf-8"]);
    assert_eq!(all(&headers, "cache-control"), ["no-store"]);
    // The hub's own headers, with the script's hash.
    let csp = all(&headers, "content-security-policy").join("");
    assert!(csp.contains("'sha256-"), "{csp}");
    assert!(headers.get("cross-origin-opener-policy").is_some());

    let response = hub
        .send(
            Method::GET,
            "/example/",
            Some(&hub.eva),
            &page_load,
            empty(),
        )
        .await;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    assert!(String::from_utf8_lossy(&body).contains("content=\"no_app_access\""));
    // `Accept: text/html` alone counts only without `Sec-Fetch-Mode`.
    let response = hub
        .send(
            Method::GET,
            "/example/",
            Some(&hub.eva),
            &[("sec-fetch-mode", "cors"), ("accept", "text/html")],
            empty(),
        )
        .await;
    let (status, _, body) = json_of(response).await;
    assert_eq!(
        (status, &body["error"]),
        (StatusCode::FORBIDDEN, &json!("no_app_access"))
    );
    // The hub's own pages stay the hub's.
    let response = hub
        .send(Method::GET, "/login", None, &page_load, empty())
        .await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    assert!(!String::from_utf8_lossy(&body).contains("roedy-error"));
    // The app's page keeps its own policy.
    let response = hub
        .send(
            Method::GET,
            "/example/",
            Some(&hub.max),
            &page_load,
            empty(),
        )
        .await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        all(response.headers(), "content-security-policy"),
        [roedy_hub_example::CSP]
    );
}

#[tokio::test]
async fn timeouts_and_unreachable_apps() {
    let limits = Limits {
        response: Duration::from_millis(500),
        ..Limits::default()
    };
    let hub = hub_with(limits, None).await;
    // No response headers in time: 504.
    let started = Instant::now();
    let (status, _, body) = hub.json("/example/slow?ms=3000", Some(&hub.max)).await;
    assert_eq!(
        (status, &body),
        (
            StatusCode::GATEWAY_TIMEOUT,
            &json!({ "error": "app_timeout" })
        )
    );
    assert!(started.elapsed() < Duration::from_millis(2500));
    // A slow body after timely headers: no total time limit.
    let (status, _, body) = hub
        .json("/example/slow?ms=0&body_ms=1000", Some(&hub.max))
        .await;
    assert_eq!((status, &body["body_ms"]), (StatusCode::OK, &json!(1000)));
    // A slow upload: the time until the response headers counts from its
    // end, so 1.5 s of upload with a limit of 0.5 s is fine.
    let chunks = stream::unfold(0u32, |n| async move {
        if n == 6 {
            return None;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
        let data = Bytes::from(vec![1u8; 1024]);
        Some((Ok::<_, Infallible>(Frame::data(data)), n + 1))
    });
    let response = hub
        .send(
            Method::POST,
            "/example/upload",
            Some(&hub.max),
            &[],
            http_body_util::BodyExt::boxed(StreamBody::new(chunks)),
        )
        .await;
    let (status, _, body) = json_of(response).await;
    assert_eq!(
        (status, &body["bytes"]),
        (StatusCode::OK, &json!(6 * 1024)),
        "{body}"
    );
    // Request headers beyond 64 KiB.
    let big = "a".repeat(70 * 1024);
    let response = hub
        .send(
            Method::GET,
            "/example/echo",
            None,
            &[("x-big", &big)],
            empty(),
        )
        .await;
    let (status, _, body) = json_of(response).await;
    assert_eq!(
        (status, &body["error"]),
        (
            StatusCode::REQUEST_HEADER_FIELDS_TOO_LARGE,
            &json!("headers_too_large")
        )
    );

    // Nothing listens at the internal address any more: 502.
    let closed = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let closed_addr = closed.local_addr().unwrap();
    drop(closed);
    let table = hub.env.state.apps.clone();
    db::call(&hub.env.state.db, move |conn| {
        conn.execute(
            "UPDATE apps SET internal_url = ?1 WHERE slug = 'example'",
            [format!("http://{closed_addr}")],
        )?;
        table.reload(conn)
    })
    .await
    .unwrap();
    let (status, _, body) = hub.json("/example/echo", Some(&hub.max)).await;
    assert_eq!(
        (status, &body),
        (
            StatusCode::BAD_GATEWAY,
            &json!({ "error": "app_unreachable" })
        )
    );
}

/// The next frame of a body within `within`: `Some(Ok)` with data, `None`
/// at its end, `Some(Err)` when it broke off.
async fn next_frame(body: &mut Incoming, within: Duration) -> Option<Result<Bytes, String>> {
    let frame = tokio::time::timeout(within, body.frame())
        .await
        .expect("no frame in time")?;
    Some(
        frame
            .map(|frame| frame.into_data().unwrap_or_default())
            .map_err(|error| error.to_string()),
    )
}

#[tokio::test]
async fn server_sent_events_are_not_buffered() {
    let hub = hub().await;
    let started = Instant::now();
    // Three events 1.5 s apart: the first arrives long before the end.
    let response = hub
        .get("/example/sse?count=3&interval_ms=1500", Some(&hub.max))
        .await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        all(response.headers(), "content-type"),
        ["text/event-stream"]
    );
    let mut body = response.into_body();
    let first = next_frame(&mut body, Duration::from_secs(2))
        .await
        .unwrap()
        .unwrap();
    assert!(String::from_utf8_lossy(&first).contains("tick 0"));
    assert!(
        started.elapsed() < Duration::from_millis(1500),
        "{:?}",
        started.elapsed()
    );
    let mut text = String::from_utf8_lossy(&first).into_owned();
    while let Some(frame) = next_frame(&mut body, Duration::from_secs(5)).await {
        text.push_str(&String::from_utf8_lossy(&frame.unwrap()));
    }
    assert!(text.contains("tick 2"), "{text}");
}

#[tokio::test]
async fn websocket_echo_and_close() {
    let hub = hub().await;
    let mut socket = hub.websocket(&hub.max, ORIGIN).await.unwrap();
    socket.send(Message::text("hello")).await.unwrap();
    assert_eq!(
        socket.next().await.unwrap().unwrap(),
        Message::text("hello")
    );
    socket.send(Message::binary(vec![1, 2, 3])).await.unwrap();
    assert_eq!(
        socket.next().await.unwrap().unwrap(),
        Message::binary(vec![1, 2, 3])
    );
    socket.close(None).await.unwrap();
    // The app's answer to the closing frame comes through, then the end.
    assert!(matches!(socket.next().await, Some(Ok(Message::Close(_)))));
    assert!(socket_ends(&mut socket, Duration::from_secs(5)).await);
    // Without `Origin` (not a browser) the upgrade goes through too.
    let mut request = format!("ws://{}/example/ws", hub.public)
        .into_client_request()
        .unwrap();
    request
        .headers_mut()
        .insert(header::COOKIE, cookie(&hub.max).parse().unwrap());
    let stream = TcpStream::connect(hub.public).await.unwrap();
    let (mut socket, _) = tokio_tungstenite::client_async(request, stream)
        .await
        .unwrap();
    socket.send(Message::text("again")).await.unwrap();
    assert_eq!(
        socket.next().await.unwrap().unwrap(),
        Message::text("again")
    );
}

/// Resident memory of this process (Linux).
fn resident() -> u64 {
    let statm = std::fs::read_to_string("/proc/self/statm").unwrap();
    let pages: u64 = statm.split_whitespace().nth(1).unwrap().parse().unwrap();
    pages * 4096
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn large_bodies_stream_with_flat_memory() {
    const TOTAL: u64 = 64 * 1024 * 1024;
    const CHUNK: usize = 64 * 1024;
    /// Far below the body: a buffered body would add all of it.
    const MARGIN: u64 = 24 * 1024 * 1024;
    let hub = hub().await;

    // Download: read as it arrives, checking every byte.
    let baseline = resident();
    let mut peak = baseline;
    let response = hub
        .get(
            &format!("/example/download?bytes={TOTAL}&chunk={CHUNK}"),
            Some(&hub.max),
        )
        .await;
    assert_eq!(response.status(), StatusCode::OK);
    let mut body = response.into_body();
    let mut offset = 0u64;
    let mut frames = 0u32;
    while let Some(frame) = next_frame(&mut body, Duration::from_secs(30)).await {
        let data = frame.unwrap();
        for (i, byte) in data.iter().enumerate() {
            assert_eq!(*byte, download_byte(offset + i as u64));
        }
        offset += data.len() as u64;
        frames += 1;
        if frames.is_multiple_of(64) {
            peak = peak.max(resident());
        }
    }
    assert_eq!(offset, TOTAL);
    assert!(
        peak - baseline < MARGIN,
        "download grew by {} bytes",
        peak - baseline
    );

    // Upload: produced as it is sent.
    let baseline = resident();
    let peak = Arc::new(AtomicU64::new(baseline));
    let sampled = peak.clone();
    let chunks = stream::unfold(0u64, move |sent| {
        let peak = sampled.clone();
        async move {
            if sent >= TOTAL {
                return None;
            }
            if (sent / CHUNK as u64).is_multiple_of(64) {
                peak.fetch_max(resident(), Ordering::Relaxed);
            }
            let data = Bytes::from(vec![7u8; CHUNK]);
            Some((Ok::<_, Infallible>(Frame::data(data)), sent + CHUNK as u64))
        }
    });
    let response = hub
        .send(
            Method::POST,
            "/example/upload",
            Some(&hub.max),
            &[],
            http_body_util::BodyExt::boxed(StreamBody::new(chunks)),
        )
        .await;
    let (status, _, body) = json_of(response).await;
    assert_eq!(
        (status, &body["bytes"]),
        (StatusCode::OK, &json!(TOTAL)),
        "{body}"
    );
    let grown = peak.load(Ordering::Relaxed) - baseline;
    assert!(grown < MARGIN, "upload grew by {grown} bytes");
}

/// Reads a WebSocket until it ends within `within`; `true` if it ended.
async fn socket_ends(
    socket: &mut tokio_tungstenite::WebSocketStream<TcpStream>,
    within: Duration,
) -> bool {
    tokio::time::timeout(within, async {
        loop {
            match socket.next().await {
                None | Some(Err(_)) => return,
                Some(Ok(Message::Close(_))) => return,
                Some(Ok(_)) => {}
            }
        }
    })
    .await
    .is_ok()
}

/// Reads an SSE body until it breaks off or ends within `within`.
async fn stream_ends(body: &mut Incoming, within: Duration) -> bool {
    tokio::time::timeout(within, async {
        loop {
            match body.frame().await {
                None | Some(Err(_)) => return,
                Some(Ok(_)) => {}
            }
        }
    })
    .await
    .is_ok()
}

/// An endless SSE stream of `session`, after its first event.
async fn open_stream(hub: &Hub, session: &str) -> Incoming {
    let response = hub
        .get("/example/sse?count=0&interval_ms=100", Some(session))
        .await;
    assert_eq!(response.status(), StatusCode::OK);
    let mut body = response.into_body();
    next_frame(&mut body, Duration::from_secs(2))
        .await
        .unwrap()
        .unwrap();
    body
}

#[tokio::test]
async fn ending_a_session_closes_its_streams() {
    let hub = hub().await;
    let within = Duration::from_secs(5);

    // Sign-out: stream and WebSocket of the session close, another
    // session's stay; the cookie gets no identity any more at once.
    let mut stream = open_stream(&hub, &hub.max).await;
    let mut socket = hub.websocket(&hub.max, ORIGIN).await.unwrap();
    let mut other = open_stream(&hub, &hub.anna).await;
    let reply = hub
        .env
        .as_user(&hub.max, Method::POST, "/auth/logout", None)
        .await;
    assert_eq!(reply.status, StatusCode::NO_CONTENT);
    // Both within the same 5 s.
    let (stream_closed, socket_closed) = tokio::join!(
        stream_ends(&mut stream, within),
        socket_ends(&mut socket, within)
    );
    assert!(stream_closed, "stream still open");
    assert!(socket_closed, "WebSocket still open");
    assert!(
        next_frame(&mut other, Duration::from_secs(2))
            .await
            .unwrap()
            .is_ok()
    );
    let (status, _, me) = hub.json("/example/whoami", Some(&hub.max)).await;
    assert_eq!(
        (status, &me["reason"]),
        (StatusCode::UNAUTHORIZED, &json!("missing"))
    );

    // Signed out while the app still prepares its answer: the response
    // registers after the sign-out and ends at once – before its headers or
    // right after them (the job does not run in this test).
    let late = hub.env.session_for("max").await;
    let request = Request::get(format!(
        "http://{}/example/slow?ms=800&body_ms=5000",
        hub.public
    ))
    .header(header::COOKIE, cookie(&late))
    .body(empty())
    .unwrap();
    let (response, reply) = tokio::join!(hub.client.request(request), async {
        tokio::time::sleep(Duration::from_millis(200)).await;
        hub.env
            .as_user(&late, Method::POST, "/auth/logout", None)
            .await
    });
    assert_eq!(reply.status, StatusCode::NO_CONTENT);
    if let Ok(response) = response {
        let mut body = response.into_body();
        assert!(
            stream_ends(&mut body, Duration::from_secs(2)).await,
            "response of a session ended during the wait"
        );
    }

    // The same for a WebSocket whose upgrade the app holds, with a frame the
    // browser sent right after its request – buffered at the hub until the
    // upgrade: the app never receives it.
    let late = hub.env.session_for("max").await;
    let mut tcp = TcpStream::connect(hub.public).await.unwrap();
    let mut handshake = format!(
        "GET /example/ws HTTP/1.1\r\nHost: {}\r\nConnection: Upgrade\r\n\
         Upgrade: websocket\r\nSec-WebSocket-Version: 13\r\n\
         Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nCookie: {}\r\n\r\n",
        hub.public,
        cookie(&late)
    )
    .into_bytes();
    // A masked text frame "hi" (RFC 6455 § 5.2).
    let mask = [1u8, 2, 3, 4];
    handshake.extend_from_slice(&[0x81, 0x80 | 2]);
    handshake.extend_from_slice(&mask);
    handshake.extend(b"hi".iter().zip(mask).map(|(byte, m)| byte ^ m));
    hub.example.hold_websockets(true);
    tcp.write_all(&handshake).await.unwrap();
    // The upgrade reached the app: the hub has read the session by now.
    tokio::time::timeout(Duration::from_secs(5), hub.example.websocket_held())
        .await
        .expect("upgrade not forwarded");
    let reply = hub
        .env
        .as_user(&late, Method::POST, "/auth/logout", None)
        .await;
    hub.example.hold_websockets(false);
    assert_eq!(reply.status, StatusCode::NO_CONTENT);
    // The hub answers the upgrade (or not) and closes the connection.
    let mut received = Vec::new();
    tokio::time::timeout(Duration::from_secs(3), tcp.read_to_end(&mut received))
        .await
        .expect("connection still open")
        .ok();
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(
        hub.example.websocket_messages(),
        0,
        "relayed after the sign-out"
    );

    // Locked by an administrator: every session of the user.
    let max = hub.env.session_for("max").await;
    let mut stream = open_stream(&hub, &max).await;
    let max_id = hub.user_id("max").await;
    let reply = hub
        .env
        .as_user(
            &hub.anna,
            Method::POST,
            &format!("/users/{max_id}/lock"),
            None,
        )
        .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    assert!(
        stream_ends(&mut stream, within).await,
        "stream of a locked user"
    );

    // Ended without the API (expired, another process): the job finds it.
    let anna = hub.env.session_for("anna").await;
    let mut stream = open_stream(&hub, &anna).await;
    let anna_id = hub.user_id("anna").await;
    let ended: Vec<String> = hub
        .env
        .sql(move |c| {
            let mut statement = c.prepare(
                "UPDATE sessions SET expires_at = '2000-01-01T00:00:00.000Z' \
                 WHERE user_id = ?1 RETURNING id",
            )?;
            let ids = statement
                .query_map([&anna_id], |r| r.get(0))?
                .collect::<Result<Vec<String>, _>>()?;
            Ok(ids)
        })
        .await;
    let closed = gateway_streams::run(&hub.env.state).await.unwrap();
    assert!(!closed.is_empty());
    assert!(closed.iter().all(|sid| ended.contains(sid)));
    assert!(
        stream_ends(&mut stream, within).await,
        "stream of an expired session"
    );
    assert!(
        stream_ends(&mut other, within).await,
        "anna's first session expired too"
    );
    // Nothing left to check: no database query.
    assert!(
        gateway_streams::run(&hub.env.state)
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn access_changes_reach_the_gateway_at_once() {
    let hub = hub().await;
    let within = Duration::from_secs(5);
    let eva = hub.user_id("eva").await;
    let access = format!("/users/{eva}/apps/example");

    // Without access: 403. Granting it applies to the next request, even
    // though the session was checked a moment ago (cache of 10 s).
    let (status, _, _) = hub.json("/example/whoami", Some(&hub.eva)).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let reply = hub
        .env
        .as_user(&hub.anna, Method::PUT, &access, Some(json!({})))
        .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    let (status, _, me) = hub.json("/example/whoami", Some(&hub.eva)).await;
    assert_eq!((status, &me["app_role"]), (StatusCode::OK, &json!("user")));

    // A new role: the next identity carries it.
    let reply = hub
        .env
        .as_user(
            &hub.anna,
            Method::PUT,
            &access,
            Some(json!({ "app_role": "admin" })),
        )
        .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    let (status, _, me) = hub.json("/example/whoami", Some(&hub.eva)).await;
    assert_eq!((status, &me["app_role"]), (StatusCode::OK, &json!("admin")));

    // Removing the access closes Eva's stream and WebSocket to the app at
    // once; Max's stay open, and Eva's next request gets 403.
    let mut stream = open_stream(&hub, &hub.eva).await;
    let mut socket = hub.websocket(&hub.eva, ORIGIN).await.unwrap();
    let mut other = open_stream(&hub, &hub.max).await;
    let reply = hub
        .env
        .as_user(&hub.anna, Method::DELETE, &access, None)
        .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    let (stream_closed, socket_closed) = tokio::join!(
        stream_ends(&mut stream, within),
        socket_ends(&mut socket, within)
    );
    assert!(stream_closed, "stream still open");
    assert!(socket_closed, "WebSocket still open");
    assert!(
        next_frame(&mut other, Duration::from_secs(2))
            .await
            .unwrap()
            .is_ok()
    );
    let (status, _, body) = hub.json("/example/whoami", Some(&hub.eva)).await;
    assert_eq!(
        (status, &body),
        (StatusCode::FORBIDDEN, &json!({ "error": "no_app_access" }))
    );
    // Eva is still signed in to the hub.
    let reply = hub
        .env
        .as_user(&hub.eva, Method::GET, "/session", None)
        .await;
    assert_eq!(reply.status, StatusCode::OK);
}

/// `PATCH /admin/apps/example` with `enabled` as Anna; the app as returned.
async fn switch(hub: &Hub, enabled: bool) -> Value {
    let reply = hub
        .env
        .as_user(
            &hub.anna,
            Method::PATCH,
            "/admin/apps/example",
            Some(json!({ "enabled": enabled })),
        )
        .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    reply.json()["app"].clone()
}

/// The slugs of `GET /api/v1/apps` for `session`.
async fn listed(hub: &Hub, session: &str) -> Vec<String> {
    let reply = hub.env.as_user(session, Method::GET, "/apps", None).await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    reply.json()["apps"]
        .as_array()
        .unwrap()
        .iter()
        .map(|app| app["slug"].as_str().unwrap().to_owned())
        .collect()
}

#[tokio::test]
async fn switching_the_app_off_and_on() {
    let web = tempfile::tempdir().unwrap();
    std::fs::write(
        web.path().join("index.html"),
        "<!doctype html><html><head><title>Roedy Hub</title></head><body></body></html>",
    )
    .unwrap();
    let hub = hub_with(Limits::default(), Some(web.path())).await;
    let within = Duration::from_secs(5);
    assert_eq!(listed(&hub, &hub.max).await, ["example"]);

    // Off: every open stream and WebSocket to the app closes at once – with
    // a session or without.
    let mut stream = open_stream(&hub, &hub.max).await;
    let mut socket = hub.websocket(&hub.max, ORIGIN).await.unwrap();
    let response = hub.get("/example/sse?count=0&interval_ms=100", None).await;
    assert_eq!(response.status(), StatusCode::OK);
    let mut anonymous = response.into_body();
    next_frame(&mut anonymous, Duration::from_secs(2))
        .await
        .unwrap()
        .unwrap();
    let app = switch(&hub, false).await;
    assert_eq!(app["enabled"], json!(false));
    assert!(app["disabled_at"].is_string(), "{app}");
    let (stream_closed, socket_closed, anonymous_closed) = tokio::join!(
        stream_ends(&mut stream, within),
        socket_ends(&mut socket, within),
        stream_ends(&mut anonymous, within)
    );
    assert!(stream_closed, "stream still open");
    assert!(socket_closed, "WebSocket still open");
    assert!(anonymous_closed, "stream without a session still open");

    // A request that waits for the app's answer while it is switched off
    // ends at once with 503 instead of waiting for the app.
    let app = switch(&hub, true).await;
    assert_eq!(app["enabled"], json!(true));
    let started = Instant::now();
    let (waiting, _) = tokio::join!(hub.get("/example/slow?ms=5000", Some(&hub.max)), async {
        tokio::time::sleep(Duration::from_millis(300)).await;
        switch(&hub, false).await
    });
    let (status, _, body) = json_of(waiting).await;
    assert_eq!(
        (status, &body),
        (
            StatusCode::SERVICE_UNAVAILABLE,
            &json!({ "error": "app_disabled" })
        )
    );
    assert!(
        started.elapsed() < Duration::from_secs(3),
        "{:?}",
        started.elapsed()
    );

    // 503 for everyone and every path, administrators included; the
    // request never reaches the app.
    for session in [
        None,
        Some(hub.max.as_str()),
        Some(hub.anna.as_str()),
        Some(hub.eva.as_str()),
    ] {
        for path in ["/example/whoami", "/example/", "/example"] {
            let (status, headers, body) = hub.json(path, session).await;
            assert_eq!(
                (status, &body),
                (
                    StatusCode::SERVICE_UNAVAILABLE,
                    &json!({ "error": "app_disabled" })
                ),
                "{path}"
            );
            assert_eq!(all(&headers, "cache-control"), ["no-store"]);
        }
    }
    let refused = hub.websocket(&hub.max, ORIGIN).await.unwrap_err();
    let tungstenite::Error::Http(response) = refused else {
        panic!("{refused:?}");
    };
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    let response = hub
        .get("/example/sse?count=0&interval_ms=100", Some(&hub.max))
        .await;
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    // A page load gets the hub's error page.
    let response = hub
        .send(
            Method::GET,
            "/example/",
            Some(&hub.max),
            &[("sec-fetch-mode", "navigate")],
            empty(),
        )
        .await;
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    assert!(String::from_utf8_lossy(&body).contains("content=\"app_disabled\""));
    // Not listed, not even for administrators; the administration shows it.
    assert!(listed(&hub, &hub.max).await.is_empty());
    assert!(listed(&hub, &hub.anna).await.is_empty());
    let reply = hub
        .env
        .as_user(&hub.anna, Method::GET, "/admin/apps", None)
        .await;
    assert_eq!(reply.json()["apps"][0]["enabled"], json!(false));
    // Its internal API keeps working: it can start and send its manifest.
    hub.example.send_manifest().await.unwrap();
    // Switching it off again changes nothing.
    let before = switch(&hub, false).await;
    let again = switch(&hub, false).await;
    assert_eq!(again["disabled_at"], before["disabled_at"]);

    // On again: forwarded as before, listed again.
    let app = switch(&hub, true).await;
    assert_eq!(
        (&app["enabled"], &app["disabled_at"]),
        (&json!(true), &Value::Null)
    );
    let (status, _, me) = hub.json("/example/whoami", Some(&hub.max)).await;
    assert_eq!((status, &me["app_role"]), (StatusCode::OK, &json!("user")));
    assert_eq!(listed(&hub, &hub.max).await, ["example"]);
    let mut stream = open_stream(&hub, &hub.max).await;
    assert!(
        next_frame(&mut stream, Duration::from_secs(2))
            .await
            .unwrap()
            .is_ok()
    );

    // Every switch is in the audit log, the repeated one not.
    let actions: Vec<String> = hub
        .env
        .sql(|c| {
            c.prepare("SELECT action FROM audit_log WHERE action LIKE 'apps.%able' ORDER BY id")?
                .query_map([], |r| r.get(0))?
                .collect()
        })
        .await;
    assert_eq!(
        actions,
        ["apps.disable", "apps.enable", "apps.disable", "apps.enable"]
    );
}

#[tokio::test]
async fn switching_off_needs_an_administrator_and_a_body() {
    let hub = hub().await;
    let reply = hub
        .env
        .as_user(
            &hub.max,
            Method::PATCH,
            "/admin/apps/example",
            Some(json!({ "enabled": false })),
        )
        .await;
    assert_eq!(reply.status, StatusCode::FORBIDDEN, "{}", reply.body);
    for body in [json!({}), json!({ "enabled": "no" })] {
        let reply = hub
            .env
            .as_user(
                &hub.anna,
                Method::PATCH,
                "/admin/apps/example",
                Some(body.clone()),
            )
            .await;
        assert!(reply.status.is_client_error(), "{body}: {}", reply.status);
    }
    let reply = hub
        .env
        .as_user(
            &hub.anna,
            Method::PATCH,
            "/admin/apps/nothing",
            Some(json!({ "enabled": false })),
        )
        .await;
    assert_eq!(reply.status, StatusCode::NOT_FOUND);
    let (status, _, _) = hub.json("/example/whoami", Some(&hub.max)).await;
    assert_eq!(status, StatusCode::OK);
}
