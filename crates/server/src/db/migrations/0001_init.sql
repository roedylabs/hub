-- Migration 0001: the initial schema. Until the first release it is edited in
-- place; after it, every change is a new migration.
--
-- The tables for settings, accounts, sign-in factors, invitations and the
-- audit log are taken over from Roedy Mail
-- (`crates/server/src/db/migrations/0001_init.sql`) and trimmed to what the
-- hub needs.
--
-- Timestamps are UTC text (ISO 8601 with milliseconds), so they compare as
-- text. IDs visible to the outside are ULIDs. Secrets are stored only
-- encrypted with the instance key (`crypto`, with a purpose and the row as
-- associated data) or as SHA-256 hashes.

-- ---------------------------------------------------------------------------
-- Instance
-- ---------------------------------------------------------------------------

-- One row per applied migration (`db::migrations`).
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

-- The uploaded branding logo (Administration › Branding). The branding itself
-- lives in `app_settings.branding` and names the logo by its ID, a hash of the
-- content.
CREATE TABLE assets (
    id         TEXT PRIMARY KEY,
    mime       TEXT NOT NULL
               CHECK (mime IN ('image/png', 'image/jpeg', 'image/webp', 'image/svg+xml')),
    data       BLOB NOT NULL,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
) STRICT;

-- ---------------------------------------------------------------------------
-- Accounts and sign-in
-- ---------------------------------------------------------------------------

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

-- Language and appearance per user (`settings`).
CREATE TABLE user_settings (
    user_id    TEXT PRIMARY KEY REFERENCES users (id) ON DELETE CASCADE,
    json       TEXT    NOT NULL CHECK (json_valid(json)),
    -- Increases with every save; `PUT settings` names the version the change
    -- is based on and fails when it is no longer current.
    version    INTEGER NOT NULL CHECK (version > 0),
    updated_at TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
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

-- Authenticator apps. At most one unconfirmed one per user (a started setup).
CREATE TABLE totp_devices (
    id           TEXT PRIMARY KEY,
    user_id      TEXT NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    -- Name of the app ("Authenticator app 2"), set on confirmation.
    name         TEXT CHECK (name IS NULL OR (name <> '' AND length(name) <= 64)),
    -- Secret, encrypted (row = ID).
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

-- Deleted users, kept permanently so that apps learn about a deletion they
-- missed (`410 user_deleted`). Only the ID and the time.
CREATE TABLE deleted_users (
    sub        TEXT PRIMARY KEY,
    deleted_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
) STRICT;

-- ---------------------------------------------------------------------------
-- Apps (docs/app-contract.md › Pairing, Access and roles, Internal API)
-- ---------------------------------------------------------------------------

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
    -- The last health check the app passed; empty before.
    online_at      TEXT,
    -- The last request of the app to the internal API with its secret (or its
    -- pairing), at most once a minute; empty before.
    contacted_at   TEXT,
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

-- ---------------------------------------------------------------------------
-- Invitations
-- ---------------------------------------------------------------------------

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

-- ---------------------------------------------------------------------------
-- Signing keys and events (docs/app-contract.md › Identity › Keys, Events)
-- ---------------------------------------------------------------------------

-- The hub's Ed25519 signing keys for the identity header and the events.
-- `signing` rotates them: a new key is published 10 minutes before it signs
-- (`active_from`), the previous one stops signing then (`retired_at`) and
-- stays published for 10 more minutes before its row is deleted.
CREATE TABLE signing_keys (
    -- Key ID (ULID), `kid` in the JWS header and the JWKS.
    kid         TEXT PRIMARY KEY,
    -- The 32-byte Ed25519 seed, encrypted (purpose `signing_keys.private`,
    -- row = kid).
    private_enc BLOB NOT NULL,
    -- The public key, as published in the JWKS.
    public      BLOB NOT NULL CHECK (length(public) = 32),
    created_at  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    -- From when the key signs; published from `created_at`.
    active_from TEXT NOT NULL,
    -- From when it no longer signs (the next key's `active_from`).
    retired_at  TEXT
) STRICT;

-- One event for one app. An event is written in the same transaction as the
-- change it reports (outbox) and delivered by the server's delivery job,
-- strictly in `seq` order per app. Events are kept for 30 days for catching
-- up (`GET /internal/v1/events`), whether delivered or not. `seq` grows by
-- one per event of the app; the counter is `event_delivery.last_seq`, so that
-- it never repeats after old events are deleted.
CREATE TABLE events (
    -- Event ID (ULID), `jti` of the signed event.
    id         TEXT PRIMARY KEY,
    app_id     TEXT NOT NULL REFERENCES apps (id) ON DELETE CASCADE,
    seq        INTEGER NOT NULL CHECK (seq > 0),
    -- E.g. `user.locked` (contract § 10.2).
    type       TEXT NOT NULL,
    -- The event's `data`, a JSON object.
    data       TEXT NOT NULL CHECK (json_valid(data) AND json_type(data) = 'object'),
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    UNIQUE (app_id, seq)
) STRICT;

-- The retention deletes by age.
CREATE INDEX events_created ON events (created_at);

-- Per app: the last `seq` given out and how far delivery got. A row exists
-- from the app's first event on.
CREATE TABLE event_delivery (
    app_id        TEXT PRIMARY KEY REFERENCES apps (id) ON DELETE CASCADE,
    -- The highest `seq` given out (0: none yet).
    last_seq      INTEGER NOT NULL DEFAULT 0 CHECK (last_seq >= 0),
    -- The highest `seq` the app acknowledged with `204`, or read with a
    -- catch-up after it.
    delivered_seq INTEGER NOT NULL DEFAULT 0 CHECK (delivered_seq BETWEEN 0 AND last_seq),
    -- Failed attempts at the next event since the last success; 0 when
    -- nothing failed.
    attempts      INTEGER NOT NULL DEFAULT 0 CHECK (attempts >= 0),
    -- When the next attempt is due after a failure. Empty with failures:
    -- the hub gave up after 7 days and waits until the app is back.
    retry_at      TEXT,
    -- The first failure of this series.
    failing_since TEXT,
    -- What went wrong last, for the administration and the log.
    last_error    TEXT,
    CHECK ((attempts = 0) = (failing_since IS NULL)),
    CHECK (attempts > 0 OR retry_at IS NULL)
) STRICT, WITHOUT ROWID;

-- ---------------------------------------------------------------------------
-- TLS (`tls::issuer`, docs/ops.md › TLS)
-- ---------------------------------------------------------------------------
--
-- Private keys are encrypted like `signing_keys`; with another instance key
-- they can no longer be opened, and the hub creates a new account and gets a
-- new certificate.

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

-- ---------------------------------------------------------------------------
-- Stalwart servers and users' accounts (docs/plan.md § 9.3), taken over from
-- Roedy Mail's `servers` and `mail_accounts`
-- ---------------------------------------------------------------------------

-- The registry (Administration › Stalwart servers). Whether users may connect
-- accounts on other servers is `app_settings.stalwart_policy`.
CREATE TABLE stalwart_servers (
    id                      TEXT PRIMARY KEY,
    label                   TEXT NOT NULL CHECK (label <> '' AND length(label) <= 100),
    -- Only scheme, host and port (`https://mail.example.com`), no trailing `/`;
    -- checked by api::stalwart. One server per address.
    base_url                TEXT NOT NULL UNIQUE,
    -- Domains of the email addresses the server serves (lowercase, without
    -- `@`); each domain belongs to at most one server (checked by
    -- api::stalwart).
    domains_json            TEXT NOT NULL DEFAULT '[]'
                            CHECK (json_valid(domains_json) AND json_type(domains_json) = 'array'),
    -- The hub's OAuth client in Stalwart (created by hand or via DCR,
    -- RFC 7591).
    oauth_client_id         TEXT CHECK (oauth_client_id IS NULL OR oauth_client_id <> ''),
    -- Confidential clients only; encrypted (purpose
    -- `stalwart_servers.oauth_client_secret`, row = ID), never sent out.
    oauth_client_secret_enc BLOB CHECK (oauth_client_secret_enc IS NULL OR oauth_client_id IS NOT NULL),
    -- Last diagnosis (stalwart::probe): ok, err or unk (not checked).
    reach                   TEXT NOT NULL DEFAULT 'unk' CHECK (reach IN ('ok', 'err', 'unk')),
    cors                    TEXT NOT NULL DEFAULT 'unk' CHECK (cors IN ('ok', 'err', 'unk')),
    oauth                   TEXT NOT NULL DEFAULT 'unk' CHECK (oauth IN ('ok', 'err', 'unk')),
    tested_at               TEXT,
    created_at              TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
) STRICT;

-- Users' Stalwart accounts, shared by every app.
CREATE TABLE stalwart_accounts (
    id                TEXT PRIMARY KEY,
    user_id           TEXT NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    -- Origin of the server like `stalwart_servers.base_url`. The account is
    -- linked to the registry through this address, not through the server's
    -- ID: the tokens belong to exactly this server.
    base_url          TEXT NOT NULL,
    -- Address of the signed-in account (lowercase), as the session names it.
    email             TEXT NOT NULL CHECK (email <> ''),
    -- Primary mail account and username of the JMAP session: by these the
    -- hub recognises the same account again (duplicates, signing in again).
    account_id        TEXT NOT NULL CHECK (account_id <> ''),
    username          TEXT NOT NULL,
    -- `oauth`: sign-in at the server, the hub exchanged the code (refresh
    -- token); `token`: the user's access token (e.g. an API key).
    auth_kind         TEXT NOT NULL CHECK (auth_kind IN ('oauth', 'token')),
    -- OAuth client the tokens were issued for (needed for the refresh).
    client_id         TEXT CHECK ((auth_kind = 'oauth') = (client_id IS NOT NULL)),
    -- Refresh token (`oauth`) or access token (`token`), encrypted (purpose
    -- `stalwart_accounts.secret`, row = ID); never leaves the hub for `oauth`.
    secret_enc        BLOB NOT NULL,
    -- Last access token (`oauth`, purpose `stalwart_accounts.access_token`)
    -- and when it expires.
    access_token_enc  BLOB CHECK (access_token_enc IS NULL OR auth_kind = 'oauth'),
    access_expires_at TEXT,
    -- The JMAP session at the last sign-in and where it is.
    session_url       TEXT NOT NULL CHECK (session_url <> ''),
    session_json      TEXT NOT NULL CHECK (json_valid(session_json)),
    -- `needs_sign_in`: the refresh was rejected ("Sign in again").
    state             TEXT NOT NULL DEFAULT 'ok' CHECK (state IN ('ok', 'needs_sign_in')),
    -- Index into the account colours (`MA_COLORS` of @roedy/tokens).
    color             INTEGER NOT NULL CHECK (color BETWEEN 0 AND 5),
    -- The user's default account: exactly one while they have any.
    is_default        INTEGER NOT NULL DEFAULT 0 CHECK (is_default IN (0, 1)),
    -- Storage as of the last check.
    last_check_at     TEXT,
    quota_used        INTEGER,
    quota_total       INTEGER,
    created_at        TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    -- When a browser last fetched an access token for the account: the
    -- early refresh renews active accounts before their token expires.
    token_used_at     TEXT,
    -- Last attempt of the job `quota_check`, even if it failed: so it asks a
    -- broken server again only the next day, not every hour.
    quota_job_at      TEXT,
    UNIQUE (user_id, base_url, account_id)
) STRICT;

CREATE INDEX stalwart_accounts_base_url ON stalwart_accounts (base_url);
CREATE UNIQUE INDEX stalwart_accounts_default ON stalwart_accounts (user_id) WHERE is_default = 1;

-- Sign-ins at a Stalwart server under way: the hub sent the browser to the
-- server's sign-in and waits for the code at /oauth/callback. Valid for 10
-- minutes and once, bound to the browser's session.
CREATE TABLE stalwart_sign_ins (
    -- SHA-256 of the `state` parameter (hex); the browser has the value.
    state_hash  TEXT PRIMARY KEY,
    user_id     TEXT NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    session_id  TEXT NOT NULL REFERENCES sessions (id) ON DELETE CASCADE,
    server_id   TEXT NOT NULL REFERENCES stalwart_servers (id) ON DELETE CASCADE,
    -- Address and client at the start: if the administrator changes either,
    -- the sign-in no longer applies.
    base_url    TEXT NOT NULL,
    client_id   TEXT NOT NULL,
    -- "Sign in again" for this account; NULL for a new one.
    account_id  TEXT REFERENCES stalwart_accounts (id) ON DELETE CASCADE,
    -- PKCE verifier, encrypted (purpose `stalwart_sign_ins.verifier`, row =
    -- state_hash).
    verifier_enc BLOB NOT NULL,
    -- The address entered (`login_hint`).
    login_hint  TEXT NOT NULL,
    -- Path on the hub's origin to return to afterwards.
    return_to   TEXT NOT NULL,
    expires_at  TEXT NOT NULL,
    created_at  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
) STRICT;

CREATE INDEX stalwart_sign_ins_user ON stalwart_sign_ins (user_id);

-- ---------------------------------------------------------------------------
-- Audit log
-- ---------------------------------------------------------------------------

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
