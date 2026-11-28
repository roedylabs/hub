//! Apps for the browser and access per user (milestone 5): `GET
//! /api/v1/apps`, the icons, the administration of apps (status, order,
//! access default, users), access and roles per user and in invitations,
//! with the `access.changed` events and audit entries they write – and who
//! may do what (administrators only, CSRF).

mod common;

use std::{net::SocketAddr, time::Duration};

use axum::{
    Router,
    body::Body,
    extract::{ConnectInfo, connect_info::MockConnectInfo},
    http::{HeaderMap, Method, Request, StatusCode, header},
};
use common::{Env, PASSWORD, Reply, env};
use http_body_util::BodyExt as _;
use roedy_hub_server::{
    app::{internal_router, router},
    auth::session,
    db::repo::{
        apps::{self, Health},
        users::Role,
    },
    signing,
};
use serde_json::{Value, json};
use tower::ServiceExt as _;

const APP_IP: [u8; 4] = [10, 0, 0, 7];

/// A hub with `anna` (administrator, signed in), `max` and `eva` (users),
/// the paired apps `mail` ("everyone") and `notes` ("selected only") and
/// the pending app `draft`.
struct Hub {
    env: Env,
    anna: String,
    secrets: [String; 2],
}

async fn hub() -> Hub {
    let env = env().await;
    signing::rotate_now(&env.state.db, env.state.secrets.clone(), &env.state.keys)
        .await
        .unwrap();
    env.add_user("anna", "anna@example.com", Role::Admin).await;
    env.add_user("max", "max@example.com", Role::User).await;
    env.add_user("eva", "eva@example.com", Role::User).await;
    let anna = env.session_for("anna").await;
    let mut hub = Hub {
        env,
        anna,
        secrets: [String::new(), String::new()],
    };
    let mail = hub.pair("mail").await;
    let (_, code) = hub.add("notes").await;
    let reply = hub
        .admin(
            Method::PATCH,
            "/admin/apps/notes",
            Some(json!({ "access_default": "selected" })),
        )
        .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    let notes = hub.pair_with(&code, "notes", &manifest()).await;
    hub.add("draft").await;
    hub.secrets = [mail, notes];
    hub
}

fn manifest() -> Value {
    roedy_hub_example::manifest()
}

impl Hub {
    async fn admin(&self, method: Method, path: &str, body: Option<Value>) -> Reply {
        self.env.as_user(&self.anna, method, path, body).await
    }

    /// Adds a pending app; returns it and its pairing code.
    async fn add(&self, slug: &str) -> (Value, String) {
        let reply = self
            .admin(
                Method::POST,
                "/admin/apps",
                // Nothing listens there: health checks fail at once.
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

    /// Adds and pairs an app; returns its secret.
    async fn pair(&self, slug: &str) -> String {
        let (_, code) = self.add(slug).await;
        self.pair_with(&code, slug, &manifest()).await
    }

    async fn pair_with(&self, code: &str, slug: &str, manifest: &Value) -> String {
        let body = json!({
            "code": code,
            "public_url": format!("https://app.example.com/{slug}"),
            "manifest": manifest,
        });
        let reply = self.internal(Method::POST, "/pair", None, Some(body)).await;
        assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
        // The health check right after pairing, so that it does not
        // overwrite what a test sets later.
        let slug = slug.to_owned();
        for _ in 0..200 {
            let slug = slug.clone();
            let checked: bool = self
                .env
                .sql(move |c| {
                    c.query_row(
                        "SELECT health_at IS NOT NULL FROM apps WHERE slug = ?1",
                        [slug],
                        |r| r.get(0),
                    )
                })
                .await;
            if checked {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        reply.json()["app_secret"].as_str().unwrap().to_owned()
    }

    async fn internal(
        &self,
        method: Method,
        path: &str,
        secret: Option<&str>,
        body: Option<Value>,
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
                .body(Body::from(body.to_string())),
            None => builder.body(Body::empty()),
        }
        .unwrap();
        let router: Router = internal_router(self.env.state.clone())
            .layer(MockConnectInfo(SocketAddr::from((APP_IP, 4711))));
        reply(router.oneshot(request).await.unwrap()).await
    }

    /// A `GET` outside `/api/v1` with `session` and `headers` (gateway,
    /// icons).
    async fn get(&self, path: &str, session: Option<&str>, headers: &[(&str, &str)]) -> Reply {
        // The gateway reads the peer address from the connection.
        let mut builder = Request::builder()
            .uri(path)
            .extension(ConnectInfo(SocketAddr::from((common::CLIENT, 4711))));
        if let Some(session) = session {
            builder = builder.header(header::COOKIE, format!("{}={session}", session::COOKIE));
        }
        for (name, value) in headers {
            builder = builder.header(*name, *value);
        }
        let app: Router = router(self.env.state.clone())
            .layer(MockConnectInfo(SocketAddr::from((common::CLIENT, 4711))));
        reply(
            app.oneshot(builder.body(Body::empty()).unwrap())
                .await
                .unwrap(),
        )
        .await
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

    /// The `access.changed` events of an app after `seq`: `data` each.
    async fn access_events(&self, slug: &'static str, after: i64) -> Vec<Value> {
        let rows: Vec<(String, String)> = self
            .env
            .sql(move |c| {
                c.prepare(
                    "SELECT e.type, e.data FROM events e JOIN apps a ON a.id = e.app_id \
                     WHERE a.slug = ?1 AND e.seq > ?2 ORDER BY e.seq",
                )?
                .query_map((slug, after), |r| Ok((r.get(0)?, r.get(1)?)))?
                .collect()
            })
            .await;
        rows.into_iter()
            .map(|(kind, data)| {
                assert_eq!(kind, "access.changed");
                serde_json::from_str(&data).unwrap()
            })
            .collect()
    }

    /// The highest `seq` of an app's events.
    async fn last_seq(&self, slug: &'static str) -> i64 {
        self.env
            .sql(move |c| {
                c.query_row(
                    "SELECT coalesce(max(e.seq), 0) FROM events e JOIN apps a ON a.id = e.app_id \
                     WHERE a.slug = ?1",
                    [slug],
                    |r| r.get(0),
                )
            })
            .await
    }

    /// Audit entries of `action`: their details.
    async fn audit(&self, action: &'static str) -> Vec<Value> {
        let rows: Vec<Option<String>> = self
            .env
            .sql(move |c| {
                c.prepare("SELECT details FROM audit_log WHERE action = ?1 ORDER BY id")?
                    .query_map([action], |r| r.get(0))?
                    .collect()
            })
            .await;
        rows.into_iter()
            .map(|details| serde_json::from_str(&details.unwrap_or_default()).unwrap_or_default())
            .collect()
    }

    /// The slugs of `GET /api/v1/apps` for `session`.
    async fn tiles(&self, session: &str) -> Vec<String> {
        let reply = self.env.as_user(session, Method::GET, "/apps", None).await;
        assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
        reply.json()["apps"]
            .as_array()
            .unwrap()
            .iter()
            .map(|app| app["slug"].as_str().unwrap().to_owned())
            .collect()
    }
}

async fn reply(response: axum::response::Response) -> Reply {
    let (status, headers) = (response.status(), response.headers().clone());
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    Reply {
        status,
        headers,
        body: String::from_utf8_lossy(&bytes).into_owned(),
    }
}

fn access(sub: &str, role: Option<&str>) -> Value {
    json!({ "sub": sub, "access": role.is_some(), "app_role": role })
}

fn header<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers.get(name).map(|v| v.to_str().unwrap())
}

#[tokio::test]
async fn permission_matrix_of_apps_icons_and_gateway() {
    let hub = hub().await;
    let (max, eva) = (
        hub.env.session_for("max").await,
        hub.env.session_for("eva").await,
    );
    let (max_id, eva_id) = (hub.user_id("max").await, hub.user_id("eva").await);
    // Eva gets `notes` (selected only) and loses `mail`; Anna loses `notes`.
    let anna_id = hub.user_id("anna").await;
    for (method, path) in [
        (Method::PUT, format!("/users/{eva_id}/apps/notes")),
        (Method::DELETE, format!("/users/{eva_id}/apps/mail")),
        (Method::DELETE, format!("/users/{anna_id}/apps/notes")),
    ] {
        let body = (method == Method::PUT).then(|| json!({}));
        let reply = hub.admin(method, &path, body).await;
        assert_eq!(reply.status, StatusCode::OK, "{path}: {}", reply.body);
    }

    // admin/user × access yes/no × paired/pending. Pending apps are never
    // listed, not even for administrators; the gateway lets only them in
    // (nothing listens: 502).
    assert_eq!(hub.tiles(&hub.anna).await, ["mail"]);
    assert_eq!(hub.tiles(&max).await, ["mail"]);
    assert_eq!(hub.tiles(&eva).await, ["notes"]);
    for (session, path, status) in [
        (&hub.anna, "/mail/", StatusCode::BAD_GATEWAY),
        (&hub.anna, "/notes/", StatusCode::FORBIDDEN),
        (&hub.anna, "/draft/", StatusCode::BAD_GATEWAY),
        (&max, "/mail/", StatusCode::BAD_GATEWAY),
        (&max, "/notes/", StatusCode::FORBIDDEN),
        (&max, "/draft/", StatusCode::SERVICE_UNAVAILABLE),
        (&eva, "/mail/", StatusCode::FORBIDDEN),
        (&eva, "/notes/", StatusCode::BAD_GATEWAY),
        (&eva, "/draft/", StatusCode::SERVICE_UNAVAILABLE),
    ] {
        let reply = hub.get(path, Some(session), &[]).await;
        assert_eq!(reply.status, status, "{path}: {}", reply.body);
    }
    // Signed out: no list; the icons are public (paired apps only).
    let reply = hub.env.request(Method::GET, "/apps", None, &[]).await;
    assert_eq!(reply.status, StatusCode::UNAUTHORIZED);
    for (path, status) in [
        ("/api/v1/apps/mail/icon.svg", StatusCode::OK),
        ("/api/v1/apps/notes/icon.svg", StatusCode::OK),
        ("/api/v1/apps/draft/icon.svg", StatusCode::NOT_FOUND),
        ("/api/v1/apps/nothing/icon.svg", StatusCode::NOT_FOUND),
    ] {
        let reply = hub.get(path, None, &[]).await;
        assert_eq!(reply.status, status, "{path}");
        let signed_in = hub.get(path, Some(&max), &[]).await;
        assert_eq!(signed_in.status, status, "{path}");
    }
    let reply = hub.get("/api/v1/apps/draft/icon.svg", None, &[]).await;
    assert_eq!(reply.body, r#"{"error":"not_found"}"#);
    assert_eq!(header(&reply.headers, "cache-control"), Some("no-store"));

    // The entry: name and description in the user's language, the address
    // of the app's home, the versioned icon and the health.
    let settings = hub.env.as_user(&max, Method::GET, "/settings", None).await;
    let reply = hub
        .env
        .as_user(
            &max,
            Method::PUT,
            "/settings",
            Some(json!({ "version": settings.json()["version"], "settings": { "lang": "de" } })),
        )
        .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    let reply = hub.env.as_user(&max, Method::GET, "/apps", None).await;
    assert_eq!(reply.header("cache-control"), Some("no-store"));
    let entry = &reply.json()["apps"][0];
    let icon = entry["icon"].as_str().unwrap().to_owned();
    assert!(icon.starts_with("/api/v1/apps/mail/icon.svg?v="), "{icon}");
    assert_eq!(icon.len(), "/api/v1/apps/mail/icon.svg?v=".len() + 16);
    assert_eq!(
        entry,
        &json!({
            "slug": "mail", "name": "Beispiel",
            "description": "Test-App, die den App-Vertrag umsetzt",
            "url": "/mail/", "icon": icon, "status": "offline",
        })
    );
    let mail = hub.app_id("mail").await;
    hub.env
        .sql(move |c| apps::set_health(c, &mail, Health::Online))
        .await;
    let reply = hub.env.as_user(&max, Method::GET, "/apps", None).await;
    assert_eq!(reply.json()["apps"][0]["status"], "online");
    assert_eq!(reply.json()["apps"][0]["name"], "Beispiel");
    let reply = hub.env.as_user(&hub.anna, Method::GET, "/apps", None).await;
    assert_eq!(reply.json()["apps"][0]["name"], "Example");

    // The session describes the administrator's links (contract § 6).
    let reply = hub
        .env
        .as_user(&hub.anna, Method::GET, "/session", None)
        .await;
    assert_eq!(reply.json()["hub"]["admin"], "/admin");
    assert_eq!(reply.json()["hub"]["name"], "Roedy Hub");
    let reply = hub.env.as_user(&max, Method::GET, "/session", None).await;
    assert_eq!(reply.json()["hub"]["admin"], Value::Null);
    let _ = max_id;
}

#[tokio::test]
async fn icons_are_cacheable_and_change_their_address() {
    let hub = hub().await;
    let max = hub.env.session_for("max").await;
    let url = hub
        .env
        .as_user(&max, Method::GET, "/apps", None)
        .await
        .json()["apps"][0]["icon"]
        .as_str()
        .unwrap()
        .to_owned();
    let version = url.split("?v=").nth(1).unwrap().to_owned();

    // As an image of a page (Fetch Metadata), with the current version: a
    // year in the cache, with the contract's CSP.
    let reply = hub
        .get(
            &url,
            None,
            &[("sec-fetch-dest", "image"), ("sec-fetch-mode", "no-cors")],
        )
        .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    assert_eq!(
        header(&reply.headers, "content-type"),
        Some("image/svg+xml")
    );
    assert_eq!(
        header(&reply.headers, "cache-control"),
        Some("public, max-age=31536000, immutable")
    );
    assert_eq!(
        header(&reply.headers, "content-security-policy"),
        Some("default-src 'none'; style-src 'unsafe-inline'; sandbox")
    );
    assert_eq!(
        header(&reply.headers, "x-content-type-options"),
        Some("nosniff")
    );
    let etag = header(&reply.headers, "etag").unwrap().to_owned();
    assert_eq!(etag, format!("\"{version}\""));
    assert!(reply.body.starts_with("<svg"), "{}", reply.body);
    assert!(
        reply.body.contains(r#"viewBox="0 0 24 24""#),
        "{}",
        reply.body
    );
    // Without or with another version: revalidated, never pinned.
    for path in [
        "/api/v1/apps/mail/icon.svg".to_owned(),
        "/api/v1/apps/mail/icon.svg?v=0000000000000000".to_owned(),
    ] {
        let reply = hub.get(&path, None, &[]).await;
        assert_eq!(reply.status, StatusCode::OK);
        assert_eq!(
            header(&reply.headers, "cache-control"),
            Some("no-cache"),
            "{path}"
        );
        assert!(header(&reply.headers, "content-security-policy").is_some());
    }
    let reply = hub
        .get(
            "/api/v1/apps/mail/icon.svg",
            None,
            &[("if-none-match", &etag)],
        )
        .await;
    assert_eq!(reply.status, StatusCode::NOT_MODIFIED);
    assert!(reply.body.is_empty());

    // A new icon with the next manifest: a new address in the list, the old
    // one is revalidated and gets the new icon.
    let mut next = manifest();
    next["version"] = json!("2.0.0");
    next["icon"] = json!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 16 16\"><circle cx=\"8\" cy=\"8\" r=\"6\"/></svg>"
    );
    let reply = hub
        .internal(Method::PUT, "/manifest", Some(&hub.secrets[0]), Some(next))
        .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    let moved = hub
        .env
        .as_user(&max, Method::GET, "/apps", None)
        .await
        .json()["apps"][0]["icon"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_ne!(moved, url);
    let reply = hub.get(&url, None, &[]).await;
    assert_eq!(header(&reply.headers, "cache-control"), Some("no-cache"));
    assert!(
        reply.body.contains(r#"viewBox="0 0 16 16""#),
        "{}",
        reply.body
    );
    let reply = hub
        .get(
            "/api/v1/apps/mail/icon.svg",
            None,
            &[("if-none-match", &etag)],
        )
        .await;
    assert_eq!(
        reply.status,
        StatusCode::OK,
        "the old ETag no longer matches"
    );
    let reply = hub.get(&moved, None, &[]).await;
    assert_eq!(
        header(&reply.headers, "cache-control"),
        Some("public, max-age=31536000, immutable")
    );
    // The administration names the same address.
    let reply = hub.admin(Method::GET, "/admin/apps/mail", None).await;
    assert_eq!(reply.json()["app"]["icon"], moved.as_str());
    assert_eq!(reply.json()["app"]["url"], "/mail/");
}

#[tokio::test]
async fn administration_shows_status_and_sets_order_and_access_default() {
    let hub = hub().await;
    let reply = hub.admin(Method::GET, "/admin/apps", None).await;
    let apps = reply.json()["apps"].as_array().unwrap().clone();
    let slugs: Vec<&str> = apps.iter().map(|a| a["slug"].as_str().unwrap()).collect();
    assert_eq!(slugs, ["mail", "notes", "draft"]);
    // Status: pairing, health, contact, events, users with access.
    let mail = &apps[0];
    assert_eq!(mail["state"], "paired");
    assert_eq!(mail["health"], "offline");
    assert!(mail["health_at"].is_string());
    assert_eq!(mail["online_at"], Value::Null);
    assert!(mail["contacted_at"].is_string(), "pairing counts");
    assert_eq!(mail["user_count"], 3);
    assert_eq!(mail["access_default"], "all");
    assert_eq!(mail["roles"][1]["id"], "admin");
    assert_eq!(
        mail["events"],
        json!({
            "pending": 3, "attempts": 0, "failing_since": null, "retry_at": null,
            "last_error": null, "given_up": false,
        })
    );
    assert_eq!(apps[1]["user_count"], 1, "notes: only the administrator");
    assert_eq!(apps[1]["access_default"], "selected");
    let draft = &apps[2];
    assert_eq!(
        (
            &draft["state"],
            &draft["icon"],
            &draft["user_count"],
            &draft["contacted_at"]
        ),
        (&json!("pending"), &Value::Null, &json!(0), &Value::Null)
    );
    assert_eq!(draft["events"]["pending"], 0);
    // A passed health check and a request with the secret show.
    let mail_id = hub.app_id("mail").await;
    hub.env
        .sql(move |c| {
            c.execute("UPDATE apps SET contacted_at = NULL", [])?;
            apps::set_health(c, &mail_id, Health::Online)
        })
        .await;
    let reply = hub
        .internal(Method::GET, "/users?limit=1", Some(&hub.secrets[0]), None)
        .await;
    assert_eq!(reply.status, StatusCode::OK);
    let mail = hub
        .admin(Method::GET, "/admin/apps/mail", None)
        .await
        .json()["app"]
        .clone();
    assert_eq!(mail["health"], "online");
    assert!(mail["online_at"].is_string());
    assert!(mail["contacted_at"].is_string());

    // Order: every app once, else 422; the list and the tiles follow.
    for order in [
        json!([]),
        json!(["notes", "mail"]),
        json!(["notes", "mail", "draft", "draft"]),
        json!(["notes", "mail", "nothing"]),
        json!(["notes", "mail", "draft", "nothing"]),
    ] {
        let reply = hub
            .admin(
                Method::PATCH,
                "/admin/apps",
                Some(json!({ "order": order })),
            )
            .await;
        assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY, "{order}");
        assert!(reply.json()["fields"]["order"]["code"].is_string());
    }
    let reply = hub
        .admin(
            Method::PATCH,
            "/admin/apps",
            Some(json!({ "order": ["draft", "notes", "mail"] })),
        )
        .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    let positions: Vec<(String, i64)> = reply.json()["apps"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| {
            (
                a["slug"].as_str().unwrap().into(),
                a["position"].as_i64().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        positions,
        [("draft".into(), 1), ("notes".into(), 2), ("mail".into(), 3)]
    );
    assert_eq!(hub.tiles(&hub.anna).await, ["notes", "mail"]);
    // The same order again changes nothing and writes no entry.
    let reply = hub
        .admin(
            Method::PATCH,
            "/admin/apps",
            Some(json!({ "order": ["draft", "notes", "mail"] })),
        )
        .await;
    assert_eq!(reply.status, StatusCode::OK);
    let entries = hub.audit("apps.order").await;
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0]["order"], json!(["draft", "notes", "mail"]));
    assert_eq!(entries[0]["previous"], json!(["mail", "notes", "draft"]));

    // Access default: only for new users; existing access stays.
    let reply = hub
        .admin(
            Method::PATCH,
            "/admin/apps/mail",
            Some(json!({ "access_default": "selected" })),
        )
        .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    assert_eq!(reply.json()["app"]["access_default"], "selected");
    assert_eq!(
        reply.json()["app"]["user_count"],
        3,
        "existing access stays"
    );
    hub.env
        .add_user("lena", "lena@example.com", Role::User)
        .await;
    let lena = hub.env.session_for("lena").await;
    assert!(hub.tiles(&lena).await.is_empty());
    let reply = hub
        .admin(
            Method::PATCH,
            "/admin/apps/mail",
            Some(json!({ "access_default": "all" })),
        )
        .await;
    assert_eq!(
        reply.json()["app"]["user_count"],
        3,
        "Lena keeps having none"
    );
    hub.env.add_user("tom", "tom@example.com", Role::User).await;
    let tom = hub.env.session_for("tom").await;
    assert_eq!(hub.tiles(&tom).await, ["mail"]);
    // Unchanged: no entry. Wrong values: 400.
    hub.admin(
        Method::PATCH,
        "/admin/apps/mail",
        Some(json!({ "access_default": "all" })),
    )
    .await;
    let entries = hub.audit("apps.access_default").await;
    // `notes` in `hub()`, then `mail` twice.
    assert_eq!(entries.len(), 3);
    assert_eq!(
        (
            &entries[1]["access_default"],
            &entries[1]["previous"],
            &entries[1]["slug"]
        ),
        (&json!("selected"), &json!("all"), &json!("mail"))
    );
    for body in [
        json!({ "access_default": "none" }),
        json!({ "position": 1 }),
    ] {
        let reply = hub
            .admin(Method::PATCH, "/admin/apps/mail", Some(body))
            .await;
        assert_eq!(reply.status, StatusCode::BAD_REQUEST);
    }
    let reply = hub
        .admin(
            Method::PATCH,
            "/admin/apps/nothing",
            Some(json!({ "access_default": "all" })),
        )
        .await;
    assert_eq!(reply.status, StatusCode::NOT_FOUND);

    // The app's users: everyone with their role, and the invitations.
    let reply = hub
        .admin(
            Method::POST,
            "/invites",
            Some(json!({ "emails": ["jonas@example.com"], "role": "user" })),
        )
        .await;
    assert_eq!(reply.status, StatusCode::CREATED, "{}", reply.body);
    let reply = hub
        .admin(Method::GET, "/admin/apps/notes/users", None)
        .await;
    assert_eq!(reply.status, StatusCode::OK);
    let users: Vec<(String, Value)> = reply.json()["users"]
        .as_array()
        .unwrap()
        .iter()
        .map(|u| {
            (
                u["username"].as_str().unwrap().into(),
                u["app_role"].clone(),
            )
        })
        .collect();
    assert_eq!(users.len(), 5);
    assert!(users.contains(&("anna".into(), json!("admin"))));
    assert!(users.contains(&("max".into(), Value::Null)));
    assert_eq!(
        reply.json()["invites"][0]["app_role"],
        Value::Null,
        "selected only"
    );
    let reply = hub.admin(Method::GET, "/admin/apps/mail/users", None).await;
    assert_eq!(reply.json()["invites"][0]["app_role"], "user");
    let reply = hub
        .admin(Method::GET, "/admin/apps/nothing/users", None)
        .await;
    assert_eq!(reply.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn access_per_user_gives_events_and_audit_entries() {
    let hub = hub().await;
    let (anna, eva) = (hub.user_id("anna").await, hub.user_id("eva").await);
    let path = format!("/users/{eva}/apps/notes");
    let (notes_seq, mail_seq) = (hub.last_seq("notes").await, hub.last_seq("mail").await);

    // Grant with the default role, change the role, remove; repeating a
    // change does nothing.
    let reply = hub.admin(Method::PUT, &path, Some(json!({}))).await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    assert_eq!(
        reply.json()["user"]["apps"],
        json!({ "mail": "user", "notes": "user" })
    );
    let reply = hub
        .admin(Method::PUT, &path, Some(json!({ "app_role": "admin" })))
        .await;
    assert_eq!(reply.json()["user"]["apps"]["notes"], "admin");
    let again = hub
        .admin(Method::PUT, &path, Some(json!({ "app_role": "admin" })))
        .await;
    assert_eq!(again.status, StatusCode::OK);
    let reply = hub.admin(Method::DELETE, &path, None).await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    assert_eq!(reply.json()["user"]["apps"], json!({ "mail": "user" }));
    let again = hub.admin(Method::DELETE, &path, None).await;
    assert_eq!(again.status, StatusCode::OK);
    assert_eq!(
        hub.access_events("notes", notes_seq).await,
        [
            access(&eva, Some("user")),
            access(&eva, Some("admin")),
            access(&eva, None),
        ]
    );
    assert_eq!(
        hub.last_seq("mail").await,
        mail_seq,
        "only the app concerned"
    );
    let grant = hub.audit("apps.access_grant").await;
    let role = hub.audit("apps.access_role").await;
    let revoke = hub.audit("apps.access_revoke").await;
    assert_eq!((grant.len(), role.len(), revoke.len()), (1, 1, 1));
    assert_eq!(
        grant[0],
        json!({
            "slug": "notes", "name": "Max Example", "email": "eva@example.com",
            "app_role": "user", "previous": null,
        })
    );
    assert_eq!(
        (&role[0]["app_role"], &role[0]["previous"]),
        (&json!("admin"), &json!("user"))
    );
    assert_eq!(
        (&revoke[0]["app_role"], &revoke[0]["previous"]),
        (&Value::Null, &json!("admin"))
    );
    // The audit log shows them in the category `apps`.
    let reply = hub
        .admin(Method::GET, "/admin/audit?category=apps", None)
        .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    assert!(
        reply
            .body
            .contains("gave Max Example access to notes with the role user"),
        "{}",
        reply.body
    );

    // The user's row and the list carry the apps.
    let reply = hub.admin(Method::GET, &format!("/users/{eva}"), None).await;
    assert_eq!(reply.json()["user"]["apps"], json!({ "mail": "user" }));
    let reply = hub.admin(Method::GET, "/users", None).await;
    let row = reply.json()["users"]
        .as_array()
        .unwrap()
        .iter()
        .find(|u| u["id"] == eva.as_str())
        .unwrap()
        .clone();
    assert_eq!(row["apps"], json!({ "mail": "user" }));

    // Also for the own account and a locked one.
    let reply = hub
        .admin(
            Method::PUT,
            &format!("/users/{anna}/apps/mail"),
            Some(json!({ "app_role": "user" })),
        )
        .await;
    assert_eq!(reply.json()["user"]["apps"]["mail"], "user");
    let reply = hub
        .admin(Method::POST, &format!("/users/{eva}/lock"), None)
        .await;
    assert_eq!(reply.status, StatusCode::OK);
    let reply = hub.admin(Method::PUT, &path, Some(json!({}))).await;
    assert_eq!(reply.json()["user"]["apps"]["notes"], "user");

    // What does not fit.
    for (method, path, body, status, code) in [
        (
            Method::PUT,
            format!("/users/{eva}/apps/draft"),
            json!({}),
            StatusCode::CONFLICT,
            "app_pending",
        ),
        (
            Method::PUT,
            format!("/users/{eva}/apps/nothing"),
            json!({}),
            StatusCode::NOT_FOUND,
            "not_found",
        ),
        (
            Method::PUT,
            "/users/nobody/apps/mail".to_owned(),
            json!({}),
            StatusCode::NOT_FOUND,
            "not_found",
        ),
        (
            Method::PUT,
            format!("/users/{eva}/apps/mail"),
            json!({ "app_role": "owner" }),
            StatusCode::UNPROCESSABLE_ENTITY,
            "invalid",
        ),
        (
            Method::DELETE,
            format!("/users/{eva}/apps/draft"),
            Value::Null,
            StatusCode::CONFLICT,
            "app_pending",
        ),
    ] {
        let body = (!body.is_null()).then_some(body);
        let reply = hub.admin(method, &path, body).await;
        assert_eq!(
            (reply.status, reply.error().as_str()),
            (status, code),
            "{path}"
        );
    }
    let reply = hub
        .admin(
            Method::PUT,
            &format!("/users/{eva}/apps/mail"),
            Some(json!({ "app_role": "owner" })),
        )
        .await;
    assert_eq!(reply.json()["fields"]["app_role"]["code"], "invalid");
}

#[tokio::test]
async fn bulk_access_for_users_and_invitations() {
    let hub = hub().await;
    let (anna, max, eva) = (
        hub.user_id("anna").await,
        hub.user_id("max").await,
        hub.user_id("eva").await,
    );
    let reply = hub
        .admin(
            Method::POST,
            "/invites",
            Some(json!({ "emails": ["jonas@example.com"], "role": "user" })),
        )
        .await;
    let invite = reply.json()["invites"][0]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let seq = hub.last_seq("notes").await;

    // Admin role in `notes` for the selection: the own account counts here.
    let bulk = async |body: Value| hub.admin(Method::POST, "/users/bulk", Some(body)).await;
    let ids = json!([anna, max, eva, invite, "nobody"]);
    let reply = bulk(json!({
        "action": "access", "ids": ids, "slug": "notes", "access": true, "app_role": "admin",
    }))
    .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    let mut changed: Vec<String> = serde_json::from_value(reply.json()["changed"].clone()).unwrap();
    changed.sort();
    let mut expected = vec![max.clone(), eva.clone(), invite.clone()];
    expected.sort();
    assert_eq!(changed, expected, "Anna had it already");
    let mut events = hub.access_events("notes", seq).await;
    events.sort_by_key(|e| e["sub"].as_str().unwrap().to_owned());
    let mut want = vec![access(&max, Some("admin")), access(&eva, Some("admin"))];
    want.sort_by_key(|e| e["sub"].as_str().unwrap().to_owned());
    assert_eq!(events, want);
    let reply = hub.admin(Method::GET, "/invites", None).await;
    assert_eq!(
        reply.json()["invites"][0]["apps"],
        json!({ "mail": "user", "notes": "admin" })
    );

    // Without access, the default role when none is named.
    let reply =
        bulk(json!({ "action": "access", "ids": [max, invite], "slug": "notes", "access": false }))
            .await;
    assert_eq!(reply.json()["changed"].as_array().unwrap().len(), 2);
    let reply =
        bulk(json!({ "action": "access", "ids": [max], "slug": "notes", "access": true })).await;
    assert_eq!(reply.json()["changed"], json!([max]));
    let reply = hub.admin(Method::GET, &format!("/users/{max}"), None).await;
    assert_eq!(reply.json()["user"]["apps"]["notes"], "user");
    let reply = hub.admin(Method::GET, "/invites", None).await;
    assert_eq!(
        reply.json()["invites"][0]["apps"],
        json!({ "mail": "user" })
    );
    assert_eq!(hub.audit("apps.invite_access").await.len(), 2);

    // What does not fit.
    for (body, status) in [
        (
            json!({ "action": "access", "ids": [max], "access": true }),
            StatusCode::UNPROCESSABLE_ENTITY,
        ),
        (
            json!({ "action": "access", "ids": [max], "slug": "notes" }),
            StatusCode::UNPROCESSABLE_ENTITY,
        ),
        (
            json!({ "action": "access", "ids": [max], "slug": "draft", "access": true }),
            StatusCode::CONFLICT,
        ),
        (
            json!({ "action": "access", "ids": [max], "slug": "nothing", "access": true }),
            StatusCode::NOT_FOUND,
        ),
        (
            json!({ "action": "access", "ids": [max], "slug": "notes", "access": true, "app_role": "owner" }),
            StatusCode::UNPROCESSABLE_ENTITY,
        ),
    ] {
        let reply = bulk(body.clone()).await;
        assert_eq!(reply.status, status, "{body}");
    }
    // Other bulk actions still skip the own account.
    let reply = bulk(json!({ "action": "lock", "ids": [anna] })).await;
    assert_eq!(reply.json()["changed"], json!([]));
}

/// Accepts an invitation (without a system mail: the link from "Copy
/// link"); returns the new user's ID.
async fn accept(hub: &Hub, invite: &str) -> String {
    let link = hub
        .admin(Method::GET, &format!("/invites/{invite}/link"), None)
        .await;
    let url = link.json()["url"].as_str().unwrap().to_owned();
    let token = url.rsplit('/').next().unwrap().to_owned();
    let reply = hub
        .env
        .request(
            Method::POST,
            &format!("/invites/{token}/accept"),
            Some(json!({ "name": "Jonas Example", "password": PASSWORD })),
            &[],
        )
        .await;
    assert_eq!(reply.status, StatusCode::CREATED, "{}", reply.body);
    reply.json()["user"]["id"].as_str().unwrap().to_owned()
}

#[tokio::test]
async fn invitations_carry_the_access_chosen_in_the_form() {
    let hub = hub().await;
    let invite = async |email: &'static str, apps: Value| {
        hub.admin(
            Method::POST,
            "/invites",
            Some(json!({ "emails": [email], "role": "user", "apps": apps })),
        )
        .await
    };
    // Checked per app.
    let reply = invite(
        "jonas@example.com",
        json!({ "nothing": "user", "draft": "user", "notes": "owner", "mail": null }),
    )
    .await;
    assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        reply.json()["fields"],
        json!({
            "apps.nothing": { "code": "not_found" },
            "apps.draft": { "code": "app_pending" },
            "apps.notes": { "code": "invalid" },
        })
    );
    assert_eq!(hub.env.count("invitations").await, 0);

    // `notes` as administrator, no `mail` (whose default is "everyone").
    let reply = invite(
        "jonas@example.com",
        json!({ "notes": "admin", "mail": null }),
    )
    .await;
    assert_eq!(reply.status, StatusCode::CREATED, "{}", reply.body);
    let jonas = reply.json()["invites"][0]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let reply = hub.admin(Method::GET, "/invites", None).await;
    assert_eq!(
        reply.json()["invites"][0]["apps"],
        json!({ "notes": "admin" })
    );
    let entry = &hub.audit("users.invite").await[0];
    assert_eq!(entry["apps"], json!({ "notes": "admin" }));
    // Changed later: only the apps named.
    let reply = hub
        .admin(
            Method::PATCH,
            &format!("/invites/{jonas}"),
            Some(json!({ "apps": { "mail": "admin" } })),
        )
        .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    assert_eq!(
        reply.json()["invite"]["apps"],
        json!({ "mail": "admin", "notes": "admin" })
    );
    assert_eq!(reply.json()["invite"]["role"], "user");
    let reply = hub
        .admin(
            Method::PATCH,
            &format!("/invites/{jonas}"),
            Some(json!({ "role": "admin", "apps": { "draft": null } })),
        )
        .await;
    assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY);
    let reply = hub
        .admin(
            Method::PATCH,
            &format!("/invites/{jonas}"),
            Some(json!({ "role": "admin" })),
        )
        .await;
    assert_eq!(reply.json()["invite"]["role"], "admin");
    let entries = hub.audit("apps.invite_access").await;
    assert_eq!(entries.len(), 1);
    assert_eq!(
        entries[0],
        json!({ "slug": "mail", "email": "jonas@example.com", "app_role": "admin", "previous": null })
    );

    // Accepting grants exactly that, with `access.changed` for each app.
    let (mail_seq, notes_seq) = (hub.last_seq("mail").await, hub.last_seq("notes").await);
    let id = accept(&hub, &jonas).await;
    assert_eq!(
        hub.access_events("mail", mail_seq).await,
        [access(&id, Some("admin"))]
    );
    assert_eq!(
        hub.access_events("notes", notes_seq).await,
        [access(&id, Some("admin"))]
    );
    let reply = hub.admin(Method::GET, &format!("/users/{id}"), None).await;
    assert_eq!(
        reply.json()["user"]["apps"],
        json!({ "mail": "admin", "notes": "admin" })
    );
    assert_eq!(
        hub.audit("users.invite_accept").await[0]["apps"],
        json!({ "mail": "admin", "notes": "admin" })
    );
    assert_eq!(hub.env.count("invitation_access").await, 0);

    // Without a choice: the defaults when accepting – and a role the
    // manifest dropped meanwhile becomes its default role.
    let reply = invite("lena@example.com", json!({ "notes": "admin" })).await;
    let lena = reply.json()["invites"][0]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let mut next = manifest();
    next["roles"][1]["id"] = json!("owner");
    let reply = hub
        .internal(Method::PUT, "/manifest", Some(&hub.secrets[1]), Some(next))
        .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    let reply = hub.admin(Method::GET, "/invites", None).await;
    assert_eq!(
        reply.json()["invites"][0]["apps"],
        json!({ "mail": "user", "notes": "user" })
    );
    let (mail_seq, notes_seq) = (hub.last_seq("mail").await, hub.last_seq("notes").await);
    let id = accept(&hub, &lena).await;
    assert_eq!(
        hub.access_events("mail", mail_seq).await,
        [access(&id, Some("user"))]
    );
    assert_eq!(
        hub.access_events("notes", notes_seq).await,
        [access(&id, Some("user"))]
    );

    // Removing an app removes the invitations' choices.
    let reply = invite("tom@example.com", json!({ "notes": null })).await;
    assert_eq!(reply.status, StatusCode::CREATED);
    assert_eq!(hub.env.count("invitation_access").await, 1);
    let reply = hub.admin(Method::DELETE, "/admin/apps/notes", None).await;
    assert_eq!(reply.status, StatusCode::NO_CONTENT);
    assert_eq!(hub.env.count("invitation_access").await, 0);
}

#[tokio::test]
async fn only_administrators_change_access_and_never_without_csrf() {
    let hub = hub().await;
    let max = hub.env.session_for("max").await;
    let eva = hub.user_id("eva").await;
    let invite = hub
        .admin(
            Method::POST,
            "/invites",
            Some(json!({ "emails": ["jonas@example.com"], "role": "user" })),
        )
        .await
        .json()["invites"][0]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let changes = [
        (
            Method::PATCH,
            "/admin/apps".to_owned(),
            json!({ "order": ["notes", "mail", "draft"] }),
        ),
        (
            Method::PATCH,
            "/admin/apps/mail".to_owned(),
            json!({ "access_default": "selected" }),
        ),
        (Method::PUT, format!("/users/{eva}/apps/notes"), json!({})),
        (Method::DELETE, format!("/users/{eva}/apps/mail"), json!({})),
        (
            Method::POST,
            "/users/bulk".to_owned(),
            json!({ "action": "access", "ids": [eva], "slug": "notes", "access": true }),
        ),
        (
            Method::PATCH,
            format!("/invites/{invite}"),
            json!({ "apps": { "notes": "user" } }),
        ),
        (
            Method::POST,
            "/invites".to_owned(),
            json!({ "emails": ["lena@example.com"], "role": "user", "apps": { "notes": "user" } }),
        ),
    ];
    let reads = ["/admin/apps", "/admin/apps/mail", "/admin/apps/mail/users"];
    let before = hub.env.count("audit_log").await;
    for (method, path, body) in &changes {
        let reply = hub
            .env
            .as_user(&max, method.clone(), path, Some(body.clone()))
            .await;
        assert_eq!(reply.status, StatusCode::FORBIDDEN, "{method} {path}");
        let reply = hub
            .env
            .request(method.clone(), path, Some(body.clone()), &[])
            .await;
        assert_eq!(reply.status, StatusCode::UNAUTHORIZED, "{method} {path}");
        let reply = hub
            .env
            .without_csrf(
                method.clone(),
                path,
                body.clone(),
                &[(session::COOKIE, &hub.anna)],
            )
            .await;
        assert_eq!(reply.error(), "csrf", "{method} {path}");
    }
    for path in reads {
        let reply = hub.env.as_user(&max, Method::GET, path, None).await;
        assert_eq!(reply.status, StatusCode::FORBIDDEN, "{path}");
    }
    assert_eq!(hub.env.count("audit_log").await, before);
    assert_eq!(hub.tiles(&max).await, ["mail"]);
    // The administrator gets through with the same requests.
    for (method, path, body) in &changes {
        let reply = hub.admin(method.clone(), path, Some(body.clone())).await;
        assert!(reply.status.is_success(), "{method} {path}: {}", reply.body);
    }
}
