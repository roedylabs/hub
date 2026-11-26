//! Table `signing_keys`: the hub's Ed25519 keys. There are only ever a few
//! rows, so `signing` reads them all and does the time arithmetic in Rust;
//! times are written in the format of the other tables ([`format`]).

use jiff::Timestamp;
use rusqlite::Connection;

/// One stored key; the private part still encrypted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub kid: String,
    pub private_enc: Vec<u8>,
    pub public: Vec<u8>,
    pub active_from: Timestamp,
    pub retired_at: Option<Timestamp>,
}

/// A time as the tables store it: UTC with milliseconds.
pub fn format(at: Timestamp) -> String {
    at.strftime("%Y-%m-%dT%H:%M:%S%.3fZ").to_string()
}

fn parse(text: &str) -> rusqlite::Result<Timestamp> {
    text.parse().map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
    })
}

/// All keys, oldest `active_from` first.
pub fn all(conn: &Connection) -> rusqlite::Result<Vec<Row>> {
    let mut stmt = conn.prepare(
        "SELECT kid, private_enc, public, active_from, retired_at FROM signing_keys \
         ORDER BY active_from, kid",
    )?;
    stmt.query_map([], |row| {
        let retired_at: Option<String> = row.get(4)?;
        Ok(Row {
            kid: row.get(0)?,
            private_enc: row.get(1)?,
            public: row.get(2)?,
            active_from: parse(&row.get::<_, String>(3)?)?,
            retired_at: retired_at.as_deref().map(parse).transpose()?,
        })
    })?
    .collect()
}

pub fn insert(
    conn: &Connection,
    kid: &str,
    private_enc: &[u8],
    public: &[u8; 32],
    active_from: Timestamp,
) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO signing_keys (kid, private_enc, public, active_from) VALUES (?1, ?2, ?3, ?4)",
        (kid, private_enc, public.as_slice(), format(active_from)),
    )?;
    Ok(())
}

/// Sets `retired_at` unless the key is already retired.
pub fn retire(conn: &Connection, kid: &str, at: Timestamp) -> rusqlite::Result<bool> {
    Ok(conn.execute(
        "UPDATE signing_keys SET retired_at = ?2 WHERE kid = ?1 AND retired_at IS NULL",
        (kid, format(at)),
    )? > 0)
}

pub fn delete(conn: &Connection, kid: &str) -> rusqlite::Result<bool> {
    Ok(conn.execute("DELETE FROM signing_keys WHERE kid = ?1", [kid])? > 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::migrations;

    #[test]
    fn stores_reads_retires_and_deletes() {
        let mut conn = Connection::open_in_memory().unwrap();
        migrations::run(&mut conn).unwrap();
        let t: Timestamp = "2026-11-26T12:00:00.123Z".parse().unwrap();
        let later = t + jiff::SignedDuration::from_mins(10);
        insert(&conn, "B", b"sealed-b", &[2; 32], later).unwrap();
        insert(&conn, "A", b"sealed-a", &[1; 32], t).unwrap();
        assert!(
            insert(&conn, "A", b"again", &[1; 32], t).is_err(),
            "unique kid"
        );

        assert!(retire(&conn, "A", later).unwrap());
        assert!(!retire(&conn, "A", t).unwrap(), "retired once");
        let rows = all(&conn).unwrap();
        assert_eq!(
            rows,
            [
                Row {
                    kid: "A".into(),
                    private_enc: b"sealed-a".to_vec(),
                    public: vec![1; 32],
                    active_from: t,
                    retired_at: Some(later),
                },
                Row {
                    kid: "B".into(),
                    private_enc: b"sealed-b".to_vec(),
                    public: vec![2; 32],
                    active_from: later,
                    retired_at: None,
                },
            ]
        );
        let stored: String = conn
            .query_row(
                "SELECT active_from FROM signing_keys WHERE kid = 'A'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(stored, "2026-11-26T12:00:00.123Z");

        assert!(delete(&conn, "A").unwrap());
        assert!(!delete(&conn, "A").unwrap());
        assert_eq!(all(&conn).unwrap().len(), 1);
    }

    #[test]
    fn formats_like_sqlite() {
        let conn = Connection::open_in_memory().unwrap();
        let now: String = conn
            .query_row("SELECT strftime('%Y-%m-%dT%H:%M:%fZ', 'now')", [], |r| {
                r.get(0)
            })
            .unwrap();
        let parsed: Timestamp = now.parse().unwrap();
        assert_eq!(format(parsed), now);
        assert_eq!(
            format("2026-11-26T12:00:00Z".parse().unwrap()),
            "2026-11-26T12:00:00.000Z"
        );
    }
}
