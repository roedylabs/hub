-- Migration 0006: apps, their pairing codes, access per user and the
-- tombstones of deleted users (docs/plan.md › Data model,
-- docs/app-contract.md › Pairing, Access and roles, Internal API).
--
-- Timestamps as in 0001: UTC text with milliseconds. Secrets only as SHA-256.

-- Apps behind the gateway. An app is `pending` from "Add" until it pairs
-- for the first time, then `paired`; pairing again keeps it `paired`.
CREATE TABLE apps (
    id             TEXT PRIMARY KEY,
    -- Path prefix, `^[a-z][a-z0-9-]{0,30}[a-z0-9]$`, not reserved (checked
    -- by the server).
    slug           TEXT NOT NULL UNIQUE CHECK (length(slug) BETWEEN 2 AND 32),
    -- Scheme, host and port at which the hub reaches the app, no path.
    internal_url   TEXT NOT NULL,
    state          TEXT NOT NULL DEFAULT 'pending' CHECK (state IN ('pending', 'paired')),
    -- The last manifest, with the icon as the hub sanitized it.
    manifest_json  TEXT CHECK (manifest_json IS NULL OR json_valid(manifest_json)),
    -- `id` and `version` of that manifest.
    manifest_id    TEXT,
    version        TEXT,
    -- SHA-256 of the app secret; the secret itself is never stored.
    secret_hash    BLOB UNIQUE CHECK (secret_hash IS NULL OR length(secret_hash) = 32),
    -- New users get access with the default role (`all`) or none (`selected`).
    access_default TEXT NOT NULL DEFAULT 'all' CHECK (access_default IN ('all', 'selected')),
    -- Order on the home page.
    position       INTEGER NOT NULL,
    -- Result of the last health check (`GET /<slug>/readyz`), empty before it.
    health         TEXT CHECK (health IN ('online', 'offline')),
    health_at      TEXT,
    paired_at      TEXT,
    created_at     TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    -- A paired app has a manifest, a secret and the time of pairing; a
    -- pending one none of them.
    CHECK ((state = 'paired' AND manifest_json IS NOT NULL AND secret_hash IS NOT NULL
            AND paired_at IS NOT NULL)
           OR (state = 'pending' AND manifest_json IS NULL AND secret_hash IS NULL
               AND paired_at IS NULL)),
    -- `manifest_id` and `version` come with the manifest.
    CHECK ((manifest_json IS NULL) = (manifest_id IS NULL)
           AND (manifest_json IS NULL) = (version IS NULL))
) STRICT;

-- The open pairing code of an app: at most one, a new one replaces it.
CREATE TABLE pairing_codes (
    app_id     TEXT PRIMARY KEY REFERENCES apps (id) ON DELETE CASCADE,
    -- SHA-256 of the canonical code (`rhp_` and 32 uppercase characters).
    code_hash  BLOB NOT NULL UNIQUE CHECK (length(code_hash) = 32),
    expires_at TEXT NOT NULL,
    created_by TEXT REFERENCES users (id) ON DELETE SET NULL,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
) STRICT;

-- Access to an app with a role from its manifest; no row, no access.
CREATE TABLE app_access (
    user_id  TEXT NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    app_id   TEXT NOT NULL REFERENCES apps (id) ON DELETE CASCADE,
    app_role TEXT NOT NULL,
    PRIMARY KEY (user_id, app_id)
) STRICT, WITHOUT ROWID;

-- `GET /internal/v1/users` pages through one app's users by `sub`.
CREATE INDEX app_access_app ON app_access (app_id, user_id);

-- Deleted users, kept permanently so that apps learn about a deletion they
-- missed (`410 user_deleted`). Only the ID and the time.
CREATE TABLE deleted_users (
    sub        TEXT PRIMARY KEY,
    deleted_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
) STRICT;
