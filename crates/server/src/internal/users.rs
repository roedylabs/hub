//! `GET /internal/v1/users` and `GET /internal/v1/users/<sub>`
//! (docs/app-contract.md › Internal API): the users with access to the
//! calling app, and the authoritative answer for one user – with access,
//! without access (`404 no_access`, also for an unknown `sub`) or deleted
//! (`410 user_deleted` from the tombstones in `deleted_users`).

use axum::{
    Json,
    extract::{Path, Query, State, rejection::QueryRejection},
    http::StatusCode,
};
use roedy_hub_contract::pairing::{User, Users};
use rusqlite::Connection;
use serde::Deserialize;

use super::PairedApp;
use crate::{
    api::ApiError,
    app::AppState,
    db::{
        self,
        repo::{
            app_access::{self, Member},
            deleted_users,
        },
    },
    settings,
};

/// Users per page without `limit`.
pub const DEFAULT_LIMIT: usize = 200;
/// Most users per page.
pub const MAX_LIMIT: usize = 500;
/// Longest cursor (a user ID).
const MAX_CURSOR_LEN: usize = 64;

#[derive(Deserialize)]
pub struct Page {
    cursor: Option<String>,
    limit: Option<usize>,
}

/// A member as the contract lists it, in their language, with their
/// time zone and formats.
fn user(conn: &Connection, member: Member) -> anyhow::Result<User> {
    let stored = settings::load(conn, &member.profile.id)?;
    let p = member.profile;
    Ok(User {
        sub: p.id,
        username: p.username,
        email: p.email,
        name: p.display_name,
        role: p.role.as_str().to_owned(),
        app_role: member.app_role,
        locale: stored.lang.code().to_owned(),
        status: if member.active { "active" } else { "locked" }.to_owned(),
        prefs: stored.app_prefs(),
    })
}

pub async fn list(
    State(state): State<AppState>,
    PairedApp(app, _): PairedApp,
    page: Result<Query<Page>, QueryRejection>,
) -> Result<Json<Users>, ApiError> {
    let Query(page) = page.map_err(|rejection| ApiError::BadRequest(rejection.body_text()))?;
    let limit = page.limit.unwrap_or(DEFAULT_LIMIT);
    if !(1..=MAX_LIMIT).contains(&limit) {
        return Err(ApiError::BadRequest(format!(
            "limit must be 1 to {MAX_LIMIT}"
        )));
    }
    let cursor = page.cursor.filter(|cursor| !cursor.is_empty());
    if cursor.as_ref().is_some_and(|c| c.len() > MAX_CURSOR_LEN) {
        return Err(ApiError::BadRequest("unknown cursor".into()));
    }
    let users = db::call(&state.db, move |conn| {
        let mut members = app_access::members(conn, &app.id, cursor.as_deref(), limit + 1)?;
        let more = members.len() > limit;
        members.truncate(limit);
        let next_cursor = more
            .then(|| members.last().map(|m| m.profile.id.clone()))
            .flatten();
        let users = members
            .into_iter()
            .map(|member| user(conn, member))
            .collect::<anyhow::Result<_>>()?;
        Ok(Users { users, next_cursor })
    })
    .await?;
    Ok(Json(users))
}

pub async fn show(
    State(state): State<AppState>,
    PairedApp(app, _): PairedApp,
    Path(sub): Path<String>,
) -> Result<Json<User>, ApiError> {
    let found = db::call(&state.db, move |conn| {
        if let Some(member) = app_access::member_of(conn, &app.id, &sub)? {
            return Ok(Ok(user(conn, member)?));
        }
        Ok(Err(match deleted_users::deleted_at(conn, &sub)? {
            Some(at) => ApiError::rejected(StatusCode::GONE, "user_deleted").with("deleted_at", at),
            None => ApiError::rejected(StatusCode::NOT_FOUND, "no_access"),
        }))
    })
    .await??;
    Ok(Json(found))
}
