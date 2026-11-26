//! Administration › Email: the sending account of the system mails.
//!
//! - `GET admin/system-mail`: the account without the secret (`has_secret`),
//!   plus the status of the last check (`none`, `unchecked`, `ok`, `err`),
//!   "Last checked" and the account's identities.
//! - `PUT admin/system-mail`: changes the given fields; the hub checks a new
//!   connection right afterwards and responds with the result.
//! - `POST admin/system-mail/test`: checks the draft (missing fields from the
//!   stored account) without saving; with `to` a test email then goes to this
//!   address.
//!
//! The server is given by its address (`url`, origin of the JMAP server, e.g.
//! `https://mail.example.com`); a new address drops the stored secret, which
//! never goes to an address it was not entered for.
//!
//! Secrets appear in no response, no log and no audit entry.
//!
//! Taken over from Roedy Mail (`crates/server/src/api/system_mail.rs`)
//! without the template preview; the server is an address instead of one
//! from Mail's registry.

use std::time::Duration;

use std::net::Ipv4Addr;

use axum::{
    Json, Router,
    extract::State,
    http::StatusCode,
    routing::{get, post},
};
use jiff::Timestamp;
use roedy_jmap as jmap;
use serde::{Deserialize, Serialize};
use serde_json::json;
use url::{Host, Url};

use super::{
    ApiError, FieldErrors, JsonBody,
    error::retry_secs,
    invites::admin_holds,
    me::{self, is_invisible},
    nullable,
};
use crate::{
    app::AppState,
    auth::{AdminUser, AuthUser, email},
    client_ip::ClientIp,
    crypto::SecretKey,
    db::{
        self,
        repo::audit_log::{self, Category, Entry},
    },
    mailer::{
        Account, MailError,
        delivery::Delivery,
        format::Format,
        system_mail::{self, Auth, Credentials, Identity, Outcome, SystemMail},
        templates::SystemTest,
    },
    rate_limit::{Key, Policy},
    settings,
};

/// Reply-to address and recipient of the test email, in characters.
const MAX_REPLY_TO: usize = 254;
/// Login name of the sending account (at most 254 characters, like an address).
const MAX_USER: usize = 254;
/// JMAP IDs have at most 255 characters (RFC 8620, 1.2).
const MAX_IDENTITY: usize = 255;

/// Test emails per administrator: ten in ten minutes – a hijacked session
/// should not be able to write to any number of addresses through the system
/// mail.
const TEST_MAIL: Policy = Policy::rate("system_mail.test", 10, Duration::from_secs(10 * 60));
/// Longest secret (app password, token).
const MAX_SECRET: usize = 1024;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/admin/system-mail", get(show).put(update))
        .route("/admin/system-mail/test", post(test))
}

/// Response of `GET` and `PUT`.
#[derive(Debug, Serialize)]
struct View {
    /// Address of the JMAP server; `null` without an account.
    url: Option<String>,
    auth: &'static str,
    user: String,
    has_secret: bool,
    identity_id: Option<String>,
    sender_name: Option<String>,
    reply_to: Option<String>,
    keep_sent: bool,
    /// `none`, `unchecked`, `ok` or `err`.
    status: &'static str,
    reason: Option<String>,
    /// With `no_mailbox` the missing mailbox (`drafts`, `sent`).
    #[serde(skip_serializing_if = "Option::is_none")]
    role: Option<String>,
    checked_at: Option<String>,
    identities: Vec<Identity>,
}

impl View {
    fn load(conn: &rusqlite::Connection) -> anyhow::Result<Self> {
        let Some(mail) = system_mail::load(conn)? else {
            return Ok(Self::empty());
        };
        let on_server = mail.base_url().is_some();
        let has_secret = mail.secret_enc().is_some();
        let (status, reason, role, checked_at, identities) = match (on_server, mail.check) {
            (false, _) => ("none", None, None, None, Vec::new()),
            (true, None) => ("unchecked", None, None, None, Vec::new()),
            (true, Some(check)) if check.ok => ("ok", None, None, Some(check.at), check.identities),
            (true, Some(check)) => (
                "err",
                check.reason,
                check.role,
                Some(check.at),
                check.identities,
            ),
        };
        Ok(Self {
            url: on_server.then_some(mail.url),
            auth: mail.auth.as_str(),
            user: mail.user,
            has_secret,
            identity_id: mail.identity_id,
            sender_name: mail.sender_name,
            reply_to: mail.reply_to,
            keep_sent: mail.keep_sent,
            status,
            reason,
            role,
            checked_at,
            identities,
        })
    }

    /// Without an account: the defaults with empty credentials.
    fn empty() -> Self {
        Self {
            url: None,
            auth: Auth::Password.as_str(),
            user: String::new(),
            has_secret: false,
            identity_id: None,
            sender_name: None,
            reply_to: None,
            keep_sent: true,
            status: "none",
            reason: None,
            role: None,
            checked_at: None,
            identities: Vec::new(),
        }
    }
}

async fn show(State(state): State<AppState>, _: AdminUser) -> Result<Json<View>, ApiError> {
    Ok(Json(db::call(&state.db, |conn| View::load(conn)).await?))
}

/// Body of `PUT` and `test`: the changed fields of the draft; `to` only for
/// the test. No `Debug`, like `Changes`: both hold the secret in plain text.
#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct Body {
    url: Option<String>,
    auth: Option<String>,
    user: Option<String>,
    /// Empty or missing: the stored one stays (like `client_secret`).
    app_password: Option<String>,
    token: Option<String>,
    #[serde(default, deserialize_with = "nullable")]
    identity_id: Option<Option<String>>,
    #[serde(default, deserialize_with = "nullable")]
    sender_name: Option<Option<String>>,
    #[serde(default, deserialize_with = "nullable")]
    reply_to: Option<Option<String>>,
    keep_sent: Option<bool>,
    to: Option<String>,
}

/// Validated changes; `None` = unchanged, for the optional fields
/// `Some(None)` = empty.
#[derive(Default)]
struct Changes {
    /// Origin of the server, checked ([`normalize_url`]).
    url: Option<String>,
    auth: Option<Auth>,
    user: Option<String>,
    app_password: Option<String>,
    token: Option<String>,
    identity_id: Option<Option<String>>,
    sender_name: Option<Option<String>>,
    reply_to: Option<Option<String>>,
    keep_sent: Option<bool>,
}

/// By hand, so the secrets never appear in any output.
impl std::fmt::Debug for Changes {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let hidden = |secret: &Option<String>| secret.as_ref().map(|_| "…");
        f.debug_struct("Changes")
            .field("url", &self.url)
            .field("auth", &self.auth)
            .field("user", &self.user)
            .field("app_password", &hidden(&self.app_password))
            .field("token", &hidden(&self.token))
            .finish_non_exhaustive()
    }
}

/// User of the sending account: without whitespace and control characters, at
/// most [`MAX_USER`] bytes.
fn is_bad_user(user: &str) -> bool {
    user.len() > MAX_USER || user.chars().any(|c| c.is_whitespace() || c.is_control())
}

/// Identity of the sending account: at most [`MAX_IDENTITY`] bytes without
/// control characters.
fn is_bad_identity(id: &str) -> bool {
    id.len() > MAX_IDENTITY || id.chars().any(char::is_control)
}

/// Empty or missing text becomes `None`, otherwise trimmed.
fn optional_text(value: Option<Option<String>>) -> Option<Option<String>> {
    value.map(clean_optional)
}

impl Body {
    /// Checks the fields one by one; [`apply`] checks required
    /// fields on the result. Returns the changes and `to`.
    fn checked(self) -> Result<(Changes, Option<String>), ApiError> {
        let mut errors = FieldErrors::default();
        let url = match self.url.as_deref().map(normalize_url) {
            None => None,
            Some(Ok(url)) => Some(url.origin().ascii_serialization()),
            Some(Err(code)) => {
                errors.code("url", code);
                None
            }
        };
        let auth = self.auth.as_deref().map(Auth::parse);
        if auth == Some(None) {
            errors.code("auth", "invalid");
        }
        let user = self.user.map(|user| user.trim().to_owned());
        if user.as_deref().is_some_and(is_bad_user) {
            errors.code("user", "invalid");
        }
        // Do not trim secrets: they may start with spaces.
        let app_password = self.app_password.filter(|value| !value.is_empty());
        check_client_field(&mut errors, "app_password", app_password.as_deref());
        let token = self.token.map(|value| value.trim().to_owned());
        let token = token.filter(|value| !value.is_empty());
        check_client_field(&mut errors, "token", token.as_deref());
        let identity_id = optional_text(self.identity_id);
        if identity_id
            .as_ref()
            .is_some_and(|id| id.as_deref().is_some_and(is_bad_identity))
        {
            errors.code("identity_id", "invalid");
        }
        let sender_name = optional_text(self.sender_name);
        if let Some(Some(name)) = &sender_name {
            if name.chars().count() > me::MAX_NAME {
                errors.code("sender_name", "too_long");
            } else if name.chars().any(|c| c.is_control() || is_invisible(c)) {
                errors.code("sender_name", "invalid");
            }
        }
        let reply_to = optional_text(self.reply_to);
        if let Some(Some(address)) = &reply_to {
            if address.chars().count() > MAX_REPLY_TO {
                errors.code("reply_to", "too_long");
            } else if !email::is_valid(address) {
                errors.code("reply_to", "invalid");
            }
        }
        let to = self.to.map(|to| to.trim().to_owned());
        if let Some(to) = &to {
            if to.chars().count() > MAX_REPLY_TO {
                errors.code("to", "too_long");
            } else {
                me::check_email(&mut errors, "to", to);
            }
        }
        if !errors.is_empty() {
            return Err(errors.into_error());
        }
        let changes = Changes {
            url,
            auth: auth.flatten(),
            user,
            app_password,
            token,
            identity_id,
            sender_name,
            reply_to,
            keep_sent: self.keep_sent,
        };
        Ok((changes, to))
    }
}

/// Result of [`apply`].
struct Applied {
    mail: SystemMail,
    base_url: Url,
    /// Names of the changed fields for the audit log (for the secret only the name).
    changed: Vec<&'static str>,
    /// Whether server, sign-in method, user or secret differ from the stored ones.
    new_connection: bool,
}

/// Applies the changes to the stored account (without saving) and checks the
/// result: a server address, user with password, secret of the chosen
/// sign-in method.
fn apply(
    secrets: &SecretKey,
    current: Option<&SystemMail>,
    changes: Changes,
) -> anyhow::Result<Result<Applied, FieldErrors>> {
    let mut errors = FieldErrors::default();
    let mut next = current.cloned().unwrap_or_default();
    if let Some(url) = changes.url {
        if url != next.url {
            // The stored secret belongs to its server; another one does not
            // get it.
            next.forget_secret();
        }
        next.url = url;
    }
    let base_url = next.base_url();
    if base_url.is_none() {
        errors.code("url", "required");
    }
    if let Some(auth) = changes.auth {
        next.set_auth(auth);
    }
    if let Some(user) = changes.user {
        next.user = user;
    }
    let secret = match next.auth {
        Auth::Password => changes.app_password,
        Auth::Token => changes.token,
    };
    // The same secret again changes nothing (the values are compared); an
    // unreadable stored one counts as different.
    if let Some(secret) = secret
        && next.secret(secrets).ok().flatten().as_deref() != Some(secret.as_str())
    {
        next.set_secret(secrets, next.auth, &secret)?;
    }
    if next.secret_enc().is_none() {
        errors.code(next.auth.secret_field(), "required");
    }
    if next.auth == Auth::Password && next.user.is_empty() {
        errors.code("user", "required");
    }
    if let Some(identity_id) = changes.identity_id {
        next.identity_id = identity_id;
    }
    if let Some(sender_name) = changes.sender_name {
        next.sender_name = sender_name;
    }
    if let Some(reply_to) = changes.reply_to {
        next.reply_to = reply_to;
    }
    if let Some(keep_sent) = changes.keep_sent {
        next.keep_sent = keep_sent;
    }
    let Some(base_url) = base_url.filter(|_| errors.is_empty()) else {
        return Ok(Err(errors));
    };
    let before = current.cloned().unwrap_or_default();
    let mut changed = Vec::new();
    for (field, differs) in [
        ("url", before.url != next.url),
        ("auth", before.auth != next.auth),
        ("user", before.user != next.user),
        (
            next.auth.secret_field(),
            next.secret_enc().is_some() && before.secret_enc() != next.secret_enc(),
        ),
        ("identity_id", before.identity_id != next.identity_id),
        ("sender_name", before.sender_name != next.sender_name),
        ("reply_to", before.reply_to != next.reply_to),
        ("keep_sent", before.keep_sent != next.keep_sent),
    ] {
        if differs {
            changed.push(field);
        }
    }
    let new_connection = current.is_none_or(|current| current.credentials() != next.credentials());
    if new_connection {
        next.check = None;
    }
    Ok(Ok(Applied {
        mail: next,
        base_url,
        changed,
        new_connection,
    }))
}

/// "Save" on the Email page. With a new connection the hub checks it
/// afterwards itself (like "Send test email"); the response
/// is the stored account with the status.
async fn update(
    State(state): State<AppState>,
    AdminUser(user): AdminUser,
    ip: ClientIp,
    JsonBody(body): JsonBody<Body>,
) -> Result<Json<View>, ApiError> {
    if body.to.is_some() {
        return Err(ApiError::BadRequest("unknown field `to`".into()));
    }
    let (changes, _) = body.checked()?;
    let secrets = state.secrets.clone();
    let saved = db::call(&state.db, move |conn| {
        let tx = conn.transaction()?;
        // The session and the role as of now, not as of the request.
        if let Err(error) = admin_holds(&tx, &user)? {
            return Ok(Err(error));
        }
        let current = system_mail::load(&tx)?;
        let applied = match apply(&secrets, current.as_ref(), changes)? {
            Ok(applied) => applied,
            Err(errors) => return Ok(Err(errors.into_error())),
        };
        if !applied.changed.is_empty() {
            system_mail::save(&tx, &applied.mail)?;
            audit_log::insert(
                &tx,
                &audit_entry(Category::Settings, "system_mail.update", &user, ip)
                    .details(json!({ "fields": applied.changed })),
            )?;
            tx.commit()?;
        }
        Ok(Ok(applied))
    })
    .await??;
    if saved.new_connection {
        let account = saved.mail.account(saved.base_url.clone(), &state.secrets)?;
        let outcome = match state.mailer.test(&account).await {
            Ok(identities) => Outcome::Ok(Some(identities.into_iter().map(Into::into).collect())),
            Err(error) => Outcome::failed(&error),
        };
        record(&state, saved.mail.credentials(), outcome).await?;
    }
    Ok(Json(db::call(&state.db, |conn| View::load(conn)).await?))
}

async fn record(
    state: &AppState,
    credentials: Credentials,
    outcome: Outcome,
) -> anyhow::Result<()> {
    db::call(&state.db, move |conn| {
        system_mail::record(conn, &credentials, Timestamp::now(), outcome)
    })
    .await
    .map(drop)
}

/// Error of a test for the caller: `mail_failed` with
/// `reason`; `auth_failed` and `forbidden` 422, `auth_locked` 429 with
/// `retry_after`, otherwise 502; `no_mailbox` names the missing mailbox with
/// `role` (`drafts`, `sent`), the page names it in the text.
pub(super) fn rejection(error: &MailError) -> ApiError {
    let rejection = |status| {
        let rejected = ApiError::rejected(status, "mail_failed").with("reason", error.reason());
        match error.mailbox_role() {
            Some(role) => rejected.with("role", role),
            None => rejected,
        }
    };
    match error {
        MailError::Locked(left) => {
            rejection(StatusCode::TOO_MANY_REQUESTS).with("retry_after", retry_secs(*left))
        }
        MailError::Jmap(jmap::Error::AuthFailed | jmap::Error::Forbidden) => {
            rejection(StatusCode::UNPROCESSABLE_ENTITY)
        }
        MailError::Jmap(_) => rejection(StatusCode::BAD_GATEWAY),
    }
}

/// What a test needs: the draft's account and, if it is the stored one, its
/// connection for the status.
struct Draft {
    account: Account,
    saved: Option<Credentials>,
    /// Language and formats of the administrator for the test email.
    format: Format,
}

/// "Test connection" and "Send" (Test section) with the draft.
async fn test(
    State(state): State<AppState>,
    AdminUser(admin): AdminUser,
    JsonBody(body): JsonBody<Body>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let (changes, to) = body.checked()?;
    let secrets = state.secrets.clone();
    let admin_id = admin.id.clone();
    let draft = db::call(&state.db, move |conn| {
        let current = system_mail::load(conn)?;
        let applied = match apply(&secrets, current.as_ref(), changes)? {
            Ok(applied) => applied,
            Err(errors) => return Ok(Err(errors.into_error())),
        };
        let saved = (!applied.new_connection).then(|| applied.mail.credentials());
        let account = applied.mail.account(applied.base_url, &secrets)?;
        let format: Format = settings::format_of(conn, &admin_id)?;
        Ok(Ok(Draft {
            account,
            saved,
            format,
        }))
    })
    .await??;

    let identities: Vec<Identity> = match state.mailer.test(&draft.account).await {
        Ok(identities) => identities.into_iter().map(Into::into).collect(),
        Err(error) => {
            if let Some(saved) = draft.saved {
                record(&state, saved, Outcome::failed(&error)).await?;
            }
            return Err(rejection(&error));
        }
    };
    if let Some(saved) = draft.saved.clone() {
        record(&state, saved, Outcome::Ok(Some(identities.clone()))).await?;
    }
    let Some(to) = to else {
        return Ok(Json(json!({ "identities": identities, "sent": false })));
    };
    // Only now: only test emails that really go out count.
    state
        .limiter
        .acquire(&TEST_MAIL, Key::User(admin.id.clone()))?;
    send_test(&state, draft, to).await?;
    Ok(Json(json!({ "identities": identities, "sent": true })))
}

/// Sends the test email in a separate task: if the call aborts, sending still
/// does not stop halfway. With the stored account's connection,
/// [`Delivery`] writes the result to the status.
async fn send_test(state: &AppState, draft: Draft, to: String) -> Result<(), ApiError> {
    let worker = state.clone();
    let task = tokio::spawn(async move {
        let delivery = Delivery::for_account(&worker, draft.account, draft.saved);
        let sent = delivery
            .send(&worker, &draft.format, |ctx| {
                SystemTest { to: &to }.render(ctx)
            })
            .await?;
        Ok::<_, anyhow::Error>(sent.map_err(|error| rejection(&error)))
    });
    task.await.map_err(anyhow::Error::from)??
}

/// Base URL of the JMAP server: `https://` (for development also `http://`
/// on `localhost` or `127.0.0.1`), host as DNS name or IPv4 address, without
/// credentials, path, query and fragment. The result is the origin. Taken
/// over from Roedy Mail (`api::servers::normalize_url`).
fn normalize_url(raw: &str) -> Result<Url, &'static str> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Err("required");
    }
    let url = Url::parse(raw.trim_end_matches('/')).map_err(|_| "invalid")?;
    let loopback = match url.host() {
        Some(Host::Domain(domain)) => {
            if !domain.split('.').all(is_dns_label) {
                return Err("invalid");
            }
            domain == "localhost"
        }
        Some(Host::Ipv4(ip)) => ip == Ipv4Addr::LOCALHOST,
        Some(Host::Ipv6(_)) | None => return Err("invalid"),
    };
    let scheme_ok = url.scheme() == "https" || (url.scheme() == "http" && loopback);
    let bare = url.username().is_empty()
        && url.password().is_none()
        && url.path() == "/"
        && url.query().is_none()
        && url.fragment().is_none();
    if !scheme_ok || !bare {
        return Err("invalid");
    }
    Url::parse(&url.origin().ascii_serialization()).map_err(|_| "invalid")
}

/// A label of a DNS name: `a–z`, `0–9`, `-`, not empty, no `-` at the edges.
fn is_dns_label(label: &str) -> bool {
    !label.is_empty()
        && label.len() <= 63
        && !label.starts_with('-')
        && !label.ends_with('-')
        && label
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

/// Empty or missing text becomes `None`, otherwise trimmed.
fn clean_optional(value: Option<String>) -> Option<String> {
    value.map(|v| v.trim().to_owned()).filter(|v| !v.is_empty())
}

/// A secret: at most [`MAX_SECRET`] bytes without control characters.
fn check_client_field(errors: &mut FieldErrors, field: &str, value: Option<&str>) {
    if let Some(value) = value
        && (value.len() > MAX_SECRET || value.chars().any(char::is_control))
    {
        errors.code(field, "invalid");
    }
}

/// Entry by an administrator.
fn audit_entry(category: Category, action: &'static str, user: &AuthUser, ip: ClientIp) -> Entry {
    Entry::new(category, action).actor(user).ip(ip.0)
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::*;

    fn checked(value: Value) -> Result<(Changes, Option<String>), Value> {
        let body: Body = serde_json::from_value(value).unwrap();
        body.checked().map_err(|error| match error {
            ApiError::Rejected { status, extra, .. } => {
                assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
                extra["fields"].clone()
            }
            other => panic!("not a 422: {other:?}"),
        })
    }

    #[test]
    fn fields_are_trimmed_and_emptied() {
        let (changes, to) = checked(json!({
            "url": " https://Mail.Example.com/ ", "auth": "token", "user": " noreply@example.org ",
            "app_password": "", "token": " abc ", "identity_id": "",
            "sender_name": "  ", "reply_to": null, "keep_sent": false,
            "to": " max@example.com ",
        }))
        .unwrap();
        assert_eq!(changes.url.as_deref(), Some("https://mail.example.com"));
        assert_eq!(changes.auth, Some(Auth::Token));
        assert_eq!(changes.user.as_deref(), Some("noreply@example.org"));
        assert_eq!(changes.app_password, None);
        assert_eq!(changes.token.as_deref(), Some("abc"));
        assert_eq!(changes.identity_id, Some(None));
        assert_eq!(changes.sender_name, Some(None));
        assert_eq!(changes.reply_to, Some(None));
        assert_eq!(changes.keep_sent, Some(false));
        assert_eq!(to.as_deref(), Some("max@example.com"));

        let (changes, to) = checked(json!({})).unwrap();
        assert!(changes.identity_id.is_none() && changes.auth.is_none() && to.is_none());
        // The password stays as it is – even with spaces.
        let (changes, _) = checked(json!({ "app_password": " a b " })).unwrap();
        assert_eq!(changes.app_password.as_deref(), Some(" a b "));
    }

    #[test]
    fn each_field_reports_its_own_code() {
        let errors = checked(json!({
            "url": "", "auth": "sso", "user": "no reply",
            "app_password": "a\u{7}b", "token": "x".repeat(5000),
            "identity_id": "i\n1", "sender_name": "x".repeat(101),
            "reply_to": "no-at", "to": "",
        }))
        .unwrap_err();
        let code = |field: &str| errors[field]["code"].as_str().unwrap().to_owned();
        assert_eq!(code("url"), "required");
        assert_eq!(code("auth"), "invalid");
        assert_eq!(code("user"), "invalid");
        assert_eq!(code("app_password"), "invalid");
        assert_eq!(code("token"), "invalid");
        assert_eq!(code("identity_id"), "invalid");
        assert_eq!(code("sender_name"), "too_long");
        assert_eq!(code("reply_to"), "invalid");
        assert_eq!(code("to"), "required");

        let errors = checked(json!({
            "sender_name": "Ex\u{200b}ample",
            "reply_to": format!("{}@example.org", "a".repeat(250)),
            "to": "max@",
        }))
        .unwrap_err();
        assert_eq!(errors["sender_name"]["code"], "invalid");
        assert_eq!(errors["reply_to"]["code"], "too_long");
        assert_eq!(errors["to"]["code"], "invalid");
    }

    #[test]
    fn unknown_fields_are_rejected() {
        assert!(serde_json::from_value::<Body>(json!({ "server_id": "s1" })).is_err());
        assert!(serde_json::from_value::<Body>(json!({ "pass": "x" })).is_err());
    }

    #[test]
    fn server_addresses_are_origins() {
        for (raw, origin) in [
            ("https://mail.example.com", "https://mail.example.com"),
            (" https://mail.example.com/ ", "https://mail.example.com"),
            (
                "https://mail.example.com:8443",
                "https://mail.example.com:8443",
            ),
            ("http://localhost:8080", "http://localhost:8080"),
            ("http://127.0.0.1:9", "http://127.0.0.1:9"),
        ] {
            let url = normalize_url(raw).unwrap();
            assert_eq!(url.origin().ascii_serialization(), origin, "{raw}");
        }
        for (raw, code) in [
            ("", "required"),
            ("mail.example.com", "invalid"),
            ("http://mail.example.com", "invalid"),
            ("https://mail.example.com/jmap", "invalid"),
            ("https://user:pw@mail.example.com", "invalid"),
            ("https://mail.example.com?x=1", "invalid"),
            ("https://[::1]", "invalid"),
            ("https://mail_x.example.com", "invalid"),
            ("ftp://mail.example.com", "invalid"),
        ] {
            assert_eq!(normalize_url(raw).err(), Some(code), "{raw}");
        }
    }

    /// A new server drops the stored secret; the same server keeps it.
    #[test]
    fn another_server_needs_the_secret_again() {
        let secrets = SecretKey::random().unwrap();
        let stored = SystemMail::new(
            &secrets,
            "https://mail.example.com",
            "noreply@example.com",
            "pw",
            None,
        )
        .unwrap();
        let moved = Changes {
            url: Some("https://other.example.com".into()),
            ..Changes::default()
        };
        let errors = apply(&secrets, Some(&stored), moved)
            .unwrap()
            .err()
            .unwrap();
        assert_eq!(
            errors.into_value(),
            json!({ "app_password": { "code": "required" } })
        );
        let same = Changes {
            sender_name: Some(Some("Example".into())),
            ..Changes::default()
        };
        let applied = apply(&secrets, Some(&stored), same).unwrap().ok().unwrap();
        assert_eq!(applied.changed, ["sender_name"]);
        assert!(!applied.new_connection);
    }

    /// The page names the missing mailbox; other reasons without `role`.
    #[test]
    fn missing_mailbox_names_its_role() {
        let extra = |error: MailError| match rejection(&error) {
            ApiError::Rejected { status, extra, .. } => (status, Value::Object(extra)),
            other => panic!("not a rejection: {other:?}"),
        };
        for role in ["drafts", "sent"] {
            assert_eq!(
                extra(MailError::Jmap(jmap::Error::NoMailbox(role))),
                (
                    StatusCode::BAD_GATEWAY,
                    json!({ "reason": "no_mailbox", "role": role })
                )
            );
        }
        assert_eq!(
            extra(MailError::Jmap(jmap::Error::NoIdentity)).1,
            json!({ "reason": "no_identity" })
        );
    }
}
