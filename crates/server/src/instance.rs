//! Instance-wide details for everyone: organization and branding
//! (Administration › Organization and Branding).
//!
//! They live in `app_settings` (keys [`ORG`] and [`BRANDING`], the logo in
//! `assets`) and in memory ([`Instance`], loaded at startup with the logos
//! drawn, set after every change while still on the database thread). `GET session`, `GET public/config`, the issuer of
//! authenticator apps and `AppState::brand` (mails) read from there – one
//! source without a database query per request.
//! Changed by: `api::organization` and `api::branding`.
//!
//! Taken over from Roedy Mail (`crates/server/src/instance.rs`) without the
//! banner, the update hint, the browser cache version, the app name and the
//! sign-in texts of the branding: the hub's name is the organization's name,
//! which also gives the logo's initial.

use std::sync::{Arc, PoisonError, RwLock};

use rusqlite::Connection;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::Value;

use crate::{
    db::repo::{app_settings, assets},
    logo::{Image, Logos},
    mailer::templates::APP_NAME,
};

/// Key of the organization in `app_settings`.
pub const ORG: &str = "org";
/// Key of the branding in `app_settings`.
pub const BRANDING: &str = "branding";

/// Logo kinds; the first is the default.
pub const KINDS: [&str; 4] = ["standard", "initial", "icon", "image"];
/// How an image is fitted; the first is the default.
pub const FITS: [&str; 2] = ["padded", "fill"];
/// Plates behind the logo; the first is the default.
pub const PLATES: [&str; 3] = ["accent", "light", "dark"];

/// Organization details; `None` means "not specified" (an empty field on the
/// page).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Org {
    /// Also the hub's name (`GET session` › `hub.name`, mails, authenticator
    /// apps); without it "Roedy Hub".
    pub name: Option<String>,
    pub website: Option<String>,
    pub help: Option<String>,
    /// Support email.
    pub email: Option<String>,
    pub imprint: Option<String>,
    pub privacy: Option<String>,
    pub terms: Option<String>,
    /// Repository address (source code, AGPL).
    pub source: Option<String>,
}

impl Org {
    /// The fields in the order of the page, for validation and the audit log.
    pub fn fields_mut(&mut self) -> [(&'static str, &mut Option<String>); 8] {
        [
            ("name", &mut self.name),
            ("website", &mut self.website),
            ("help", &mut self.help),
            ("email", &mut self.email),
            ("imprint", &mut self.imprint),
            ("privacy", &mut self.privacy),
            ("terms", &mut self.terms),
            ("source", &mut self.source),
        ]
    }

    /// The hub's name: the organization's or "Roedy Hub".
    pub fn hub_name(&self) -> &str {
        self.name.as_deref().unwrap_or(APP_NAME)
    }
}

/// Branding: the logo (`kind`, `letter`, `icon`, `image`, `fit`, `plate`,
/// `tilt`), the accent colour (one of `mailer::templates::ACCENTS`) and
/// whether users may choose their own (`userAccent`). `image` is the ID of
/// the logo in `assets` (its address comes from [`Image::path`]),
/// `imageName` and `imageMeta` describe it on the page (file name and size).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Branding {
    pub kind: String,
    pub letter: String,
    pub icon: String,
    pub image: Option<String>,
    pub image_name: String,
    pub image_meta: String,
    pub fit: String,
    pub plate: String,
    pub tilt: bool,
    pub color: String,
    pub user_accent: bool,
}

impl Default for Branding {
    /// The standard logo in apricot; users may choose their own accent.
    fn default() -> Self {
        Self {
            kind: KINDS[0].into(),
            letter: String::new(),
            icon: "ti-mountain".into(),
            image: None,
            image_name: String::new(),
            image_meta: String::new(),
            fit: FITS[0].into(),
            plate: PLATES[0].into(),
            tilt: true,
            color: "apricot".into(),
            user_accent: true,
        }
    }
}

/// The branding with its logo and the PNGs drawn from it.
#[derive(Debug, Clone)]
pub struct BrandingState {
    pub branding: Branding,
    pub image: Option<Image>,
    /// `None` if drawing failed (then only the plate).
    pub logos: Option<Arc<Logos>>,
}

impl Default for BrandingState {
    fn default() -> Self {
        Self {
            branding: Branding::default(),
            image: None,
            logos: Logos::standard(),
        }
    }
}

impl BrandingState {
    /// Whether the drawn logo shows the hub's name: the initial without a
    /// letter of its own (also the kind "image" without an image, and an
    /// unknown kind, which draw the initial as in the web app).
    pub fn shows_name(&self) -> bool {
        let kind = self.branding.kind.as_str();
        let draws_initial = match kind {
            "standard" | "icon" => false,
            "image" => self.image.is_none(),
            _ => true,
        };
        draws_initial && self.branding.letter.trim().is_empty()
    }

    /// Draws the logos for branding and image; `name` is the hub's name
    /// for the initial.
    pub fn new(branding: Branding, image: Option<Image>, name: &str) -> Self {
        let logos = Logos::render_or_warn(&branding, image.as_ref(), name);
        Self {
            branding,
            image,
            logos,
        }
    }

    /// Address of the logo, if there is one.
    pub fn image_path(&self) -> Option<String> {
        self.image.as_ref().map(Image::path)
    }
}

/// Reads `key`; `T` itself tolerates missing and unknown fields. If a value
/// does not fit (say a text instead of a number), `fallback` applies with a
/// warning – otherwise the server would not start, and `PUT` could never
/// replace the entry.
fn load<T: DeserializeOwned>(
    conn: &Connection,
    key: &str,
    fallback: impl FnOnce(&Value) -> T,
) -> anyhow::Result<Option<T>> {
    let Some(value) = app_settings::get::<Value>(conn, key)? else {
        return Ok(None);
    };
    Ok(Some(serde_json::from_value(value.clone()).unwrap_or_else(
        |error| {
            tracing::warn!("app_settings.{key} unreadable, using default: {error}");
            fallback(&value)
        },
    )))
}

/// Organization from the database; without a (readable) entry nothing is
/// specified.
pub fn load_org(conn: &Connection) -> anyhow::Result<Org> {
    Ok(load(conn, ORG, |_| Org::default())?.unwrap_or_default())
}

/// Branding from the database, without a (readable) entry the default, with
/// the logo from `assets`. If it is missing there, the branding applies
/// without an image (with a warning) – the kind "image" then draws the
/// initial.
pub fn load_branding(conn: &Connection) -> anyhow::Result<(Branding, Option<Image>)> {
    let mut branding: Branding = load(conn, BRANDING, |_| Branding::default())?.unwrap_or_default();
    let image = match &branding.image {
        Some(id) => {
            let image = assets::get(conn, id)?;
            if image.is_none() {
                tracing::warn!("branding logo {id} missing from assets");
                branding.image = None;
            }
            image
        }
        None => None,
    };
    Ok((branding, image))
}

/// The in-memory state.
#[derive(Default)]
pub struct Instance {
    org: RwLock<Org>,
    branding: RwLock<Arc<BrandingState>>,
}

impl Instance {
    /// Reads everything from the database and draws the logo.
    pub fn load(&self, conn: &Connection) -> anyhow::Result<()> {
        let org = load_org(conn)?;
        let (branding, image) = load_branding(conn)?;
        self.set_branding(BrandingState::new(branding, image, org.hub_name()));
        self.set_org(org);
        Ok(())
    }

    pub fn org(&self) -> Org {
        self.org
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub fn set_org(&self, org: Org) {
        *self.org.write().unwrap_or_else(PoisonError::into_inner) = org;
    }

    pub fn branding(&self) -> Arc<BrandingState> {
        self.branding
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub fn set_branding(&self, branding: BrandingState) {
        *self
            .branding
            .write()
            .unwrap_or_else(PoisonError::into_inner) = Arc::new(branding);
    }

    /// The hub's name ([`Org::hub_name`]).
    pub fn name(&self) -> String {
        self.org
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .hub_name()
            .to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::migrations;

    fn conn() -> Connection {
        let mut conn = Connection::open_in_memory().unwrap();
        migrations::run(&mut conn).unwrap();
        conn
    }

    #[test]
    fn defaults_without_entries() {
        let conn = conn();
        assert_eq!(load_org(&conn).unwrap(), Org::default());
        let (branding, image) = load_branding(&conn).unwrap();
        assert_eq!(image, None);
        assert_eq!(
            serde_json::to_value(branding).unwrap(),
            serde_json::json!({
                "kind": "standard", "letter": "", "icon": "ti-mountain", "image": null,
                "imageName": "", "imageMeta": "", "fit": "padded", "plate": "accent",
                "tilt": true, "color": "apricot", "userAccent": true,
            })
        );
        let instance = Instance::default();
        instance.load(&conn).unwrap();
        assert_eq!(instance.name(), "Roedy Hub");
        assert_eq!(instance.branding().logos, Logos::standard());
        assert!(!instance.branding().shows_name());
    }

    #[test]
    fn reads_tolerantly() {
        let conn = conn();
        // Missing and unknown fields from another program version.
        conn.execute(
            "INSERT INTO app_settings (key, json) VALUES \
             ('org', '{\"name\":\"Example Inc.\",\"fax\":\"0\"}'), \
             ('branding', '{\"color\":\"sage\",\"kind\":\"image\",\"image\":\"missing\",\"later\":1}')",
            [],
        )
        .unwrap();
        let instance = Instance::default();
        instance.load(&conn).unwrap();
        assert_eq!(instance.name(), "Example Inc.");
        assert_eq!(instance.org().email, None);
        let state = instance.branding();
        assert_eq!(state.branding.color, "sage");
        assert_eq!(state.branding.kind, "image");
        // An image missing from `assets`: the kind "image" draws the initial.
        assert_eq!(
            (state.branding.image.as_deref(), state.image_path()),
            (None, None)
        );
        assert!(state.shows_name());
        assert!(state.logos.is_some());
        assert_ne!(state.logos, Logos::standard());
    }

    #[test]
    fn unreadable_entries_fall_back_instead_of_failing() {
        let conn = conn();
        conn.execute(
            "INSERT INTO app_settings (key, json) VALUES \
             ('org', '{\"name\":7}'), ('branding', '{\"userAccent\":\"yes\"}')",
            [],
        )
        .unwrap();
        assert_eq!(load_org(&conn).unwrap(), Org::default());
        assert_eq!(load_branding(&conn).unwrap().0, Branding::default());
    }

    #[test]
    fn branding_with_image_from_assets() {
        let conn = conn();
        let image = crate::logo::upload::check(
            crate::logo::Format::Svg,
            br#"<svg xmlns="http://www.w3.org/2000/svg" width="4" height="4"><rect width="4" height="4"/></svg>"#.to_vec(),
        )
        .unwrap();
        assets::insert(&conn, &image).unwrap();
        let branding = Branding {
            kind: "image".into(),
            image: Some(image.id.clone()),
            ..Branding::default()
        };
        app_settings::set(&conn, BRANDING, &branding).unwrap();
        let instance = Instance::default();
        instance.load(&conn).unwrap();
        let state = instance.branding();
        assert_eq!(state.branding, branding);
        assert_eq!(state.image.as_ref(), Some(&image));
        assert_eq!(
            state.image_path().unwrap(),
            format!("/branding/image/{}.svg", image.id)
        );
    }
}
