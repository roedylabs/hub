//! The own account via the API: profile, password, authenticator apps and
//! recovery codes, signed-in devices, settings and deleting the account.
//! Taken over from Roedy Mail (`crates/server/tests/account.rs`,
//! `sessions.rs`, `settings.rs`), trimmed to the hub.

mod common;

use axum::http::{Method, StatusCode};
use common::{PASSWORD, code, env, wrong_code};
use roedy_hub_server::{auth::session, db::repo::users::Role};
use serde_json::{Value, json};

#[tokio::test]
async fn me_shows_the_account() {
    let env = env().await;
    let id = env.add_user("max", "max@example.com", Role::Admin).await;
    let token = env.session_for("max").await;
    let reply = env.as_user(&token, Method::GET, "/auth/me", None).await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    let me = reply.json();
    assert_eq!(me["id"], id);
    assert_eq!(me["role"], "admin");
    assert_eq!(me["mfa_enabled"], false);
    assert_eq!(me["sole_admin"], true);
    assert!(me["password_changed_at"].as_str().unwrap().ends_with('Z'));
    assert_eq!(
        env.request(Method::GET, "/auth/me", None, &[]).await.status,
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn profile_changes_name_and_username() {
    let env = env().await;
    env.add_user("max", "max@example.com", Role::User).await;
    env.add_user("eva", "eva@example.com", Role::User).await;
    let token = env.session_for("max").await;
    let patch = |body: Value| env.as_user(&token, Method::PATCH, "/auth/me", Some(body));

    let reply = patch(json!({ "display_name": " Max Miller ", "username": "maxi" })).await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    assert_eq!(reply.json()["user"]["display_name"], "Max Miller");
    assert_eq!(reply.json()["user"]["username"], "maxi");
    let me = env.as_user(&token, Method::GET, "/session", None).await;
    assert_eq!(me.json()["user"]["name"], "Max Miller");

    let reply = patch(json!({ "username": "eva" })).await;
    assert_eq!(
        reply.json(),
        json!({ "error": "invalid", "fields": { "username": { "code": "taken" } } })
    );
    let reply = patch(json!({ "display_name": "Max 2", "username": "Max" })).await;
    assert_eq!(
        reply.json()["fields"],
        json!({ "display_name": { "code": "invalid" }, "username": { "code": "invalid" } })
    );
    // A new email address waits for the password (second step, `email.rs`).
    let reply = patch(json!({ "email": "new@example.com" })).await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(reply.json()["status"], "reauth_required");
    let me = env.as_user(&token, Method::GET, "/auth/me", None).await;
    assert_eq!(me.json()["email"], "max@example.com");
    assert_eq!(me.json()["email_change"], Value::Null);
    // Nothing changed: no audit entry.
    let reply = patch(json!({ "display_name": "Max Miller" })).await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(
        env.audit_actions().await,
        ["auth.login", "auth.profile_change"]
    );
}

#[tokio::test]
async fn changing_the_password_ends_the_other_sessions() {
    let env = env().await;
    env.add_user("max", "max@example.com", Role::User).await;
    let token = env.session_for("max").await;
    let other = env.session_for("max").await;
    let change = |current: &str, new: &str| {
        env.as_user(
            &token,
            Method::PUT,
            "/auth/password",
            Some(json!({ "current_password": current, "new_password": new })),
        )
    };

    let reply = change(PASSWORD, "short").await;
    assert_eq!(reply.json()["fields"]["new_password"]["code"], "policy");
    let reply = change("Summer2026?", "Winter2026!").await;
    assert_eq!(reply.error(), "invalid_password");
    let reply = change(PASSWORD, "Winter2026!").await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    assert_eq!(reply.json()["sessions_ended"], 1);

    assert_eq!(
        env.as_user(&other, Method::GET, "/session", None)
            .await
            .status,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        env.as_user(&token, Method::GET, "/session", None)
            .await
            .status,
        StatusCode::OK
    );
    assert_eq!(
        env.login("max", PASSWORD).await.error(),
        "invalid_credentials"
    );
    assert_eq!(env.login("max", "Winter2026!").await.status, StatusCode::OK);
}

#[tokio::test]
async fn changing_the_password_needs_the_code_with_an_app() {
    let env = env().await;
    let id = env.add_user("max", "max@example.com", Role::User).await;
    let secret = env.add_device(&id, "Phone").await;
    let token = env
        .login_with_code("max", &code(&secret, 0))
        .await
        .cookie(session::COOKIE)
        .unwrap();
    let change = |code: Option<String>| {
        env.as_user(
            &token,
            Method::PUT,
            "/auth/password",
            Some(json!({
                "current_password": PASSWORD, "new_password": "Winter2026!", "code": code,
            })),
        )
    };
    assert_eq!(
        change(None).await.json()["fields"],
        json!({ "code": { "code": "required" } })
    );
    assert_eq!(
        change(Some(wrong_code(&secret))).await.error(),
        "invalid_code"
    );
    let reply = change(Some(code(&secret, 1))).await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
}

#[tokio::test]
async fn wrong_passwords_in_the_session_lock_after_five() {
    let env = env().await;
    env.add_user("max", "max@example.com", Role::User).await;
    let token = env.session_for("max").await;
    for _ in 0..5 {
        let reply = env
            .as_user(
                &token,
                Method::PUT,
                "/auth/password",
                Some(json!({ "current_password": "Summer2026?", "new_password": "Winter2026!" })),
            )
            .await;
        assert_eq!(reply.error(), "invalid_password");
    }
    let reply = env
        .as_user(
            &token,
            Method::PUT,
            "/auth/password",
            Some(json!({ "current_password": PASSWORD, "new_password": "Winter2026!" })),
        )
        .await;
    assert_eq!(reply.status, StatusCode::TOO_MANY_REQUESTS);
}

#[tokio::test]
async fn adding_and_removing_authenticator_apps() {
    let env = env().await;
    env.add_user("max", "max@example.com", Role::User).await;
    let token = env.session_for("max").await;

    let enroll = env
        .as_user(&token, Method::POST, "/auth/totp/devices/enroll", None)
        .await;
    assert_eq!(enroll.status, StatusCode::OK, "{}", enroll.body);
    let id = enroll.json()["id"].as_str().unwrap().to_owned();
    assert!(
        enroll.json()["uri"]
            .as_str()
            .unwrap()
            .starts_with("otpauth://totp/Roedy%20Hub:max%40example.com?")
    );
    let secret = totp_rs::Secret::try_from_base32(enroll.json()["secret"].as_str().unwrap())
        .unwrap()
        .to_vec();
    let confirm = |code: String| {
        env.as_user(
            &token,
            Method::POST,
            "/auth/totp/devices/confirm",
            Some(json!({ "id": id, "name": " Phone ", "password": PASSWORD, "code": code })),
        )
    };
    assert_eq!(confirm(wrong_code(&secret)).await.error(), "invalid_code");
    let first = code(&secret, 0);
    let reply = confirm(first.clone()).await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    let body = reply.json();
    assert_eq!(body["devices"][0]["name"], "Phone");
    assert_eq!(body["recovery"]["unused"], 10);
    assert_eq!(body["recovery_codes"].as_array().unwrap().len(), 10);
    let device = body["devices"][0]["id"].as_str().unwrap().to_owned();
    let me = env.as_user(&token, Method::GET, "/auth/me", None).await;
    assert_eq!(me.json()["mfa_enabled"], true);

    // A second app: no new recovery codes; the default name is numbered.
    let enroll = env
        .as_user(&token, Method::POST, "/auth/totp/devices/enroll", None)
        .await;
    let second_id = enroll.json()["id"].as_str().unwrap().to_owned();
    let second = totp_rs::Secret::try_from_base32(enroll.json()["secret"].as_str().unwrap())
        .unwrap()
        .to_vec();
    let reply = env
        .as_user(
            &token,
            Method::POST,
            "/auth/totp/devices/confirm",
            Some(json!({ "id": second_id, "password": PASSWORD, "code": code(&second, 0) })),
        )
        .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    assert_eq!(reply.json()["recovery_codes"], Value::Null);
    assert_eq!(reply.json()["devices"][1]["name"], "Authenticator app 2");

    // New codes need password and a code of one of the apps.
    let reply = env
        .as_user(
            &token,
            Method::POST,
            "/auth/recovery-codes",
            Some(json!({ "password": PASSWORD, "code": code(&second, 1) })),
        )
        .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    assert_eq!(reply.json()["recovery_codes"].as_array().unwrap().len(), 10);

    // Removing both: the last one takes the recovery codes along.
    let remove = async |device: &str, code: String| {
        env.as_user(
            &token,
            Method::DELETE,
            &format!("/auth/totp/devices/{device}"),
            Some(json!({ "password": PASSWORD, "code": code })),
        )
        .await
    };
    let reply = remove(&device, code(&secret, 1)).await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    assert_eq!(reply.json()["devices"].as_array().unwrap().len(), 1);
    assert_eq!(
        remove(&device, code(&second, 1)).await.status,
        StatusCode::NOT_FOUND
    );
    // The second app's code of step +1 is used up; take a fresh device code.
    let reply = remove(&second_id, wrong_code(&second)).await;
    assert_eq!(reply.error(), "invalid_code");
    env.sql(|c| c.execute("UPDATE totp_devices SET last_step = NULL", []))
        .await;
    let reply = remove(&second_id, code(&second, 0)).await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    assert_eq!(reply.json(), json!({ "devices": [], "recovery": null }));
    let reply = env
        .as_user(
            &token,
            Method::POST,
            "/auth/recovery-codes",
            Some(json!({ "password": PASSWORD, "code": "123456" })),
        )
        .await;
    assert_eq!(reply.error(), "mfa_disabled");
    assert_eq!(
        env.audit_actions().await,
        [
            "auth.login",
            "auth.totp_add",
            "auth.totp_add",
            "auth.recovery_codes",
            "auth.totp_remove",
            "auth.totp_remove",
        ]
    );
}

#[tokio::test]
async fn signed_in_devices_and_signing_them_out() {
    let env = env().await;
    env.add_user("max", "max@example.com", Role::User).await;
    let token = env.session_for("max").await;
    let other = env.session_for("max").await;
    let third = env.session_for("max").await;

    let reply = env
        .as_user(&token, Method::GET, "/auth/sessions", None)
        .await;
    let sessions = reply.json()["sessions"].as_array().unwrap().clone();
    assert_eq!(sessions.len(), 3);
    assert_eq!(sessions[0]["current"], true, "the own one first");
    assert_eq!(sessions[0]["device"], "Firefox on Linux");
    assert_eq!(sessions[0]["device_type"], "desktop");
    assert_eq!(sessions[0]["ip"], "203.0.x.x");
    let other_id = sessions[1]["id"].as_str().unwrap().to_owned();

    let reply = env
        .as_user(
            &token,
            Method::DELETE,
            &format!("/auth/sessions/{other_id}"),
            None,
        )
        .await;
    assert_eq!(reply.status, StatusCode::NO_CONTENT);
    assert!(!reply.clears(session::COOKIE));
    let reply = env
        .as_user(
            &token,
            Method::DELETE,
            &format!("/auth/sessions/{other_id}"),
            None,
        )
        .await;
    assert_eq!(reply.status, StatusCode::NOT_FOUND);

    let reply = env
        .as_user(&token, Method::POST, "/auth/sessions/revoke-others", None)
        .await;
    assert_eq!(reply.json(), json!({ "sessions_ended": 1 }));
    for gone in [&other, &third] {
        assert_eq!(
            env.as_user(gone, Method::GET, "/session", None)
                .await
                .status,
            StatusCode::UNAUTHORIZED
        );
    }

    // The own session signs out like `auth/logout`.
    let own = env.as_user(&token, Method::GET, "/session", None).await;
    let own_id = own.json()["session"]["id"].as_str().unwrap().to_owned();
    let reply = env
        .as_user(
            &token,
            Method::DELETE,
            &format!("/auth/sessions/{own_id}"),
            None,
        )
        .await;
    assert_eq!(reply.status, StatusCode::NO_CONTENT);
    assert!(reply.clears(session::COOKIE));
    assert_eq!(env.count("sessions").await, 0);
}

#[tokio::test]
async fn settings_with_versions() {
    let env = env().await;
    env.add_user("max", "max@example.com", Role::User).await;
    let token = env.session_for("max").await;
    let reply = env.as_user(&token, Method::GET, "/settings", None).await;
    assert_eq!(
        reply.json(),
        json!({ "version": 0, "settings": { "lang": "en", "mode": null, "accent": null } })
    );
    let put = |body: Value| env.as_user(&token, Method::PUT, "/settings", Some(body));

    let reply =
        put(json!({ "version": 0, "settings": { "mode": "dark", "accent": "#F80" } })).await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    assert_eq!(
        reply.json(),
        json!({ "version": 1, "settings": { "lang": "en", "mode": "dark", "accent": "#ff8800" } })
    );
    // Nothing changes: the version stays.
    let reply = put(json!({ "version": 1, "settings": { "mode": "dark" } })).await;
    assert_eq!(reply.json()["version"], 1);

    let reply = put(json!({ "version": 0, "settings": { "lang": "de" } })).await;
    assert_eq!(reply.status, StatusCode::CONFLICT);
    assert_eq!(reply.error(), "version_conflict");
    assert_eq!(reply.json()["version"], 1);
    assert_eq!(reply.json()["settings"]["mode"], "dark");

    let reply = put(json!({ "version": 1, "settings": { "lang": "fr", "font": "large" } })).await;
    assert_eq!(
        reply.json()["fields"],
        json!({ "settings.lang": { "code": "invalid" }, "settings.font": { "code": "unknown" } })
    );
    let reply = env.as_user(&token, Method::GET, "/settings", None).await;
    assert_eq!(reply.json()["settings"]["lang"], "en", "nothing changed");
}

#[tokio::test]
async fn deleting_the_account() {
    let env = env().await;
    env.add_user("anna", "anna@example.com", Role::Admin).await;
    env.add_user("max", "max@example.com", Role::User).await;
    let anna = env.session_for("anna").await;
    let max = env.session_for("max").await;
    let delete = async |token: &str, word: &str| {
        env.as_user(
            token,
            Method::DELETE,
            "/auth/me",
            Some(json!({ "confirm": word })),
        )
        .await
    };

    assert_eq!(
        delete(&max, "löschen").await.json()["fields"],
        json!({ "confirm": { "code": "invalid" } })
    );
    let reply = delete(&anna, "delete").await;
    assert_eq!(reply.status, StatusCode::CONFLICT);
    assert_eq!(reply.error(), "last_admin");

    // The confirmation named another account (another tab signed in someone else meanwhile): nothing happens.
    let anna_id: String = env
        .sql(|c| {
            c.query_row("SELECT id FROM users WHERE username = 'anna'", [], |r| {
                r.get(0)
            })
        })
        .await;
    let max_id: String = env
        .sql(|c| {
            c.query_row("SELECT id FROM users WHERE username = 'max'", [], |r| {
                r.get(0)
            })
        })
        .await;
    let named = async |token: &str, id: &str| {
        env.as_user(
            token,
            Method::DELETE,
            "/auth/me",
            Some(json!({ "confirm": "delete", "id": id })),
        )
        .await
    };
    let reply = named(&max, &anna_id).await;
    assert_eq!(reply.status, StatusCode::CONFLICT);
    assert_eq!(reply.error(), "account_mismatch");
    assert_eq!(env.count("users").await, 2);

    let reply = named(&max, &max_id).await;
    assert_eq!(reply.status, StatusCode::NO_CONTENT);
    assert!(reply.clears(session::COOKIE));
    assert_eq!(env.count("users").await, 1);
    assert_eq!(
        env.as_user(&max, Method::GET, "/session", None)
            .await
            .status,
        StatusCode::UNAUTHORIZED
    );
    let (name, actor): (String, Option<String>) = env
        .sql(|c| {
            c.query_row(
                "SELECT actor_name, actor_id FROM audit_log WHERE action = 'users.delete_self'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
        })
        .await;
    assert_eq!((name.as_str(), actor), ("Max Example", None));
}
