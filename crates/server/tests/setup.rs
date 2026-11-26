//! Initial setup via the API: setup token with lockout, the administrator's
//! authenticator app with recovery codes and the completion that creates and
//! signs in the administrator. Taken over from Roedy Mail
//! (`crates/server/tests/setup.rs`), trimmed to the hub's steps.

mod common;

use axum::http::{Method, StatusCode};
use common::{CLIENT, Env, PASSWORD, Reply, code, env, wrong_code};
use roedy_hub_server::{
    api::setup::TOKEN_HEADER,
    auth::{challenge, session},
    db, setup_token,
};
use serde_json::{Value, json};

/// The administrator's address in [`complete_body`].
const EMAIL: &str = "Max@Example.com";

struct Setup {
    env: Env,
    token: String,
}

async fn setup() -> Setup {
    let env = env().await;
    let token = db::call(&env.state.db, |conn| setup_token::ensure(conn))
        .await
        .unwrap()
        .unwrap();
    Setup { env, token }
}

impl Setup {
    async fn post_from(&self, ip: [u8; 4], path: &str, token: Option<&str>, body: Value) -> Reply {
        let headers: Vec<(&str, &str)> = token.map(|t| (TOKEN_HEADER, t)).into_iter().collect();
        self.env
            .send(ip, Method::POST, path, Some(body), &[], &headers)
            .await
    }

    async fn post(&self, path: &str, body: Value) -> Reply {
        self.post_from(CLIENT, path, Some(&self.token), body).await
    }

    /// `setup/totp` for `email`: the secret from the answer.
    async fn enroll(&self, email: &str) -> Vec<u8> {
        let reply = self.post("/setup/totp", json!({ "email": email })).await;
        assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
        totp_rs::Secret::try_from_base32(reply.json()["secret"].as_str().unwrap())
            .unwrap()
            .to_vec()
    }

    async fn verify(&self, code: &str) -> Reply {
        self.post("/setup/totp/verify", json!({ "code": code }))
            .await
    }

    /// The administrator's authenticator app, set up and confirmed: secret and
    /// recovery codes.
    /// The code used for the confirmation is the third part.
    async fn confirmed(&self) -> (Vec<u8>, Vec<String>, String) {
        let secret = self.enroll(EMAIL).await;
        let used = code(&secret, 0);
        let reply = self.verify(&used).await;
        assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
        let codes = serde_json::from_value(reply.json()["recovery_codes"].clone()).unwrap();
        (secret, codes, used)
    }

    async fn complete(&self) -> Reply {
        self.post("/setup/complete", complete_body()).await
    }
}

fn complete_body() -> Value {
    json!({
        "name": "Max Example",
        "email": EMAIL,
        "password": PASSWORD,
        "settings": { "lang": "de", "mode": "dark", "accent": "sage" },
    })
}

#[tokio::test]
async fn token_locks_after_five_failures_per_client() {
    let s = setup().await;
    for remaining in (1..5).rev() {
        let reply = s
            .post_from(CLIENT, "/setup/token", None, json!({ "token": "WRONG" }))
            .await;
        assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(
            reply.json(),
            json!({ "error": "invalid_token", "remaining": remaining })
        );
    }
    let reply = s
        .post_from(CLIENT, "/setup/token", None, json!({ "token": "WRONG" }))
        .await;
    assert_eq!(reply.status, StatusCode::TOO_MANY_REQUESTS);
    // Now even the right token waits – in every step.
    let reply = s
        .post_from(CLIENT, "/setup/token", None, json!({ "token": s.token }))
        .await;
    assert_eq!(reply.status, StatusCode::TOO_MANY_REQUESTS);
    let reply = s
        .post_from(
            CLIENT,
            "/setup/totp",
            Some(&s.token),
            json!({ "email": EMAIL }),
        )
        .await;
    assert_eq!(reply.status, StatusCode::TOO_MANY_REQUESTS);
    // Another client is not affected.
    let reply = s
        .post_from(
            [198, 51, 100, 1],
            "/setup/token",
            None,
            json!({ "token": s.token.to_lowercase() }),
        )
        .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    assert_eq!(reply.json(), json!({}));
}

#[tokio::test]
async fn correct_tokens_do_not_count() {
    let s = setup().await;
    for _ in 0..10 {
        let reply = s
            .post_from(CLIENT, "/setup/token", None, json!({ "token": s.token }))
            .await;
        assert_eq!(reply.status, StatusCode::OK);
    }
    let reply = s
        .post_from(CLIENT, "/setup/token", None, json!({ "token": "WRONG" }))
        .await;
    assert_eq!(reply.json()["remaining"], 4);
}

#[tokio::test]
async fn the_steps_need_the_token() {
    let s = setup().await;
    for (path, body) in [
        ("/setup/totp", json!({ "email": EMAIL })),
        ("/setup/totp/verify", json!({ "code": "123456" })),
        ("/setup/complete", complete_body()),
    ] {
        let reply = s.post_from(CLIENT, path, None, body.clone()).await;
        assert_eq!(reply.status, StatusCode::BAD_REQUEST, "{path}");
        let reply = s.post_from(CLIENT, path, Some("WRONG"), body).await;
        assert_eq!(reply.error(), "invalid_token", "{path}");
    }
    assert_eq!(s.env.count("users").await, 0);
}

#[tokio::test]
async fn enrollment_answers_secret_and_uri() {
    let s = setup().await;
    let reply = s
        .post("/setup/totp", json!({ "email": " max@example.com " }))
        .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    let uri = reply.json()["uri"].as_str().unwrap().to_owned();
    assert!(
        uri.starts_with("otpauth://totp/Roedy%20Hub:max%40example.com?"),
        "{uri}"
    );
    assert!(uri.contains("issuer=Roedy%20Hub"), "{uri}");
    let secret = reply.json()["secret"].as_str().unwrap().to_owned();
    assert_eq!(secret.len(), 32, "160 bits in Base32");
    assert!(uri.contains(&format!("secret={secret}")), "{uri}");

    let reply = s
        .post("/setup/totp", json!({ "email": "max@example" }))
        .await;
    assert_eq!(
        reply.json(),
        json!({ "error": "invalid", "fields": { "email": { "code": "invalid" } } })
    );
}

#[tokio::test]
async fn verification_confirms_each_time_step_once_and_answers_the_codes() {
    let s = setup().await;
    let reply = s.verify("123456").await;
    assert_eq!(reply.error(), "totp_required", "no enrollment yet");

    let secret = s.enroll(EMAIL).await;
    assert_eq!(s.verify("12345").await.error(), "invalid_code");
    let current = code(&secret, 0);
    let first = s.verify(&current).await;
    assert_eq!(first.status, StatusCode::OK, "{}", first.body);
    let codes = first.json()["recovery_codes"].as_array().unwrap().clone();
    assert_eq!(codes.len(), 10);
    assert_eq!(s.verify(&current).await.error(), "code_used");
    // The next step confirms again, with the same codes.
    let again = s.verify(&code(&secret, 1)).await;
    assert_eq!(again.status, StatusCode::OK, "{}", again.body);
    assert_eq!(again.json()["recovery_codes"].as_array().unwrap(), &codes);

    // A new enrollment needs its own confirmation.
    let other = s.enroll(EMAIL).await;
    assert_eq!(s.verify(&code(&secret, 1)).await.error(), "invalid_code");
    assert_eq!(s.verify(&code(&other, 0)).await.status, StatusCode::OK);
}

#[tokio::test]
async fn five_wrong_codes_lock_the_verification() {
    let s = setup().await;
    let secret = s.enroll(EMAIL).await;
    let wrong = wrong_code(&secret);
    for _ in 0..5 {
        assert_eq!(s.verify(&wrong).await.error(), "invalid_code");
    }
    let reply = s.verify(&code(&secret, 0)).await;
    assert_eq!(reply.status, StatusCode::TOO_MANY_REQUESTS);
    // A new enrollment does not lift the lock.
    let other = s.enroll(EMAIL).await;
    assert_eq!(
        s.verify(&code(&other, 0)).await.status,
        StatusCode::TOO_MANY_REQUESTS
    );
}

#[tokio::test]
async fn completion_needs_a_confirmed_enrollment_for_the_address() {
    let s = setup().await;
    assert_eq!(s.complete().await.error(), "totp_required");
    let secret = s.enroll(EMAIL).await;
    assert_eq!(s.complete().await.error(), "totp_required", "unconfirmed");
    // Confirmed for another address.
    s.enroll("eva@example.com").await;
    let reply = s
        .post("/setup/totp/verify", json!({ "code": code(&secret, 0) }))
        .await;
    assert_eq!(reply.error(), "invalid_code", "the secret was replaced");
    let other = s.enroll("eva@example.com").await;
    assert_eq!(s.verify(&code(&other, 0)).await.status, StatusCode::OK);
    assert_eq!(s.complete().await.error(), "totp_required");
    assert_eq!(s.env.count("users").await, 0);
}

#[tokio::test]
async fn complete_validates_before_storing_anything() {
    let s = setup().await;
    s.confirmed().await;
    let reply = s
        .post(
            "/setup/complete",
            json!({
                "name": " ",
                "email": EMAIL,
                "password": "short",
                "settings": { "lang": "en", "mode": "system", "accent": "apricot" },
            }),
        )
        .await;
    assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        reply.json()["fields"],
        json!({
            "name": { "code": "required" },
            "password": { "code": "policy", "rules": ["len", "case", "num"] },
        })
    );
    let mut unknown = complete_body();
    unknown["role"] = json!("user");
    let reply = s.post("/setup/complete", unknown).await;
    assert_eq!(reply.status, StatusCode::BAD_REQUEST);
    assert_eq!(s.env.count("users").await, 0);
}

#[tokio::test]
async fn completion_creates_and_signs_in_the_administrator() {
    let s = setup().await;
    let (_, codes, used) = s.confirmed().await;
    let reply = s.complete().await;
    assert_eq!(reply.status, StatusCode::CREATED, "{}", reply.body);
    let user = reply.json()["user"].clone();
    assert_eq!(user["username"], "max");
    assert_eq!(user["email"], EMAIL);
    assert_eq!(user["display_name"], "Max Example");
    assert_eq!(user["role"], "admin");
    let cookie = reply
        .set_cookies()
        .into_iter()
        .find(|c| c.starts_with(session::COOKIE))
        .unwrap();
    assert!(cookie.contains("Max-Age=2592000; Path=/; HttpOnly; Secure; SameSite=Lax"));
    let token = reply.cookie(session::COOKIE).unwrap();

    // Signed in, with the appearance of the first step.
    let me = s.env.as_user(&token, Method::GET, "/session", None).await;
    assert_eq!(me.status, StatusCode::OK, "{}", me.body);
    assert_eq!(me.json()["user"]["locale"], "de");
    assert_eq!(me.json()["user"]["theme"], "dark");
    assert_eq!(me.json()["hub"]["admin"], "/admin");
    let settings = s.env.as_user(&token, Method::GET, "/settings", None).await;
    assert_eq!(
        settings.json(),
        json!({ "version": 1, "settings": { "lang": "de", "mode": "dark", "accent": "sage" } })
    );
    let devices = s
        .env
        .as_user(&token, Method::GET, "/auth/totp/devices", None)
        .await;
    let devices = devices.json();
    assert_eq!(devices["devices"].as_array().unwrap().len(), 1);
    // The default name in the administrator's language.
    assert_eq!(devices["devices"][0]["name"], "Authenticator-App");
    assert_eq!(devices["recovery"]["unused"], 10);

    // Token and enrollment are gone, every step answers `setup_done`.
    assert_eq!(
        db::call(&s.env.state.db, |conn| setup_token::current(conn))
            .await
            .unwrap(),
        None
    );
    for path in ["/setup/token", "/setup/totp", "/setup/complete"] {
        let reply = s.post(path, json!({ "token": s.token })).await;
        assert_eq!(reply.status, StatusCode::CONFLICT, "{path}");
        assert_eq!(reply.error(), "setup_done");
    }
    assert_eq!(
        s.env.audit_actions().await,
        ["setup.complete", "auth.login"]
    );

    // Signing in again needs the app; the code of the confirmation is used up.
    let first = s.env.login("max", PASSWORD).await;
    assert_eq!(first.json()["status"], "totp_required");
    let mfa = first.cookie(challenge::COOKIE).unwrap();
    let reply = s
        .env
        .request(
            Method::POST,
            "/auth/totp",
            Some(json!({ "code": used })),
            &[(challenge::COOKIE, &mfa)],
        )
        .await;
    assert_eq!(reply.error(), "code_used", "{}", reply.body);
    // A recovery code from the setup works.
    let reply = s
        .env
        .request(
            Method::POST,
            "/auth/recovery",
            Some(json!({ "code": codes[0] })),
            &[(challenge::COOKIE, &mfa)],
        )
        .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
}
