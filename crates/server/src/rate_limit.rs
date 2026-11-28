//! In-memory rate limits.
//!
//! Counting happens per rule ([`Policy`]) and key: an IP address (IPv6 per
//! /64, see [`crate::client_ip::ClientIp::network`]), a user or one counter
//! for everyone. Two kinds of rules:
//!
//! - [`Policy::rate`]: `burst` events at once, then one per
//!   `period / burst` (GCRA, "Generic Cell Rate Algorithm") – for requests
//!   like "resend mail".
//! - [`Policy::lockout`]: after `failures` failed attempts within `window`
//!   (from the first one) a lock of `lock`, then the full allowance again –
//!   for checks like "5 failed attempts → 30 s lock" (setup token).
//!
//! [`RateLimiter::acquire`] takes an attempt that always counts (such as
//! "resend mail"). For a check whose result decides whether the attempt counts
//! (a password, a setup token, a TOTP code that was already used gives
//! nothing away), [`RateLimiter::reserve`] holds the attempt as pending: it
//! occupies the allowance – even with many concurrent requests, never more
//! get through than the rule allows – but only counts once the check fails
//! ([`Reservation::count`]); otherwise it ends without a trace. The counters
//! are lost on a restart; with one process per instance that is acceptable.
//! The clock is read under the lock, so that a thread that waited for it
//! cannot store a time that has already passed.
//!
//! Taken over from Roedy Mail (`crates/server/src/rate_limit.rs`). Unlike
//! Mail there is no `refund`: an attempt taken and given back later could
//! erase another one counted in between (GCRA after a pause, or a lockout
//! window that started over).

use std::{
    collections::HashMap,
    net::IpAddr,
    sync::{Mutex, MutexGuard, PoisonError},
    time::{Duration, Instant},
};

/// After this many calls, expired counters are removed.
const SWEEP_EVERY: u32 = 1024;

/// A rule; the name separates the counters of different rules.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Policy {
    pub name: &'static str,
    pub limit: Limit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Limit {
    Rate {
        burst: u32,
        period: Duration,
    },
    Lockout {
        failures: u32,
        window: Duration,
        lock: Duration,
    },
}

impl Policy {
    /// `burst` events at once, all of them again after `period`.
    pub const fn rate(name: &'static str, burst: u32, period: Duration) -> Self {
        Self {
            name,
            limit: Limit::Rate { burst, period },
        }
    }

    /// Locked for `lock` after `failures` attempts within `window`.
    pub const fn lockout(
        name: &'static str,
        failures: u32,
        window: Duration,
        lock: Duration,
    ) -> Self {
        Self {
            name,
            limit: Limit::Lockout {
                failures,
                window,
                lock,
            },
        }
    }
}

/// What is counted.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Key {
    Ip(IpAddr),
    User(String),
    /// One counter for everyone (e.g. setup token).
    Global,
    /// A Stalwart server (origin), e.g. to protect against its Fail2Ban.
    Server(String),
}

/// Limit reached; another attempt is allowed after `retry_after`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limited {
    pub retry_after: Duration,
}

#[derive(Debug, Clone, Copy)]
enum State {
    /// Theoretical time of the next event (GCRA).
    Rate { next: Instant },
    /// Attempts in the window until `window_end`; locked until `locked_until`.
    Lockout {
        count: u32,
        window_end: Instant,
        locked_until: Option<Instant>,
    },
}

impl State {
    /// From when the state equals a missing one (after a lock the full
    /// allowance applies again).
    fn expires(self) -> Instant {
        match self {
            Self::Rate { next } => next,
            Self::Lockout {
                window_end,
                locked_until,
                ..
            } => locked_until.unwrap_or(window_end),
        }
    }

    /// The state if it still matters at `now`.
    fn live(state: Option<Self>, now: Instant) -> Option<Self> {
        state.filter(|state| state.expires() > now)
    }
}

/// GCRA: next event (not before `now`), leeway and interval. An event is
/// allowed as long as `next` does not exceed the leeway.
fn gcra(
    burst: u32,
    period: Duration,
    next: Option<Instant>,
    now: Instant,
) -> (Instant, Instant, Duration) {
    let interval = period / burst.max(1);
    let next = next.map_or(now, |next| next.max(now));
    // Leeway: this far the counter may run ahead of the present.
    (next, now + period.saturating_sub(interval), interval)
}

/// How many attempts the rule still allows with `pending` reservations
/// open, or how long until the next one.
fn allowance(
    limit: Limit,
    state: Option<State>,
    pending: u32,
    now: Instant,
) -> Result<u32, Limited> {
    match (limit, state) {
        (
            Limit::Lockout { .. },
            Some(State::Lockout {
                locked_until: Some(until),
                ..
            }),
        ) => Err(Limited {
            retry_after: until - now,
        }),
        (Limit::Lockout { failures, .. }, state) => {
            let count = match state {
                Some(State::Lockout { count, .. }) => count,
                _ => 0,
            };
            match failures
                .max(1)
                .saturating_sub(count.saturating_add(pending))
            {
                // Everything left is held by checks still running; they end
                // within moments.
                0 => Err(Limited {
                    retry_after: PENDING_RETRY,
                }),
                left => Ok(left),
            }
        }
        (Limit::Rate { burst, period }, state) => {
            let next = match state {
                Some(State::Rate { next }) => Some(next),
                _ => None,
            };
            let (recorded, allowed_until, interval) = gcra(burst, period, next, now);
            if recorded > allowed_until {
                return Err(Limited {
                    retry_after: recorded - allowed_until,
                });
            }
            // Each pending reservation moves `next` on by one interval. If
            // they alone stand in the way, time does not help, but their end
            // within moments does.
            let next = recorded + interval.saturating_mul(pending);
            if next > allowed_until {
                return Err(Limited {
                    retry_after: PENDING_RETRY,
                });
            }
            let steps = (allowed_until - next).as_nanos() / interval.as_nanos().max(1);
            Ok(u32::try_from(steps + 1).map_or(burst, |n| n.min(burst.max(1))))
        }
    }
}

/// `state` after one more event at `now`, without checking the limit.
fn record(limit: Limit, state: Option<State>, now: Instant) -> State {
    match limit {
        Limit::Rate { burst, period } => {
            let next = match state {
                Some(State::Rate { next }) => Some(next),
                _ => None,
            };
            let (next, _, interval) = gcra(burst, period, next, now);
            State::Rate {
                next: next + interval,
            }
        }
        Limit::Lockout {
            failures,
            window,
            lock,
        } => match state {
            Some(
                locked @ State::Lockout {
                    locked_until: Some(_),
                    ..
                },
            ) => locked,
            // The window starts with the first attempt; later ones do not
            // extend it.
            Some(State::Lockout {
                count, window_end, ..
            }) => {
                let count = count.saturating_add(1);
                State::Lockout {
                    count,
                    window_end,
                    locked_until: (count >= failures.max(1)).then_some(now + lock),
                }
            }
            _ => State::Lockout {
                count: 1,
                window_end: now + window,
                locked_until: (failures <= 1).then_some(now + lock),
            },
        },
    }
}

/// `retry_after` while all attempts left are held by running checks.
const PENDING_RETRY: Duration = Duration::from_secs(1);

#[derive(Default)]
pub struct RateLimiter {
    inner: Mutex<Inner>,
}

#[derive(Default)]
struct Inner {
    states: HashMap<(&'static str, Key), Counter>,
    calls: u32,
    /// Last [`Counter::id`] handed out.
    last_id: u64,
}

/// One rule and key: the recorded attempts and the open reservations. A
/// counter that is reset or has expired starts over with a new ID, so that a
/// [`Reservation`] of the old one never ends one of the new one.
#[derive(Debug, Clone, Copy)]
struct Counter {
    id: u64,
    state: Option<State>,
    /// Open [`Reservation`]s; while there are any, the counter stays.
    pending: u32,
}

impl Counter {
    fn live(self, now: Instant) -> bool {
        self.pending > 0 || State::live(self.state, now).is_some()
    }
}

type Slot = (&'static str, Key);

impl Inner {
    /// Removes expired counters every [`SWEEP_EVERY`] changes.
    fn sweep(&mut self, now: Instant) {
        self.calls = self.calls.wrapping_add(1);
        if self.calls.is_multiple_of(SWEEP_EVERY) {
            self.states.retain(|_, counter| counter.live(now));
        }
    }

    /// The counter of `slot` with its live state, or a new one.
    fn counter(&mut self, slot: &Slot, now: Instant) -> Counter {
        match self.states.get(slot).filter(|counter| counter.live(now)) {
            Some(&counter) => Counter {
                state: State::live(counter.state, now),
                ..counter
            },
            None => {
                self.last_id += 1;
                Counter {
                    id: self.last_id,
                    state: None,
                    pending: 0,
                }
            }
        }
    }

    fn store(&mut self, slot: Slot, counter: Counter, now: Instant) {
        if counter.live(now) {
            self.states.insert(slot, counter);
        } else {
            self.states.remove(&slot);
        }
    }
}

impl RateLimiter {
    /// Takes an attempt that counts, if the rule allows it.
    pub fn acquire(&self, policy: &Policy, key: Key) -> Result<(), Limited> {
        self.acquire_with(policy, key, Instant::now)
    }

    /// Opens an attempt for a check whose result decides whether it counts:
    /// held as pending and counted only with [`Reservation::count`];
    /// otherwise it ends without a trace – even if the request is aborted
    /// before.
    pub fn reserve<'a>(&'a self, policy: &'a Policy, key: Key) -> Result<Reservation<'a>, Limited> {
        let counter = self.begin_with(policy, key.clone(), Instant::now)?;
        Ok(Reservation {
            limiter: self,
            policy,
            key: Some(key),
            counter,
        })
    }

    /// Counts an event that has already happened, even beyond the limit – for
    /// limits that only check with [`Self::remaining`] beforehand instead of
    /// reserving (correct passwords). With [`Policy::rate`] the next
    /// allowed time moves on by one interval, so a burst beyond the limit
    /// locks the following events correspondingly longer; with
    /// [`Policy::lockout`] it counts towards the lock, and a lock stays.
    pub fn charge(&self, policy: &Policy, key: Key) {
        self.charge_with(policy, key, Instant::now);
    }

    /// Resets the counter (e.g. after a successful setup).
    pub fn reset(&self, policy: &Policy, key: &Key) {
        self.lock().states.remove(&(policy.name, key.clone()));
    }

    /// How many attempts the rule still allows now (open reservations
    /// count), e.g. for "3 attempts left"; [`Limited`] as long as none is
    /// allowed.
    pub fn remaining(&self, policy: &Policy, key: &Key) -> Result<u32, Limited> {
        self.remaining_with(policy, key, Instant::now)
    }

    // The `…_with` functions take the clock and read it only once they hold
    // the lock.

    fn charge_with(&self, policy: &Policy, key: Key, clock: impl FnOnce() -> Instant) {
        let mut inner = self.lock();
        let now = clock();
        inner.sweep(now);
        let slot = (policy.name, key);
        let mut counter = inner.counter(&slot, now);
        counter.state = Some(record(policy.limit, counter.state, now));
        inner.store(slot, counter, now);
    }

    fn remaining_with(
        &self,
        policy: &Policy,
        key: &Key,
        clock: impl FnOnce() -> Instant,
    ) -> Result<u32, Limited> {
        let inner = self.lock();
        let now = clock();
        let (state, pending) = inner
            .states
            .get(&(policy.name, key.clone()))
            .filter(|counter| counter.live(now))
            .map_or((None, 0), |counter| {
                (State::live(counter.state, now), counter.pending)
            });
        allowance(policy.limit, state, pending, now)
    }

    fn acquire_with(
        &self,
        policy: &Policy,
        key: Key,
        clock: impl FnOnce() -> Instant,
    ) -> Result<(), Limited> {
        let mut inner = self.lock();
        let now = clock();
        inner.sweep(now);
        let slot = (policy.name, key);
        let mut counter = inner.counter(&slot, now);
        allowance(policy.limit, counter.state, counter.pending, now)?;
        counter.state = Some(record(policy.limit, counter.state, now));
        inner.store(slot, counter, now);
        Ok(())
    }

    /// Opens a reservation if the rule allows another attempt; returns the
    /// ID of its counter.
    fn begin_with(
        &self,
        policy: &Policy,
        key: Key,
        clock: impl FnOnce() -> Instant,
    ) -> Result<u64, Limited> {
        let mut inner = self.lock();
        let now = clock();
        inner.sweep(now);
        let slot = (policy.name, key);
        let mut counter = inner.counter(&slot, now);
        allowance(policy.limit, counter.state, counter.pending, now)?;
        counter.pending += 1;
        let id = counter.id;
        inner.store(slot, counter, now);
        Ok(id)
    }

    /// Ends a reservation of counter `id`; with `counted` its attempt counts
    /// now. If the counter was reset in between, only a counted attempt
    /// reaches the new one.
    fn finish_with(
        &self,
        policy: &Policy,
        key: Key,
        id: u64,
        counted: bool,
        clock: impl FnOnce() -> Instant,
    ) {
        let mut inner = self.lock();
        let now = clock();
        let slot = (policy.name, key);
        let mut counter = inner.counter(&slot, now);
        if counter.id == id {
            counter.pending = counter.pending.saturating_sub(1);
        } else if !counted {
            return;
        }
        if counted {
            counter.state = Some(record(policy.limit, counter.state, now));
        }
        inner.store(slot, counter, now);
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        // A panic under the lock only leaves counters behind; carry on.
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// The same with a fixed time, for tests across refill and window
/// boundaries.
#[cfg(test)]
impl RateLimiter {
    fn len(&self) -> usize {
        self.lock().states.len()
    }

    fn acquire_at(&self, policy: &Policy, key: Key, now: Instant) -> Result<(), Limited> {
        self.acquire_with(policy, key, || now)
    }

    fn charge_at(&self, policy: &Policy, key: Key, now: Instant) {
        self.charge_with(policy, key, || now);
    }

    fn remaining_at(&self, policy: &Policy, key: &Key, now: Instant) -> Result<u32, Limited> {
        self.remaining_with(policy, key, || now)
    }

    fn begin_at(&self, policy: &Policy, key: Key, now: Instant) -> Result<u64, Limited> {
        self.begin_with(policy, key, || now)
    }

    fn finish_at(&self, policy: &Policy, key: Key, id: u64, counted: bool, now: Instant) {
        self.finish_with(policy, key, id, counted, || now);
    }
}

/// An open attempt from [`RateLimiter::reserve`].
pub struct Reservation<'a> {
    limiter: &'a RateLimiter,
    policy: &'a Policy,
    /// `None` once the reservation has ended.
    key: Option<Key>,
    /// The counter the reservation was opened on.
    counter: u64,
}

impl Reservation<'_> {
    /// The attempt counts (e.g. a rejected password).
    pub fn count(mut self) {
        if let Some(key) = self.key.take() {
            self.limiter
                .finish_with(self.policy, key, self.counter, true, Instant::now);
        }
    }
}

impl Drop for Reservation<'_> {
    fn drop(&mut self) {
        if let Some(key) = self.key.take() {
            self.limiter
                .finish_with(self.policy, key, self.counter, false, Instant::now);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reservation_counts_only_when_told() {
        let limiter = RateLimiter::default();
        let policy = Policy::lockout(
            "reserve",
            2,
            Duration::from_secs(60),
            Duration::from_secs(60),
        );
        // Not counted (also aborted): any number of times.
        for _ in 0..5 {
            drop(limiter.reserve(&policy, Key::Global).unwrap());
        }
        limiter.reserve(&policy, Key::Global).unwrap().count();
        limiter.reserve(&policy, Key::Global).unwrap().count();
        assert!(limiter.reserve(&policy, Key::Global).is_err());
    }

    /// The case from the review: a reservation that ends late must not
    /// erase an attempt counted in between, even after the counter refilled.
    #[test]
    fn late_reservation_leaves_later_attempts_alone() {
        let limiter = RateLimiter::default();
        let t0 = Instant::now();
        let a = limiter.begin_at(&FIVE_PER_MINUTE, user(), t0).unwrap();
        let b = limiter
            .begin_at(&FIVE_PER_MINUTE, user(), t0 + secs(10))
            .unwrap();
        limiter.finish_at(&FIVE_PER_MINUTE, user(), b, true, t0 + secs(10));
        limiter.finish_at(&FIVE_PER_MINUTE, user(), a, false, t0 + secs(13));
        // Exactly as if only B had happened at t=10.
        let only_b = RateLimiter::default();
        only_b
            .acquire_at(&FIVE_PER_MINUTE, user(), t0 + secs(10))
            .unwrap();
        for at in [13, 21, 22, 30, 70] {
            assert_eq!(
                limiter.remaining_at(&FIVE_PER_MINUTE, &user(), t0 + secs(at)),
                only_b.remaining_at(&FIVE_PER_MINUTE, &user(), t0 + secs(at)),
                "t={at}"
            );
        }
        assert_eq!(
            limiter.remaining_at(&FIVE_PER_MINUTE, &user(), t0 + secs(13)),
            Ok(4)
        );

        // Counted late instead, both attempts count.
        let limiter = RateLimiter::default();
        let a = limiter.begin_at(&FIVE_PER_MINUTE, user(), t0).unwrap();
        limiter
            .acquire_at(&FIVE_PER_MINUTE, user(), t0 + secs(10))
            .unwrap();
        limiter.finish_at(&FIVE_PER_MINUTE, user(), a, true, t0 + secs(13));
        assert_eq!(
            limiter.remaining_at(&FIVE_PER_MINUTE, &user(), t0 + secs(13)),
            Ok(3)
        );
    }

    #[test]
    fn open_reservations_hold_the_allowance() {
        let once = Policy::rate("once", 1, secs(30));
        let limiter = RateLimiter::default();
        let t0 = Instant::now();
        let a = limiter.begin_at(&once, Key::Global, t0).unwrap();
        // Only the open reservation stands in the way: retry within moments,
        // not after a whole period.
        let short = Limited {
            retry_after: PENDING_RETRY,
        };
        let later = t0 + secs(60);
        assert_eq!(limiter.acquire_at(&once, Key::Global, later), Err(short));
        assert_eq!(limiter.begin_at(&once, Key::Global, later), Err(short));
        assert_eq!(limiter.remaining_at(&once, &Key::Global, later), Err(short));
        // Ending without counting leaves nothing behind.
        limiter.finish_at(&once, Key::Global, a, false, t0 + secs(61));
        assert_eq!(limiter.len(), 0);
        limiter
            .acquire_at(&once, Key::Global, t0 + secs(61))
            .unwrap();

        // Lockout: all attempts left held by running checks.
        let limiter = RateLimiter::default();
        for _ in 0..5 {
            limiter.begin_at(&SETUP, Key::Global, t0).unwrap();
        }
        assert_eq!(
            limiter.begin_at(&SETUP, Key::Global, t0),
            Err(Limited {
                retry_after: PENDING_RETRY
            })
        );
        assert!(limiter.remaining_at(&SETUP, &Key::Global, t0).is_err());
    }

    /// A recorded attempt still reports its refill time.
    #[test]
    fn recorded_attempts_keep_their_refill_time() {
        let once = Policy::rate("once", 1, secs(30));
        let limiter = RateLimiter::default();
        let t0 = Instant::now();
        limiter.acquire_at(&once, Key::Global, t0).unwrap();
        limiter.begin_at(&once, Key::Global, t0 + secs(30)).unwrap();
        assert_eq!(
            limiter.acquire_at(&once, Key::Global, t0 + secs(30)),
            Err(Limited {
                retry_after: PENDING_RETRY
            })
        );
        let limiter = RateLimiter::default();
        limiter.acquire_at(&once, Key::Global, t0).unwrap();
        assert_eq!(
            limiter.acquire_at(&once, Key::Global, t0 + secs(10)),
            Err(Limited {
                retry_after: secs(20)
            })
        );
    }

    /// The case from the review: an attempt that ends after its lockout
    /// window started over must not erase a failure of the new window.
    #[test]
    fn reservation_across_a_window_rollover_keeps_the_new_failure() {
        let three = Policy::lockout("three", 3, secs(600), secs(60));
        let limiter = RateLimiter::default();
        let t0 = Instant::now();
        limiter.acquire_at(&three, Key::Global, t0).unwrap();
        let a = limiter
            .begin_at(&three, Key::Global, t0 + secs(599))
            .unwrap();
        // The window has passed; B fails in the new one.
        let b = limiter
            .begin_at(&three, Key::Global, t0 + secs(601))
            .unwrap();
        limiter.finish_at(&three, Key::Global, b, true, t0 + secs(601));
        limiter.finish_at(&three, Key::Global, a, false, t0 + secs(602));
        assert_eq!(
            limiter.remaining_at(&three, &Key::Global, t0 + secs(602)),
            Ok(2)
        );
    }

    #[test]
    fn reads_the_clock_while_holding_the_lock() {
        let limiter = RateLimiter::default();
        let clock = || {
            assert!(
                limiter.inner.try_lock().is_err(),
                "clock read before the lock"
            );
            Instant::now()
        };
        limiter
            .acquire_with(&FIVE_PER_MINUTE, user(), clock)
            .unwrap();
        limiter.charge_with(&FIVE_PER_MINUTE, user(), clock);
        limiter
            .remaining_with(&FIVE_PER_MINUTE, &user(), clock)
            .unwrap();
        let id = limiter.begin_with(&FIVE_PER_MINUTE, user(), clock).unwrap();
        limiter.finish_with(&FIVE_PER_MINUTE, user(), id, false, clock);
    }

    #[test]
    fn reservation_ended_after_reset_leaves_the_new_counter_alone() {
        let policy = Policy::lockout("drop", 2, secs(600), secs(60));
        let limiter = RateLimiter::default();
        let reservation = limiter.reserve(&policy, Key::Global).unwrap();
        limiter.reset(&policy, &Key::Global);
        let _held = limiter.reserve(&policy, Key::Global).unwrap();
        limiter.reserve(&policy, Key::Global).unwrap().count();
        // Ending the old reservation frees nothing of the new counter.
        drop(reservation);
        assert!(limiter.reserve(&policy, Key::Global).is_err());
    }

    #[test]
    fn concurrent_reservations_never_exceed_the_limit() {
        let limiter = RateLimiter::default();
        let held: Vec<_> = (0..500)
            .filter_map(|_| limiter.reserve(&FIVE_PER_MINUTE, user()).ok())
            .collect();
        assert_eq!(held.len(), 5);
        let held: Vec<_> = (0..500)
            .filter_map(|_| limiter.reserve(&SETUP, Key::Global).ok())
            .collect();
        assert_eq!(held.len(), 5);
    }

    const FIVE_PER_MINUTE: Policy = Policy::rate("test", 5, Duration::from_secs(60));
    const SETUP: Policy = Policy::lockout(
        "lockout",
        5,
        Duration::from_secs(600),
        Duration::from_secs(30),
    );

    fn user() -> Key {
        Key::User("max".into())
    }

    fn secs(n: u64) -> Duration {
        Duration::from_secs(n)
    }

    #[test]
    fn rate_allows_burst_then_one_per_interval() {
        let limiter = RateLimiter::default();
        let t0 = Instant::now();
        for _ in 0..5 {
            limiter.acquire_at(&FIVE_PER_MINUTE, user(), t0).unwrap();
        }
        let err = limiter
            .acquire_at(&FIVE_PER_MINUTE, user(), t0)
            .unwrap_err();
        assert_eq!(err.retry_after, secs(12));
        // Other keys and rules count separately.
        limiter
            .acquire_at(&FIVE_PER_MINUTE, Key::Ip("192.0.2.1".parse().unwrap()), t0)
            .unwrap();
        let other = Policy::rate("other", 5, secs(60));
        limiter.acquire_at(&other, user(), t0).unwrap();

        let t1 = t0 + secs(12);
        limiter.acquire_at(&FIVE_PER_MINUTE, user(), t1).unwrap();
        assert!(limiter.acquire_at(&FIVE_PER_MINUTE, user(), t1).is_err());
        // After the full period the whole allowance again.
        let t2 = t1 + secs(60);
        for _ in 0..5 {
            limiter.acquire_at(&FIVE_PER_MINUTE, user(), t2).unwrap();
        }
    }

    #[test]
    fn reservations_that_do_not_count_leave_nothing() {
        let limiter = RateLimiter::default();
        let t0 = Instant::now();
        // Ten successful attempts: none of them counts.
        for _ in 0..10 {
            let id = limiter.begin_at(&FIVE_PER_MINUTE, user(), t0).unwrap();
            limiter.finish_at(&FIVE_PER_MINUTE, user(), id, false, t0);
        }
        assert_eq!(limiter.len(), 0);
        for _ in 0..5 {
            limiter.acquire_at(&FIVE_PER_MINUTE, user(), t0).unwrap();
        }
        assert!(limiter.acquire_at(&FIVE_PER_MINUTE, user(), t0).is_err());
        limiter.reset(&FIVE_PER_MINUTE, &user());
        limiter.acquire_at(&FIVE_PER_MINUTE, user(), t0).unwrap();
    }

    #[test]
    fn concurrent_attempts_never_exceed_the_limit() {
        // All reserve before any result is known (500 parallel logins): only
        // as many get through as the rule allows.
        let limiter = RateLimiter::default();
        let t0 = Instant::now();
        let passed = (0..500)
            .filter(|_| limiter.acquire_at(&FIVE_PER_MINUTE, user(), t0).is_ok())
            .count();
        assert_eq!(passed, 5);
        let passed = (0..500)
            .filter(|_| limiter.acquire_at(&SETUP, user(), t0).is_ok())
            .count();
        assert_eq!(passed, 5);
    }

    #[test]
    fn lockout_after_failures_then_full_allowance() {
        let limiter = RateLimiter::default();
        let t0 = Instant::now();
        for _ in 0..5 {
            limiter.acquire_at(&SETUP, Key::Global, t0).unwrap();
        }
        let err = limiter
            .acquire_at(&SETUP, Key::Global, t0 + secs(10))
            .unwrap_err();
        assert_eq!(err.retry_after, secs(20));
        // After the lock five attempts again, not just one.
        let t1 = t0 + secs(30);
        for _ in 0..5 {
            limiter.acquire_at(&SETUP, Key::Global, t1).unwrap();
        }
        assert!(limiter.acquire_at(&SETUP, Key::Global, t1).is_err());
    }

    #[test]
    fn lockout_forgets_successes_and_old_failures() {
        let limiter = RateLimiter::default();
        let t0 = Instant::now();
        for _ in 0..4 {
            limiter.acquire_at(&SETUP, Key::Global, t0).unwrap();
        }
        // The fifth attempt is a success and does not count.
        let id = limiter.begin_at(&SETUP, Key::Global, t0).unwrap();
        limiter.finish_at(&SETUP, Key::Global, id, false, t0);
        limiter.acquire_at(&SETUP, Key::Global, t0).unwrap();
        assert!(limiter.acquire_at(&SETUP, Key::Global, t0).is_err());

        // Four failures, then ten quiet minutes: the window starts over.
        let limiter = RateLimiter::default();
        for _ in 0..4 {
            limiter.acquire_at(&SETUP, Key::Global, t0).unwrap();
        }
        let later = t0 + secs(601);
        for _ in 0..5 {
            limiter.acquire_at(&SETUP, Key::Global, later).unwrap();
        }
        assert!(limiter.acquire_at(&SETUP, Key::Global, later).is_err());
    }

    #[test]
    fn lockout_window_starts_with_the_first_failure() {
        // One failure, then successes (not counted) every five minutes: they do
        // not extend the window, after ten minutes the failure is gone.
        let limiter = RateLimiter::default();
        let t0 = Instant::now();
        limiter.acquire_at(&SETUP, Key::Global, t0).unwrap();
        for minute in [5, 9] {
            let at = t0 + secs(minute * 60);
            let id = limiter.begin_at(&SETUP, Key::Global, at).unwrap();
            limiter.finish_at(&SETUP, Key::Global, id, false, at);
        }
        let later = t0 + secs(601);
        for _ in 0..5 {
            limiter.acquire_at(&SETUP, Key::Global, later).unwrap();
        }
        assert!(limiter.acquire_at(&SETUP, Key::Global, later).is_err());
    }

    #[test]
    fn successes_at_the_last_attempt_keep_the_window() {
        // Four failures, then correct tokens (each reserved as the last
        // attempt, then not counted): after ten minutes the failures are still
        // forgotten.
        let limiter = RateLimiter::default();
        let t0 = Instant::now();
        for _ in 0..4 {
            limiter.acquire_at(&SETUP, Key::Global, t0).unwrap();
        }
        for minute in [3, 6, 9] {
            let at = t0 + secs(minute * 60);
            let id = limiter.begin_at(&SETUP, Key::Global, at).unwrap();
            limiter.finish_at(&SETUP, Key::Global, id, false, at);
            assert_eq!(limiter.remaining_at(&SETUP, &Key::Global, at), Ok(1));
        }
        assert_eq!(
            limiter.remaining_at(&SETUP, &Key::Global, t0 + secs(601)),
            Ok(5)
        );
    }

    #[test]
    fn remaining_counts_down_to_the_lock() {
        let limiter = RateLimiter::default();
        let t0 = Instant::now();
        assert_eq!(limiter.remaining_at(&SETUP, &Key::Global, t0), Ok(5));
        for left in (1..5).rev() {
            limiter.acquire_at(&SETUP, Key::Global, t0).unwrap();
            assert_eq!(limiter.remaining_at(&SETUP, &Key::Global, t0), Ok(left));
        }
        limiter.acquire_at(&SETUP, Key::Global, t0).unwrap();
        assert_eq!(
            limiter.remaining_at(&SETUP, &Key::Global, t0 + secs(10)),
            Err(Limited {
                retry_after: secs(20)
            })
        );
        assert_eq!(
            limiter.remaining_at(&SETUP, &Key::Global, t0 + secs(30)),
            Ok(5)
        );

        // Rate: as many as `acquire` would still let through.
        assert_eq!(limiter.remaining_at(&FIVE_PER_MINUTE, &user(), t0), Ok(5));
        for left in (0..5).rev() {
            limiter.acquire_at(&FIVE_PER_MINUTE, user(), t0).unwrap();
            if left > 0 {
                assert_eq!(
                    limiter.remaining_at(&FIVE_PER_MINUTE, &user(), t0),
                    Ok(left)
                );
            }
        }
        assert_eq!(
            limiter.remaining_at(&FIVE_PER_MINUTE, &user(), t0),
            Err(Limited {
                retry_after: secs(12)
            })
        );
        assert_eq!(
            limiter.remaining_at(&FIVE_PER_MINUTE, &user(), t0 + secs(24)),
            Ok(2)
        );
    }

    #[test]
    fn charge_counts_beyond_the_limit() {
        // Two at once, then one per 5 s.
        let two = Policy::rate("two", 2, secs(10));
        let limiter = RateLimiter::default();
        let t0 = Instant::now();
        assert_eq!(limiter.remaining_at(&two, &Key::Global, t0), Ok(2));
        limiter.charge_at(&two, Key::Global, t0);
        assert_eq!(limiter.remaining_at(&two, &Key::Global, t0), Ok(1));
        // Four events, two beyond the limit: only after three intervals is
        // another one allowed.
        for _ in 0..3 {
            limiter.charge_at(&two, Key::Global, t0);
        }
        let err = limiter.remaining_at(&two, &Key::Global, t0).unwrap_err();
        assert_eq!(err.retry_after, secs(15));
        assert_eq!(
            limiter.remaining_at(&two, &Key::Global, t0 + secs(15)),
            Ok(1)
        );
        // Lockouts count like `acquire`.
        let limiter = RateLimiter::default();
        for _ in 0..6 {
            limiter.charge_at(&SETUP, Key::Global, t0);
        }
        assert!(limiter.remaining_at(&SETUP, &Key::Global, t0).is_err());
    }

    #[test]
    fn single_attempt_rate_locks_for_period() {
        let once = Policy::rate("once", 1, secs(30));
        let limiter = RateLimiter::default();
        let t0 = Instant::now();
        limiter.acquire_at(&once, Key::Global, t0).unwrap();
        let err = limiter.acquire_at(&once, Key::Global, t0).unwrap_err();
        assert_eq!(err.retry_after, secs(30));
    }

    #[test]
    fn sweeps_expired_counters() {
        let limiter = RateLimiter::default();
        let t0 = Instant::now();
        for i in 0..100u8 {
            let rate = Key::Ip(IpAddr::from([192, 0, 2, i]));
            let lockout = Key::Ip(IpAddr::from([198, 51, 100, i]));
            limiter.acquire_at(&FIVE_PER_MINUTE, rate, t0).unwrap();
            limiter.acquire_at(&SETUP, lockout, t0).unwrap();
        }
        assert_eq!(limiter.len(), 200);
        let later = t0 + secs(601);
        for _ in 0..SWEEP_EVERY {
            limiter
                .acquire_at(&FIVE_PER_MINUTE, user(), later)
                .unwrap_or(());
        }
        assert_eq!(limiter.len(), 1);

        // `charge` alone (checked with `remaining`) sweeps as well.
        let limiter = RateLimiter::default();
        for i in 0..100u8 {
            limiter.charge_at(&FIVE_PER_MINUTE, Key::Ip(IpAddr::from([192, 0, 2, i])), t0);
        }
        assert_eq!(limiter.len(), 100);
        for _ in 0..SWEEP_EVERY {
            limiter.charge_at(&FIVE_PER_MINUTE, user(), later);
        }
        assert_eq!(limiter.len(), 1);
    }
}
