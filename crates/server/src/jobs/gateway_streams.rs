//! Open streams of ended sessions (contract § 3.5): the API closes a
//! session's streams when it ends the session itself; this job catches the
//! sessions that end otherwise – expired, trimmed by a new sign-in beyond the
//! limit, ended by `roedy-hub reset-admin` in another process – every 3 s,
//! so that their streams close within 5 s. Without open streams it costs no
//! database query.

use std::time::Duration;

use super::Job;
use crate::{
    app::AppState,
    db::{self, repo::sessions},
};

pub const EVERY: Duration = Duration::from_secs(3);

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
    let valid = db::call(&state.db, move |conn| {
        Ok(sessions::valid_among(conn, &ids)?)
    })
    .await?;
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
