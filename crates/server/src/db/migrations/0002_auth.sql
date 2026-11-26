-- Migration 0002: sign-in factors and per-user settings. Taken over from Roedy
-- Mail (`crates/server/src/db/migrations/0001_init.sql`: `totp_devices`,
-- `recovery_codes`, `user_settings`).

-- Authenticator apps. At most one unconfirmed one per user (a started setup).
CREATE TABLE totp_devices (
    id           TEXT PRIMARY KEY,
    user_id      TEXT NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    -- Name of the app ("Authenticator app 2"), set on confirmation.
    name         TEXT CHECK (name IS NULL OR (name <> '' AND length(name) <= 64)),
    -- Secret, encrypted with purpose and row ID (`crypto`).
    secret_enc   BLOB NOT NULL,
    created_at   TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    confirmed_at TEXT,
    last_used_at TEXT,
    -- Last accepted time step: a code is valid only once (RFC 6238, 5.2);
    -- checked and set in one UPDATE.
    last_step    INTEGER,
    CHECK (confirmed_at IS NULL OR name IS NOT NULL)
) STRICT;

CREATE INDEX totp_devices_user ON totp_devices (user_id);

-- Recovery codes: ten per user, each valid once. Only a hash is stored
-- (`auth::recovery`).
CREATE TABLE recovery_codes (
    id         INTEGER PRIMARY KEY,
    user_id    TEXT NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    code_hash  BLOB NOT NULL CHECK (length(code_hash) = 32),
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    used_at    TEXT,
    UNIQUE (user_id, code_hash)
) STRICT;

-- Language and appearance per user (`settings`).
CREATE TABLE user_settings (
    user_id    TEXT PRIMARY KEY REFERENCES users (id) ON DELETE CASCADE,
    json       TEXT    NOT NULL CHECK (json_valid(json)),
    -- Increases with every save; `PUT settings` names the version the change
    -- is based on and fails when it is no longer current.
    version    INTEGER NOT NULL CHECK (version > 0),
    updated_at TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
) STRICT;
