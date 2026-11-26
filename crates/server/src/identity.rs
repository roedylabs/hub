//! Identity tokens for the `X-Roedy-Identity` header (docs/app-contract.md ›
//! Identity): signed per (session, app) with the current signing key, valid
//! for [`LIFETIME`].
//!
//! The gateway asks for a token on every forwarded request, so tokens are
//! cached: a request gets the cached token of its session and app while it
//! is younger than [`REUSE`] – half its lifetime, so that every token an app
//! receives is still valid for at least 30 s – and while the claims and the
//! signing key are the same. A change of name, role or language, a fresh
//! sign-in (`auth_time`) or a new signing key gives a new token at once.
//! [`Identities::forget_session`] drops a revoked session's tokens.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use jiff::Timestamp;
use roedy_hub_contract::Identity;

use crate::{
    auth::session::AuthUser, db::repo::users::Role, settings::Language, signing::KeyStore,
};

/// Lifetime of a token (`exp − iat`), the most the contract allows.
pub const LIFETIME: i64 = 60;
/// A cached token is used for this long after `iat`, not longer.
pub const REUSE: i64 = 30;
/// Past this many cached tokens, stale ones are dropped on a miss – at most
/// once per [`REUSE`], so that many live tokens do not cost a scan each time.
const PRUNE_FROM: usize = 1024;

/// What the hub sends about a user to one app: the claims without issuer,
/// audience and times.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Subject {
    pub sub: String,
    pub username: String,
    pub email: String,
    pub name: String,
    pub role: Role,
    /// The user's role in the app, an `id` from its manifest.
    pub app_role: String,
    /// Language for the app's texts: German or English.
    pub locale: Language,
    pub sid: String,
    /// Unix seconds.
    pub auth_time: i64,
}

impl Subject {
    /// The signed-in user of a session, with their role in the app and
    /// their language as the server's texts use it (`settings::lang_of`).
    pub fn new(user: &AuthUser, app_role: impl Into<String>, locale: Language) -> Self {
        Self {
            sub: user.id.clone(),
            username: user.username.clone(),
            email: user.email.clone(),
            name: user.display_name.clone(),
            role: user.role,
            app_role: app_role.into(),
            locale,
            sid: user.session_id.clone(),
            auth_time: user.auth_time,
        }
    }

    fn claims(&self, iss: &str, aud: &str, iat: i64) -> Identity {
        Identity {
            iss: iss.to_owned(),
            aud: aud.to_owned(),
            sub: self.sub.clone(),
            username: self.username.clone(),
            email: self.email.clone(),
            name: self.name.clone(),
            role: self.role.as_str().to_owned(),
            app_role: self.app_role.clone(),
            // `System` is resolved before; should it come here, English.
            locale: self.locale.or_sign_in(None).code().to_owned(),
            sid: self.sid.clone(),
            auth_time: self.auth_time,
            iat,
            exp: iat + LIFETIME,
        }
    }
}

struct Cached {
    subject: Subject,
    kid: String,
    iat: i64,
    token: Arc<str>,
}

/// Issues and caches the identity tokens of one hub.
pub struct Identities {
    /// `iss`: the hub's public URL without trailing slash.
    issuer: String,
    cache: Mutex<Cache>,
}

#[derive(Default)]
struct Cache {
    tokens: HashMap<(String, String), Cached>,
    /// Unix seconds of the last pruning.
    pruned_at: Option<i64>,
}

impl Identities {
    /// For the hub at `public_url`.
    pub fn new(public_url: &url::Url) -> Self {
        Self {
            issuer: public_url.as_str().trim_end_matches('/').to_owned(),
            cache: Mutex::default(),
        }
    }

    /// The issuer, `iss` of every token.
    pub fn issuer(&self) -> &str {
        &self.issuer
    }

    /// The token for `subject` in the app `aud` (its slug) at `now`, from the
    /// cache if possible. Fails only without a signing key.
    pub fn issue(
        &self,
        keys: &KeyStore,
        aud: &str,
        subject: &Subject,
        now: Timestamp,
    ) -> anyhow::Result<Arc<str>> {
        let signer = keys
            .signer(now)
            .ok_or_else(|| anyhow::anyhow!("no signing key"))?;
        let now = now.as_second();
        let key = (subject.sid.clone(), aud.to_owned());
        {
            let cache = self.lock();
            if let Some(cached) = cache.tokens.get(&key)
                && cached.kid == signer.kid()
                && (0..REUSE).contains(&(now - cached.iat))
                && cached.subject == *subject
            {
                return Ok(cached.token.clone());
            }
        }

        let token: Arc<str> = signer.sign(&subject.claims(&self.issuer, aud, now))?.into();
        let mut cache = self.lock();
        let due = cache
            .pruned_at
            .is_none_or(|at| !(0..REUSE).contains(&(now - at)));
        if cache.tokens.len() >= PRUNE_FROM && due {
            cache
                .tokens
                .retain(|_, cached| (0..REUSE).contains(&(now - cached.iat)));
            cache.pruned_at = Some(now);
        }
        cache.tokens.insert(
            key,
            Cached {
                subject: subject.clone(),
                kid: signer.kid().to_owned(),
                iat: now,
                token: token.clone(),
            },
        );
        Ok(token)
    }

    /// Drops the cached tokens of a session, when it is revoked.
    pub fn forget_session(&self, sid: &str) {
        self.lock().tokens.retain(|(session, _), _| session != sid);
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Cache> {
        self.cache.lock().unwrap_or_else(|e| e.into_inner())
    }

    #[cfg(test)]
    fn cached(&self) -> usize {
        self.lock().tokens.len()
    }

    #[cfg(test)]
    fn pruned_at(&self) -> Option<i64> {
        self.lock().pruned_at
    }
}

#[cfg(test)]
mod tests {
    use jiff::SignedDuration;
    use roedy_hub_contract::{Expected, verify};
    use rusqlite::Connection;

    use super::*;
    use crate::{
        crypto::SecretKey,
        db::migrations,
        signing::{self, PUBLISH_BEFORE, ROTATE_AFTER},
    };

    fn t0() -> Timestamp {
        "2026-11-26T12:00:00Z".parse().unwrap()
    }

    fn user() -> AuthUser {
        AuthUser {
            id: "01JDX2Q7B5W3Y1Z9X7V5T3R1P0".into(),
            username: "alex".into(),
            email: "alex@example.com".into(),
            display_name: "Alex Example".into(),
            role: Role::Admin,
            session_id: "01JDX3A1C3E5G7J9K1M3N5P7Q9".into(),
            auth_time: 1_795_600_000,
        }
    }

    fn subject() -> Subject {
        Subject::new(&user(), "admin", Language::German)
    }

    struct Hub {
        conn: Connection,
        secrets: SecretKey,
        keys: KeyStore,
        identities: Identities,
    }

    impl Hub {
        fn new() -> Self {
            let mut conn = Connection::open_in_memory().unwrap();
            migrations::run(&mut conn).unwrap();
            let mut hub = Self {
                conn,
                secrets: SecretKey::random().unwrap(),
                keys: KeyStore::default(),
                identities: Identities::new(&"https://app.example.com/".parse().unwrap()),
            };
            hub.rotate(t0());
            hub
        }

        fn rotate(&mut self, now: Timestamp) {
            let (_, keys) = signing::rotate(&mut self.conn, &self.secrets, now).unwrap();
            self.keys.replace(keys);
        }

        fn issue(&self, aud: &str, subject: &Subject, now: Timestamp) -> Arc<str> {
            self.identities
                .issue(&self.keys, aud, subject, now)
                .unwrap()
        }

        fn verify(&self, token: &str, now: Timestamp) -> Identity {
            verify::<Identity>(
                token,
                &self.keys.jwks(now),
                &Expected {
                    issuer: "https://app.example.com",
                    audience: "mail",
                    now: now.as_second(),
                },
            )
            .unwrap()
        }
    }

    #[test]
    fn issues_tokens_apps_can_verify() {
        let hub = Hub::new();
        let token = hub.issue("mail", &subject(), t0());
        let now = t0().as_second();
        assert_eq!(
            hub.verify(&token, t0()),
            Identity {
                iss: "https://app.example.com".into(),
                aud: "mail".into(),
                sub: "01JDX2Q7B5W3Y1Z9X7V5T3R1P0".into(),
                username: "alex".into(),
                email: "alex@example.com".into(),
                name: "Alex Example".into(),
                role: "admin".into(),
                app_role: "admin".into(),
                locale: "de".into(),
                sid: "01JDX3A1C3E5G7J9K1M3N5P7Q9".into(),
                auth_time: 1_795_600_000,
                iat: now,
                exp: now + 60,
            }
        );
        // Valid for the whole lifetime, not after it.
        let end = t0() + SignedDuration::from_secs(LIFETIME);
        hub.verify(&token, end);
        let jwks = hub.keys.jwks(end);
        let late = Expected {
            issuer: "https://app.example.com",
            audience: "mail",
            now: end.as_second() + roedy_hub_contract::LEEWAY,
        };
        assert!(verify::<Identity>(&token, &jwks, &late).is_err());

        let english = Subject {
            locale: Language::System,
            ..subject()
        };
        let token = hub.issue("mail", &english, t0());
        assert_eq!(hub.verify(&token, t0()).locale, "en");
    }

    #[test]
    fn reuses_a_token_for_30_seconds() {
        let hub = Hub::new();
        let first = hub.issue("mail", &subject(), t0());
        let at = |secs: i64| t0() + SignedDuration::from_secs(secs);
        assert_eq!(hub.issue("mail", &subject(), at(1)), first);
        assert_eq!(hub.issue("mail", &subject(), at(29)), first);

        let renewed = hub.issue("mail", &subject(), at(30));
        assert_ne!(renewed, first);
        assert_eq!(hub.verify(&renewed, at(30)).iat, at(30).as_second());
        assert_eq!(hub.issue("mail", &subject(), at(59)), renewed);

        // A clock that went back gets a new token, not one from the future.
        let back = hub.issue("mail", &subject(), at(10));
        assert_eq!(hub.verify(&back, at(10)).iat, at(10).as_second());
    }

    #[test]
    fn a_new_token_for_other_claims_apps_and_sessions() {
        let hub = Hub::new();
        let first = hub.issue("mail", &subject(), t0());
        type Change = fn(&mut Subject);
        let changes: [(&str, Change); 8] = [
            ("username", |s| s.username = "alexa".into()),
            ("email", |s| s.email = "alexa@example.com".into()),
            ("name", |s| s.name = "Alexa Example".into()),
            ("role", |s| s.role = Role::User),
            ("app_role", |s| s.app_role = "user".into()),
            ("locale", |s| s.locale = Language::English),
            ("fresh sign-in", |s| s.auth_time += 100),
            ("other session", |s| {
                s.sid = "01JDX3A1C3E5G7J9K1M3N5P7R0".into()
            }),
        ];
        for (name, change) in changes {
            let mut changed = subject();
            change(&mut changed);
            let token = hub.issue("mail", &changed, t0());
            assert_ne!(token, first, "{name}");
        }
        let contacts = hub.issue("contacts", &subject(), t0());
        assert_ne!(contacts, first);
        // Each app keeps its own token.
        assert_eq!(
            hub.issue("mail", &subject(), t0()),
            hub.issue("mail", &subject(), t0())
        );
        assert_eq!(hub.issue("contacts", &subject(), t0()), contacts);
    }

    #[test]
    fn switches_to_a_new_key_at_once() {
        let mut hub = Hub::new();
        let rotated = t0() + ROTATE_AFTER;
        hub.rotate(rotated);
        let switch = rotated + PUBLISH_BEFORE;
        let before = hub.issue("mail", &subject(), switch - SignedDuration::from_secs(1));
        let after = hub.issue("mail", &subject(), switch);
        assert_ne!(before, after);
        let kid = |token: &str| {
            use base64::Engine as _;
            let header = token.split('.').next().unwrap();
            let header = base64::engine::general_purpose::URL_SAFE_NO_PAD
                .decode(header)
                .unwrap();
            serde_json::from_slice::<serde_json::Value>(&header).unwrap()["kid"]
                .as_str()
                .unwrap()
                .to_owned()
        };
        assert_eq!(kid(&after), hub.keys.signer(switch).unwrap().kid());
        assert_ne!(kid(&before), kid(&after));
        // Both verify against the JWKS of that time.
        hub.verify(&before, switch);
        hub.verify(&after, switch);
    }

    #[test]
    fn forgets_revoked_sessions_and_prunes_stale_tokens() {
        let hub = Hub::new();
        hub.issue("mail", &subject(), t0());
        hub.issue("contacts", &subject(), t0());
        let other = Subject {
            sid: "01JDX3A1C3E5G7J9K1M3N5P7R0".into(),
            ..subject()
        };
        hub.issue("mail", &other, t0());
        assert_eq!(hub.identities.cached(), 3);
        hub.identities.forget_session(&subject().sid);
        assert_eq!(hub.identities.cached(), 1);
        // Signed anew (the same token: Ed25519 is deterministic).
        hub.issue("mail", &subject(), t0());
        assert_eq!(hub.identities.cached(), 2);

        // Many sessions: once there are PRUNE_FROM, stale tokens go.
        for i in 0..PRUNE_FROM {
            let s = Subject {
                sid: format!("S{i}"),
                ..subject()
            };
            hub.issue("mail", &s, t0());
        }
        assert!(hub.identities.cached() >= PRUNE_FROM);
        assert_eq!(hub.identities.pruned_at(), Some(t0().as_second()));
        let pruned = t0() + SignedDuration::from_secs(REUSE);
        hub.issue("mail", &subject(), pruned);
        assert_eq!(hub.identities.cached(), 1);
        assert_eq!(hub.identities.pruned_at(), Some(pruned.as_second()));

        // Live tokens are not scanned again on every miss: the next pruning
        // waits REUSE.
        for i in 0..PRUNE_FROM {
            let s = Subject {
                sid: format!("L{i}"),
                ..subject()
            };
            hub.issue("mail", &s, pruned);
        }
        let soon = pruned + SignedDuration::from_secs(REUSE - 1);
        hub.issue("contacts", &subject(), soon);
        assert_eq!(hub.identities.pruned_at(), Some(pruned.as_second()));
        assert_eq!(hub.identities.cached(), PRUNE_FROM + 2);
        let due = pruned + SignedDuration::from_secs(REUSE);
        hub.issue("calendar", &subject(), due);
        assert_eq!(hub.identities.pruned_at(), Some(due.as_second()));
        assert_eq!(
            hub.identities.cached(),
            2,
            "only the token of `soon` and the new one"
        );
    }

    #[test]
    fn fails_without_a_signing_key() {
        let identities = Identities::new(&"https://app.example.com".parse().unwrap());
        assert_eq!(identities.issuer(), "https://app.example.com");
        let error = identities
            .issue(&KeyStore::default(), "mail", &subject(), t0())
            .unwrap_err();
        assert_eq!(error.to_string(), "no signing key");
    }
}
