//! Audit log retention (daily): deletes what is older than the configured
//! period (30, 90 or 365 days, `audit_log::keep_days`). Reading and export
//! already hide such entries; the cleanup keeps the table small and removes
//! IP addresses of old entries.
//!
//! Taken over from Roedy Mail (`crates/server/src/jobs/audit_retention.rs`).

use std::time::Duration;

use super::Job;
use crate::{app::AppState, db, db::repo::audit_log};

pub fn job() -> Job<AppState> {
    Job {
        name: "audit_retention",
        first: Duration::from_secs(10 * 60),
        every: Duration::from_secs(24 * 60 * 60),
        run: |state| Box::pin(async move { run(&state).await.map(|_| ()) }),
    }
}

/// One run; returns the number of deleted entries.
pub async fn run(state: &AppState) -> anyhow::Result<usize> {
    let deleted = db::call(&state.db, |conn| {
        let days = audit_log::keep_days(conn)?;
        Ok(audit_log::delete_old(conn, days)?)
    })
    .await?;
    if deleted > 0 {
        tracing::info!(deleted, "old audit log entries deleted");
    }
    Ok(deleted)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::repo::audit_log::{Category, Entry};

    #[tokio::test]
    async fn deletes_what_is_older_than_the_retention() {
        let (state, _tmp) = crate::app::tests::state().await;
        db::call(&state.db, |conn| {
            for action in ["old", "middle", "new"] {
                audit_log::insert(conn, &Entry::new(Category::System, action))?;
            }
            conn.execute(
                "UPDATE audit_log SET at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now', \
                 CASE action WHEN 'old' THEN '-100 days' ELSE '-40 days' END) \
                 WHERE action <> 'new'",
                [],
            )?;
            Ok(())
        })
        .await
        .unwrap();
        assert_eq!(run(&state).await.unwrap(), 1, "default: 90 days");
        db::call(&state.db, |conn| audit_log::set_keep_days(conn, 30))
            .await
            .unwrap();
        assert_eq!(run(&state).await.unwrap(), 1);
        assert_eq!(run(&state).await.unwrap(), 0);
    }
}
