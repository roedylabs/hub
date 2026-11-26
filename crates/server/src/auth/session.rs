//! Hub sessions: an opaque token in the cookie `__Host-roedy_hub_session`,
//! only its SHA-256 in the database. A session expires after 30 days without
//! activity; every request extends it (throttled, see
//! [`super::extract::resolve`]). The cookie lies outside every app's
//! namespace, so the gateway never forwards it (docs/app-contract.md ›
//! Namespaces in the browser).
//!
//! Taken over from Roedy Mail (`crates/server/src/auth/session.rs`) without
//! the installed app and the place; with `auth_time`.

use std::{fmt::Write as _, net::IpAddr, time::Duration};

use axum::http::{HeaderMap, HeaderValue, header};
use rusqlite::Connection;
use sha2::{Digest as _, Sha256};

use super::device::Device;
use crate::{
    crypto::SecretKey,
    db::repo::{
        sessions,
        users::{self, Profile, Role},
    },
    random,
    settings::Language,
};

/// `__Host-`: only with `Secure`, `Path=/` and without `Domain` – a subdomain
/// can thus neither set nor overwrite the cookie.
pub const COOKIE: &str = "__Host-roedy_hub_session";
/// Shared attributes; deleting only works with the same ones.
const ATTRIBUTES: &str = "Path=/; HttpOnly; Secure; SameSite=Lax";
/// Expiry without activity.
pub const IDLE: Duration = Duration::from_secs(30 * 24 * 60 * 60);
/// Random bytes per token (256 bits), as 64 hex characters in the cookie.
const TOKEN_BYTES: usize = 32;

/// Signed-in user of a valid session (active, not expired).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthUser {
    pub id: String,
    pub username: String,
    pub email: String,
    pub display_name: String,
    pub role: Role,
    /// ID of the session (`auth/sessions/:id`, `sid` for the apps).
    pub session_id: String,
    /// Last full sign-in in this session, Unix seconds (`auth_time`).
    pub auth_time: i64,
}

impl AuthUser {
    pub fn new(profile: Profile, session_id: String, auth_time: i64) -> Self {
        Self {
            id: profile.id,
            username: profile.username,
            email: profile.email,
            display_name: profile.display_name,
            role: profile.role,
            session_id,
            auth_time,
        }
    }

    pub fn is_admin(&self) -> bool {
        self.role == Role::Admin
    }
}

/// A session token in this server's format (64 characters `0-9a-f`). Only
/// such a token can build a cookie – arbitrary text from a request cannot
/// make the header invalid.
#[derive(Clone, PartialEq, Eq)]
pub struct Token(String);

impl Token {
    /// New token from the random source (also for the intermediate sign-in
    /// step, `auth::challenge`).
    pub fn generate() -> anyhow::Result<Self> {
        let mut token = String::with_capacity(2 * TOKEN_BYTES);
        for b in random::bytes::<TOKEN_BYTES>()? {
            let _ = write!(token, "{b:02x}");
        }
        Ok(Self(token))
    }

    /// Checks the format; anything else is certainly not a valid token (saves
    /// the database query).
    pub fn parse(value: &str) -> Option<Self> {
        let well_formed = value.len() == 2 * TOKEN_BYTES
            && value
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
        well_formed.then(|| Self(value.to_owned()))
    }

    /// The token as text, for links in mails (invitations, "Forgot
    /// password", email change).
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// SHA-256 for `sessions.token_hash`.
    pub fn hash(&self) -> [u8; 32] {
        Sha256::digest(self.0.as_bytes()).into()
    }

    /// `Set-Cookie` for the session, also for renewing. `persistent`
    /// ("Stay signed in"): with the full lifetime, otherwise without `Max-Age` –
    /// then the browser deletes the cookie when it closes.
    pub fn cookie(&self, persistent: bool) -> HeaderValue {
        self.cookie_named(COOKIE, persistent.then_some(IDLE))
    }

    /// `Set-Cookie` for a cookie `name` with the same attributes; without
    /// `max_age` until the browser closes.
    pub fn cookie_named(&self, name: &str, max_age: Option<Duration>) -> HeaderValue {
        let value = match max_age {
            Some(max_age) => format!(
                "{name}={}; Max-Age={}; {ATTRIBUTES}",
                self.0,
                max_age.as_secs()
            ),
            None => format!("{name}={}; {ATTRIBUTES}", self.0),
        };
        HeaderValue::try_from(value).expect("fixed name, token only hex characters")
    }
}

impl std::fmt::Debug for Token {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Token(***)")
    }
}

/// New session: the cookie goes to the browser exactly once.
pub struct Started {
    pub id: String,
    pub cookie: HeaderValue,
    /// Unix seconds, as in [`AuthUser::auth_time`].
    pub auth_time: i64,
}

/// What is known about the client at creation.
#[derive(Debug, Default, Clone)]
pub struct Client<'a> {
    pub user_agent: Option<&'a str>,
    pub ip: Option<IpAddr>,
    /// Language the browser prefers ([`accept_language`]); a session stores
    /// it as the user's sign-in language.
    pub lang: Option<Language>,
}

/// Origin of a request: user agent, client IP, language and the ID of the
/// device (`auth::device::Device::hash`) – for session, audit log, known
/// devices and security notices.
#[derive(Debug, Clone)]
pub struct Origin {
    pub user_agent: Option<String>,
    pub ip: IpAddr,
    pub lang: Option<Language>,
    pub device_hash: [u8; 32],
}

impl Origin {
    pub fn new(headers: &HeaderMap, ip: IpAddr, secrets: &SecretKey) -> Self {
        let user_agent = user_agent(headers);
        let device_hash = Device::parse(user_agent.as_deref()).hash(secrets, ip);
        Self {
            user_agent,
            ip,
            lang: accept_language(headers),
            device_hash,
        }
    }

    /// Device from the user agent, for the security notices.
    pub fn device(&self) -> Device {
        Device::parse(self.user_agent.as_deref())
    }

    pub fn client(&self) -> Client<'_> {
        Client {
            user_agent: self.user_agent.as_deref(),
            ip: Some(self.ip),
            lang: self.lang,
        }
    }
}

/// User agent of the request for [`Client`], truncated as in `sessions`.
pub fn user_agent(headers: &HeaderMap) -> Option<String> {
    headers
        .get(header::USER_AGENT)
        .and_then(|value| value.to_str().ok())
        .map(|ua| sessions::truncate(ua, sessions::MAX_USER_AGENT).to_owned())
}

/// Language the browser prefers, from `Accept-Language`
/// ([`Language::from_accept_language`]).
pub fn accept_language(headers: &HeaderMap) -> Option<Language> {
    headers
        .get(header::ACCEPT_LANGUAGE)
        .and_then(|value| value.to_str().ok())
        .and_then(Language::from_accept_language)
}

/// Creates a session for `user_id`; `persistent` as in [`Token::cookie`].
/// Synchronous so that callers can include it in their transaction (e.g.
/// `setup/complete`). Beyond [`sessions::MAX_PER_USER`] the user's least
/// recently used session ends. A language of the client becomes the user's
/// sign-in language (server texts for the language `system`).
pub fn start(
    conn: &Connection,
    user_id: &str,
    client: &Client<'_>,
    persistent: bool,
) -> anyhow::Result<Started> {
    let token = Token::generate()?;
    let (id, auth_time) = sessions::insert(conn, user_id, &token.hash(), client, persistent)?;
    sessions::trim(conn, user_id, &id, sessions::MAX_PER_USER)?;
    if let Some(lang) = client.lang {
        users::set_sign_in_lang(conn, user_id, lang)?;
    }
    Ok(Started {
        id,
        cookie: token.cookie(persistent),
        auth_time,
    })
}

/// Value of the session cookie from the cookie header (also across several
/// `Cookie` lines, HTTP/2), not yet checked.
pub fn cookie_value(headers: &HeaderMap) -> Option<&str> {
    named_cookie(headers, COOKIE)
}

/// Value of the cookie `name`, not yet checked. Reading goes per cookie, not
/// per line: a cookie with bytes outside ASCII, e.g. set by an app or another
/// subdomain, must not hide the session. Only ASCII whitespace is removed and
/// the name compared byte for byte: `str::trim` would turn
/// `\u{2000}__Host-…` into the name of the session cookie, although the
/// browser accepts such a cookie without the `__Host-` rules too.
pub fn named_cookie<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .flat_map(|value| value.as_bytes().split(|&b| b == b';'))
        .find_map(|pair| {
            let (key, value) = std::str::from_utf8(pair.trim_ascii())
                .ok()?
                .split_once('=')?;
            (key == name).then_some(value)
        })
}

/// `Set-Cookie` that deletes the session in the browser.
pub fn clear_cookie() -> HeaderValue {
    clear_named_cookie(COOKIE)
}

/// `Set-Cookie` that deletes the cookie `name` (attributes as when setting).
pub fn clear_named_cookie(name: &str) -> HeaderValue {
    HeaderValue::try_from(format!("{name}=; Max-Age=0; {ATTRIBUTES}"))
        .expect("fixed ASCII characters")
}

/// Whether a response already sets the session cookie itself (sign-in,
/// sign-out).
pub fn sets_cookie(headers: &HeaderMap) -> bool {
    headers.get_all(header::SET_COOKIE).iter().any(|v| {
        v.as_bytes()
            .strip_prefix(COOKIE.as_bytes())
            .is_some_and(|rest| rest.starts_with(b"="))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::repo::users::tests::{add, conn};

    #[test]
    fn tokens_are_random_hex() {
        let a = Token::generate().unwrap();
        let b = Token::generate().unwrap();
        assert_ne!(a, b);
        assert_eq!(Token::parse(&a.0).as_ref(), Some(&a));
        for bad in [
            "",
            "abc",
            &a.0.to_uppercase(),
            &format!("{}0", a.0),
            // Always one character outside hex.
            &format!("g{}", &a.0[1..]),
        ] {
            assert_eq!(Token::parse(bad), None, "{bad}");
        }
        assert_eq!(format!("{a:?}"), "Token(***)");
    }

    #[test]
    fn finds_cookie_among_others() {
        let mut headers = HeaderMap::new();
        headers.append(header::COOKIE, HeaderValue::from_static("a=1; theme=dark"));
        assert_eq!(cookie_value(&headers), None);
        headers.append(
            header::COOKIE,
            HeaderValue::from_static("x=2;__Host-roedy_hub_session=abc ; y=3"),
        );
        assert_eq!(cookie_value(&headers), Some("abc"));
        // An app's cookie with a similar name is not the session.
        let mut headers = HeaderMap::new();
        headers.append(
            header::COOKIE,
            HeaderValue::from_static("__Host-roedy-mail_session=app; __Host-roedy_hub_session_x=1"),
        );
        assert_eq!(cookie_value(&headers), None);
        // A cookie with UTF-8 in the same line does not hide the session.
        let mut headers = HeaderMap::new();
        headers.append(
            header::COOKIE,
            HeaderValue::from_bytes("x=ä; __Host-roedy_hub_session=abc".as_bytes()).unwrap(),
        );
        assert_eq!(cookie_value(&headers), Some("abc"));
        // Unicode whitespace belongs to the name: such a cookie is not a
        // `__Host-` cookie and must not shadow the real one.
        let mut headers = HeaderMap::new();
        headers.append(
            header::COOKIE,
            HeaderValue::from_bytes(
                "\u{2000}__Host-roedy_hub_session=foreign; __Host-roedy_hub_session=abc".as_bytes(),
            )
            .unwrap(),
        );
        assert_eq!(cookie_value(&headers), Some("abc"));
    }

    #[test]
    fn cookie_attributes() {
        let token = Token("0".repeat(64));
        assert_eq!(
            token.cookie(true).to_str().unwrap(),
            format!(
                "__Host-roedy_hub_session={}; Max-Age=2592000; Path=/; HttpOnly; Secure; \
                 SameSite=Lax",
                "0".repeat(64)
            )
        );
        assert_eq!(
            token.cookie(false).to_str().unwrap(),
            format!(
                "__Host-roedy_hub_session={}; Path=/; HttpOnly; Secure; SameSite=Lax",
                "0".repeat(64)
            )
        );
        assert_eq!(
            clear_cookie().to_str().unwrap(),
            "__Host-roedy_hub_session=; Max-Age=0; Path=/; HttpOnly; Secure; SameSite=Lax"
        );

        let mut headers = HeaderMap::new();
        headers.append(header::SET_COOKIE, HeaderValue::from_static("other=1"));
        headers.append(
            header::SET_COOKIE,
            HeaderValue::from_static("__Host-roedy_hub_session_old=1"),
        );
        assert!(!sets_cookie(&headers));
        headers.append(header::SET_COOKIE, clear_cookie());
        assert!(sets_cookie(&headers));
    }

    #[test]
    fn start_stores_only_the_hash_and_the_language() {
        let conn = conn();
        let user = add(&conn, "max", "max@example.com", Role::User);
        let client = Client {
            lang: Some(Language::German),
            ..Client::default()
        };
        let started = start(&conn, &user, &client, true).unwrap();
        let cookie = started.cookie.to_str().unwrap();
        let value = cookie
            .strip_prefix("__Host-roedy_hub_session=")
            .and_then(|rest| rest.split(';').next())
            .unwrap();
        let token = Token::parse(value).unwrap();
        let stored: Vec<u8> = conn
            .query_row(
                "SELECT token_hash FROM sessions WHERE id = ?1",
                [&started.id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(stored, token.hash());
        assert_eq!(
            users::sign_in_lang(&conn, &user).unwrap(),
            Some(Language::German)
        );
    }

    #[test]
    fn origin_reads_agent_and_language() {
        let mut headers = HeaderMap::new();
        headers.insert(header::USER_AGENT, HeaderValue::from_static("Firefox/140"));
        headers.insert(header::ACCEPT_LANGUAGE, HeaderValue::from_static("de-DE"));
        let key = SecretKey::random().unwrap();
        let origin = Origin::new(&headers, "198.51.100.1".parse().unwrap(), &key);
        assert_eq!(origin.user_agent.as_deref(), Some("Firefox/140"));
        assert_eq!(origin.lang, Some(Language::German));
        let client = origin.client();
        assert_eq!(client.ip, Some("198.51.100.1".parse().unwrap()));
        let other = Origin::new(&HeaderMap::new(), "198.51.100.1".parse().unwrap(), &key);
        assert_ne!(other.device_hash, origin.device_hash, "another browser");
        assert_eq!((other.user_agent, other.lang), (None, None));
    }
}
