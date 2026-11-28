//! Slugs (contract § 1, § 3.1), pairing codes and app secrets (§ 9), and
//! the bodies of the internal API (§ 7, § 9) as shared types.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::Jwks;

/// First path segments that belong to the hub and can never be a slug
/// (contract § 3.1 › Reserved paths). Some can never match the slug format
/// anyway (`.well-known`, `_hub`, `favicon.ico`, `robots.txt`); they are
/// listed so that the list reads like the contract.
pub const RESERVED_SLUGS: [&str; 23] = [
    "login",
    "account",
    "admin",
    "setup",
    "invite",
    "api",
    "internal",
    "healthz",
    "readyz",
    ".well-known",
    "_hub",
    "favicon.ico",
    "robots.txt",
    // Kept free for later.
    "apps",
    "auth",
    "branding",
    "error",
    "help",
    "hub",
    "logout",
    "oauth",
    "settings",
    "static",
];

/// The slug format `^[a-z][a-z0-9-]{0,30}[a-z0-9]$` (2–32 characters);
/// reserved names pass this check, see [`is_reserved_slug`].
pub fn is_valid_slug(slug: &str) -> bool {
    let bytes = slug.as_bytes();
    (2..=32).contains(&bytes.len())
        && bytes[0].is_ascii_lowercase()
        && bytes
            .last()
            .is_some_and(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
        && bytes
            .iter()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || *b == b'-')
}

/// Whether the hub keeps this first path segment for itself.
pub fn is_reserved_slug(slug: &str) -> bool {
    RESERVED_SLUGS.contains(&slug)
}

/// Prefix of a pairing code.
pub const PAIRING_CODE_PREFIX: &str = "rhp_";
/// Characters of a pairing code after the prefix (160 bits).
pub const PAIRING_CODE_CHARS: usize = 32;
/// Prefix of an app secret.
pub const APP_SECRET_PREFIX: &str = "rhs_";

/// Crockford's Base32 alphabet.
pub const CROCKFORD: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";

/// A pairing code as typed: with or without spaces and hyphens, in any
/// case, with Crockford's look-alikes (`I`, `L` → `1`, `O` → `0`). Returns
/// the canonical form `rhp_` + 32 uppercase characters, or `None` if it
/// cannot be a code.
pub fn normalize_pairing_code(input: &str) -> Option<String> {
    let compact: String = input
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '-')
        .collect();
    let prefix = compact.get(..PAIRING_CODE_PREFIX.len())?;
    if !prefix.eq_ignore_ascii_case(PAIRING_CODE_PREFIX) {
        return None;
    }
    let mut code = String::from(PAIRING_CODE_PREFIX);
    for c in compact[PAIRING_CODE_PREFIX.len()..].chars() {
        let c = match c.to_ascii_uppercase() {
            'I' | 'L' => '1',
            'O' => '0',
            c => c,
        };
        if !c.is_ascii() || !CROCKFORD.contains(&(c as u8)) {
            return None;
        }
        code.push(c);
    }
    (code.len() == PAIRING_CODE_PREFIX.len() + PAIRING_CODE_CHARS).then_some(code)
}

/// The code in groups of four for display: `rhp_7K2M 9QXR …`.
pub fn display_pairing_code(code: &str) -> String {
    let Some(rest) = code.strip_prefix(PAIRING_CODE_PREFIX) else {
        return code.to_owned();
    };
    let groups: Vec<&str> = rest
        .as_bytes()
        .chunks(4)
        .map(|chunk| std::str::from_utf8(chunk).unwrap_or_default())
        .collect();
    format!("{PAIRING_CODE_PREFIX}{}", groups.join(" "))
}

/// Body of `POST /internal/v1/pair` (contract § 9). The manifest stays raw
/// JSON so that the hub can name every broken field.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PairRequest {
    pub code: String,
    pub public_url: String,
    pub manifest: Value,
}

/// Response of `POST /internal/v1/pair`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Paired {
    pub app_id: String,
    pub slug: String,
    pub app_secret: String,
    pub issuer: String,
    pub public_url: String,
    pub jwks: Jwks,
    pub contract: u64,
}

/// Response of `PUT /internal/v1/manifest` (contract § 7).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManifestAccepted {
    pub app_id: String,
    pub slug: String,
    pub issuer: String,
    pub public_url: String,
}

/// A user with access, as `GET /internal/v1/users[/<sub>]` lists them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct User {
    pub sub: String,
    pub username: String,
    pub email: String,
    pub name: String,
    /// Hub role: `admin` or `user`.
    pub role: String,
    pub app_role: String,
    /// `en` or `de`.
    pub locale: String,
    /// `active` or `locked`.
    pub status: String,
    /// Time zone, region and formats for texts the app writes for the user.
    pub prefs: UserPrefs,
}

/// A user's shared settings that an app's backend needs for texts and mails
/// (contract § 7, also in `user.updated`); values as the contract lists them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserPrefs {
    /// IANA time zone, e.g. `Europe/Berlin`.
    pub time_zone: String,
    /// `DE`, `AT`, `CH`, `FR`, `IT`, `NL`, `ES`, `GB`, `US`; `None`: not set.
    pub region: Option<String>,
    /// `de` (25.10.2026), `dmy` (25/10/2026), `iso` (2026-10-25) or `us` (10/25/2026).
    pub date_format: String,
    /// `24h` or `12h`.
    pub time_format: String,
    /// `monday` or `sunday`.
    pub week_start: String,
    /// `1.234,56`, `1,234.56`, `1\u{a0}234,56` or `1\u{2019}234.56`.
    pub number_format: String,
}

/// Response of `GET /internal/v1/users`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Users {
    pub users: Vec<User>,
    pub next_cursor: Option<String>,
}

/// Response of `GET /internal/v1/events` (contract § 10.3): event JWTs,
/// oldest first, each verified like a delivery.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventPage {
    pub events: Vec<String>,
    /// Further events follow after the last one.
    pub more: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slug_format() {
        for slug in ["mail", "ab", "a1", "mail-archive", "x0-9", &"a".repeat(32)] {
            assert!(is_valid_slug(slug), "{slug}");
        }
        for slug in [
            "",
            "a",
            "Mail",
            "1mail",
            "-mail",
            "mail-",
            "ma_il",
            "ma.il",
            "mäil",
            &"a".repeat(33),
        ] {
            assert!(!is_valid_slug(slug), "{slug}");
        }
    }

    #[test]
    fn reserved_slugs() {
        for slug in [
            "login", "account", "admin", "setup", "invite", "api", "internal", "healthz", "readyz",
            "apps", "auth", "branding", "error", "help", "hub", "logout", "oauth", "settings",
            "static",
        ] {
            assert!(is_valid_slug(slug) && is_reserved_slug(slug), "{slug}");
        }
        for slug in [".well-known", "_hub", "favicon.ico", "robots.txt"] {
            assert!(is_reserved_slug(slug) && !is_valid_slug(slug), "{slug}");
        }
        assert!(!is_reserved_slug("mail") && !is_reserved_slug("logins"));
    }

    #[test]
    fn pairing_codes_are_read_leniently() {
        let code = "rhp_7K2M9QXR0ABCDEFGHJKMNPQRSTVWXYZ2";
        assert_eq!(normalize_pairing_code(code).as_deref(), Some(code));
        for typed in [
            "RHP_7k2m 9qxr 0abc defg hjkm npqr stvw xyz2",
            " rhp_7K2M-9QXR-OABC-DEFG-HJKM-NPQR-STVW-XYZ2 ",
            "rhp_7K2M9QXR0ABCDEFGHJKMNPQRSTVWXYZ2\n",
        ] {
            assert_eq!(
                normalize_pairing_code(typed).as_deref(),
                Some(code),
                "{typed}"
            );
        }
        let look_alikes = format!("rhp_IL{}O", "0".repeat(29));
        assert_eq!(
            normalize_pairing_code(&look_alikes),
            Some(format!("rhp_11{}", "0".repeat(30)))
        );
        for wrong in [
            "",
            "rhp_",
            "rhs_7K2M9QXR0ABCDEFGHJKMNPQRSTVWXYZ2",
            "rhp_7K2M9QXR0ABCDEFGHJKMNPQRSTVWXYZ",
            "rhp_7K2M9QXR0ABCDEFGHJKMNPQRSTVWXYZ22",
            "rhp_7K2M9QXR0ABCDEFGHJKMNPQRSTVWXYZU",
            "rhp_7K2M9QXR0ABCDEFGHJKMNPQRSTVWXYZÄ",
        ] {
            assert_eq!(normalize_pairing_code(wrong), None, "{wrong}");
        }
        assert_eq!(
            display_pairing_code(code),
            "rhp_7K2M 9QXR 0ABC DEFG HJKM NPQR STVW XYZ2"
        );
    }
}
