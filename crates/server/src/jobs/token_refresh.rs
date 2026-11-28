//! Early refresh: Stalwart accounts for which a browser fetched an access
//! token in the last hour get a new one before the old one expires – so
//! that the browser does not wait for the server on its next fetch. The job
//! runs every minute; errors are already in the token broker's log.
//!
//! Taken over from Roedy Mail (`crates/server/src/jobs/token_refresh.rs`).

use std::time::Duration;

use super::Job;
use crate::{
    app::AppState,
    db::{self, repo::stalwart_accounts as accounts},
    stalwart::token::{self, ACTIVE, PREFETCH_MARGIN},
};

pub fn job() -> Job<AppState> {
    Job {
        name: "token_refresh",
        first: Duration::from_secs(60),
        every: Duration::from_secs(60),
        run: |state| Box::pin(async move { run(&state).await.map(|_| ()) }),
    }
}

/// One run; returns how many accounts were due.
pub async fn run(state: &AppState) -> anyhow::Result<usize> {
    let due = db::call(&state.db, |conn| {
        Ok(accounts::due_for_refresh(conn, ACTIVE, PREFETCH_MARGIN)?)
    })
    .await?;
    let count = due.len();
    for id in due {
        // One after the other; an error only affects this account.
        if let Err(token::Error::Internal(error)) = token::prefetch(state, &id).await {
            tracing::warn!(account = %id, "early refresh failed: {error:#}");
        }
    }
    Ok(count)
}
