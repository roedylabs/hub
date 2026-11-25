-- Migration 0001: initial schema of the server base. Taken over from Roedy
-- Mail (`crates/server/src/db/migrations/0001_init.sql` and `0003`) and
-- trimmed to settings, users, sessions and the audit log; the tables for
-- sign-in factors, invitations, apps and events follow in later migrations.
--
-- Timestamps are UTC text (ISO 8601 with milliseconds), so they compare as
-- text. IDs visible to the outside are ULIDs. Secrets are stored only
-- encrypted (`crypto`) or as SHA-256 hashes.

CREATE TABLE schema_migrations (
    version    INTEGER PRIMARY KEY,
    name       TEXT    NOT NULL,
    applied_at TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
) STRICT;

-- Instance settings as JSON per key (setup token, organization, branding, …).
CREATE TABLE app_settings (
    key        TEXT PRIMARY KEY,
    json       TEXT NOT NULL CHECK (json_valid(json)),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
) STRICT;

-- Hub accounts. The ID is the `sub` the apps receive.
CREATE TABLE users (
    id                  TEXT PRIMARY KEY,
    -- Login name next to the email address (lowercase, 2 to 32 characters).
    username            TEXT NOT NULL UNIQUE
                        CHECK (username = lower(username) AND length(username) BETWEEN 2 AND 32),
    -- Unique ignoring case (NOCASE folds ASCII only).
    email               TEXT NOT NULL UNIQUE COLLATE NOCASE CHECK (email <> ''),
    display_name        TEXT NOT NULL,
    role                TEXT NOT NULL CHECK (role IN ('admin', 'user')),
    status              TEXT NOT NULL DEFAULT 'active' CHECK (status IN ('active', 'locked')),
    -- Argon2id PHC string.
    password_hash       TEXT NOT NULL,
    password_changed_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    last_seen_at        TEXT,
    created_at          TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    -- Language of the last sign-in (`de` for German, `en` for every other
    -- language): for mails and texts when the user's language setting is
    -- `system`; empty until the first sign-in.
    sign_in_lang        TEXT CHECK (sign_in_lang IN ('de', 'en'))
) STRICT;

-- Signed-in browsers.
CREATE TABLE sessions (
    id           TEXT PRIMARY KEY,
    user_id      TEXT NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    -- SHA-256 of the cookie token; the token itself is never stored.
    token_hash   BLOB NOT NULL UNIQUE CHECK (length(token_hash) = 32),
    user_agent   TEXT,
    ip           TEXT,
    created_at   TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    last_seen_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    -- Moves with activity (30 days after last_seen_at).
    expires_at   TEXT NOT NULL,
    -- "Stay signed in": 1 = cookie with a lifetime, 0 = cookie until the
    -- browser closes. On the server the session expires in both cases after
    -- 30 days without activity.
    persistent   INTEGER NOT NULL DEFAULT 1 CHECK (persistent IN (0, 1)),
    -- Last full sign-in (password and second factor) in this session: at
    -- sign-in and on re-authentication (`/login?reauth=1`). The apps receive
    -- it as the `auth_time` claim (docs/app-contract.md › Identity).
    auth_time    TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
) STRICT;

CREATE INDEX sessions_user ON sessions (user_id);
CREATE INDEX sessions_expires ON sessions (expires_at);

-- Admin and security actions.
CREATE TABLE audit_log (
    id         INTEGER PRIMARY KEY,
    at         TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    category   TEXT NOT NULL CHECK (category IN ('login', 'users', 'apps', 'settings', 'system')),
    -- Action as a key, e.g. `auth.login`; texts are built when reading.
    action     TEXT NOT NULL,
    -- Who acted; the name stays readable once the user is deleted.
    actor_id   TEXT REFERENCES users (id) ON DELETE SET NULL,
    actor_name TEXT,
    -- Affected object, e.g. the ID of a user or the slug of an app.
    target     TEXT,
    ip         TEXT,
    details    TEXT CHECK (details IS NULL OR json_valid(details))
) STRICT;

CREATE INDEX audit_log_at ON audit_log (at);
