//! Apps and pairing: the administration API, `POST /internal/v1/pair`,
//! `PUT /internal/v1/manifest`, the users endpoint, the health poll and a
//! pairing with the example app over real connections.

mod common;

use std::{net::SocketAddr, time::Duration};

use axum::{
    Router,
    body::Body,
    extract::connect_info::MockConnectInfo,
    http::{Method, Request, StatusCode, header},
    routing::get,
};
use common::{Env, Reply, env};
use http_body_util::BodyExt as _;
use roedy_hub_contract::{IDENTITY_HEADER, pairing::display_pairing_code};
use roedy_hub_example::Example;
use roedy_hub_server::{
    app::internal_router,
    db,
    db::repo::{
        apps::{Health, Status},
        users::{self, Role},
    },
    identity::Subject,
    jobs::health_poll,
    settings::{self, Language, Settings},
    signing,
};
use serde_json::{Value, json};
use tokio::net::TcpListener;
use tower::ServiceExt as _;

const APP_IP: [u8; 4] = [10, 0, 0, 7];

/// Environment with signing keys, an administrator `anna` (signed in) and a
/// user `max`.
async fn setup() -> (Env, String) {
    let env = env().await;
    signing::rotate_now(&env.state.db, env.state.secrets.clone(), &env.state.keys)
        .await
        .unwrap();
    env.add_user("anna", "anna@example.com", Role::Admin).await;
    env.add_user("max", "max@example.com", Role::User).await;
    let session = env.session_for("anna").await;
    (env, session)
}

/// A request to the internal listener from `ip`.
async fn internal(
    env: &Env,
    ip: [u8; 4],
    method: Method,
    path: &str,
    secret: Option<&str>,
    body: Option<String>,
) -> Reply {
    let mut builder = Request::builder()
        .method(method)
        .uri(format!("/internal/v1{path}"));
    if let Some(secret) = secret {
        builder = builder.header(header::AUTHORIZATION, format!("Bearer {secret}"));
    }
    let request = match body {
        Some(body) => builder
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(body)),
        None => builder.body(Body::empty()),
    }
    .unwrap();
    let router: Router =
        internal_router(env.state.clone()).layer(MockConnectInfo(SocketAddr::from((ip, 4711))));
    let response = router.oneshot(request).await.unwrap();
    let (status, headers) = (response.status(), response.headers().clone());
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    Reply {
        status,
        headers,
        body: String::from_utf8(bytes.to_vec()).unwrap(),
    }
}

/// Adds an app; returns the response's `app` and the pairing code.
async fn add_app(env: &Env, session: &str, slug: &str) -> (Value, String) {
    let reply = env
        .as_user(
            session,
            Method::POST,
            "/admin/apps",
            // Nothing listens there: the health check after pairing fails
            // at once, without leaving the machine.
            Some(json!({ "slug": slug, "internal_url": "http://127.0.0.1:9" })),
        )
        .await;
    assert_eq!(reply.status, StatusCode::CREATED, "{}", reply.body);
    let json = reply.json();
    (
        json["app"].clone(),
        json["pairing"]["code"].as_str().unwrap().to_owned(),
    )
}

fn manifest() -> Value {
    roedy_hub_example::manifest()
}

fn pair_body(code: &str, slug: &str, manifest: &Value) -> String {
    json!({
        "code": code,
        "public_url": format!("https://app.example.com/{slug}"),
        "manifest": manifest,
    })
    .to_string()
}

async fn pair(env: &Env, code: &str, slug: &str, manifest: &Value) -> Reply {
    internal(
        env,
        APP_IP,
        Method::POST,
        "/pair",
        None,
        Some(pair_body(code, slug, manifest)),
    )
    .await
}

/// Pairs `slug` and returns the secret.
async fn paired_app(env: &Env, session: &str, slug: &str) -> String {
    let (_, code) = add_app(env, session, slug).await;
    let reply = pair(env, &code, slug, &manifest()).await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    reply.json()["app_secret"].as_str().unwrap().to_owned()
}

async fn user_id(env: &Env, username: &'static str) -> String {
    env.sql(move |c| {
        c.query_row(
            "SELECT id FROM users WHERE username = ?1",
            [username],
            |r| r.get(0),
        )
    })
    .await
}

async fn app_role(env: &Env, username: &'static str, slug: &'static str) -> Option<String> {
    env.sql(move |c| {
        let mut stmt = c.prepare(
            "SELECT x.app_role FROM app_access x JOIN users u ON u.id = x.user_id \
             JOIN apps a ON a.id = x.app_id WHERE u.username = ?1 AND a.slug = ?2",
        )?;
        let mut rows = stmt.query_map([username, slug], |r| r.get::<_, String>(0))?;
        rows.next().transpose()
    })
    .await
}

#[tokio::test]
async fn the_administration_needs_an_administrator() {
    let (env, admin) = setup().await;
    let user = env.session_for("max").await;
    for (method, path, body) in [
        (Method::GET, "/admin/apps", None),
        (Method::GET, "/admin/apps/mail", None),
        (
            Method::POST,
            "/admin/apps",
            Some(json!({ "slug": "mail", "internal_url": "http://mail.test" })),
        ),
        (Method::POST, "/admin/apps/mail/pairing-code", None),
        (Method::DELETE, "/admin/apps/mail", None),
    ] {
        let reply = env.as_user(&user, method.clone(), path, body.clone()).await;
        assert_eq!(reply.status, StatusCode::FORBIDDEN, "{method} {path}");
        let reply = env.request(method.clone(), path, body.clone(), &[]).await;
        assert_eq!(reply.status, StatusCode::UNAUTHORIZED, "{method} {path}");
    }
    // A foreign form cannot add an app.
    let reply = env
        .without_csrf(
            Method::POST,
            "/admin/apps",
            json!({ "slug": "mail", "internal_url": "http://mail.test" }),
            &[(roedy_hub_server::auth::session::COOKIE, &admin)],
        )
        .await;
    assert_eq!(reply.error(), "csrf");
    assert_eq!(env.count("apps").await, 0);
    // An administrator who lost the role meanwhile is refused in the change.
    let anna = user_id(&env, "anna").await;
    env.add_user("lena", "lena@example.com", Role::Admin).await;
    env.sql(move |c| users::set_role(c, &anna, Role::User))
        .await;
    let reply = env
        .as_user(
            &admin,
            Method::POST,
            "/admin/apps",
            Some(json!({ "slug": "mail", "internal_url": "http://mail.test" })),
        )
        .await;
    assert_eq!(reply.status, StatusCode::FORBIDDEN);
    assert_eq!(env.count("apps").await, 0);
}

#[tokio::test]
async fn adding_an_app_checks_slug_and_address() {
    let (env, admin) = setup().await;
    let add = |slug: &str, url: &str| {
        let body = json!({ "slug": slug, "internal_url": url });
        let env = &env;
        let admin = admin.clone();
        async move {
            env.as_user(&admin, Method::POST, "/admin/apps", Some(body))
                .await
        }
    };
    for slug in roedy_hub_contract::pairing::RESERVED_SLUGS {
        let reply = add(slug, "http://mail.test").await;
        assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY, "{slug}");
        assert_eq!(reply.json()["fields"]["slug"]["code"], "reserved", "{slug}");
    }
    for (slug, url, field, code) in [
        ("", "http://mail.test", "slug", "required"),
        ("Mail", "http://mail.test", "slug", "invalid"),
        ("m", "http://mail.test", "slug", "invalid"),
        ("mail-", "http://mail.test", "slug", "invalid"),
        ("mail", "", "internal_url", "required"),
        ("mail", "mail.test:8080", "internal_url", "invalid"),
        ("mail", "http://mail.test/mail", "internal_url", "invalid"),
        ("mail", "ftp://mail.test", "internal_url", "invalid"),
        (
            "mail",
            "http://user:pw@mail.test",
            "internal_url",
            "invalid",
        ),
    ] {
        let reply = add(slug, url).await;
        assert_eq!(
            reply.status,
            StatusCode::UNPROCESSABLE_ENTITY,
            "{slug} {url}"
        );
        assert_eq!(reply.json()["fields"][field]["code"], code, "{slug} {url}");
    }
    let reply = env
        .as_user(
            &admin,
            Method::POST,
            "/admin/apps",
            Some(json!({ "slug": "mail", "internal_url": "http://x.test", "extra": 1 })),
        )
        .await;
    assert_eq!(reply.error(), "bad_request");
    assert_eq!(env.count("apps").await, 0);

    let reply = add(" mail ", "http://Roedy-Mail:8080/").await;
    assert_eq!(reply.status, StatusCode::CREATED, "{}", reply.body);
    let json = reply.json();
    let app = &json["app"];
    assert_eq!(app["slug"], "mail");
    assert_eq!(app["internal_url"], "http://roedy-mail:8080");
    assert_eq!(app["public_url"], "https://app.example.com/mail");
    assert_eq!(app["state"], "pending");
    assert_eq!(app["access_default"], "all");
    assert_eq!(app["position"], 1);
    assert_eq!(app["name"], Value::Null);
    assert_eq!(app["health"], Value::Null);
    let pairing = &json["pairing"];
    let code = pairing["code"].as_str().unwrap();
    assert!(code.starts_with("rhp_") && code.len() == 36, "{code}");
    assert_eq!(pairing["display"], display_pairing_code(code));
    assert_eq!(app["pairing"]["expires_at"], pairing["expires_at"]);
    // Only the hash is stored.
    let stored: Vec<u8> = env
        .sql(|c| c.query_row("SELECT code_hash FROM pairing_codes", [], |r| r.get(0)))
        .await;
    assert_eq!(stored.len(), 32);
    assert!(!String::from_utf8_lossy(&stored).contains(&code[4..]));

    let reply = add("mail", "http://other.test").await;
    assert_eq!(reply.json()["fields"]["slug"]["code"], "taken");
    assert!(
        env.state.apps.get("mail").is_some(),
        "the table is reloaded"
    );

    let list = env.as_user(&admin, Method::GET, "/admin/apps", None).await;
    assert_eq!(list.json()["apps"].as_array().unwrap().len(), 1);
    let one = env
        .as_user(&admin, Method::GET, "/admin/apps/mail", None)
        .await;
    assert_eq!(one.json()["app"]["id"], app["id"]);
    let missing = env
        .as_user(&admin, Method::GET, "/admin/apps/notes", None)
        .await;
    assert_eq!(missing.status, StatusCode::NOT_FOUND);
    assert_eq!(env.audit_actions().await.last().unwrap(), "apps.create");
}

#[tokio::test]
async fn pairing_abuse_is_refused() {
    let (env, admin) = setup().await;
    let (_, code) = add_app(&env, &admin, "mail").await;

    // Wrong, malformed and missing codes look the same.
    let wrong = format!("rhp_{}", "0".repeat(32));
    for body in [
        pair_body(&wrong, "mail", &manifest()),
        pair_body("rhp_short", "mail", &manifest()),
        json!({ "public_url": "https://app.example.com/mail", "manifest": manifest() }).to_string(),
        // A wrong code wins over a broken manifest.
        pair_body(&wrong, "mail", &json!({ "contract": 1 })),
    ] {
        let reply = internal(&env, [10, 0, 0, 1], Method::POST, "/pair", None, Some(body)).await;
        assert_eq!(reply.status, StatusCode::UNAUTHORIZED);
        assert_eq!(reply.json(), json!({ "error": "invalid_code" }));
        assert_eq!(reply.header("cache-control"), Some("no-store"));
    }
    let reply = internal(
        &env,
        [10, 0, 0, 1],
        Method::POST,
        "/pair",
        None,
        Some("{".into()),
    )
    .await;
    assert_eq!(reply.error(), "bad_request");

    // The manifest before the address; neither uses up the code.
    let mut broken = manifest();
    broken["roles"] = json!([]);
    let reply = internal(
        &env,
        [10, 0, 0, 2],
        Method::POST,
        "/pair",
        None,
        Some(pair_body(&code, "notes", &broken)),
    )
    .await;
    assert_eq!(reply.status, StatusCode::BAD_REQUEST);
    assert_eq!(
        reply.json(),
        json!({ "error": "invalid_manifest", "fields": { "roles": { "code": "required" } } })
    );
    let mut future = manifest();
    future["contract"] = json!(2);
    let reply = pair(&env, &code, "mail", &future).await;
    assert_eq!(
        reply.json(),
        json!({ "error": "unsupported_contract", "supported": [1] })
    );
    let reply = internal(
        &env,
        [10, 0, 0, 2],
        Method::POST,
        "/pair",
        None,
        Some(json!({ "code": code, "public_url": "x" }).to_string()),
    )
    .await;
    assert_eq!(reply.json()["fields"]["manifest"]["code"], "required");
    for public_url in [
        "https://app.example.com/notes",
        "https://app.example.com/mail/",
        "http://app.example.com/mail",
        "https://other.example.com/mail",
    ] {
        let body = json!({ "code": code, "public_url": public_url, "manifest": manifest() });
        let reply = internal(
            &env,
            [10, 0, 0, 3],
            Method::POST,
            "/pair",
            None,
            Some(body.to_string()),
        )
        .await;
        assert_eq!(reply.status, StatusCode::CONFLICT, "{public_url}");
        assert_eq!(
            reply.json(),
            json!({ "error": "public_url_mismatch", "expected": "https://app.example.com/mail" })
        );
    }

    // Icons: too large, not sanitizable, not square; a malicious one is
    // cleaned.
    for (icon, code_) in [
        (
            format!(
                "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 24 24\"><desc>{}</desc></svg>",
                "x".repeat(70 * 1024)
            ),
            "too_large",
        ),
        ("<svg><script>".to_owned(), "unreadable"),
        (
            "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 32 16\"><path d=\"M0 0h1v1z\"/></svg>"
                .to_owned(),
            "not_square",
        ),
        (
            "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 24 24\"><text>M</text></svg>"
                .to_owned(),
            "svg_text",
        ),
    ] {
        let mut with_icon = manifest();
        with_icon["icon"] = json!(icon);
        let reply = pair(&env, &code, "mail", &with_icon).await;
        assert_eq!(reply.status, StatusCode::BAD_REQUEST, "{code_}");
        assert_eq!(reply.json()["fields"]["icon"]["code"], code_);
    }
    assert_eq!(
        env.count("pairing_codes").await,
        1,
        "the code is still open"
    );
    assert_eq!(env.state.apps.get("mail").unwrap().status, Status::Pending);

    let mut malicious = manifest();
    malicious["icon"] = json!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 24 24\" onload=\"alert(1)\">\
         <script>alert(1)</script><path d=\"M2 2h20v20H2z\" onclick=\"x()\"/>\
         <image href=\"https://tracker.invalid/p.png\" width=\"1\" height=\"1\"/></svg>"
    );
    let reply = pair(&env, &code.to_lowercase(), "mail", &malicious).await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    let icon = env
        .state
        .apps
        .get("mail")
        .unwrap()
        .manifest
        .clone()
        .unwrap()
        .icon
        .clone();
    for part in ["script", "onload", "onclick", "tracker"] {
        assert!(!icon.contains(part), "{part}: {icon}");
    }

    // Used: the same code does not work again.
    let reply = pair(&env, &code, "mail", &manifest()).await;
    assert_eq!(reply.error(), "invalid_code");

    // Expired.
    let reply = env
        .as_user(&admin, Method::POST, "/admin/apps/mail/pairing-code", None)
        .await;
    let fresh = reply.json()["pairing"]["code"].as_str().unwrap().to_owned();
    env.sql(|c| {
        c.execute(
            "UPDATE pairing_codes SET expires_at = '2000-01-01T00:00:00.000Z'",
            [],
        )
    })
    .await;
    let reply = pair(&env, &fresh, "mail", &manifest()).await;
    assert_eq!(reply.error(), "invalid_code");
}

#[tokio::test]
async fn pairing_is_rate_limited() {
    let (env, _) = setup().await;
    let wrong = pair_body(&format!("rhp_{}", "1".repeat(32)), "mail", &manifest());
    for _ in 0..10 {
        let reply = internal(
            &env,
            [10, 0, 1, 1],
            Method::POST,
            "/pair",
            None,
            Some(wrong.clone()),
        )
        .await;
        assert_eq!(reply.error(), "invalid_code");
    }
    let reply = internal(
        &env,
        [10, 0, 1, 1],
        Method::POST,
        "/pair",
        None,
        Some(wrong.clone()),
    )
    .await;
    assert_eq!(reply.status, StatusCode::TOO_MANY_REQUESTS);
    assert!(reply.json()["retry_after"].as_u64().unwrap() > 0);
    assert!(reply.header("retry-after").is_some());
    // 30 in total across addresses.
    for i in 2..=3 {
        for _ in 0..10 {
            let reply = internal(
                &env,
                [10, 0, 1, i],
                Method::POST,
                "/pair",
                None,
                Some(wrong.clone()),
            )
            .await;
            assert_eq!(reply.error(), "invalid_code");
        }
    }
    let reply = internal(
        &env,
        [10, 0, 1, 4],
        Method::POST,
        "/pair",
        None,
        Some(wrong),
    )
    .await;
    assert_eq!(reply.status, StatusCode::TOO_MANY_REQUESTS);
}

#[tokio::test]
async fn pairing_grants_access_and_a_secret() {
    let (env, admin) = setup().await;
    let (app, code) = add_app(&env, &admin, "notes").await;
    // Typed with groups and in lower case.
    let typed = display_pairing_code(&code).to_lowercase();
    let reply = pair(&env, &typed, "notes", &manifest()).await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    let paired = reply.json();
    assert_eq!(paired["app_id"], app["id"]);
    assert_eq!(paired["slug"], "notes");
    assert_eq!(paired["issuer"], "https://app.example.com");
    assert_eq!(paired["public_url"], "https://app.example.com/notes");
    assert_eq!(paired["contract"], 1);
    assert_eq!(
        paired["jwks"],
        serde_json::to_value(env.state.keys.jwks(jiff::Timestamp::now())).unwrap()
    );
    let secret = paired["app_secret"].as_str().unwrap().to_owned();
    assert!(
        secret.starts_with(&format!("rhs_{}_", app["id"].as_str().unwrap())),
        "{secret}"
    );
    // Only the hash is stored.
    let stored: Vec<u8> = env
        .sql(|c| c.query_row("SELECT secret_hash FROM apps", [], |r| r.get(0)))
        .await;
    assert_eq!(stored.len(), 32);

    // Defaults after pairing: administrators get the admin role.
    assert_eq!(
        app_role(&env, "anna", "notes").await.as_deref(),
        Some("admin")
    );
    assert_eq!(
        app_role(&env, "max", "notes").await.as_deref(),
        Some("user")
    );
    // New users get the default role.
    env.add_user("lena", "lena@example.com", Role::Admin).await;
    assert_eq!(
        app_role(&env, "lena", "notes").await.as_deref(),
        Some("user")
    );

    let shown = env
        .as_user(&admin, Method::GET, "/admin/apps/notes", None)
        .await
        .json();
    let app = &shown["app"];
    assert_eq!(app["state"], "paired");
    assert_eq!(app["name"]["de"], "Beispiel");
    assert_eq!(app["manifest_id"], "roedy.example");
    assert_eq!(app["roles"][1]["admin"], true);
    assert_eq!(app["pairing"], Value::Null, "the code is used up");
    assert!(app["paired_at"].is_string());
    let table = env.state.apps.get("notes").unwrap();
    assert_eq!(table.status, Status::Paired);
    env.wait_for_audit("apps.pair", 1).await;

    // The secret works.
    let reply = internal(&env, APP_IP, Method::GET, "/users", Some(&secret), None).await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);

    // Pairing again replaces the secret and keeps changed roles.
    env.sql(|c| c.execute("UPDATE app_access SET app_role = 'user'", []))
        .await;
    let reply = env
        .as_user(&admin, Method::POST, "/admin/apps/notes/pairing-code", None)
        .await;
    assert_eq!(reply.status, StatusCode::OK);
    let again = reply.json()["pairing"]["code"].as_str().unwrap().to_owned();
    // Until the new code is used, the old secret keeps working.
    let reply = internal(&env, APP_IP, Method::GET, "/users", Some(&secret), None).await;
    assert_eq!(reply.status, StatusCode::OK);
    let reply = pair(&env, &again, "notes", &manifest()).await;
    assert_eq!(reply.status, StatusCode::OK);
    let new_secret = reply.json()["app_secret"].as_str().unwrap().to_owned();
    assert_ne!(new_secret, secret);
    let reply = internal(&env, APP_IP, Method::GET, "/users", Some(&secret), None).await;
    assert_eq!(reply.status, StatusCode::UNAUTHORIZED);
    assert_eq!(reply.json(), json!({ "error": "invalid_app_credentials" }));
    let reply = internal(&env, APP_IP, Method::GET, "/users", Some(&new_secret), None).await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(
        app_role(&env, "anna", "notes").await.as_deref(),
        Some("user")
    );
}

#[tokio::test]
async fn the_manifest_is_updated() {
    let (env, admin) = setup().await;
    let secret = paired_app(&env, &admin, "mail").await;
    let put = |manifest: Value, secret: Option<String>| {
        let env = &env;
        async move {
            internal(
                env,
                APP_IP,
                Method::PUT,
                "/manifest",
                secret.as_deref(),
                Some(manifest.to_string()),
            )
            .await
        }
    };
    // Authentication.
    for secret in [
        None,
        Some("rhs_unknown".to_owned()),
        Some("other".to_owned()),
    ] {
        let reply = put(manifest(), secret.clone()).await;
        assert_eq!(reply.status, StatusCode::UNAUTHORIZED, "{secret:?}");
        assert_eq!(reply.error(), "invalid_app_credentials");
    }
    let reply = put(manifest(), Some(secret.clone())).await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    let accepted = reply.json();
    assert_eq!(accepted["slug"], "mail");
    assert_eq!(accepted["issuer"], "https://app.example.com");
    assert_eq!(accepted["public_url"], "https://app.example.com/mail");
    assert!(accepted["app_id"].is_string());
    // The same version: no audit entry.
    assert_eq!(
        env.audit_actions()
            .await
            .iter()
            .filter(|a| *a == "apps.update")
            .count(),
        0
    );

    // A new version without the admin role: its users fall back.
    let mut next = manifest();
    next["version"] = json!("2.0.0");
    next["name"]["en"] = json!("Example 2");
    next["roles"] = json!([
        { "id": "user", "name": { "en": "User", "de": "Benutzer" },
          "description": { "en": "", "de": "" }, "default": true }
    ]);
    assert_eq!(
        app_role(&env, "anna", "mail").await.as_deref(),
        Some("admin")
    );
    let reply = put(next, Some(secret.clone())).await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    assert_eq!(
        app_role(&env, "anna", "mail").await.as_deref(),
        Some("user")
    );
    let manifest_now = env
        .state
        .apps
        .get("mail")
        .unwrap()
        .manifest
        .clone()
        .unwrap();
    assert_eq!(manifest_now.version, "2.0.0");
    assert_eq!(manifest_now.name["en"], "Example 2");
    let actions = env.audit_actions().await;
    assert!(
        actions.ends_with(&["apps.update".into(), "apps.roles_fallback".into()]),
        "{actions:?}"
    );

    // Broken manifests change nothing.
    let mut broken = manifest();
    broken["id"] = json!("Not Valid");
    let reply = put(broken, Some(secret.clone())).await;
    assert_eq!(reply.json()["fields"]["id"]["code"], "invalid");
    let mut future = manifest();
    future["contract"] = json!(3);
    assert_eq!(
        put(future, Some(secret.clone())).await.error(),
        "unsupported_contract"
    );
    let reply = internal(
        &env,
        APP_IP,
        Method::PUT,
        "/manifest",
        Some(&secret),
        Some("[".into()),
    )
    .await;
    assert_eq!(reply.error(), "bad_request");
    assert_eq!(
        env.state
            .apps
            .get("mail")
            .unwrap()
            .manifest
            .clone()
            .unwrap()
            .version,
        "2.0.0"
    );
}

#[tokio::test]
async fn the_directory_by_app() {
    let (env, admin) = setup().await;
    env.add_user("eva", "eva@example.com", Role::User).await;
    paired_app(&env, &admin, "mail").await;
    let eva = user_id(&env, "eva").await;
    env.sql(move |c| c.execute("DELETE FROM app_access WHERE user_id = ?1", [&eva]))
        .await;
    let directory = async |session: &str, query: &str| {
        env.as_user(session, Method::GET, &format!("/directory{query}"), None)
            .await
    };
    let names = |reply: &Reply| -> Vec<String> {
        reply.json()["users"]
            .as_array()
            .unwrap()
            .iter()
            .map(|u| u["email"].as_str().unwrap().to_owned())
            .collect()
    };
    // Everyone sees every active user; with `app` only those with access.
    let max = env.session_for("max").await;
    assert_eq!(
        names(&directory(&max, "").await),
        ["anna@example.com", "eva@example.com", "max@example.com"]
    );
    assert_eq!(
        names(&directory(&max, "?app=mail").await),
        ["anna@example.com", "max@example.com"]
    );
    // Without access to the app: refused.
    let eva_session = env.session_for("eva").await;
    let reply = directory(&eva_session, "?app=mail").await;
    assert_eq!(reply.status, StatusCode::FORBIDDEN);
    assert_eq!(reply.error(), "no_access");
    // A switched-off app counts as unknown.
    env.sql(|c| {
        c.execute(
            "UPDATE apps SET disabled_at = '2026-11-28T00:00:00.000Z' WHERE slug = 'mail'",
            [],
        )
    })
    .await;
    assert_eq!(
        directory(&max, "?app=mail").await.status,
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn the_users_endpoint_lists_access_and_tombstones() {
    let (env, admin) = setup().await;
    env.add_user("eva", "eva@example.com", Role::User).await;
    let secret = paired_app(&env, &admin, "mail").await;
    let get = |path: String| {
        let env = &env;
        let secret = secret.clone();
        async move { internal(env, APP_IP, Method::GET, &path, Some(&secret), None).await }
    };
    let (anna, max, eva) = (
        user_id(&env, "anna").await,
        user_id(&env, "max").await,
        user_id(&env, "eva").await,
    );
    let mut ids = vec![anna.clone(), max.clone(), eva.clone()];
    ids.sort();

    // Locale and status.
    let max_ = max.clone();
    db::call(&env.state.db, move |c| {
        let german = Settings {
            lang: Language::German,
            ..Settings::default()
        };
        settings::save(c, &max_, 0, &german)?;
        Ok(users::lock(c, &max_)?)
    })
    .await
    .unwrap();

    let reply = get("/users".into()).await;
    assert_eq!(reply.status, StatusCode::OK);
    let list = reply.json();
    let listed: Vec<&str> = list["users"]
        .as_array()
        .unwrap()
        .iter()
        .map(|u| u["sub"].as_str().unwrap())
        .collect();
    assert_eq!(listed, ids);
    assert_eq!(list["next_cursor"], Value::Null);
    let one = get(format!("/users/{max}")).await.json();
    assert_eq!(
        one,
        json!({
            "sub": max, "username": "max", "email": "max@example.com", "name": "Max Example",
            "role": "user", "app_role": "user", "locale": "de", "status": "locked",
            "prefs": {
                "time_zone": "Europe/Berlin", "region": null, "date_format": "de",
                "time_format": "24h", "week_start": "monday", "number_format": "1.234,56",
            },
        })
    );
    assert_eq!(
        get(format!("/users/{anna}")).await.json()["app_role"],
        "admin"
    );
    assert_eq!(get(format!("/users/{anna}")).await.json()["locale"], "en");

    // Pages.
    let first = get("/users?limit=2".into()).await.json();
    assert_eq!(first["users"].as_array().unwrap().len(), 2);
    let cursor = first["next_cursor"].as_str().unwrap().to_owned();
    assert_eq!(cursor, ids[1]);
    let second = get(format!("/users?limit=2&cursor={cursor}")).await.json();
    assert_eq!(second["users"][0]["sub"], ids[2]);
    assert_eq!(second["next_cursor"], Value::Null);
    for query in ["limit=0", "limit=501", "limit=x", "limit=-1"] {
        let reply = get(format!("/users?{query}")).await;
        assert_eq!(reply.status, StatusCode::BAD_REQUEST, "{query}");
        assert_eq!(reply.error(), "bad_request");
    }
    assert_eq!(get("/users?limit=500".into()).await.status, StatusCode::OK);

    // Without access: listed no more, 404 no_access – also for unknown IDs.
    let eva_ = eva.clone();
    env.sql(move |c| c.execute("DELETE FROM app_access WHERE user_id = ?1", [&eva_]))
        .await;
    let reply = get(format!("/users/{eva}")).await;
    assert_eq!(reply.status, StatusCode::NOT_FOUND);
    assert_eq!(reply.json(), json!({ "error": "no_access" }));
    assert_eq!(
        get("/users/01JDX2Q7B5W3Y1Z9X7V5T3R1P0".into())
            .await
            .error(),
        "no_access"
    );
    assert_eq!(
        get("/users".into()).await.json()["users"]
            .as_array()
            .unwrap()
            .len(),
        2
    );

    // Deleted: 410 with the time, kept after the user is gone.
    let reply = env
        .as_user(&admin, Method::DELETE, &format!("/users/{max}"), None)
        .await;
    assert_eq!(reply.status, StatusCode::NO_CONTENT, "{}", reply.body);
    let reply = get(format!("/users/{max}")).await;
    assert_eq!(reply.status, StatusCode::GONE);
    let gone = reply.json();
    assert_eq!(gone["error"], "user_deleted");
    assert!(gone["deleted_at"].as_str().unwrap().ends_with('Z'));
    // Deleting the own account leaves one too.
    let eva_session = env.session_for("eva").await;
    let reply = env
        .as_user(
            &eva_session,
            Method::DELETE,
            "/auth/me",
            Some(json!({ "confirm": "delete", "id": eva })),
        )
        .await;
    assert_eq!(reply.status, StatusCode::NO_CONTENT, "{}", reply.body);
    assert_eq!(get(format!("/users/{eva}")).await.status, StatusCode::GONE);

    // Another app's secret sees its own users only; a removed app none.
    let reply = env
        .as_user(&admin, Method::DELETE, "/admin/apps/mail", None)
        .await;
    assert_eq!(reply.status, StatusCode::NO_CONTENT);
    assert_eq!(get("/users".into()).await.status, StatusCode::UNAUTHORIZED);
    assert_eq!(env.count("app_access").await, 0);
    assert!(env.state.apps.get("mail").is_none());
    assert_eq!(env.audit_actions().await.last().unwrap(), "apps.remove");
    let reply = env
        .as_user(&admin, Method::DELETE, "/admin/apps/mail", None)
        .await;
    assert_eq!(reply.status, StatusCode::NOT_FOUND);
}

/// Serves `router` on a free local port.
async fn serve(router: Router) -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    addr
}

/// Adds and pairs an app at `internal_url` directly in the database.
async fn add_paired(env: &Env, session: &str, slug: &str, internal_url: String) {
    let reply = env
        .as_user(
            session,
            Method::POST,
            "/admin/apps",
            Some(json!({ "slug": slug, "internal_url": internal_url })),
        )
        .await;
    assert_eq!(reply.status, StatusCode::CREATED, "{}", reply.body);
    let code = reply.json()["pairing"]["code"].as_str().unwrap().to_owned();
    let reply = pair(env, &code, slug, &manifest()).await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
}

#[tokio::test]
async fn the_health_poll_stores_online_and_offline() {
    let (env, admin) = setup().await;
    let ready = serve(Router::new().route("/up/readyz", get(|| async { "ok" }))).await;
    let failing = serve(Router::new().route(
        "/down/readyz",
        get(|| async { (StatusCode::SERVICE_UNAVAILABLE, "starting") }),
    ))
    .await;
    // A port nobody listens on.
    let closed = TcpListener::bind("127.0.0.1:0")
        .await
        .unwrap()
        .local_addr()
        .unwrap();
    add_paired(&env, &admin, "up", format!("http://{ready}")).await;
    add_paired(&env, &admin, "down", format!("http://{failing}")).await;
    add_paired(&env, &admin, "gone", format!("http://{closed}")).await;
    // Pending apps are not checked.
    add_app(&env, &admin, "pending").await;

    let results = health_poll::run(&env.state).await.unwrap();
    assert_eq!(
        results,
        [
            ("down".to_owned(), Health::Offline),
            ("gone".to_owned(), Health::Offline),
            ("up".to_owned(), Health::Online),
        ]
    );
    assert_eq!(
        env.state.apps.get("up").unwrap().health,
        Some(Health::Online)
    );
    assert_eq!(
        env.state.apps.get("down").unwrap().health,
        Some(Health::Offline)
    );
    assert_eq!(env.state.apps.get("pending").unwrap().health, None);
    let shown = env
        .as_user(&admin, Method::GET, "/admin/apps/up", None)
        .await
        .json();
    assert_eq!(shown["app"]["health"], "online");
    assert!(shown["app"]["health_at"].is_string());
    assert_eq!(
        health_poll::check_one(&env.state, "pending").await.unwrap(),
        None
    );
}

#[tokio::test]
async fn the_health_check_times_out() {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    };
    let (env, admin) = setup().await;
    // The probe reaches this handler; it answers late until `fast` is set.
    let (hits, fast) = (
        Arc::new(AtomicUsize::new(0)),
        Arc::new(AtomicBool::new(false)),
    );
    let handler = {
        let (hits, fast) = (hits.clone(), fast.clone());
        move || async move {
            hits.fetch_add(1, Ordering::SeqCst);
            if !fast.load(Ordering::SeqCst) {
                tokio::time::sleep(Duration::from_secs(5)).await;
            }
            "ok"
        }
    };
    let slow = serve(Router::new().route("/slow/readyz", get(handler))).await;
    add_paired(&env, &admin, "slow", format!("http://{slow}")).await;
    let app = env.state.apps.get("slow").unwrap();
    // The process-wide TLS provider is set by the hub's own clients.
    let client = reqwest::Client::builder()
        .timeout(Duration::from_millis(200))
        .build()
        .unwrap();
    // The check right after pairing arrives first.
    for _ in 0..200 {
        if hits.load(Ordering::SeqCst) > 0 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let before = hits.load(Ordering::SeqCst);
    assert_eq!(before, 1);
    assert_eq!(
        roedy_hub_server::apps::check_health(&client, &app).await,
        Health::Offline
    );
    assert_eq!(hits.load(Ordering::SeqCst), before + 1, "the probe arrived");
    // The same probe in time is online: only the timeout made it offline.
    fast.store(true, Ordering::SeqCst);
    assert_eq!(
        roedy_hub_server::apps::check_health(&client, &app).await,
        Health::Online
    );
}

#[tokio::test]
async fn pairs_with_the_example_app() {
    let (env, admin) = setup().await;
    // The hub's internal listener and the example app on real ports.
    let hub = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let hub_addr = hub.local_addr().unwrap();
    let internal = internal_router(env.state.clone());
    tokio::spawn(async move {
        axum::serve(
            hub,
            internal.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
        .unwrap();
    });
    let example = Example::new(
        "https://app.example.com/example",
        &format!("http://{hub_addr}"),
    )
    .unwrap();
    let app_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let app_addr = app_listener.local_addr().unwrap();
    let served = example.clone();
    tokio::spawn(async move {
        roedy_hub_example::serve(app_listener, &served, std::future::pending())
            .await
            .unwrap();
    });

    let reply = env
        .as_user(
            &admin,
            Method::POST,
            "/admin/apps",
            Some(json!({ "slug": "example", "internal_url": format!("http://{app_addr}") })),
        )
        .await;
    assert_eq!(reply.status, StatusCode::CREATED, "{}", reply.body);
    let display = reply.json()["pairing"]["display"]
        .as_str()
        .unwrap()
        .to_owned();

    // A wrong code first, then the one the administrator reads out.
    let error = example
        .pair(&format!("rhp_{}", "2".repeat(32)))
        .await
        .unwrap_err();
    assert_eq!((error.status, error.code()), (401, Some("invalid_code")));
    assert!(example.connection().is_none());
    let paired = example.pair(&display).await.unwrap();
    assert_eq!(paired.slug, "example");
    let connection = example.connection().unwrap();
    assert_eq!(connection.issuer, "https://app.example.com");

    // Right after pairing the hub checks the app's health.
    let mut health = None;
    for _ in 0..100 {
        health = env.state.apps.get("example").unwrap().health;
        if health.is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(health, Some(Health::Online));
    example.set_ready(false);
    assert_eq!(
        health_poll::check_one(&env.state, "example").await.unwrap(),
        Some(Health::Offline)
    );
    example.set_ready(true);

    // Manifest on start, users, JWKS.
    let accepted = example.send_manifest().await.unwrap();
    assert_eq!(accepted.public_url, "https://app.example.com/example");
    let users = example.users(None, Some(1)).await.unwrap();
    assert_eq!(users.users.len(), 1);
    let rest = example
        .users(users.next_cursor.as_deref(), None)
        .await
        .unwrap();
    assert_eq!(rest.users.len(), 1);
    assert_eq!(rest.next_cursor, None);
    let anna = user_id(&env, "anna").await;
    let found = example.user(&anna).await.unwrap();
    assert_eq!(
        (found.username.as_str(), found.app_role.as_str()),
        ("anna", "admin")
    );
    let missing = example
        .user("01JDX2Q7B5W3Y1Z9X7V5T3R1P0")
        .await
        .unwrap_err();
    assert_eq!((missing.status, missing.code()), (404, Some("no_access")));
    example.refresh_jwks().await.unwrap();

    // An identity the hub issues verifies at the app.
    let profile = env
        .sql({
            let anna = anna.clone();
            move |c| Ok(users::credentials_by_id(c, &anna)?.unwrap().user)
        })
        .await;
    let user = roedy_hub_server::auth::AuthUser::new(
        profile,
        "01JDX3A1C3E5G7J9K1M3N5P7Q9".into(),
        1_795_600_000,
    );
    let token = env
        .state
        .identities
        .issue(
            &env.state.keys,
            "example",
            &Subject::new(&user, "admin", Language::German),
            jiff::Timestamp::now(),
        )
        .unwrap();
    let client = reqwest::Client::new();
    let whoami = client
        .get(format!("http://{app_addr}/example/whoami"))
        .header(IDENTITY_HEADER, token.as_ref())
        .send()
        .await
        .unwrap();
    assert_eq!(whoami.status(), 200);
    let claims: Value = serde_json::from_slice(&whoami.bytes().await.unwrap()).unwrap();
    assert_eq!(claims["sub"], anna.as_str());
    assert_eq!(claims["app_role"], "admin");
    assert_eq!(claims["locale"], "de");
    // An identity for another app does not.
    let other = env
        .state
        .identities
        .issue(
            &env.state.keys,
            "mail",
            &Subject::new(&user, "admin", Language::German),
            jiff::Timestamp::now(),
        )
        .unwrap();
    let whoami = client
        .get(format!("http://{app_addr}/example/whoami"))
        .header(IDENTITY_HEADER, other.as_ref())
        .send()
        .await
        .unwrap();
    assert_eq!(whoami.status(), 401);

    // Removing the app ends the connection.
    let reply = env
        .as_user(&admin, Method::DELETE, "/admin/apps/example", None)
        .await;
    assert_eq!(reply.status, StatusCode::NO_CONTENT);
    let error = example.send_manifest().await.unwrap_err();
    assert_eq!(
        (error.status, error.code()),
        (401, Some("invalid_app_credentials"))
    );
}
