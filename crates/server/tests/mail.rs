//! System mails through the API against a fake JMAP server: the sending
//! account under Administration › Email, "Forgot password", the email
//! change, invitations and the security notices.

mod common;
mod fake_jmap;

use std::time::Duration;

use axum::http::{Method, StatusCode};
use common::{Env, PASSWORD, code, env};
use fake_jmap::{MAIL_PASSWORD, MAIL_USER, MailState, Options, Server};
use roedy_hub_server::{auth::session, db::repo::users::Role};
use serde_json::{Value, json};

/// Environment with a sending account on a running fake server.
async fn with_mail() -> (Env, Server) {
    let env = env().await;
    let fake = Server::start(Options::default()).await;
    env.configure_mail(&fake.url, MAIL_USER, MAIL_PASSWORD)
        .await;
    (env, fake)
}

/// Waits until `n` mails to `to` with `subject` were sent and returns them
/// (the API sends some in the background).
async fn mails(fake: &Server, to: &str, subject: &str, n: usize) -> Vec<Value> {
    for _ in 0..300 {
        let sent: Vec<Value> = fake
            .mail()
            .sent_to(to)
            .into_iter()
            .filter(|mail| mail["subject"] == subject)
            .cloned()
            .collect();
        if sent.len() >= n {
            assert_eq!(sent.len(), n, "more mails to {to} than expected");
            return sent;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!(
        "not {n} mails {subject:?} to {to}: {:?}",
        fake.mail().submitted
    );
}

/// The one mail to `to` with `subject`.
async fn mail(fake: &Server, to: &str, subject: &str) -> Value {
    mails(fake, to, subject, 1).await.remove(0)
}

const NEW_SIGN_IN: &str = "New sign-in to Roedy Hub";

/// Gives background tasks time; then nothing more must have been sent.
async fn settle() {
    tokio::time::sleep(Duration::from_millis(100)).await;
}

/// The token after `path` in the mail's text.
fn token_after(mail: &Value, path: &str) -> String {
    let text = MailState::text(mail);
    let start = text
        .find(path)
        .unwrap_or_else(|| panic!("{path} not in {text}"))
        + path.len();
    text[start..]
        .chars()
        .take_while(char::is_ascii_alphanumeric)
        .collect()
}

#[tokio::test]
async fn administrators_set_and_test_the_sending_account() {
    let env = env().await;
    let fake = Server::start(Options::default()).await;
    env.add_user("anna", "anna@example.com", Role::Admin).await;
    env.add_user("max", "max@example.com", Role::User).await;
    let admin = env.session_for("anna").await;
    let user = env.session_for("max").await;

    let shown = env
        .as_user(&admin, Method::GET, "/admin/system-mail", None)
        .await;
    assert_eq!(shown.status, StatusCode::OK);
    assert_eq!(shown.json()["status"], "none");
    assert_eq!(shown.json()["url"], Value::Null);
    let me = env.as_user(&admin, Method::GET, "/auth/me", None).await;
    assert_eq!(me.json()["system_mail_configured"], false);
    // Only for administrators.
    let reply = env
        .as_user(&user, Method::GET, "/admin/system-mail", None)
        .await;
    assert_eq!(reply.status, StatusCode::FORBIDDEN);
    assert!(
        env.as_user(&user, Method::GET, "/auth/me", None)
            .await
            .json()
            .get("system_mail_configured")
            .is_none()
    );

    // A wrong password is stored, but checked right away.
    let put = |body: Value| env.as_user(&admin, Method::PUT, "/admin/system-mail", Some(body));
    let reply = put(json!({
        "url": fake.url, "user": MAIL_USER, "app_password": "wrong",
    }))
    .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    assert_eq!(reply.json()["status"], "err");
    assert_eq!(reply.json()["reason"], "auth_failed");
    assert!(!reply.body.contains("wrong"));

    let reply = put(json!({ "app_password": MAIL_PASSWORD })).await;
    let view = reply.json();
    assert_eq!(view["status"], "ok", "{view}");
    assert_eq!(view["url"], fake.url);
    assert_eq!(view["has_secret"], true);
    assert_eq!(view["identities"].as_array().unwrap().len(), 2);
    assert!(!reply.body.contains(MAIL_PASSWORD));
    let me = env.as_user(&admin, Method::GET, "/auth/me", None).await;
    assert_eq!(me.json()["system_mail_configured"], true);

    // Missing fields and an address that is no server.
    let reply = put(json!({ "url": "https://mail.example.com/jmap" })).await;
    assert_eq!(
        reply.json()["fields"],
        json!({ "url": { "code": "invalid" } })
    );

    // Test without saving, then a test email.
    let test =
        |body: Value| env.as_user(&admin, Method::POST, "/admin/system-mail/test", Some(body));
    let reply = test(json!({})).await;
    assert_eq!(reply.json()["sent"], false, "{}", reply.body);
    let reply = test(json!({ "to": "anna@example.com" })).await;
    assert_eq!(reply.json()["sent"], true, "{}", reply.body);
    mail(&fake, "anna@example.com", "Test email from Roedy Hub").await;
    // A draft with a wrong password is reported, the stored one stays.
    let reply = test(json!({ "app_password": "wrong" })).await;
    assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(reply.json()["reason"], "auth_failed");
    let shown = env
        .as_user(&admin, Method::GET, "/admin/system-mail", None)
        .await;
    assert_eq!(shown.json()["status"], "ok");

    // Saving without `X-Requested-With` is a CSRF error.
    let reply = env
        .without_csrf(
            Method::PUT,
            "/admin/system-mail",
            json!({}),
            &[(session::COOKIE, &admin)],
        )
        .await;
    assert_eq!(reply.status, StatusCode::FORBIDDEN);
    assert_eq!(reply.error(), "csrf");
    assert!(
        env.audit_actions()
            .await
            .contains(&"system_mail.update".to_owned())
    );
}

#[tokio::test]
async fn forgot_password_sends_a_link_that_works_once() {
    let (env, fake) = with_mail().await;
    let id = env.add_user("max", "max@example.com", Role::User).await;
    let other = env.session_for("max").await;
    let forgot = |email: &str| {
        env.request(
            Method::POST,
            "/auth/password/forgot",
            Some(json!({ "email": email })),
            &[],
        )
    };
    // Unknown addresses get the same answer, but no mail.
    assert_eq!(
        forgot("nobody@example.com").await.status,
        StatusCode::ACCEPTED
    );
    assert_eq!(forgot("MAX@example.com").await.status, StatusCode::ACCEPTED);
    let sent = mail(&fake, "max@example.com", "Reset your Roedy Hub password").await;
    let token = token_after(&sent, "/login/reset/");
    assert_eq!(token.len(), 64);
    assert!(fake.mail().sent_to("nobody@example.com").is_empty());

    let reset = async |token: &str, password: &str| {
        env.request(
            Method::POST,
            &format!("/auth/password/reset/{token}"),
            Some(json!({ "password": password })),
            &[],
        )
        .await
    };
    // The policy applies.
    let reply = reset(&token, "short").await;
    assert_eq!(reply.json()["fields"]["password"]["code"], "policy");
    let reply = reset(&token, "Winter2026!").await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    assert_eq!(reply.json()["sessions_ended"], 1);
    // Used up, and the other session is gone.
    assert_eq!(reset(&token, "Spring2026!").await.error(), "invalid_token");
    assert_eq!(reset("abc", "Spring2026!").await.error(), "invalid_token");
    let me = env.as_user(&other, Method::GET, "/auth/me", None).await;
    assert_eq!(me.status, StatusCode::UNAUTHORIZED);
    assert_eq!(
        env.login("max", PASSWORD).await.error(),
        "invalid_credentials"
    );
    assert_eq!(env.login("max", "Winter2026!").await.status, StatusCode::OK);

    // "Password reset" notice to the account.
    mail(&fake, "max@example.com", "Your password was reset").await;
    let actions = env.audit_actions().await;
    assert!(actions.contains(&"auth.password_forgot".to_owned()));
    assert!(actions.contains(&"auth.password_reset".to_owned()));
    let _ = id;
}

#[tokio::test]
async fn forgot_password_needs_csrf_and_a_valid_address() {
    let (env, _fake) = with_mail().await;
    let reply = env
        .request(
            Method::POST,
            "/auth/password/forgot",
            Some(json!({ "email": "no-address" })),
            &[],
        )
        .await;
    assert_eq!(reply.json()["fields"]["email"]["code"], "invalid");
    let reply = env
        .without_csrf(
            Method::POST,
            "/auth/password/forgot",
            json!({ "email": "max@example.com" }),
            &[],
        )
        .await;
    assert_eq!(reply.error(), "csrf");
    // The other new public endpoints as well.
    for path in [
        "/auth/password/reset/abc",
        "/auth/email/confirm",
        "/invites/abc/accept",
        "/invites/abc/totp",
    ] {
        let reply = env.without_csrf(Method::POST, path, json!({}), &[]).await;
        assert_eq!(reply.error(), "csrf", "{path}");
    }
}

/// Ten requests per IP at once, then 429.
#[tokio::test]
async fn forgot_password_is_limited_per_ip() {
    let (env, _fake) = with_mail().await;
    for _ in 0..10 {
        let reply = env
            .request(
                Method::POST,
                "/auth/password/forgot",
                Some(json!({ "email": "max@example.com" })),
                &[],
            )
            .await;
        assert_eq!(reply.status, StatusCode::ACCEPTED);
    }
    let reply = env
        .request(
            Method::POST,
            "/auth/password/forgot",
            Some(json!({ "email": "max@example.com" })),
            &[],
        )
        .await;
    assert_eq!(reply.status, StatusCode::TOO_MANY_REQUESTS);
}

#[tokio::test]
async fn email_change_in_two_steps_with_confirmation() {
    let (env, fake) = with_mail().await;
    env.add_user("max", "max@example.com", Role::User).await;
    env.add_user("eva", "eva@example.com", Role::User).await;
    let token = env.session_for("max").await;
    let patch = |body: Value| env.as_user(&token, Method::PATCH, "/auth/me", Some(body));

    // Taken by another account.
    let reply = patch(json!({ "email": "eva@example.com" })).await;
    assert_eq!(reply.error(), "email_taken");
    // First step: the name applies, the address waits.
    let reply = patch(json!({ "display_name": "Max Miller", "email": "new@example.com" })).await;
    assert_eq!(reply.json()["status"], "reauth_required", "{}", reply.body);
    assert_eq!(reply.json()["user"]["display_name"], "Max Miller");
    assert_eq!(reply.json()["user"]["email"], "max@example.com");
    // Second step with a wrong password: nothing sent.
    let reply = patch(json!({ "email": "new@example.com", "password": "wrong" })).await;
    assert_eq!(reply.error(), "invalid_password");
    let reply = patch(json!({ "email": "new@example.com", "password": PASSWORD })).await;
    assert_eq!(reply.json()["status"], "email_sent", "{}", reply.body);
    assert_eq!(reply.json()["email_change"]["email"], "new@example.com");
    let me = env.as_user(&token, Method::GET, "/auth/me", None).await;
    assert_eq!(me.json()["email_change"]["email"], "new@example.com");
    assert_eq!(me.json()["email"], "max@example.com");

    // The notice to the previous address, the link to the new one.
    let notice = mail(
        &fake,
        "max@example.com",
        "Your email address for Roedy Hub is being changed",
    )
    .await;
    assert!(MailState::text(&notice).contains("New address: new@example.com"));
    let subject = "Confirm your new email address for Roedy Hub";
    let confirm = mail(&fake, "new@example.com", subject).await;
    let link = token_after(&confirm, "/account/confirm-email/");

    // Checking changes nothing; confirming applies the address.
    let check = env
        .request(
            Method::POST,
            "/auth/email/check",
            Some(json!({ "token": link })),
            &[],
        )
        .await;
    assert_eq!(
        check.json(),
        json!({ "state": "valid", "own_session": false, "email": "new@example.com" })
    );
    let confirmed = env
        .as_user(
            &token,
            Method::POST,
            "/auth/email/confirm",
            Some(json!({ "token": link })),
        )
        .await;
    assert_eq!(confirmed.json()["state"], "confirmed", "{}", confirmed.body);
    assert_eq!(confirmed.json()["own_session"], true);
    let me = env.as_user(&token, Method::GET, "/auth/me", None).await;
    assert_eq!(me.json()["email"], "new@example.com");
    assert_eq!(me.json()["email_change"], Value::Null);
    // Again: already confirmed; unknown: invalid.
    let again = env
        .request(
            Method::POST,
            "/auth/email/confirm",
            Some(json!({ "token": link })),
            &[],
        )
        .await;
    assert_eq!(again.json()["state"], "already_confirmed");
    let unknown = env
        .request(
            Method::POST,
            "/auth/email/check",
            Some(json!({ "token": "f".repeat(64) })),
            &[],
        )
        .await;
    assert_eq!(unknown.json()["state"], "invalid");
    assert_eq!(
        env.login("new@example.com", PASSWORD).await.status,
        StatusCode::OK
    );
}

/// Changing the password and "Sign out all others" discard open links from
/// mails (the steps `security_email` advises).
#[tokio::test]
async fn open_links_end_with_password_change_and_sign_out() {
    let (env, fake) = with_mail().await;
    env.add_user("max", "max@example.com", Role::User).await;
    let token = env.session_for("max").await;
    let patch = |body: Value| env.as_user(&token, Method::PATCH, "/auth/me", Some(body));
    let reply = patch(json!({ "email": "new@example.com", "password": PASSWORD })).await;
    assert_eq!(reply.json()["status"], "email_sent", "{}", reply.body);
    assert_eq!(env.count("email_changes").await, 1);
    let reply = env
        .as_user(&token, Method::POST, "/auth/sessions/revoke-others", None)
        .await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(env.count("email_changes").await, 0);

    let reply = patch(json!({ "email": "new@example.com", "password": PASSWORD })).await;
    assert_eq!(reply.json()["status"], "email_sent", "{}", reply.body);
    env.request(
        Method::POST,
        "/auth/password/forgot",
        Some(json!({ "email": "max@example.com" })),
        &[],
    )
    .await;
    let changing = "Your email address for Roedy Hub is being changed";
    mails(&fake, "max@example.com", changing, 2).await;
    mail(&fake, "max@example.com", "Reset your Roedy Hub password").await;
    assert_eq!(env.count("password_resets").await, 1);
    let reply = env
        .as_user(
            &token,
            Method::PUT,
            "/auth/password",
            Some(json!({ "current_password": PASSWORD, "new_password": "Winter2026!" })),
        )
        .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    assert_eq!(env.count("email_changes").await, 0);
    assert_eq!(env.count("password_resets").await, 0);
    mail(&fake, "max@example.com", "Your password was changed").await;
}

/// Without a sending account the change cannot start (502 `mail_failed`)
/// and nothing is stored.
#[tokio::test]
async fn email_change_without_system_mail_fails() {
    let env = env().await;
    env.add_user("max", "max@example.com", Role::User).await;
    let token = env.session_for("max").await;
    let reply = env
        .as_user(
            &token,
            Method::PATCH,
            "/auth/me",
            Some(json!({ "email": "new@example.com", "password": PASSWORD })),
        )
        .await;
    assert_eq!(reply.status, StatusCode::BAD_GATEWAY);
    assert_eq!(reply.error(), "mail_failed");
    assert_eq!(env.count("email_changes").await, 0);
}

#[tokio::test]
async fn invitations_are_sent_and_accepted() {
    let (env, fake) = with_mail().await;
    env.add_user("anna", "anna@example.com", Role::Admin).await;
    env.add_user("max", "max@example.com", Role::User).await;
    let admin = env.session_for("anna").await;
    let user = env.session_for("max").await;

    let create = async |session: &str, body: Value| {
        env.as_user(session, Method::POST, "/invites", Some(body))
            .await
    };
    assert_eq!(
        create(
            &user,
            json!({ "emails": ["x@example.com"], "role": "user" })
        )
        .await
        .status,
        StatusCode::FORBIDDEN
    );
    let reply = create(
        &admin,
        json!({ "emails": ["jonas@example.com", "max@example.com"], "role": "user" }),
    )
    .await;
    assert_eq!(
        reply.json()["fields"],
        json!({ "emails.1": { "code": "taken" } })
    );
    let reply = create(
        &admin,
        json!({ "emails": ["jonas@example.com"], "role": "user", "name": "Jonas Example" }),
    )
    .await;
    assert_eq!(reply.status, StatusCode::CREATED, "{}", reply.body);
    assert_eq!(reply.json()["system_mail"], true);
    assert_eq!(reply.json()["invites"][0]["sent"], true);
    let id = reply.json()["invites"][0]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let sent = mail(&fake, "jonas@example.com", "Invitation to Roedy Hub").await;
    let token = token_after(&sent, "/invite/");

    let list = env.as_user(&admin, Method::GET, "/invites", None).await;
    assert_eq!(list.json()["invites"][0]["email"], "jonas@example.com");
    assert!(list.json()["invites"][0]["mail_sent_at"].is_string());
    let link = env
        .as_user(&admin, Method::GET, &format!("/invites/{id}/link"), None)
        .await;
    assert_eq!(
        link.json()["url"],
        format!("https://app.example.com/invite/{token}")
    );
    let resent = env
        .as_user(&admin, Method::POST, &format!("/invites/{id}/resend"), None)
        .await;
    assert_eq!(resent.status, StatusCode::OK, "{}", resent.body);
    mails(&fake, "jonas@example.com", "Invitation to Roedy Hub", 2).await;

    // The invitation page.
    let shown = env
        .request(Method::GET, &format!("/invites/{token}"), None, &[])
        .await;
    assert_eq!(shown.json()["name"], "Jonas Example");
    assert_eq!(shown.json()["inviter"], "Max Example");
    assert_eq!(shown.json()["expired"], false);
    let unknown = env
        .request(
            Method::GET,
            &format!("/invites/{}", "a".repeat(64)),
            None,
            &[],
        )
        .await;
    assert_eq!(unknown.status, StatusCode::NOT_FOUND);

    // Without the multi-factor step: password only.
    let accept = async |body: Value| {
        env.request(
            Method::POST,
            &format!("/invites/{token}/accept"),
            Some(body),
            &[],
        )
        .await
    };
    let reply = accept(json!({ "name": "Jonas Example", "password": "short" })).await;
    assert_eq!(reply.json()["fields"]["password"]["code"], "policy");
    let reply = accept(json!({ "name": "Jonas Example", "password": "Winter2026!" })).await;
    assert_eq!(reply.status, StatusCode::CREATED, "{}", reply.body);
    assert_eq!(reply.json()["user"]["email"], "jonas@example.com");
    assert_eq!(reply.json()["recovery_codes"], Value::Null);
    let cookie = reply.cookie(session::COOKIE).unwrap();
    let me = env.as_user(&cookie, Method::GET, "/auth/me", None).await;
    assert_eq!(me.json()["mfa_enabled"], false);
    // The link is used up; the device of the acceptance is known.
    assert_eq!(
        accept(json!({ "name": "J", "password": "Winter2026!" }))
            .await
            .status,
        StatusCode::NOT_FOUND
    );
    assert_eq!(env.count("invitations").await, 0);
    assert_eq!(
        env.login("jonas@example.com", "Winter2026!").await.status,
        StatusCode::OK
    );
    settle().await;
    assert!(
        fake.mail()
            .sent_to("jonas@example.com")
            .iter()
            .all(|mail| mail["subject"] != NEW_SIGN_IN)
    );
    let actions = env.audit_actions().await;
    for action in ["users.invite", "users.invite_resend", "users.invite_accept"] {
        assert!(actions.contains(&action.to_owned()), "{action}");
    }
}

#[tokio::test]
async fn invitations_with_an_authenticator_app() {
    let (env, _fake) = with_mail().await;
    env.add_user("anna", "anna@example.com", Role::Admin).await;
    let admin = env.session_for("anna").await;
    let reply = env
        .as_user(
            &admin,
            Method::POST,
            "/invites",
            Some(json!({ "emails": ["lena@example.com"], "role": "admin" })),
        )
        .await;
    let id = reply.json()["invites"][0]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let link = env
        .as_user(&admin, Method::GET, &format!("/invites/{id}/link"), None)
        .await;
    let url = link.json()["url"].as_str().unwrap().to_owned();
    let token = url.rsplit('/').next().unwrap().to_owned();
    let accept = async |body: Value| {
        env.request(
            Method::POST,
            &format!("/invites/{token}/accept"),
            Some(body),
            &[],
        )
        .await
    };
    // A code without the step.
    let reply =
        accept(json!({ "name": "Lena", "password": "Winter2026!", "code": "123456" })).await;
    assert_eq!(reply.error(), "totp_required");

    let enrolled = env
        .request(Method::POST, &format!("/invites/{token}/totp"), None, &[])
        .await;
    assert_eq!(enrolled.status, StatusCode::OK, "{}", enrolled.body);
    assert!(
        enrolled.json()["uri"]
            .as_str()
            .unwrap()
            .starts_with("otpauth://totp/")
    );
    let secret = totp_rs::Secret::try_from_base32(enrolled.json()["secret"].as_str().unwrap())
        .unwrap()
        .to_vec();
    let reply = accept(
        json!({ "name": "Lena", "password": "Winter2026!", "code": common::wrong_code(&secret) }),
    )
    .await;
    assert_eq!(reply.error(), "invalid_code");
    let reply =
        accept(json!({ "name": "Lena", "password": "Winter2026!", "code": code(&secret, 0) }))
            .await;
    assert_eq!(reply.status, StatusCode::CREATED, "{}", reply.body);
    assert_eq!(reply.json()["user"]["role"], "admin");
    assert_eq!(reply.json()["recovery_codes"].as_array().unwrap().len(), 10);
    let cookie = reply.cookie(session::COOKIE).unwrap();
    let me = env.as_user(&cookie, Method::GET, "/auth/me", None).await;
    assert_eq!(me.json()["mfa_enabled"], true);
}

#[tokio::test]
async fn invitations_without_system_mail_are_only_created_and_can_be_revoked() {
    let env = env().await;
    env.add_user("anna", "anna@example.com", Role::Admin).await;
    let admin = env.session_for("anna").await;
    let reply = env
        .as_user(
            &admin,
            Method::POST,
            "/invites",
            Some(json!({ "emails": ["jonas@example.com"], "role": "user" })),
        )
        .await;
    assert_eq!(reply.status, StatusCode::CREATED);
    assert_eq!(reply.json()["system_mail"], false);
    assert_eq!(reply.json()["invites"][0]["sent"], false);
    let id = reply.json()["invites"][0]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let resent = env
        .as_user(&admin, Method::POST, &format!("/invites/{id}/resend"), None)
        .await;
    assert_eq!(resent.error(), "mail_failed");
    let revoked = env
        .as_user(&admin, Method::DELETE, &format!("/invites/{id}"), None)
        .await;
    assert_eq!(revoked.status, StatusCode::NO_CONTENT);
    let again = env
        .as_user(&admin, Method::DELETE, &format!("/invites/{id}"), None)
        .await;
    assert_eq!(again.status, StatusCode::NOT_FOUND);
    assert_eq!(env.count("invitations").await, 0);
}

/// A sign-in from another network or browser is a new device; the same
/// device again sends nothing.
#[tokio::test]
async fn new_devices_get_a_sign_in_notice() {
    let (env, fake) = with_mail().await;
    env.add_user("max", "max@example.com", Role::User).await;
    env.session_for("max").await;
    let first = mail(&fake, "max@example.com", NEW_SIGN_IN).await;
    assert!(MailState::text(&first).contains("Firefox on Linux"));
    assert!(MailState::text(&first).contains("203.0.x.x"));
    env.session_for("max").await;
    settle().await;
    assert_eq!(fake.mail().sent_to("max@example.com").len(), 1);

    let reply = env
        .send(
            [198, 51, 100, 9],
            Method::POST,
            "/auth/login",
            Some(json!({ "login": "max", "password": PASSWORD })),
            &[],
            &[],
        )
        .await;
    assert_eq!(reply.status, StatusCode::OK);
    let sent = mails(&fake, "max@example.com", NEW_SIGN_IN, 2).await;
    assert!(MailState::text(&sent[1]).contains("198.51.x.x"));
    assert_eq!(env.count("known_devices").await, 2);
}

#[tokio::test]
async fn authenticator_changes_send_notices() {
    let (env, fake) = with_mail().await;
    let id = env.add_user("max", "max@example.com", Role::User).await;
    let token = env.session_for("max").await;
    mail(&fake, "max@example.com", NEW_SIGN_IN).await;
    let secret = env.add_device(&id, "Phone").await;
    let reply = env
        .as_user(
            &token,
            Method::POST,
            "/auth/recovery-codes",
            Some(json!({ "password": PASSWORD, "code": code(&secret, 0) })),
        )
        .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    mail(&fake, "max@example.com", "New recovery codes generated").await;
    let codes: Vec<String> =
        serde_json::from_value(reply.json()["recovery_codes"].clone()).unwrap();

    // Sign-in with a recovery code: its own notice, no "New sign-in".
    let first = env.login("max", PASSWORD).await;
    let mfa = first
        .cookie(roedy_hub_server::auth::challenge::COOKIE)
        .unwrap();
    let reply = env
        .request(
            Method::POST,
            "/auth/recovery",
            Some(json!({ "code": codes[0] })),
            &[(roedy_hub_server::auth::challenge::COOKIE, &mfa)],
        )
        .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    let used = "Recovery code used to sign in to Roedy Hub";
    let sent = mail(&fake, "max@example.com", used).await;
    assert!(MailState::text(&sent).contains("Unused: 9 of 10"));
    // The same device: no further "New sign-in".
    mails(&fake, "max@example.com", NEW_SIGN_IN, 1).await;

    let devices = env
        .as_user(&token, Method::GET, "/auth/totp/devices", None)
        .await;
    let device = devices.json()["devices"][0]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let reply = env
        .as_user(
            &token,
            Method::DELETE,
            &format!("/auth/totp/devices/{device}"),
            Some(json!({ "password": PASSWORD, "code": code(&secret, 1) })),
        )
        .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    mail(&fake, "max@example.com", "Authenticator app removed").await;
}

/// Without a reachable server notices are only logged; the change itself
/// succeeds.
#[tokio::test]
async fn unreachable_server_does_not_block_sign_in() {
    let env = env().await;
    env.configure_mail(&fake_jmap::unreachable_url(), MAIL_USER, MAIL_PASSWORD)
        .await;
    env.add_user("max", "max@example.com", Role::User).await;
    assert_eq!(env.login("max", PASSWORD).await.status, StatusCode::OK);
}
