//! Daily diagnosis of the Stalwart servers: servers of the registry whose
//! last test (automatic or by "Test connection" in the administration) is
//! more than a day ago are checked again. The job runs hourly so that
//! restarts do not shift the rhythm.
//!
//! Taken over from Roedy Mail (`crates/server/src/jobs/server_probe.rs`).

use std::time::Duration;

use super::Job;
use crate::{
    app::AppState,
    db::{self, repo::stalwart_servers as servers},
    stalwart::probe,
};

/// Age of a diagnosis after which the job checks again.
pub const EVERY: Duration = Duration::from_secs(24 * 60 * 60);

pub fn job() -> Job<AppState> {
    Job {
        name: "server_probe",
        first: Duration::from_secs(5 * 60),
        every: Duration::from_secs(60 * 60),
        run: |state| Box::pin(async move { run(&state).await.map(|_| ()) }),
    }
}

/// One run; returns how many servers were checked.
pub async fn run(state: &AppState) -> anyhow::Result<usize> {
    let due = db::call(&state.db, |conn| servers::due_for_test(conn, EVERY)).await?;
    let count = due.len();
    for server in due {
        // One after the other; an error only affects this server.
        if let Err(error) = probe::test_registered(state, &server, true).await {
            tracing::warn!(server = %server.id, "diagnosis not stored: {error:#}");
        }
    }
    Ok(count)
}
