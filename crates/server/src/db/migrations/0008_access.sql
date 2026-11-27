-- Migration 0008: the app access chosen in an invitation and when an app
-- was last heard of (docs/plan.md › Data model, docs/app-contract.md ›
-- Access and roles).
--
-- Timestamps as in 0001: UTC text with milliseconds.

-- Access chosen in the invitation form, per app: a role from the app's
-- manifest, or NULL for "no access". An app without a row gets its default
-- when the invitation is accepted ("everyone": the default role; "selected
-- only": none). Accepting, revoking or removing the app deletes the rows.
CREATE TABLE invitation_access (
    invitation_id TEXT NOT NULL REFERENCES invitations (id) ON DELETE CASCADE,
    app_id        TEXT NOT NULL REFERENCES apps (id) ON DELETE CASCADE,
    app_role      TEXT CHECK (app_role IS NULL OR app_role <> ''),
    PRIMARY KEY (invitation_id, app_id)
) STRICT, WITHOUT ROWID;

-- Removing an app finds its rows.
CREATE INDEX invitation_access_app ON invitation_access (app_id);

-- The last request of the app to the internal API with its secret (or its
-- pairing), at most once a minute; empty before.
ALTER TABLE apps ADD COLUMN contacted_at TEXT;
-- The last health check the app passed; empty before.
ALTER TABLE apps ADD COLUMN online_at TEXT;
