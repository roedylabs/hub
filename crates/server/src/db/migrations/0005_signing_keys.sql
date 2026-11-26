-- Migration 0005: the hub's Ed25519 signing keys for the identity header and
-- the events (docs/app-contract.md › Identity › Keys). `signing` rotates
-- them: a new key is published 10 minutes before it signs (`active_from`),
-- the previous one stops signing then (`retired_at`) and stays published for
-- 10 more minutes before its row is deleted.
--
-- Timestamps as in 0001: UTC text with milliseconds.

CREATE TABLE signing_keys (
    -- Key ID (ULID), `kid` in the JWS header and the JWKS.
    kid         TEXT PRIMARY KEY,
    -- The 32-byte Ed25519 seed, encrypted with the instance key (`crypto`,
    -- purpose `signing_keys.private`, row = kid).
    private_enc BLOB NOT NULL,
    -- The public key, as published in the JWKS.
    public      BLOB NOT NULL CHECK (length(public) = 32),
    created_at  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    -- From when the key signs; published from `created_at`.
    active_from TEXT NOT NULL,
    -- From when it no longer signs (the next key's `active_from`).
    retired_at  TEXT
) STRICT;
