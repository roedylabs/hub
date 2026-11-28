//! `GET apps`: the signed-in user's apps for the home page and the apps'
//! switchers (docs/app-contract.md › Browser API of the hub). Part of the
//! contract: fields are only added within version 1.
//!
//! Listed are the paired apps the user has access to (`app_access`), in the
//! administrator's order, with name and description in the user's language
//! (`en` or `de`, for `system` the one of the last sign-in). Hub
//! administrators see the same: their role gives them no app they have no
//! access to. Pending apps are never listed – they have no manifest, so no
//! name or icon, and the gateway lets only hub administrators through to
//! finish their setup; the administration lists them (`admin/apps`). Apps
//! an administrator switched off are left out as well, for everyone.
//!
//! The icon (`GET apps/<slug>/icon.svg`) is served outside the API's
//! middleware by `web::app_icons`: it is public and cacheable, and the
//! browser loads it as an image.

use axum::{Json, Router, extract::State, routing::get};
use serde_json::{Value, json};

use super::ApiError;
use crate::{
    app::AppState,
    apps,
    auth::CurrentUser,
    db::{
        self,
        repo::{
            app_access,
            apps::{self as repo, Health, Status},
        },
    },
    settings,
};

pub fn routes() -> Router<AppState> {
    Router::new().route("/apps", get(list))
}

/// `200 {"apps": [...]}` while signed in, otherwise `401 unauthorized`.
/// `url` is `/<slug><home>`, `icon` carries the icon's version, `status` is
/// `online` after a passed health check, otherwise `offline`.
async fn list(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
) -> Result<Json<Value>, ApiError> {
    let (apps, roles, lang) = db::call(&state.db, move |conn| {
        let apps = repo::list(conn)?;
        let roles = app_access::of_user(conn, &user.id)?;
        let lang = settings::lang_of(conn, &user.id)?;
        Ok((apps, roles, lang))
    })
    .await?;
    let lang = lang.code();
    let rows: Vec<Value> = apps
        .iter()
        .filter(|app| {
            app.status == Status::Paired && app.enabled() && roles.contains_key(&app.slug)
        })
        .filter_map(|app| {
            let manifest = app.manifest.as_ref()?;
            Some(json!({
                "slug": app.slug,
                "name": manifest.name_in(lang),
                "description": manifest.description_in(lang),
                "url": format!("/{}{}", app.slug, manifest.home),
                "icon": apps::icon_url(&app.slug, manifest),
                "status": if app.health == Some(Health::Online) { "online" } else { "offline" },
            }))
        })
        .collect();
    Ok(Json(json!({ "apps": rows })))
}
