//! `GET directory`: the instance's active users for the apps' web front ends
//! (docs/app-contract.md › Browser API of the hub), e.g. for suggestions
//! while writing a mail or sharing a calendar. Part of the contract: fields
//! are only added within version 1.
//!
//! Every signed-in user sees every active user with ID, display name and
//! email address – what colleagues in one organization see of each other
//! anyway. Locked users are left out; deleted users and open invitations are
//! no users. Login names, roles, status and settings stay with the
//! administration and the apps' backends (`GET /internal/v1/users`).
//!
//! `?app=<slug>` lists only the users with access to that app; the caller
//! needs access to it as well (otherwise `403 no_access`), and the app must
//! be paired and switched on (otherwise `404 not_found`, like an unknown
//! slug).
//!
//! Taken over from Roedy Mail (`crates/server/src/api/directory.rs`) with the
//! user's ID and the filter by app.

use axum::{
    Json, Router,
    extract::{Query, State, rejection::QueryRejection},
    http::StatusCode,
    routing::get,
};
use serde::Deserialize;
use serde_json::{Value, json};

use super::ApiError;
use crate::{
    app::AppState,
    auth::CurrentUser,
    db::{
        self,
        repo::{
            app_access,
            apps::{self as repo, Status},
            users,
        },
    },
};

pub fn routes() -> Router<AppState> {
    Router::new().route("/directory", get(list))
}

#[derive(Deserialize)]
struct Filter {
    app: Option<String>,
}

/// `200 {"users": [{"id", "name", "email"}]}`, sorted by name.
async fn list(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    filter: Result<Query<Filter>, QueryRejection>,
) -> Result<Json<Value>, ApiError> {
    let Query(filter) = filter.map_err(|rejection| ApiError::BadRequest(rejection.body_text()))?;
    let entries = db::call(&state.db, move |conn| {
        let app_id = match filter.app.as_deref() {
            None => None,
            Some(slug) => {
                let app = repo::by_slug(conn, slug)?
                    .filter(|app| app.status == Status::Paired && app.enabled());
                let Some(app) = app else {
                    return Ok(Err(ApiError::NotFound));
                };
                if app_access::role_of(conn, &user.id, &app.id)?.is_none() {
                    return Ok(Err(ApiError::rejected(StatusCode::FORBIDDEN, "no_access")));
                }
                Some(app.id)
            }
        };
        Ok(Ok(users::directory(conn, app_id.as_deref())?))
    })
    .await??;
    let users: Vec<Value> = entries
        .into_iter()
        .map(|entry| json!({ "id": entry.id, "name": entry.name, "email": entry.email }))
        .collect();
    Ok(Json(json!({ "users": users })))
}
