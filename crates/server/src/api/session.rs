//! `GET session`: who is signed in, for the apps' web front ends on the same
//! origin (docs/app-contract.md › Browser API of the hub). Part of the
//! contract: fields are only added within version 1.

use axum::{Json, Router, extract::State, routing::get};
use serde_json::{Value, json};

use super::ApiError;
use crate::{app::AppState, auth::CurrentUser, db, settings};

/// Name of the hub until the branding names an organization.
pub const DEFAULT_NAME: &str = "Roedy Hub";

pub fn routes() -> Router<AppState> {
    Router::new().route("/session", get(current))
}

/// `200` with user, session and the hub's links while signed in, otherwise
/// `401 unauthorized`. `locale` is the user's language (`en` or `de`, for
/// `system` the one of the last sign-in), `theme` the color scheme (`system`
/// without a choice).
async fn current(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
) -> Result<Json<Value>, ApiError> {
    let id = user.id.clone();
    let stored = db::call(&state.db, move |conn| settings::load(conn, &id)).await?;
    Ok(Json(json!({
        "user": {
            "id": user.id,
            "username": user.username,
            "email": user.email,
            "name": user.display_name,
            "role": user.role.as_str(),
            "locale": stored.lang.code(),
            "theme": stored.settings.theme().as_str(),
        },
        "session": { "id": user.session_id, "auth_time": user.auth_time },
        "hub": {
            "name": DEFAULT_NAME,
            "home": "/",
            "account": "/account",
            "admin": user.is_admin().then_some("/admin"),
        },
    })))
}
