//! Own account: `GET auth/me`, `PATCH auth/me` (Account › Profile),
//! `DELETE auth/me` (delete account) and `PUT auth/password`, plus the check
//! of password and code in an existing session ([`Reauth`]), which
//! Account › Security also uses for authenticator apps and recovery codes
//! (`api::mfa`).
//!
//! Taken over from Roedy Mail (`crates/server/src/api/me.rs`) without the
//! email change, which follows with the system mails, and without mail
//! accounts and notifications.

use std::{net::IpAddr, time::Duration};

use axum::{
    Json, Router,
    extract::State,
    http::{StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, put},
};
use icu_properties::{
    CodePointMapData,
    props::{GeneralCategory, GeneralCategoryGroup},
};
use rusqlite::Connection;
use serde::Deserialize;
use serde_json::{Value, json};

use super::{
    ApiError, FieldErrors, JsonBody,
    auth::{MAX_PASSWORD, code_used, invalid_code, user_json},
};
use crate::{
    app::AppState,
    auth::{AuthUser, CurrentUser, email, password, session, totp, username},
    client_ip::ClientIp,
    db::{
        self,
        repo::{
            audit_log::{self, Category, Entry},
            sessions, totp_devices,
            users::{self, Since},
        },
    },
    rate_limit::{Key, Policy, Reservation},
};

/// Shared per-user limit for every check of the current password or of a
/// code in an existing session: five failed attempts in 15 minutes, then a
/// 15-minute lockout.
pub const REAUTH: Policy = Policy::lockout(
    "auth.reauth",
    5,
    Duration::from_secs(15 * 60),
    Duration::from_secs(15 * 60),
);

/// Correct passwords per user in an existing session: ten at once, then one
/// per 90 s, like `auth.login.success` at sign-in. Otherwise a loop over "new
/// codes" or "change password" would tie up Argon2 for everyone (503
/// `busy`). Reserved before the hash; a wrong password ends the reservation
/// without counting.
pub const REAUTH_SUCCESS: Policy =
    Policy::rate("auth.reauth.success", 10, Duration::from_secs(15 * 60));

/// Longest display name (setup, profile, invitations).
pub const MAX_NAME: usize = 100;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/auth/me", get(me).patch(update).delete(delete_me))
        .route("/auth/password", put(change_password))
}

/// Checks an email address the same way as the web app (setup, profile).
pub fn check_email(errors: &mut FieldErrors, field: &str, address: &str) {
    if address.is_empty() {
        errors.code(field, "required");
    } else if !email::is_valid(address) {
        errors.code(field, "invalid");
    }
}

/// Checks a display name (setup, profile, invitations): required, at most
/// [`MAX_NAME`] characters, only letters of any language with their marks,
/// spaces, hyphens and apostrophes (`'` and `’`) – the same rule as in the
/// web app.
pub fn check_display_name(errors: &mut FieldErrors, field: &str, name: &str) {
    if name.is_empty() {
        errors.code(field, "required");
    } else if name.chars().count() > MAX_NAME {
        errors.code(field, "too_long");
    } else if !name.chars().all(is_name_char) {
        errors.code(field, "invalid");
    }
}

/// Letter (`L*`) or mark (`M*`, e.g. a combining umlaut), space, hyphen or
/// apostrophe.
fn is_name_char(c: char) -> bool {
    if matches!(c, ' ' | '-' | '\'' | '\u{2019}') {
        return true;
    }
    let category = CodePointMapData::<GeneralCategory>::new().get(c);
    GeneralCategoryGroup::Letter.contains(category) || GeneralCategoryGroup::Mark.contains(category)
}

/// Line and paragraph separators and format characters that are not visible
/// (names of accounts and authenticator apps).
pub fn is_invisible(c: char) -> bool {
    matches!(
        c,
        '\u{00AD}'
            | '\u{061C}'
            | '\u{180E}'
            | '\u{200B}'..='\u{200F}'
            | '\u{2028}'..='\u{202E}'
            | '\u{2060}'..='\u{206F}'
            | '\u{FEFF}'
            | '\u{FFF9}'..='\u{FFFB}'
    )
}

/// Checks a new password against the policy (setup, change password):
/// `policy` with the violated rules like the web app.
pub fn check_new_password(errors: &mut FieldErrors, field: &str, password: &str) {
    if password.len() > MAX_PASSWORD {
        errors.code(field, "too_long");
        return;
    }
    let rules = password::violations(password);
    if !rules.is_empty() {
        let ids: Vec<&str> = rules.into_iter().map(password::Rule::id).collect();
        errors.add(field, json!({ "code": "policy", "rules": ids }));
    }
}

/// Current password for a check in the session: required, at most as long
/// as at sign-in (otherwise Argon2 would hash any length).
pub fn check_current_password(errors: &mut FieldErrors, field: &str, password: &str) {
    if password.is_empty() {
        errors.code(field, "required");
    } else if password.len() > MAX_PASSWORD {
        errors.code(field, "too_long");
    }
}

/// Code for a check in the session: with an authenticator app required and
/// six digits (`fields.code`), otherwise ignored.
pub fn check_code(errors: &mut FieldErrors, mfa: bool, code: Option<&str>) -> Option<String> {
    if !mfa {
        return None;
    }
    let code = totp::compact(code.unwrap_or_default());
    if code.is_empty() {
        errors.code("code", "required");
    } else if !totp::is_well_formed(&code) {
        errors.code("code", "invalid");
    }
    Some(code)
}

/// Check of password and code in an existing session with the shared limit
/// [`REAUTH`]. The password comes before the code so that a wrong password
/// does not use up a code.
pub struct Reauth<'a> {
    attempt: Reservation<'a>,
    guard: Guard,
}

/// State at the password check, for the transaction of the change that
/// follows ([`Guard::check`]).
#[derive(Debug, Clone)]
pub struct Guard {
    password_changed_at: String,
}

impl Guard {
    /// Whether the session still exists, the account is active and the
    /// password is unchanged since the check. Otherwise the change does not
    /// apply: [`Stale`].
    pub fn check(&self, conn: &Connection, user: &AuthUser) -> rusqlite::Result<Result<(), Stale>> {
        if !sessions::is_valid(conn, &user.id, &user.session_id)? {
            return Ok(Err(Stale::SessionEnded));
        }
        Ok(
            match users::since(conn, &user.id, &self.password_changed_at)? {
                Since::Unchanged(_) => Ok(()),
                Since::PasswordChanged => Err(Stale::PasswordChanged),
                Since::Inactive => Err(Stale::SessionEnded),
            },
        )
    }
}

/// Why a check no longer holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stale {
    /// Session ended or account locked (e.g. password changed on another
    /// device): 401 like any ended session.
    SessionEnded,
    /// The password has meanwhile been changed in this session (double
    /// submit, two concurrent requests): the one entered is no longer the
    /// current one (422 `invalid_password`, does not count towards the
    /// limit).
    PasswordChanged,
}

impl From<Stale> for ApiError {
    fn from(stale: Stale) -> Self {
        match stale {
            Stale::SessionEnded => Self::Unauthorized,
            Stale::PasswordChanged => invalid_password(),
        }
    }
}

fn invalid_password() -> ApiError {
    ApiError::rejected(StatusCode::UNPROCESSABLE_ENTITY, "invalid_password")
}

/// Result of a code check for [`Reauth::finish`].
pub enum CodeResult {
    Accepted,
    Used,
    Invalid,
}

impl From<totp::Check> for CodeResult {
    fn from(check: totp::Check) -> Self {
        match check {
            totp::Check::Accepted { .. } => Self::Accepted,
            totp::Check::Used => Self::Used,
            totp::Check::Invalid => Self::Invalid,
        }
    }
}

impl<'a> Reauth<'a> {
    /// Reserves an attempt and checks the password (wrong: 422
    /// `invalid_password`, counts towards [`REAUTH`]; correct: counts
    /// towards [`REAUTH_SUCCESS`]).
    pub async fn password(
        state: &'a AppState,
        user: &AuthUser,
        password: &str,
    ) -> Result<Self, ApiError> {
        let attempt = state.limiter.reserve(&REAUTH, Key::User(user.id.clone()))?;
        let success = state
            .limiter
            .reserve(&REAUTH_SUCCESS, Key::User(user.id.clone()))?;
        let id = user.id.clone();
        let credentials = db::call(&state.db, move |conn| {
            Ok(users::credentials_by_id(conn, &id)?)
        })
        .await?
        .ok_or(ApiError::Unauthorized)?;
        if !state
            .hasher
            .verify(password, Some(&credentials.password_hash))
            .await?
        {
            attempt.count();
            return Err(invalid_password());
        }
        // The hash ran: the success counts, even if the code is wrong then.
        success.count();
        let guard = Guard {
            password_changed_at: credentials.password_changed_at,
        };
        Ok(Self { attempt, guard })
    }

    /// State of the check for the transaction of the change.
    pub fn guard(&self) -> Guard {
        self.guard.clone()
    }

    /// Evaluates the code: accepted or already used end the attempt without
    /// counting, a wrong code counts.
    pub fn finish(self, result: impl Into<CodeResult>) -> Result<(), ApiError> {
        match result.into() {
            CodeResult::Accepted => Ok(()),
            CodeResult::Used => Err(code_used()),
            CodeResult::Invalid => Err(self.reject()),
        }
    }

    /// A wrong code: the attempt counts.
    pub fn reject(self) -> ApiError {
        self.attempt.count();
        invalid_code()
    }

    /// Checks `code` against the user's devices and evaluates it.
    pub async fn totp(
        self,
        state: &AppState,
        user: &AuthUser,
        code: String,
    ) -> Result<Guard, ApiError> {
        let secrets = state.secrets.clone();
        let id = user.id.clone();
        let check = db::call(&state.db, move |conn| {
            totp::check(conn, &secrets, &id, &code, totp::now())
        })
        .await?;
        let guard = self.guard();
        self.finish(check)?;
        Ok(guard)
    }
}

pub async fn mfa_enabled(state: &AppState, user: &AuthUser) -> Result<bool, ApiError> {
    let id = user.id.clone();
    Ok(db::call(&state.db, move |conn| {
        Ok(totp_devices::has_confirmed(conn, &id)?)
    })
    .await?)
}

/// The own account: profile, role, whether an authenticator app is set up,
/// when the password was changed and `sole_admin` – the only active
/// administrator, who cannot delete their account (`DELETE auth/me` would
/// answer 409 `last_admin`); always `false` for users.
async fn me(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
) -> Result<Json<Value>, ApiError> {
    let id = user.id.clone();
    let admin = user.is_admin();
    let (changed_at, mfa, sole_admin) = db::call(&state.db, move |conn| {
        let changed_at = users::credentials_by_id(conn, &id)?
            .map(|c| c.password_changed_at)
            .unwrap_or_default();
        let mfa = totp_devices::has_confirmed(conn, &id)?;
        let sole_admin = admin && users::sole_admin(conn, &id)?;
        Ok((changed_at, mfa, sole_admin))
    })
    .await?;
    let mut body = user_json(&user);
    body["mfa_enabled"] = mfa.into();
    body["password_changed_at"] = changed_at.into();
    body["sole_admin"] = sole_admin.into();
    Ok(Json(body))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ProfileBody {
    display_name: Option<String>,
    username: Option<String>,
}

/// Account › Profile: display name and username apply immediately. A taken
/// username is 422 `invalid` with `fields.username.code` = `taken`; on an
/// error nothing is saved.
async fn update(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    ip: ClientIp,
    JsonBody(body): JsonBody<ProfileBody>,
) -> Result<Json<Value>, ApiError> {
    let mut errors = FieldErrors::default();
    let name = body.display_name.as_deref().map(str::trim);
    if let Some(name) = name {
        check_display_name(&mut errors, "display_name", name);
    }
    let username = body.username.as_deref().map(str::trim);
    if let Some(username) = username.filter(|u| !username::is_valid(u)) {
        let code = if username.is_empty() {
            "required"
        } else {
            "invalid"
        };
        errors.code("username", code);
    }
    if !errors.is_empty() {
        return Err(errors.into_error());
    }

    let change = ProfileChange {
        display_name: name.filter(|n| *n != user.display_name).map(str::to_owned),
        username: username.filter(|u| *u != user.username).map(str::to_owned),
    };
    let updated = if change.is_some() {
        save_profile(&state, &user, change, ip).await?
    } else {
        user
    };
    Ok(Json(json!({ "status": "ok", "user": user_json(&updated) })))
}

/// Display name and username from `PATCH auth/me`; `None` = unchanged.
#[derive(Debug, Clone)]
pub struct ProfileChange {
    pub display_name: Option<String>,
    pub username: Option<String>,
}

impl ProfileChange {
    fn is_some(&self) -> bool {
        self.display_name.is_some() || self.username.is_some()
    }
}

/// A username that is taken.
pub fn username_taken() -> ApiError {
    let mut errors = FieldErrors::default();
    errors.code("username", "taken");
    errors.into_error()
}

/// Saves `change` with an audit entry in the caller's transaction and
/// returns the user with the new profile; `None` if the username is taken
/// (then nothing is changed).
pub fn apply_profile(
    conn: &Connection,
    user: &AuthUser,
    change: &ProfileChange,
    ip: IpAddr,
) -> anyhow::Result<Option<AuthUser>> {
    let (name, username) = (change.display_name.as_deref(), change.username.as_deref());
    if let Err(error) = users::update_profile(conn, &user.id, name, username) {
        if error.sqlite_error_code() == Some(rusqlite::ErrorCode::ConstraintViolation) {
            return Ok(None);
        }
        return Err(error.into());
    }
    let fields: Vec<&str> = [name.map(|_| "display_name"), username.map(|_| "username")]
        .into_iter()
        .flatten()
        .collect();
    audit_log::insert(
        conn,
        &Entry::new(Category::Login, "auth.profile_change")
            .actor(user)
            .ip(ip)
            .target(&user.id)
            .details(json!({ "fields": fields })),
    )?;
    let mut updated = user.clone();
    if let Some(name) = name {
        name.clone_into(&mut updated.display_name);
    }
    if let Some(username) = username {
        username.clone_into(&mut updated.username);
    }
    Ok(Some(updated))
}

/// Saves display name and username – only while the own session still
/// exists (otherwise 401 like any ended session).
async fn save_profile(
    state: &AppState,
    user: &AuthUser,
    change: ProfileChange,
    ip: ClientIp,
) -> Result<AuthUser, ApiError> {
    let user = user.clone();
    db::call(&state.db, move |conn| {
        let tx = conn.transaction()?;
        if !sessions::is_valid(&tx, &user.id, &user.session_id)? {
            return Ok(Err(ApiError::Unauthorized));
        }
        let Some(updated) = apply_profile(&tx, &user, &change, ip.0)? else {
            return Ok(Err(username_taken()));
        };
        tx.commit()?;
        Ok(Ok(updated))
    })
    .await?
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PasswordBody {
    current_password: String,
    new_password: String,
    /// Required with an authenticator app.
    code: Option<String>,
}

/// "Change password" (Account › Security): current password, a code with an
/// authenticator app, new password per the policy. All other sessions end –
/// an attacker's too; the own one stays. The response states their number
/// (`sessions_ended`): only if there were others does the page report that
/// other devices were signed out.
async fn change_password(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    ip: ClientIp,
    JsonBody(body): JsonBody<PasswordBody>,
) -> Result<Json<Value>, ApiError> {
    let mfa = mfa_enabled(&state, &user).await?;
    let mut errors = FieldErrors::default();
    check_current_password(&mut errors, "current_password", &body.current_password);
    check_new_password(&mut errors, "new_password", &body.new_password);
    let code = check_code(&mut errors, mfa, body.code.as_deref());
    if !errors.is_empty() {
        return Err(errors.into_error());
    }

    let reauth = Reauth::password(&state, &user, &body.current_password).await?;
    let guard = match code {
        Some(code) => reauth.totp(&state, &user, code).await?,
        None => reauth.guard(),
    };
    let hash = state.hasher.hash(&body.new_password).await?;
    let changed = db::call(&state.db, move |conn| {
        let tx = conn.transaction()?;
        if let Err(stale) = guard.check(&tx, &user)? {
            return Ok(Err(stale));
        }
        let changed_at = users::set_password(&tx, &user.id, &hash)?;
        let ended = sessions::delete_for_user(&tx, &user.id, Some(&user.session_id))?;
        audit_log::insert(
            &tx,
            &Entry::new(Category::Login, "auth.password_change")
                .actor(&user)
                .ip(ip.0)
                .target(&user.id)
                .details(json!({ "sessions_ended": ended })),
        )?;
        tx.commit()?;
        Ok(Ok((changed_at, ended)))
    })
    .await?;
    let (changed_at, ended) = changed?;
    Ok(Json(
        json!({ "password_changed_at": changed_at, "sessions_ended": ended }),
    ))
}

/// Confirmation value of `DELETE auth/me`; the web app sends it once the user
/// has typed the confirmation word of their interface language.
const DELETE_WORD: &str = "delete";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DeleteBody {
    confirm: String,
}

/// "Delete account": with the confirmation word `confirm` the hub removes
/// the user with everything attached to them (`users::delete`: sessions,
/// authenticator apps, codes, settings) and signs out. The only active
/// administrator cannot delete their account: 409 `last_admin`. The audit
/// log keeps the entry with their name.
async fn delete_me(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    ip: ClientIp,
    JsonBody(body): JsonBody<DeleteBody>,
) -> Result<Response, ApiError> {
    if body.confirm != DELETE_WORD {
        let mut errors = FieldErrors::default();
        errors.code("confirm", "invalid");
        return Err(errors.into_error());
    }
    let removed = user.id.clone();
    let deleted = db::call(&state.db, move |conn| {
        let tx = conn.transaction()?;
        if !sessions::is_valid(&tx, &user.id, &user.session_id)? {
            return Ok(Err(ApiError::Unauthorized));
        }
        if users::sole_admin(&tx, &user.id)? {
            return Ok(Err(ApiError::rejected(StatusCode::CONFLICT, "last_admin")));
        }
        // Name and role as of now, not as of the start of the request.
        let Some(profile) = users::credentials_by_id(&tx, &user.id)?.map(|c| c.user) else {
            return Ok(Err(ApiError::Unauthorized));
        };
        // Before deleting: `actor_id` becomes `NULL` afterwards, the name stays.
        audit_log::insert(
            &tx,
            &Entry::new(Category::Users, "users.delete_self")
                .actor_profile(&profile)
                .ip(ip.0)
                .target(&profile.id)
                .details(json!({
                    "name": profile.display_name,
                    "email": profile.email,
                    "role": profile.role.as_str(),
                })),
        )?;
        if !users::delete(&tx, &user.id)? {
            return Ok(Err(ApiError::Unauthorized));
        }
        tx.commit()?;
        Ok(Ok(()))
    })
    .await?;
    deleted?;
    state.challenges.discard_user(&removed);
    tracing::info!(user = %removed, "account deleted");
    Ok((
        StatusCode::NO_CONTENT,
        [(header::SET_COOKIE, session::clear_cookie())],
    )
        .into_response())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        auth::session::{self, Client},
        db::repo::users::{
            Profile, Role,
            tests::{add, conn},
        },
    };

    #[test]
    fn display_names_are_letters_spaces_hyphens_and_apostrophes() {
        let check = |name: &str| {
            let mut errors = FieldErrors::default();
            check_display_name(&mut errors, "name", name);
            errors.into_value()["name"]["code"]
                .as_str()
                .map(str::to_owned)
        };
        for name in [
            "Max Smith",
            "Müller-Lüdenscheidt",
            // Decomposed: u with a combining diaeresis.
            "Mu\u{308}ller",
            "O’Brien",
            "O'Brien",
            "Zoë Ångström",
            "Łukasz Żółć",
            "Ελένη Παπαδοπούλου",
            "Иван Петров",
            "李小龙",
            "محمد",
            "देवनागरी",
        ] {
            assert_eq!(check(name), None, "{name}");
        }
        for name in [
            "Max Smith 2",
            "Max (Admin)",
            "Max.Smith",
            "max@example.com",
            "Max_Smith",
            "Max 🙂",
            "Max\u{200B}Smith",
            "Max\u{202E}htimS",
            "Max\tSmith",
            "Max\u{a0}Smith",
        ] {
            assert_eq!(check(name).as_deref(), Some("invalid"), "{name}");
        }
        assert_eq!(check(""), Some("required".into()));
        assert_eq!(check(&"x".repeat(MAX_NAME)), None);
        assert_eq!(check(&"x".repeat(MAX_NAME + 1)), Some("too_long".into()));
        // Characters, not bytes.
        assert_eq!(check(&"ü".repeat(MAX_NAME)), None);
    }

    #[test]
    fn passwords_and_codes_are_checked_like_the_web_app() {
        let mut errors = FieldErrors::default();
        check_new_password(&mut errors, "new", "short");
        check_new_password(&mut errors, "long", &"Aa1".repeat(400));
        check_new_password(&mut errors, "good", "Summer2026!");
        check_current_password(&mut errors, "current", "");
        assert_eq!(check_code(&mut errors, false, None), None);
        assert_eq!(
            check_code(&mut errors, true, Some("123 456")).as_deref(),
            Some("123456")
        );
        assert_eq!(
            errors.into_value(),
            json!({
                "new": { "code": "policy", "rules": ["len", "case", "num"] },
                "long": { "code": "too_long" },
                "current": { "code": "required" },
            })
        );
        for (code, expected) in [(None, "required"), (Some("12345"), "invalid")] {
            let mut errors = FieldErrors::default();
            check_code(&mut errors, true, code);
            assert_eq!(errors.into_value(), json!({ "code": { "code": expected } }));
        }
    }

    #[test]
    fn guard_fails_after_concurrent_changes() {
        let conn = conn();
        let id = add(&conn, "max", "max@example.com", Role::User);
        let started = session::start(&conn, &id, &Client::default(), true).unwrap();
        let profile = Profile {
            id: id.clone(),
            username: "max".into(),
            email: "max@example.com".into(),
            display_name: "Max".into(),
            role: Role::User,
        };
        let user = AuthUser::new(profile, started.id.clone(), started.auth_time);
        let checked = users::credentials_by_id(&conn, &id).unwrap().unwrap();
        let guard = Guard {
            password_changed_at: checked.password_changed_at,
        };
        let check = |guard: &Guard| guard.check(&conn, &user).unwrap();
        assert_eq!(check(&guard), Ok(()));

        // Password changed in the meantime (own session stayed).
        conn.execute(
            "UPDATE users SET password_changed_at = '2099-01-01T00:00:00.000Z' WHERE id = ?1",
            [&id],
        )
        .unwrap();
        assert_eq!(check(&guard), Err(Stale::PasswordChanged));
        let fresh = Guard {
            password_changed_at: "2099-01-01T00:00:00.000Z".into(),
        };
        assert_eq!(check(&fresh), Ok(()));

        // Locked.
        conn.execute("UPDATE users SET status = 'locked' WHERE id = ?1", [&id])
            .unwrap();
        assert_eq!(check(&fresh), Err(Stale::SessionEnded));
        conn.execute("UPDATE users SET status = 'active' WHERE id = ?1", [&id])
            .unwrap();

        // Session ended.
        sessions::delete(&conn, &id, &started.id).unwrap();
        assert_eq!(check(&fresh), Err(Stale::SessionEnded));
    }
}
