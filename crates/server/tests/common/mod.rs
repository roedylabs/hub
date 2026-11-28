//! Test environment for the API: a fresh database in a temporary directory,
//! fast password hashing, requests through the router without network.
//! Taken over from Roedy Mail's API tests (`crates/server/tests/auth.rs`).

#![allow(dead_code, reason = "each test file uses its own part")]

use std::{net::SocketAddr, sync::Arc, time::Duration};

use axum::{
    Router,
    body::Body,
    extract::connect_info::MockConnectInfo,
    http::{HeaderMap, Method, Request, StatusCode, header},
};
use http_body_util::BodyExt as _;
use roedy_hub_server::{
    api,
    app::{AppState, router},
    auth::{
        challenge,
        password::{Hasher, Params},
        session, totp,
    },
    config::Config,
    crypto::SecretKey,
    db::{
        self, Db,
        repo::{
            stalwart_servers, totp_devices,
            users::{self, NewUser, Role},
        },
    },
    mailer::system_mail::{self, SystemMail},
    web::static_files::WebAssets,
};
use serde_json::{Value, json};
use tower::ServiceExt as _;
use url::Url;

pub const PASSWORD: &str = "Summer2026!";
pub const CLIENT: [u8; 4] = [203, 0, 113, 7];

pub struct Env {
    _tmp: tempfile::TempDir,
    pub state: AppState,
}

pub fn fast_hasher() -> Arc<Hasher> {
    Arc::new(Hasher::with_params(Params::new(64, 1, 1, None).unwrap()))
}

pub async fn env() -> Env {
    let tmp = tempfile::tempdir().unwrap();
    let conn = db::open(tmp.path()).unwrap();
    let mut state = AppState::new(
        Db::from(conn),
        Arc::new(WebAssets::None),
        &Config::default(),
        Url::parse("https://app.example.com").unwrap(),
        SecretKey::random().unwrap(),
    )
    .unwrap();
    state.hasher = fast_hasher();
    Env { _tmp: tmp, state }
}

impl Env {
    pub async fn add_user(&self, username: &str, email: &str, role: Role) -> String {
        let hash = self.state.hasher.hash(PASSWORD).await.unwrap();
        let (username, email) = (username.to_owned(), email.to_owned());
        db::call(&self.state.db, move |conn| {
            users::insert(
                conn,
                &NewUser {
                    username: &username,
                    email: &email,
                    display_name: "Max Example",
                    role,
                    password_hash: &hash,
                },
            )
        })
        .await
        .unwrap()
    }

    /// Confirmed authenticator app with a new secret (without an accepted
    /// time step, so that every current code is fresh).
    pub async fn add_device(&self, user_id: &str, name: &str) -> Vec<u8> {
        let id = db::new_id().unwrap();
        let secret = totp::generate_secret().unwrap().to_vec();
        let sealed = self
            .state
            .secrets
            .seal(totp::SECRET_PURPOSE, &id, &secret)
            .unwrap();
        let (user_id, name) = (user_id.to_owned(), name.to_owned());
        db::call(&self.state.db, move |conn| {
            totp_devices::insert_pending(conn, &id, &user_id, &sealed)?;
            totp_devices::confirm(conn, &id, &name, 0)?;
            Ok(())
        })
        .await
        .unwrap();
        secret
    }

    /// Stores a sending account on the JMAP server `url` with the fake's
    /// credentials (`fake_stalwart::MAIL_USER`, `MAIL_PASSWORD`); the server goes
    /// into the Stalwart registry first (ID `system`) unless it is there.
    pub async fn configure_mail(&self, url: &str, user: &str, password: &str) {
        let server_id = self.add_server("system", url).await;
        let mail = SystemMail::new(&self.state.secrets, &server_id, user, password, None).unwrap();
        db::call(&self.state.db, move |conn| system_mail::save(conn, &mail))
            .await
            .unwrap();
    }

    /// A Stalwart server in the registry with the origin of `url`, without an
    /// OAuth client; returns its ID (an existing one with the origin keeps
    /// its own).
    pub async fn add_server(&self, id: &str, url: &str) -> String {
        let origin = Url::parse(url).unwrap().origin().ascii_serialization();
        let id = id.to_owned();
        db::call(&self.state.db, move |conn| {
            if let Some(server) = stalwart_servers::find_by_base_url(conn, &origin)? {
                return Ok(server.id);
            }
            stalwart_servers::insert(
                conn,
                &id,
                &stalwart_servers::Fields {
                    label: "Stalwart",
                    base_url: &origin,
                    domains: &[],
                    client_id: None,
                },
                None,
            )?;
            Ok(id)
        })
        .await
        .unwrap()
    }

    pub async fn sql<T: Send + 'static>(
        &self,
        f: impl FnOnce(&rusqlite::Connection) -> rusqlite::Result<T> + Send + 'static,
    ) -> T {
        db::call(&self.state.db, move |conn| Ok(f(conn)?))
            .await
            .unwrap()
    }

    pub async fn count(&self, table: &'static str) -> i64 {
        self.sql(move |c| c.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0)))
            .await
    }

    /// Waits until `n` entries `action` are in the audit log – the API writes
    /// failed sign-ins in the background.
    pub async fn wait_for_audit(&self, action: &'static str, n: i64) {
        for _ in 0..200 {
            let count: i64 = self
                .sql(move |c| {
                    c.query_row(
                        "SELECT COUNT(*) FROM audit_log WHERE action = ?1",
                        [action],
                        |r| r.get(0),
                    )
                })
                .await;
            if count >= n {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("{action} not {n} times in the audit log");
    }

    pub async fn audit_actions(&self) -> Vec<String> {
        self.sql(|conn| {
            conn.prepare("SELECT action FROM audit_log ORDER BY id")?
                .query_map([], |r| r.get(0))?
                .collect()
        })
        .await
    }

    pub async fn request(
        &self,
        method: Method,
        path: &str,
        body: Option<Value>,
        cookies: &[(&str, &str)],
    ) -> Reply {
        self.send(CLIENT, method, path, body, cookies, &[]).await
    }

    /// A request from `ip` with further `headers`.
    pub async fn send(
        &self,
        ip: [u8; 4],
        method: Method,
        path: &str,
        body: Option<Value>,
        cookies: &[(&str, &str)],
        headers: &[(&str, &str)],
    ) -> Reply {
        let mut builder = Request::builder()
            .method(method)
            .uri(format!("/api/v1{path}"))
            .header(api::CSRF_HEADER, "fetch")
            .header(
                header::USER_AGENT,
                "Mozilla/5.0 (X11; Linux x86_64) Firefox/140.0",
            );
        for (name, value) in headers {
            builder = builder.header(*name, *value);
        }
        if !cookies.is_empty() {
            let cookie: Vec<String> = cookies.iter().map(|(k, v)| format!("{k}={v}")).collect();
            builder = builder.header(header::COOKIE, cookie.join("; "));
        }
        let request = match body {
            Some(body) => builder
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(body.to_string())),
            None => builder.body(Body::empty()),
        }
        .unwrap();
        let app: Router =
            router(self.state.clone()).layer(MockConnectInfo(SocketAddr::from((ip, 4711))));
        let response = app.oneshot(request).await.unwrap();
        let status = response.status();
        let headers = response.headers().clone();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        Reply {
            status,
            headers,
            body: String::from_utf8(bytes.to_vec()).unwrap(),
        }
    }

    /// `GET` of a file outside `/api` (branding, web app) without sign-in:
    /// status, headers and body.
    pub async fn file(&self, path: &str) -> (StatusCode, HeaderMap, Vec<u8>) {
        let request = Request::builder().uri(path).body(Body::empty()).unwrap();
        let app: Router =
            router(self.state.clone()).layer(MockConnectInfo(SocketAddr::from((CLIENT, 4711))));
        let response = app.oneshot(request).await.unwrap();
        let (status, headers) = (response.status(), response.headers().clone());
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        (status, headers, bytes.to_vec())
    }

    /// A modifying request without `X-Requested-With`, as a foreign form
    /// would send it.
    pub async fn without_csrf(
        &self,
        method: Method,
        path: &str,
        body: Value,
        cookies: &[(&str, &str)],
    ) -> Reply {
        let mut builder = Request::builder()
            .method(method)
            .uri(format!("/api/v1{path}"))
            .header(header::CONTENT_TYPE, "application/json");
        if !cookies.is_empty() {
            let cookie: Vec<String> = cookies.iter().map(|(k, v)| format!("{k}={v}")).collect();
            builder = builder.header(header::COOKIE, cookie.join("; "));
        }
        let request = builder.body(Body::from(body.to_string())).unwrap();
        let app: Router =
            router(self.state.clone()).layer(MockConnectInfo(SocketAddr::from((CLIENT, 4711))));
        let response = app.oneshot(request).await.unwrap();
        let status = response.status();
        let headers = response.headers().clone();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        Reply {
            status,
            headers,
            body: String::from_utf8(bytes.to_vec()).unwrap(),
        }
    }

    pub async fn login(&self, login: &str, password: &str) -> Reply {
        self.request(
            Method::POST,
            "/auth/login",
            Some(json!({ "login": login, "password": password })),
            &[],
        )
        .await
    }

    /// Sign-in with password and code from an authenticator app.
    pub async fn login_with_code(&self, login: &str, code: &str) -> Reply {
        let first = self.login(login, PASSWORD).await;
        assert_eq!(first.json()["status"], "totp_required", "{}", first.body);
        let mfa = first.cookie(challenge::COOKIE).unwrap();
        self.request(
            Method::POST,
            "/auth/totp",
            Some(json!({ "code": code })),
            &[(challenge::COOKIE, &mfa)],
        )
        .await
    }

    /// The session cookie of a sign-in with the password alone.
    pub async fn session_for(&self, login: &str) -> String {
        let reply = self.login(login, PASSWORD).await;
        assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
        reply.cookie(session::COOKIE).unwrap()
    }

    pub async fn as_user(
        &self,
        session: &str,
        method: Method,
        path: &str,
        body: Option<Value>,
    ) -> Reply {
        self.request(method, path, body, &[(session::COOKIE, session)])
            .await
    }
}

pub struct Reply {
    pub status: StatusCode,
    pub headers: HeaderMap,
    pub body: String,
}

impl Reply {
    pub fn json(&self) -> Value {
        serde_json::from_str(&self.body).unwrap_or_else(|_| panic!("not JSON: {}", self.body))
    }

    pub fn error(&self) -> String {
        self.json()["error"].as_str().unwrap_or_default().to_owned()
    }

    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(name).map(|v| v.to_str().unwrap())
    }

    pub fn set_cookies(&self) -> Vec<String> {
        self.headers
            .get_all(header::SET_COOKIE)
            .iter()
            .map(|v| v.to_str().unwrap().to_owned())
            .collect()
    }

    /// Value of a set (not deleted) cookie.
    pub fn cookie(&self, name: &str) -> Option<String> {
        self.set_cookies().iter().find_map(|c| {
            let value = c.strip_prefix(&format!("{name}="))?.split(';').next()?;
            (!value.is_empty()).then(|| value.to_owned())
        })
    }

    /// Whether the response deletes the cookie `name`.
    pub fn clears(&self, name: &str) -> bool {
        self.set_cookies()
            .iter()
            .any(|c| c.starts_with(&format!("{name}=;")) && c.contains("Max-Age=0"))
    }
}

/// The app's code for the time step `offset` relative to now (only 0 and 1:
/// if the test crosses a step boundary, both stay valid).
pub fn code(secret: &[u8], offset: u64) -> String {
    totp_rs::Builder::new()
        .with_secret(secret.to_vec())
        .build()
        .unwrap()
        .generate(totp::now() + offset * 30)
        .to_string()
}

/// A code that matches none of the valid time steps.
pub fn wrong_code(secret: &[u8]) -> String {
    let valid: Vec<String> = (0..3)
        .map(|step| {
            totp_rs::Builder::new()
                .with_secret(secret.to_vec())
                .build()
                .unwrap()
                .generate((totp::now() / 30 + step).saturating_sub(1) * 30)
                .to_string()
        })
        .collect();
    (0..1_000_000)
        .map(|n| format!("{n:06}"))
        .find(|c| !valid.contains(c))
        .unwrap()
}
