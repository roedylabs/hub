//! Per-user settings: language and appearance (`crate::settings`).
//!
//! - `GET settings`: `{version, settings}` with all fields; without stored
//!   settings the defaults with version 0.
//! - `PUT settings` (`{version, settings}`): takes over the given fields, the
//!   others stay. `version` is the version the change is based on; if another
//!   one applies by now (second tab, other device), 409 `version_conflict`
//!   with the current state (`version`, `settings`), so the client can apply
//!   its change to it again. Unknown fields and values the hub does not offer
//!   are 422 `invalid` with `fields` (`settings.<field>`: `unknown` or
//!   `invalid`), and nothing changes. If nothing changes, the version stays
//!   too – unless the stored document contains something this version
//!   ignores; then it is rewritten.
//!
//! Taken over from Roedy Mail (`crates/server/src/api/settings.rs`) without
//! the default mail account and "clear local data".

use axum::{Json, Router, extract::State, http::StatusCode, routing::get};
use serde::Deserialize;
use serde_json::{Map, Value, json};

use super::{ApiError, FieldErrors, JsonBody};
use crate::{
    app::AppState,
    auth::CurrentUser,
    db,
    settings::{self, Rejected, Stored},
};

pub fn routes() -> Router<AppState> {
    Router::new().route("/settings", get(get_settings).put(put_settings))
}

fn stored_json(stored: &Stored) -> anyhow::Result<Value> {
    Ok(json!({
        "version": stored.version,
        "settings": serde_json::to_value(&stored.settings)?,
    }))
}

async fn get_settings(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
) -> Result<Json<Value>, ApiError> {
    let stored = db::call(&state.db, move |conn| settings::load(conn, &user.id)).await?;
    Ok(Json(stored_json(&stored)?))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PutBody {
    version: i64,
    settings: Map<String, Value>,
}

enum Outcome {
    Saved(Stored),
    Conflict(Stored),
    Invalid(FieldErrors),
}

async fn put_settings(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    JsonBody(body): JsonBody<PutBody>,
) -> Result<Json<Value>, ApiError> {
    let outcome = db::call(&state.db, move |conn| {
        let tx = conn.transaction()?;
        let current = settings::load(&tx, &user.id)?;
        let next = match current.settings.apply(&body.settings) {
            Ok(next) => next,
            Err(Rejected(fields)) => {
                let mut inner = FieldErrors::default();
                for (field, code) in fields {
                    inner.code(&field, code);
                }
                let mut errors = FieldErrors::default();
                errors.nest("settings", inner);
                return Ok(Outcome::Invalid(errors));
            }
        };
        if body.version != current.version {
            return Ok(Outcome::Conflict(current));
        }
        if next == current.settings && !current.rewrite {
            return Ok(Outcome::Saved(current));
        }
        // Read in the same transaction, so the version still applies.
        settings::save(&tx, &user.id, current.version, &next)?
            .ok_or_else(|| anyhow::anyhow!("settings version has changed"))?;
        let saved = settings::load(&tx, &user.id)?;
        tx.commit()?;
        Ok(Outcome::Saved(saved))
    })
    .await?;
    match outcome {
        Outcome::Saved(stored) => Ok(Json(stored_json(&stored)?)),
        Outcome::Conflict(current) => {
            let settings = serde_json::to_value(&current.settings).map_err(anyhow::Error::from)?;
            Err(ApiError::rejected(StatusCode::CONFLICT, "version_conflict")
                .with("version", current.version)
                .with("settings", settings))
        }
        Outcome::Invalid(errors) => Err(errors.into_error()),
    }
}
