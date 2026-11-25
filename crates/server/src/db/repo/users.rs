//! Table `users`. For now only what the setup token needs; the rest follows
//! with sign-in and the administration (taken over from Roedy Mail,
//! `crates/server/src/db/repo/users.rs`).

use rusqlite::Connection;

/// Whether the initial setup has created an administrator.
pub fn admin_exists(conn: &Connection) -> rusqlite::Result<bool> {
    conn.query_row(
        "SELECT EXISTS (SELECT 1 FROM users WHERE role = 'admin')",
        [],
        |row| row.get(0),
    )
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::db::migrations;

    pub(crate) fn conn() -> Connection {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.pragma_update(None, "foreign_keys", "ON").unwrap();
        migrations::run(&mut conn).unwrap();
        conn
    }

    /// Adds a user with `role` (`admin` or `user`); returns the ID.
    pub(crate) fn add(conn: &Connection, username: &str, email: &str, role: &str) -> String {
        let id = format!("U-{username}");
        conn.execute(
            "INSERT INTO users (id, username, email, display_name, role, password_hash) \
             VALUES (?1, ?2, ?3, 'Test User', ?4, '$argon2id$test')",
            (&id, username, email, role),
        )
        .unwrap();
        id
    }

    #[test]
    fn admin_exists_only_with_the_admin_role() {
        let conn = conn();
        assert!(!admin_exists(&conn).unwrap());
        add(&conn, "eva", "eva@example.com", "user");
        assert!(!admin_exists(&conn).unwrap());
        add(&conn, "max", "max@example.com", "admin");
        assert!(admin_exists(&conn).unwrap());
    }
}
