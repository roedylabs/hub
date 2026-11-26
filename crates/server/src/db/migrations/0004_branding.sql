-- Migration 0004: the uploaded branding logo (Administration › Branding).
-- Taken over from Roedy Mail (`crates/server/src/db/migrations/0001_init.sql`,
-- table `assets`). The branding itself lives in `app_settings.branding` and
-- names the logo by its ID, a hash of the content.

CREATE TABLE assets (
    id         TEXT PRIMARY KEY,
    mime       TEXT NOT NULL
               CHECK (mime IN ('image/png', 'image/jpeg', 'image/webp', 'image/svg+xml')),
    data       BLOB NOT NULL,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
) STRICT;
