//! The manifest an app describes itself with (contract § 8), and its rules.
//!
//! [`Manifest::parse`] checks a manifest as JSON the way the hub does when
//! an app pairs or sends it again: field by field, so that the answer names
//! every field that breaks a rule (`400 invalid_manifest` with `fields`).
//! Unknown fields are ignored. The icon is only checked for its type and
//! size here; the hub sanitizes the SVG itself and rejects what it cannot
//! sanitize or what has no square `viewBox`.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// The contract versions this crate implements.
pub const SUPPORTED_CONTRACTS: [u64; 1] = [1];

/// Longest icon, in bytes of the SVG document.
pub const MAX_ICON_BYTES: usize = 64 * 1024;
/// Longest name per language, in characters.
pub const MAX_NAME_CHARS: usize = 40;
/// Longest description per language, in characters.
pub const MAX_DESCRIPTION_CHARS: usize = 200;
/// Most roles per app.
pub const MAX_ROLES: usize = 16;
/// Longest `home`, in bytes.
pub const MAX_HOME_LEN: usize = 256;
/// Longest `version`, in bytes.
pub const MAX_VERSION_LEN: usize = 64;
/// Languages every name and description has.
pub const REQUIRED_LANGUAGES: [&str; 2] = ["en", "de"];

/// Texts by language code, e.g. `{"en": "Mail", "de": "Mail"}`.
pub type Texts = BTreeMap<String, String>;

/// A manifest that keeps the rules of contract § 8.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    pub contract: u64,
    pub id: String,
    pub version: String,
    pub name: Texts,
    pub description: Texts,
    /// The SVG document.
    pub icon: String,
    #[serde(default = "default_home")]
    pub home: String,
    pub roles: Vec<Role>,
    /// The app uses users' Stalwart accounts (contract § 8, § 6.1).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stalwart: Option<Stalwart>,
}

/// What the app needs of users' Stalwart accounts (`stalwart` in the
/// manifest). Present at all: the app gets the `stalwart.*` events and may
/// read the servers' origins over the internal API.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Stalwart {
    /// The app's backend fetches access tokens for users' accounts over the
    /// internal API, without a request from the user.
    #[serde(default, skip_serializing_if = "is_false")]
    pub backend: bool,
}

/// A role in the app (contract § 8, § 11).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Role {
    pub id: String,
    pub name: Texts,
    pub description: Texts,
    /// The role new users get.
    #[serde(default, skip_serializing_if = "is_false")]
    pub default: bool,
    /// The role that gives full control of the app.
    #[serde(default, skip_serializing_if = "is_false")]
    pub admin: bool,
}

#[expect(
    clippy::trivially_copy_pass_by_ref,
    reason = "serde passes a reference"
)]
fn is_false(value: &bool) -> bool {
    !*value
}

fn default_home() -> String {
    "/".to_owned()
}

/// Why a manifest is not accepted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ManifestError {
    /// `contract` names a version the hub does not implement:
    /// `400 unsupported_contract` with `supported`.
    UnsupportedContract,
    /// Fields break the rules: `400 invalid_manifest` with `fields`, per
    /// field (a path such as `roles.1.id`) a code.
    Invalid(BTreeMap<String, &'static str>),
}

impl Manifest {
    /// Checks a manifest. `contract` is read first: a version this crate
    /// does not implement is [`ManifestError::UnsupportedContract`] whatever
    /// else the manifest holds. Every other rule is checked and all broken
    /// ones are named.
    pub fn parse(value: &Value) -> Result<Self, ManifestError> {
        let mut errors = Errors::default();
        let Some(object) = value.as_object() else {
            errors.add("manifest", "invalid");
            return Err(errors.into_error());
        };
        match object.get("contract") {
            None | Some(Value::Null) => errors.add("contract", "required"),
            Some(Value::Number(n)) if n.as_u64().is_some() => {
                if !n.as_u64().is_some_and(|v| SUPPORTED_CONTRACTS.contains(&v)) {
                    return Err(ManifestError::UnsupportedContract);
                }
            }
            Some(_) => errors.add("contract", "invalid"),
        }
        let id = errors.string(object, "id").filter(|id| {
            let valid = is_valid_manifest_id(id);
            if !valid {
                errors.add("id", "invalid");
            }
            valid
        });
        let version = errors.string(object, "version").filter(|version| {
            let valid = version.len() <= MAX_VERSION_LEN && is_semver(version);
            if !valid {
                errors.add("version", "invalid");
            }
            valid
        });
        let name = errors.texts(object, "name", 1, MAX_NAME_CHARS);
        let description = errors.texts(object, "description", 0, MAX_DESCRIPTION_CHARS);
        let icon = errors.string(object, "icon").filter(|icon| {
            let fits = icon.len() <= MAX_ICON_BYTES;
            if !fits {
                errors.add("icon", "too_large");
            }
            fits
        });
        let home = match object.get("home") {
            None | Some(Value::Null) => Some(default_home()),
            Some(Value::String(home)) if is_valid_home(home) => Some(home.clone()),
            Some(_) => {
                errors.add("home", "invalid");
                None
            }
        };
        let roles = errors.roles(object);
        let stalwart = errors.stalwart(object);
        match (id, version, name, description, icon, home, roles) {
            (
                Some(id),
                Some(version),
                Some(name),
                Some(description),
                Some(icon),
                Some(home),
                Some(roles),
            ) if errors.0.is_empty() => Ok(Self {
                contract: 1,
                id,
                version,
                name,
                description,
                icon,
                home,
                roles,
                stalwart,
            }),
            _ => Err(errors.into_error()),
        }
    }

    /// The role new users get: exactly one after [`Self::parse`]; `None`
    /// only for a manifest built or deserialized without it.
    pub fn default_role(&self) -> Option<&Role> {
        self.roles.iter().find(|role| role.default)
    }

    /// The role that gives full control, if the app has one.
    pub fn admin_role(&self) -> Option<&Role> {
        self.roles.iter().find(|role| role.admin)
    }

    /// The role administrators of the hub get after pairing (contract
    /// § 11): the admin role, otherwise the default role.
    pub fn role_for_hub_admins(&self) -> Option<&Role> {
        self.admin_role().or_else(|| self.default_role())
    }

    /// Whether the app has a role with this ID.
    pub fn has_role(&self, id: &str) -> bool {
        self.roles.iter().any(|role| role.id == id)
    }

    /// The name in `lang`, else in English.
    pub fn name_in(&self, lang: &str) -> &str {
        text_in(&self.name, lang)
    }

    /// The description in `lang`, else in English.
    pub fn description_in(&self, lang: &str) -> &str {
        text_in(&self.description, lang)
    }
}

/// The text in `lang`, else in English (always there after
/// [`Manifest::parse`]).
pub fn text_in<'a>(texts: &'a Texts, lang: &str) -> &'a str {
    texts
        .get(lang)
        .or_else(|| texts.get("en"))
        .map_or("", String::as_str)
}

/// Collects the broken rules.
#[derive(Default)]
struct Errors(BTreeMap<String, &'static str>);

impl Errors {
    fn add(&mut self, field: &str, code: &'static str) {
        self.0.entry(field.to_owned()).or_insert(code);
    }

    fn into_error(self) -> ManifestError {
        ManifestError::Invalid(self.0)
    }

    /// A required string field.
    fn string(&mut self, object: &Map<String, Value>, field: &str) -> Option<String> {
        self.string_at(object, field, field)
    }

    fn string_at(&mut self, object: &Map<String, Value>, key: &str, path: &str) -> Option<String> {
        match object.get(key) {
            None | Some(Value::Null) => {
                self.add(path, "required");
                None
            }
            Some(Value::String(value)) => Some(value.clone()),
            Some(_) => {
                self.add(path, "invalid");
                None
            }
        }
    }

    /// Texts by language with `en` and `de`, each `min..=max` characters.
    fn texts(
        &mut self,
        object: &Map<String, Value>,
        field: &str,
        min: usize,
        max: usize,
    ) -> Option<Texts> {
        self.texts_at(object.get(field), field, min, max)
    }

    fn texts_at(
        &mut self,
        value: Option<&Value>,
        path: &str,
        min: usize,
        max: usize,
    ) -> Option<Texts> {
        let object = match value {
            None | Some(Value::Null) => {
                self.add(path, "required");
                return None;
            }
            Some(Value::Object(object)) => object,
            Some(_) => {
                self.add(path, "invalid");
                return None;
            }
        };
        let before = self.0.len();
        let mut texts = Texts::new();
        for (lang, text) in object {
            let field = format!("{path}.{lang}");
            if !is_language(lang) {
                self.add(&field, "invalid");
                continue;
            }
            let Some(text) = text.as_str() else {
                self.add(&field, "invalid");
                continue;
            };
            let chars = text.chars().count();
            if chars < min || text.trim().len() < min {
                self.add(&field, "required");
            } else if chars > max {
                self.add(&field, "too_long");
            } else if text.chars().any(char::is_control) {
                self.add(&field, "invalid");
            } else {
                texts.insert(lang.clone(), text.to_owned());
            }
        }
        for lang in REQUIRED_LANGUAGES {
            if !object.contains_key(lang) {
                self.add(&format!("{path}.{lang}"), "required");
            }
        }
        (self.0.len() == before).then_some(texts)
    }

    /// The optional `stalwart` object with an optional `backend` flag;
    /// unknown fields in it are ignored like everywhere else.
    fn stalwart(&mut self, object: &Map<String, Value>) -> Option<Stalwart> {
        let fields = match object.get("stalwart") {
            None | Some(Value::Null) => return None,
            Some(Value::Object(fields)) => fields,
            Some(_) => {
                self.add("stalwart", "invalid");
                return None;
            }
        };
        let backend = match fields.get("backend") {
            None | Some(Value::Null) => false,
            Some(Value::Bool(value)) => *value,
            Some(_) => {
                self.add("stalwart.backend", "invalid");
                false
            }
        };
        Some(Stalwart { backend })
    }

    /// 1 to [`MAX_ROLES`] roles with unique IDs, exactly one default and at
    /// most one admin role.
    fn roles(&mut self, object: &Map<String, Value>) -> Option<Vec<Role>> {
        let list = match object.get("roles") {
            None | Some(Value::Null) => {
                self.add("roles", "required");
                return None;
            }
            Some(Value::Array(list)) => list,
            Some(_) => {
                self.add("roles", "invalid");
                return None;
            }
        };
        if list.is_empty() {
            self.add("roles", "required");
            return None;
        }
        if list.len() > MAX_ROLES {
            self.add("roles", "too_many");
            return None;
        }
        let before = self.0.len();
        let mut roles = Vec::with_capacity(list.len());
        // Every valid ID seen, also of roles with other broken fields, so
        // that a duplicate is named together with them.
        let mut ids = std::collections::BTreeSet::new();
        for (i, value) in list.iter().enumerate() {
            let path = format!("roles.{i}");
            let Some(role) = value.as_object() else {
                self.add(&path, "invalid");
                continue;
            };
            let id = self.string_at(role, "id", &format!("{path}.id"));
            let id = id.filter(|id| {
                let valid = is_valid_role_id(id);
                if !valid {
                    self.add(&format!("{path}.id"), "invalid");
                } else if !ids.insert(id.clone()) {
                    self.add(&format!("{path}.id"), "duplicate");
                }
                valid
            });
            let name = self.texts_at(role.get("name"), &format!("{path}.name"), 1, MAX_NAME_CHARS);
            let description = self.texts_at(
                role.get("description"),
                &format!("{path}.description"),
                0,
                MAX_DESCRIPTION_CHARS,
            );
            let mut flag = |key: &str| match role.get(key) {
                None | Some(Value::Null) => false,
                Some(Value::Bool(value)) => *value,
                Some(_) => {
                    self.add(&format!("{path}.{key}"), "invalid");
                    false
                }
            };
            let (default, admin) = (flag("default"), flag("admin"));
            if let (Some(id), Some(name), Some(description)) = (id, name, description) {
                roles.push(Role {
                    id,
                    name,
                    description,
                    default,
                    admin,
                });
            }
        }
        if self.0.len() != before {
            return None;
        }
        if roles.iter().filter(|role| role.default).count() != 1 {
            self.add("roles", "default_role");
            return None;
        }
        if roles.iter().filter(|role| role.admin).count() > 1 {
            self.add("roles", "admin_role");
            return None;
        }
        Some(roles)
    }
}

/// `id` of a manifest: `^[a-z][a-z0-9.-]{1,63}$`.
pub fn is_valid_manifest_id(id: &str) -> bool {
    let bytes = id.as_bytes();
    (2..=64).contains(&bytes.len())
        && bytes[0].is_ascii_lowercase()
        && bytes[1..]
            .iter()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'.' | b'-'))
}

/// `id` of a role: `^[a-z][a-z0-9_-]{0,31}$`.
pub fn is_valid_role_id(id: &str) -> bool {
    let bytes = id.as_bytes();
    (1..=32).contains(&bytes.len())
        && bytes[0].is_ascii_lowercase()
        && bytes[1..]
            .iter()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'_' | b'-'))
}

/// A language code as a key of [`Texts`]: two or three lowercase letters,
/// optionally followed by subtags (`pt-BR`, `zh-Hant`).
fn is_language(code: &str) -> bool {
    let mut parts = code.split('-');
    let primary = parts.next().unwrap_or_default();
    (2..=3).contains(&primary.len())
        && primary.bytes().all(|b| b.is_ascii_lowercase())
        && code.len() <= 35
        && parts.all(|part| {
            (1..=8).contains(&part.len()) && part.bytes().all(|b| b.is_ascii_alphanumeric())
        })
}

/// `home`: a path that starts with exactly one `/`, without `\`, control
/// characters, spaces, fragment or `.`/`..` segments, so that
/// `/<slug><home>` stays inside the app's prefix.
pub fn is_valid_home(home: &str) -> bool {
    let path = home.split('?').next().unwrap_or_default();
    home.len() <= MAX_HOME_LEN
        && home.starts_with('/')
        && !home.starts_with("//")
        && !home.contains(['\\', '#'])
        && !home.chars().any(|c| c.is_control() || c.is_whitespace())
        && path.split('/').all(|segment| {
            // Browsers read `%2e` as a dot in a path segment too.
            let segment = segment.to_ascii_lowercase().replace("%2e", ".");
            segment != "." && segment != ".."
        })
}

/// SemVer 2.0: `MAJOR.MINOR.PATCH`, optionally `-pre.release` and
/// `+build.meta`; numbers without leading zeros.
pub fn is_semver(version: &str) -> bool {
    let (rest, build) = match version.split_once('+') {
        Some((rest, build)) => (rest, Some(build)),
        None => (version, None),
    };
    let (core, pre) = match rest.split_once('-') {
        Some((core, pre)) => (core, Some(pre)),
        None => (rest, None),
    };
    let number = |part: &str| {
        !part.is_empty()
            && part.bytes().all(|b| b.is_ascii_digit())
            && (part == "0" || !part.starts_with('0'))
    };
    let identifier = |part: &str| {
        !part.is_empty() && part.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
    };
    let core_parts: Vec<&str> = core.split('.').collect();
    core_parts.len() == 3
        && core_parts.iter().all(|part| number(part))
        && pre.is_none_or(|pre| {
            pre.split('.').all(|part| {
                identifier(part) && (!part.bytes().all(|b| b.is_ascii_digit()) || number(part))
            })
        })
        && build.is_none_or(|build| build.split('.').all(identifier))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    pub(crate) fn example() -> Value {
        json!({
            "contract": 1,
            "id": "roedy.mail",
            "version": "0.6.0",
            "name": { "en": "Mail", "de": "Mail" },
            "description": { "en": "Email for the team", "de": "E-Mail für das Team" },
            "icon": "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 24 24\"><path d=\"M0 0h24v24H0z\"/></svg>",
            "home": "/",
            "roles": [
                { "id": "user", "name": { "en": "User", "de": "Benutzer" },
                  "description": { "en": "Reads and sends email", "de": "Liest und sendet E-Mails" },
                  "default": true },
                { "id": "admin", "name": { "en": "Administrator", "de": "Administrator" },
                  "description": { "en": "Also manages mail servers", "de": "Verwaltet auch die Mailserver" },
                  "admin": true }
            ],
            "x-experiment": true
        })
    }

    fn fields(value: &Value) -> Vec<(String, &'static str)> {
        match Manifest::parse(value) {
            Err(ManifestError::Invalid(fields)) => fields.into_iter().collect(),
            other => panic!("not invalid: {other:?}"),
        }
    }

    fn with(change: impl FnOnce(&mut Value)) -> Value {
        let mut value = example();
        change(&mut value);
        value
    }

    #[test]
    fn the_contract_example_is_valid() {
        let manifest = Manifest::parse(&example()).unwrap();
        assert_eq!(manifest.id, "roedy.mail");
        assert_eq!(manifest.default_role().unwrap().id, "user");
        assert_eq!(manifest.admin_role().unwrap().id, "admin");
        assert_eq!(manifest.role_for_hub_admins().unwrap().id, "admin");
        assert_eq!(manifest.name_in("de"), "Mail");
        assert_eq!(manifest.name_in("fr"), "Mail");
        assert!(manifest.has_role("user") && !manifest.has_role("owner"));
        // Round trip: the stored form parses again.
        let stored = serde_json::to_value(&manifest).unwrap();
        assert_eq!(Manifest::parse(&stored).unwrap(), manifest);
        assert!(stored.get("x-experiment").is_none());
        // `home` defaults to `/`.
        let without = with(|v| {
            v.as_object_mut().unwrap().remove("home");
        });
        assert_eq!(Manifest::parse(&without).unwrap().home, "/");
        // Without an admin role, administrators get the default role.
        let no_admin = with(|v| {
            v["roles"][1]["admin"] = json!(false);
        });
        assert_eq!(
            Manifest::parse(&no_admin)
                .unwrap()
                .role_for_hub_admins()
                .unwrap()
                .id,
            "user"
        );
    }

    #[test]
    fn other_contract_versions_are_unsupported() {
        for contract in [json!(0), json!(2), json!(99)] {
            let value = with(|v| v["contract"] = contract.clone());
            assert_eq!(
                Manifest::parse(&value),
                Err(ManifestError::UnsupportedContract),
                "{contract}"
            );
        }
        // Unsupported wins over other errors.
        let value = with(|v| {
            v["contract"] = json!(2);
            v["id"] = json!("X");
        });
        assert_eq!(
            Manifest::parse(&value),
            Err(ManifestError::UnsupportedContract)
        );
        for contract in [json!("1"), json!(1.5), json!(-1)] {
            let value = with(|v| v["contract"] = contract.clone());
            assert_eq!(fields(&value), [("contract".into(), "invalid")]);
        }
    }

    #[test]
    fn names_every_broken_field() {
        let value = json!({
            "id": "Roedy Mail",
            "version": "1.0",
            "name": { "en": "", "fr": "Courrier" },
            "description": { "en": "x".repeat(201), "de": "ok" },
            "icon": 5,
            "home": "//evil.example",
            "roles": [],
        });
        assert_eq!(
            fields(&value),
            [
                ("contract".into(), "required"),
                ("description.en".into(), "too_long"),
                ("home".into(), "invalid"),
                ("icon".into(), "invalid"),
                ("id".into(), "invalid"),
                ("name.de".into(), "required"),
                ("name.en".into(), "required"),
                ("roles".into(), "required"),
                ("version".into(), "invalid"),
            ]
        );
        assert_eq!(fields(&json!([])), [("manifest".into(), "invalid")]);
    }

    #[test]
    fn icon_size_is_limited() {
        let big = format!("<svg>{}</svg>", " ".repeat(MAX_ICON_BYTES));
        assert_eq!(
            fields(&with(|v| v["icon"] = json!(big))),
            [("icon".into(), "too_large")]
        );
    }

    #[test]
    fn role_rules() {
        let two_defaults = with(|v| v["roles"][1]["default"] = json!(true));
        assert_eq!(fields(&two_defaults), [("roles".into(), "default_role")]);
        let no_default = with(|v| v["roles"][0]["default"] = json!(false));
        assert_eq!(fields(&no_default), [("roles".into(), "default_role")]);
        let two_admins = with(|v| v["roles"][0]["admin"] = json!(true));
        assert_eq!(fields(&two_admins), [("roles".into(), "admin_role")]);
        let duplicate = with(|v| v["roles"][1]["id"] = json!("user"));
        assert_eq!(fields(&duplicate), [("roles.1.id".into(), "duplicate")]);
        // Named together with another broken field of the first role.
        let duplicate_and_more = with(|v| {
            v["roles"][1]["id"] = json!("user");
            v["roles"][0]["name"] = json!({ "en": "User" });
        });
        assert_eq!(
            fields(&duplicate_and_more),
            [
                ("roles.0.name.de".into(), "required"),
                ("roles.1.id".into(), "duplicate"),
            ]
        );
        // Built without the rules: no default role, no panic.
        let mut unchecked = Manifest::parse(&example()).unwrap();
        unchecked.roles.clear();
        assert!(unchecked.default_role().is_none() && unchecked.role_for_hub_admins().is_none());
        let bad = with(|v| {
            v["roles"][0]["id"] = json!("User");
            v["roles"][1]["admin"] = json!("yes");
            v["roles"][1]["name"] = json!({ "en": "Admin" });
        });
        assert_eq!(
            fields(&bad),
            [
                ("roles.0.id".into(), "invalid"),
                ("roles.1.admin".into(), "invalid"),
                ("roles.1.name.de".into(), "required"),
            ]
        );
        let many = with(|v| {
            let role = v["roles"][1].clone();
            let roles = v["roles"].as_array_mut().unwrap();
            for i in 0..MAX_ROLES {
                let mut role = role.clone();
                role["id"] = json!(format!("r{i}"));
                role["admin"] = json!(false);
                roles.push(role);
            }
        });
        assert_eq!(fields(&many), [("roles".into(), "too_many")]);
    }

    #[test]
    fn identifiers() {
        for id in [
            "roedy.mail",
            "ab",
            "a-b.c9",
            &format!("a{}", "b".repeat(63)),
        ] {
            assert!(is_valid_manifest_id(id), "{id}");
        }
        for id in [
            "a",
            "9mail",
            "Mail",
            "roedy_mail",
            &format!("a{}", "b".repeat(64)),
        ] {
            assert!(!is_valid_manifest_id(id), "{id}");
        }
        for id in [
            "user",
            "a",
            "mail_admin",
            "r-2",
            &format!("a{}", "b".repeat(31)),
        ] {
            assert!(is_valid_role_id(id), "{id}");
        }
        for id in ["", "User", "_a", "a.b", &format!("a{}", "b".repeat(32))] {
            assert!(!is_valid_role_id(id), "{id}");
        }
    }

    #[test]
    fn languages_and_texts() {
        for code in ["en", "de", "pt-BR", "zh-Hant", "gsw"] {
            assert!(is_language(code), "{code}");
        }
        for code in ["", "e", "EN", "english", "en_US", "en-", "en-toolongsubtag"] {
            assert!(!is_language(code), "{code}");
        }
        let extra = with(|v| v["name"]["pt-BR"] = json!("Correio"));
        assert_eq!(Manifest::parse(&extra).unwrap().name["pt-BR"], "Correio");
        let control = with(|v| v["name"]["en"] = json!("Ma\u{7}il"));
        assert_eq!(fields(&control), [("name.en".into(), "invalid")]);
        let blank = with(|v| v["name"]["en"] = json!("   "));
        assert_eq!(fields(&blank), [("name.en".into(), "required")]);
        let empty_description = with(|v| v["description"]["en"] = json!(""));
        assert!(Manifest::parse(&empty_description).is_ok());
        let forty = with(|v| v["name"]["de"] = json!("ä".repeat(40)));
        assert!(Manifest::parse(&forty).is_ok(), "characters, not bytes");
    }

    #[test]
    fn stalwart_accounts() {
        assert_eq!(Manifest::parse(&example()).unwrap().stalwart, None);
        let browser = with(|v| v["stalwart"] = json!({ "x-later": 1 }));
        let parsed = Manifest::parse(&browser).unwrap();
        assert_eq!(parsed.stalwart, Some(Stalwart { backend: false }));
        // Stored without the default, and it parses again.
        let stored = serde_json::to_value(&parsed).unwrap();
        assert_eq!(stored["stalwart"], json!({}));
        assert_eq!(Manifest::parse(&stored).unwrap(), parsed);
        let backend = with(|v| v["stalwart"] = json!({ "backend": true }));
        assert_eq!(
            Manifest::parse(&backend).unwrap().stalwart,
            Some(Stalwart { backend: true })
        );
        let null = with(|v| v["stalwart"] = Value::Null);
        assert_eq!(Manifest::parse(&null).unwrap().stalwart, None);
        assert_eq!(
            fields(&with(|v| v["stalwart"] = json!(true))),
            [("stalwart".to_owned(), "invalid")]
        );
        assert_eq!(
            fields(&with(|v| v["stalwart"] = json!({ "backend": "yes" }))),
            [("stalwart.backend".to_owned(), "invalid")]
        );
    }

    #[test]
    fn home_paths() {
        for home in ["/", "/inbox", "/a/b?x=1", "/a.b/c"] {
            assert!(is_valid_home(home), "{home}");
        }
        for home in [
            "",
            "inbox",
            "//x",
            "/a\\b",
            "/a#b",
            "/a b",
            "/../admin",
            "/a/./b",
            "/a/..",
            "/%2e%2e/admin",
            "/%2E%2E",
            "/a/.%2e/b",
            "/%2e/x?y",
        ] {
            assert!(!is_valid_home(home), "{home}");
        }
    }

    #[test]
    fn semantic_versions() {
        for version in [
            "0.6.0",
            "1.2.3",
            "10.20.30",
            "1.0.0-alpha",
            "1.0.0-alpha.1",
            "1.0.0-0.3.7",
            "1.0.0+20130313144700",
            "1.0.0-beta+exp.sha.5114f85",
            "1.0.0-x-y-z.--",
        ] {
            assert!(is_semver(version), "{version}");
        }
        for version in [
            "1",
            "1.2",
            "1.2.3.4",
            "01.2.3",
            "1.02.3",
            "1.2.3-",
            "1.2.3-01",
            "1.2.3+",
            "1.2.3-a..b",
            "v1.2.3",
            "1.2.3 ",
        ] {
            assert!(!is_semver(version), "{version}");
        }
    }
}
