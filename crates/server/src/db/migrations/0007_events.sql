-- Migration 0007: the events for the apps and the state of their delivery
-- (docs/plan.md › Keys and events, docs/app-contract.md › Events).
--
-- An event is written in the same transaction as the change it reports
-- (outbox) and delivered by the server's delivery job, strictly in `seq`
-- order per app. Events are kept for 30 days for catching up
-- (`GET /internal/v1/events`), whether delivered or not.
--
-- Timestamps as in 0001: UTC text with milliseconds.

-- One event for one app. `seq` grows by one per event of the app; the
-- counter is `event_delivery.last_seq`, so that it never repeats after old
-- events are deleted.
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
