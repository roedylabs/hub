//! Sign-in and sign-out.
//!
//! - `POST auth/login` (`{login, password, remember?}`): email address or
//!   username. Without an authenticator app, signed in right away (`status:
//!   ok` with the session cookie), otherwise `status: totp_required` with the
//!   intermediate step in the cookie `__Host-roedy_hub_mfa`
//!   (`auth::challenge`, five minutes).
//! - `POST auth/totp` (`{code}`, code from an authenticator app) and
//!   `POST auth/recovery` (`{code}`, recovery code) complete the sign-in.
//! - `POST auth/logout` ends the session (docs/app-contract.md › Signing
//!   out).
//!
//! Errors are domain errors (422), never 401: `invalid_credentials` (wrong
//! password, unknown or locked account – the same answer, so it reveals
//! nothing), `invalid_code`, `code_used` (code already used, does not count
//! towards the limit) and `login_expired` (intermediate step expired or no
//! longer valid, enter the password again); limits as 429 `rate_limited`.
//!
//! A sign-in as another account ends the browser's previous session. A
//! sign-in as the same account (`/login?reauth=1`, docs/app-contract.md ›
//! Fresh sign-in) keeps the session and its ID and only renews `auth_time`,
//! so the apps keep what they bound to the session.
//!
//! A sign-in from a device the account has not used before sends "New
//! sign-in" (`security_login`), one with a recovery code "Recovery code
//! used" (`security_recovery_used`, which then also names a new device).
//!
//! Taken over from Roedy Mail (`crates/server/src/api/auth.rs`) without
//! maintenance mode.

use std::time::Duration;

use axum::{
    Json, Router,
    extract::State,
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse as _, Response},
    routing::post,
};
use rusqlite::Connection;
use serde::Deserialize;
use serde_json::{Value, json};

use super::{ApiError, FieldErrors, JsonBody};
use crate::{
    app::AppState,
    auth::{
        AuthUser, CurrentUser,
        challenge::{self, Challenge, Claim, Ticket},
        device::Device,
        recovery::{self, Redeem},
        session::{self, Origin},
        totp::{self, Check},
    },
    client_ip::ClientIp,
    db::{
        self,
        repo::{
            audit_log::{self, Category, Entry},
            known_devices, recovery_codes, sessions, totp_devices,
            users::{self, Credentials, Profile, Since},
        },
    },
    notices::{self, Outbox, Security},
    rate_limit::{Key, Policy},
};

/// Failed attempts per client IP (IPv6 per /64): 20 at once, then one every
/// 30 s. Counts passwords and codes.
const LOGIN_IP: Policy = Policy::rate("auth.login.ip", 20, Duration::from_secs(10 * 60));
/// Wrong passwords per spelling of the login ([`login_key`], also for
/// unknown input): ten in 15 minutes, then a five-minute lockout.
pub(super) const LOGIN_ACCOUNT: Policy = Policy::lockout(
    "auth.login.account",
    10,
    Duration::from_secs(15 * 60),
    Duration::from_secs(5 * 60),
);
/// Correct passwords per spelling ([`login_key`]): ten at once, then one
/// every 90 s. Only checked before the hash, so that a sign-in loop does not
/// tie up Argon2 for everyone (503 `busy`), and counted only after a correct
/// password. Failed attempts do not touch it: a reservation per attempt would
/// let concurrent failed attempts bump into the owner's successes and so
/// reveal the account. How many sign-ins run at once is already limited by
/// [`LOGIN_ACCOUNT`] (ten per spelling).
pub(super) const LOGIN_SUCCESS: Policy =
    Policy::rate("auth.login.success", 10, Duration::from_secs(15 * 60));
/// Wrong codes per account in the second step: five in 15 minutes, then a
/// 15-minute lockout (RFC 6238, 5.2: limit guessing).
const LOGIN_CODE: Policy = Policy::lockout(
    "auth.login.code",
    5,
    Duration::from_secs(15 * 60),
    Duration::from_secs(15 * 60),
);

/// Longest input for email address or username.
const MAX_LOGIN: usize = 254;
/// Argon2 handles any length; no password needs more (also for setup and
/// "change password").
pub const MAX_PASSWORD: usize = 1024;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/auth/login", post(login))
        .route("/auth/totp", post(totp_step))
        .route("/auth/recovery", post(recovery_step))
        .route("/auth/logout", post(logout))
}

/// User in responses (sign-in, setup, `auth/me`).
pub fn user_json(user: &AuthUser) -> Value {
    json!({
        "id": user.id,
        "username": user.username,
        "email": user.email,
        "display_name": user.display_name,
        "role": user.role.as_str(),
    })
}

/// Origin of the request with the ID of the device ([`Device::hash`]).
pub fn origin(state: &AppState, headers: &HeaderMap, ip: ClientIp) -> Origin {
    Origin::new(headers, ip.0, &state.secrets)
}

/// Device of the request, for the security notices.
pub fn device(headers: &HeaderMap) -> Device {
    Device::parse(session::user_agent(headers).as_deref())
}

fn rejected(code: &'static str) -> ApiError {
    ApiError::rejected(StatusCode::UNPROCESSABLE_ENTITY, code)
}

/// Key of [`LOGIN_ACCOUNT`]: the input normalized the way
/// `users::find_for_login` searches (`users::normalize_login`). Equal keys
/// thus mean exactly "the same account or the same unknown input".
pub(super) fn login_key(login: &str) -> Key {
    Key::User(format!("login:{}", users::normalize_login(login)))
}

pub fn invalid_code() -> ApiError {
    rejected("invalid_code")
}

pub fn code_used() -> ApiError {
    rejected("code_used")
}

fn login_expired() -> ApiError {
    rejected("login_expired")
}

/// Ends the own session (`auth/logout`, `DELETE auth/sessions/:id`) with
/// the audit entry `auth.logout`; `false` if it had already ended.
pub fn end_own_session(conn: &Connection, user: &AuthUser, ip: ClientIp) -> rusqlite::Result<bool> {
    if !sessions::delete(conn, &user.id, &user.session_id)? {
        return Ok(false);
    }
    audit_log::insert(
        conn,
        &Entry::new(Category::Login, "auth.logout")
            .actor(user)
            .ip(ip.0)
            .target(&user.session_id),
    )?;
    Ok(true)
}

/// Failed sign-in to a known account (`reason`: `password`, `locked`,
/// `totp`, `recovery`). Unknown input is not logged – it could contain a
/// mistyped password.
fn failed(profile: &Profile, ip: ClientIp, reason: &str) -> Entry {
    Entry::new(Category::Login, "auth.login_failed")
        .actor_profile(profile)
        .target(&profile.id)
        .ip(ip.0)
        .details(json!({ "reason": reason }))
}

/// Writes an entry in the background. The response does not wait for it:
/// otherwise a wrong password for a known account would take measurably
/// longer than for an unknown one, and a write error would turn the
/// rejection into a 500.
fn record_later(state: &AppState, entry: Entry) {
    let db = state.db.clone();
    state.background.spawn(async move {
        let written = db::call(&db, move |conn| Ok(audit_log::insert(conn, &entry)?)).await;
        if let Err(error) = written {
            tracing::warn!("audit log entry not written: {error:#}");
        }
    });
}

/// How the sign-in was completed (audit log).
enum Method {
    Password,
    Totp { device: String },
    Recovery { remaining: u32 },
}

impl Method {
    fn details(&self) -> Value {
        match self {
            Self::Password => json!({ "method": "password" }),
            Self::Totp { device } => json!({ "method": "totp", "device": device }),
            Self::Recovery { remaining } => json!({ "method": "recovery", "remaining": remaining }),
        }
    }
}

/// Creates the session – or renews `auth_time` of the browser's session of
/// the same account – and logs `auth.login` (or `auth.reauth`). A sign-in as
/// another account ends the browser's previous session.
///
/// `checked_at` is `password_changed_at` at the time the password was
/// checked: if the password has changed since, the check applied to the old
/// state – then no session ([`Started::Expired`]), otherwise a sign-in with
/// the old password would survive the change. An account locked in the
/// meantime gets the same answer as a wrong password ([`Started::Locked`]).
/// A password alone signs in only without an authenticator app (otherwise
/// [`Started::Code`]), a code or recovery code only while one is still set up
/// (otherwise [`Started::Expired`]: it was removed after the code was
/// checked), both checked in the same transaction. The session carries the
/// current profile, not the one from the check.
///
/// The device is remembered (`known_devices`); an unknown one adds "New
/// sign-in" to `outbox`, a recovery code "Recovery code used" instead.
#[allow(clippy::too_many_arguments, reason = "all parts of one sign-in")]
fn start_session(
    conn: &mut Connection,
    outbox: &mut Outbox,
    user_id: &str,
    checked_at: &str,
    origin: &Origin,
    persistent: bool,
    previous: Option<&AuthUser>,
    method: &Method,
    ticket: Option<&Ticket>,
) -> anyhow::Result<Started> {
    // The intermediate step was discarded after its code was checked (the
    // account was locked, removed or reset meanwhile, perhaps unlocked
    // again): no session.
    if ticket.is_some_and(Ticket::discarded) {
        return Ok(Started::Expired);
    }
    let tx = conn.transaction()?;
    let profile = match users::since(&tx, user_id, checked_at)? {
        Since::Unchanged(profile) => profile,
        Since::PasswordChanged => return Ok(Started::Expired),
        Since::Inactive => return Ok(Started::Locked),
    };
    let mfa = totp_devices::has_confirmed(&tx, user_id)?;
    match method {
        Method::Password if mfa => return Ok(Started::Code),
        Method::Totp { .. } | Method::Recovery { .. } if !mfa => return Ok(Started::Expired),
        _ => {}
    }
    let client = origin.client();
    let renewed = match previous.filter(|previous| previous.id == user_id) {
        Some(previous) => sessions::reauthenticate(&tx, user_id, &previous.session_id)?
            .map(|auth_time| (previous.session_id.clone(), auth_time)),
        None => None,
    };
    if let Some((session_id, auth_time)) = renewed {
        let user = AuthUser::new(profile, session_id, auth_time);
        notice(&tx, outbox, &user, origin, method)?;
        audit_log::insert(
            &tx,
            &Entry::new(Category::Login, "auth.reauth")
                .actor(&user)
                .target(&user.session_id)
                .ip(origin.ip)
                .details(method.details()),
        )?;
        tx.commit()?;
        return Ok(Started::Done(user, None));
    }
    if let Some(previous) = previous {
        sessions::delete(&tx, &previous.id, &previous.session_id)?;
    }
    let started = session::start(&tx, user_id, &client, persistent)?;
    let user = AuthUser::new(profile, started.id.clone(), started.auth_time);
    notice(&tx, outbox, &user, origin, method)?;
    audit_log::insert(
        &tx,
        &Entry::new(Category::Login, "auth.login")
            .actor(&user)
            .target(&started.id)
            .ip(origin.ip)
            .details(method.details()),
    )?;
    tx.commit()?;
    Ok(Started::Done(user, Some(started.cookie)))
}

/// Remembers the device of a sign-in and reports "New sign-in" or "Recovery
/// code used" in the sign-in's transaction.
fn notice(
    conn: &Connection,
    outbox: &mut Outbox,
    user: &AuthUser,
    origin: &Origin,
    method: &Method,
) -> anyhow::Result<()> {
    let new_device = known_devices::remember(conn, &user.id, &origin.device_hash)?;
    let event = match method {
        Method::Recovery { .. } => {
            let codes = recovery_codes::summary(conn, &user.id)?;
            Security::RecoveryUsed {
                remaining: codes.as_ref().map_or(0, |c| c.unused),
                total: codes.map_or(0, |c| c.total),
                new_device,
                at: origin.into(),
            }
        }
        _ if new_device => Security::Login { at: origin.into() },
        _ => return Ok(()),
    };
    notices::security(conn, outbox, user.into(), event)
}

/// Result of [`start_session`].
enum Started {
    /// Signed in; the cookie of a new session (`None`: the session was
    /// renewed and keeps its cookie).
    Done(AuthUser, Option<HeaderValue>),
    /// The account has an authenticator app: continue with the intermediate step.
    Code,
    Expired,
    Locked,
}

/// [`start_session`] on the database thread; an outdated state becomes
/// `login_expired`.
#[allow(clippy::too_many_arguments, reason = "all parts of one sign-in")]
async fn sign_in(
    state: &AppState,
    user_id: String,
    checked_at: String,
    origin: Origin,
    persistent: bool,
    previous: Option<CurrentUser>,
    method: Method,
    ticket: Option<Ticket>,
) -> Result<Option<(AuthUser, Option<HeaderValue>)>, ApiError> {
    let worker = state.clone();
    let started = db::call(&state.db, move |conn| {
        let previous = previous.map(|CurrentUser(user)| user);
        let mut outbox = Outbox::default();
        let started = start_session(
            conn,
            &mut outbox,
            &user_id,
            &checked_at,
            &origin,
            persistent,
            previous.as_ref(),
            &method,
            ticket.as_ref(),
        )?;
        // Right after the commit, even if the request is gone by now.
        outbox.send(&worker);
        Ok(started)
    })
    .await?;
    match started {
        Started::Done(user, cookie) => {
            tracing::info!(user = %user.id, "signed in");
            Ok(Some((user, cookie)))
        }
        Started::Code => Ok(None),
        Started::Expired => Err(login_expired()),
        Started::Locked => Err(rejected("invalid_credentials")),
    }
}

/// 200 with `status: ok`, the user and the cookies.
fn signed_in(user: &AuthUser, cookies: impl IntoIterator<Item = HeaderValue>) -> Response {
    let mut response = Json(json!({ "status": "ok", "user": user_json(user) })).into_response();
    for cookie in cookies {
        response.headers_mut().append(header::SET_COOKIE, cookie);
    }
    response
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LoginBody {
    login: String,
    password: String,
    /// "Stay signed in" (preselected on the page).
    #[serde(default = "yes")]
    remember: bool,
}

fn yes() -> bool {
    true
}

/// "Sign in": check the password, then signed in or intermediate step.
async fn login(
    State(state): State<AppState>,
    ip: ClientIp,
    headers: HeaderMap,
    previous: Option<CurrentUser>,
    JsonBody(body): JsonBody<LoginBody>,
) -> Result<Response, ApiError> {
    let mut errors = FieldErrors::default();
    let login = body.login.trim().to_owned();
    if login.is_empty() {
        errors.code("login", "required");
    } else if login.len() > MAX_LOGIN {
        errors.code("login", "too_long");
    }
    if body.password.is_empty() {
        errors.code("password", "required");
    } else if body.password.len() > MAX_PASSWORD {
        errors.code("password", "too_long");
    }
    if !errors.is_empty() {
        return Err(errors.into_error());
    }

    // Reserve up front; it counts only once the password is wrong.
    let ip_attempt = state.limiter.reserve(&LOGIN_IP, Key::Ip(ip.network()))?;
    let lookup = login.clone();
    let found = db::call(&state.db, move |conn| {
        Ok(users::find_for_login(conn, &lookup)?)
    })
    .await?;
    // Known and unknown input count the same, per spelling: a key per
    // account would reveal that address and username belong together.
    let key = login_key(&login);
    let account_attempt = state.limiter.reserve(&LOGIN_ACCOUNT, key.clone())?;
    // Successes are limited too: before the hash only check, without
    // reserving.
    state.limiter.remaining(&LOGIN_SUCCESS, &key)?;
    // Unknown accounts against the placeholder, so the response takes
    // equally long.
    let matches = state
        .hasher
        .verify(
            &body.password,
            found.as_ref().map(|c| c.password_hash.as_str()),
        )
        .await?;
    let credentials = match found {
        Some(credentials) if matches && credentials.active => credentials,
        other => {
            ip_attempt.count();
            account_attempt.count();
            if let Some(credentials) = other {
                let reason = if matches { "locked" } else { "password" };
                record_later(&state, failed(&credentials.user, ip, reason));
            }
            tracing::info!(ip = %ip.0, "sign-in rejected");
            return Err(rejected("invalid_credentials"));
        }
    };
    // The hash ran: the success counts, even if the code or an outdated
    // state follows, and also beyond the limit when several were checked at
    // once.
    state.limiter.charge(&LOGIN_SUCCESS, key);
    drop((ip_attempt, account_attempt));
    rehash_if_outdated(&state, &credentials, &body.password);

    let Credentials {
        user: profile,
        password_changed_at,
        ..
    } = credentials;
    let remember = body.remember;
    let signed = sign_in(
        &state,
        profile.id.clone(),
        password_changed_at.clone(),
        origin(&state, &headers, ip),
        remember,
        previous,
        Method::Password,
        None,
    )
    .await?;
    if let Some((user, cookie)) = signed {
        return Ok(signed_in(&user, cookie));
    }
    let cookie = state.challenges.start(Challenge {
        user_id: profile.id,
        remember,
        password_changed_at,
    })?;
    Ok((
        [(header::SET_COOKIE, cookie)],
        Json(json!({ "status": "totp_required" })),
    )
        .into_response())
}

/// Rehashes the password after a sign-in when the hash was created with
/// other parameters – in the background, so the response does not wait for a
/// second hash. If that fails, the old hash stays.
fn rehash_if_outdated(state: &AppState, credentials: &Credentials, password: &str) {
    if !state.hasher.is_outdated(&credentials.password_hash) {
        return;
    }
    let (hasher, db) = (state.hasher.clone(), state.db.clone());
    let (id, old, password) = (
        credentials.user.id.clone(),
        credentials.password_hash.clone(),
        password.to_owned(),
    );
    state.background.spawn(async move {
        let result = async {
            let new = hasher.hash(&password).await?;
            let user = id.clone();
            db::call(&db, move |conn| {
                Ok(users::rehash_password(conn, &user, &old, &new)?)
            })
            .await
        }
        .await;
        if let Err(error) = result {
            tracing::warn!(user = %id, "password not rehashed: {error:#}");
        }
    });
}

/// Takes out the intermediate step for checking a code (see
/// `auth::challenge`). If the account has since been locked, the password
/// changed or the second factor removed, it is no longer valid
/// (`login_expired`, enter the password again).
async fn claim(state: &AppState, headers: &HeaderMap) -> Result<(Claim, Profile), ApiError> {
    let claim = state.challenges.claim(headers).ok_or_else(login_expired)?;
    let id = claim.challenge().user_id.clone();
    let found = db::call(&state.db, move |conn| {
        let credentials = users::credentials_by_id(conn, &id)?;
        let mfa = totp_devices::has_confirmed(conn, &id)?;
        Ok(credentials.filter(|c| c.active && mfa))
    })
    .await?;
    match found {
        Some(c) if c.password_changed_at == claim.challenge().password_changed_at => {
            Ok((claim, c.user))
        }
        _ => {
            claim.consume();
            Err(login_expired())
        }
    }
}

/// Completes the intermediate step with a session.
async fn complete(
    state: &AppState,
    headers: &HeaderMap,
    ip: ClientIp,
    previous: Option<CurrentUser>,
    (claim, profile): (Claim, Profile),
    method: Method,
) -> Result<Response, ApiError> {
    let (challenge, ticket) = claim.consume();
    let signed = sign_in(
        state,
        profile.id,
        challenge.password_changed_at,
        origin(state, headers, ip),
        challenge.remember,
        previous,
        method,
        Some(ticket),
    )
    .await?;
    // With a confirmed code there is no further step.
    let (user, cookie) = signed.ok_or_else(login_expired)?;
    Ok(signed_in(
        &user,
        cookie.into_iter().chain([challenge::clear_cookie()]),
    ))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CodeBody {
    code: String,
}

/// Second step with a code from an authenticator app.
async fn totp_step(
    State(state): State<AppState>,
    ip: ClientIp,
    headers: HeaderMap,
    previous: Option<CurrentUser>,
    JsonBody(body): JsonBody<CodeBody>,
) -> Result<Response, ApiError> {
    let claimed = claim(&state, &headers).await?;
    let code = totp::compact(&body.code);
    // Not six digits: not an attempt, nothing to count.
    if !totp::is_well_formed(&code) {
        return Err(invalid_code());
    }
    let user_id = claimed.1.id.clone();
    let ip_attempt = state.limiter.reserve(&LOGIN_IP, Key::Ip(ip.network()))?;
    let code_attempt = state
        .limiter
        .reserve(&LOGIN_CODE, Key::User(user_id.clone()))?;
    let secrets = state.secrets.clone();
    let check = db::call(&state.db, move |conn| {
        totp::check(conn, &secrets, &user_id, &code, totp::now())
    })
    .await?;
    match check {
        Check::Accepted { name, .. } => {
            drop((ip_attempt, code_attempt));
            let method = Method::Totp { device: name };
            complete(&state, &headers, ip, previous, claimed, method).await
        }
        Check::Used => Err(code_used()),
        Check::Invalid => {
            ip_attempt.count();
            code_attempt.count();
            record_later(&state, failed(&claimed.1, ip, "totp"));
            Err(invalid_code())
        }
    }
}

/// Second step with a recovery code.
async fn recovery_step(
    State(state): State<AppState>,
    ip: ClientIp,
    headers: HeaderMap,
    previous: Option<CurrentUser>,
    JsonBody(body): JsonBody<CodeBody>,
) -> Result<Response, ApiError> {
    let claimed = claim(&state, &headers).await?;
    let Some(code) = recovery::normalize(&body.code) else {
        return Err(invalid_code());
    };
    let user_id = claimed.1.id.clone();
    let ip_attempt = state.limiter.reserve(&LOGIN_IP, Key::Ip(ip.network()))?;
    let code_attempt = state
        .limiter
        .reserve(&LOGIN_CODE, Key::User(user_id.clone()))?;
    let redeemed = db::call(&state.db, move |conn| {
        recovery::redeem(conn, &user_id, &code)
    })
    .await?;
    match redeemed {
        Redeem::Accepted { remaining } => {
            drop((ip_attempt, code_attempt));
            let method = Method::Recovery { remaining };
            complete(&state, &headers, ip, previous, claimed, method).await
        }
        Redeem::Used => Err(code_used()),
        Redeem::Invalid => {
            ip_attempt.count();
            code_attempt.count();
            record_later(&state, failed(&claimed.1, ip, "recovery"));
            Err(invalid_code())
        }
    }
}

/// `Clear-Site-Data` of the sign-out: cache and storage of the whole origin,
/// the apps' too (docs/app-contract.md › Signing out). Not `cookies`: the
/// session cookie is cleared on its own, and apps keep their cookies.
const CLEAR_SITE_DATA: &str = "\"cache\", \"storage\"";

/// "Sign out": ends the session and clears the cookie, also without a valid
/// session (then there is nothing to do).
async fn logout(
    State(state): State<AppState>,
    ip: ClientIp,
    user: Option<CurrentUser>,
) -> Result<Response, ApiError> {
    if let Some(CurrentUser(user)) = user {
        db::call(&state.db, move |conn| {
            let tx = conn.transaction()?;
            end_own_session(&tx, &user, ip)?;
            tx.commit()?;
            Ok(())
        })
        .await?;
    }
    Ok((
        StatusCode::NO_CONTENT,
        [
            (header::SET_COOKIE, session::clear_cookie()),
            (
                header::HeaderName::from_static("clear-site-data"),
                HeaderValue::from_static(CLEAR_SITE_DATA),
            ),
        ],
    )
        .into_response())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::repo::users::{
        Role,
        tests::{add, conn},
    };

    fn checked_at(conn: &Connection, id: &str) -> String {
        users::credentials_by_id(conn, id)
            .unwrap()
            .unwrap()
            .password_changed_at
    }

    fn origin() -> Origin {
        Origin {
            user_agent: Some("Mozilla/5.0 (X11; Linux x86_64) Firefox/140.0".into()),
            ip: "198.51.100.56".parse().unwrap(),
            lang: None,
            device_hash: [7; 32],
        }
    }

    fn start(
        conn: &mut Connection,
        id: &str,
        checked_at: &str,
        previous: Option<&AuthUser>,
        method: &Method,
    ) -> Started {
        start_with(conn, id, checked_at, previous, method).0
    }

    /// Like [`start`], plus the templates of the notices.
    fn start_with(
        conn: &mut Connection,
        id: &str,
        checked_at: &str,
        previous: Option<&AuthUser>,
        method: &Method,
    ) -> (Started, Vec<&'static str>) {
        let mut outbox = Outbox::default();
        let started = start_session(
            conn,
            &mut outbox,
            id,
            checked_at,
            &origin(),
            true,
            previous,
            method,
            None,
        )
        .unwrap();
        (started, outbox.templates())
    }

    /// The first sign-in from a device sends "New sign-in", the next ones
    /// nothing; a recovery code always sends its own notice, which then names
    /// a new device.
    #[test]
    fn unknown_devices_and_recovery_codes_send_notices() {
        let mut conn = conn();
        let id = add(&conn, "max", "max@example.com", Role::User);
        let at = checked_at(&conn, &id);
        let (_, sent) = start_with(&mut conn, &id, &at, None, &Method::Password);
        assert_eq!(sent, ["security_login"]);
        let (_, sent) = start_with(&mut conn, &id, &at, None, &Method::Password);
        assert!(sent.is_empty(), "{sent:?}");

        let other = add(&conn, "eva", "eva@example.com", Role::User);
        let at = checked_at(&conn, &other);
        known_devices::remember(&conn, &other, &[7; 32]).unwrap();
        totp_devices::insert_pending(&conn, "d2", &other, b"secret").unwrap();
        totp_devices::confirm(&conn, "d2", "Phone", 0).unwrap();
        let method = Method::Recovery { remaining: 9 };
        let (_, sent) = start_with(&mut conn, &other, &at, None, &method);
        assert_eq!(sent, ["security_recovery_used"]);
    }

    fn count(conn: &Connection, table: &str) -> i64 {
        conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
            .unwrap()
    }

    #[test]
    fn no_session_after_concurrent_password_change_or_lock() {
        let mut conn = conn();
        let id = add(&conn, "max", "max@example.com", Role::User);
        let old = checked_at(&conn, &id);
        // The password was changed while the old one was being checked.
        conn.execute(
            "UPDATE users SET password_changed_at = '2099-01-01T00:00:00.000Z' WHERE id = ?1",
            [&id],
        )
        .unwrap();
        assert!(matches!(
            start(&mut conn, &id, &old, None, &Method::Password),
            Started::Expired
        ));
        // Locked in the meantime: the same answer as a wrong password.
        let now = checked_at(&conn, &id);
        conn.execute("UPDATE users SET status = 'locked' WHERE id = ?1", [&id])
            .unwrap();
        assert!(matches!(
            start(&mut conn, &id, &now, None, &Method::Password),
            Started::Locked
        ));
        assert_eq!(count(&conn, "sessions"), 0);
        assert_eq!(count(&conn, "audit_log"), 0);

        // The code was checked, then the account was locked and unlocked:
        // the discarded step starts no session, although the state fits.
        conn.execute("UPDATE users SET status = 'active' WHERE id = ?1", [&id])
            .unwrap();
        let store = std::sync::Arc::new(challenge::Challenges::default());
        let cookie = store
            .start(Challenge {
                user_id: id.clone(),
                remember: true,
                password_changed_at: now.clone(),
            })
            .unwrap();
        let pair = cookie.to_str().unwrap().split(';').next().unwrap();
        let mut headers = HeaderMap::new();
        headers.insert(header::COOKIE, HeaderValue::from_str(pair).unwrap());
        let claim = store.claim(&headers).unwrap();
        store.discard_user(&id);
        let (_, ticket) = claim.consume();
        let mut outbox = Outbox::default();
        let started = start_session(
            &mut conn,
            &mut outbox,
            &id,
            &now,
            &origin(),
            true,
            None,
            &Method::Password,
            Some(&ticket),
        )
        .unwrap();
        assert!(matches!(started, Started::Expired));
        assert_eq!(count(&conn, "sessions"), 0);

        // Otherwise signed in, with the current profile.
        conn.execute(
            "UPDATE users SET status = 'active', display_name = 'New' WHERE id = ?1",
            [&id],
        )
        .unwrap();
        let Started::Done(user, Some(_)) = start(&mut conn, &id, &now, None, &Method::Password)
        else {
            panic!("expected a new session");
        };
        assert_eq!(user.display_name, "New");
        assert_eq!(count(&conn, "sessions"), 1);
    }

    #[test]
    fn password_only_needs_code_once_an_app_is_confirmed() {
        let mut conn = conn();
        let id = add(&conn, "max", "max@example.com", Role::User);
        // An app was confirmed between the password check and the session.
        totp_devices::insert_pending(&conn, "d1", &id, b"secret").unwrap();
        totp_devices::confirm(&conn, "d1", "Phone", 0).unwrap();
        let at = checked_at(&conn, &id);
        assert!(matches!(
            start(&mut conn, &id, &at, None, &Method::Password),
            Started::Code
        ));
        assert_eq!(count(&conn, "sessions"), 0);
        // Completed with a code, the app of course applies.
        let method = Method::Totp {
            device: "Phone".into(),
        };
        assert!(matches!(
            start(&mut conn, &id, &at, None, &method),
            Started::Done(..)
        ));
        // The app was removed after the code was checked: no session.
        totp_devices::delete(&conn, &id, "d1").unwrap();
        for method in [method, Method::Recovery { remaining: 9 }] {
            assert!(matches!(
                start(&mut conn, &id, &at, None, &method),
                Started::Expired
            ));
        }
        assert_eq!(count(&conn, "sessions"), 1);
    }

    #[test]
    fn same_account_renews_the_session_another_replaces_it() {
        let mut conn = conn();
        let max = add(&conn, "max", "max@example.com", Role::User);
        let eva = add(&conn, "eva", "eva@example.com", Role::User);
        let at = checked_at(&conn, &max);
        let Started::Done(first, Some(_)) = start(&mut conn, &max, &at, None, &Method::Password)
        else {
            panic!("expected a new session");
        };
        conn.execute(
            "UPDATE sessions SET auth_time = '2026-01-01T00:00:00.000Z' WHERE id = ?1",
            [&first.session_id],
        )
        .unwrap();
        let old = AuthUser {
            auth_time: 1_767_225_600,
            ..first.clone()
        };

        let Started::Done(renewed, None) =
            start(&mut conn, &max, &at, Some(&old), &Method::Password)
        else {
            panic!("expected the same session");
        };
        assert_eq!(renewed.session_id, first.session_id);
        assert!(renewed.auth_time > old.auth_time);
        assert_eq!(count(&conn, "sessions"), 1);
        let action: String = conn
            .query_row(
                "SELECT action FROM audit_log ORDER BY id DESC LIMIT 1",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(action, "auth.reauth");

        // Another account in the same browser: the previous session ends.
        let other_at = checked_at(&conn, &eva);
        let Started::Done(switched, Some(_)) = start(
            &mut conn,
            &eva,
            &other_at,
            Some(&renewed),
            &Method::Password,
        ) else {
            panic!("expected a new session");
        };
        assert_ne!(switched.session_id, renewed.session_id);
        assert!(!sessions::is_valid(&conn, &max, &renewed.session_id).unwrap());
        assert_eq!(count(&conn, "sessions"), 1);

        // A renewal of a session that ended meanwhile starts a new one.
        let ended = AuthUser {
            session_id: "gone".into(),
            ..renewed
        };
        let Started::Done(fresh, Some(_)) =
            start(&mut conn, &max, &at, Some(&ended), &Method::Password)
        else {
            panic!("expected a new session");
        };
        assert_ne!(fresh.session_id, "gone");
    }
}
