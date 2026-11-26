//! Administration via the API: users (role, lock, second factor, removal,
//! bulk actions), invitations (role, "Extend", "Request a new invitation"),
//! the audit log with its texts, organization and branding, `reset-admin`.
//! Taken over from Roedy Mail's API tests (`crates/server/tests/users.rs`,
//! `audit.rs`, `branding.rs`), trimmed to the hub.

mod common;

use axum::http::{Method, StatusCode};
use common::{Env, PASSWORD, code, env};
use roedy_hub_server::{
    auth::{challenge, session},
    db,
    db::repo::users::Role,
    reset_admin::{self, Secret},
};
use serde_json::{Value, json};

/// An administrator (`anna`) and a user (`max`), both signed in.
struct Team {
    env: Env,
    admin_id: String,
    user_id: String,
    admin: String,
    user: String,
}

async fn team() -> Team {
    let env = env().await;
    let admin_id = env.add_user("anna", "anna@example.com", Role::Admin).await;
    let user_id = env.add_user("max", "max@example.com", Role::User).await;
    let admin = env.session_for("anna").await;
    let user = env.session_for("max").await;
    Team {
        env,
        admin_id,
        user_id,
        admin,
        user,
    }
}

impl Team {
    async fn admin(&self, method: Method, path: &str, body: Option<Value>) -> common::Reply {
        self.env.as_user(&self.admin, method, path, body).await
    }
}

#[tokio::test]
async fn administration_needs_an_administrator() {
    let t = team().await;
    let user_id = t.user_id.clone();
    let id = |path: &str| path.replace("{id}", &user_id);
    let endpoints: Vec<(Method, String, Option<Value>)> = vec![
        (Method::GET, "/users".into(), None),
        (Method::GET, id("/users/{id}"), None),
        (
            Method::PATCH,
            id("/users/{id}"),
            Some(json!({ "role": "admin" })),
        ),
        (Method::POST, id("/users/{id}/lock"), None),
        (Method::POST, id("/users/{id}/unlock"), None),
        (Method::POST, id("/users/{id}/reset-mfa"), None),
        (Method::DELETE, id("/users/{id}"), None),
        (
            Method::POST,
            "/users/bulk".into(),
            Some(json!({ "action": "lock", "ids": [user_id] })),
        ),
        (
            Method::PATCH,
            "/invites/x".into(),
            Some(json!({ "role": "admin" })),
        ),
        (Method::POST, "/invites/x/renew".into(), None),
        (Method::GET, "/admin/audit".into(), None),
        (Method::POST, "/admin/audit/export".into(), None),
        (Method::GET, "/admin/audit/retention".into(), None),
        (
            Method::PUT,
            "/admin/audit/retention".into(),
            Some(json!({ "days": 30 })),
        ),
        (Method::GET, "/admin/org".into(), None),
        (
            Method::PUT,
            "/admin/org".into(),
            Some(json!({ "name": "X" })),
        ),
        (Method::GET, "/admin/branding".into(), None),
        (
            Method::PUT,
            "/admin/branding".into(),
            Some(json!({ "color": "sage" })),
        ),
    ];
    for (method, path, body) in endpoints {
        let signed_out = t
            .env
            .request(method.clone(), &path, body.clone(), &[])
            .await;
        assert_eq!(
            signed_out.status,
            StatusCode::UNAUTHORIZED,
            "{method} {path}"
        );
        let user = t
            .env
            .as_user(&t.user, method.clone(), &path, body.clone())
            .await;
        assert_eq!(user.status, StatusCode::FORBIDDEN, "{method} {path}");
        assert_eq!(user.error(), "forbidden", "{method} {path}");
        // Modifying requests need `X-Requested-With`, also from an administrator.
        if method != Method::GET {
            let foreign = t
                .env
                .without_csrf(
                    method.clone(),
                    &path,
                    body.unwrap_or(json!({})),
                    &[(session::COOKIE, &t.admin)],
                )
                .await;
            assert_eq!(foreign.error(), "csrf", "{method} {path}");
        }
    }
    assert!(
        t.env
            .audit_actions()
            .await
            .iter()
            .all(|a| a == "auth.login"),
        "nothing changed"
    );
}

#[tokio::test]
async fn list_and_details_show_users_and_invitations() {
    let t = team().await;
    t.env.add_device(&t.user_id, "Phone").await;
    let reply = t
        .admin(
            Method::POST,
            "/invites",
            Some(json!({ "emails": ["jonas@example.com"], "role": "user" })),
        )
        .await;
    assert_eq!(reply.status, StatusCode::CREATED, "{}", reply.body);

    let list = t.admin(Method::GET, "/users", None).await.json();
    // Users carry `mfa` as a boolean; invitations have none.
    let rows: Vec<(&str, &str, Option<bool>)> = list["users"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| {
            let mfa = row
                .get("mfa")
                .map(|mfa| mfa.as_bool().expect("mfa is a boolean"));
            (
                row["email"].as_str().unwrap(),
                row["status"].as_str().unwrap(),
                mfa,
            )
        })
        .collect();
    assert_eq!(
        rows,
        [
            ("anna@example.com", "active", Some(false)),
            ("max@example.com", "active", Some(true)),
            ("jonas@example.com", "invited", None),
        ]
    );
    assert!(list["users"][2]["requested_at"].is_null());

    let reply = t
        .admin(Method::GET, &format!("/users/{}", t.user_id), None)
        .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    let user = &reply.json()["user"];
    assert_eq!(user["username"], "max");
    assert_eq!(user["sessions"].as_array().unwrap().len(), 1);
    assert!(user["sessions"][0].get("ip").is_none(), "no IP addresses");
    assert!(user["recovery_codes"].is_null());
    assert_eq!(
        t.admin(Method::GET, "/users/missing", None).await.status,
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn the_own_account_is_not_changed_here() {
    let t = team().await;
    let own = format!("/users/{}", t.admin_id);
    for (method, path, body) in [
        (Method::PATCH, own.clone(), Some(json!({ "role": "user" }))),
        (Method::POST, format!("{own}/lock"), None),
        (Method::POST, format!("{own}/reset-mfa"), None),
        (Method::DELETE, own.clone(), None),
    ] {
        let reply = t.admin(method.clone(), &path, body).await;
        assert_eq!(
            (reply.status, reply.error().as_str()),
            (StatusCode::UNPROCESSABLE_ENTITY, "own_account"),
            "{method} {path}"
        );
    }
    // In bulk actions the own account is skipped.
    let reply = t
        .admin(
            Method::POST,
            "/users/bulk",
            Some(json!({ "action": "lock", "ids": [t.admin_id] })),
        )
        .await;
    assert_eq!(reply.json(), json!({ "changed": [] }));
    let me = t.admin(Method::GET, "/session", None).await;
    assert_eq!(me.status, StatusCode::OK);
}

#[tokio::test]
async fn locking_ends_sessions_and_pending_sign_ins() {
    let t = team().await;
    let secret = t.env.add_device(&t.user_id, "Phone").await;
    // A sign-in waiting for the code.
    let first = t.env.login("max", PASSWORD).await;
    let challenge = first.cookie(challenge::COOKIE).unwrap();

    let reply = t
        .admin(Method::POST, &format!("/users/{}/lock", t.user_id), None)
        .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    assert_eq!(reply.json()["user"]["status"], "locked");
    let session = t.env.as_user(&t.user, Method::GET, "/session", None).await;
    assert_eq!(session.status, StatusCode::UNAUTHORIZED);
    let again = t.env.login("max", PASSWORD).await;
    assert_ne!(again.status, StatusCode::OK);

    // Locking twice changes nothing; unlocking lets the user in again – but
    // not with the sign-in step from before the lock.
    t.admin(Method::POST, &format!("/users/{}/lock", t.user_id), None)
        .await;
    let reply = t
        .admin(Method::POST, &format!("/users/{}/unlock", t.user_id), None)
        .await;
    assert_eq!(reply.json()["user"]["status"], "active");
    let totp = async |challenge: &str| {
        t.env
            .request(
                Method::POST,
                "/auth/totp",
                Some(json!({ "code": code(&secret, 0) })),
                &[(challenge::COOKIE, challenge)],
            )
            .await
    };
    let old_step = totp(&challenge).await;
    assert_ne!(old_step.status, StatusCode::OK, "{}", old_step.body);
    assert!(old_step.cookie(session::COOKIE).is_none());
    let reply = t.env.login("max", PASSWORD).await;
    assert_eq!(reply.json()["status"], "totp_required", "{}", reply.body);
    let fresh = reply.cookie(challenge::COOKIE).unwrap();
    let signed_in = totp(&fresh).await;
    assert_eq!(signed_in.status, StatusCode::OK, "{}", signed_in.body);
    assert!(signed_in.cookie(session::COOKIE).is_some());

    let entries = audit_rows(&t.env, "users.%").await;
    assert_eq!(
        entries,
        [
            (
                "users.lock".into(),
                r#"{"email":"max@example.com","name":"Max Example","sessions_ended":1}"#.into()
            ),
            (
                "users.unlock".into(),
                r#"{"email":"max@example.com","name":"Max Example"}"#.into()
            ),
        ]
    );
}

/// `(action, details)` of the audit entries matching `pattern`.
async fn audit_rows(env: &Env, pattern: &'static str) -> Vec<(String, String)> {
    env.sql(move |conn| {
        conn.prepare(
            "SELECT action, coalesce(details, '') FROM audit_log WHERE action LIKE ?1 ORDER BY id",
        )?
        .query_map([pattern], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect()
    })
    .await
}

#[tokio::test]
async fn roles_change_and_a_demoted_admin_can_no_longer_act() {
    let t = team().await;
    let reply = t
        .admin(
            Method::PATCH,
            &format!("/users/{}", t.user_id),
            Some(json!({ "role": "admin" })),
        )
        .await;
    assert_eq!(reply.json()["user"]["role"], "admin", "{}", reply.body);
    // The new administrator takes the role of the first one …
    let reply = t
        .env
        .as_user(
            &t.user,
            Method::PATCH,
            &format!("/users/{}", t.admin_id),
            Some(json!({ "role": "user" })),
        )
        .await;
    assert_eq!(reply.json()["user"]["role"], "user", "{}", reply.body);
    // … who cannot act any more, although the session still says admin.
    let reply = t
        .admin(Method::POST, &format!("/users/{}/lock", t.user_id), None)
        .await;
    assert_eq!(reply.status, StatusCode::FORBIDDEN);
    let rows = audit_rows(&t.env, "users.role").await;
    assert_eq!(rows.len(), 2);
    assert!(rows[1].1.contains(r#""previous":"admin""#), "{}", rows[1].1);
    let reply = t
        .admin(
            Method::PATCH,
            &format!("/users/{}", t.user_id),
            Some(json!({ "role": "owner" })),
        )
        .await;
    assert_eq!(
        reply.status,
        StatusCode::FORBIDDEN,
        "checked before the body"
    );
}

#[tokio::test]
async fn a_locked_admin_session_can_no_longer_act() {
    let t = team().await;
    let eva_id = t.env.add_user("eva", "eva@example.com", Role::Admin).await;
    let eva = t.env.session_for("eva").await;
    t.admin(Method::POST, &format!("/users/{eva_id}/lock"), None)
        .await;
    let reply = t
        .env
        .as_user(
            &eva,
            Method::POST,
            &format!("/users/{}/lock", t.admin_id),
            None,
        )
        .await;
    assert_eq!(reply.status, StatusCode::UNAUTHORIZED);
    let me = t.admin(Method::GET, "/session", None).await;
    assert_eq!(me.status, StatusCode::OK, "anna stays");
}

#[tokio::test]
async fn reset_mfa_removes_apps_and_codes() {
    let t = team().await;
    let path = format!("/users/{}/reset-mfa", t.user_id);
    let reply = t.admin(Method::POST, &path, None).await;
    assert_eq!(
        (reply.status, reply.error().as_str()),
        (StatusCode::CONFLICT, "mfa_disabled")
    );
    t.env.add_device(&t.user_id, "Phone").await;
    t.env.add_device(&t.user_id, "Tablet").await;
    let user_id = t.user_id.clone();
    t.env
        .sql(move |conn| {
            conn.execute(
                "INSERT INTO recovery_codes (user_id, code_hash) VALUES (?1, zeroblob(32))",
                [&user_id],
            )
        })
        .await;
    let reply = t.admin(Method::POST, &path, None).await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    assert_eq!(reply.json()["user"]["mfa"], false);
    assert_eq!(t.env.count("totp_devices").await, 0);
    assert_eq!(t.env.count("recovery_codes").await, 0);
    // The session stays; the password alone signs in now.
    let session = t.env.as_user(&t.user, Method::GET, "/session", None).await;
    assert_eq!(session.status, StatusCode::OK);
    let reply = t.env.login("max", PASSWORD).await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    let rows = audit_rows(&t.env, "users.mfa_reset").await;
    assert!(rows[0].1.contains(r#""devices":2"#), "{}", rows[0].1);
}

#[tokio::test]
async fn removing_a_user_deletes_the_account() {
    let t = team().await;
    let reply = t
        .admin(Method::DELETE, &format!("/users/{}", t.user_id), None)
        .await;
    assert_eq!(reply.status, StatusCode::NO_CONTENT, "{}", reply.body);
    assert_eq!(
        t.env
            .as_user(&t.user, Method::GET, "/session", None)
            .await
            .status,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(t.env.count("users").await, 1);
    assert_eq!(
        t.admin(Method::DELETE, &format!("/users/{}", t.user_id), None)
            .await
            .status,
        StatusCode::NOT_FOUND
    );
    let rows = audit_rows(&t.env, "users.remove").await;
    assert_eq!(
        rows,
        [(
            "users.remove".into(),
            r#"{"email":"max@example.com","name":"Max Example","role":"user"}"#.into()
        )]
    );
}

#[tokio::test]
async fn bulk_actions_change_what_fits() {
    let t = team().await;
    let eva = t.env.add_user("eva", "eva@example.com", Role::User).await;
    let created = t
        .admin(
            Method::POST,
            "/invites",
            Some(json!({ "emails": ["jonas@example.com"], "role": "user" })),
        )
        .await
        .json();
    let invite = created["invites"][0]["id"].as_str().unwrap().to_owned();
    let bulk = async |body: Value| t.admin(Method::POST, "/users/bulk", Some(body)).await;

    let reply = bulk(json!({ "action": "role", "ids": [eva] })).await;
    assert_eq!(
        reply.json()["fields"],
        json!({ "role": { "code": "required" } })
    );
    let reply = bulk(json!({ "action": "lock", "ids": [] })).await;
    assert_eq!(
        reply.json()["fields"],
        json!({ "ids": { "code": "required" } })
    );

    let reply = bulk(json!({ "action": "role", "role": "admin", "ids": [eva, invite, "x"] })).await;
    let mut changed: Vec<String> = serde_json::from_value(reply.json()["changed"].clone()).unwrap();
    changed.sort();
    let mut expected = vec![eva.clone(), invite.clone()];
    expected.sort();
    assert_eq!(changed, expected);
    let reply = bulk(json!({ "action": "lock", "ids": [eva, t.user_id, invite] })).await;
    let mut changed: Vec<String> = serde_json::from_value(reply.json()["changed"].clone()).unwrap();
    changed.sort();
    let mut expected = vec![eva.clone(), t.user_id.clone()];
    expected.sort();
    assert_eq!(changed, expected, "an invitation is not locked");
    assert_eq!(
        t.env
            .as_user(&t.user, Method::GET, "/session", None)
            .await
            .status,
        StatusCode::UNAUTHORIZED
    );
    let reply = bulk(json!({ "action": "remove", "ids": [eva, invite] })).await;
    assert_eq!(reply.json()["changed"].as_array().unwrap().len(), 2);
    assert_eq!(t.env.count("invitations").await, 0);
    assert_eq!(t.env.count("users").await, 2);
}

#[tokio::test]
async fn invitations_change_role_extend_and_can_be_requested_again() {
    let t = team().await;
    let created = t
        .admin(
            Method::POST,
            "/invites",
            Some(json!({ "emails": ["jonas@example.com"], "role": "user" })),
        )
        .await
        .json();
    let id = created["invites"][0]["id"].as_str().unwrap().to_owned();
    let link = t
        .admin(Method::GET, &format!("/invites/{id}/link"), None)
        .await
        .json();
    let token = link["url"]
        .as_str()
        .unwrap()
        .rsplit('/')
        .next()
        .unwrap()
        .to_owned();

    let reply = t
        .admin(
            Method::PATCH,
            &format!("/invites/{id}"),
            Some(json!({ "role": "admin" })),
        )
        .await;
    assert_eq!(reply.json()["invite"]["role"], "admin", "{}", reply.body);

    let request = async || {
        t.env
            .request(
                Method::POST,
                &format!("/invites/{token}/request"),
                None,
                &[],
            )
            .await
    };
    let reply = request().await;
    assert_eq!(
        (reply.status, reply.error().as_str()),
        (StatusCode::UNPROCESSABLE_ENTITY, "invite_valid")
    );
    t.env
        .sql(|conn| {
            conn.execute(
                "UPDATE invitations SET expires_at = '2000-01-01T00:00:00.000Z'",
                [],
            )
        })
        .await;
    for _ in 0..2 {
        assert_eq!(request().await.status, StatusCode::ACCEPTED);
    }
    let rows = audit_rows(&t.env, "users.invite_request").await;
    assert_eq!(rows.len(), 1, "once per 24 hours");
    let list = t.admin(Method::GET, "/invites", None).await.json();
    assert!(list["invites"][0]["requested_at"].is_string());
    assert_eq!(list["invites"][0]["expired"], true);
    assert_eq!(
        t.env
            .request(Method::POST, "/invites/unknown/request", None, &[])
            .await
            .status,
        StatusCode::NOT_FOUND
    );

    let reply = t
        .admin(Method::POST, &format!("/invites/{id}/renew"), None)
        .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    assert_eq!(reply.json()["invite"]["expired"], false);
    assert!(reply.json()["invite"]["mail_sent_at"].is_null(), "no mail");
    let show = t
        .env
        .request(Method::GET, &format!("/invites/{token}"), None, &[])
        .await;
    assert_eq!(show.json()["expired"], false, "the same link works again");
    assert_eq!(show.json()["role"], "admin");

    let actions = t.env.audit_actions().await;
    for action in [
        "users.invite_role",
        "users.invite_request",
        "users.invite_renew",
    ] {
        assert!(actions.iter().any(|a| a == action), "{action}: {actions:?}");
    }
}

/// The page of the audit log for `query` (`?…`).
async fn audit_page(t: &Team, query: &str) -> Value {
    let reply = t
        .admin(Method::GET, &format!("/admin/audit{query}"), None)
        .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    reply.json()
}

#[tokio::test]
async fn audit_log_pages_filters_and_searches() {
    let t = team().await;
    t.admin(Method::POST, &format!("/users/{}/lock", t.user_id), None)
        .await;
    t.admin(
        Method::PUT,
        "/admin/org",
        Some(json!({ "name": "Example Org" })),
    )
    .await;

    let page = audit_page(&t, "").await;
    let acts: Vec<&str> = page["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["act"].as_str().unwrap())
        .collect();
    assert_eq!(
        acts,
        [
            " changed the organization",
            " suspended Max Example",
            " signed in",
            " signed in"
        ]
    );
    assert_eq!(
        page["counts"],
        json!({ "all": 4, "login": 2, "users": 1, "apps": 0, "settings": 1, "system": 0 })
    );
    let first = &page["entries"][1];
    assert_eq!(first["cat"], "users");
    assert_eq!(first["icon"], "ti-lock");
    assert_eq!(first["who"], "Max Example");
    assert_eq!(first["warn"], false);
    assert_eq!(
        page["entries"][2]["meta"], "203.0.x.x",
        "sign-ins show the IP"
    );
    assert!(page["next"].is_null());

    let users = audit_page(&t, "?cat=users").await;
    assert_eq!(users["entries"].as_array().unwrap().len(), 1);
    assert_eq!(users["counts"]["all"], 4, "counts ignore the category");

    // Paging with `before`.
    let one = audit_page(&t, "?limit=1").await;
    let next = one["next"].as_i64().unwrap();
    let two = audit_page(&t, &format!("?limit=1&before={next}")).await;
    assert_eq!(two["entries"][0]["act"], " suspended Max Example");

    // The search matches the shown text.
    let found = audit_page(&t, "?q=SUSPENDED").await;
    assert_eq!(found["entries"].as_array().unwrap().len(), 1);
    assert_eq!(found["counts"]["all"], 1);
    let found = audit_page(&t, "?q=sign-in&cat=login").await;
    assert_eq!(
        found["entries"].as_array().unwrap().len(),
        2,
        "category label"
    );
    // A search counts towards a limit, so only from the web app.
    let foreign = t
        .env
        .without_csrf(
            Method::GET,
            "/admin/audit?q=x",
            json!(null),
            &[(session::COOKIE, &t.admin)],
        )
        .await;
    assert_eq!(foreign.error(), "csrf");

    for bad in ["?cat=Sign-in", "?from=yesterday", "?limit=x"] {
        let reply = t
            .admin(Method::GET, &format!("/admin/audit{bad}"), None)
            .await;
        assert_eq!(reply.status, StatusCode::BAD_REQUEST, "{bad}");
    }
    let future = audit_page(&t, "?from=2999-01-01T00:00:00Z").await;
    assert_eq!(future["counts"]["all"], 0);
}

#[tokio::test]
async fn audit_texts_follow_the_administrators_language() {
    let t = team().await;
    t.admin(Method::POST, &format!("/users/{}/lock", t.user_id), None)
        .await;
    let reply = t
        .admin(
            Method::PUT,
            "/settings",
            Some(json!({ "version": 0, "settings": { "lang": "de" } })),
        )
        .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    let page = audit_page(&t, "").await;
    assert_eq!(page["entries"][0]["act"], " hat Max Example gesperrt");
    assert_eq!(page["entries"][1]["act"], " hat sich angemeldet");
    let found = audit_page(&t, "?q=gesperrt").await;
    assert_eq!(
        found["counts"]["all"], 1,
        "the search uses the same language"
    );
    let found = audit_page(&t, "?q=Anmeldung").await;
    assert_eq!(found["counts"]["login"], 2);
}

#[tokio::test]
async fn audit_export_is_csv_recorded_and_limited() {
    let t = team().await;
    t.admin(Method::POST, &format!("/users/{}/lock", t.user_id), None)
        .await;
    let reply = t
        .admin(Method::POST, "/admin/audit/export?cat=users", None)
        .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    assert_eq!(
        reply.header("content-type"),
        Some("text/csv; charset=utf-8")
    );
    assert!(
        reply
            .header("content-disposition")
            .unwrap()
            .starts_with("attachment; filename=\"audit-log-")
    );
    assert_eq!(reply.header("cache-control"), Some("no-store"));
    let lines: Vec<&str> = reply.body.split("\r\n").collect();
    assert_eq!(
        lines[0],
        "\u{feff}\"Time\";\"Category\";\"Person\";\"Action\";\"Details\""
    );
    assert!(
        lines[1].ends_with(r#";"Users";"Max Example";"suspended Max Example";"""#),
        "{}",
        lines[1]
    );
    assert_eq!(lines.len(), 3, "one entry and the final line break");
    let rows = audit_rows(&t.env, "audit.export").await;
    assert_eq!(rows, [("audit.export".into(), r#"{"count":1}"#.into())]);

    // Ten exports per administrator, then a pause.
    for _ in 1..10 {
        let reply = t.admin(Method::POST, "/admin/audit/export", None).await;
        assert_eq!(reply.status, StatusCode::OK);
    }
    let reply = t.admin(Method::POST, "/admin/audit/export", None).await;
    assert_eq!(reply.status, StatusCode::TOO_MANY_REQUESTS);
    assert!(reply.header("retry-after").is_some());
}

#[tokio::test]
async fn audit_retention_is_one_of_the_choices() {
    let t = team().await;
    let path = "/admin/audit/retention";
    assert_eq!(
        t.admin(Method::GET, path, None).await.json(),
        json!({ "days": 90 })
    );
    let reply = t.admin(Method::PUT, path, Some(json!({ "days": 7 }))).await;
    assert_eq!(
        reply.json()["fields"],
        json!({ "days": { "code": "invalid" } })
    );
    for _ in 0..2 {
        let reply = t
            .admin(Method::PUT, path, Some(json!({ "days": 365 })))
            .await;
        assert_eq!(reply.json(), json!({ "days": 365 }));
    }
    assert_eq!(
        t.admin(Method::GET, path, None).await.json(),
        json!({ "days": 365 })
    );
    let rows = audit_rows(&t.env, "audit.retention").await;
    assert_eq!(
        rows,
        [(
            "audit.retention".into(),
            r#"{"days":365,"previous":90}"#.into()
        )],
        "only a change is recorded"
    );
    let page = audit_page(&t, "?cat=settings").await;
    assert_eq!(
        page["entries"][0]["act"],
        " changed the audit log retention to 1 year"
    );
}

#[tokio::test]
async fn the_organization_names_the_hub() {
    let t = team().await;
    let session = t.admin(Method::GET, "/session", None).await.json();
    assert_eq!(session["hub"]["name"], "Roedy Hub");
    let public = t
        .env
        .request(Method::GET, "/public/config", None, &[])
        .await;
    assert_eq!(
        public.json(),
        json!({
            "name": "Roedy Hub",
            "org": {
                "name": null, "website": null, "help": null, "email": null,
                "imprint": null, "privacy": null, "terms": null, "source": null,
            },
            "branding": {
                "kind": "standard", "letter": "", "icon": "ti-mountain", "image": null,
                "fit": "padded", "plate": "accent", "tilt": true, "color": "apricot",
                "userAccent": true,
            },
            "setup_required": false,
        })
    );

    let reply = t
        .admin(
            Method::PUT,
            "/admin/org",
            Some(json!({
                "name": " Example: Org ", "website": "https://example.com",
                "email": "help@example.com", "privacy": "javascript:alert(1)",
            })),
        )
        .await;
    assert_eq!(
        reply.json()["fields"],
        json!({ "privacy": { "code": "invalid" } })
    );
    let reply = t
        .admin(
            Method::PUT,
            "/admin/org",
            Some(json!({
                "name": " Example: Org ", "website": "https://example.com",
                "email": "help@example.com", "privacy": "",
            })),
        )
        .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    assert_eq!(reply.json()["name"], "Example: Org");
    let reply = t
        .admin(Method::PUT, "/admin/org", Some(json!({ "fax": "0" })))
        .await;
    assert_eq!(reply.status, StatusCode::BAD_REQUEST);

    for session in [&t.admin, &t.user] {
        let me = t.env.as_user(session, Method::GET, "/session", None).await;
        assert_eq!(me.json()["hub"]["name"], "Example: Org");
    }
    let public = t
        .env
        .request(Method::GET, "/public/config", None, &[])
        .await;
    assert_eq!(public.json()["name"], "Example: Org");
    assert_eq!(public.json()["org"]["email"], "help@example.com");
    // The issuer of authenticator apps, without the colon of the label.
    let enroll = t
        .env
        .as_user(&t.user, Method::POST, "/auth/totp/devices/enroll", None)
        .await;
    assert_eq!(enroll.status, StatusCode::OK, "{}", enroll.body);
    let uri = enroll.json()["uri"].as_str().unwrap().to_owned();
    assert!(uri.contains("issuer=Example%20%20Org"), "{uri}");

    let rows = audit_rows(&t.env, "org.update").await;
    assert_eq!(
        rows,
        [(
            "org.update".into(),
            r#"{"fields":["name","website","email"]}"#.into()
        )]
    );
    // Deleting the name returns to "Roedy Hub".
    t.admin(Method::PUT, "/admin/org", Some(json!({ "name": null })))
        .await;
    let me = t.admin(Method::GET, "/session", None).await.json();
    assert_eq!(me["hub"]["name"], "Roedy Hub");
}

fn svg_data_url(svg: &str) -> String {
    use base64::Engine as _;
    format!(
        "data:image/svg+xml;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(svg)
    )
}

/// Branding: logo, plate and colours, the logo as data URL validated and
/// sanitized, `public/config`, mail logo, manifest and mails from the same
/// source, from the database after a restart.
#[tokio::test]
async fn branding_is_checked_saved_drawn_and_public() {
    let t = team().await;
    let put = async |body: Value| t.admin(Method::PUT, "/admin/branding", Some(body)).await;
    let brand0 = json!({
        "kind": "standard", "letter": "", "icon": "ti-mountain", "image": null,
        "imageName": "", "imageMeta": "", "fit": "padded", "plate": "accent",
        "tilt": true, "color": "apricot", "userAccent": true,
    });
    let get = t.admin(Method::GET, "/admin/branding", None).await;
    assert_eq!(get.json(), brand0, "{}", get.body);
    let (_, _, standard_logo) = t.env.file("/branding/mail-logo.png").await;
    let standard_logos = t.env.state.brand().logos.clone().unwrap();

    // Validated per field; unknown fields are 400.
    let invalid = put(json!({ "letter": "abc", "color": "Grass", "icon": "ti-mail" })).await;
    assert_eq!(
        invalid.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "{}",
        invalid.body
    );
    assert_eq!(
        invalid.json()["fields"],
        json!({
            "letter": { "code": "too_long" }, "color": { "code": "invalid" },
            "icon": { "code": "invalid" },
        })
    );
    for unknown in [json!({ "name": "Post" }), json!({ "loginTitle": "Hi" })] {
        assert_eq!(put(unknown).await.status, StatusCode::BAD_REQUEST);
    }
    let required = put(json!({ "kind": "image" })).await;
    assert_eq!(required.json()["fields"]["image"]["code"], "required");
    for (url, code) in [
        ("data:image/gif;base64,R0lGODlh".to_owned(), "unsupported"),
        (
            svg_data_url(r#"<svg xmlns="http://www.w3.org/2000/svg"><text>Post</text></svg>"#),
            "svg_text",
        ),
        ("/branding/image/foreign.png".to_owned(), "invalid"),
    ] {
        let reply = put(json!({ "kind": "image", "image": url })).await;
        assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY, "{code}");
        assert_eq!(reply.json()["fields"]["image"]["code"], code);
    }
    assert!(audit_rows(&t.env, "branding.%").await.is_empty());

    // An SVG with a script is stored sanitized.
    let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 10 10"><script>alert(1)</script><rect width="10" height="10" fill="#ff0000" onclick="alert(2)"/></svg>"##;
    let saved = put(json!({
        "kind": "image", "image": svg_data_url(svg), "imageName": "logo.svg",
        "imageMeta": "SVG · 1 KB", "fit": "fill", "plate": "dark", "color": "sage",
        "userAccent": false,
    }))
    .await;
    assert_eq!(saved.status, StatusCode::OK, "{}", saved.body);
    let saved = saved.json();
    let image = saved["image"].as_str().unwrap().to_owned();
    assert!(
        image.starts_with("/branding/image/") && image.ends_with(".svg"),
        "{image}"
    );
    assert_eq!(saved["imageName"], "logo.svg");
    let (status, headers, body) = t.env.file(&image).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers["content-type"], "image/svg+xml");
    assert_eq!(
        headers["cache-control"],
        "public, max-age=31536000, immutable"
    );
    assert!(
        headers["content-security-policy"]
            .to_str()
            .unwrap()
            .starts_with("default-src 'none'")
    );
    let body = String::from_utf8(body).unwrap();
    assert!(!body.contains("alert") && body.contains("<path"), "{body}");
    assert_eq!(
        t.env.file("/branding/image/0000.svg").await.0,
        StatusCode::NOT_FOUND
    );

    // public/config without details of the file, mail logo drawn anew.
    let config = t
        .env
        .request(Method::GET, "/public/config", None, &[])
        .await
        .json();
    assert_eq!(config["branding"]["image"], image);
    assert_eq!(config["branding"]["userAccent"], false);
    assert_eq!(config["branding"]["plate"], "dark");
    assert!(config["branding"].get("imageName").is_none(), "{config}");
    let (status, _, logo) = t.env.file("/branding/mail-logo.png").await;
    assert_eq!(status, StatusCode::OK);
    assert_ne!(logo, standard_logo);
    assert_eq!(logo, t.env.state.brand().mail_logo().unwrap().bytes);

    // Web manifest: name, colours and new icons with version.
    let (status, _, manifest) = t.env.file("/manifest.webmanifest").await;
    assert_eq!(status, StatusCode::OK);
    let manifest: Value = serde_json::from_slice(&manifest).unwrap();
    assert_eq!(manifest["name"], "Roedy Hub");
    assert_eq!(manifest["background_color"], "#2b221c");
    let brand = t.env.state.brand();
    let logos = brand.logos.as_ref().unwrap();
    assert_eq!(
        manifest["icons"][1]["src"],
        format!("/branding/app-icon.png?v={}", logos.icon.version)
    );
    assert_ne!(logos.icon, standard_logos.icon);
    for (path, png) in [
        ("/branding/app-icon.png", &logos.icon),
        ("/branding/app-icon-192.png", &logos.small),
        ("/branding/app-icon-maskable.png", &logos.maskable),
        ("/branding/apple-touch-icon.png", &logos.touch),
    ] {
        let (status, _, bytes) = t.env.file(path).await;
        assert_eq!(
            (status, bytes.as_slice()),
            (StatusCode::OK, png.bytes.as_ref()),
            "{path}"
        );
    }
    // The mails take the plate, the accent and the logo with its version.
    assert_eq!(brand.accent.base, "#94c48d");

    // Previous URL: the logo stays; without a change no entry.
    let kept = put(json!({ "image": image, "tilt": false })).await.json();
    assert_eq!(kept["image"], image);
    assert_eq!(kept["imageName"], "logo.svg");
    let same = put(json!({ "tilt": false })).await;
    assert_eq!(same.status, StatusCode::OK);

    // After a restart from the database, with the same logo.
    let restarted = roedy_hub_server::app::AppState::new(
        t.env.state.db.clone(),
        std::sync::Arc::new(roedy_hub_server::web::static_files::WebAssets::None),
        &roedy_hub_server::config::Config::default(),
        url::Url::parse("https://app.example.com").unwrap(),
        roedy_hub_server::crypto::SecretKey::random().unwrap(),
    )
    .unwrap();
    let instance = restarted.instance.clone();
    db::call(&restarted.db, move |conn| instance.load(conn))
        .await
        .unwrap();
    assert_eq!(
        restarted.instance.branding().image_path(),
        Some(image.clone())
    );
    assert_eq!(
        restarted.brand().mail_logo(),
        t.env.state.brand().mail_logo()
    );

    // Reset to the defaults: logo gone, also from `assets`.
    let reset = put(brand0.clone()).await.json();
    assert_eq!(reset, brand0);
    assert_eq!(t.env.file(&image).await.0, StatusCode::NOT_FOUND);
    assert_eq!(t.env.file("/branding/mail-logo.png").await.2, standard_logo);
    assert_eq!(t.env.count("assets").await, 0);

    let rows = audit_rows(&t.env, "branding.update").await;
    assert_eq!(rows.len(), 3, "{rows:?}");
    assert!(rows[1].1.contains(r#""fields":["tilt"]"#), "{}", rows[1].1);
}

/// The initial of the logo is the hub's name: a new organization name draws
/// it anew.
#[tokio::test]
async fn the_initial_follows_the_organization_name() {
    let t = team().await;
    t.admin(
        Method::PUT,
        "/admin/branding",
        Some(json!({ "kind": "initial" })),
    )
    .await;
    let hub = t.env.file("/branding/mail-logo.png").await.2;
    t.admin(
        Method::PUT,
        "/admin/org",
        Some(json!({ "name": "Example Org" })),
    )
    .await;
    let org = t.env.file("/branding/mail-logo.png").await.2;
    assert_ne!(hub, org, "R for Roedy Hub, E for Example Org");
    t.admin(
        Method::PUT,
        "/admin/org",
        Some(json!({ "website": "https://example.com" })),
    )
    .await;
    assert_eq!(t.env.file("/branding/mail-logo.png").await.2, org);
    // With a letter of its own, the name does not matter.
    t.admin(
        Method::PUT,
        "/admin/branding",
        Some(json!({ "letter": "Q" })),
    )
    .await;
    let letter = t.env.file("/branding/mail-logo.png").await.2;
    t.admin(Method::PUT, "/admin/org", Some(json!({ "name": "Other" })))
        .await;
    assert_eq!(t.env.file("/branding/mail-logo.png").await.2, letter);
}

#[tokio::test]
async fn public_config_reports_a_pending_setup() {
    let env = env().await;
    let reply = env.request(Method::GET, "/public/config", None, &[]).await;
    assert_eq!(reply.json()["setup_required"], true);
}

/// `reset-admin` on the hub's database: the administrator signs in with the
/// new password, without the second factor and after being locked.
#[tokio::test]
async fn reset_admin_restores_the_sign_in() {
    let t = team().await;
    t.env.add_device(&t.admin_id, "Phone").await;
    let admin_id = t.admin_id.clone();
    t.env
        .sql(move |conn| {
            conn.execute(
                "UPDATE users SET status = 'locked' WHERE id = ?1",
                [&admin_id],
            )
        })
        .await;
    let password = reset_admin::generate_password().unwrap();
    let hash = t.env.state.hasher.hash(&password).await.unwrap();
    let secret = Secret::Password(password.clone());
    let reset = db::call(&t.env.state.db, move |conn| {
        let admin = reset_admin::select(conn, None)?;
        reset_admin::reset(conn, &admin, secret, &hash)
    })
    .await
    .unwrap();
    assert_eq!(
        (reset.devices, reset.sessions, reset.unlocked),
        (1, 1, true)
    );
    assert_eq!(
        t.env
            .as_user(&t.admin, Method::GET, "/session", None)
            .await
            .status,
        StatusCode::UNAUTHORIZED,
        "the session is gone, also after unlocking"
    );
    let reply = t.env.login("anna", &password).await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    assert_eq!(reply.json()["status"], "ok", "{}", reply.body);
    let admin = reply.cookie(session::COOKIE).unwrap();
    let page = t
        .env
        .as_user(&admin, Method::GET, "/admin/audit?cat=system", None)
        .await
        .json();
    assert_eq!(page["entries"][0]["act"], " reset access for Max Example");
    assert_eq!(page["entries"][0]["who"], "System");
    assert_eq!(page["entries"][0]["warn"], true);
}
