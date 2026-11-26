//! The hub's signing keys (docs/app-contract.md › Identity › Keys): Ed25519
//! keys in `signing_keys`, the private part encrypted with the instance key.
//!
//! [`rotate`] runs at start and once a day (`jobs::key_rotation`):
//!
//! - without a usable key it creates one that signs at once (first start;
//!   no app has a JWKS yet);
//! - once the newest key has signed for [`ROTATE_AFTER`], it creates the next
//!   one, published at once and signing after [`PUBLISH_BEFORE`]; the
//!   previous key signs until then (`retired_at`) and stays in the JWKS for
//!   [`KEEP_AFTER`] more – every token it signed has expired by then;
//! - keys out of the JWKS are deleted.
//!
//! A key that cannot be decrypted (the instance key changed) can no longer
//! sign: it is retired, and a new key signs at once. Apps that meet its
//! unknown `kid` refresh their JWKS (contract § 4.3).
//!
//! [`KeyStore`] holds the decrypted keys in memory and picks the signing key
//! and the published ones by time, so that a new key takes over on time
//! between two runs.

use std::sync::{Arc, RwLock};

use jiff::{SignedDuration, Timestamp};
use roedy_hub_contract::{Jwks, SigningKey};
use rusqlite::Connection;

use crate::{crypto::SecretKey, db, db::repo::signing_keys, random};

/// How long a key signs before the next one takes over.
pub const ROTATE_AFTER: SignedDuration = SignedDuration::from_hours(90 * 24);
/// A new key is published this long before it signs.
pub const PUBLISH_BEFORE: SignedDuration = SignedDuration::from_mins(10);
/// A retired key stays published this long after its last signature.
pub const KEEP_AFTER: SignedDuration = SignedDuration::from_mins(10);

/// Purpose of the encryption (`crypto`); the row is the `kid`.
const PURPOSE: &str = "signing_keys.private";

/// A decrypted key with its times.
#[derive(Debug, Clone)]
pub struct Key {
    pub key: Arc<SigningKey>,
    pub active_from: Timestamp,
    pub retired_at: Option<Timestamp>,
}

impl Key {
    /// Whether the key is in the JWKS at `now`.
    fn published(&self, now: Timestamp) -> bool {
        self.retired_at
            .is_none_or(|retired| now < retired + KEEP_AFTER)
    }
}

/// The keys in memory, oldest `active_from` first. Replaced as a whole
/// after each [`rotate`].
#[derive(Debug, Default)]
pub struct KeyStore {
    keys: RwLock<Arc<[Key]>>,
}

impl KeyStore {
    pub fn replace(&self, keys: Vec<Key>) {
        *self.keys.write().unwrap_or_else(|e| e.into_inner()) = keys.into();
    }

    fn keys(&self) -> Arc<[Key]> {
        self.keys.read().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// The key that signs at `now`: the newest one active by then. If the
    /// clock went back before every `active_from`, the oldest key – signing
    /// with a published key is better than not at all.
    pub fn signer(&self, now: Timestamp) -> Option<Arc<SigningKey>> {
        let keys = self.keys();
        keys.iter()
            .rev()
            .find(|k| k.active_from <= now)
            .or_else(|| keys.first())
            .map(|k| k.key.clone())
    }

    /// The public keys at `now` (contract § 4.4).
    pub fn jwks(&self, now: Timestamp) -> Jwks {
        Jwks {
            keys: self
                .keys()
                .iter()
                .filter(|k| k.published(now))
                .map(|k| k.key.jwk())
                .collect(),
        }
    }
}

/// What [`rotate`] did, for the log and the tests.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Rotation {
    /// Nothing to do.
    None,
    /// A new key that signs at once: first start or no usable key.
    Created(String),
    /// The next key, signing from `PUBLISH_BEFORE` on.
    Scheduled(String),
}

/// One rotation at `now` in a transaction; returns what it did and the
/// keys for [`KeyStore::replace`].
pub fn rotate(
    conn: &mut Connection,
    secrets: &SecretKey,
    now: Timestamp,
) -> anyhow::Result<(Rotation, Vec<Key>)> {
    let tx = conn.transaction()?;
    let mut keys = Vec::new();
    for row in signing_keys::all(&tx)? {
        let retired_at = row.retired_at;
        if retired_at.is_some_and(|retired| retired + KEEP_AFTER <= now) {
            signing_keys::delete(&tx, &row.kid)?;
            tracing::info!(kid = row.kid, "retired signing key deleted");
            continue;
        }
        match open(secrets, &row) {
            Ok(key) => keys.push(Key {
                key: Arc::new(key),
                active_from: row.active_from,
                retired_at,
            }),
            Err(error) => {
                // Out of the JWKS after KEEP_AFTER; deleted with the next run after that.
                if signing_keys::retire(&tx, &row.kid, now)? {
                    tracing::warn!(kid = row.kid, "signing key retired: {error:#}");
                }
            }
        }
    }

    // The newest key that may still sign.
    let newest = keys
        .iter()
        .filter(|k| k.retired_at.is_none_or(|retired| now < retired))
        .max_by_key(|k| k.active_from);
    let rotation = match newest {
        None => Rotation::Created(create(&tx, secrets, now, &mut keys)?),
        Some(newest) if now < newest.active_from + ROTATE_AFTER => Rotation::None,
        Some(_) => {
            let from = now + PUBLISH_BEFORE;
            for key in &mut keys {
                if key.retired_at.is_none() && signing_keys::retire(&tx, key.key.kid(), from)? {
                    key.retired_at = Some(from);
                }
            }
            Rotation::Scheduled(create(&tx, secrets, from, &mut keys)?)
        }
    };
    tx.commit()?;
    match &rotation {
        Rotation::None => {}
        Rotation::Created(kid) => tracing::info!(kid, "signing key created"),
        Rotation::Scheduled(kid) => {
            tracing::info!(kid, active_from = %(now + PUBLISH_BEFORE), "next signing key published");
        }
    }
    keys.sort_by_key(|k| k.active_from);
    Ok((rotation, keys))
}

/// Runs [`rotate`] on the database and puts the keys into `store`.
pub async fn rotate_now(
    database: &db::Db,
    secrets: Arc<SecretKey>,
    store: &KeyStore,
) -> anyhow::Result<Rotation> {
    let (rotation, keys) = db::call(database, move |conn| {
        rotate(conn, &secrets, Timestamp::now())
    })
    .await?;
    store.replace(keys);
    Ok(rotation)
}

fn open(secrets: &SecretKey, row: &signing_keys::Row) -> anyhow::Result<SigningKey> {
    let seed: [u8; 32] = secrets
        .open(PURPOSE, &row.kid, &row.private_enc)?
        .try_into()
        .map_err(|_| anyhow::anyhow!("signing key {}: wrong length", row.kid))?;
    let key = SigningKey::from_seed(row.kid.clone(), &seed)?;
    anyhow::ensure!(
        key.public_key().as_slice() == row.public,
        "signing key {}: public key does not match",
        row.kid
    );
    Ok(key)
}

/// A new key from the random source, signing from `active_from`.
fn create(
    conn: &Connection,
    secrets: &SecretKey,
    active_from: Timestamp,
    keys: &mut Vec<Key>,
) -> anyhow::Result<String> {
    let kid = db::new_id()?;
    let seed = random::bytes::<32>()?;
    let key = SigningKey::from_seed(kid.clone(), &seed)?;
    let sealed = secrets.seal(PURPOSE, &kid, &seed)?;
    signing_keys::insert(conn, &kid, &sealed, &key.public_key(), active_from)?;
    keys.push(Key {
        key: Arc::new(key),
        active_from,
        retired_at: None,
    });
    Ok(kid)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::migrations;

    fn t0() -> Timestamp {
        "2026-11-26T12:00:00Z".parse().unwrap()
    }

    fn conn() -> Connection {
        let mut conn = Connection::open_in_memory().unwrap();
        migrations::run(&mut conn).unwrap();
        conn
    }

    fn kids(jwks: &Jwks) -> Vec<String> {
        jwks.keys.iter().map(|k| k.kid.clone().unwrap()).collect()
    }

    fn rows(conn: &Connection) -> usize {
        signing_keys::all(conn).unwrap().len()
    }

    /// The whole life of two keys, step by step.
    #[test]
    fn rotates_every_90_days_with_overlap() {
        let mut conn = conn();
        let secrets = SecretKey::random().unwrap();
        let store = KeyStore::default();
        assert!(store.signer(t0()).is_none());
        assert!(store.jwks(t0()).keys.is_empty());

        // Rotation at `now` and the number of stored keys after it.
        let mut at = |now: Timestamp, store: &KeyStore| {
            let (rotation, keys) = rotate(&mut conn, &secrets, now).unwrap();
            store.replace(keys);
            (rotation, rows(&conn))
        };

        // First start: a key that signs at once.
        let (Rotation::Created(first), 1) = at(t0(), &store) else {
            panic!("no first key");
        };
        assert_eq!(store.signer(t0()).unwrap().kid(), first);
        assert_eq!(kids(&store.jwks(t0())), [first.as_str()]);

        // Daily runs change nothing for 90 days.
        let day = SignedDuration::from_hours(24);
        for days in [1, 30, 89] {
            assert_eq!(
                at(t0() + day * days, &store),
                (Rotation::None, 1),
                "day {days}"
            );
        }
        let almost = t0() + ROTATE_AFTER - SignedDuration::from_secs(1);
        assert_eq!(at(almost, &store), (Rotation::None, 1));

        // After 90 days: the next key is published, the first still signs.
        let rotated = t0() + ROTATE_AFTER;
        let (Rotation::Scheduled(second), 2) = at(rotated, &store) else {
            panic!("no rotation");
        };
        let switch = rotated + PUBLISH_BEFORE;
        assert_eq!(kids(&store.jwks(rotated)), [first.clone(), second.clone()]);
        assert_eq!(store.signer(rotated).unwrap().kid(), first);
        let just_before = switch - SignedDuration::from_millis(1);
        assert_eq!(store.signer(just_before).unwrap().kid(), first);
        // A second run in between changes nothing.
        assert_eq!(
            at(rotated + SignedDuration::from_mins(5), &store),
            (Rotation::None, 2)
        );

        // From the switch on the new key signs; the old one stays published
        // for KEEP_AFTER.
        assert_eq!(store.signer(switch).unwrap().kid(), second);
        let gone = switch + KEEP_AFTER;
        let still = gone - SignedDuration::from_millis(1);
        assert_eq!(kids(&store.jwks(still)), [first.clone(), second.clone()]);
        assert_eq!(kids(&store.jwks(gone)), [second.as_str()]);

        // The next run deletes the old key.
        assert_eq!(at(rotated + day, &store), (Rotation::None, 1));
        assert_eq!(kids(&store.jwks(rotated + day)), [second.as_str()]);
        assert_eq!(store.signer(rotated + day).unwrap().kid(), second);

        // 90 days after the second key's start, the third follows.
        assert_eq!(
            at(switch + ROTATE_AFTER - SignedDuration::from_secs(1), &store),
            (Rotation::None, 1)
        );
        assert!(matches!(
            at(switch + ROTATE_AFTER, &store),
            (Rotation::Scheduled(_), 2)
        ));
    }

    #[test]
    fn keys_survive_a_restart() {
        let mut conn = conn();
        let secrets = SecretKey::random().unwrap();
        let (_, before) = rotate(&mut conn, &secrets, t0()).unwrap();
        let (rotation, after) = rotate(&mut conn, &secrets, t0()).unwrap();
        assert_eq!(rotation, Rotation::None);
        assert_eq!(after.len(), 1);
        assert_eq!(after[0].key.kid(), before[0].key.kid());
        assert_eq!(after[0].key.public_key(), before[0].key.public_key());
    }

    #[test]
    fn the_private_key_is_stored_encrypted() {
        let mut conn = conn();
        let secrets = SecretKey::random().unwrap();
        let (_, keys) = rotate(&mut conn, &secrets, t0()).unwrap();
        let row = signing_keys::all(&conn).unwrap().remove(0);
        assert_eq!(row.public, keys[0].key.public_key());
        // Seed (32 bytes) sealed: version, nonce, ciphertext and tag.
        assert_eq!(row.private_enc.len(), 1 + 24 + 32 + 16);
        // Bound to its row: moved to another kid, it does not open.
        let moved = signing_keys::Row {
            kid: "other".into(),
            ..row.clone()
        };
        assert!(open(&secrets, &moved).is_err());
        assert_eq!(
            open(&secrets, &row).unwrap().public_key(),
            keys[0].key.public_key()
        );
    }

    #[test]
    fn replaces_keys_it_cannot_decrypt() {
        let mut conn = conn();
        let (_, old) = rotate(&mut conn, &SecretKey::random().unwrap(), t0()).unwrap();
        let old = old[0].key.kid().to_owned();

        // Another instance key: the old key cannot sign any more.
        let secrets = SecretKey::random().unwrap();
        let later = t0() + SignedDuration::from_hours(1);
        let store = KeyStore::default();
        let (rotation, keys) = rotate(&mut conn, &secrets, later).unwrap();
        store.replace(keys);
        let Rotation::Created(new) = rotation else {
            panic!("no new key: {rotation:?}");
        };
        assert_eq!(store.signer(later).unwrap().kid(), new);
        assert_eq!(kids(&store.jwks(later)), [new.as_str()]);
        let rows = signing_keys::all(&conn).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(
            rows.iter().find(|r| r.kid == old).unwrap().retired_at,
            Some(later)
        );

        // Deleted once out of the JWKS's time.
        let (rotation, _) = rotate(&mut conn, &secrets, later + KEEP_AFTER).unwrap();
        assert_eq!(rotation, Rotation::None);
        assert_eq!(signing_keys::all(&conn).unwrap().len(), 1);
    }

    #[test]
    fn signs_with_the_oldest_key_when_the_clock_went_back() {
        let mut conn = conn();
        let secrets = SecretKey::random().unwrap();
        let store = KeyStore::default();
        let (_, keys) = rotate(&mut conn, &secrets, t0()).unwrap();
        store.replace(keys);
        let before = t0() - SignedDuration::from_hours(1);
        assert!(store.signer(before).is_some());
    }

    #[tokio::test]
    async fn rotate_now_fills_the_store() {
        let (state, _tmp) = crate::app::tests::state().await;
        let store = KeyStore::default();
        assert_eq!(
            rotate_now(&state.db, state.secrets.clone(), &store)
                .await
                .unwrap(),
            Rotation::None,
            "the test state already has a key"
        );
        let now = Timestamp::now();
        assert_eq!(store.jwks(now), state.keys.jwks(now));
        assert_eq!(store.jwks(now).keys.len(), 1);
    }
}
