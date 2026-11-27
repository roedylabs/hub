//! Event retention (daily): deletes events older than 30 days
//! (`events::RETENTION`, docs/app-contract.md › Catching up), delivered or
//! not. An app that asks for events before them gets `410 events_expired`.

use std::time::Duration;

use super::Job;
use crate::{app::AppState, db, db::repo::events, events::RETENTION};

pub fn job() -> Job<AppState> {
    Job {
        name: "event_retention",
        first: Duration::from_secs(15 * 60),
        every: Duration::from_secs(24 * 60 * 60),
        run: |state| Box::pin(async move { run(&state).await.map(|_| ()) }),
    }
}

/// One run; returns the number of deleted events.
pub async fn run(state: &AppState) -> anyhow::Result<usize> {
    let deleted = db::call(&state.db, |conn| {
        Ok(events::delete_older_than(conn, RETENTION)?)
    })
    .await?;
    if deleted > 0 {
        tracing::info!(deleted, "old events deleted");
    }
    Ok(deleted)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::repo::events::tests::paired;

    #[tokio::test]
    async fn deletes_what_is_older_than_30_days() {
        let (state, _tmp) = crate::app::tests::state().await;
        let mail = db::call(&state.db, |conn| {
            let mail = paired(conn, "mail", 1);
            for sub in ["old", "edge", "new"] {
                events::user_locked(conn, sub)?;
            }
            conn.execute(
                "UPDATE events SET created_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now', \
                 CASE data ->> 'sub' WHEN 'old' THEN '-31 days' ELSE '-29 days' END) \
                 WHERE data ->> 'sub' <> 'new'",
                [],
            )?;
            Ok(mail)
        })
        .await
        .unwrap();
        assert_eq!(run(&state).await.unwrap(), 1);
        assert_eq!(run(&state).await.unwrap(), 0);
        let left = db::call(&state.db, move |conn| Ok(events_of_kept(conn, &mail)))
            .await
            .unwrap();
        assert_eq!(left, [2, 3]);
    }

    fn events_of_kept(conn: &rusqlite::Connection, app_id: &str) -> Vec<u64> {
        conn.prepare("SELECT seq FROM events WHERE app_id = ?1 ORDER BY seq")
            .unwrap()
            .query_map([app_id], |row| row.get::<_, i64>(0))
            .unwrap()
            .map(|seq| u64::try_from(seq.unwrap()).unwrap())
            .collect()
    }
}
