-- Migration 0009: built-in TLS with certificates from an ACME CA
-- (`tls::issuer`, docs/ops.md › TLS). Private keys are encrypted with the
-- instance key (`crypto`) like `signing_keys`; with another instance key
-- they can no longer be opened, and the hub creates a new account and gets a
-- new certificate.
--
-- Timestamps as in 0001: UTC text with milliseconds.

-- One ACME account per CA (directory URL).
CREATE TABLE acme_accounts (
    directory  TEXT PRIMARY KEY,
    -- The account's P-256 key as PKCS#8, encrypted (purpose
    -- `acme_accounts.key`, row = directory).
    key_enc    BLOB NOT NULL,
    -- The account URL (`kid` in the JWS header).
    url        TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
) STRICT;

-- The current certificate per DNS name.
CREATE TABLE tls_certificates (
    domain     TEXT PRIMARY KEY,
    -- The certificate's P-256 key as PKCS#8, encrypted (purpose
    -- `tls_certificates.key`, row = domain).
    key_enc    BLOB NOT NULL,
    -- The chain as the CA sent it (PEM), leaf first.
    chain_pem  TEXT NOT NULL,
    not_before TEXT NOT NULL,
    not_after  TEXT NOT NULL,
    -- The CA that issued it; another `ROEDY_ACME_DIRECTORY` gets a new one.
    directory  TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
) STRICT;
