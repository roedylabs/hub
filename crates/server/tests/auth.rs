//! Sign-in via the API: email or username, authenticator apps with codes
//! valid only once, recovery codes, a fresh sign-in in the same session,
//! sign-out and `GET session`. Taken over from Roedy Mail
//! (`crates/server/tests/auth.rs`), trimmed to the hub.

mod common;

use axum::{
    body::Body,
    http::{Method, Request, StatusCode},
};
use common::{PASSWORD, code, env, wrong_code};
use roedy_hub_server::{
    app::router,
    auth::{challenge, recovery, session},
    db::{self, repo::users::Role},
};
use serde_json::json;
use tower::ServiceExt as _;

#[tokio::test]
async fn login_with_email_or_username() {
    let env = env().await;
    let id = env.add_user("max", "Max@Example.com", Role::User).await;

    for login in ["max@example.com", " MAX ", "Max@Example.com"] {
        let reply = env.login(login, PASSWORD).await;
        assert_eq!(reply.status, StatusCode::OK, "{login}: {}", reply.body);
        assert_eq!(reply.json()["status"], "ok");
        assert_eq!(reply.json()["user"]["id"], id);
        assert_eq!(reply.json()["user"]["username"], "max");
        let cookie = reply
            .set_cookies()
            .into_iter()
            .find(|c| c.starts_with(session::COOKIE))
            .unwrap();
        assert!(
            cookie.ends_with("; Max-Age=2592000; Path=/; HttpOnly; Secure; SameSite=Lax"),
            "{cookie}"
        );
    }
    let actions = env.audit_actions().await;
    assert_eq!(actions, ["auth.login", "auth.login", "auth.login"]);
    let (method, ip): (String, String) = env
        .sql(|c| {
            c.query_row(
                "SELECT details ->> 'method', ip FROM audit_log LIMIT 1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
        })
        .await;
    assert_eq!((method.as_str(), ip.as_str()), ("password", "203.0.113.7"));
}

#[tokio::test]
async fn wrong_password_unknown_and_locked_look_the_same() {
    let env = env().await;
    let id = env.add_user("max", "max@example.com", Role::User).await;
    env.add_user("eva", "eva@example.com", Role::User).await;
    env.sql(|c| {
        c.execute(
            "UPDATE users SET status = 'locked' WHERE username = 'eva'",
            [],
        )
    })
    .await;

    for (login, password) in [
        ("max", "Summer2026?"),
        ("moritz@example.com", PASSWORD),
        ("eva", PASSWORD),
    ] {
        let reply = env.login(login, password).await;
        assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY, "{login}");
        assert_eq!(reply.json(), json!({ "error": "invalid_credentials" }));
        assert!(reply.cookie(session::COOKIE).is_none());
    }
    // Only known accounts in the audit log, never the input.
    env.wait_for_audit("auth.login_failed", 2).await;
    let rows: Vec<(String, String, String)> = env
        .sql(|c| {
            c.prepare("SELECT action, target, details FROM audit_log ORDER BY id")?
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
                .collect()
        })
        .await;
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].1, id);
    assert_eq!(rows[0].2, r#"{"reason":"password"}"#);
    assert_eq!(rows[1].2, r#"{"reason":"locked"}"#);

    let reply = env.login(" ", "").await;
    assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        reply.json()["fields"],
        json!({ "login": { "code": "required" }, "password": { "code": "required" } })
    );
}

#[tokio::test]
async fn wrong_passwords_lock_the_spelling() {
    let env = env().await;
    env.add_user("max", "max@example.com", Role::User).await;
    for _ in 0..10 {
        let reply = env.login("max", "Summer2026?").await;
        assert_eq!(reply.error(), "invalid_credentials");
    }
    let reply = env.login("MAX", PASSWORD).await;
    assert_eq!(reply.status, StatusCode::TOO_MANY_REQUESTS);
    assert!(reply.header("retry-after").is_some());
    // The address is another spelling with its own counter.
    let reply = env.login("max@example.com", PASSWORD).await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
}

#[tokio::test]
async fn remember_me_controls_cookie_lifetime() {
    let env = env().await;
    env.add_user("max", "max@example.com", Role::User).await;
    let reply = env
        .request(
            Method::POST,
            "/auth/login",
            Some(json!({ "login": "max", "password": PASSWORD, "remember": false })),
            &[],
        )
        .await;
    let cookie = reply
        .set_cookies()
        .into_iter()
        .find(|c| c.starts_with(session::COOKIE))
        .unwrap();
    assert!(!cookie.contains("Max-Age"), "{cookie}");
    let token = reply.cookie(session::COOKIE).unwrap();

    // The renewed cookie also stays without a lifetime.
    env.sql(|c| {
        c.execute(
            "UPDATE sessions SET last_seen_at = '2000-01-01T00:00:00.000Z'",
            [],
        )
    })
    .await;
    let me = env.as_user(&token, Method::GET, "/auth/me", None).await;
    assert_eq!(me.status, StatusCode::OK);
    let renewed = me.set_cookies();
    assert_eq!(renewed.len(), 1, "{renewed:?}");
    assert!(!renewed[0].contains("Max-Age"), "{renewed:?}");
}

#[tokio::test]
async fn an_unknown_session_cookie_is_cleared() {
    let env = env().await;
    let reply = env
        .as_user(&"0".repeat(64), Method::GET, "/session", None)
        .await;
    assert_eq!(reply.status, StatusCode::UNAUTHORIZED);
    assert_eq!(reply.json(), json!({ "error": "unauthorized" }));
    assert!(reply.clears(session::COOKIE));
    // Without a cookie nothing to clear.
    let reply = env.request(Method::GET, "/session", None, &[]).await;
    assert_eq!(reply.status, StatusCode::UNAUTHORIZED);
    assert!(reply.set_cookies().is_empty());
}

#[tokio::test]
async fn second_factor_with_codes_valid_once() {
    let env = env().await;
    let id = env.add_user("max", "max@example.com", Role::User).await;
    let secret = env.add_device(&id, "Phone").await;

    let first = env.login("max", PASSWORD).await;
    assert_eq!(first.status, StatusCode::OK);
    assert_eq!(first.json(), json!({ "status": "totp_required" }));
    assert!(first.cookie(session::COOKIE).is_none(), "no session yet");
    let mfa = first.cookie(challenge::COOKIE).unwrap();
    let step = |code: String| {
        let mfa = mfa.clone();
        let env = &env;
        async move {
            env.request(
                Method::POST,
                "/auth/totp",
                Some(json!({ "code": code })),
                &[(challenge::COOKIE, &mfa)],
            )
            .await
        }
    };

    assert_eq!(step("12345".into()).await.error(), "invalid_code");
    assert_eq!(step(wrong_code(&secret)).await.error(), "invalid_code");
    env.wait_for_audit("auth.login_failed", 1).await;
    let current = code(&secret, 0);
    let reply = step(current.clone()).await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    assert_eq!(reply.json()["user"]["id"], id);
    assert!(reply.cookie(session::COOKIE).is_some());
    assert!(reply.clears(challenge::COOKIE));
    // The intermediate step is used up.
    assert_eq!(step(current.clone()).await.error(), "login_expired");

    // The same code cannot sign in a second time.
    let again = env.login_with_code("max", &current).await;
    assert_eq!(again.error(), "code_used", "{}", again.body);
    let method: String = env
        .sql(|c| {
            c.query_row(
                "SELECT details ->> 'method' FROM audit_log WHERE action = 'auth.login'",
                [],
                |r| r.get(0),
            )
        })
        .await;
    assert_eq!(method, "totp");
}

#[tokio::test]
async fn five_wrong_codes_lock_the_second_step() {
    let env = env().await;
    let id = env.add_user("max", "max@example.com", Role::User).await;
    let secret = env.add_device(&id, "Phone").await;
    let wrong = wrong_code(&secret);
    for _ in 0..5 {
        assert_eq!(
            env.login_with_code("max", &wrong).await.error(),
            "invalid_code"
        );
    }
    let reply = env.login_with_code("max", &code(&secret, 0)).await;
    assert_eq!(reply.status, StatusCode::TOO_MANY_REQUESTS);
}

#[tokio::test]
async fn recovery_codes_sign_in_once() {
    let env = env().await;
    let id = env.add_user("max", "max@example.com", Role::User).await;
    env.add_device(&id, "Phone").await;
    let user = id.clone();
    let codes = db::call(&env.state.db, move |conn| recovery::replace(conn, &user))
        .await
        .unwrap();

    let recover = |code: String| {
        let env = &env;
        async move {
            let first = env.login("max", PASSWORD).await;
            let mfa = first.cookie(challenge::COOKIE).unwrap();
            env.request(
                Method::POST,
                "/auth/recovery",
                Some(json!({ "code": code })),
                &[(challenge::COOKIE, &mfa)],
            )
            .await
        }
    };
    let reply = recover(codes[0].to_lowercase()).await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    assert_eq!(recover(codes[0].clone()).await.error(), "code_used");
    assert_eq!(recover("ZZZZ-ZZZZ".into()).await.error(), "invalid_code");
    assert_eq!(recover("short".into()).await.error(), "invalid_code");
    let remaining: String = env
        .sql(|c| {
            c.query_row(
                "SELECT details ->> 'remaining' FROM audit_log WHERE action = 'auth.login'",
                [],
                |r| r.get::<_, i64>(0).map(|n| n.to_string()),
            )
        })
        .await;
    assert_eq!(remaining, "9");
}

#[tokio::test]
async fn second_step_without_or_with_an_outdated_challenge() {
    let env = env().await;
    let id = env.add_user("max", "max@example.com", Role::User).await;
    let secret = env.add_device(&id, "Phone").await;
    let reply = env
        .request(
            Method::POST,
            "/auth/totp",
            Some(json!({ "code": code(&secret, 0) })),
            &[],
        )
        .await;
    assert_eq!(reply.error(), "login_expired");

    // The password changed after the first step.
    let first = env.login("max", PASSWORD).await;
    let mfa = first.cookie(challenge::COOKIE).unwrap();
    env.sql(|c| {
        c.execute(
            "UPDATE users SET password_changed_at = '2099-01-01T00:00:00.000Z'",
            [],
        )
    })
    .await;
    let reply = env
        .request(
            Method::POST,
            "/auth/totp",
            Some(json!({ "code": code(&secret, 0) })),
            &[(challenge::COOKIE, &mfa)],
        )
        .await;
    assert_eq!(reply.error(), "login_expired");
    assert_eq!(env.count("sessions").await, 0);
}

#[tokio::test]
async fn a_fresh_sign_in_keeps_the_session_and_renews_auth_time() {
    let env = env().await;
    let id = env.add_user("max", "max@example.com", Role::User).await;
    let secret = env.add_device(&id, "Phone").await;
    let token = env
        .login_with_code("max", &code(&secret, 0))
        .await
        .cookie(session::COOKIE)
        .unwrap();
    env.sql(|c| {
        c.execute(
            "UPDATE sessions SET auth_time = '2026-01-01T00:00:00.000Z'",
            [],
        )
    })
    .await;
    let before = env.as_user(&token, Method::GET, "/session", None).await;
    assert_eq!(before.json()["session"]["auth_time"], 1_767_225_600);
    let sid = before.json()["session"]["id"].clone();

    // `/login?reauth=1`: password and code again, in the same browser.
    let first = env
        .request(
            Method::POST,
            "/auth/login",
            Some(json!({ "login": "max", "password": PASSWORD })),
            &[(session::COOKIE, &token)],
        )
        .await;
    assert_eq!(first.json()["status"], "totp_required");
    let mfa = first.cookie(challenge::COOKIE).unwrap();
    let reply = env
        .request(
            Method::POST,
            "/auth/totp",
            Some(json!({ "code": code(&secret, 1) })),
            &[(session::COOKIE, &token), (challenge::COOKIE, &mfa)],
        )
        .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    assert!(
        reply.cookie(session::COOKIE).is_none(),
        "the cookie stays the same"
    );
    assert!(reply.clears(challenge::COOKIE));

    let after = env.as_user(&token, Method::GET, "/session", None).await;
    assert_eq!(after.json()["session"]["id"], sid);
    assert!(after.json()["session"]["auth_time"].as_i64().unwrap() > 1_767_225_600);
    assert_eq!(env.count("sessions").await, 1);
    assert_eq!(env.audit_actions().await, ["auth.login", "auth.reauth"]);
}

#[tokio::test]
async fn another_account_ends_the_previous_session() {
    let env = env().await;
    env.add_user("max", "max@example.com", Role::User).await;
    env.add_user("eva", "eva@example.com", Role::User).await;
    let max = env.session_for("max").await;
    let reply = env
        .request(
            Method::POST,
            "/auth/login",
            Some(json!({ "login": "eva", "password": PASSWORD })),
            &[(session::COOKIE, &max)],
        )
        .await;
    assert_eq!(reply.status, StatusCode::OK);
    let eva = reply.cookie(session::COOKIE).unwrap();
    assert_ne!(eva, max);
    assert_eq!(
        env.as_user(&max, Method::GET, "/session", None)
            .await
            .status,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(env.count("sessions").await, 1);
}

#[tokio::test]
async fn session_describes_user_session_and_hub() {
    let env = env().await;
    let id = env.add_user("max", "max@example.com", Role::User).await;
    let token = env.session_for("max").await;
    let reply = env.as_user(&token, Method::GET, "/session", None).await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    assert_eq!(reply.header("cache-control"), Some("no-store"));
    let body = reply.json();
    let sid = body["session"]["id"].as_str().unwrap().to_owned();
    assert_eq!(sid.len(), 26, "ULID");
    assert!(body["session"]["auth_time"].as_i64().unwrap() > 1_700_000_000);
    assert_eq!(
        body,
        json!({
            "user": {
                "id": id, "username": "max", "email": "max@example.com",
                "name": "Max Example", "role": "user", "locale": "en",
            },
            "prefs": {
                "theme": "system", "accent": null, "font_size": "medium", "density": "normal",
                "animations": true, "language": "en", "region": null,
                "time_zone": "Europe/Berlin", "time_zone_auto": true, "date_format": "iso",
                "time_format": "24h", "week_start": "monday", "number_format": "1,234.56",
                "confirm_delete": true, "undo_delay": 5, "shortcuts": true, "developer": false,
            },
            "session": { "id": sid, "auth_time": body["session"]["auth_time"] },
            "hub": { "name": "Roedy Hub", "home": "/", "account": "/account", "admin": null },
        })
    );

    // German browser, settings on `system`: the language of the sign-in.
    let reply = env
        .send(
            common::CLIENT,
            Method::POST,
            "/auth/login",
            Some(json!({ "login": "max", "password": PASSWORD })),
            &[],
            &[("accept-language", "de-DE,de;q=0.9")],
        )
        .await;
    let german = reply.cookie(session::COOKIE).unwrap();
    let settings = env.as_user(&german, Method::GET, "/settings", None).await;
    let reply = env
        .as_user(
            &german,
            Method::PUT,
            "/settings",
            Some(json!({ "version": settings.json()["version"], "settings": { "lang": "system", "mode": "light" } })),
        )
        .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    let reply = env.as_user(&german, Method::GET, "/session", None).await;
    assert_eq!(reply.json()["user"]["locale"], "de");
    assert_eq!(reply.json()["prefs"]["theme"], "light");
    assert_eq!(reply.json()["prefs"]["language"], "system");
    // Formats without a region follow the language of the sign-in.
    assert_eq!(reply.json()["prefs"]["date_format"], "de");
}

#[tokio::test]
async fn logout_ends_the_session_and_clears_site_data() {
    let env = env().await;
    env.add_user("max", "max@example.com", Role::User).await;
    let token = env.session_for("max").await;
    let reply = env
        .as_user(&token, Method::POST, "/auth/logout", None)
        .await;
    assert_eq!(reply.status, StatusCode::NO_CONTENT);
    assert!(reply.clears(session::COOKIE));
    assert_eq!(
        reply.header("clear-site-data"),
        Some(r#""cache", "storage""#)
    );
    assert_eq!(env.count("sessions").await, 0);
    assert_eq!(env.audit_actions().await, ["auth.login", "auth.logout"]);
    assert_eq!(
        env.as_user(&token, Method::GET, "/session", None)
            .await
            .status,
        StatusCode::UNAUTHORIZED
    );
    // Without a session too.
    let reply = env.request(Method::POST, "/auth/logout", None, &[]).await;
    assert_eq!(reply.status, StatusCode::NO_CONTENT);
    assert!(reply.clears(session::COOKIE));
}

#[tokio::test]
async fn sign_in_needs_the_csrf_header() {
    let env = env().await;
    env.add_user("max", "max@example.com", Role::User).await;
    let request = Request::post("/api/v1/auth/login")
        .header("content-type", "application/json")
        .body(Body::from(
            json!({ "login": "max", "password": PASSWORD }).to_string(),
        ))
        .unwrap();
    let response = router(env.state.clone()).oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert_eq!(env.count("sessions").await, 0);
}
