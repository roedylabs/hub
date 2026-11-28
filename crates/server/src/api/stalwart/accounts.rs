//! Users' Stalwart accounts (Account › Accounts), shared by every app.
//!
//! For the apps (contract § 6.1):
//!
//! - `GET stalwart/accounts`: the own accounts ([`view`]) without tokens.
//! - `POST stalwart/accounts/:id/token`: an access token from the token
//!   broker ([`token`]); `{"refresh": true}` renews it although it is still
//!   valid (the server rejected it).
//!
//! For the hub's own pages:
//!
//! - `POST stalwart/sign-in`: starts the sign-in at a registry server with
//!   the hub's OAuth client: `state` and the PKCE verifier stay in the hub
//!   (`stalwart_sign_ins`, 10 minutes, bound to the session), the answer is
//!   the address of the server's sign-in page. With `account_id` it signs an
//!   existing account in again.
//! - `POST stalwart/sign-in/complete`: the page at `/oauth/callback` hands
//!   over `state` and `code` (or `error`); the hub exchanges the code itself,
//!   reads the account from the JMAP session and stores it – or, signing in
//!   again, checks that it is the same account. A grant it does not keep is
//!   revoked.
//! - `POST stalwart/accounts`: connects an account with an access token the
//!   user entered (`token`), at a registry server or – with the policy on –
//!   at a custom server (public addresses only, [`outbound`]).
//! - `POST stalwart/accounts/:id/reauth`: a new access token for such an
//!   account; it must name the same account.
//! - `PATCH stalwart/accounts/:id`: colour (`color`, index into `MA_COLORS`
//!   of `@roedy/tokens`) and `default: true`.
//! - `POST stalwart/accounts/:id/check`: session and storage (`Quota/get`)
//!   with an access token from the broker.
//! - `DELETE stalwart/accounts/:id`: removes the account; if it was the
//!   default account, the oldest remaining one becomes it. The broker then
//!   revokes the grant at the server.
//!
//! Adding, signing in again and removing give `stalwart.account.added`,
//! `.signed_in` and `.removed` to the apps that use Stalwart accounts. Deleting
//! a user revokes the grants of all their accounts ([`grants`],
//! [`revoke_grants`]).
//!
//! Taken over from Roedy Mail (`crates/server/src/api/mail_accounts.rs`).
//! Unlike Mail the hub starts the sign-in and exchanges every code itself;
//! the browser never sees a code verifier or a refresh token.

use std::{
    collections::BTreeSet,
    time::{Duration, Instant},
};

use axum::{
    Json, Router,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    routing::{get, post},
};
use roedy_jmap::Storage;
use rusqlite::Connection;
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};
use url::Url;

use super::servers::clean_optional;
use crate::{
    api::{ApiError, FieldErrors, JsonBody, require_requested_with},
    app::AppState,
    auth::{AuthUser, CurrentUser, email},
    db::{
        self,
        repo::{
            events::{self, Kind},
            sessions,
            stalwart_accounts::{
                self as accounts, ACCESS_PURPOSE, AuthKind, COLORS, Credentials, NewAccount,
                NewSignIn, SECRET_PURPOSE, SignedIn, State as AccountState, VERIFIER_PURPOSE,
            },
            stalwart_servers::{self as servers, Server},
        },
    },
    locks::KeyedLock,
    rate_limit::{Key, Policy},
    stalwart::{
        account, blocked, normalize_custom_url, normalize_url, oauth,
        outbound::{self, Blocked},
        redirect_uri, token,
        view::{self, AccountView},
    },
};

/// Every attempt queries the server.
const ADD_LIMIT: Policy = Policy::rate("stalwart.add", 10, Duration::from_secs(10 * 60));
/// Rejected codes and tokens per user.
const REJECTED_LIMIT: Policy = Policy::lockout(
    "stalwart.rejected",
    5,
    Duration::from_secs(15 * 60),
    Duration::from_secs(15 * 60),
);
/// Rejected codes and tokens per user over one hour: at most half as many as
/// lock the server for everyone ([`REJECTED_SERVER_LIMIT`]) – otherwise a
/// single user with wrong tokens would lock adding and signing in again for
/// all users of the server.
const REJECTED_HOUR_LIMIT: Policy = Policy::lockout(
    "stalwart.rejected_hour",
    10,
    Duration::from_secs(60 * 60),
    Duration::from_secs(60 * 60),
);
/// Rejected codes and tokens per server: Stalwart blocks addresses after
/// failed sign-ins (Fail2Ban) – here the hub's, and with it the token refresh
/// of all accounts on that server. Per server, so that a server that always
/// rejects (e.g. a custom one) does not lock the others.
const REJECTED_SERVER_LIMIT: Policy = Policy::lockout(
    "stalwart.rejected_server",
    20,
    Duration::from_secs(60 * 60),
    Duration::from_secs(60 * 60),
);

/// Access tokens per user: every app fetches one per account at start and
/// then before expiry, in every tab.
const TOKEN_LIMIT: Policy = Policy::rate("stalwart.token", 60, Duration::from_secs(60));
/// Refreshing on demand (`refresh`) queries the server every time.
const FORCE_REFRESH_LIMIT: Policy =
    Policy::rate("stalwart.token_refresh", 10, Duration::from_secs(10 * 60));
/// The check queries the server three to eight times (session with redirect,
/// `Quota/get`; after a 401 metadata and token endpoint, then once more).
const CHECK_LIMIT: Policy = Policy::rate("stalwart.check", 10, Duration::from_secs(60));

/// How long a sign-in at the server may take.
const SIGN_IN_VALID: Duration = Duration::from_secs(10 * 60);
/// Longest `return_to`.
const MAX_RETURN_TO: usize = 2048;
/// Where the browser goes after the sign-in when the page names nothing.
const DEFAULT_RETURN_TO: &str = "/account/accounts";

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/stalwart/accounts", get(list).post(create))
        .route(
            "/stalwart/accounts/{id}",
            axum::routing::patch(update).delete(remove),
        )
        .route("/stalwart/accounts/{id}/token", post(access_token))
        .route("/stalwart/accounts/{id}/reauth", post(reauth))
        .route("/stalwart/accounts/{id}/check", post(check))
        .route("/stalwart/sign-in", post(sign_in))
        .route("/stalwart/sign-in/complete", post(complete))
}

async fn list(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
) -> Result<Json<Value>, ApiError> {
    let accounts = db::call(&state.db, move |conn| view::of_user(conn, &user.id)).await?;
    Ok(Json(json!({ "accounts": accounts })))
}

/// Body of `POST …/token`, optional.
#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct TokenBody {
    /// The server rejected the access token: renew it although it is still
    /// valid.
    #[serde(default)]
    refresh: bool,
}

/// Access token for the browser; API responses carry `Cache-Control:
/// no-store`. The body is optional (without `Content-Type`).
async fn access_token(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    headers: HeaderMap,
    Path(id): Path<String>,
    body: Option<JsonBody<TokenBody>>,
) -> Result<Json<Value>, ApiError> {
    require_requested_with(&headers)?;
    let body = body.map(|JsonBody(body)| body).unwrap_or_default();
    state
        .limiter
        .acquire(&TOKEN_LIMIT, Key::User(user.id.clone()))?;
    if body.refresh {
        state
            .limiter
            .acquire(&FORCE_REFRESH_LIMIT, Key::User(user.id.clone()))?;
    }
    let access = token::access(&state, &user.id, &id, body.refresh)
        .await
        .map_err(token_error)?;
    Ok(Json(json!({
        "access_token": access.access_token,
        "expires_at": access.expires_at,
    })))
}

/// Token broker errors for `POST …/token` and `POST …/check` (also the
/// internal API's).
pub(crate) fn token_error(error: token::Error) -> ApiError {
    match error {
        token::Error::NotFound => ApiError::NotFound,
        token::Error::Blocked => ApiError::rejected(StatusCode::CONFLICT, "blocked"),
        token::Error::NeedsSignIn => ApiError::rejected(StatusCode::CONFLICT, "needs_sign_in"),
        token::Error::Refresh(error) => {
            ApiError::rejected(StatusCode::BAD_GATEWAY, "refresh_failed")
                .with("reason", error.reason())
        }
        token::Error::Limited(limited) => limited.into(),
        token::Error::Internal(error) => error.into(),
    }
}

/// Where an account belongs.
enum Target {
    Registry(Server),
    /// Custom server; public addresses only.
    Custom(Url),
}

impl Target {
    fn origin(&self) -> anyhow::Result<Url> {
        match self {
            Self::Registry(server) => Ok(Url::parse(&server.base_url)?),
            Self::Custom(url) => Ok(url.clone()),
        }
    }
}

/// Server from the registry or a custom address. A custom address with the
/// origin of a registry server counts as that server.
fn target(
    conn: &Connection,
    server_id: Option<&str>,
    url: Option<&str>,
    errors: &mut FieldErrors,
) -> anyhow::Result<Option<Target>> {
    let raw = match (server_id, url) {
        (Some(_), Some(_)) => {
            errors.code("url", "conflict");
            return Ok(None);
        }
        (Some(id), None) => {
            let found = servers::get(conn, id)?;
            if found.is_none() {
                errors.code("server_id", "not_found");
            }
            return Ok(found.map(Target::Registry));
        }
        (None, Some(raw)) => raw,
        (None, None) => {
            errors.code("server_id", "required");
            return Ok(None);
        }
    };
    if let Ok(url) = normalize_url(raw) {
        let origin = url.origin().ascii_serialization();
        if let Some(server) = servers::find_by_base_url(conn, &origin)? {
            return Ok(Some(Target::Registry(server)));
        }
    }
    let url = match normalize_custom_url(raw) {
        Ok(url) => url,
        Err(code) => {
            errors.code("url", code);
            return Ok(None);
        }
    };
    if !servers::allow_custom_servers(conn)? {
        errors.code("url", "not_allowed");
        return Ok(None);
    }
    Ok(Some(Target::Custom(url)))
}

/// Does the server still hold as [`target`] found it before the check at
/// the server? In between the administrator may have removed it, changed its
/// address or switched the policy off.
fn recheck(conn: &Connection, target: &Target) -> anyhow::Result<Result<(), FieldErrors>> {
    let mut errors = FieldErrors::default();
    match target {
        Target::Registry(server) => match servers::get(conn, &server.id)? {
            Some(now) if now.base_url == server.base_url => {}
            _ => errors.code("server_id", "not_found"),
        },
        Target::Custom(url) => {
            let origin = url.origin().ascii_serialization();
            let found = servers::find_by_base_url(conn, &origin)?.is_some();
            if !found && !servers::allow_custom_servers(conn)? {
                errors.code("url", "not_allowed");
            }
        }
    }
    Ok(if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    })
}

fn check_failed(status: StatusCode, reason: &'static str) -> ApiError {
    ApiError::rejected(status, "check_failed").with("reason", reason)
}

/// The server did not exchange the code: 422 if it rejects what the hub
/// sent (code, verifier or client), otherwise 502.
fn exchange_failed(error: &oauth::Error) -> ApiError {
    let status = match error {
        oauth::Error::InvalidGrant | oauth::Error::InvalidClient | oauth::Error::Rejected(_) => {
            StatusCode::UNPROCESSABLE_ENTITY
        }
        _ => StatusCode::BAD_GATEWAY,
    };
    let rejection = ApiError::rejected(status, "exchange_failed").with("reason", error.reason());
    match error {
        oauth::Error::Rejected(code) => rejection.with("detail", code.clone()),
        _ => rejection,
    }
}

/// The counters for rejected codes and tokens of a user at a server; if the
/// request is aborted, the reservations return the attempt.
struct Rejections<'a> {
    reservations: [crate::rate_limit::Reservation<'a>; 3],
}

impl<'a> Rejections<'a> {
    fn reserve(state: &'a AppState, user_id: &str, origin: &str) -> Result<Self, ApiError> {
        let user = Key::User(user_id.to_owned());
        Ok(Self {
            reservations: [
                state.limiter.reserve(&REJECTED_LIMIT, user.clone())?,
                state.limiter.reserve(&REJECTED_HOUR_LIMIT, user)?,
                state
                    .limiter
                    .reserve(&REJECTED_SERVER_LIMIT, Key::Server(origin.to_owned()))?,
            ],
        })
    }

    /// The server rejected the attempt.
    fn count(self) {
        for reservation in self.reservations {
            reservation.count();
        }
    }
}

/// A verified sign-in: the tokens and the account according to the session.
struct Verified {
    origin: String,
    /// Refresh token (`oauth`) or the user's access token (`token`).
    secret: String,
    /// `oauth`: access token and its lifetime from receipt.
    access: Option<(String, Option<u32>)>,
    received: Instant,
    account: account::Account,
}

/// Queries the server for the session of `bearer` ([`account::check`]).
/// Rejected tokens count against the limits (the server's Fail2Ban).
async fn verify_session(
    state: &AppState,
    user_id: &str,
    target: &Target,
    bearer: &str,
    hint: &str,
) -> Result<account::Account, ApiError> {
    let base_url = target.origin()?;
    let http = match target {
        Target::Registry(_) => &state.stalwart.http,
        Target::Custom(url) => {
            match outbound::check_public(url).await {
                Ok(()) => {}
                Err(Blocked::Private) => {
                    let mut errors = FieldErrors::default();
                    errors.code("url", "private");
                    return Err(errors.into_error());
                }
                Err(Blocked::Unresolvable) => {
                    return Err(check_failed(StatusCode::BAD_GATEWAY, "unreachable"));
                }
            }
            &state.stalwart.public_http
        }
    };
    let origin = base_url.origin().ascii_serialization();
    let rejections = Rejections::reserve(state, user_id, &origin)?;
    let result = account::check(http, &base_url, bearer, hint).await;
    match result {
        Ok(account) => {
            drop(rejections);
            Ok(account)
        }
        Err(error) => {
            if error.is_rejected() {
                rejections.count();
            } else {
                drop(rejections);
            }
            tracing::info!(user = %user_id, url = %origin, reason = error.reason(), "Stalwart account not verified");
            let status = if error.is_rejected() {
                StatusCode::UNPROCESSABLE_ENTITY
            } else {
                StatusCode::BAD_GATEWAY
            };
            Err(check_failed(status, error.reason()))
        }
    }
}

/// Encrypted tokens of a verified account (the ID is the AAD).
struct Sealed {
    secret_enc: Vec<u8>,
    access_token_enc: Option<Vec<u8>>,
    /// Lifetime of the access token from now.
    expires_in: Option<u32>,
    session_json: String,
}

fn seal(state: &AppState, id: &str, verified: &Verified) -> anyhow::Result<Sealed> {
    let secret_enc = state
        .secrets
        .seal(SECRET_PURPOSE, id, verified.secret.as_bytes())?;
    let (access_token_enc, expires_in) = match &verified.access {
        Some((access, expires_in)) => {
            // The time since receipt has already passed (rounded up).
            let elapsed =
                u32::try_from(verified.received.elapsed().as_secs() + 1).unwrap_or(u32::MAX);
            (
                Some(state.secrets.seal(ACCESS_PURPOSE, id, access.as_bytes())?),
                expires_in.map(|secs| secs.saturating_sub(elapsed)),
            )
        }
        None => (None, None),
    };
    Ok(Sealed {
        secret_enc,
        access_token_enc,
        expires_in,
        session_json: serde_json::to_string(&verified.account.session)?,
    })
}

/// A token: required, otherwise as [`oauth::token_problem`].
fn token_field(errors: &mut FieldErrors, field: &str, value: Option<String>) -> Option<String> {
    let value = value.map(|v| v.trim().to_owned()).unwrap_or_default();
    match oauth::token_problem(&value) {
        None => Some(value),
        Some(code) => {
            errors.code(field, code);
            None
        }
    }
}

/// The address entered: required, an email address, lower case.
fn hint_field(errors: &mut FieldErrors, value: &str) -> String {
    let hint = value.trim().to_lowercase();
    if hint.is_empty() {
        errors.code("email", "required");
    } else if !email::is_valid(&hint) {
        errors.code("email", "invalid");
    }
    hint
}

/// A path on the hub's origin to return to: starts with one `/`, no
/// backslash, no control characters.
fn return_to_field(errors: &mut FieldErrors, value: Option<String>) -> String {
    let Some(value) = value.map(|v| v.trim().to_owned()).filter(|v| !v.is_empty()) else {
        return DEFAULT_RETURN_TO.to_owned();
    };
    let valid = value.len() <= MAX_RETURN_TO
        && value.starts_with('/')
        && !value.starts_with("//")
        && !value.contains('\\')
        && !value.chars().any(char::is_control);
    if !valid {
        errors.code("return_to", "invalid");
    }
    value
}

/// 409 `dup` with the account that is connected already.
fn duplicate(existing: &accounts::Account) -> ApiError {
    // The page shows the address of the signed-in account and "open account".
    ApiError::rejected(StatusCode::CONFLICT, "dup").with(
        "account",
        json!({ "id": existing.id, "email": existing.email }),
    )
}

/// Event `stalwart.account.added` with the account as the apps see it.
fn account_added(conn: &Connection, user_id: &str, view: &AccountView) -> anyhow::Result<()> {
    events::to_stalwart_apps(
        conn,
        Kind::StalwartAccountAdded,
        &json!({ "sub": user_id, "account": view }),
    )?;
    Ok(())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SignInBody {
    /// Registry server of a new account.
    server_id: Option<String>,
    /// The address entered (`login_hint`); for a new account.
    email: Option<String>,
    /// "Sign in again" for this account.
    account_id: Option<String>,
    /// Path on the hub's origin to return to afterwards.
    return_to: Option<String>,
}

/// Starts the sign-in at a registry server: 200 with `authorize_url`. The
/// server needs the hub's OAuth client (`no_client` otherwise); signing in
/// again needs an account of the user at a registry server.
async fn sign_in(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    JsonBody(body): JsonBody<SignInBody>,
) -> Result<Json<Value>, ApiError> {
    let mut errors = FieldErrors::default();
    let return_to = return_to_field(&mut errors, body.return_to);
    let hint = match (&body.account_id, body.email.as_deref()) {
        (Some(_), _) => None,
        (None, email) => Some(hint_field(&mut errors, email.unwrap_or_default())),
    };
    let (user_id, account_id, server_id) =
        (user.id.clone(), body.account_id.clone(), body.server_id);
    let found = db::call(&state.db, move |conn| {
        let mut errors = FieldErrors::default();
        let (server, hint) = match &account_id {
            Some(id) => {
                let Some(account) = accounts::get(conn, &user_id, id)? else {
                    return Ok(Err(ApiError::NotFound));
                };
                let Some(server) = servers::find_by_base_url(conn, &account.base_url)? else {
                    let allow = servers::allow_custom_servers(conn)?;
                    let code = if blocked(&account.base_url, false, allow) {
                        "blocked"
                    } else {
                        "no_client"
                    };
                    return Ok(Err(ApiError::rejected(StatusCode::CONFLICT, code)));
                };
                if account.auth_kind != AuthKind::Oauth {
                    return Ok(Err(ApiError::rejected(
                        StatusCode::CONFLICT,
                        "token_account",
                    )));
                }
                (Some(server), Some(account.email))
            }
            None => {
                let server = match server_id.as_deref() {
                    None => {
                        errors.code("server_id", "required");
                        None
                    }
                    Some(id) => {
                        let found = servers::get(conn, id)?;
                        if found.is_none() {
                            errors.code("server_id", "not_found");
                        }
                        found
                    }
                };
                (server, None)
            }
        };
        if let Some(server) = &server
            && server.client_id.is_none()
        {
            errors.code("server_id", "no_client");
        }
        Ok(Ok((server, hint, errors)))
    })
    .await??;
    let (server, stored_hint, other) = found;
    errors.merge(other);
    let server = match server {
        Some(server) if errors.is_empty() => server,
        _ => return Err(errors.into_error()),
    };
    let hint = stored_hint.or(hint).unwrap_or_default();
    state
        .limiter
        .acquire(&ADD_LIMIT, Key::User(user.id.clone()))?;

    let client_id = server.client_id.clone().unwrap_or_default();
    let state_value = oauth::new_verifier()?;
    let verifier = oauth::new_verifier()?;
    let state_hash = hex(&Sha256::digest(state_value.as_bytes()));
    let redirect = redirect_uri(&state.public_url);
    let base_url = Url::parse(&server.base_url).map_err(anyhow::Error::from)?;
    let url = oauth::authorize_url(
        &state.stalwart.http,
        &base_url,
        &oauth::Authorize {
            client_id: &client_id,
            redirect_uri: &redirect,
            state: &state_value,
            verifier: &verifier,
            login_hint: Some(&hint),
        },
    )
    .await
    .map_err(|error| {
        tracing::info!(url = %server.base_url, reason = error.reason(), "sign-in at Stalwart not started");
        ApiError::rejected(StatusCode::BAD_GATEWAY, "sign_in_unavailable")
            .with("reason", error.reason())
    })?;
    let verifier_enc = state
        .secrets
        .seal(VERIFIER_PURPOSE, &state_hash, verifier.as_bytes())?;
    db::call(&state.db, move |conn| {
        Ok(accounts::insert_sign_in(
            conn,
            &NewSignIn {
                state_hash: &state_hash,
                user_id: &user.id,
                session_id: &user.session_id,
                server_id: &server.id,
                base_url: &server.base_url,
                client_id: &client_id,
                account_id: body.account_id.as_deref(),
                verifier_enc: &verifier_enc,
                login_hint: &hint,
                return_to: &return_to,
                valid_for: SIGN_IN_VALID,
            },
        )?)
    })
    .await?;
    Ok(Json(json!({ "authorize_url": url.as_str() })))
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes.iter().fold(String::new(), |mut out, byte| {
        let _ = write!(out, "{byte:02x}");
        out
    })
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CompleteBody {
    state: String,
    code: Option<String>,
    /// RFC 9207: the issuer of the response, if the server names it.
    iss: Option<String>,
    /// The server's error instead of a code (RFC 6749, section 4.1.2.1).
    error: Option<String>,
}

/// Completes the sign-in: 201 with the new account, 200 after signing in
/// again, each with `return_to`.
async fn complete(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    JsonBody(body): JsonBody<CompleteBody>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    let state_hash = hex(&Sha256::digest(body.state.trim().as_bytes()));
    let taken = db::call(&state.db, move |conn| {
        let Some(sign_in) = accounts::take_sign_in(conn, &state_hash)? else {
            return Ok(None);
        };
        let server = servers::get(conn, &sign_in.server_id)?;
        Ok(Some((sign_in, server, state_hash)))
    })
    .await?;
    let invalid_state = || ApiError::rejected(StatusCode::BAD_REQUEST, "invalid_state");
    let Some((sign_in, server, state_hash)) = taken else {
        return Err(invalid_state());
    };
    if sign_in.user_id != user.id || sign_in.session_id != user.session_id {
        // Started in another session (another sign-in in this browser
        // meanwhile): nothing is exchanged or stored.
        return Err(ApiError::rejected(StatusCode::CONFLICT, "session_changed"));
    }
    let return_to = sign_in.return_to.clone();
    let with_return = |error: ApiError| error.with("return_to", return_to.clone());
    if sign_in.expired {
        return Err(with_return(ApiError::rejected(
            StatusCode::GONE,
            "sign_in_expired",
        )));
    }
    if let Some(error) = body.error {
        let code = Some(error)
            .filter(|code| crate::stalwart::is_error_code(code))
            .unwrap_or_else(|| "unknown".into());
        tracing::info!(user = %user.id, url = %sign_in.base_url, reason = %code, "sign-in at Stalwart ended with an error");
        return Err(with_return(
            ApiError::rejected(StatusCode::UNPROCESSABLE_ENTITY, "sign_in_failed")
                .with("reason", code),
        ));
    }
    let mut errors = FieldErrors::default();
    let code = token_field(&mut errors, "code", body.code);
    let Some(code) = code.filter(|_| errors.is_empty()) else {
        return Err(errors.into_error());
    };
    // The administrator changed the server since the start: the client and
    // its secret belong to the old state.
    let server = match server {
        Some(server)
            if server.base_url == sign_in.base_url
                && server.client_id.as_deref() == Some(sign_in.client_id.as_str()) =>
        {
            server
        }
        _ => {
            return Err(with_return(ApiError::rejected(
                StatusCode::CONFLICT,
                "server_changed",
            )));
        }
    };
    let base_url = Url::parse(&server.base_url).map_err(anyhow::Error::from)?;
    if let Some(iss) = &body.iss
        && !is_issuer(&base_url, iss)
    {
        return Err(with_return(ApiError::rejected(
            StatusCode::BAD_REQUEST,
            "issuer_mismatch",
        )));
    }
    let verifier = state
        .secrets
        .open(VERIFIER_PURPOSE, &state_hash, &sign_in.verifier_enc)?;
    let verifier = String::from_utf8(verifier).map_err(anyhow::Error::from)?;
    let target = Target::Registry(server.clone());
    let _owner = lock_owner(&state, &user).await?;

    // Exchange the code; rejected codes count like rejected tokens.
    let origin = base_url.origin().ascii_serialization();
    let secret = token::client_secret(&state, &server, &sign_in.client_id).await?;
    let rejections = Rejections::reserve(&state, &user.id, &origin)?;
    let redirect = redirect_uri(&state.public_url);
    let exchanged = oauth::request(
        &state.stalwart.http,
        &base_url,
        oauth::Client {
            id: &sign_in.client_id,
            secret: secret.as_deref(),
        },
        oauth::Grant::Code {
            code: &code,
            verifier: &verifier,
            redirect_uri: &redirect,
        },
    )
    .await;
    let tokens = match exchanged {
        Ok(tokens) => {
            drop(rejections);
            tokens
        }
        Err(error) => {
            if error.is_rejected() {
                rejections.count();
            } else {
                drop(rejections);
            }
            tracing::info!(user = %user.id, url = %origin, reason = error.reason(), "code not exchanged");
            return Err(with_return(exchange_failed(&error)));
        }
    };
    let received = Instant::now();
    let Some(refresh) = tokens.refresh_token else {
        tracing::info!(user = %user.id, url = %origin, "code exchange without refresh token");
        return Err(with_return(
            ApiError::rejected(StatusCode::BAD_GATEWAY, "exchange_failed")
                .with("reason", "no_refresh_token"),
        ));
    };
    let discarded = token::Discarded {
        server: server.clone(),
        client_id: sign_in.client_id.clone(),
        client_secret: secret.clone(),
        refresh_token: refresh.clone(),
    };
    let account = match verify_session(
        &state,
        &user.id,
        &target,
        &tokens.access_token,
        &sign_in.login_hint,
    )
    .await
    {
        Ok(account) => account,
        Err(error) => {
            // The grant just exchanged is of no use without the account.
            token::revoke_discarded(&state, &user.id, discarded);
            return Err(with_return(error));
        }
    };
    let verified = Verified {
        origin,
        secret: refresh,
        access: Some((tokens.access_token, tokens.expires_in)),
        received,
        account,
    };
    let saved = match sign_in.account_id {
        Some(id) => signed_in_again(
            &state,
            &user,
            &id,
            target,
            verified,
            Some(&sign_in.client_id),
        )
        .await
        .map(|view| (StatusCode::OK, view)),
        None => add(
            &state,
            &user,
            target,
            verified,
            AuthKind::Oauth,
            Some(&sign_in.client_id),
        )
        .await
        .map(|view| (StatusCode::CREATED, view)),
    };
    match saved {
        Ok((status, view)) => Ok((
            status,
            Json(json!({ "account": view, "return_to": sign_in.return_to })),
        )),
        Err(error) => {
            // Nobody else holds the refresh token of the exchange.
            if error.discard {
                token::revoke_discarded(&state, &user.id, discarded);
            }
            Err(with_return(error.error))
        }
    }
}

/// `iss` of the response (RFC 9207) is the server's origin, at most with a
/// trailing `/` – as `issuer` of its metadata (`probe`).
fn is_issuer(base_url: &Url, iss: &str) -> bool {
    Url::parse(iss).is_ok_and(|url| {
        url.origin() == base_url.origin()
            && url.path() == "/"
            && url.query().is_none()
            && url.fragment().is_none()
            && url.username().is_empty()
            && url.password().is_none()
    })
}

/// Why a verified sign-in was not saved; with `discard` the sign-in is of no
/// use (the account is connected already, another account, removed
/// meanwhile), and a grant from a code exchange is revoked.
struct NotSaved {
    error: ApiError,
    discard: bool,
}

impl From<ApiError> for NotSaved {
    fn from(error: ApiError) -> Self {
        Self {
            error,
            discard: false,
        }
    }
}

impl From<anyhow::Error> for NotSaved {
    fn from(error: anyhow::Error) -> Self {
        ApiError::from(error).into()
    }
}

/// Stores a new verified account under the user's lock, with the event.
async fn add(
    state: &AppState,
    user: &AuthUser,
    target: Target,
    verified: Verified,
    kind: AuthKind,
    client_id: Option<&str>,
) -> Result<AccountView, NotSaved> {
    // The ID is fixed before encrypting (it is the AAD).
    let id = db::new_id()?;
    let sealed = seal(state, &id, &verified)?;
    let Verified {
        origin, account, ..
    } = verified;
    let (user_id, client_id) = (user.id.clone(), client_id.map(str::to_owned));
    let saved = db::call(&state.db, move |conn| {
        let tx = conn.transaction()?;
        if let Err(errors) = recheck(&tx, &target)? {
            // Nothing is stored: a grant from the exchange is of no use.
            return Ok(Err(NotSaved {
                error: errors.into_error(),
                discard: true,
            }));
        }
        if let Some(existing) = accounts::find(&tx, &user_id, &origin, &account.account_id)? {
            return Ok(Err(NotSaved {
                error: duplicate(&existing),
                discard: true,
            }));
        }
        let before = accounts::list(&tx, &user_id)?;
        let stored = accounts::insert(
            &tx,
            &NewAccount {
                id: &id,
                user_id: &user_id,
                base_url: &origin,
                email: &account.email,
                account_id: &account.account_id,
                username: &account.username,
                auth_kind: kind,
                client_id: client_id.as_deref(),
                secret_enc: &sealed.secret_enc,
                access_token_enc: sealed.access_token_enc.as_deref(),
                expires_in: sealed.expires_in,
                session_url: &account.session_url,
                session_json: &sealed.session_json,
                color: accounts::free_color(&before),
            },
        )?;
        let view = view::current(&tx, stored)?;
        account_added(&tx, &user_id, &view)?;
        tx.commit()?;
        Ok(Ok(view))
    })
    .await
    .map_err(NotSaved::from)??;
    tracing::info!(user = %user.id, account = %saved.id, url = %saved.url, kind = saved.auth, "Stalwart account connected");
    Ok(saved)
}

/// Replaces the tokens of an account with those of a verified sign-in under
/// the user's lock; the session must name the same account – account ID and
/// username – otherwise the entry including colour and default would
/// silently read someone else's data. The previous grant is revoked.
async fn signed_in_again(
    state: &AppState,
    user: &AuthUser,
    id: &str,
    target: Target,
    verified: Verified,
    client_id: Option<&str>,
) -> Result<AccountView, NotSaved> {
    let (user_id, row) = (user.id.clone(), id.to_owned());
    let stored = db::call(&state.db, move |conn| {
        Ok(accounts::get(conn, &user_id, &row)?)
    })
    .await?
    .ok_or(NotSaved {
        error: ApiError::NotFound,
        discard: true,
    })?;
    if verified.account.account_id != stored.account_id
        || verified.account.username != stored.username
        || verified.origin != stored.base_url
    {
        tracing::info!(user = %user.id, account = %id, "sign-in again with a different account rejected");
        // The page reports "you signed in as ‹address›".
        return Err(NotSaved {
            error: ApiError::rejected(StatusCode::CONFLICT, "other_account")
                .with("account", json!({ "email": verified.account.email })),
            discard: true,
        });
    }
    let sealed = seal(state, id, &verified)?;
    // Without the account's lock: a refresh may run at the same time and
    // follows the new tokens afterwards. The refresh token stored at the time
    // of replacing is revoked; with it Stalwart revokes the grant across all
    // generations, including one the refresh still receives.
    let (user_id, row) = (user.id.clone(), id.to_owned());
    let (client_id, session_url) = (
        client_id.map(str::to_owned),
        verified.account.session_url.clone(),
    );
    let saved = db::call(&state.db, move |conn| {
        let tx = conn.transaction()?;
        if let Err(errors) = recheck(&tx, &target)? {
            // Nothing is stored: a grant from the exchange is of no use.
            return Ok(Err(NotSaved {
                error: errors.into_error(),
                discard: true,
            }));
        }
        let previous = accounts::credentials(&tx, &row, Some(&user_id))?
            .map(|creds| grant(&tx, creds))
            .transpose()?
            .flatten();
        let replaced = accounts::signed_in_again(
            &tx,
            &user_id,
            &row,
            &SignedIn {
                client_id: client_id.as_deref(),
                secret_enc: &sealed.secret_enc,
                access_token_enc: sealed.access_token_enc.as_deref(),
                expires_in: sealed.expires_in,
                session_url: &session_url,
                session_json: &sealed.session_json,
            },
        )?;
        // Removed in the meantime.
        let Some(before) = replaced else {
            return Ok(Err(NotSaved {
                error: ApiError::NotFound,
                discard: true,
            }));
        };
        if before.state == AccountState::NeedsSignIn {
            events::stalwart_account(&tx, Kind::StalwartAccountSignedIn, &user_id, &row)?;
        }
        let account = accounts::get(&tx, &user_id, &row)?
            .ok_or_else(|| anyhow::anyhow!("Stalwart account missing"))?;
        let view = view::current(&tx, account)?;
        tx.commit()?;
        Ok(Ok((view, previous)))
    })
    .await
    .map_err(NotSaved::from)?;
    let (view, previous) = saved?;
    tracing::info!(user = %user.id, account = %view.id, "Stalwart account signed in again");
    // Otherwise the previous grant stays valid until it expires – unless it
    // is the same token again.
    if let Some(previous) = previous
        && !same_secret(state, &previous.creds, &verified.secret)
    {
        revoke_grants(state, vec![previous]);
    }
    Ok(view)
}

/// Whether the stored refresh token or access token is `secret`.
fn same_secret(state: &AppState, creds: &Credentials, secret: &str) -> bool {
    state
        .secrets
        .open(SECRET_PURPOSE, &creds.account.id, &creds.secret_enc)
        .is_ok_and(|stored| stored == secret.as_bytes())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateBody {
    /// Registry server …
    server_id: Option<String>,
    /// … or custom server ("Other server …"); not both.
    url: Option<String>,
    /// The address entered; the session tells which one applies.
    email: String,
    /// The user's access token (e.g. an API key).
    token: Option<String>,
}

/// Connects an account with an access token: 201 with the account.
async fn create(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    JsonBody(body): JsonBody<CreateBody>,
) -> Result<(StatusCode, Json<AccountView>), ApiError> {
    let received = Instant::now();
    let mut errors = FieldErrors::default();
    let hint = hint_field(&mut errors, &body.email);
    let token = token_field(&mut errors, "token", body.token);
    let (server_id, url) = (clean_optional(body.server_id), clean_optional(body.url));
    let target = db::call(&state.db, move |conn| {
        let mut errors = FieldErrors::default();
        Ok(target(conn, server_id.as_deref(), url.as_deref(), &mut errors)?.ok_or(errors))
    })
    .await?;
    let target = match (target, token) {
        (Ok(target), Some(token)) if errors.is_empty() => (target, token),
        (target, _) => {
            if let Err(other) = target {
                errors.merge(other);
            }
            return Err(errors.into_error());
        }
    };
    let (target, token) = target;
    state
        .limiter
        .acquire(&ADD_LIMIT, Key::User(user.id.clone()))?;
    let _owner = lock_owner(&state, &user).await?;
    let account = verify_session(&state, &user.id, &target, &token, &hint).await?;
    let verified = Verified {
        origin: target.origin()?.origin().ascii_serialization(),
        secret: token,
        access: None,
        received,
        account,
    };
    let view = add(&state, &user, target, verified, AuthKind::Token, None)
        .await
        .map_err(|not_saved| not_saved.error)?;
    Ok((StatusCode::CREATED, Json(view)))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReauthBody {
    token: Option<String>,
}

/// A new access token for an account connected with one (`token`); accounts
/// signed in at the server use `POST stalwart/sign-in` with `account_id`
/// (409 `oauth_account`).
async fn reauth(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(id): Path<String>,
    JsonBody(body): JsonBody<ReauthBody>,
) -> Result<Json<AccountView>, ApiError> {
    let received = Instant::now();
    let mut errors = FieldErrors::default();
    let token = token_field(&mut errors, "token", body.token);
    let (user_id, row) = (user.id.clone(), id.clone());
    let found = db::call(&state.db, move |conn| {
        let Some(account) = accounts::get(conn, &user_id, &row)? else {
            return Ok(None);
        };
        let server = servers::find_by_base_url(conn, &account.base_url)?;
        let allow = servers::allow_custom_servers(conn)?;
        Ok(Some((account, server, allow)))
    })
    .await?;
    let (stored, server, allow) = found.ok_or(ApiError::NotFound)?;
    if stored.auth_kind != AuthKind::Token {
        return Err(ApiError::rejected(StatusCode::CONFLICT, "oauth_account"));
    }
    let target = match server {
        Some(server) => Target::Registry(server),
        None if blocked(&stored.base_url, false, allow) => {
            return Err(ApiError::rejected(StatusCode::CONFLICT, "blocked"));
        }
        None => Target::Custom(Url::parse(&stored.base_url).map_err(anyhow::Error::from)?),
    };
    let Some(token) = token.filter(|_| errors.is_empty()) else {
        return Err(errors.into_error());
    };
    state
        .limiter
        .acquire(&ADD_LIMIT, Key::User(user.id.clone()))?;
    let _owner = lock_owner(&state, &user).await?;
    let account = verify_session(&state, &user.id, &target, &token, &stored.email).await?;
    let verified = Verified {
        origin: stored.base_url.clone(),
        secret: token,
        access: None,
        received,
        account,
    };
    signed_in_again(&state, &user, &id, target, verified, None)
        .await
        .map(Json)
        .map_err(|not_saved| not_saved.error)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UpdateBody {
    color: Option<i64>,
    /// `true` makes the account the default account; there is always one
    /// while the user has accounts, so `false` is not accepted.
    default: Option<bool>,
}

/// Colour and default account; without fields everything stays as it is.
async fn update(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(id): Path<String>,
    JsonBody(body): JsonBody<UpdateBody>,
) -> Result<Json<AccountView>, ApiError> {
    let mut errors = FieldErrors::default();
    if body
        .color
        .is_some_and(|color| !(0..COLORS).contains(&color))
    {
        errors.code("color", "invalid");
    }
    if body.default == Some(false) {
        errors.code("default", "invalid");
    }
    if !errors.is_empty() {
        return Err(errors.into_error());
    }
    let updated = db::call(&state.db, move |conn| {
        let tx = conn.transaction()?;
        if !accounts::exists(&tx, &user.id, &id)? {
            return Ok(None);
        }
        if let Some(color) = body.color {
            accounts::set_color(&tx, &user.id, &id, color)?;
        }
        if body.default == Some(true) {
            accounts::set_default(&tx, &user.id, &id)?;
        }
        let account = accounts::get(&tx, &user.id, &id)?;
        tx.commit()?;
        account
            .map(|account| view::current(conn, account))
            .transpose()
    })
    .await?;
    updated.map(Json).ok_or(ApiError::NotFound)
}

/// The account's check: access token from the broker, with it session and
/// `Quota/get`; if the server rejects an access token from the sign-in, once
/// more with a refreshed one (like the browser). Broker errors as for `POST
/// …/token`, server errors 502 `check_failed` with `reason` – the account
/// stays as it is.
async fn check(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(id): Path<String>,
) -> Result<Json<AccountView>, ApiError> {
    state
        .limiter
        .acquire(&CHECK_LIMIT, Key::User(user.id.clone()))?;
    let (user_id, row) = (user.id.clone(), id.clone());
    let found = db::call(&state.db, move |conn| {
        let Some(account) = accounts::get(conn, &user_id, &row)? else {
            return Ok(None);
        };
        let registered = servers::find_by_base_url(conn, &account.base_url)?.is_some();
        Ok(Some((account, registered)))
    })
    .await?;
    let (account, registered) = found.ok_or(ApiError::NotFound)?;
    let base_url = Url::parse(&account.base_url).map_err(anyhow::Error::from)?;
    let http = state.stalwart.http(registered);
    let mut force = false;
    let checked = loop {
        let access = token::access(&state, &user.id, &id, force)
            .await
            .map_err(token_error)?;
        let result = account::storage(http, &base_url, access.access_token).await;
        // Force a refresh like `POST …/token` with `refresh`, with the same
        // limit; once it is reached, the rejection stands.
        let retry = !force
            && account.auth_kind == AuthKind::Oauth
            && result == Err(roedy_jmap::Error::AuthFailed)
            && state
                .limiter
                .acquire(&FORCE_REFRESH_LIMIT, Key::User(user.id.clone()))
                .is_ok();
        if !retry {
            break result;
        }
        force = true;
    };
    let storage = checked.map_err(|error| {
        tracing::info!(user = %user.id, account = %id, reason = error.reason(), "account check failed");
        check_failed(StatusCode::BAD_GATEWAY, error.reason())
    })?;
    let quota = storage.map(|Storage { used, total }| (used, total));
    let user_id = user.id.clone();
    let saved = db::call(&state.db, move |conn| {
        accounts::store_check(conn, &user_id, &id, quota)?
            .map(|account| view::current(conn, account))
            .transpose()
    })
    .await?;
    saved.map(Json).ok_or(ApiError::NotFound)
}

async fn remove(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    // First the own account, then its lock (foreign IDs lock nothing): no
    // refresh runs while the account disappears, and the last stored refresh
    // token is revoked.
    let (user_id, account) = (user.id.clone(), id.clone());
    let own = db::call(&state.db, move |conn| {
        Ok(accounts::exists(conn, &user_id, &account)?)
    })
    .await?;
    if !own {
        return Err(ApiError::NotFound);
    }
    let _lock = state.stalwart.token_locks.lock(&id).await;
    let (user_id, account) = (user.id.clone(), id.clone());
    let removed = db::call(&state.db, move |conn| {
        let tx = conn.transaction()?;
        let Some(creds) = accounts::credentials(&tx, &account, Some(&user_id))? else {
            return Ok(None);
        };
        if !accounts::delete(&tx, &user_id, &account)? {
            return Ok(None);
        }
        let grant = grant(&tx, creds)?;
        events::stalwart_account(&tx, Kind::StalwartAccountRemoved, &user_id, &account)?;
        tx.commit()?;
        Ok(Some(grant))
    })
    .await?;
    let Some(grant) = removed else {
        return Err(ApiError::NotFound);
    };
    tracing::info!(user = %user.id, account = %id, "Stalwart account removed");
    revoke_grants(&state, grant.into_iter().collect());
    Ok(StatusCode::NO_CONTENT)
}

/// A grant the token broker revokes: account with tokens and the registry
/// server with its origin.
pub(crate) struct Grant {
    creds: Credentials,
    server: Option<Server>,
}

/// The grant of an account, read in the transaction that removes it or
/// replaces its tokens; `None` for blocked accounts – the hub does not query
/// their servers.
fn grant(conn: &Connection, creds: Credentials) -> anyhow::Result<Option<Grant>> {
    let server = servers::find_by_base_url(conn, &creds.account.base_url)?;
    let allow = servers::allow_custom_servers(conn)?;
    Ok((!blocked(&creds.account.base_url, server.is_some(), allow))
        .then_some(Grant { creds, server }))
}

/// The grants of all accounts of a user, in the transaction that deletes
/// them (`DELETE account`, removal by an administrator). The caller takes
/// the locks beforehand ([`lock_owners`]).
pub(crate) fn grants(conn: &Connection, user_id: &str) -> anyhow::Result<Vec<Grant>> {
    let mut grants = Vec::new();
    for creds in accounts::credentials_of_user(conn, user_id)? {
        if let Some(grant) = grant(conn, creds)? {
            grants.push(grant);
        }
    }
    Ok(grants)
}

/// Key of a user's lock in `token_locks` (IDs are ULIDs, without `:`):
/// adding and signing in again hold it from the code exchange until saving
/// ([`lock_owner`]), [`lock_owners`] takes it before the accounts' locks.
fn owner_key(user_id: &str) -> String {
    format!("user:{user_id}")
}

/// Takes the user's lock ([`owner_key`]) for adding an account or signing in
/// again, held until after saving: removing the user waits that long and
/// then revokes this grant too; if the removal was faster, the session is no
/// longer valid and no grant is created that nobody would revoke. Whoever
/// holds it takes no account lock, so the order of [`lock_owners`] (users,
/// then accounts) holds.
async fn lock_owner(state: &AppState, user: &AuthUser) -> Result<KeyedLock, ApiError> {
    let lock = state.stalwart.token_locks.lock(&owner_key(&user.id)).await;
    let (owner, session_id) = (user.id.clone(), user.session_id.clone());
    if !db::call(&state.db, move |conn| {
        Ok(sessions::is_valid(conn, &owner, &session_id)?)
    })
    .await?
    {
        return Err(ApiError::Unauthorized);
    }
    Ok(lock)
}

/// Locks for removing the users `user_ids` (`DELETE account`, removal by an
/// administrator): first the users – a running sign-in saves before, a new
/// one waits –, then their accounts – no refresh runs while they disappear,
/// and the last stored refresh token is revoked. Both in ascending order so
/// that two removals never block each other; whoever holds an account's lock
/// never waits for a user's.
pub(crate) async fn lock_owners(
    state: &AppState,
    user_ids: impl IntoIterator<Item = String>,
) -> anyhow::Result<Vec<KeyedLock>> {
    let user_ids: BTreeSet<String> = user_ids.into_iter().collect();
    let mut locks = Vec::new();
    for id in &user_ids {
        locks.push(state.stalwart.token_locks.lock(&owner_key(id)).await);
    }
    let accounts = db::call(&state.db, move |conn| {
        let mut accounts = BTreeSet::new();
        for id in &user_ids {
            accounts.extend(accounts::list(conn, id)?.into_iter().map(|a| a.id));
        }
        Ok(accounts)
    })
    .await?;
    for id in &accounts {
        locks.push(state.stalwart.token_locks.lock(id).await);
    }
    Ok(locks)
}

/// Revokes the grants after the commit, each in its own task
/// ([`token::revoke`]).
pub(crate) fn revoke_grants(state: &AppState, grants: Vec<Grant>) {
    for Grant { creds, server } in grants {
        token::revoke(state, creds, server);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn code(value: Option<&str>) -> (String, Value) {
        let mut errors = FieldErrors::default();
        let path = return_to_field(&mut errors, value.map(str::to_owned));
        (path, errors.into_value())
    }

    #[test]
    fn return_paths_stay_on_the_origin() {
        assert_eq!(code(None).0, DEFAULT_RETURN_TO);
        assert_eq!(code(Some(" ")).0, DEFAULT_RETURN_TO);
        assert_eq!(
            code(Some("/mail/?a=1#b")),
            ("/mail/?a=1#b".into(), json!({}))
        );
        for bad in [
            "//evil.example",
            "https://evil.example",
            "/\\evil",
            "mail",
            "/a\nb",
        ] {
            assert_eq!(
                code(Some(bad)).1,
                json!({ "return_to": { "code": "invalid" } }),
                "{bad:?}"
            );
        }
    }

    #[test]
    fn issuers_are_the_servers_origin() {
        let base = Url::parse("https://mail.example.com").unwrap();
        assert!(is_issuer(&base, "https://mail.example.com"));
        assert!(is_issuer(&base, "https://mail.example.com/"));
        for other in [
            "https://other.example",
            "http://mail.example.com",
            "https://mail.example.com/realm",
            "nonsense",
        ] {
            assert!(!is_issuer(&base, other), "{other}");
        }
    }

    #[test]
    fn state_hashes_are_hex() {
        assert_eq!(hex(&[0, 15, 255]), "000fff");
    }
}
