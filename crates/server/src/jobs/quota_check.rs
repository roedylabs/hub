//! Daily storage check: for every signed-in Stalwart account of an active
//! user the job asks once a day with an access token from the broker for
//! `Quota/get` and stores the storage like the account's check. The job runs
//! hourly so that restarts do not shift the rhythm; a failed attempt counts
//! too, otherwise it would ask a broken server every hour.
//!
//! Taken over from Roedy Mail (`crates/server/src/jobs/quota_check.rs`)
//! without the "storage almost full" notification, which comes with the
//! hub's notifications.

use std::time::Duration;

use roedy_jmap::Storage;
use rusqlite::Connection;
use url::Url;

use super::Job;
use crate::{
    app::AppState,
    db::{
        self,
        repo::{
            stalwart_accounts::{self as accounts, Account},
            stalwart_servers as servers,
        },
    },
    stalwart::{account, token},
};

/// Interval between checks per account.
const EVERY: Duration = Duration::from_secs(24 * 60 * 60);

pub fn job() -> Job<AppState> {
    Job {
        name: "quota_check",
        first: Duration::from_secs(15 * 60),
        every: Duration::from_secs(60 * 60),
        run: |state| Box::pin(async move { run(&state).await.map(|_| ()) }),
    }
}

/// Outcome of a check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Outcome {
    /// Storage according to the server (`None` without a limit).
    Checked(Option<Storage>),
    /// Failed (written to the log); counts as the day's attempt.
    Failed,
    /// The broker is currently throttling the server (`Limited`, e.g. after a
    /// temporary error of another account): no attempt, the next run asks
    /// again – otherwise a single error would shift all other accounts of the
    /// server by a day.
    Later,
}

/// One run; returns how many accounts were due.
pub async fn run(state: &AppState) -> anyhow::Result<usize> {
    let due = db::call(&state.db, |conn| {
        Ok(accounts::due_for_quota_check(conn, EVERY)?)
    })
    .await?;
    let count = due.len();
    for account in due {
        // One after the other; an error only affects this account.
        let outcome = check(state, &account).await;
        if outcome == Outcome::Later {
            continue;
        }
        let id = account.id.clone();
        let saved = db::call(&state.db, move |conn| record(conn, &account, outcome)).await;
        if let Err(error) = saved {
            tracing::warn!(account = %id, "storage not stored: {error:#}");
        }
    }
    Ok(count)
}

/// Asks the server for the storage. Unlike the account's check without a
/// second attempt with a refreshed token: the next day there is a new one
/// anyway.
async fn check(state: &AppState, account: &Account) -> Outcome {
    let id = &account.id;
    let access = match token::for_backend(state, id, None, false).await {
        Ok(access) => access,
        Err(token::Error::Limited(_)) => {
            tracing::debug!(account = %id, "check storage later: server throttled");
            return Outcome::Later;
        }
        Err(token::Error::Internal(error)) => {
            tracing::warn!(account = %id, "storage not checked: {error:#}");
            return Outcome::Failed;
        }
        Err(error) => {
            tracing::info!(account = %id, reason = error.reason(), "storage not checked");
            return Outcome::Failed;
        }
    };
    let base_url = match Url::parse(&account.base_url) {
        Ok(url) => url,
        Err(error) => {
            tracing::warn!(account = %id, "storage not checked: {error}");
            return Outcome::Failed;
        }
    };
    // Look it up now, not at the start of the run: a server removed in the
    // meantime is a custom one and only goes through the SSRF guard.
    let origin = account.base_url.clone();
    let registered = db::call(&state.db, move |conn| {
        Ok(servers::find_by_base_url(conn, &origin)?.is_some())
    })
    .await;
    let registered = match registered {
        Ok(registered) => registered,
        Err(error) => {
            tracing::warn!(account = %id, "storage not checked: {error:#}");
            return Outcome::Failed;
        }
    };
    let http = state.stalwart.http(registered);
    match account::storage(http, &base_url, access.access_token).await {
        Ok(storage) => Outcome::Checked(storage),
        Err(error) => {
            tracing::info!(account = %id, reason = error.reason(), "storage not checked");
            Outcome::Failed
        }
    }
}

/// Stores an attempt: always the time, with a result also the storage.
fn record(conn: &Connection, account: &Account, outcome: Outcome) -> anyhow::Result<()> {
    accounts::mark_quota_job(conn, &account.id)?;
    if let Outcome::Checked(storage) = outcome {
        let quota = storage.map(|Storage { used, total }| (used, total));
        // `None`: removed in the meantime.
        accounts::store_check(conn, &account.user_id, &account.id, quota)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::repo::{
        stalwart_accounts::tests::account as new_account,
        users::{
            Role,
            tests::{add, conn},
        },
    };

    #[test]
    fn every_attempt_counts_and_results_are_stored() {
        let conn = conn();
        let max = add(&conn, "max", "max@example.com", Role::User);
        accounts::insert(&conn, &new_account("A1", &max, "a")).unwrap();
        let account = accounts::get(&conn, &max, "A1").unwrap().unwrap();
        record(&conn, &account, Outcome::Failed).unwrap();
        assert!(
            accounts::due_for_quota_check(&conn, EVERY)
                .unwrap()
                .is_empty()
        );
        let stored = accounts::get(&conn, &max, "A1").unwrap().unwrap();
        assert_eq!(stored.last_check_at, None);
        let storage = Storage { used: 5, total: 10 };
        record(&conn, &account, Outcome::Checked(Some(storage))).unwrap();
        let stored = accounts::get(&conn, &max, "A1").unwrap().unwrap();
        assert_eq!((stored.quota_used, stored.quota_total), (Some(5), Some(10)));
    }
}
