//! Delivery of the events to the apps (docs/app-contract.md › Events).
//!
//! The changes write their events into the outbox (`db::repo::events`) in
//! their own transaction. [`start`] runs the delivery: one worker per paired
//! app, which sends the app's events one at a time in `seq` order as a
//! signed JWT ([`sign`], `typ` `roedy-event+jwt`, valid for 300 s) to
//! `POST <internal address>/<slug>/_roedy/v1/events` and goes on only after
//! a `204`. Anything else, or no answer within 10 s, is a failure: the same
//! event follows after 10 s, 30 s, 1 min, 5 min, 15 min and then every hour
//! ([`Schedule`]). Once the next attempt would fall more than 7 days after
//! the first failure, the hub gives up and waits until the app is back
//! (`PUT manifest`, a catch-up, pairing again or a successful health check);
//! the app finds what it missed with `GET /internal/v1/events`.
//!
//! **Waking:** SQLite's update hook on the connection tells the delivery
//! about every new event and every change to `apps`, before the change
//! commits; since every statement goes through the one database thread, the
//! worker's next read sees the committed state. So a fresh event goes out
//! at once, without polling. Events written by another process
//! (`roedy-hub reset-admin`) are found by the check every minute.
//!
//! Removing an app deletes its events (`ON DELETE CASCADE`) and stops its
//! worker. Switching an app off stops its worker too, but its events stay
//! in the outbox and are written on; switching it on starts the worker
//! again, which delivers them in order at once (`api::apps`: the backoff of
//! a failing delivery is reset).

use std::{
    collections::{HashMap, hash_map::Entry},
    time::Duration,
};

use anyhow::Context as _;
use jiff::Timestamp;
use roedy_hub_contract::Event;
use rusqlite::hooks::Action;
use tokio::{
    sync::watch,
    task::{AbortHandle, JoinHandle, JoinSet},
};

use crate::{
    app::AppState,
    apps,
    db::{
        self,
        repo::events::{self as repo, Next, Stored},
    },
};

/// Lifetime of a signed event (`exp − iat`), the most the contract allows.
pub const LIFETIME: i64 = 300;

/// How long events are kept for catching up (contract § 10.3).
pub const RETENTION: Duration = Duration::from_secs(30 * 24 * 60 * 60);

/// Path of the delivery below the app's prefix (contract § 10.1).
pub const DELIVERY_PATH: &str = "/_roedy/v1/events";

/// Timing of the delivery; the defaults are the contract's (§ 10.1).
#[derive(Debug, Clone)]
pub struct Schedule {
    /// Wait after the 1st, 2nd … failure; the last one repeats.
    pub delays: Vec<Duration>,
    /// No attempt later than this after the first failure of a series.
    pub give_up: Duration,
    /// Until the app accepts the connection.
    pub connect_timeout: Duration,
    /// For the whole delivery: no `204` by then is a failure.
    pub timeout: Duration,
    /// Without a wake-up, look again after this long (events from another
    /// process).
    pub recheck: Duration,
}

impl Default for Schedule {
    fn default() -> Self {
        let minute = Duration::from_secs(60);
        Self {
            delays: vec![
                Duration::from_secs(10),
                Duration::from_secs(30),
                minute,
                5 * minute,
                15 * minute,
                60 * minute,
            ],
            give_up: Duration::from_secs(7 * 24 * 60 * 60),
            connect_timeout: Duration::from_secs(5),
            timeout: Duration::from_secs(10),
            recheck: minute,
        }
    }
}

impl Schedule {
    /// The wait after `attempts` failures (at least one).
    pub fn delay(&self, attempts: u32) -> Duration {
        let index = usize::try_from(attempts.max(1) - 1).unwrap_or(usize::MAX);
        self.delays
            .get(index)
            .or(self.delays.last())
            .copied()
            .unwrap_or(Duration::from_secs(60 * 60))
    }
}

/// Wake-up and timing of the delivery; part of [`AppState`].
pub struct Events {
    changed: watch::Sender<()>,
    schedule: Schedule,
    client: reqwest::Client,
}

impl Events {
    /// With the contract's timing.
    pub fn new() -> anyhow::Result<Self> {
        Self::with_schedule(Schedule::default())
    }

    /// With other timing (tests).
    pub fn with_schedule(schedule: Schedule) -> anyhow::Result<Self> {
        Ok(Self {
            changed: watch::Sender::new(()),
            client: apps::client_with(schedule.connect_timeout, schedule.timeout)?,
            schedule,
        })
    }

    pub fn schedule(&self) -> &Schedule {
        &self.schedule
    }

    /// Something changed (a new event, an app, a resumed delivery): every
    /// worker looks again.
    pub fn wake(&self) {
        self.changed.send_replace(());
    }

    fn subscribe(&self) -> watch::Receiver<()> {
        self.changed.subscribe()
    }
}

/// The event signed for app `slug` at `now` (Unix seconds): `jti`, `seq`,
/// `type` and `data` as stored, `iat` now, valid for [`LIFETIME`]. Every
/// attempt and every catch-up signs anew.
pub fn sign(state: &AppState, slug: &str, event: &Stored, now: i64) -> anyhow::Result<String> {
    let signer = state
        .keys
        .signer(Timestamp::now())
        .context("no signing key")?;
    Ok(signer.sign(&Event {
        iss: state.identities.issuer().to_owned(),
        aud: slug.to_owned(),
        jti: event.id.clone(),
        seq: event.seq,
        kind: event.kind.clone(),
        data: event.data.clone(),
        iat: now,
        exp: now + LIFETIME,
    })?)
}

/// The running delivery; [`Self::shutdown`] stops it.
pub struct Delivery {
    task: JoinHandle<()>,
}

impl Delivery {
    /// Stops every worker; a delivery under way is dropped and repeated
    /// after the next start (at least once).
    pub async fn shutdown(self) {
        self.task.abort();
        let _ = self.task.await;
    }
}

/// Installs the wake-up on the database connection and starts the delivery.
pub async fn start(state: &AppState) -> anyhow::Result<Delivery> {
    let events = state.events.clone();
    db::call(&state.db, move |conn| {
        conn.update_hook(Some(
            move |action: Action, _db: &str, table: &str, _row: i64| {
                let new_event = table == "events" && matches!(action, Action::SQLITE_INSERT);
                if new_event || table == "apps" {
                    events.wake();
                }
            },
        ))?;
        Ok(())
    })
    .await
    .context("event delivery not started")?;
    Ok(Delivery {
        task: tokio::spawn(supervise(state.clone())),
    })
}

/// Keeps one worker per paired app that is switched on.
async fn supervise(state: AppState) {
    let mut changed = state.events.subscribe();
    let mut workers: HashMap<String, AbortHandle> = HashMap::new();
    // Dropped with this task: then every worker ends too.
    let mut tasks = JoinSet::new();
    loop {
        changed.borrow_and_update();
        match db::call(&state.db, |conn| Ok(repo::delivered_apps(conn)?)).await {
            Ok(apps) => {
                workers.retain(|id, worker| {
                    let keep = apps.contains(id) && !worker.is_finished();
                    if !keep {
                        worker.abort();
                    }
                    keep
                });
                for id in apps {
                    if let Entry::Vacant(entry) = workers.entry(id) {
                        let worker = tasks.spawn(work(state.clone(), entry.key().clone()));
                        entry.insert(worker);
                    }
                }
            }
            Err(error) => tracing::warn!("event delivery: apps not read: {error:#}"),
        }
        while tasks.try_join_next().is_some() {}
        tokio::select! {
            _ = changed.changed() => {}
            () = tokio::time::sleep(state.events.schedule.recheck) => {}
        }
    }
}

/// Delivers the events of one app until it is gone.
async fn work(state: AppState, app_id: String) {
    let mut changed = state.events.subscribe();
    let recheck = state.events.schedule.recheck;
    loop {
        changed.borrow_and_update();
        let id = app_id.clone();
        let wait = match db::call(&state.db, move |conn| Ok(repo::next(conn, &id)?)).await {
            Ok(Next::Gone) => return,
            Ok(Next::Idle | Next::Suspended) => recheck,
            Ok(Next::Wait(wait)) => wait,
            Ok(Next::Deliver {
                slug,
                internal_url,
                event,
                attempts,
            }) => match attempt(&state, &app_id, &slug, &internal_url, event, attempts).await {
                Ok(()) => continue,
                Err(error) => {
                    tracing::warn!(slug, "event delivery not recorded: {error:#}");
                    recheck
                }
            },
            Err(error) => {
                tracing::warn!("event delivery: state not read: {error:#}");
                recheck
            }
        };
        tokio::select! {
            _ = changed.changed() => {}
            () = tokio::time::sleep(wait) => {}
        }
    }
}

/// One attempt at `event`; records the outcome. An error only if the
/// outcome could not be recorded.
async fn attempt(
    state: &AppState,
    app_id: &str,
    slug: &str,
    internal_url: &str,
    event: Stored,
    attempts: u32,
) -> anyhow::Result<()> {
    let url = format!(
        "{}/{slug}{DELIVERY_PATH}",
        internal_url.trim_end_matches('/')
    );
    let seq = event.seq;
    let outcome = match sign(state, slug, &event, Timestamp::now().as_second()) {
        Ok(token) => post(&state.events.client, &url, token).await,
        Err(error) => Err(format!("not signed: {error:#}")),
    };
    let id = app_id.to_owned();
    match outcome {
        Ok(()) => {
            db::call(&state.db, move |conn| Ok(repo::delivered(conn, &id, seq)?)).await?;
            if attempts > 0 {
                tracing::info!(slug, seq, attempts, "event delivered after failures");
            } else {
                tracing::debug!(slug, seq, kind = event.kind, "event delivered");
            }
        }
        Err(error) => {
            let schedule = state.events.schedule.clone();
            let reason = error.clone();
            let failure = db::call(&state.db, move |conn| {
                Ok(repo::failed(
                    conn,
                    &id,
                    seq,
                    &reason,
                    |n| schedule.delay(n),
                    schedule.give_up,
                )?)
            })
            .await?;
            match failure {
                Some(failure) if failure.suspended => tracing::warn!(
                    slug,
                    seq,
                    attempts = failure.attempts,
                    error,
                    "event delivery given up until the app is back"
                ),
                Some(failure) if failure.attempts == 1 => tracing::warn!(
                    slug,
                    seq,
                    error,
                    retry_in = ?state.events.schedule.delay(1),
                    "event delivery failed, retrying"
                ),
                Some(failure) => tracing::debug!(
                    slug,
                    seq,
                    attempts = failure.attempts,
                    error,
                    "event delivery failed again"
                ),
                None => {}
            }
        }
    }
    Ok(())
}

/// Sends one signed event; `Ok` only for `204`.
async fn post(client: &reqwest::Client, url: &str, token: String) -> Result<(), String> {
    let response = client
        .post(url)
        .header(reqwest::header::CONTENT_TYPE, "application/jwt")
        .body(token)
        .send()
        .await;
    match response {
        Ok(response) if response.status() == reqwest::StatusCode::NO_CONTENT => Ok(()),
        Ok(response) => Err(format!("status {}", response.status().as_u16())),
        Err(error) if error.is_timeout() => Err("timeout".to_owned()),
        Err(error) if error.is_connect() => Err("not reachable".to_owned()),
        Err(error) => Err(format!("request failed: {error}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_follows_the_contract() {
        let schedule = Schedule::default();
        let secs: Vec<u64> = (1..=8).map(|n| schedule.delay(n).as_secs()).collect();
        assert_eq!(secs, [10, 30, 60, 300, 900, 3600, 3600, 3600]);
        assert_eq!(schedule.delay(0).as_secs(), 10);
        assert_eq!(schedule.give_up.as_secs(), 7 * 24 * 3600);
        assert_eq!(schedule.timeout.as_secs(), 10);
    }
}
