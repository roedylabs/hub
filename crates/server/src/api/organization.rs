//! Administration › Organization and Branding.
//!
//! - `GET admin/org`, `PUT admin/org`: the fields of [`Org`]; `PUT`
//!   changes the given ones, `null` or an empty text deletes one. Links only
//!   as `http(s)://…`, the support address only in address form – checked on
//!   save, and the mail templates check again to be safe. The name is the
//!   hub's name (`GET session` › `hub.name`, mails, authenticator apps).
//! - `GET admin/branding`, `PUT admin/branding`: the fields of [`Branding`]
//!   (`color`, `userAccent`); `PUT` changes the given ones.
//!
//! Both live in `app_settings` and in the in-memory store
//! (`crate::instance`), which `GET session`, `public/config` and the mails
//! read; changes are recorded in the audit log (category settings) with the
//! names of the changed fields.
//!
//! Taken over from Roedy Mail (`crates/server/src/api/instance.rs` and
//! `api/branding.rs`) without the banner and the logo.

use std::collections::BTreeMap;

use axum::{Json, Router, extract::State, routing::get};
use serde::Deserialize;
use serde_json::json;

use super::{ApiError, FieldErrors, JsonBody};
use crate::{
    app::AppState,
    auth::{AdminUser, AuthUser},
    client_ip::ClientIp,
    db::{
        self,
        repo::{
            app_settings,
            audit_log::{self, Category, Entry},
        },
    },
    instance::{self, BRANDING, Branding, ORG, Org},
    mailer::templates::{ACCENTS, is_address, is_link},
};

/// Maximum lengths in characters: organization name, links, support address.
const MAX_NAME: usize = 120;
const MAX_LINK: usize = 2048;
const MAX_EMAIL: usize = 254;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/admin/org", get(get_org).put(put_org))
        .route("/admin/branding", get(get_branding).put(put_branding))
}

/// A single-line text without control characters with at most `max`
/// characters.
fn text_error(value: &str, max: usize) -> Option<&'static str> {
    if value.chars().count() > max {
        Some("too_long")
    } else if value.chars().any(char::is_control) {
        Some("invalid")
    } else {
        None
    }
}

fn link_error(value: &str) -> Option<&'static str> {
    if value.chars().count() > MAX_LINK {
        Some("too_long")
    } else if !is_link(value) {
        Some("invalid")
    } else {
        None
    }
}

/// Error of an organization field (value already trimmed, not empty).
fn org_error(field: &str, value: &str) -> Option<&'static str> {
    match field {
        "name" => text_error(value, MAX_NAME),
        "email" if value.chars().count() > MAX_EMAIL => Some("too_long"),
        "email" => (!is_address(value)).then_some("invalid"),
        _ => link_error(value),
    }
}

fn entry(action: &'static str, user: &AuthUser, ip: ClientIp) -> Entry {
    Entry::new(Category::Settings, action).actor(user).ip(ip.0)
}

async fn get_org(State(state): State<AppState>, _: AdminUser) -> Json<Org> {
    Json(state.instance.org())
}

/// The given fields of a change, trimmed; `None` for empty or `null`.
/// Unknown fields are 400 like elsewhere with `deny_unknown_fields`.
fn org_changes(
    body: BTreeMap<String, Option<String>>,
) -> Result<Vec<(&'static str, Option<String>)>, ApiError> {
    let fields = Org::default().fields_mut().map(|(field, _)| field);
    body.into_iter()
        .map(|(key, value)| {
            let field = fields
                .iter()
                .find(|field| **field == key)
                .ok_or_else(|| ApiError::BadRequest(format!("unknown field `{key}`")))?;
            let value = value
                .map(|value| value.trim().to_owned())
                .filter(|value| !value.is_empty());
            Ok((*field, value))
        })
        .collect()
}

/// "Save" on the Organization page. The audit log records the names of the
/// changed fields (`org.update`); without a change everything stays as it
/// is.
async fn put_org(
    State(state): State<AppState>,
    AdminUser(user): AdminUser,
    ip: ClientIp,
    JsonBody(body): JsonBody<BTreeMap<String, Option<String>>>,
) -> Result<Json<Org>, ApiError> {
    let changes = org_changes(body)?;
    let mut errors = FieldErrors::default();
    for (field, value) in &changes {
        if let Some(code) = value.as_deref().and_then(|value| org_error(field, value)) {
            errors.code(field, code);
        }
    }
    if !errors.is_empty() {
        return Err(errors.into_error());
    }
    let instance = state.instance.clone();
    let org = db::call(&state.db, move |conn| {
        let tx = conn.transaction()?;
        let mut org = instance::load_org(&tx)?;
        let mut changed = Vec::new();
        for (field, slot) in org.fields_mut() {
            if let Some((_, value)) = changes.iter().find(|(name, _)| *name == field)
                && slot != value
            {
                slot.clone_from(value);
                changed.push(field);
            }
        }
        if !changed.is_empty() {
            app_settings::set(&tx, ORG, &org)?;
            audit_log::insert(
                &tx,
                &entry("org.update", &user, ip).details(json!({ "fields": changed })),
            )?;
            tx.commit()?;
        }
        // Still on the connection: a later change never sets the store
        // before this one.
        instance.set_org(org.clone());
        Ok(org)
    })
    .await?;
    Ok(Json(org))
}

async fn get_branding(State(state): State<AppState>, _: AdminUser) -> Json<Branding> {
    Json(state.instance.branding())
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct BrandingBody {
    color: Option<String>,
    user_accent: Option<bool>,
}

impl BrandingBody {
    fn check(&self) -> FieldErrors {
        let mut errors = FieldErrors::default();
        if self
            .color
            .as_deref()
            .is_some_and(|color| !ACCENTS.iter().any(|(name, _)| *name == color))
        {
            errors.code("color", "invalid");
        }
        errors
    }

    /// The branding after the change and the names of the changed fields.
    fn apply(&self, current: &Branding) -> (Branding, Vec<&'static str>) {
        let mut next = current.clone();
        let mut changed = Vec::new();
        if let Some(color) = &self.color
            && *color != next.color
        {
            next.color.clone_from(color);
            changed.push("color");
        }
        if let Some(user_accent) = self.user_accent
            && user_accent != next.user_accent
        {
            next.user_accent = user_accent;
            changed.push("userAccent");
        }
        (next, changed)
    }
}

/// "Save" on the Branding page (also "Reset" with the default values). The
/// audit log records the names of the changed fields (`branding.update`).
async fn put_branding(
    State(state): State<AppState>,
    AdminUser(user): AdminUser,
    ip: ClientIp,
    JsonBody(body): JsonBody<BrandingBody>,
) -> Result<Json<Branding>, ApiError> {
    let errors = body.check();
    if !errors.is_empty() {
        return Err(errors.into_error());
    }
    let instance = state.instance.clone();
    let branding = db::call(&state.db, move |conn| {
        let tx = conn.transaction()?;
        let (next, changed) = body.apply(&instance::load_branding(&tx)?);
        if !changed.is_empty() {
            app_settings::set(&tx, BRANDING, &next)?;
            audit_log::insert(
                &tx,
                &entry("branding.update", &user, ip).details(json!({ "fields": changed })),
            )?;
            tx.commit()?;
        }
        instance.set_branding(next.clone());
        Ok(next)
    })
    .await?;
    Ok(Json(branding))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn org_fields_by_kind() {
        assert_eq!(org_error("name", "Example Ltd"), None);
        assert_eq!(org_error("name", &"x".repeat(121)), Some("too_long"));
        assert_eq!(org_error("name", "a\u{7}b"), Some("invalid"));
        assert_eq!(org_error("email", "support@example.com"), None);
        assert_eq!(
            org_error("email", "support@example.com?cc=x"),
            Some("invalid")
        );
        assert_eq!(org_error("email", "no name@example.com"), Some("invalid"));
        assert_eq!(
            org_error("email", &format!("{}@example.com", "a".repeat(250))),
            Some("too_long")
        );
        assert_eq!(org_error("source", "https://example.com/a/b"), None);
        assert_eq!(org_error("terms", "javascript:x"), Some("invalid"));
        assert_eq!(
            org_error("help", &format!("https://example.com/{}", "a".repeat(2048))),
            Some("too_long")
        );
    }

    #[test]
    fn org_changes_trim_and_reject_unknown_fields() {
        let body = |pairs: &[(&str, Option<&str>)]| -> BTreeMap<String, Option<String>> {
            pairs
                .iter()
                .map(|(k, v)| ((*k).to_owned(), v.map(str::to_owned)))
                .collect()
        };
        assert_eq!(
            org_changes(body(&[
                ("name", Some(" Example ")),
                ("help", Some("  ")),
                ("terms", None)
            ]))
            .unwrap(),
            [
                ("help", None),
                ("name", Some("Example".to_owned())),
                ("terms", None)
            ]
        );
        assert!(org_changes(body(&[("fax", Some("0"))])).is_err());
    }

    #[test]
    fn branding_checks_and_names_changes() {
        let body =
            |json: serde_json::Value| -> BrandingBody { serde_json::from_value(json).unwrap() };
        assert!(body(json!({ "color": "rose" })).check().is_empty());
        assert!(!body(json!({ "color": "Green" })).check().is_empty());
        let (next, changed) =
            body(json!({ "color": "sage", "userAccent": true })).apply(&Branding::default());
        assert_eq!(changed, ["color"]);
        assert_eq!(next.color, "sage");
        let (_, changed) = body(json!({ "userAccent": false })).apply(&next);
        assert_eq!(changed, ["userAccent"]);
        assert!(serde_json::from_value::<BrandingBody>(json!({ "kind": "image" })).is_err());
    }
}
