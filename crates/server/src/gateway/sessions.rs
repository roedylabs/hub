//! Sessions as the gateway sees them (docs/plan.md › Gateway):
//!
//! - [`SessionCache`]: the signed-in user of a session cookie with their
//!   language and roles in the apps, kept for [`TTL`], so that the requests
//!   for an app's assets do not each cost a database round trip.
//! - [`Streams`]: the open responses and WebSockets per session. Ending a
//!   session ([`Streams::end_session`], [`Streams::end_user`]) closes them;
//!   the job `gateway_streams` ends those of sessions that ended without the
//!   hub's API (expiry, `roedy-hub reset-admin`, the limit of sessions per
//!   user) within seconds (contract § 3.5: within 5 s).
//!
//! `AppState::end_session` and `AppState::end_user_sessions` clear both,
//! together with the session's identity tokens.

use std::{
    collections::HashMap,
    future::Future,
    pin::Pin,
    sync::{
        Arc, Mutex, MutexGuard, PoisonError,
        atomic::{AtomicU64, Ordering},
    },
    task::{Context, Poll},
    time::{Duration, Instant},
};

use tokio::sync::watch;

use crate::{auth::session::AuthUser, settings::Language};

/// How long a checked session counts as valid without asking the database.
pub const TTL: Duration = Duration::from_secs(10);
/// Past this many entries, stale ones are dropped on an insert.
const PRUNE_FROM: usize = 4096;

/// What the gateway needs about the user of a session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Signed {
    pub user: AuthUser,
    /// Language for the identity (`locale`).
    pub locale: Language,
    /// Role per app ID; no entry, no access.
    pub roles: HashMap<String, String>,
}

struct Entry {
    signed: Arc<Signed>,
    at: Instant,
}

/// Checked sessions by the SHA-256 of their token.
#[derive(Default)]
pub struct SessionCache {
    inner: Mutex<CacheInner>,
}

#[derive(Default)]
struct CacheInner {
    entries: HashMap<[u8; 32], Entry>,
    /// Counts the revocations: a check that started before one does not
    /// store its result (it may have read the session just before it ended).
    generation: u64,
}

impl SessionCache {
    fn lock(&self) -> MutexGuard<'_, CacheInner> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The session of `hash`, while checked less than [`TTL`] ago.
    pub fn get(&self, hash: &[u8; 32]) -> Option<Arc<Signed>> {
        let inner = self.lock();
        let entry = inner.entries.get(hash)?;
        (entry.at.elapsed() < TTL).then(|| entry.signed.clone())
    }

    /// Take before reading a session from the database, pass to
    /// [`Self::insert`].
    pub fn generation(&self) -> u64 {
        self.lock().generation
    }

    /// Keeps a session read at `generation`, unless a session ended since.
    pub fn insert(&self, hash: [u8; 32], signed: Arc<Signed>, generation: u64) {
        let mut inner = self.lock();
        if inner.generation != generation {
            return;
        }
        if inner.entries.len() >= PRUNE_FROM {
            inner.entries.retain(|_, entry| entry.at.elapsed() < TTL);
        }
        inner.entries.insert(
            hash,
            Entry {
                signed,
                at: Instant::now(),
            },
        );
    }

    /// Drops the entries that `ended` selects.
    fn forget(&self, ended: impl Fn(&AuthUser) -> bool) {
        let mut inner = self.lock();
        inner.generation += 1;
        inner.entries.retain(|_, entry| !ended(&entry.signed.user));
    }

    /// A session ended.
    pub fn forget_session(&self, sid: &str) {
        self.forget(|user| user.session_id == sid);
    }

    /// The sessions of a user ended, except `keep`; also after a change of
    /// their role, so that the next request reads it anew.
    pub fn forget_user(&self, user_id: &str, keep: Option<&str>) {
        self.forget(|user| user.id == user_id && Some(user.session_id.as_str()) != keep);
    }

    pub fn len(&self) -> usize {
        self.lock().entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

struct Open {
    /// Tells this entry from a later one of the same session.
    id: u64,
    user_id: String,
    /// Responses and WebSockets of the session that are still open.
    count: usize,
    ended: watch::Sender<bool>,
}

/// Open responses and WebSockets by session ID.
pub struct Streams {
    open: Mutex<HashMap<String, Open>>,
    next_id: AtomicU64,
    /// `true` once the server shuts down: every stream ends.
    shutdown: watch::Sender<bool>,
}

impl Default for Streams {
    fn default() -> Self {
        Self {
            open: Mutex::default(),
            next_id: AtomicU64::new(0),
            shutdown: watch::Sender::new(false),
        }
    }
}

impl Streams {
    fn lock(&self) -> MutexGuard<'_, HashMap<String, Open>> {
        self.open.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Registers a stream of the session `sid` of `user_id` (or of a request
    /// without a session); the stream ends when [`Watch::ended`] does.
    pub fn watch(self: &Arc<Self>, session: Option<(&str, &str)>) -> Watch {
        let shutdown = self.shutdown.subscribe();
        let (session_ended, guard) = match session {
            Some((sid, user_id)) => {
                let mut open = self.lock();
                let entry = open.entry(sid.to_owned()).or_insert_with(|| Open {
                    id: self.next_id.fetch_add(1, Ordering::Relaxed),
                    user_id: user_id.to_owned(),
                    count: 0,
                    ended: watch::Sender::new(false),
                });
                entry.count += 1;
                let receiver = entry.ended.subscribe();
                (
                    Some(receiver),
                    Some(Guard {
                        streams: self.clone(),
                        sid: sid.to_owned(),
                        id: entry.id,
                    }),
                )
            }
            None => (None, None),
        };
        let ended = async move {
            let session = async move {
                match session_ended {
                    // An error means the entry is gone: ended as well.
                    Some(mut receiver) => {
                        let _ = receiver.wait_for(|ended| *ended).await;
                    }
                    None => std::future::pending().await,
                }
            };
            let mut shutdown = shutdown;
            tokio::select! {
                () = session => {}
                _ = shutdown.wait_for(|stop| *stop) => {}
            }
        };
        Watch {
            ended: Box::pin(ended),
            _guard: guard,
        }
    }

    /// Ends the streams of the session `sid`.
    pub fn end_session(&self, sid: &str) {
        if let Some(open) = self.lock().remove(sid) {
            open.ended.send_replace(true);
        }
    }

    /// Ends the streams of every session of `user_id` except `keep`.
    pub fn end_user(&self, user_id: &str, keep: Option<&str>) {
        let mut open = self.lock();
        open.retain(|sid, open| {
            let ends = open.user_id == user_id && Some(sid.as_str()) != keep;
            if ends {
                open.ended.send_replace(true);
            }
            !ends
        });
    }

    /// Ends every stream (shutdown).
    pub fn end_all(&self) {
        self.shutdown.send_replace(true);
        for (_, open) in self.lock().drain() {
            open.ended.send_replace(true);
        }
    }

    /// The sessions with open streams.
    pub fn sessions(&self) -> Vec<String> {
        self.lock().keys().cloned().collect()
    }
}

/// Removes a session's entry once its last stream is gone.
struct Guard {
    streams: Arc<Streams>,
    sid: String,
    id: u64,
}

impl Drop for Guard {
    fn drop(&mut self) {
        let mut open = self.streams.lock();
        if let Some(entry) = open.get_mut(&self.sid).filter(|entry| entry.id == self.id) {
            entry.count = entry.count.saturating_sub(1);
            if entry.count == 0 {
                open.remove(&self.sid);
            }
        }
    }
}

/// A registered stream; dropping it unregisters.
pub struct Watch {
    ended: Pin<Box<dyn Future<Output = ()> + Send>>,
    _guard: Option<Guard>,
}

impl Watch {
    /// Ready once the session ended or the server shuts down.
    pub fn poll_ended(&mut self, cx: &mut Context<'_>) -> Poll<()> {
        self.ended.as_mut().poll(cx)
    }

    /// Waits until the session ended or the server shuts down.
    pub async fn ended(&mut self) {
        self.ended.as_mut().await;
    }
}

impl std::fmt::Debug for Watch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Watch").finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::repo::users::Role;

    fn user(id: &str, sid: &str) -> AuthUser {
        AuthUser {
            id: id.into(),
            username: "max".into(),
            email: "max@example.com".into(),
            display_name: "Max Example".into(),
            role: Role::User,
            session_id: sid.into(),
            auth_time: 1_795_600_000,
        }
    }

    fn signed(id: &str, sid: &str) -> Arc<Signed> {
        Arc::new(Signed {
            user: user(id, sid),
            locale: Language::English,
            roles: HashMap::from([("app".into(), "user".into())]),
        })
    }

    #[test]
    fn cache_forgets_ended_sessions() {
        let cache = SessionCache::default();
        let generation = cache.generation();
        cache.insert([1; 32], signed("u1", "s1"), generation);
        cache.insert([2; 32], signed("u1", "s2"), generation);
        cache.insert([3; 32], signed("u2", "s3"), generation);
        assert_eq!(cache.get(&[1; 32]).unwrap().user.session_id, "s1");
        assert!(cache.get(&[9; 32]).is_none());

        cache.forget_session("s1");
        assert!(cache.get(&[1; 32]).is_none());
        assert!(cache.get(&[2; 32]).is_some());
        cache.forget_user("u1", Some("s2"));
        assert!(cache.get(&[2; 32]).is_some(), "kept");
        cache.forget_user("u1", None);
        assert!(cache.get(&[2; 32]).is_none());
        assert!(cache.get(&[3; 32]).is_some(), "another user");

        // A check that read the session before it ended does not store it.
        cache.insert([1; 32], signed("u1", "s1"), generation);
        assert!(cache.get(&[1; 32]).is_none());
        assert_eq!(cache.len(), 1);
    }

    fn ended(watch: &mut Watch) -> bool {
        let waker = std::task::Waker::noop();
        watch.poll_ended(&mut Context::from_waker(waker)).is_ready()
    }

    #[test]
    fn streams_end_with_their_session() {
        let streams = Arc::new(Streams::default());
        let mut a1 = streams.watch(Some(("s1", "u1")));
        let mut a2 = streams.watch(Some(("s1", "u1")));
        let mut b = streams.watch(Some(("s2", "u1")));
        let mut c = streams.watch(Some(("s3", "u2")));
        let mut anonymous = streams.watch(None);
        let mut open = streams.sessions();
        open.sort();
        assert_eq!(open, ["s1", "s2", "s3"]);
        assert!(!ended(&mut a1) && !ended(&mut anonymous));

        streams.end_session("s1");
        assert!(ended(&mut a1) && ended(&mut a2));
        assert!(!ended(&mut b) && !ended(&mut c));

        streams.end_user("u1", Some("s2"));
        assert!(!ended(&mut b), "kept");
        streams.end_user("u1", None);
        assert!(ended(&mut b));
        assert!(!ended(&mut c));

        streams.end_session("s3");
        assert!(ended(&mut c));
        assert!(streams.sessions().is_empty());

        assert!(!ended(&mut anonymous));
        streams.end_all();
        assert!(ended(&mut anonymous));
    }

    #[test]
    fn closed_streams_unregister() {
        let streams = Arc::new(Streams::default());
        let a = streams.watch(Some(("s1", "u1")));
        let b = streams.watch(Some(("s1", "u1")));
        drop(a);
        assert_eq!(streams.sessions(), ["s1"]);
        drop(b);
        assert!(streams.sessions().is_empty());
        // A stream that outlives the end of its session unregisters
        // without touching a newer entry of the same ID.
        let old = streams.watch(Some(("s1", "u1")));
        streams.end_session("s1");
        let _new = streams.watch(Some(("s1", "u1")));
        drop(old);
        assert_eq!(streams.sessions(), ["s1"]);
    }
}
