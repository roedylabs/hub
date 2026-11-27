//! Renewal of the TLS certificate (daily, `ROEDY_TLS=acme` only;
//! `tls::issuer`). The start of `roedy-hub serve` checks it once, so the
//! first scheduled run follows a day later. A failure is logged by the
//! issuer and waits as its backoff says; the next run tries again.

use std::time::Duration;

use super::Job;
use crate::app::AppState;

pub fn job() -> Job<AppState> {
    Job {
        name: "certificate_renewal",
        first: Duration::from_secs(24 * 60 * 60),
        every: Duration::from_secs(24 * 60 * 60),
        run: |state| Box::pin(async move { run(&state).await }),
    }
}

/// One run; nothing to do without ACME.
pub async fn run(state: &AppState) -> anyhow::Result<()> {
    let Some(issuer) = state.tls.as_ref().and_then(|tls| tls.issuer.clone()) else {
        return Ok(());
    };
    issuer.run_once().await.map(drop)
}
