# Roedy Hub – implementation plan

> **Status: agreed on 2026-11-25.** Companion to [`app-contract.md`](app-contract.md)
> and the roadmap of the repository; finished milestones are ticked off here.

## 1. What gets built

One binary `roedy-hub` (Rust, axum, SQLite in WAL mode, embedded SvelteKit SPA,
static musl build, image on `scratch`, user 65532) that is three things at the
root of one origin:

```
Browser ──HTTPS──► TLS proxy ──► roedy-hub :8080 ─┬─ /, /login, /account, /admin, /setup, /invite, /api/v1  (hub)
                                                  ├─ /mail/…      ──► roedy-mail:8080      (+ X-Roedy-Identity)
                                                  └─ /contacts/…  ──► roedy-contacts:8080
                     roedy-hub :8081 (internal, not published) ◄── pairing, JWKS, manifest, events, users
                     roedy-hub ──► POST /<slug>/_roedy/v1/events (signed events to the apps)
```

Everything an app needs is in `docs/app-contract.md`; this file covers how the
hub itself is built.

## 2. Taken over from Roedy Mail

Copied and adapted from `../mail` (same project, same licence), with a note of
origin in each file's module comment:

- **Workspace and tools:** Cargo workspace with the same lints and profiles,
  pnpm workspace, mise, lefthook hooks (Conventional Commits, no push to
  `main`), ESLint/Prettier config, justfile, cliff.toml, deny.toml,
  `.github/` (CI, CodeQL, PR title check, Scorecard, Dependabot with 7-day
  cooldown, pinned actions and images and their tests), release workflow with
  cosign, SBOM and provenance.
- **Server:** `config` (same `ROEDY_*` names and meaning, plus
  `ROEDY_INTERNAL_LISTEN`), logging, `db` with migrations and the `Db` thread,
  secret key (`ROEDY_SECRET_KEY[_FILE]`, `secret.key`), `client_ip` and
  `ROEDY_TRUST_PROXY`, `rate_limit`, `setup_token`, `healthcheck`,
  `security_headers`, `static_files` (embedded SPA, inline script hashes),
  graceful shutdown, `Background` tasks.
- **Auth:** Argon2id hasher with placeholder hash, sessions (opaque token,
  SHA-256 in the DB, 30 days idle, "Stay signed in"), multi-step sign-in
  challenge, TOTP with replay protection, recovery codes, known devices,
  username and email rules, CSRF (`X-Requested-With` + Fetch Metadata).
- **API modules:** setup, auth, me, password, mfa, sessions, users, invites,
  email change, audit (with texts), branding/organization, public config.
- **Web:** SvelteKit SPA set-up, Paraglide, `#lib/…` imports, icon registry,
  design tokens (copy of `packages/tokens` with a note of origin), the auth
  layout, setup wizard, sign-in, invitation, account and admin pages, axe
  audit script.
- **Mail templates:** askama templates for invitation, password reset, email
  change and the security notices that go with them (password changed, email
  changed, new sign-in), with the snapshot tests.

Not taken over: the token broker, mail servers, tags, notifications (bell,
push, digests), backups in the app, update check, maintenance mode, GeoIP.
Each can follow later; v1 stays small.

## 3. Repository layout

| Path | Content |
|---|---|
| `crates/server` | `roedy-hub-server`, binary `roedy-hub` |
| `crates/contract` | `roedy-hub-contract`: manifest, claims, events, Ed25519 JWS signing and verification, test vectors. Used by the server and the example app. |
| `crates/example-app` | `roedy-hub-example`: the test app that implements the whole contract. Not part of the image. |
| `apps/web` | `@roedy/hub-web`: SvelteKit SPA (home page, sign-in, setup, account, administration) |
| `packages/tokens` | Copy of Roedy Mail's design tokens |
| `docs/` | `app-contract.md`, `ops.md`, `plan.md`, setup guides |
| `deploy/` | `compose.yml`, `.env.example`, `Caddyfile` |
| `scripts/` | Hooks, E2E runner, image smoke test, pin tests |

Commit scopes: `server`, `gateway`, `contract`, `example`, `web`, `tokens`,
`ci`, `deploy`, `docs`, `deps`, `repo`, `release`.

## 4. Server design

### Listeners

- **Public** (`ROEDY_LISTEN`, `0.0.0.0:8080`): hub SPA, `/api/v1`, `/healthz`,
  `/readyz`, `/.well-known/change-password` (→ `/account/security`), and the
  gateway for every other first path segment that is a known slug.
- **Internal** (`ROEDY_INTERNAL_LISTEN`, `0.0.0.0:8081`): `/internal/v1/…`
  only. Empty value turns it off (then no app can pair).

### Gateway

- Router: first path segment → slug → app (in-memory table, reloaded on every
  change). Reserved names are checked when an app is created and again here.
- Session check per request with a short in-memory cache (10 s) so that asset
  requests do not each cost a database round trip; revocation clears the
  cache entry immediately.
- Identity: Ed25519 signature with `ring` (already in Roedy Mail's tree, no new
  crypto dependency); a token per (session, app) is cached for up to 30 s of
  its 60 s lifetime.
- HTTP client: `hyper-util`'s pooled client over HTTP/1.1 with streaming
  bodies; WebSocket via hyper's upgrade mechanism and
  `tokio::io::copy_bidirectional` (no WebSocket library needed, the hub does
  not parse frames).
- Open streams and WebSockets are registered per session ID; revoking a session
  cancels them.
- Header rules exactly as in contract § 3.2 and § 3.3, implemented as pure
  functions with table-driven tests.

### Keys and events

- Signing keys in `signing_keys`, private part encrypted with the instance key.
  A daily job rotates every 90 days (new key published 10 min before use, old
  one kept 10 min after).
- Events in `events` (per app `seq`), a delivery job per app with backoff as in
  contract § 10.1, retention 30 days.
- Health poll of every paired app every 30 s (`GET /<slug>/readyz`).

### Data model

Migration 0001 (server base) holds `schema_migrations`, `app_settings`,
`users`, `sessions` and `audit_log`; the other tables follow in later
migrations with the milestones that use them.

From Mail, trimmed: `schema_migrations`, `app_settings`, `users`, `sessions`
(plus `auth_time`), `audit_log` (category `apps` added), `totp_devices`,
`recovery_codes`, `known_devices`, `password_resets`, `email_changes`,
`invitations` (plus the app access chosen in the form), `user_settings`
(language, theme, accent), `assets` (logo).

New:

| Table | Content |
|---|---|
| `apps` | `id`, `slug` (unique), `internal_url`, `state` (`pending`, `paired`), `manifest_json`, `manifest_id`, `version`, `secret_hash`, `access_default` (`all`, `selected`), `position`, `health`, `health_at`, `paired_at`, `created_at` |
| `pairing_codes` | `app_id` (one open code per app), `code_hash`, `expires_at`, `created_by` |
| `app_access` | `user_id`, `app_id`, `app_role`; primary key (user, app) |
| `signing_keys` | `kid`, `private_enc`, `public`, `created_at`, `active_from`, `retired_at` |
| `events` | `id`, `app_id`, `seq`, `type`, `data`, `created_at`; unique (app, seq) |
| `deleted_users` | `sub`, `deleted_at`: tombstones for `GET /internal/v1/users/<sub>`, kept permanently |
| `event_delivery` | `app_id`, `delivered_seq`, `attempts`, `retry_at`, `last_error` |

### New dependencies (to be justified in the PRs)

| Crate | Why |
|---|---|
| `hyper`, `hyper-util` (client, http1), `http-body-util` | Proxy client with streaming and upgrades; `hyper` already comes with axum |
| `roedy-jmap` (copy of Mail's `crates/jmap`), `reqwest` (rustls without provider), `rustls` (ring) | System mails go out over JMAP from a sending account, as in Mail (decided on 2026-11-26) |
| `askama`, `jiff` (embedded time zone database) | Same as Mail: mail templates, times in the recipient's time zone |
| `argon2`, `totp-rs`, `icu_properties` | Same as Mail: password hashes, authenticator apps, letters in display names (`icu_properties` already comes with `url`) |
| `ulid`, `ring`, `usvg` | Same as Mail: IDs, Ed25519, sanitizing the app icons |
| `resvg`, `roxmltree`, `imagesize`, `zune-jpeg`, `image-webp`, `skrifa` | Same as Mail: drawing the branding logo for mails and app icons, checking uploaded logos (decided on 2026-11-26) |

No JWT library: the JWS is ~150 lines on top of `ring`, `base64` and
`serde_json`, which keeps `alg` handling strict.

## 5. Web app

Routes: `/` (home page), `/login` (+ `forgot`, `reset/[token]`), `/setup`,
`/invite/[token]`, `/account` (`profile`, `security`, `appearance`,
`language`, `confirm-email/[token]`), `/admin` (`users`, `apps`,
`apps/[slug]`, `audit-log`, `branding`, `organization`, `email`), 403 and
error pages for the gateway. SvelteKit's asset directory is `/_hub/`.

Design: sign-in, setup and invitation follow the new "Roedy Mail" design
(`.design-live/Sign-in`, `Setup`, `Invitation` in `../mail`); account and
administration the current Roedy Mail app. The home page has no design yet; it
is built plainly with the tokens first and finished once there is a draft.

## 6. Milestones

Each milestone is one PR (milestone 3 possibly two: server API, then pages). All
checks pass in every PR; every PR has tests for what it adds.

1. [x] **Scaffold** – workspace, tools, hooks, CI, `CLAUDE.md`, `CONTRIBUTING.md`,
   `README.md`, `LICENSE`, `SECURITY.md`, the approved contract; empty
   `roedy-hub --version`, SPA skeleton with tokens.
   *Tests:* pin tests, commit-msg hook, a trivial unit test per package.
2. [x] **Server base** – config with both listeners, logging, database and
   migration 0001 (users, sessions, settings, audit), secret key, health,
   `healthcheck`, embedded SPA with security headers and CSP, setup token,
   rate limiter, client IP.
   *Tests:* config parsing (every variable, error messages), migrations,
   `/healthz` and `/readyz`, headers, SPA fallback, trust proxy.
3. **Auth and users** – setup wizard with mandatory TOTP, sign-in with
   password, TOTP and recovery codes, "Stay signed in", `?next=` validation,
   account pages, signed-in devices, invitations, password reset, email
   change, system mails, administration of users, audit log, branding and
   organization.
   *Tests:* API integration tests per flow (as in Mail), rate limits, CSRF,
   `next` open-redirect cases, mail snapshots, component tests, axe.
   Split into four PRs (decided on 2026-11-26):
   1. [x] **Auth core (API)** – migration 0002 (authenticator apps, recovery
      codes, user settings), setup with mandatory TOTP, sign-in with
      password, TOTP and recovery codes, a fresh sign-in in the same session
      (`auth_time`, the session ID stays), sign-out with `Clear-Site-Data`,
      `GET /api/v1/session`, profile, password change, authenticator apps,
      signed-in devices, settings, deleting the own account.
   2. [x] **System mails (API)** – JMAP sending account, templates and
      snapshots, invitations, password reset, email change, security notices
      with known devices. Done: `crates/jmap` (`roedy-jmap`, Mail's client
      trimmed to sending), `mailer` (sending account in
      `app_settings.system_mail` with the JMAP server's address instead of
      Mail's registry, failure limits, `mail_failed`), askama templates in
      `crates/server/templates/{en,de}` with snapshots, `notices` (Mail's
      `events::security` and outbox without notifications in the app),
      migration 0003 (`known_devices`, `password_resets`, `email_changes`,
      `invitations`), `admin/system-mail` (get, set, test), `invites`
      (list, create, resend, copy link, revoke; accept with password and an
      optional authenticator app), `auth/password/forgot` and
      `auth/password/reset/:token`, the email change in two steps in
      `PATCH auth/me` with `auth/email/check|confirm`, resend and discard.
      Mails use the default time zone (Europe/Berlin) until the settings
      offer one; their links point to `/login/forgot`, `/login/reset/…`,
      `/invite/…` and `/account/confirm-email/…`. The advice for the sole
      administrator sends them to the server operator; it can name
      `roedy-hub reset-admin` once that command exists (administration).
   3. [x] **Administration (API)** – users (lock, roles, reset the second
      factor), audit log with texts, branding and organization, cleanup of
      expired sessions. Done: `users` (list with the invitations, details,
      role, lock and unlock – ends sessions, links from mails and pending
      sign-ins –, reset the second factor with the notice
      `security_mfa_reset`, remove, bulk actions; never the own account,
      `last_admin` for the sole active administrator), `invites` (role,
      "Extend", "Request a new invitation" without notifications: it shows
      as `requested_at`), `admin/audit` (pages, categories, search, counts,
      CSV export with a limit, retention 30/90/365 days, texts in en and de
      for every key in `api::audit::texts`), `admin/org` and
      `admin/branding` (`instance`: the organization's name is `hub.name`,
      the issuer of authenticator apps, the name in mails and the logo's
      initial; Mail's logo with kind, letter, icon, upload, fit, plate,
      tilt, accent colour and `userAccent`, drawn by `logo` with `resvg` for
      the mail header and the app icons, uploads checked and SVGs
      sanitized as in Mail, stored in `assets` – migration 0004),
      `public/config`, the public branding files (`/branding/…`,
      `/manifest.webmanifest`), `roedy-hub reset-admin`
      (`--link` prints a reset link instead of a password; the sole-admin
      advice in mails names it again) and the jobs `session_cleanup`
      (hourly) and `audit_retention` (daily) on Mail's small scheduler. Not
      taken over: banner, sign-in texts and the push icon of Mail's
      branding.
   4. [ ] **Pages** – setup, sign-in (with `?next=` and `reauth=1`),
      invitation, account and administration in the web app.
4. **Gateway, pairing, identity** – `crates/contract`, signing keys and
   rotation, the proxy, internal listener, pairing, manifest, events with
   delivery and catch-up, users endpoint, health poll, the example app.
   *Tests:* proxy integration against the example app – path and query
   unchanged, SSE without buffering (first event arrives before the stream
   ends), WebSocket echo and close, large upload and download streamed
   (memory stays flat), timeouts → 502/504; header cleaning (forged
   `X-Roedy-*`, hub cookie, foreign cookies, `Set-Cookie` outside the
   namespace, `Clear-Site-Data`, `Service-Worker-Allowed`, `Forwarded`,
   `X-Forwarded-*`); identity (forged signature, `alg: none`, wrong `aud`,
   expired, `iat` in the future, unknown `kid` → JWKS refresh, key rotation);
   pairing abuse (wrong, expired, reused code, rate limit, wrong
   `public_url`, oversized or malicious SVG, invalid manifest); events
   (order, retry, idempotency, catch-up, 410); revoked session closes streams;
   test vectors from `crates/contract`.
5. **Home page and permissions** – home page tiles, browser API
   (`/api/v1/session`, `/api/v1/apps`, icons), administration of apps (add,
   pairing code, status, order, access default, remove), access and roles per
   user and in the invitation form, 403 page.
   *Tests:* permission matrix (admin/user × access yes/no × app
   pending/paired), `access.changed` events, component tests, axe.
6. **Delivery, docs, end-to-end** – Dockerfile, release workflow, image size
   and smoke test, `deploy/compose.yml` (Caddy + hub + Mail, only Caddy
   published; the example app in a test override), `.env.example`,
   `docs/ops.md`, README; Playwright suite with axe against hub + example app
   (setup, sign-in, pairing, opening the app, SSE and WebSocket through the
   browser, sign-out, no access, light and dark, desktop and mobile).
   *Budgets checked in CI:* image ≤ 25 MB, idle ≤ 40 MB.

## 7. Open points (not in this assignment)

- **Shared design tokens:** move `packages/tokens` into a package both repos
  use (npm package from a `roedylabs/design` repository, or a Git
  dependency). Until then the copy carries its origin and version.
- **Moving Mail's users into the hub** – proposal: once Mail is paired, it
  offers "Move users to the hub" in its administration. It calls a new
  internal endpoint `POST /internal/v1/import/users` with username, email,
  name, role, status, the Argon2id hash and the TOTP secrets of each user (over
  the private network, secret-authenticated). The hub shows the request to an
  administrator, who approves it; then the hub creates the users (or matches
  existing ones by email), grants them access to Mail with the matching role
  and returns the mapping from Mail's user IDs to `sub`. Mail stores the
  mapping, so that existing data stays with the right person. Passwords and
  second factors keep working; sessions start anew.
- **Shared Stalwart accounts and tokens** for all apps – see contract § 14.
- **Ready-made app switcher** as a small script/web component served by the
  hub, so that each app does not build its own.
- **Notifications, web push, backups in the app, update check, maintenance
  mode, GeoIP** for the hub.
- **OIDC/SAML, groups** – not in v1.

## 8. Additions to the brief (agreed on 2026-11-25)

These go beyond the original brief and are part of the contract:

1. **Cookie isolation:** the hub forwards only cookies named
   `__Host-roedy-<slug>_…` to an app and drops `Set-Cookie` for other names.
2. **Response headers:** the hub removes `Clear-Site-Data` and
   `Service-Worker-Allowed` from app responses.
3. **Extra claims** `locale` and `auth_time`, plus explicit `typ` values
   (`roedy-id+jwt`, `roedy-event+jwt`) so that an event can never pass as an
   identity.
4. **Re-authentication** via `/login?reauth=1&next=…` for sensitive actions.
5. **Event details:** per-app `seq`, ordered delivery, catch-up endpoint;
   extra types `user.unlocked` and `access.changed`.
6. **Users endpoint** `GET /internal/v1/users[/<sub>]` for background work and
   directories.
7. **Reserved app path** `/<slug>/_roedy/` for calls from the hub, never
   forwarded from outside.
8. **Pending apps** are reachable only for hub administrators (without
   identity), so that the app's own setup can be finished through the hub.
9. **Sign-out** sends `Clear-Site-Data: "cache", "storage"` for the whole
   origin.
10. **`X-Forwarded-For`:** trusted entries plus the peer; the app's
    `ROEDY_TRUST_PROXY` is the hub's plus one.
11. **Revoked sessions** lose their open streams and WebSockets within 5 s.
12. **Defaults after pairing:** hub administrators get the app's `admin`
    role.
13. **Shared preferences** `localStorage["roedy.hub.prefs"]` (theme, language)
    that apps may read to avoid a flash before their first request.
14. **Additional reserved slugs** (`apps`, `auth`, `branding`, `error`,
    `help`, `hub`, `logout`, `oauth`, `settings`, `static`, `_hub`,
    `favicon.ico`, `robots.txt`).
