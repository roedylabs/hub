//! Rotation of the signing keys (daily; `signing::rotate`). The start of
//! `roedy-hub serve` runs it once before the listeners open, so the first
//! scheduled run follows a day later.

use std::time::Duration;

use super::Job;
use crate::{app::AppState, signing};

pub fn job() -> Job<AppState> {
    Job {
        name: "key_rotation",
        first: Duration::from_secs(24 * 60 * 60),
        every: Duration::from_secs(24 * 60 * 60),
        run: |state| Box::pin(async move { run(&state).await.map(|_| ()) }),
    }
}

/// One run.
pub async fn run(state: &AppState) -> anyhow::Result<signing::Rotation> {
    signing::rotate_now(&state.db, state.secrets.clone(), &state.keys).await
}
