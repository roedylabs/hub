//! Table `deleted_users`: tombstones of deleted users, kept permanently, so
//! that `GET /internal/v1/users/<sub>` answers `410 user_deleted` for them
//! (docs/app-contract.md › Internal API). `users::delete` records them.

use rusqlite::{Connection, OptionalExtension as _};

/// Records that `sub` was deleted (once; the first time stays).
pub fn record(conn: &Connection, sub: &str) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT OR IGNORE INTO deleted_users (sub) VALUES (?1)",
        [sub],
    )?;
    Ok(())
}

/// When `sub` was deleted, if it was.
pub fn deleted_at(conn: &Connection, sub: &str) -> rusqlite::Result<Option<String>> {
    conn.query_row(
        "SELECT deleted_at FROM deleted_users WHERE sub = ?1",
        [sub],
        |row| row.get(0),
    )
    .optional()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::repo::users::{
        self, Role,
        tests::{add, conn},
    };

    #[test]
    fn deleting_a_user_leaves_a_tombstone() {
        let conn = conn();
        let max = add(&conn, "max", "max@example.com", Role::User);
        assert_eq!(deleted_at(&conn, &max).unwrap(), None);
        assert!(users::delete(&conn, &max).unwrap());
        let at = deleted_at(&conn, &max).unwrap().unwrap();
        assert_eq!(at.len(), "2026-11-26T12:00:00.000Z".len());
        // Deleting what is gone changes nothing.
        assert!(!users::delete(&conn, &max).unwrap());
        record(&conn, &max).unwrap();
        assert_eq!(deleted_at(&conn, &max).unwrap(), Some(at));
        // Unknown IDs have none.
        assert!(!users::delete(&conn, "unknown").unwrap());
        assert_eq!(deleted_at(&conn, "unknown").unwrap(), None);
    }
}
