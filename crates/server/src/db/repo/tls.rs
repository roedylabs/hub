//! Tables `acme_accounts` and `tls_certificates` (`tls::issuer`). Keys are
//! stored encrypted; this module only moves the bytes.

use jiff::Timestamp;
use rusqlite::{Connection, OptionalExtension as _};

use super::signing_keys::format;

/// A stored ACME account.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Account {
    pub directory: String,
    pub key_enc: Vec<u8>,
    pub url: String,
}

pub fn account(conn: &Connection, directory: &str) -> rusqlite::Result<Option<Account>> {
    conn.query_row(
        "SELECT directory, key_enc, url FROM acme_accounts WHERE directory = ?1",
        [directory],
        |row| {
            Ok(Account {
                directory: row.get(0)?,
                key_enc: row.get(1)?,
                url: row.get(2)?,
            })
        },
    )
    .optional()
}

/// Stores the account of a CA, replacing an earlier one.
pub fn save_account(conn: &Connection, account: &Account) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO acme_accounts (directory, key_enc, url) VALUES (?1, ?2, ?3) \
         ON CONFLICT (directory) DO UPDATE SET key_enc = excluded.key_enc, url = excluded.url, \
         created_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')",
        (&account.directory, &account.key_enc, &account.url),
    )?;
    Ok(())
}

/// A stored certificate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Certificate {
    pub domain: String,
    pub key_enc: Vec<u8>,
    pub chain_pem: String,
    pub not_before: Timestamp,
    pub not_after: Timestamp,
    pub directory: String,
}

fn parse(text: &str) -> rusqlite::Result<Timestamp> {
    text.parse().map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
    })
}

pub fn certificate(conn: &Connection, domain: &str) -> rusqlite::Result<Option<Certificate>> {
    conn.query_row(
        "SELECT domain, key_enc, chain_pem, not_before, not_after, directory \
         FROM tls_certificates WHERE domain = ?1",
        [domain],
        |row| {
            Ok(Certificate {
                domain: row.get(0)?,
                key_enc: row.get(1)?,
                chain_pem: row.get(2)?,
                not_before: parse(&row.get::<_, String>(3)?)?,
                not_after: parse(&row.get::<_, String>(4)?)?,
                directory: row.get(5)?,
            })
        },
    )
    .optional()
}

/// Stores the certificate of its domain and deletes those of other domains
/// (the public address changed): the hub only ever serves one name.
pub fn save_certificate(conn: &Connection, cert: &Certificate) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO tls_certificates (domain, key_enc, chain_pem, not_before, not_after, directory) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6) \
         ON CONFLICT (domain) DO UPDATE SET key_enc = excluded.key_enc, \
         chain_pem = excluded.chain_pem, not_before = excluded.not_before, \
         not_after = excluded.not_after, directory = excluded.directory, \
         created_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')",
        (
            &cert.domain,
            &cert.key_enc,
            &cert.chain_pem,
            format(cert.not_before),
            format(cert.not_after),
            &cert.directory,
        ),
    )?;
    conn.execute(
        "DELETE FROM tls_certificates WHERE domain <> ?1",
        [&cert.domain],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::migrations;

    #[test]
    fn stores_and_replaces() {
        let mut conn = Connection::open_in_memory().unwrap();
        migrations::run(&mut conn).unwrap();
        let directory = "https://ca.example.test/directory";
        assert_eq!(account(&conn, directory).unwrap(), None);
        let mut acct = Account {
            directory: directory.into(),
            key_enc: b"sealed".to_vec(),
            url: "https://ca.example.test/acct/1".into(),
        };
        save_account(&conn, &acct).unwrap();
        acct.url = "https://ca.example.test/acct/2".into();
        save_account(&conn, &acct).unwrap();
        assert_eq!(account(&conn, directory).unwrap(), Some(acct));

        let mut cert = Certificate {
            domain: "old.example.test".into(),
            key_enc: b"k".to_vec(),
            chain_pem: "PEM".into(),
            not_before: "2026-11-27T00:00:00Z".parse().unwrap(),
            not_after: "2027-02-25T00:00:00Z".parse().unwrap(),
            directory: directory.into(),
        };
        save_certificate(&conn, &cert).unwrap();
        assert_eq!(
            certificate(&conn, "old.example.test").unwrap(),
            Some(cert.clone())
        );
        cert.domain = "hub.example.test".into();
        save_certificate(&conn, &cert).unwrap();
        cert.chain_pem = "PEM 2".into();
        save_certificate(&conn, &cert).unwrap();
        assert_eq!(certificate(&conn, "old.example.test").unwrap(), None);
        assert_eq!(certificate(&conn, "hub.example.test").unwrap(), Some(cert));
    }
}
