//! Open streams of ended sessions (contract § 3.5): the API closes a
//! session's streams when it ends the session itself; this job catches the
//! sessions that end otherwise – expired, trimmed by a new sign-in beyond the
//! limit, ended by `roedy-hub reset-admin` in another process – every 2 s
//! (at most 2.2 s with the scheduler's jitter) with a check of at most
//! [`CHECK_WITHIN`], so that their streams close within 5 s. If the check
//! does not finish in time (the database is busy), the streams stay open
//! until a later run can check them: ending every stream whenever the
//! database is slow would cut off every user. Without open streams it costs
//! no database query.

use std::time::Duration;

use super::Job;
use crate::{
    app::AppState,
    db::{self, repo::sessions},
};

pub const EVERY: Duration = Duration::from_secs(2);
/// The longest a run waits for the database.
pub const CHECK_WITHIN: Duration = Duration::from_millis(1500);

pub fn job() -> Job<AppState> {
    Job {
        name: "gateway_streams",
        first: EVERY,
        every: EVERY,
        run: |state| Box::pin(async move { run(&state).await.map(|_| ()) }),
    }
}

/// One run: ends the streams of the sessions that are no longer valid and
/// returns their IDs.
pub async fn run(state: &AppState) -> anyhow::Result<Vec<String>> {
    let open = state.gateway.streams.sessions();
    if open.is_empty() {
        return Ok(Vec::new());
    }
    let ids = open.clone();
    let check = db::call(&state.db, move |conn| {
        Ok(sessions::valid_among(conn, &ids)?)
    });
    let valid = tokio::time::timeout(CHECK_WITHIN, check)
        .await
        .map_err(|_| anyhow::anyhow!("sessions not checked within {CHECK_WITHIN:?}"))??;
    let ended: Vec<String> = open
        .into_iter()
        .filter(|sid| !valid.contains(sid))
        .collect();
    for sid in &ended {
        state.end_session(sid);
    }
    if !ended.is_empty() {
        tracing::info!(sessions = ended.len(), "streams of ended sessions closed");
    }
    Ok(ended)
}
