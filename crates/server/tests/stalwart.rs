//! Stalwart servers, users' accounts and the token broker against the fake
//! Stalwart (`fake_stalwart`): the registry with DCR and diagnosis, the
//! sign-in with PKCE whose code the hub exchanges, strict rotation of the
//! refresh tokens, "Sign in again", removal with revocation, accounts with
//! an access token and the SSRF guard for custom servers, the internal API
//! for an app's backend (the example app), the events and the jobs.

mod common;
mod fake_stalwart;

use std::{collections::HashMap, net::SocketAddr};

use axum::http::{Method, StatusCode, header};
use common::{Env, Reply, env};
use fake_stalwart::{
    ADMIN, ADMIN_PASSWORD, CLIENT_ID, CLIENT_SECRET, MAIL_TOKEN, MAIL_USER, OTHER_TOKEN, Options,
    Server,
};
use roedy_hub_example::Example;
use roedy_hub_server::{
    app::internal_router,
    auth::session,
    db::{
        self,
        repo::{stalwart_servers, users::Role},
    },
    jobs::{quota_check, server_probe, token_refresh},
    rate_limit::Key,
    signing,
    stalwart::{
        oauth,
        token::{self, REFRESH_FAILED_LIMIT},
    },
};
use serde_json::{Value, json};
use tokio::net::TcpListener;
use url::Url;

const ORIGIN: &str = "https://app.example.com";

/// Options of a Stalwart server whose token endpoint requires the secret
/// that DCR hands out.
fn confidential() -> Options {
    Options {
        client_secret: Some(CLIENT_SECRET),
        ..Options::default()
    }
}

/// A hub with `anna` (administrator) and `max`, `eva` (users), signed in.
struct Hub {
    env: Env,
    anna: String,
    max: String,
    eva: String,
}

async fn hub() -> Hub {
    let env = env().await;
    env.add_user("anna", "anna@example.com", Role::Admin).await;
    env.add_user("max", "max@example.com", Role::User).await;
    env.add_user("eva", "eva@example.com", Role::User).await;
    let anna = env.session_for("anna").await;
    let max = env.session_for("max").await;
    let eva = env.session_for("eva").await;
    Hub {
        env,
        anna,
        max,
        eva,
    }
}

impl Hub {
    async fn admin(&self, method: Method, path: &str, body: Option<Value>) -> Reply {
        self.env.as_user(&self.anna, method, path, body).await
    }

    async fn max(&self, method: Method, path: &str, body: Option<Value>) -> Reply {
        self.env.as_user(&self.max, method, path, body).await
    }

    /// Adds `fake` to the registry and registers the hub's client there by
    /// DCR; returns the server's ID.
    async fn registered(&self, fake: &Server) -> String {
        let created = self
            .admin(
                Method::POST,
                "/admin/stalwart/servers",
                Some(json!({ "label": "Mail", "url": fake.url, "domains": ["example.com"] })),
            )
            .await;
        assert_eq!(created.status, StatusCode::CREATED, "{}", created.body);
        let id = created.json()["id"].as_str().unwrap().to_owned();
        let reply = self
            .admin(
                Method::POST,
                &format!("/admin/stalwart/servers/{id}/register-client"),
                Some(json!({ "username": ADMIN, "password": ADMIN_PASSWORD })),
            )
            .await;
        assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
        id
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

    /// The stored events as `(app, type, data)`, oldest first.
    async fn events(&self) -> Vec<(String, String, Value)> {
        self.env
            .sql(|c| {
                c.prepare("SELECT app_id, type, data FROM events ORDER BY app_id, seq")?
                    .query_map([], |r| {
                        let data: String = r.get(2)?;
                        Ok((r.get(0)?, r.get(1)?, serde_json::from_str(&data).unwrap()))
                    })?
                    .collect()
            })
            .await
    }
}

/// The sign-in at the fake as a browser goes through it: start, the
/// server's sign-in page, and the hand-over of the code by the page at
/// `/oauth/callback`. Returns the reply of `complete` and the query the
/// server redirected back with.
async fn sign_in(env: &Env, session: &str, body: Value) -> (Reply, HashMap<String, String>) {
    let start = env
        .as_user(session, Method::POST, "/stalwart/sign-in", Some(body))
        .await;
    assert_eq!(start.status, StatusCode::OK, "{}", start.body);
    let back = authorize(start.json()["authorize_url"].as_str().unwrap()).await;
    let reply = env
        .as_user(
            session,
            Method::POST,
            "/stalwart/sign-in/complete",
            Some(json!({ "state": back["state"], "code": back["code"], "iss": back["iss"] })),
        )
        .await;
    (reply, back)
}

/// Follows the server's sign-in page; returns the query of the redirect back.
async fn authorize(url: &str) -> HashMap<String, String> {
    let http = roedy_hub_server::stalwart::http_client().unwrap();
    let response = http.get(url).send().await.unwrap();
    assert_eq!(response.status(), StatusCode::FOUND);
    let location = response.headers()[header::LOCATION].to_str().unwrap();
    let location = Url::parse(location).unwrap();
    assert_eq!(
        location.as_str().split('?').next(),
        Some("https://app.example.com/oauth/callback")
    );
    location.query_pairs().into_owned().collect()
}

#[tokio::test]
async fn administrators_keep_the_registry() {
    let hub = hub().await;
    let fake = Server::start(confidential()).await;

    // Only administrators, and the address must be an origin.
    let reply = hub
        .max(
            Method::POST,
            "/admin/stalwart/servers",
            Some(json!({ "label": "Mail", "url": fake.url })),
        )
        .await;
    assert_eq!(reply.status, StatusCode::FORBIDDEN);
    let reply = hub
        .admin(
            Method::POST,
            "/admin/stalwart/servers",
            Some(json!({ "label": "", "url": "https://mail.example.com/jmap" })),
        )
        .await;
    assert_eq!(
        reply.json()["fields"],
        json!({ "label": { "code": "required" }, "url": { "code": "invalid" } })
    );

    let id = hub.registered(&fake).await;
    let server = hub
        .admin(Method::GET, &format!("/admin/stalwart/servers/{id}"), None)
        .await
        .json();
    assert_eq!(server["client_id"], CLIENT_ID);
    assert_eq!(server["has_secret"], true);
    assert_eq!(server["accounts"], 0);
    // DCR asked for every app's scope and the hub's redirect.
    let registration = &fake.registrations()[0];
    assert_eq!(
        registration["redirect_uris"],
        json!(["https://app.example.com/oauth/callback"])
    );
    assert_eq!(registration["client_name"], "Roedy Hub");
    assert!(
        registration["scope"]
            .as_str()
            .unwrap()
            .contains("urn:ietf:params:oauth:scope:calendars")
    );
    // A second origin is taken; wrong credentials at DCR count.
    let reply = hub
        .admin(
            Method::POST,
            "/admin/stalwart/servers",
            Some(json!({ "label": "Again", "url": fake.url })),
        )
        .await;
    assert_eq!(reply.json()["fields"]["url"]["code"], "taken");
    let reply = hub
        .admin(
            Method::POST,
            &format!("/admin/stalwart/servers/{id}/register-client"),
            Some(json!({ "username": ADMIN, "password": "wrong" })),
        )
        .await;
    assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(reply.json()["reason"], "auth_failed");

    // Diagnosis: session, CORS for the hub's origin and OAuth.
    let tested = hub
        .admin(
            Method::POST,
            &format!("/admin/stalwart/servers/{id}/test"),
            None,
        )
        .await;
    assert_eq!(tested.status, StatusCode::OK, "{}", tested.body);
    let server = &tested.json()["server"];
    assert_eq!(
        (&server["reach"], &server["cors"], &server["oauth"]),
        (&json!("ok"), &json!("ok"), &json!("ok"))
    );

    // Users see the server without the secret.
    let list = hub.max(Method::GET, "/stalwart/servers", None).await;
    assert_eq!(list.json()["servers"][0]["sign_in"], true);
    assert_eq!(list.json()["allow_custom_servers"], false);
    assert!(!list.body.contains(CLIENT_SECRET));
    let list = hub
        .admin(Method::GET, "/admin/stalwart/servers", None)
        .await;
    assert_eq!(
        list.json()["redirect_uri"],
        "https://app.example.com/oauth/callback"
    );
    assert!(!list.body.contains(CLIENT_SECRET));

    // A new address drops the secret and the diagnosis.
    let moved = hub
        .admin(
            Method::PATCH,
            &format!("/admin/stalwart/servers/{id}"),
            Some(json!({ "url": "https://mail.example.com" })),
        )
        .await;
    assert_eq!(moved.json()["has_secret"], false, "{}", moved.body);
    assert_eq!(moved.json()["reach"], "unk");
    let policy = hub
        .admin(
            Method::PUT,
            "/admin/stalwart/policy",
            Some(json!({ "allow_custom_servers": true })),
        )
        .await;
    assert_eq!(policy.json()["allow_custom_servers"], true);
    let removed = hub
        .admin(
            Method::DELETE,
            &format!("/admin/stalwart/servers/{id}"),
            None,
        )
        .await;
    assert_eq!(removed.status, StatusCode::NO_CONTENT);
    let actions = hub.env.audit_actions().await;
    for action in [
        "stalwart.servers.create",
        "stalwart.servers.register_client",
        "stalwart.servers.update",
        "stalwart.policy",
        "stalwart.servers.delete",
    ] {
        assert!(actions.iter().any(|a| a == action), "{action}: {actions:?}");
    }
}

#[tokio::test]
async fn the_hub_signs_in_with_pkce_and_exchanges_the_code() {
    let hub = hub().await;
    let fake = Server::start(confidential()).await;
    let server = hub.registered(&fake).await;

    let (reply, back) = sign_in(
        &hub.env,
        &hub.max,
        json!({ "server_id": server, "email": MAIL_USER, "return_to": "/mail/" }),
    )
    .await;
    assert_eq!(reply.status, StatusCode::CREATED, "{}", reply.body);
    assert_eq!(reply.json()["return_to"], "/mail/");
    let account = &reply.json()["account"];
    assert_eq!(account["email"], MAIL_USER);
    assert_eq!(account["auth"], "oauth");
    assert_eq!(account["default"], true);
    assert_eq!(account["state"], "ok");
    assert_eq!(account["server"]["id"], json!(server));
    assert_eq!(account["session_url"], format!("{}/jmap/session", fake.url));
    assert!(
        account["capabilities"]
            .as_array()
            .unwrap()
            .contains(&json!("urn:ietf:params:jmap:mail"))
    );
    // The browser saw neither verifier nor refresh token.
    assert!(!reply.body.contains("refresh"));

    // The sign-in page got the S256 challenge, the hint and every scope;
    // the token request the verifier and the client's secret via Basic.
    let asked = &fake.authorizations()[0];
    assert_eq!(asked["code_challenge_method"], "S256");
    assert_eq!(asked["login_hint"], MAIL_USER);
    assert!(asked["scope"].contains("offline_access"));
    assert!(!asked.contains_key("code_verifier"));
    let exchange = &fake.token_requests()[0];
    assert_eq!(exchange.form["grant_type"], "authorization_code");
    assert_eq!(exchange.form["code"], back["code"]);
    assert!(
        exchange
            .authorization
            .as_deref()
            .unwrap()
            .starts_with("Basic ")
    );

    // A state counts once.
    let again = hub
        .max(
            Method::POST,
            "/stalwart/sign-in/complete",
            Some(json!({ "state": back["state"], "code": back["code"] })),
        )
        .await;
    assert_eq!(again.status, StatusCode::BAD_REQUEST);
    assert_eq!(again.error(), "invalid_state");

    // The same account again: 409 `dup`, and the new grant is revoked.
    let (dup, _) = sign_in(
        &hub.env,
        &hub.max,
        json!({ "server_id": server, "email": MAIL_USER }),
    )
    .await;
    assert_eq!(dup.status, StatusCode::CONFLICT, "{}", dup.body);
    assert_eq!(dup.error(), "dup");
    wait_for(|| fake.revocations().len() == 1).await;

    // Another session cannot complete someone else's sign-in.
    let start = hub
        .max(
            Method::POST,
            "/stalwart/sign-in",
            Some(json!({ "server_id": server, "email": "eva@example.com" })),
        )
        .await;
    let back = authorize(start.json()["authorize_url"].as_str().unwrap()).await;
    let foreign = hub
        .env
        .as_user(
            &hub.eva,
            Method::POST,
            "/stalwart/sign-in/complete",
            Some(json!({ "state": back["state"], "code": back["code"] })),
        )
        .await;
    assert_eq!(foreign.status, StatusCode::CONFLICT);
    assert_eq!(foreign.error(), "session_changed");

    // The server's error comes back with the path to return to.
    let start = hub
        .max(
            Method::POST,
            "/stalwart/sign-in",
            Some(json!({ "server_id": server, "email": MAIL_USER })),
        )
        .await;
    let back = authorize(start.json()["authorize_url"].as_str().unwrap()).await;
    let denied = hub
        .max(
            Method::POST,
            "/stalwart/sign-in/complete",
            Some(json!({ "state": back["state"], "error": "access_denied" })),
        )
        .await;
    assert_eq!(denied.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(denied.json()["reason"], "access_denied");
    assert_eq!(denied.json()["return_to"], "/account/accounts");

    // A server without the hub's client offers no sign-in.
    let other = Server::start(Options::default()).await;
    let bare = hub.env.add_server("bare", &other.url).await;
    let reply = hub
        .max(
            Method::POST,
            "/stalwart/sign-in",
            Some(json!({ "server_id": bare, "email": MAIL_USER })),
        )
        .await;
    assert_eq!(reply.json()["fields"]["server_id"]["code"], "no_client");
}

#[tokio::test]
async fn the_broker_rotates_strictly_and_hands_out_short_lived_tokens() {
    let hub = hub().await;
    // Access tokens live shorter than the broker's margin: every fetch
    // refreshes.
    let fake = Server::start(Options {
        expires_in: 60,
        ..confidential()
    })
    .await;
    let server = hub.registered(&fake).await;
    let (reply, _) = sign_in(
        &hub.env,
        &hub.max,
        json!({ "server_id": server, "email": MAIL_USER }),
    )
    .await;
    let id = reply.json()["account"]["id"].as_str().unwrap().to_owned();
    let path = format!("/stalwart/accounts/{id}/token");

    let first = hub.max(Method::POST, &path, None).await;
    assert_eq!(first.status, StatusCode::OK, "{}", first.body);
    let token = first.json()["access_token"].as_str().unwrap().to_owned();
    assert!(first.json()["expires_at"].as_str().unwrap().ends_with('Z'));
    assert_eq!(first.header("cache-control"), Some("no-store"));
    let requests = fake.token_requests();
    let refresh = requests.last().unwrap();
    assert_eq!(refresh.form["grant_type"], "refresh_token");
    let used = refresh.form["refresh_token"].clone();

    // A lost response: the hub tries the same refresh token once more, and
    // Stalwart issues the pending generation again.
    fake.lose_token_responses(true);
    let lost = hub.max(Method::POST, &path, None).await;
    assert_eq!(lost.status, StatusCode::OK, "{}", lost.body);
    assert_eq!(
        lost.json()["access_token"],
        json!(token),
        "the previous token"
    );
    // The hub kept the refresh token it had: the server still takes it.
    let kept = fake.token_requests().last().unwrap().form["refresh_token"].clone();
    assert!(fake.refresh_valid(&kept));
    fake.lose_token_responses(false);
    // Past the pause after the temporary error.
    let origin = Url::parse(&fake.url)
        .unwrap()
        .origin()
        .ascii_serialization();
    hub.env
        .state
        .limiter
        .reset(&REFRESH_FAILED_LIMIT, &Key::Server(origin));
    let next = hub.max(Method::POST, &path, None).await;
    assert_eq!(next.status, StatusCode::OK, "{}", next.body);
    assert_ne!(next.json()["access_token"], json!(token));
    // Rotated strictly: once the next generation was used, the first refresh
    // token would revoke the grant – the hub never sends it again.
    assert!(!fake.refresh_valid(&used));
    assert_ne!(
        fake.token_requests().last().unwrap().form["refresh_token"],
        used
    );

    // Only for the own accounts, and only with `X-Requested-With`.
    let foreign = hub.env.as_user(&hub.eva, Method::POST, &path, None).await;
    assert_eq!(foreign.status, StatusCode::NOT_FOUND);
    let forged = hub
        .env
        .without_csrf(
            Method::POST,
            &path,
            json!({}),
            &[(session::COOKIE, &hub.max)],
        )
        .await;
    assert_eq!(forged.status, StatusCode::FORBIDDEN);

    // The grant is revoked at the server: the account needs a new sign-in,
    // and the apps hear of it.
    fake.revoke_refresh_tokens();
    let gone = hub
        .max(Method::POST, &path, Some(json!({ "refresh": true })))
        .await;
    assert_eq!(gone.status, StatusCode::CONFLICT, "{}", gone.body);
    assert_eq!(gone.error(), "needs_sign_in");
    let list = hub.max(Method::GET, "/stalwart/accounts", None).await;
    assert_eq!(list.json()["accounts"][0]["state"], "needs_sign_in");

    // Signing in again brings it back; another account would not do.
    fake.sign_in_as(Some(OTHER_TOKEN));
    let (other, _) = sign_in(&hub.env, &hub.max, json!({ "account_id": id })).await;
    assert_eq!(other.status, StatusCode::CONFLICT, "{}", other.body);
    assert_eq!(other.error(), "other_account");
    fake.sign_in_as(None);
    let (again, _) = sign_in(&hub.env, &hub.max, json!({ "account_id": id })).await;
    assert_eq!(again.status, StatusCode::OK, "{}", again.body);
    assert_eq!(again.json()["account"]["state"], "ok");
    let fresh = hub.max(Method::POST, &path, None).await;
    assert_eq!(fresh.status, StatusCode::OK, "{}", fresh.body);
}

#[tokio::test]
async fn removing_an_account_revokes_its_grant() {
    let hub = hub().await;
    let fake = Server::start(confidential()).await;
    let server = hub.registered(&fake).await;
    let (first, _) = sign_in(
        &hub.env,
        &hub.max,
        json!({ "server_id": server, "email": MAIL_USER }),
    )
    .await;
    let (second, _) = sign_in(
        &hub.env,
        &hub.max,
        json!({ "server_id": server, "email": "eva@example.com" }),
    )
    .await;
    let (first, second) = (
        first.json()["account"]["id"].as_str().unwrap().to_owned(),
        second.json()["account"]["id"].as_str().unwrap().to_owned(),
    );
    assert_eq!(
        hub.admin(Method::GET, "/admin/stalwart/servers", None)
            .await
            .json()["servers"][0]["accounts"],
        2
    );
    // Colour and default account.
    let reply = hub
        .max(
            Method::PATCH,
            &format!("/stalwart/accounts/{second}"),
            Some(json!({ "color": 4, "default": true })),
        )
        .await;
    assert_eq!(reply.json()["color"], 4, "{}", reply.body);
    assert_eq!(reply.json()["default"], true);
    let reply = hub
        .max(
            Method::PATCH,
            &format!("/stalwart/accounts/{first}"),
            Some(json!({ "color": 6, "default": false })),
        )
        .await;
    assert_eq!(
        reply.json()["fields"],
        json!({ "color": { "code": "invalid" }, "default": { "code": "invalid" } })
    );

    let removed = hub
        .max(
            Method::DELETE,
            &format!("/stalwart/accounts/{second}"),
            None,
        )
        .await;
    assert_eq!(removed.status, StatusCode::NO_CONTENT);
    wait_for(|| fake.revocations().len() == 1).await;
    let revocation = &fake.revocations()[0];
    assert_eq!(revocation.form["token_type_hint"], "refresh_token");
    assert!(!fake.grant_exists(&revocation.form["token"]));
    // The remaining account is the default one now.
    let list = hub.max(Method::GET, "/stalwart/accounts", None).await;
    assert_eq!(list.json()["accounts"].as_array().unwrap().len(), 1);
    assert_eq!(list.json()["accounts"][0]["default"], true);

    // Deleting the user revokes the rest.
    let max = hub.user_id("max").await;
    let reply = hub
        .admin(Method::DELETE, &format!("/users/{max}"), None)
        .await;
    assert_eq!(reply.status, StatusCode::NO_CONTENT, "{}", reply.body);
    wait_for(|| fake.revocations().len() == 2).await;
    assert_eq!(hub.env.count("stalwart_accounts").await, 0);
}

#[tokio::test]
async fn accounts_with_a_token_and_custom_servers() {
    let hub = hub().await;
    let fake = Server::start(Options {
        quota: Some((2048, 4096)),
        ..Options::default()
    })
    .await;
    let server = hub.env.add_server("s1", &fake.url).await;

    // An access token on a registry server.
    let create = |body: Value| hub.max(Method::POST, "/stalwart/accounts", Some(body));
    let wrong = create(json!({ "server_id": server, "email": MAIL_USER, "token": "nope" })).await;
    assert_eq!(
        wrong.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "{}",
        wrong.body
    );
    assert_eq!(wrong.json()["reason"], "auth_failed");
    let created =
        create(json!({ "server_id": server, "email": MAIL_USER, "token": MAIL_TOKEN })).await;
    assert_eq!(created.status, StatusCode::CREATED, "{}", created.body);
    assert_eq!(created.json()["auth"], "token");
    let id = created.json()["id"].as_str().unwrap().to_owned();
    let token = hub
        .max(
            Method::POST,
            &format!("/stalwart/accounts/{id}/token"),
            None,
        )
        .await;
    assert_eq!(token.json()["access_token"], MAIL_TOKEN);
    assert_eq!(token.json()["expires_at"], Value::Null);
    // The body is optional, but must fit when it is there.
    let odd = hub
        .max(
            Method::POST,
            &format!("/stalwart/accounts/{id}/token"),
            Some(json!({ "refresh": "yes" })),
        )
        .await;
    assert_eq!(odd.status, StatusCode::BAD_REQUEST, "{}", odd.body);
    // The address of the server counts as that server.
    let dup = create(json!({ "url": fake.url, "email": MAIL_USER, "token": MAIL_TOKEN })).await;
    assert_eq!(dup.error(), "dup", "{}", dup.body);

    // The check reads the storage.
    let checked = hub
        .max(
            Method::POST,
            &format!("/stalwart/accounts/{id}/check"),
            None,
        )
        .await;
    assert_eq!(checked.status, StatusCode::OK, "{}", checked.body);
    assert_eq!(checked.json()["quota"]["used"], 2048);
    assert_eq!(checked.json()["quota"]["total"], 4096);

    // Custom servers only with the policy, and only public addresses.
    let custom =
        json!({ "url": "https://127.0.0.1:8443", "email": MAIL_USER, "token": MAIL_TOKEN });
    let reply = create(custom.clone()).await;
    assert_eq!(reply.json()["fields"]["url"]["code"], "not_allowed");
    hub.admin(
        Method::PUT,
        "/admin/stalwart/policy",
        Some(json!({ "allow_custom_servers": true })),
    )
    .await;
    let reply = create(custom).await;
    assert_eq!(
        reply.json()["fields"]["url"]["code"],
        "private",
        "{}",
        reply.body
    );
    let reply =
        create(json!({ "url": "https://localhost", "email": MAIL_USER, "token": MAIL_TOKEN }))
            .await;
    assert_eq!(
        reply.json()["fields"]["url"]["code"],
        "private",
        "{}",
        reply.body
    );
    let reply = create(
        json!({ "url": "http://mail.example.com", "email": MAIL_USER, "token": MAIL_TOKEN }),
    )
    .await;
    assert_eq!(reply.json()["fields"]["url"]["code"], "invalid");

    // A removed registry server leaves its accounts blocked while the policy
    // is off (a loopback address is no custom server anyway).
    hub.admin(
        Method::DELETE,
        &format!("/admin/stalwart/servers/{server}"),
        None,
    )
    .await;
    let list = hub.max(Method::GET, "/stalwart/accounts", None).await;
    assert_eq!(list.json()["accounts"][0]["state"], "blocked");
    assert_eq!(list.json()["accounts"][0]["server"], Value::Null);
    let blocked = hub
        .max(
            Method::POST,
            &format!("/stalwart/accounts/{id}/token"),
            None,
        )
        .await;
    assert_eq!(blocked.error(), "blocked");
}

/// A hub with its internal listener and the example app paired at
/// `/example` (its manifest asks for Stalwart accounts with `backend`).
async fn with_example(hub: &Hub) -> Example {
    signing::rotate_now(
        &hub.env.state.db,
        hub.env.state.secrets.clone(),
        &hub.env.state.keys,
    )
    .await
    .unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let internal = format!("http://{}", listener.local_addr().unwrap());
    let router = internal_router(hub.env.state.clone());
    tokio::spawn(async move {
        axum::serve(
            listener,
            router.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
        .unwrap();
    });
    let example = Example::new(&format!("{ORIGIN}/example"), &internal).unwrap();
    let reply = hub
        .admin(
            Method::POST,
            "/admin/apps",
            Some(json!({ "slug": "example", "internal_url": "http://127.0.0.1:9" })),
        )
        .await;
    assert_eq!(reply.status, StatusCode::CREATED, "{}", reply.body);
    let code = reply.json()["pairing"]["code"].as_str().unwrap().to_owned();
    example.pair(&code).await.unwrap();
    example
}

#[tokio::test]
async fn apps_get_servers_accounts_tokens_and_events() {
    let hub = hub().await;
    let fake = Server::start(confidential()).await;
    let server = hub.registered(&fake).await;
    let example = with_example(&hub).await;
    let max = hub.user_id("max").await;

    let servers = example.stalwart_servers().await.unwrap();
    assert_eq!(servers, json!({ "origins": [fake.url], "custom": false }));
    assert_eq!(
        example.stalwart_accounts(&max).await.unwrap(),
        json!({ "accounts": [] })
    );
    let (reply, _) = sign_in(
        &hub.env,
        &hub.max,
        json!({ "server_id": server, "email": MAIL_USER }),
    )
    .await;
    let id = reply.json()["account"]["id"].as_str().unwrap().to_owned();
    let accounts = example.stalwart_accounts(&max).await.unwrap();
    assert_eq!(accounts["accounts"][0]["id"], json!(id));
    assert_eq!(accounts["accounts"][0], reply.json()["account"]);
    let token = example.stalwart_token(&max, &id, false).await.unwrap();
    assert!(
        token["access_token"]
            .as_str()
            .unwrap()
            .starts_with("access-")
    );
    let refreshed = example.stalwart_token(&max, &id, true).await.unwrap();
    assert_ne!(refreshed["access_token"], token["access_token"]);
    // Someone else's account, and a user without access.
    let eva = hub.user_id("eva").await;
    assert_eq!(
        example
            .stalwart_token(&eva, &id, false)
            .await
            .unwrap_err()
            .status,
        404
    );
    let reply = hub
        .admin(Method::DELETE, &format!("/users/{eva}/apps/example"), None)
        .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    let denied = example.stalwart_accounts(&eva).await.unwrap_err();
    assert_eq!(
        (denied.status, &denied.body["error"]),
        (404, &json!("no_access"))
    );

    // Without `backend` in the manifest no tokens, without `stalwart`
    // nothing at all.
    let mut manifest = roedy_hub_example::manifest();
    manifest["stalwart"] = json!({});
    example.send_manifest_with(&manifest).await.unwrap();
    let refused = example.stalwart_token(&max, &id, false).await.unwrap_err();
    assert_eq!(
        (refused.status, &refused.body["error"]),
        (403, &json!("not_requested"))
    );
    assert!(example.stalwart_accounts(&max).await.is_ok());
    manifest.as_object_mut().unwrap().remove("stalwart");
    example.send_manifest_with(&manifest).await.unwrap();
    assert_eq!(example.stalwart_servers().await.unwrap_err().status, 403);

    // The events: added (with the account), removed; none after the
    // manifest stopped asking.
    let events: Vec<(String, Value)> = hub
        .events()
        .await
        .into_iter()
        .filter(|(_, kind, _)| kind.starts_with("stalwart."))
        .map(|(_, kind, data)| (kind, data))
        .collect();
    assert_eq!(events.len(), 1, "{events:?}");
    assert_eq!(events[0].0, "stalwart.account.added");
    assert_eq!(events[0].1["sub"], json!(max));
    assert_eq!(events[0].1["account"]["id"], json!(id));
    manifest["stalwart"] = json!({ "backend": true });
    example.send_manifest_with(&manifest).await.unwrap();
    hub.max(Method::DELETE, &format!("/stalwart/accounts/{id}"), None)
        .await;
    hub.admin(
        Method::PUT,
        "/admin/stalwart/policy",
        Some(json!({ "allow_custom_servers": true })),
    )
    .await;
    let kinds: Vec<String> = hub
        .events()
        .await
        .into_iter()
        .map(|(_, kind, _)| kind)
        .filter(|kind| kind.starts_with("stalwart."))
        .collect();
    assert_eq!(
        kinds,
        [
            "stalwart.account.added",
            "stalwart.account.removed",
            "stalwart.servers.changed"
        ]
    );
}

#[tokio::test]
async fn needing_a_new_sign_in_and_signing_in_again_reach_the_apps() {
    let hub = hub().await;
    let fake = Server::start(confidential()).await;
    let server = hub.registered(&fake).await;
    let _example = with_example(&hub).await;
    let (reply, _) = sign_in(
        &hub.env,
        &hub.max,
        json!({ "server_id": server, "email": MAIL_USER }),
    )
    .await;
    let id = reply.json()["account"]["id"].as_str().unwrap().to_owned();
    fake.revoke_refresh_tokens();
    let gone = hub
        .max(
            Method::POST,
            &format!("/stalwart/accounts/{id}/token"),
            Some(json!({ "refresh": true })),
        )
        .await;
    assert_eq!(gone.error(), "needs_sign_in");
    let (again, _) = sign_in(&hub.env, &hub.max, json!({ "account_id": id })).await;
    assert_eq!(again.status, StatusCode::OK, "{}", again.body);
    let max = hub.user_id("max").await;
    let events: Vec<(String, Value)> = hub
        .events()
        .await
        .into_iter()
        .filter(|(_, kind, _)| kind.starts_with("stalwart.account."))
        .map(|(_, kind, data)| (kind, data))
        .collect();
    assert_eq!(events[1].0, "stalwart.account.needs_sign_in");
    assert_eq!(events[1].1, json!({ "sub": max, "account_id": id }));
    assert_eq!(events[2].0, "stalwart.account.signed_in");
    assert_eq!(events.len(), 3);
}

#[tokio::test]
async fn jobs_probe_servers_refresh_early_and_check_storage() {
    let hub = hub().await;
    let fake = Server::start(Options {
        expires_in: 300,
        quota: Some((10, 100)),
        ..confidential()
    })
    .await;
    let server = hub.registered(&fake).await;
    // Diagnosis of the servers never tested.
    assert_eq!(server_probe::run(&hub.env.state).await.unwrap(), 1);
    assert_eq!(server_probe::run(&hub.env.state).await.unwrap(), 0);
    let tested = hub
        .admin(
            Method::GET,
            &format!("/admin/stalwart/servers/{server}"),
            None,
        )
        .await;
    assert_eq!(tested.json()["oauth"], "ok", "{}", tested.body);

    let (reply, _) = sign_in(
        &hub.env,
        &hub.max,
        json!({ "server_id": server, "email": MAIL_USER }),
    )
    .await;
    let id = reply.json()["account"]["id"].as_str().unwrap().to_owned();
    // Not active yet: no browser fetched a token.
    assert_eq!(token_refresh::run(&hub.env.state).await.unwrap(), 0);
    hub.max(
        Method::POST,
        &format!("/stalwart/accounts/{id}/token"),
        None,
    )
    .await;
    // Active, and the token lives less than ten minutes: renewed early.
    let before = fake.token_requests().len();
    assert_eq!(token_refresh::run(&hub.env.state).await.unwrap(), 1);
    assert_eq!(fake.token_requests().len(), before + 1);

    assert_eq!(quota_check::run(&hub.env.state).await.unwrap(), 1);
    assert_eq!(quota_check::run(&hub.env.state).await.unwrap(), 0);
    let list = hub.max(Method::GET, "/stalwart/accounts", None).await;
    assert_eq!(list.json()["accounts"][0]["quota"]["used"], 10);
}

#[tokio::test]
async fn the_system_mail_sends_from_a_registry_server() {
    let hub = hub().await;
    let fake = Server::start(Options::default()).await;
    let server = hub.env.add_server("s1", &fake.url).await;
    let reply = hub
        .admin(
            Method::PUT,
            "/admin/system-mail",
            Some(json!({
                "server_id": server, "server_url": fake.url, "user": MAIL_USER,
                "app_password": fake_stalwart::MAIL_PASSWORD,
            })),
        )
        .await;
    assert_eq!(reply.json()["status"], "ok", "{}", reply.body);
    let list = hub
        .admin(Method::GET, "/admin/stalwart/servers", None)
        .await;
    assert_eq!(list.json()["system_mail_server"], json!(server));
    // A new address makes the sending account forget its password.
    hub.admin(
        Method::PATCH,
        &format!("/admin/stalwart/servers/{server}"),
        Some(json!({ "url": "https://mail.example.com" })),
    )
    .await;
    let shown = hub.admin(Method::GET, "/admin/system-mail", None).await;
    assert_eq!(shown.json()["has_secret"], false, "{}", shown.body);
    // A password typed while the form still showed the old address is not
    // stored for the new one.
    let stale = hub
        .admin(
            Method::PUT,
            "/admin/system-mail",
            Some(json!({ "app_password": "new", "server_url": fake.url })),
        )
        .await;
    assert_eq!(
        stale.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "{}",
        stale.body
    );
    assert_eq!(
        stale.json()["fields"]["server_id"],
        json!({ "code": "changed", "url": "https://mail.example.com" })
    );
    let shown = hub.admin(Method::GET, "/admin/system-mail", None).await;
    assert_eq!(shown.json()["has_secret"], false);
}

#[tokio::test]
async fn a_discarded_grant_is_revoked_even_after_its_server_is_removed() {
    let hub = hub().await;
    let fake = Server::start(confidential()).await;
    let id = hub.registered(&fake).await;
    let lookup = id.clone();
    let server = db::call(&hub.env.state.db, move |conn| {
        stalwart_servers::get(conn, &lookup)
    })
    .await
    .unwrap()
    .unwrap();
    // A grant from an exchange the hub then does not keep …
    let base = Url::parse(&fake.url).unwrap();
    let tokens = oauth::request(
        &hub.env.state.stalwart.http,
        &base,
        oauth::Client {
            id: CLIENT_ID,
            secret: Some(CLIENT_SECRET),
        },
        oauth::Grant::Code {
            code: fake_stalwart::CODE,
            verifier: fake_stalwart::VERIFIER,
            redirect_uri: "https://app.example.com/oauth/callback",
        },
    )
    .await
    .unwrap();
    let refresh = tokens.refresh_token.unwrap();
    // … while the administrator removes the server: the secret of the
    // exchange still revokes it.
    let removed = hub
        .admin(
            Method::DELETE,
            &format!("/admin/stalwart/servers/{id}"),
            None,
        )
        .await;
    assert_eq!(removed.status, StatusCode::NO_CONTENT);
    token::revoke_discarded(
        &hub.env.state,
        "someone",
        token::Discarded {
            server,
            client_id: CLIENT_ID.into(),
            client_secret: Some(CLIENT_SECRET.into()),
            refresh_token: refresh.clone(),
        },
    );
    wait_for(|| !fake.grant_exists(&refresh)).await;
    let revocation = &fake.revocations()[0];
    assert!(
        revocation
            .authorization
            .as_deref()
            .unwrap()
            .starts_with("Basic ")
    );
}

/// Waits up to 5 s for `done` (revocations run in their own task).
async fn wait_for(mut done: impl FnMut() -> bool) {
    for _ in 0..500 {
        if done() {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    panic!("timed out");
}
