//! Stalwart accounts for an app's backend (docs/app-contract.md › Internal
//! API), only for apps whose manifest has `stalwart` (otherwise `403
//! not_requested`):
//!
//! - `GET /internal/v1/stalwart/servers`: the origins of the registry and
//!   whether custom servers are allowed – what the app's `connect-src`
//!   needs.
//! - `GET /internal/v1/users/<sub>/stalwart-accounts`: a user's accounts as
//!   in the browser API, only for users with access to the app.
//! - `POST /internal/v1/users/<sub>/stalwart-accounts/<id>/token`: an access
//!   token from the token broker, only with `stalwart.backend` in the
//!   manifest and only for active users with access to the app. It does not
//!   count as a browser fetch (no early refresh for it).

use axum::{
    Json,
    body::Bytes,
    extract::{Path, State},
    http::StatusCode,
};
use roedy_hub_contract::Stalwart;
use serde::Deserialize;
use serde_json::{Value, json};

use super::PairedApp;
use crate::{
    api::{ApiError, stalwart::accounts::token_error},
    app::AppState,
    db::{
        self,
        repo::{app_access, apps::App, stalwart_servers as servers},
    },
    rate_limit::{Key, Policy},
    stalwart::{token, view},
};

/// Access tokens per app and user.
const TOKEN_LIMIT: Policy = Policy::rate(
    "stalwart.internal_token",
    60,
    std::time::Duration::from_secs(60),
);

/// Refreshing on demand (`refresh`) per app and user.
const FORCE_REFRESH_LIMIT: Policy = Policy::rate(
    "stalwart.internal_token_refresh",
    10,
    std::time::Duration::from_secs(10 * 60),
);

/// What the app's manifest asks for; `403 not_requested` without
/// `stalwart`.
fn requested(app: &App) -> Result<Stalwart, ApiError> {
    app.manifest
        .as_ref()
        .and_then(|manifest| manifest.stalwart)
        .ok_or_else(not_requested)
}

fn not_requested() -> ApiError {
    ApiError::rejected(StatusCode::FORBIDDEN, "not_requested")
}

pub async fn servers(
    State(state): State<AppState>,
    PairedApp(app, _): PairedApp,
) -> Result<Json<Value>, ApiError> {
    requested(&app)?;
    let (origins, custom) = db::call(&state.db, |conn| {
        Ok((
            servers::base_urls(conn)?,
            servers::allow_custom_servers(conn)?,
        ))
    })
    .await?;
    Ok(Json(json!({ "origins": origins, "custom": custom })))
}

/// The user's access to the app: `Ok(active)`, or `404 no_access`.
fn access(conn: &rusqlite::Connection, app: &App, sub: &str) -> anyhow::Result<Option<bool>> {
    Ok(app_access::member_of(conn, &app.id, sub)?.map(|member| member.active))
}

fn no_access() -> ApiError {
    ApiError::rejected(StatusCode::NOT_FOUND, "no_access")
}

pub async fn accounts(
    State(state): State<AppState>,
    PairedApp(app, _): PairedApp,
    Path(sub): Path<String>,
) -> Result<Json<Value>, ApiError> {
    requested(&app)?;
    let found = db::call(&state.db, move |conn| {
        if access(conn, &app, &sub)?.is_none() {
            return Ok(None);
        }
        Ok(Some(view::of_user(conn, &sub)?))
    })
    .await?;
    let accounts = found.ok_or_else(no_access)?;
    Ok(Json(json!({ "accounts": accounts })))
}

/// Body of `POST …/token`, optional.
#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct TokenBody {
    #[serde(default)]
    refresh: bool,
}

pub async fn access_token(
    State(state): State<AppState>,
    PairedApp(app, _): PairedApp,
    Path((sub, id)): Path<(String, String)>,
    body: Bytes,
) -> Result<Json<Value>, ApiError> {
    if !requested(&app)?.backend {
        return Err(not_requested());
    }
    let body: TokenBody = if body.iter().all(u8::is_ascii_whitespace) {
        TokenBody::default()
    } else {
        serde_json::from_slice(&body).map_err(|error| ApiError::BadRequest(error.to_string()))?
    };
    let (app_id, user) = (app.id.clone(), sub.clone());
    let active = db::call(&state.db, move |conn| access(conn, &app, &user)).await?;
    match active {
        None => return Err(no_access()),
        Some(false) => return Err(ApiError::rejected(StatusCode::FORBIDDEN, "user_locked")),
        Some(true) => {}
    }
    let key = Key::User(format!("{app_id}:{sub}"));
    state.limiter.acquire(&TOKEN_LIMIT, key.clone())?;
    if body.refresh {
        state.limiter.acquire(&FORCE_REFRESH_LIMIT, key)?;
    }
    let access = token::for_backend(&state, &id, Some(&sub), body.refresh)
        .await
        .map_err(token_error)?;
    Ok(Json(json!({
        "access_token": access.access_token,
        "expires_at": access.expires_at,
    })))
}
