//! Apps behind the gateway (docs/app-contract.md › Pairing, Manifest):
//!
//! - [`AppTable`]: every app by slug in memory, reloaded from the database
//!   after every change, so that the gateway (milestone 4.3) finds an app
//!   without a database round trip.
//! - The rules for what an administrator enters: [`check_slug`] (format and
//!   reserved names) and [`internal_origin`] (scheme, host and port).
//! - [`sanitize_icon`]: the manifest's SVG, checked and rewritten like an
//!   uploaded branding logo (`logo::upload`), with a square `viewBox`.
//! - Pairing codes and app secrets ([`new_pairing_code`], [`new_secret`],
//!   [`hash`]): random, only their SHA-256 is stored.
//! - [`check_health`]: `GET /<slug>/readyz` against the internal address,
//!   every 30 s by `jobs::health_poll`.

use std::{
    collections::BTreeMap,
    sync::{Arc, PoisonError, RwLock},
    time::Duration,
};

use roedy_hub_contract::{
    Manifest,
    pairing::{APP_SECRET_PREFIX, CROCKFORD, PAIRING_CODE_PREFIX, is_reserved_slug, is_valid_slug},
};
use rusqlite::Connection;
use sha2::{Digest as _, Sha256};
use url::Url;

use crate::{
    db::repo::apps::{self as repo, AccessDefault, Health, Status},
    logo::{Format, upload},
    random,
};

/// An app as the gateway needs it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct App {
    pub id: String,
    pub slug: String,
    /// Origin of the app, e.g. `http://roedy-mail:8080`.
    pub internal_url: Url,
    pub status: Status,
    /// `None` while pending.
    pub manifest: Option<Arc<Manifest>>,
    pub access_default: AccessDefault,
    pub health: Option<Health>,
    /// [`icon_version`] of the manifest's icon; `None` while pending.
    pub icon_version: Option<String>,
}

impl App {
    fn from_row(row: repo::App) -> Option<Self> {
        let internal_url = match Url::parse(&row.internal_url) {
            Ok(url) => url,
            Err(error) => {
                tracing::warn!(
                    slug = row.slug,
                    "internal address not readable, app skipped: {error}"
                );
                return None;
            }
        };
        Some(Self {
            id: row.id,
            slug: row.slug,
            internal_url,
            status: row.status,
            icon_version: row.manifest.as_ref().map(icon_version),
            manifest: row.manifest.map(Arc::new),
            access_default: row.access_default,
            health: row.health,
        })
    }

    /// `<internal address>/<slug><path>`; `path` starts with `/`.
    pub fn internal(&self, path: &str) -> String {
        format!(
            "{}/{}{path}",
            self.internal_url.as_str().trim_end_matches('/'),
            self.slug
        )
    }
}

/// All apps by slug; [`Self::reload`] after every change.
#[derive(Debug, Default)]
pub struct AppTable {
    apps: RwLock<Arc<BTreeMap<String, Arc<App>>>>,
}

impl AppTable {
    /// Reads all apps from the database and replaces the table. Run it in
    /// the same database job as the change, so that reloads happen in the
    /// order of the changes.
    pub fn reload(&self, conn: &Connection) -> anyhow::Result<()> {
        let apps: BTreeMap<String, Arc<App>> = repo::list(conn)?
            .into_iter()
            .filter_map(App::from_row)
            .map(|app| (app.slug.clone(), Arc::new(app)))
            .collect();
        *self.apps.write().unwrap_or_else(PoisonError::into_inner) = Arc::new(apps);
        Ok(())
    }

    fn snapshot(&self) -> Arc<BTreeMap<String, Arc<App>>> {
        self.apps
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// The app with this slug.
    pub fn get(&self, slug: &str) -> Option<Arc<App>> {
        self.snapshot().get(slug).cloned()
    }

    /// The paired apps, by slug.
    pub fn paired(&self) -> Vec<Arc<App>> {
        self.snapshot()
            .values()
            .filter(|app| app.status == Status::Paired)
            .cloned()
            .collect()
    }

    pub fn len(&self) -> usize {
        self.snapshot().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// The version of the manifest's (sanitized) icon for `?v=` in its URL: the
/// first 16 hex digits of its SHA-256, so that the URL changes with the
/// icon.
pub fn icon_version(manifest: &Manifest) -> String {
    use std::fmt::Write as _;

    Sha256::digest(manifest.icon.as_bytes())
        .iter()
        .take(8)
        .fold(String::with_capacity(16), |mut hex, byte| {
            let _ = write!(hex, "{byte:02x}");
            hex
        })
}

/// The browser's address of the app's icon (contract § 6), with the
/// version: `/api/v1/apps/<slug>/icon.svg?v=<version>`.
pub fn icon_url(slug: &str, manifest: &Manifest) -> String {
    format!("/api/v1/apps/{slug}/icon.svg?v={}", icon_version(manifest))
}

/// Why a slug is not accepted: `invalid` (format) or `reserved`.
pub fn check_slug(slug: &str) -> Result<(), &'static str> {
    if is_reserved_slug(slug) {
        Err("reserved")
    } else if !is_valid_slug(slug) {
        Err("invalid")
    } else {
        Ok(())
    }
}

/// The internal address as an origin (`http://roedy-mail:8080`): `http` or
/// `https` with a host, without credentials, path (other than `/`), query
/// or fragment.
pub fn internal_origin(input: &str) -> Option<String> {
    let url = Url::parse(input.trim()).ok()?;
    let fits = matches!(url.scheme(), "http" | "https")
        && url.host().is_some()
        && url.username().is_empty()
        && url.password().is_none()
        && url.path() == "/"
        && url.query().is_none()
        && url.fragment().is_none();
    fits.then(|| url.origin().ascii_serialization())
}

/// Why an icon is not accepted (`fields.icon` of `invalid_manifest`).
pub fn sanitize_icon(svg: &str) -> Result<String, &'static str> {
    // The upload check first: it rejects entities before anything else
    // reads the document, and rewrites it without scripts, event handlers,
    // `foreignObject` and external references.
    let image =
        upload::check(Format::Svg, svg.as_bytes().to_vec()).map_err(|rejected| match rejected {
            upload::Rejected::TooLarge => "too_large",
            upload::Rejected::SvgText => "svg_text",
            upload::Rejected::Dimensions => "dimensions",
            upload::Rejected::Unsupported | upload::Rejected::Unreadable => "unreadable",
        })?;
    // A raster image the check recognized by its content is no icon.
    if image.format != Format::Svg {
        return Err("unreadable");
    }
    if !has_square_view_box(svg) {
        return Err("not_square");
    }
    String::from_utf8(image.data.to_vec()).map_err(|_| "unreadable")
}

/// Whether the root element has a `viewBox` with equal, positive width and
/// height.
fn has_square_view_box(svg: &str) -> bool {
    let svg = svg.strip_prefix('\u{feff}').unwrap_or(svg);
    let Ok(doc) = roxmltree::Document::parse_with_options(
        svg,
        roxmltree::ParsingOptions {
            allow_dtd: true,
            ..Default::default()
        },
    ) else {
        return false;
    };
    let Some(view_box) = doc.root_element().attribute("viewBox") else {
        return false;
    };
    let numbers: Vec<f64> = view_box
        .split(|c: char| c.is_whitespace() || c == ',')
        .filter(|part| !part.is_empty())
        .map(str::parse)
        .collect::<Result<_, _>>()
        .unwrap_or_default();
    matches!(numbers.as_slice(), [_, _, w, h] if w.is_finite() && *w > 0.0 && (w - h).abs() < 1e-9)
}

/// Crockford Base32 of `bytes`, 5 bits per character.
fn crockford(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 8 / 5 + 1);
    let (mut buffer, mut bits) = (0u32, 0u32);
    for byte in bytes {
        buffer = (buffer << 8) | u32::from(*byte);
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            out.push(char::from(CROCKFORD[((buffer >> bits) & 31) as usize]));
        }
    }
    if bits > 0 {
        out.push(char::from(
            CROCKFORD[((buffer << (5 - bits)) & 31) as usize],
        ));
    }
    out
}

/// A new pairing code: `rhp_` and 32 characters (160 random bits), in the
/// canonical form `normalize_pairing_code` gives.
pub fn new_pairing_code() -> anyhow::Result<String> {
    Ok(format!(
        "{PAIRING_CODE_PREFIX}{}",
        crockford(&random::bytes::<20>()?)
    ))
}

/// A new app secret: `rhs_<app ID>_` and 256 random bits.
pub fn new_secret(app_id: &str) -> anyhow::Result<String> {
    Ok(format!(
        "{APP_SECRET_PREFIX}{app_id}_{}",
        crockford(&random::bytes::<32>()?)
    ))
}

/// SHA-256 of a pairing code or secret, as stored.
pub fn hash(secret: &str) -> [u8; 32] {
    Sha256::digest(secret.as_bytes()).into()
}

/// Until the app accepts the connection.
const PROBE_CONNECT: Duration = Duration::from_secs(5);
/// For the whole health check.
const PROBE_TIMEOUT: Duration = Duration::from_secs(10);

/// HTTP client for the health checks: no redirects, short timeouts.
pub fn probe_client() -> anyhow::Result<reqwest::Client> {
    client_with(PROBE_CONNECT, PROBE_TIMEOUT)
}

pub(crate) fn client_with(connect: Duration, total: Duration) -> anyhow::Result<reqwest::Client> {
    // As in `mailer::http_client`: ring for the whole process, if no
    // provider is set yet.
    let _ = rustls::crypto::ring::default_provider().install_default();
    reqwest::Client::builder()
        .user_agent(concat!("Roedy-Hub/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(connect)
        .timeout(total)
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|error| anyhow::anyhow!("HTTP client not created: {error}"))
}

/// `GET /<slug>/readyz` at the internal address: online on a 2xx answer,
/// offline on any other answer, without an answer and after the timeout.
pub async fn check_health(client: &reqwest::Client, app: &App) -> Health {
    match client.get(app.internal("/readyz")).send().await {
        Ok(response) if response.status().is_success() => Health::Online,
        Ok(response) => {
            tracing::debug!(slug = app.slug, status = %response.status(), "app not ready");
            Health::Offline
        }
        Err(error) => {
            tracing::debug!(slug = app.slug, "app not reachable: {error}");
            Health::Offline
        }
    }
}

#[cfg(test)]
mod tests {
    use roedy_hub_contract::pairing::normalize_pairing_code;

    use super::*;
    use crate::db::repo::{apps::tests::manifest, users::tests::conn};

    #[test]
    fn slugs_follow_the_contract() {
        assert_eq!(check_slug("mail"), Ok(()));
        assert_eq!(check_slug("mail-archive2"), Ok(()));
        for reserved in ["login", "admin", "api", "internal", "apps", "hub", "static"] {
            assert_eq!(check_slug(reserved), Err("reserved"), "{reserved}");
        }
        for reserved in ["_hub", ".well-known", "favicon.ico", "robots.txt"] {
            assert_eq!(check_slug(reserved), Err("reserved"), "{reserved}");
        }
        for invalid in [
            "",
            "m",
            "Mail",
            "mail-",
            "2mail",
            "ma/il",
            "mail ",
            &"a".repeat(33),
        ] {
            assert_eq!(check_slug(invalid), Err("invalid"), "{invalid}");
        }
    }

    #[test]
    fn internal_addresses_are_origins() {
        for (input, origin) in [
            ("http://roedy-mail:8080", "http://roedy-mail:8080"),
            ("http://roedy-mail:8080/", "http://roedy-mail:8080"),
            (" HTTP://Roedy-Mail ", "http://roedy-mail"),
            (
                "https://mail.internal.test:443",
                "https://mail.internal.test",
            ),
            ("http://10.0.0.5:8080", "http://10.0.0.5:8080"),
            ("http://[::1]:8080", "http://[::1]:8080"),
        ] {
            assert_eq!(internal_origin(input).as_deref(), Some(origin), "{input}");
        }
        for input in [
            "",
            "roedy-mail:8080",
            "ftp://roedy-mail",
            "http://roedy-mail:8080/mail",
            "http://roedy-mail:8080/?x=1",
            "http://roedy-mail:8080/#x",
            "http://user:secret@roedy-mail:8080",
            "file:///etc/passwd",
        ] {
            assert_eq!(internal_origin(input), None, "{input}");
        }
    }

    #[test]
    fn icons_are_sanitized_and_square() {
        let clean = sanitize_icon(
            r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" onload="alert(1)"><script>alert(1)</script><a href="javascript:alert(1)"><path d="M2 2h20v20H2z" onclick="x()"/></a><foreignObject><div xmlns="http://www.w3.org/1999/xhtml">x</div></foreignObject><image href="https://tracker.invalid/x.png" width="1" height="1"/></svg>"#,
        )
        .unwrap();
        for unsafe_part in [
            "script",
            "onload",
            "onclick",
            "javascript",
            "foreignObject",
            "tracker",
        ] {
            assert!(!clean.contains(unsafe_part), "{unsafe_part}: {clean}");
        }
        assert!(clean.contains("<path"), "{clean}");

        for (svg, code) in [
            (
                r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 12"><path d="M0 0h1v1z"/></svg>"#,
                "not_square",
            ),
            (
                r#"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24"><path d="M0 0h1v1z"/></svg>"#,
                "not_square",
            ),
            (
                r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 0 0"><path d="M0 0h1v1z"/></svg>"#,
                "not_square",
            ),
            (
                r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><text>A</text></svg>"#,
                "svg_text",
            ),
            ("<svg", "unreadable"),
            ("not an svg", "unreadable"),
            (
                r#"<?xml version="1.0"?><!DOCTYPE svg [<!ENTITY a "aaaaaaaa"><!ENTITY b "&a;&a;&a;&a;">]><svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path d="M0 0h1v1z"/><desc>&b;</desc></svg>"#,
                "unreadable",
            ),
            (
                r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><image href="https://tracker.invalid/x.png" width="24" height="24"/></svg>"#,
                "unreadable",
            ),
        ] {
            assert_eq!(sanitize_icon(svg), Err(code), "{svg}");
        }
        // WebP is plain ASCII at the start; the content decides, not the
        // declaration, so it must not pass as an SVG.
        let webp = "RIFF\u{24}\0\0\0WEBPVP8 \u{18}\0\0\0\u{30}\u{1}\0\u{9d}\u{1}*\u{1}\0\u{1}\0";
        assert!(sanitize_icon(webp).is_err());
        assert!(has_square_view_box(r#"<svg viewBox="0,0,24,24"/>"#));
        assert!(has_square_view_box(r#"<svg viewBox=" -12 -12  24 24 "/>"#));
        assert!(!has_square_view_box(r#"<svg viewBox="0 0 24"/>"#));
        assert!(!has_square_view_box(r#"<svg viewBox="0 0 NaN NaN"/>"#));
    }

    #[test]
    fn codes_and_secrets() {
        let code = new_pairing_code().unwrap();
        assert_eq!(code.len(), 4 + 32);
        assert_eq!(
            normalize_pairing_code(&code).as_deref(),
            Some(code.as_str())
        );
        assert_ne!(code, new_pairing_code().unwrap());
        let secret = new_secret("01JDX4AAAAAAAAAAAAAAAAAAAA").unwrap();
        assert!(secret.starts_with("rhs_01JDX4AAAAAAAAAAAAAAAAAAAA_"));
        assert_eq!(secret.len(), 4 + 26 + 1 + 52);
        assert_eq!(hash(&secret), hash(&secret.clone()));
        assert_ne!(hash(&secret), hash(&code));
        assert_eq!(crockford(&[0xff; 5]), "ZZZZZZZZ");
        assert_eq!(crockford(&[0; 5]), "00000000");
        assert_eq!(crockford(&[0x80]), "G0");
    }

    #[test]
    fn the_table_follows_the_database() {
        let conn = conn();
        let table = AppTable::default();
        table.reload(&conn).unwrap();
        assert!(table.is_empty());
        let mail = repo::insert(&conn, "mail", "http://mail.test:8080").unwrap();
        repo::insert(&conn, "notes", "http://notes.test").unwrap();
        table.reload(&conn).unwrap();
        assert_eq!(table.len(), 2);
        let app = table.get("mail").unwrap();
        assert_eq!(app.status, Status::Pending);
        assert_eq!(app.internal("/readyz"), "http://mail.test:8080/mail/readyz");
        assert!(table.paired().is_empty());
        repo::pair(&conn, &mail, &manifest(), &[1; 32]).unwrap();
        table.reload(&conn).unwrap();
        let paired = table.paired();
        assert_eq!(paired.len(), 1);
        assert_eq!(paired[0].manifest.as_deref(), Some(&manifest()));
        repo::delete(&conn, &mail).unwrap();
        table.reload(&conn).unwrap();
        assert!(table.get("mail").is_none());
        assert!(table.get("notes").is_some());
    }
}
