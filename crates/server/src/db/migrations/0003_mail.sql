-- Migration 0003: what the system mails need. Taken over from Roedy Mail
-- (`crates/server/src/db/migrations/0001_init.sql`: `known_devices`,
-- `password_resets`, `email_changes`, `invitations`) with the same columns;
-- the app access chosen in the invitation form follows with milestone 5.

-- Devices a user has signed in from (`auth::device::Device::hash`): a
-- sign-in from another one sends "New sign-in".
CREATE TABLE known_devices (
    user_id       TEXT NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    device_hash   BLOB NOT NULL CHECK (length(device_hash) = 32),
    first_seen_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    PRIMARY KEY (user_id, device_hash)
) STRICT, WITHOUT ROWID;

-- Links from "Forgot password": 60 minutes, once. Only the SHA-256 of the
-- token is stored.
CREATE TABLE password_resets (
    id         TEXT PRIMARY KEY,
    user_id    TEXT NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    token_hash BLOB NOT NULL UNIQUE CHECK (length(token_hash) = 32),
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    expires_at TEXT NOT NULL,
    used_at    TEXT
) STRICT;

CREATE INDEX password_resets_user ON password_resets (user_id);

-- Changes of the own email address until they are confirmed (24 hours).
CREATE TABLE email_changes (
    id           TEXT PRIMARY KEY,
    user_id      TEXT NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    token_hash   BLOB NOT NULL UNIQUE CHECK (length(token_hash) = 32),
    new_email    TEXT NOT NULL COLLATE NOCASE CHECK (new_email <> ''),
    created_at   TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    expires_at   TEXT NOT NULL,
    -- Both mails sent; until then the link is only reserved.
    sent_at      TEXT,
    confirmed_at TEXT
) STRICT;

CREATE INDEX email_changes_user ON email_changes (user_id);

CREATE TABLE invitations (
    id              TEXT PRIMARY KEY,
    -- Unique ignoring case like `users.email`; the address counts as taken
    -- while the invitation exists.
    email           TEXT NOT NULL UNIQUE COLLATE NOCASE CHECK (email <> ''),
    -- Name from the form ("Last name, First name").
    name            TEXT CHECK (name IS NULL OR name <> ''),
    role            TEXT NOT NULL CHECK (role IN ('admin', 'user')),
    -- SHA-256 of the token for lookups, plus the token encrypted (row = ID)
    -- so that "Copy link" can show it.
    token_hash      BLOB NOT NULL UNIQUE CHECK (length(token_hash) = 32),
    token_enc       BLOB NOT NULL,
    -- Who invited or last resent; the invitation page names them.
    invited_by      TEXT REFERENCES users (id) ON DELETE SET NULL,
    created_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    -- Seven days from creation or from the last successful send.
    expires_at      TEXT NOT NULL,
    -- Last successful send; empty = "Not sent".
    mail_sent_at    TEXT,
    -- Authenticator app secret from the invitation's "Multi-factor" step,
    -- encrypted; accepting turns it into the first app.
    totp_secret_enc BLOB,
    -- Last "Request new invitation" (at most once per 24 hours).
    requested_at    TEXT
) STRICT;
