//! `GET public/config`: what the web app needs before any sign-in – sign-in,
//! setup, invitation and error pages. Readable without a session; so nothing
//! here concerns administrators only.
//!
//! - `name`: the hub's name (the organization's, otherwise "Roedy Hub").
//! - `org`: details of the organization like `instance::Org` (name, website,
//!   help, support, legal notice, privacy, terms of use, source code),
//!   `null` = not given.
//! - `branding`: `color` and `userAccent`.
//! - `setup_required`: setup is pending (no administrator yet).
//!
//! Organization and branding come from `AppState::instance`, from which
//! `AppState::brand` also takes the details for mails – one source for mails
//! and web app.
//!
//! Taken over from Roedy Mail (`crates/server/src/api/public.rs`) without
//! banner, logo, maintenance mode, version and update hint.

use axum::{Json, Router, extract::State, routing::get};
use serde_json::{Value, json};

use super::ApiError;
use crate::{app::AppState, db, db::repo::users};

pub fn routes() -> Router<AppState> {
    Router::new().route("/public/config", get(config))
}

async fn config(State(state): State<AppState>) -> Result<Json<Value>, ApiError> {
    let setup_required = !db::call(&state.db, |conn| Ok(users::admin_exists(conn)?)).await?;
    let org = state.instance.org();
    Ok(Json(json!({
        "name": org.hub_name(),
        "org": org,
        "branding": state.instance.branding(),
        "setup_required": setup_required,
    })))
}
