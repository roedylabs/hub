//! Delete expired sessions, links from mails and pairing codes (hourly). They are no longer
//! valid anyway (`sessions::authenticate` and the links check the expiry);
//! the cleanup keeps the tables small and removes IP addresses nobody needs
//! any more. Pending email changes stay until the person discards or
//! replaces them (`email_changes::delete_stale`).
//!
//! Taken over from Roedy Mail (`crates/server/src/jobs/session_cleanup.rs`).

use std::time::Duration;

use super::Job;
use crate::{
    app::AppState,
    db,
    db::repo::{apps, email_changes, password_resets, sessions},
};

pub fn job() -> Job<AppState> {
    Job {
        name: "session_cleanup",
        first: Duration::from_secs(60),
        every: Duration::from_secs(60 * 60),
        run: |state| Box::pin(async move { run(&state).await.map(|_| ()) }),
    }
}

/// One run; returns the number of deleted sessions and links.
pub async fn run(state: &AppState) -> anyhow::Result<(usize, usize)> {
    let (deleted, links) = db::call(&state.db, |conn| {
        let deleted = sessions::delete_expired(conn)?;
        let links = password_resets::delete_expired(conn)?
            + email_changes::delete_stale(conn)?
            + apps::delete_expired_codes(conn)?;
        Ok((deleted, links))
    })
    .await?;
    if deleted > 0 {
        tracing::info!(deleted, "expired sessions deleted");
    }
    if links > 0 {
        tracing::info!(deleted = links, "expired links and pairing codes deleted");
    }
    Ok((deleted, links))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        app::tests::state,
        auth::session::{self, Client},
        db::repo::users::{Role, tests::add},
    };

    #[tokio::test]
    async fn deletes_only_what_has_expired() {
        let (state, _tmp) = state().await;
        db::call(&state.db, |conn| {
            let max = add(conn, "max", "max@example.com", Role::User);
            session::start(conn, &max, &Client::default(), true)?;
            session::start(conn, &max, &Client::default(), true)?;
            password_resets::insert(conn, &max, &[1; 32])?;
            password_resets::insert(conn, &max, &[2; 32])?;
            let change = email_changes::reserve(conn, &max, "new@example.com", &[3; 32])?;
            email_changes::release(conn, &change)?;
            let past = "'2000-01-01T00:00:00.000Z'";
            conn.execute(
                &format!(
                    "UPDATE sessions SET expires_at = {past} \
                     WHERE rowid = (SELECT min(rowid) FROM sessions)"
                ),
                [],
            )?;
            conn.execute(
                &format!("UPDATE password_resets SET expires_at = {past}"),
                [],
            )?;
            Ok(())
        })
        .await
        .unwrap();

        assert_eq!(run(&state).await.unwrap(), (1, 1));
        assert_eq!(run(&state).await.unwrap(), (0, 0), "nothing left");
        let left = db::call(&state.db, |conn| {
            let count = |table: &str| -> rusqlite::Result<i64> {
                conn.query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
            };
            Ok((
                count("sessions")?,
                count("password_resets")?,
                count("email_changes")?,
            ))
        })
        .await
        .unwrap();
        assert_eq!(left, (1, 0, 1), "the open email change stays");
    }
}
