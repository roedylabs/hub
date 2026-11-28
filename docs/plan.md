# Roedy Hub – implementation plan

> **Status: agreed on 2026-11-25; platform direction (§ 9) agreed on
> 2026-11-28.** Companion to [`app-contract.md`](app-contract.md) and the
> roadmap of the repository; finished milestones are ticked off here.

## 1. What gets built

One binary `roedy-hub` (Rust, axum, SQLite in WAL mode, embedded SvelteKit SPA,
static musl build, image on `scratch`, user 65532) that is three things at the
root of one origin:

```
Browser ──HTTPS──► roedy-hub :8443 (TLS, ACME) ─┬─ /, /login, /account, /admin, /setup, /invite, /api/v1  (hub)
                                                ├─ /mail/…      ──► roedy-mail:8080      (+ X-Roedy-Identity)
                                                └─ /contacts/…  ──► roedy-contacts:8080
        ──HTTP───► roedy-hub :8080 (ACME HTTP-01, redirect to HTTPS)
                   roedy-hub :8081 (internal, not published) ◄── pairing, JWKS, manifest, events, users
                   roedy-hub ──► POST /<slug>/_roedy/v1/events (signed events to the apps)
```

**No proxy in front (decided on 2026-11-27):** no Caddy and no other
infrastructure image. roedy-hub terminates TLS itself and gets its
certificates itself with its own ACME client; `ROEDY_TLS=off` keeps plain
HTTP for development and for operators with a proxy of their own.

**DNS-01 with deSEC (decided on 2026-11-27):** the user's certificates are
validated with DNS-01 (a TXT record at `_acme-challenge.<host>`); the
domains are hosted at deSEC. `ROEDY_ACME_CHALLENGE=dns-01` with
`ROEDY_ACME_DNS_PROVIDER=desec`; TLS-ALPN-01/HTTP-01 stay the default.
deSEC client and the DNS queries of the propagation check are in-house, as
the rest of the ACME client.

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

Not taken over in milestones 1–6: the token broker, mail servers, tags,
notifications (bell, push, digests), backups in the app, update check,
maintenance mode, GeoIP. Since the platform direction (§ 9) the shared ones
among them follow in milestones 7–11; tags stay with Mail.

## 3. Repository layout

| Path | Content |
|---|---|
| `crates/server` | `roedy-hub-server`, binary `roedy-hub` |
| `crates/contract` | `roedy-hub-contract`: manifest, claims, events, Ed25519 JWS signing and verification, test vectors (`docs/app-contract/vectors.json`). Used by the server and the example app. |
| `crates/example-app` | `roedy-hub-example`: the test app that implements the whole contract. Not part of the image. |
| `apps/web` | `@roedy/hub-web`: SvelteKit SPA (home page, sign-in, setup, account, administration) |
| `packages/tokens` | Copy of Roedy Mail's design tokens |
| `docs/` | `app-contract.md`, `ops.md`, `plan.md`, setup guides |
| `deploy/` | `compose.yml` (the hub publishes 443 and 80 itself, no proxy), `compose.dns01.yml` (DNS-01 with deSEC), `.env.example`, `restore.sh`, the test override `compose.test.yml` |
| `scripts/` | Hooks, E2E runner, image smoke test, pin tests |

Commit scopes: `server`, `gateway`, `contract`, `example`, `web`, `tokens`,
`ci`, `deploy`, `docs`, `deps`, `repo`, `release`.

## 4. Server design

### Listeners

- **Public** (`ROEDY_LISTEN`, `0.0.0.0:8443` with TLS, `0.0.0.0:8080`
  without): hub SPA, `/api/v1`, `/healthz`, `/readyz`,
  `/.well-known/change-password` (→ `/account/security`), and the gateway for
  every other first path segment that is a known slug.
- **HTTP** (`ROEDY_HTTP_LISTEN`, `0.0.0.0:8080`, only with `ROEDY_TLS`):
  ACME HTTP-01 challenges and a `308` to `ROEDY_PUBLIC_URL`, nothing else
  (with DNS-01 only the redirect; publishing it is optional).
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

One initial schema, `crates/server/src/db/migrations/0001_init.sql`, holds
every table. Until the first release it is edited in place and nothing needs a
migration path from an earlier state; from the first release on, every schema
change is a new migration.

From Mail, trimmed: `schema_migrations`, `app_settings`, `users`, `sessions`
(plus `auth_time`), `audit_log` (category `apps` added), `totp_devices`,
`recovery_codes`, `known_devices`, `password_resets`, `email_changes`,
`invitations` (the app access chosen in the form in `invitation_access`), `user_settings`
(language, theme, accent), `assets` (logo).

New:

| Table | Content |
|---|---|
| `apps` | `id`, `slug` (unique), `internal_url`, `state` (`pending`, `paired`), `manifest_json`, `manifest_id`, `version`, `secret_hash`, `access_default` (`all`, `selected`), `position`, `health`, `health_at`, `online_at`, `contacted_at`, `paired_at`, `created_at` |
| `pairing_codes` | `app_id` (one open code per app), `code_hash`, `expires_at`, `created_by` |
| `app_access` | `user_id`, `app_id`, `app_role`; primary key (user, app) |
| `invitation_access` | `invitation_id`, `app_id`, `app_role` (empty: no access); what an invitation gives, other apps get their default |
| `signing_keys` | `kid`, `private_enc`, `public`, `created_at`, `active_from`, `retired_at` |
| `events` | `id`, `app_id`, `seq`, `type`, `data`, `created_at`; unique (app, seq) |
| `deleted_users` | `sub`, `deleted_at`: tombstones for `GET /internal/v1/users/<sub>`, kept permanently |
| `event_delivery` | `app_id`, `last_seq`, `delivered_seq`, `attempts`, `retry_at`, `failing_since`, `last_error` |
| `acme_accounts` | `directory`, `key_enc`, `url`, `created_at`: one ACME account per CA |
| `tls_certificates` | `domain`, `key_enc`, `chain_pem`, `not_before`, `not_after`, `directory`, `created_at` |
| `stalwart_servers` | Mail's `servers`: `id`, `label`, `base_url` (unique origin), `domains_json`, `oauth_client_id`, `oauth_client_secret_enc`, `reach`, `cors`, `oauth`, `tested_at`, `created_at`; the policy for custom servers in `app_settings.stalwart_policy` |
| `stalwart_accounts` | Mail's `mail_accounts`: `user_id`, `base_url`, `email`, `account_id`, `username`, `auth_kind` (`oauth`, `token`), `client_id`, `secret_enc`, `access_token_enc`, `access_expires_at`, `session_url`, `session_json`, `state` (`ok`, `needs_sign_in`), `color`, `is_default` (one per user), storage and job times; unique (user, origin, account ID) |
| `stalwart_sign_ins` | Sign-ins at a server under way: `state_hash`, `user_id`, `session_id`, `server_id`, `base_url`, `client_id`, `account_id` (sign in again), `verifier_enc` (PKCE), `login_hint`, `return_to`, `expires_at` (10 minutes) |

### New dependencies (to be justified in the PRs)

| Crate | Why |
|---|---|
| `hyper`, `hyper-util` (client, http1), `http-body-util` | Proxy client with streaming and upgrades; `hyper` already comes with axum |
| `roedy-jmap` (copy of Mail's `crates/jmap`), `reqwest` (rustls without provider), `rustls` (ring) | System mails go out over JMAP from a sending account, as in Mail (decided on 2026-11-26) |
| `askama`, `jiff` (embedded time zone database) | Same as Mail: mail templates, times in the recipient's time zone |
| `argon2`, `totp-rs`, `icu_properties` | Same as Mail: password hashes, authenticator apps, letters in display names (`icu_properties` already comes with `url`) |
| `ulid`, `ring`, `usvg` | Same as Mail: IDs, Ed25519, sanitizing the app icons |
| `resvg`, `roxmltree`, `imagesize`, `zune-jpeg`, `image-webp`, `skrifa` | Same as Mail: drawing the branding logo for mails and app icons, checking uploaded logos (decided on 2026-11-26) |

| `tokio-rustls` (ring, tls12), `ring` | Built-in TLS: rustls on tokio streams for the HTTPS listener and the healthcheck; ECDSA P-256 for the own ACME client. Both already in the tree through hyper-rustls, reqwest and the contract crate (decided on 2026-11-27) |
| `rustls-webpki` (dev only) | The TLS tests check the certificates the hub writes as rustls would; the verifier rustls brings |

No JWT library: the JWS is ~150 lines on top of `ring`, `base64` and
`serde_json`, which keeps `alg` handling strict.

No ACME and no X.509 library either (decided on 2026-11-27): the ACME client
(RFC 8555 with TLS-ALPN-01 from RFC 8737 and ARI from RFC 9773), the ES256
JWS and the DER of the CSR and of the TLS-ALPN-01 certificate are written in
`tls` on top of `ring`, `reqwest` and `rustls`.

No DNS library either (decided on 2026-11-27): DNS-01's deSEC client
(`tls::desec`, on `reqwest`) and the TXT queries to the zone's name servers
(`tls::dns`: RFC 1035 wire format over UDP with TCP fallback, ~250 lines)
are the hub's own; one question type without recursion does not need a
resolver.

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
checks pass in every PR; every PR has tests for what it adds. Milestones 7–11
(the platform for all Roedy apps) are in § 9.6.

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
3. [x] **Auth and users** – setup wizard with mandatory TOTP, sign-in with
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
   4. **Pages** in the web app, split into two PRs:
      1. [x] **Before and around the sign-in** – setup, sign-in (with
         `?next=` and `reauth=1`), password reset, invitation, email
         confirmation, the error pages for the gateway, sign-out. Done:
         Mail's guest pages in the new design (`AuthShell`, steps of setup,
         sign-in and invitation) with the hub's routes (`/login/forgot`,
         `/login/reset/…`, `/account/confirm-email/…`) and API; session
         from `GET session`, settings (language, colour scheme, accent; the
         scheme follows the system without a choice) also in
         `roedy.hub.prefs`, storage keys under `roedy.hub.`; `next` only for
         same-origin paths (`#lib/next.ts`), the hub's pages open in the
         web app, an app's with a full page load; `reauth=1` asks the
         signed-in account again and keeps the session; the invitation's
         authenticator app is optional ("Set up later"); error pages for
         403, 404, 500, 502, 503, 504 and offline – the gateway serves the
         web app with `<meta name="roedy-error" content="<code>">` (codes of
         contract § 12) and the page shows that error at the same address;
         the home page stays the placeholder, with "Sign out". Icons from
         Tabler (`@tabler/icons-svelte-runes`), the QR code with `qrcode`,
         as in Mail. Tests: component tests per page and state, the
         open-redirect cases of `next`, axe in jsdom for every state light
         and dark (`src/a11y.test.ts`), and `scripts/a11y/audit.mjs` (axe
         with contrast in Chromium, desktop and mobile, light and dark, via
         `scripts/dev/browser.sh`).
      2. [x] **Account and administration** – `/account/*` (profile,
         security, appearance, language) and `/admin/*` (users, audit log,
         branding, organization, email) as in the current Roedy Mail app.
         Done: Mail's shell for both areas (header with the home link,
         administration only for administrators and the account menu with
         colour scheme, language and sign-out; the settings navigation and
         the content card on desktop, the area's list and "Back" on mobile;
         `/account` and `/admin` open the first page on desktop, 403 inside
         the shell for users), the confirmation dialog, `DirtyBar`, the
         loading and failure cards and the colour editor; the pages with
         Mail's markup on the hub's API: profile (name, username, email
         change with password and code, pending confirmation with resend and
         discard, deleting the account with the typed word and
         `last_admin`; `DELETE auth/me` now also takes the confirmed
         account's `id` and answers 409 `account_mismatch` when another
         account signed in meanwhile), security (password, authenticator apps with QR code,
         recovery codes, signed-in devices), appearance and language (the
         three stored settings; the accent is locked without `userAccent`),
         users with invitations (search, filters, sorting, details with
         devices, role, lock, second factor, remove, bulk actions, resend,
         extend, copy link, `requested_at`), audit log (categories with
         counts, search, paging, CSV export, retention), branding (logo
         kinds with upload checks, plate, tilt, brand colour,
         `userAccent`), organization and email (sending account, test mail;
         no template preview, the API has none). Every load is sequenced so
         that only the latest response writes and nothing writes after the
         page is left. Without undo in the hub's toasts actions run at once,
         destructive ones after a confirmation. Times in the browser's time
         zone and the interface language. Tests: state modules, component
         tests per page and state, axe in jsdom per page state light and
         dark (`a11y.test.ts` next to each page), and the Chromium audit
         with one scenario module per page (`scripts/a11y/scenarios/`).
4. [x] **Gateway, pairing, identity** – `crates/contract`, signing keys and
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
   Split into four PRs (decided on 2026-11-26):
   1. [x] **Contract crate, signing keys and identity** – `crates/contract`
      with the JWS, signing keys with rotation, the JWKS and issuing
      identity tokens. Done: `crates/contract` (`roedy-hub-contract`:
      `Identity` and `Event` claims, `Jwks`/`Jwk`, the compact JWS on `ring`,
      `base64` and `serde_json` – `alg` checked before anything else, `typ`
      tied to the claims type so that an event never passes as an identity,
      strict base64url and JSON objects, `crit` rejected, times with the
      leeway of 10 s; error codes as in the vectors), test vectors in
      `docs/app-contract/vectors.json` (RFC 8037's test key as in contract
      § 4.4, the examples of § 15 and every rejection case; made and checked
      by `crates/contract/tests/vectors.rs`), migration 0005
      (`signing_keys`, the Ed25519 seed sealed with the instance key per
      `kid`), `signing` (rotation at start and daily in the job
      `key_rotation`: every 90 days, the next key published 10 min before it
      signs, the previous one published 10 min after; keys that cannot be
      decrypted are retired and replaced at once; the key store picks the
      signing key by time), `GET /internal/v1/jwks` on the internal listener
      (`Cache-Control: max-age=300`) and `identity` (tokens per session and
      app, valid 60 s, reused for 30 s while claims and key stay the same,
      `forget_session` for revocation). The proxy sends them in part 3.
   2. [x] **Apps and pairing** – apps, pairing codes, manifest, the internal
      listener's `pair`, `manifest` and `users`, health poll, the example
      app. Done: migration 0006 (`apps`, `pairing_codes`, `app_access`,
      `deleted_users`; `users::delete` leaves a tombstone, `users::insert`
      grants the default role of "everyone" apps), the manifest's rules,
      slugs with the reserved names, pairing codes and the internal API's
      bodies in `roedy-hub-contract` (`manifest`, `pairing`), `admin/apps`
      (list, add with slug and internal address – returns the first code –,
      show, new pairing code, remove; audit category `apps` with en/de
      texts), `POST /internal/v1/pair` (rate limit 10 per address and 30 in
      total per 15 minutes, every attempt counts; code – only SHA-256
      stored, one open code per app, 15 minutes, used up only by a pairing
      that succeeds –, manifest, `public_url`; the icon sanitized like a
      branding upload and with a square `viewBox`; secret `rhs_<app ID>_…`
      stored as SHA-256; hub administrators get the admin role, with
      "everyone" all others the default role, existing entries stay),
      `PUT /internal/v1/manifest` (removed roles fall back to the default
      role with an audit entry; the `access.changed` event follows with
      part 4), `GET /internal/v1/users[/<sub>]` (pages by `sub`, locale from
      the settings, `404 no_access`, `410 user_deleted`), the app secret as
      `PairedApp`, the in-memory `AppTable` (reloaded in the database job of
      every change, for the gateway), the job `health_poll` (every 30 s,
      `GET /<slug>/readyz` with 5 s to connect and 10 s in total, 2xx is
      online; once more right after pairing) and `crates/example-app`
      (`roedy-hub-example`: pairs, sends its manifest, verifies the identity
      with the contract crate and refreshes the JWKS for an unknown `kid`,
      accepts events idempotently; echo, SSE, WebSocket echo with the
      `Origin` check, streamed upload and download, slow and custom
      responses for the gateway's tests; axum's `ws` brings
      tokio-tungstenite, only into the example). Not yet: the browser API,
      the icon endpoint, order and access default, access per user (5).
   3. [x] **Gateway proxy** – routing, header rules, streaming, WebSocket,
      session cache, the identity header, cancelling streams on revocation.
      Done: `gateway` as the public listener's fallback – a first segment
      with a slug's format that is not reserved goes to the app from the
      `AppTable`, everything else stays with the web app; unknown slugs and
      `/<slug>/_roedy/…` (also percent-encoded or with dot segments) `404
      not_found`; a paired app gets requests without identity when signed
      out, with `X-Roedy-Identity` (`identity::Identities::issue`, the role
      from `app_access`) with access, else `403 no_app_access`; a pending
      app only hub administrators' requests, without identity, else `503
      app_not_ready`. The session cache (`gateway::sessions`, 10 s per
      cookie, with language and roles per app; a revocation during a check
      keeps its result out) is cleared at once by `AppState::end_session`
      and `end_user_sessions` – sign-out, revoking devices, "Sign out all
      others", password change and reset, deleting the own account, lock
      and removal (also in bulk), a sign-in or invitation that replaces the
      browser's session; a role change drops the cache entries. hyper's
      pooled client (HTTP/1.1, `hyper-rustls` with the platform verifier
      for `https` addresses, as reqwest's), bodies streamed both ways,
      WebSocket via hyper's upgrades and `copy_bidirectional` after the
      app's `101`, `Origin` other than the hub's `403 origin_mismatch`.
      Header rules as pure functions in `gateway::headers` with
      table-driven tests (`client_ip::trusted_entries` shared with the
      hub's own client IP). Limits (`gateway::Limits`): 5 s to connect →
      `502 app_unreachable`, 300 s from the end of the request body to the
      response headers → `504 app_timeout`, 64 KiB of request headers → `431 headers_too_large`
      (added to contract § 3.4 and § 12). Errors as JSON with `no-store`,
      for page loads the web app with `<meta name="roedy-error">`
      (`static_files::error_page`, with the hub's security headers); the
      app's responses are marked `Proxied` and keep their own headers.
      Response bodies and WebSockets are registered per session
      (`gateway::sessions::Streams`) and close when it ends – also when it
      ended while the request waited for the app (checked again after the
      registration); the job `gateway_streams` (every second, a query of at
      most 1 s, only while streams are open) closes those of sessions
      that ended otherwise (expired, trimmed, `reset-admin`), the shutdown
      all of them. The connect deadline covers lookup, TCP and TLS
      (`gateway::client::Deadline`); the 300 s count from the end of the
      request body (`gateway::client::Upload`), so uploads have no time
      limit. Tests: unit tests of the
      rules, routing and access, and `tests/gateway.rs` against the example
      app over real connections (path and query, headers both ways, the
      identity verified at the app and its rejections with the hub's real
      key, key rotation with a JWKS refresh, permissions, error pages, SSE
      before the stream ends, WebSocket echo and close, 64 MiB up and down
      with flat memory, timeouts, unreachable app, streams closed on
      sign-out, lock and expiry).
   4. [x] **Events** – delivery with backoff, catch-up, retention. Done:
      migration 0007 (`events` with `id` = `jti`, per-app `seq`, `type`,
      `data`; `event_delivery` with `last_seq` – the counter, so that `seq`
      never repeats after the retention –, `delivered_seq`, `attempts`,
      `retry_at`, `failing_since`, `last_error`), the outbox in
      `db::repo::events`: the functions that change users, sessions and
      access write their events in the same transaction (`db::atomic`, a
      savepoint) – `users` (`user.locked`, `user.unlocked`, `user.deleted`,
      `user.updated` for name, username, email, role and the sign-in
      language when it decides the language), `settings::save` (`locale`),
      `sessions` (`session.revoked`: `logout` for sign-out and a replaced
      browser session, `revoked` for devices, "Sign out all others" and the
      21st session, `password_changed` also for `reset-admin`, `locked`,
      `expired` in the hourly cleanup and for expired rows that end with
      others) and `app_access` (`access.changed` for the defaults after
      pairing, new users with "everyone" apps and the manifest's role
      fallback); user and session events to every paired app, `access.changed`
      only to its app, pending apps none, `user.deleted` without events for
      the sessions and access that go with it. `events` delivers: the update
      hook on the connection wakes one worker per paired app right after a
      new event or a change of `apps` (a check every minute finds events of
      `reset-admin`), the event is signed anew per attempt with the current
      key (`typ` `roedy-event+jwt`, 300 s) and POSTed to
      `/<slug>/_roedy/v1/events`, `204` within 10 s moves on, anything else
      retries after 10 s, 30 s, 1 min, 5 min, 15 min, hourly; past 7 days
      from the first failure the delivery waits until the app is back (`PUT
      manifest`, catch-up, pairing, an online health check). `GET
      /internal/v1/events` (secret, `after`, `limit` 1–100, `more`, `410
      events_expired` with `resume_after`; acknowledges up to `after` and
      retries at once), the daily job `event_retention` (30 days, delivered
      or not; per app only a contiguous prefix of `seq`, so that a clock
      that went back cannot leave a gap), removing an app deletes its events (cascade) and stops its
      worker. The example app verifies events with the contract crate,
      stores the highest `seq`, catches up at start and on a gap (`503`
      while it cannot) and continues from `resume_after` after a `410`; it
      can fail or delay deliveries for the tests. The contract (§ 10)
      states the giving up and resuming, gaps, the first events after
      pairing, the absence of a "user created" event, when `user.updated`
      and `expired` come, and the catch-up's defaults and acknowledgement.
      Tests: the outbox, backoff and paging in `db::repo::events`, and
      `tests/events.rs` against the example app over real connections
      (every emitting path with the apps that get its event, a second app
      with "selected only", a pending one; order and retry after 5xx, timeout
      and an unreachable app; giving up and resuming; redelivery; catch-up
      with paging and `410`; a gap; forged, wrong-`typ`, wrong-`aud` and too
      long-lived events at the example; removal). `access.changed` with
      `access: false` has no path yet: access per user comes with
      milestone 5.
5. [x] **Home page and permissions** – home page tiles, browser API
   (`/api/v1/session`, `/api/v1/apps`, icons), administration of apps (add,
   pairing code, status, order, access default, remove), access and roles per
   user and in the invitation form, 403 page.
   *Tests:* permission matrix (admin/user × access yes/no × app
   pending/paired), `access.changed` events, component tests, axe.
   Split into two PRs (decided on 2026-11-27):
   1. [x] **Server API** – browser API, administration of apps, access per
      user and in invitations. Done: `GET /api/v1/apps` (`api::home`: paired
      apps with access in the administrator's order, name and description in
      the user's language, `url` `/<slug><home>`, the versioned `icon`,
      `status` from the last health check; pending apps are never listed,
      not even for hub administrators – they reach them through the
      administration and the gateway) and `GET /api/v1/apps/<slug>/icon.svg`
      (`web::app_icons`, outside the API's middleware: public, `ETag`,
      `immutable` only with the current `?v=` – the first 16 hex digits of
      the sanitized icon's SHA-256, `apps::icon_version` –, `no-cache`
      otherwise, the contract's CSP, 404 for pending apps; the sanitized icon
      now keeps a `viewBox`, so that it scales in an `<img>` – a square
      `viewBox` with a rectangular size is `not_square`, and icons stored
      before get theirs when read, `apps::stored_icon`);
      `GET /api/v1/session` already matched § 6. Migration 0008
      (`invitation_access`; `apps.contacted_at` – the app's last request
      with its secret or its pairing, written at most once a minute –, and
      `apps.online_at`, the last passed health check). `admin/apps` with the
      status (`health`, `health_at`, `online_at`, `contacted_at`, the event
      delivery from `event_delivery`, `user_count`, `url`, `icon`),
      `PATCH admin/apps` (`{order}`: every slug once), `PATCH
      admin/apps/:slug` (`{access_default}`: for new users only, existing
      access stays as it is in both directions), `GET admin/apps/:slug/users`
      (every user with `app_role` or `null`, and what the open invitations
      will give); audit entries `apps.order` and `apps.access_default`.
      Access per user: `PUT|DELETE users/:id/apps/:slug` (`{app_role?}`,
      without one the default role; also the own account and locked ones;
      pending apps 409 `app_pending`) and the bulk action `access` (`slug`,
      `access`, `app_role?`; for users and invitations, the own account
      included), `apps` (slug → role) on every user and invitation row; each
      change writes `access.changed` (`app_access::set`, removal with
      `access: false`), an audit entry `apps.access_grant|access_role|
      access_revoke` and, after the commit, drops the user's gateway cache
      entries and on removal closes their streams and WebSockets to that
      app (`gateway::sessions::Streams` now per session and app; a request
      that waited for the app while the access went is checked again).
      Invitations: `apps` (slug → role or `null`) in `POST invites` and
      `PATCH invites/:id` (only the apps named change; `role` is optional
      now), stored in `invitation_access`, audit `apps.invite_access`;
      accepting grants the choice and the defaults for the other apps
      (`users::insert_with`, `app_access::grant_invited`) with
      `access.changed` per app, and names the access in
      `users.invite_accept`. Tests: `tests/access.rs` (the permission matrix
      over `/api/v1/apps`, the icon and the gateway, icon headers and cache
      busting, status, order and access default, access per user with its
      events and audit entries, bulk, invitations up to the acceptance,
      admin-only and CSRF for every new modifying endpoint),
      `tests/gateway.rs` (grant, role change and removal at once, streams
      closed) and `tests/events.rs` (role change and removal delivered to the
      example app).
   2. [x] **Pages** – home page tiles, Administration › Apps (list with status,
      order, add, pairing code, access default, the app's users), access and
      roles in the user and invitation forms, the 403 page. Done: the home
      page (`#lib/home.svelte.ts`) gets the shell's header (brand,
      administration, settings, account menu) without navigation and card;
      greeting, the apps from `GET /api/v1/apps` as tiles (icon, name,
      description, "Offline"; the name is the link and covers the tile),
      loading again when the tab becomes visible, an empty state (for
      administrators "add your first app" or "give yourself access", with the
      way to Administration › Apps) and, for administrators, a hint while
      apps wait for pairing (from `admin/apps`, as `GET /api/v1/apps` never
      lists them). Administration › Apps (`#lib/admin-apps.svelte.ts`, new
      entry in the navigation): the list in the home page's order with
      status (online, offline, not checked, waiting with the code's
      validity), users and failing event delivery; "Move up/down" buttons
      (keyboard and screen reader, the new position announced, focus kept;
      `PATCH admin/apps` one after another, a 422 reloads); "Add app" with
      slug and internal address checked like the server, field errors at
      the fields, the pairing code shown once with "Copy". The app's page
      `/admin/apps/[slug]`: status, addresses, version, app ID, event
      delivery, "New pairing code", the access default as a radio group
      (saved at once; "applies to new users only"), the roles with "Default"
      and "Full control", who has access (users and invitations, those
      without folded), "Remove app" with confirmation; "App not found" for
      an unknown slug. Access in Administration › Users
      (`#lib/users.svelte.ts`, `AccessPicker.svelte`): per paired app a radio
      group "No access" / roles in the expanded row (at once, for
      invitations through `PATCH invites/:id`, shown over lists loaded
      meanwhile, a later change wins), in the invitation form (preset from
      each app's default; only changed apps are sent, errors
      `apps.<slug>` at the app) and as the bulk action "App access"
      ("Unchanged" per app, one `users/bulk` per changed app); `app_pending`,
      a role or app gone reload the apps and the list. The gateway's 403
      `no_app_access` has its own text ("No access to this app"); the other
      gateway pages are unchanged. Tests: state modules (`home`, `apps`,
      `admin-apps`, access in `users`), component tests per page, axe per
      page state in light and dark (`a11y.test.ts` next to each page) and
      the Chromium audit (`scripts/a11y/scenarios/home.mjs`, `apps.mjs`, the
      access states in `users.mjs`). Open: the home page has no design yet –
      it is built plainly with the tokens (greeting, tiles in a grid, a list
      on mobile) and is finished once there is a draft.
6. [x] **Delivery, TLS, end-to-end** – four PRs. The user decided on 2026-11-27
   that there is no Caddy and no other infrastructure image: roedy-hub
   terminates TLS and gets its certificates itself, in-house.
   *Budgets checked in CI:* image ≤ 25 MB, idle ≤ 40 MB.
   1. [x] **Delivery** (PR #19) – Dockerfile, release workflow, budgets and
      smoke test in CI, Compose setup, `.env.example`, `docs/ops.md`, README.
      Done: as in Roedy Mail, the `Dockerfile` only packages the static musl
      binaries from `just dist` (`cargo auditable zigbuild`, embedded web app)
      on `scratch` with CA certificates, user 65532, `/data`, the healthcheck
      and mimalloc's settings; it exposes 8080 only. `release.yml` (tags `v*`
      on the release commit, manual dry run) builds both platforms, checks
      size, smoke test and Trivy, pushes to `ghcr.io/roedylabs/hub` with SBOM
      and provenance, signs with cosign, attests image and binaries and
      creates the GitHub release with git-cliff's notes; `just
      release`/`release-tag` (`scripts/release.sh`) prepare it. The CI job
      "Image" (part of "CI passed") builds the amd64 image on every pull
      request: size ≤ 25 MB, `scripts/image-smoke.sh` (user and privileges,
      health, SPA, CSP, internal listener listening but neither exposed nor on
      the public port, setup token, idle RSS ≤ 40 MB, clean stop) and
      `scripts/compose-smoke.sh`. `deploy/compose.yml`: Caddy (only published
      service, `deploy/Caddyfile`), the hub on the networks `edge` and `apps`
      with its internal listener bound to its fixed address in `apps`, Roedy
      Mail with the profile `mail` (until Mail has hub mode);
      `deploy/compose.test.yml` adds the example app
      (`deploy/example-app.Dockerfile`, `just dist-example`) and Caddy on
      plain HTTP at 127.0.0.1:8080. Dependabot keeps the Docker images up to
      date, `scripts/docker-pins.test.mjs` checks their digests.
   2. [x] **Built-in TLS and ACME** – the hub terminates TLS and gets its
      certificate itself. Done: `ROEDY_TLS` (`off` by default, `acme`,
      `files`); with TLS on `ROEDY_LISTEN` is HTTPS on `0.0.0.0:8443` and
      `ROEDY_HTTP_LISTEN` (`0.0.0.0:8080`) answers ACME HTTP-01 and redirects
      (`308`) to `ROEDY_PUBLIC_URL`; `ROEDY_ACME_DIRECTORY` (Let's Encrypt
      production by default), `ROEDY_ACME_EMAIL`, `ROEDY_ACME_AGREE_TOS=true`
      required; `ROEDY_TLS_CERT_FILE`/`ROEDY_TLS_KEY_FILE` reloaded when they
      change. rustls (ring, TLS 1.3 and 1.2, ring's AEAD suites), ALPN
      `http/1.1` only – axum serves HTTP/1.1 and the gateway's WebSockets
      need HTTP/1.1's `Upgrade` (h2 would need RFC 8441) –, SNI must be the
      public host, the certificate swapped through a resolver without
      dropping connections, handshakes in their own tasks with a 10 s
      deadline. Own ACME client (`tls::acme`): ES256 JWS and JWK thumbprint
      (`tls::jose`), nonces with `badNonce` retries, account, order,
      TLS-ALPN-01 on the HTTPS port first and HTTP-01 after a failed
      validation, CSR and the challenge certificate as hand-written DER
      (`tls::der`, `tls::x509`; no `rcgen`), finalize, download, ARI with
      `replaces`. Renewal 30 days before the end (two thirds of a shorter
      lifetime) or by ARI in the daily job `certificate_renewal`; first
      issuance in the background at start while the HTTPS port answers only
      TLS-ALPN-01; a background task retries every failed attempt, also a
      failed renewal. Failures wait 1 min, 5 min, 15 min, 1 h, then 3 h, or
      longer by `Retry-After`; the next attempt is stored, so restarts do not
      hammer the CA. Migration 0009: `acme_accounts`, `tls_certificates`,
      keys sealed with the instance key (another instance key: new account
      and certificate). Audit `tls.issue`, `tls.renew`, `tls.fail` (first of
      a series). HSTS also with TLS on; `roedy-hub healthcheck` speaks TLS
      then. *Tests:* RFC 7638 vector, JWS, DER and X.509 (CSR and
      certificates parsed back, chains verified with webpki), config
      validation, resolver, redirect, files reload, and an in-process fake
      ACME server with its own CA (`tests/fake_acme`, `tests/tls.rs`):
      issuance over TLS-ALPN-01 and HTTP-01 with real validations against
      the hub, `badNonce`, rate limit with `Retry-After`, error documents,
      renewal by lifetime and by ARI with an open connection kept, another
      instance key, a restart serving the stored certificate, and HTTPS to a
      rustls client trusting the fake CA. Open: an admin view of the
      certificate's state (the audit log and the log have it for now); a CI
      job against Pebble.
      - [x] **ACME DNS-01 with deSEC** (extra PR, decided on 2026-11-27: the
        user's domains are at deSEC and validated by DNS). Done:
        `ROEDY_ACME_CHALLENGE` (`tls-alpn-01` by default, `dns-01`),
        `ROEDY_ACME_DNS_PROVIDER=desec`, `ROEDY_DESEC_TOKEN_FILE` or
        `ROEDY_DESEC_TOKEN` (never logged; the start fails on incomplete
        settings). Per validation: the domain owning `_acme-challenge.<host>`
        (`GET domains/?owns_qname=`), the value added to the TXT record set by a
        bulk `PUT` (the set's TTL or the zone's minimum, other values kept;
        the hub assumes it is the set's only writer – deSEC has no conditional
        update), every name server of the zone (its `NS` records) asked
        directly until it serves the value authoritatively (5 min at most),
        then the CA; afterwards, also after a failure, the value's removal is
        tried (a refused one is logged, the value stays). deSEC's `429` is
        waited out in place for each `Retry-After` of up to a minute, three
        times per request (a write from a fresh read); a longer one ends the
        certificate attempt (the issuer's backoff) or, during the removal, is
        logged and the value stays. No migration: nothing new is
        stored. ops.md documents a scoped token (token policies: read-only
        default, write only on the TXT set). *Tests:* DNS wire format
        (compression pointers, loops, truncation, several strings), deSEC
        bodies and errors, config validation, and with an in-process fake deSEC
        and two fake name servers next to the fake ACME server: issuance and
        renewal over DNS-01, record set created, updated and deleted, cleanup
        after a failed validation, `403`, `429` short and long, waiting for a
        slow name server, giving up on one that never serves the value or does
        not answer, a name outside the account, TCP fallback, and the token in
        no log line, error or audit entry. Deploy (after "Deploy without
        Caddy"): `deploy/compose.dns01.yml` adds DNS-01 to `compose.yml`
        (`COMPOSE_FILE` in `.env`, the token as the Docker secret
        `./desec_token`, gitignored; port 80 only redirects then) –
        recommended in `.env.example`, README and ops.md; an override rather
        than variables in `compose.yml`, because a Compose secret whose file
        is missing fails every setup without deSEC. `compose-smoke.sh`
        validates it. Open: a CNAME at `_acme-challenge` (delegation) is not
        followed.
   3. [x] **Deploy without Caddy** – compose publishes the hub's 443 and 80
      (`443:8443`, `80:8080`) and drops Caddy and `deploy/Caddyfile`,
      `ROEDY_TLS=acme` in `.env.example`, ops docs, smoke tests with TLS.
      Done: `deploy/compose.yml` has no Caddy, no `Caddyfile` and no network
      `edge`; the hub publishes `443:8443` and `80:8080` (TCP) with
      `ROEDY_TLS=acme` (`ROEDY_ACME_AGREE_TOS`, `ROEDY_ACME_EMAIL`,
      `ROEDY_ACME_DIRECTORY` and the `files` settings from `.env`),
      `ROEDY_TRUST_PROXY` off, Mail's `1`, no capability, the internal
      listener on its fixed address in `apps` and never published; health
      check start period 3 minutes (image and Compose) for the first
      certificate. The `Dockerfile` exposes `8080 8443`.
      `deploy/compose.test.yml` runs the hub with `ROEDY_TLS=files` and a
      self-signed certificate for `localhost` from `scripts/test-cert.sh` on
      `127.0.0.1:8443`/`8080` (`ROEDY_PUBLIC_URL=https://localhost:8443`,
      `ROEDY_TEST_TLS_DIR`): no CA needed, offline, and still HTTPS, SNI,
      redirect, HSTS and the healthcheck over TLS end to end.
      `scripts/image-smoke.sh` runs a second container with TLS from files
      (certificate and SNI, HSTS, `308` for every path on 8080, the ACME path,
      no web app or API on plain HTTP, healthcheck, privileges, idle RSS with
      TLS); `scripts/compose-smoke.sh` checks the production settings
      (only the hub publishes 443/80, `acme`, no trusted proxy), HTTPS and the
      redirect through the test setup and that only the apps' network reaches
      the internal listener and the apps. `docs/ops.md`: installation without
      Caddy, DNS, firewall, the TOS opt-in, staging,
      behind your own proxy. Open: the hub does not compress responses
      (Caddy did); a CI job against Pebble.
   4. [x] **End-to-end** – Playwright suite with axe against hub + example
      app on the Compose test setup (setup, sign-in, pairing, opening the app,
      SSE and WebSocket through the browser, sign-out, no access, light and
      dark, desktop and mobile).
      Done: `apps/e2e` (`@roedy/hub-e2e`, Playwright and axe-core; the axe
      run with the Chromium audit's rule set, TOTP and the authenticator from
      Mail's suite) and `scripts/e2e.sh`, which starts `deploy/compose.yml`
      with `compose.test.yml` (or, with `--host`, debug binaries without
      containers), makes the test certificate, hands the suite the setup token,
      restarts the example app with a pairing code when the suite calls
      `scripts/e2e.sh --pair` and stops everything again. The browser trusts only the
      test certificate (`--ignore-certificate-errors-spki-list`), Node gets it
      as `NODE_EXTRA_CA_CERTS`; no TLS check is switched off. One serial flow:
      `308` from plain HTTP and HSTS; the setup's seven steps with the code
      from the shown key (desktop, light); sign-in with password and code and
      `next` (mobile, dark); Administration › Apps › Add, the pairing code,
      the example paired and online, its details (desktop, dark); the tile
      on the home page opens `/example/` through the gateway, the example
      verified the identity (`whoami`: `iss`, `aud`, email, roles); SSE (three
      events as they are sent) and WebSocket (text, binary, a clean close) in
      the browser; an invitation without access accepted on a phone with "Set
      up later", the home page without apps and the `403` page "No access to
      this app"; sign-in, home, apps, app details, users, the example and the
      `403` page in desktop and mobile × light and dark; sign-out from the
      account menu in another tab: `Clear-Site-Data: "cache", "storage"`, the
      session cookie gone, the app's SSE stream and WebSocket ended at once,
      the example without identity. axe checks each of these page states
      (none found anything). CI: the job "End-to-end" in `ci.yml` after
      "Image", with the images it built (an artifact), the browser cached by
      Playwright version, traces, screenshots and the container logs as an
      artifact on failure; part of "CI passed" – the only browser test across
      hub, gateway and app, a few minutes after "Image". Found on the way:
      the example app dropped the connection right after a closing frame
      instead of letting tungstenite answer it (browsers saw `1006`); it now
      completes the closing handshake, its and the gateway's tests check that.
      Its page has `<main>` and a viewport (axe) and `connect-src 'self'` (the
      suite opens SSE and WebSocket from it). Not covered end to end: the
      sign-in with a recovery code, events delivered to the app, the German
      interface, browsers other than Chromium, ACME (the in-process fake CA
      covers it; Pebble stays open), and the setup and invitation steps in
      the other views and themes (the Chromium audit covers them with mocked
      data). The example's page and SSE are public (contract § 5.1), so after
      the sign-out its EventSource connects again – without identity.

## 7. Open points (not in this assignment)

The shared design tokens, Stalwart accounts and tokens, the app switcher,
notifications, web push, backups in the app, update check, maintenance mode
and GeoIP are milestones now (§ 9.6). Moving Mail's users into the hub is no
longer a goal (decided on 2026-11-28): the standalone Roedy Mail is archived,
the new Mail starts on the hub, and the mail itself stays in Stalwart.

- **Restoring an older backup** leaves the apps' event position ahead of the
  hub's numbering (`docs/ops.md` › Backup and restore); the hub could
  announce a new stream (e.g. an epoch in the events) so that apps reset it.
- **The new Roedy Mail** pairs like the example app: `deploy/compose.yml`
  passes the hub's address and the pairing code as `ROEDY_HUB_INTERNAL_URL`
  and `ROEDY_PAIRING_CODE`, and the profile `mail` starts the new image
  instead of the standalone one once it exists.
- **Backups of the apps' data:** each app backs up its own database for now;
  the hub could trigger and list them (an internal endpoint or an event) so
  that Administration › Maintenance covers the whole installation.
- **Single sign-on to Stalwart:** the hub as an OpenID Connect provider for
  Stalwart, so that connecting an account needs no second sign-in at the mail
  server. Depends on what Stalwart supports as an external identity source.
- **Audit entries from apps:** an internal endpoint so that apps record
  administrative actions in the hub's audit log instead of keeping their own.
- **Search across apps** on the home page (mails, events, contacts) through
  the apps' own search; and **tags shared by several apps** – both only once
  Calendar and Contacts exist.
- **Installable apps:** one installed "Roedy" with the switcher, or one web
  app manifest per app with its own scope – decided with the new Mail.
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

## 9. Platform for all Roedy apps (agreed on 2026-11-28)

### 9.1 Direction and decisions

- **The hub is the shared platform (decided on 2026-11-28):** everything that
  several Roedy apps need (Mail, Calendar, Contacts …) is built in the hub;
  the apps contain only what is specific to them. There is no release yet, so
  the hub stays clean code without compatibility layers for earlier states.
- **Roedy Mail standalone** (`../mail`) is archived as `mail-standalone` and
  only a reference from now on. A new Roedy Mail is built on the hub.
- **Apps stay separate containers behind the gateway (decided on
  2026-11-28).** Considered and rejected: apps as modules inside the
  `roedy-hub` binary, switched on and off in the web interface (as in
  node64). The contract (`app-contract.md`) stays the only interface between
  hub and apps.
- **Switching apps on and off (decided on 2026-11-28):** Administration ›
  Apps turns an app off – the gateway answers `503`, it disappears from the
  switcher and the home page, it gets no events, its data and pairing stay.
  The containers are started and stopped with Compose profiles
  (`COMPOSE_PROFILES=mail,calendar`); the hub gets no Docker socket.
- **Separate repositories with published packages (decided on
  2026-11-28):** each app has its own repository; the hub publishes what they
  share as packages (§ 9.4, § 9.5).
- **Shared interface: hybrid (recommendation, no preference from the
  maintainer, 2026-11-28):** the frame that every app shows the same way is
  served by the hub at run time, everything else comes as packages at build
  time (§ 9.4).
- **Stalwart: the hub owns servers, accounts and tokens, without a JMAP proxy
  (recommendation, no preference from the maintainer, 2026-11-28):** browsers
  keep talking JMAP directly to Stalwart with short-lived tokens from the hub
  (§ 9.3).
- **Milestone order (recommendation, no preference from the maintainer,
  2026-11-28):** what the new Mail needs first (§ 9.6).

### 9.2 What Roedy Mail has, and where it goes

Inventory of `../mail` (paths below `crates/server/src/`, `apps/web/src/` and
`packages/`). **Hub:** already in the hub. **Shared:** belongs in the hub, with
its milestone. **Mail:** stays in the app.

| Building block | In Roedy Mail | Where |
|---|---|---|
| Sign-in, sessions, devices, authenticator apps, recovery codes, fresh sign-in | `auth/*`, `api/auth`, `api/mfa`, `api/sessions`, `api/me`, `api/password` | Hub |
| Setup, setup token, `reset-admin` | `api/setup`, `setup_token`, `setup_totp`, `reset_admin` | Hub |
| Users, invitations, email change | `api/users`, `api/invites`, `api/email` | Hub |
| Audit log with texts | `api/audit`, `api/audit/texts` | Hub |
| Branding, logo, organization, public config | `api/branding`, `api/instance` (without the banner), `api/public`, `logo/*`, `web/branding` | Hub |
| System mails over JMAP, templates, security notices | `mailer/*` (without `preview`), `events/mail`, `crates/jmap` (sending) | Hub |
| Language, colour scheme, accent | `settings` (these three fields) | Hub |
| Server base: config, database, scheduler, rate limits, client IP, security headers, embedded web app, healthcheck, locks, server texts | `config`, `db`, `jobs/scheduler`, `rate_limit`, `client_ip`, `web/*`, `healthcheck`, `locks`, `i18n` | Hub |
| Guest pages, account and administration shell | `routes/(auth)`, `setup`, `invite`, `account`, `admin`, `components/{auth,setup,signin,invite}` | Hub |
| App switcher | `components/AppsMenu.svelte` | Shared, 7 (done) |
| Interface building blocks: Sheet, Toast, Tooltip, ConfirmDialog, DirtyBar, Skeleton, PickerList, ErrorView, FirstLoad, Header, AccountMenu, SettingsNav, ColorEditor, TimePicker, CommandPalette, ShortcutsDialog, LicensesDialog, DebugBar | `lib/components/*` | Shared, 7 (`@roedy/ui`) |
| Actions and helpers: focus trap, tooltip, radio group, tab list, scroll bar; `api`, `ago`, `date`, `theme`, `swipe`, `keys`, `wait`, `undo`, network state and "No connection", new-build detection, developer log | `lib/actions/*`, `lib/*.ts`, `lib/state/{network,devlog}`, `lib/app-worker.ts` | Shared, 7 (`@roedy/ui`) |
| Design tokens | `packages/tokens` | Shared, 7 (`@roedy/tokens`, the hub's copy becomes the source) |
| JMAP for the browser: client, session, token, batching, sync, EventSource stream, push, errors; contact cards | `packages/jmap/src/{client,core,session,token,methods,sync,stream,push,errors,contacts}.ts` | Shared, 7 (`@roedy/jmap`) |
| JMAP for servers | `crates/jmap` (session, requests, quota) | Shared, 7 (`roedy-jmap`, already in the hub for sending) |
| Stalwart server registry, policy for custom servers, diagnosis, OAuth client by DCR | `api/servers`, `broker/{probe,dcr,outbound}`, `jobs/server_probe`, `db/repo/servers`, `admin/mail-servers`, `ServerDialog` | Shared, 8 (API done) |
| Users' Stalwart accounts: several per user, default account, colour, "Sign in again", removal | `api/mail_accounts`, `broker/account`, `db/repo/mail_accounts`, `routes/{account/mail-accounts,oauth/callback,no-account}`, `lib/auth/*` | Shared, 8 (API done) |
| Token broker with strict rotation, early refresh, revocation | `broker/{token,oauth,crypto}`, `jobs/token_refresh` | Shared, 8 (done) |
| Storage per account | `jobs/quota_check`, `crates/jmap/src/quota.rs` | Shared, 8 (done) |
| Shared settings: region, time zone, date, time and number formats, week start, font size, density, animations, keyboard shortcuts on or off, confirmations and undo delay, developer mode | `settings`, `routes/settings/{appearance,language,general}`, `lib/{timezone,locale,date}.ts` | Shared, 9 |
| Directory of the instance's users | `api/directory` | Shared, 9 |
| Notifications: catalogue, bell, channels per topic, retention | `events/{mod,texts}`, `api/notifications`, `jobs/notification_cleanup`, `NotificationsMenu`, `routes/settings/notifications` | Shared, 10 |
| Digest by email, quiet hours, sound | `events/schedule`, `jobs/notification_mailer`, `db/repo/notification_deliveries`, `lib/chime.ts` | Shared, 10 |
| Web push: VAPID, encryption, subscriptions, the service worker's notifications | `push/*`, `api/push`, `jobs/push_sender`, `db/repo/push_subscriptions`, `service-worker`, `lib/push.svelte.ts`, `PushDialog` | Shared, 10 |
| Maintenance mode, storage and "Compact", banner (notice bar) | `web/maintenance`, `api/maintenance`, `api/instance` (banner), `NoticeBar`, `admin/{maintenance,banner}` | Shared, 11 |
| Update check | `updates/*`, `jobs/update_check`, `api/updates`, `admin/updates` | Shared, 11 |
| Backups in the app | `backups/*`, `api/backups`, `jobs/backup` | Shared, 11 (the hub's own data) |
| GeoIP | `geoip`, `jobs/geoip_reload` | Shared, 11 |
| Template preview for Administration › Email | `mailer/preview` | Shared, later (no milestone) |
| Mailboxes, threads, lists, message view, rendering and sanitizing, external content | `lib/mail/*`, `lib/state/{mailboxes,threads,message,list-view,folders}`, `MailList*`, `MessageView`, `FolderTree` | Mail |
| Compose, drafts, scheduled sending, replies, recipient suggestions | `ComposeView`, `lib/state/{compose,scheduled,sent-replies,contacts}` | Mail |
| Rules (Sieve), vacation, identities | `lib/sieve`, `packages/jmap/src/sieve.ts`, `MailRules`, `MailVacation`, `MailIdentities` | Mail |
| JMAP for mail, push for new mail | `packages/jmap/src/mail.ts`, `crates/jmap/src/mail.rs` (beyond sending), `lib/mail-push-*` | Mail |
| Mail search | `lib/search*`, `SearchFilters`, `SearchSuggestions` | Mail |
| Tags | `api/tags`, `db/repo/tags`, `TagList` | Mail |
| Mail's own settings: list design, swipe actions, external content, start page | `settings` (`pfDesign`, `swL`, `swR`, `extMode`, `extAllow`, `gStart`) | Mail |

Borderline cases:

- **Tags** group mailboxes across a user's accounts; nothing else uses them
  yet. They stay with Mail; tags shared by several apps are an open point.
- **Search:** Mail searches with `Email/query`, which only Mail can do. A
  search across apps on the home page would ask each app and is an open
  point.
- **Backups:** the hub backs up its own database; each app its own until the
  hub coordinates them (open point). The mail, events and contacts are
  Stalwart's to back up.
- **Account colour and default account** belong to the Stalwart account, so
  the hub keeps them; every app shows them the same way.
- **App settings:** the hub keeps the settings that every app applies (§ 9.2,
  milestone 9). An app's own settings stay in the app's database; the hub
  offers no generic settings store.
- **About, licences, developer page:** the components are shared, the
  content (version, licence list, log) is each app's.
- **New-build detection and the offline app shell** are shared helpers; every
  app registers its own service worker under its prefix.
- **Push for new mail** is JMAP push from Stalwart to Mail's service worker,
  not a hub notification; `@roedy/jmap` has the shared part.
- **Imports and exports** (mbox, iCalendar, vCard) and attachments are each
  app's; JMAP blobs live in Stalwart, so the hub needs no blob storage.

### 9.3 Stalwart

Mail, Calendar and Contacts talk to the same Stalwart server: JMAP for Mail
(RFC 8621), Calendars and Contacts (RFC 9610), one account and one OAuth grant
per person and server. The hub owns what is the same for all of them:

- **Servers:** the registry with the OAuth client per server (DCR with an
  administrator's credentials, confidential clients with their secret in the
  hub), the policy for custom servers (public addresses only, the SSRF guard
  of Mail's `broker::outbound`), the diagnosis and its daily run. The system
  mail's sending account picks a server from the registry.
- **Accounts:** each user's connected accounts (several, one default, a
  colour), the sign-in at Stalwart with PKCE (the code exchanged by the hub for
  confidential clients), "Sign in again", removal with revocation, the check
  and the storage per account.
- **Tokens:** refresh tokens only in the hub, encrypted with the instance key;
  the token broker with Stalwart's strict rotation, early refresh and one
  refresh per account at a time. Access tokens are short-lived; a refresh
  token never leaves the hub.

What the apps get (details with milestone 8):

- **In the browser:** the hub's browser API lists the user's accounts (with
  the JMAP session URL, state and the capabilities the server offers) and
  hands out an access token per account. Apps share the hub's origin, so
  their pages call it with the hub's session like the other browser API
  (contract § 6). Apps add the servers' origins to their own `connect-src`.
  Script in any app on the origin can fetch these tokens, which the trust
  model already accepts (contract § 2: only apps trusted like the hub).
- **In the background:** an internal endpoint gives an app's backend an
  access token for a user's account – with the app's secret, only for users
  with access to the app and only if its manifest asks for it.
- **Events** when an account is added, removed or needs a new sign-in.

What stays with the apps: everything above the JMAP session – their data
types, their own push subscriptions at Stalwart, and per-account settings of
their own (e.g. Mail's identities).

No JMAP proxy: as in Mail, the browser talks to Stalwart directly (requests,
EventSource, uploads and downloads of blobs). A proxy would carry all mail
traffic through the hub against the budgets, and Stalwart has to be
reachable from the browsers for webmail anyway.

### 9.4 Shared interface and packages

- **Frame served by the hub (run time):** web components under a versioned
  path of the hub, which every app loads with one script: the app switcher
  (milestone 7, done), the notification bell (10) and the
  notice bar for the banner and maintenance mode (11). They always match the
  running hub, and apps need no new build when the hub changes them. They read
  the design tokens' custom properties, which reach into the shadow DOM.
- **Packages (build time),** built and tested in this repository and
  published with every hub release under the hub's version (`0.x`
  pre-releases until the first release):
  - `@roedy/tokens` – `packages/tokens` becomes the source; Mail's copy is
    archived with it.
  - `@roedy/ui` – Svelte 5 components, actions and helpers (§ 9.2), published
    as source with `@sveltejs/package`, with their own English and German
    texts so that apps need no message keys for them. The hub's web app uses
    the same package from the workspace.
  - `@roedy/jmap` – JMAP for the browser: the core, contact cards and
    calendars. Mail-specific types (mail, Sieve) stay in Mail.
  - Crates `roedy-hub-contract` (identity and event verification, manifest,
    vectors) and `roedy-jmap` (JMAP for servers).
- **Publishing:** the release workflow publishes to npm with provenance and
  to crates.io. The maintainer sets up the npm scope and the crates.io
  ownership before the first publication; nothing is created without asking.

Options considered: everything as packages (the frame would only change when
every app is rebuilt), everything as web components (shadow DOM for every
component, a second Svelte runtime per page, weaker types), Git dependencies
(see § 9.5).

### 9.5 Repositories

| Option | For | Against |
|---|---|---|
| **Separate repositories, published packages (decided)** | Clear boundary: apps use only the contract and published packages, as an app from someone else would; releases and changelogs per app | A shared change needs a hub release and an update in the app; a release pipeline for packages |
| Apps in this repository (monorepo) | Hub and app change in one PR, no package releases, one CI | The boundary is a convention only; one repository grows with every app |
| Separate repositories, Git dependencies | No registry | Pinned commits, slower installs, weak Dependabot support |

Consequences: the new Mail (and later Calendar and Contacts) lives in a
repository of its own – created by the maintainer; this repository publishes
`@roedy/*` and the two crates; the example app stays the reference for the
contract. Every image is built and released in its own repository.

### 9.6 Milestones

Ordered by what the new Mail needs first: it can start once 7 and 8.1 are
merged. Each item is one PR; "API" and "pages" are separate PRs where named.

7. [x] **Shared interface and packages**
   1. [x] **App switcher** – the first web component of the hub's frame.
      Done: `<roedy-app-switcher>` in `GET /api/v1/frame.js` (contract
      § 5.5), source in `apps/web/src/frame`, used by the example app.
   2. [x] **Packages** – `packages/ui` from the hub's web app and Mail's
      shared components, actions and helpers (§ 9.2); `packages/jmap` from
      Mail's shared JMAP layer; `packages/tokens` as the source. The hub's web
      app uses them from the workspace.
      Done: `@roedy/ui` with the hub's shell components, actions and helpers
      moved out of `apps/web` plus Mail's command palette, shortcuts,
      licences and debug dialogs, time picker, undo toasts, network state,
      developer log, app worker and date; own English and German texts
      (`texts()`), the app's language and colour scheme via
      `connectSettings()`, `base.css`, test helpers (`@roedy/ui/testing`).
      `@roedy/jmap` without mail and Sieve, extensible by declaration merging
      and `registerCapabilities()`, with room for calendars.
      `@roedy/tokens` is the source. Left for later: Mail's time field
      around the picker, the palette as an ARIA combobox, building
      `licenses.json` in the hub.
   3. [x] **Publishing** – packing (`@sveltejs/package`), versions with the
      hub's release, npm with provenance and crates.io in the release
      workflow, a dry run in CI.
      Done: `pnpm pack` builds the packages (`svelte-package` for
      `@roedy/ui`, `tsc` for `@roedy/jmap` and `@roedy/tokens`) and switches
      their exports from the sources to `dist/`; the root `package.json`
      holds the one version, `just release` writes it to the packages and
      the crates (`scripts/versions.test.mjs`). `scripts/packages.sh` packs,
      checks the tarballs in a SvelteKit app outside the workspace
      (`scripts/packages/consumer`), and publishes; the crates have their
      crates.io metadata. CI job "Packages" on every pull request (part of
      "CI passed"), job "Packages" in `release.yml` after the image on
      release tags (`NPM_TOKEN` or npm trusted publishing,
      `CARGO_REGISTRY_TOKEN`). The npm scope `@roedy`, the tokens and the
      crates.io ownership are the maintainer's to set up before the first
      release (CONTRIBUTING.md › Packages and crates).
8. [ ] **Stalwart servers, accounts and tokens**
   1. [x] **API** – registry and policy, diagnosis and its daily job, DCR,
      accounts with OAuth and PKCE, the token broker with early refresh and
      revocation, storage, the system mail's server from the registry, the
      browser and internal endpoints, events, the manifest field.
      Done: `stalwart` (Mail's `broker`: probe, DCR, OAuth, SSRF guard,
      token broker with strict rotation) and `api::stalwart` – the registry
      for administrators (DCR for a saved server, the secret straight into
      it), the sign-in started by the hub with `state` and the PKCE verifier
      kept in `stalwart_sign_ins` and the code exchanged by the hub (Mail's
      browser did both), accounts with an access token on custom servers,
      default account and colour, check, removal and deletion of the user
      with revocation; jobs `server_probe`, `token_refresh`, `quota_check`
      (without "storage almost full", which comes with notifications). For
      the apps (contract § 6.1, § 7.1, § 8, § 10.2): `GET
      /api/v1/stalwart/accounts`, `POST …/<id>/token`, the internal
      endpoints for the servers' origins, a user's accounts and tokens
      (`"stalwart": {"backend": true}`), the events `stalwart.account.added`,
      `.removed`, `.needs_sign_in`, `.signed_in` and
      `stalwart.servers.changed`; the example app reads them. The system
      mail names a registry server (`server_id`); Administration › Email picks
      it from the registry. Scopes for every app (mail, contacts,
      calendars). No new dependency.
   2. [ ] **Pages** – Administration › Stalwart servers, Account › Accounts,
      the sign-in at Stalwart and its return, "No account yet".
9. [ ] **Shared settings and directory** – the settings of § 9.2 with their
   pages under Account, handed to the apps (browser API, the users endpoint,
   `roedy.hub.prefs`), the recipient's time zone and formats in the hub's
   mails, the directory of active users for the apps.
10. [ ] **Notifications**
    1. [ ] **Bell** – notifications with topics from the apps' manifests,
       an internal endpoint to send them, channels per topic, retention, the
       bell as a web component of the frame and on the home page, the hub's
       security notices in it.
    2. [ ] **Email and web push** – digest and quiet hours, VAPID, encryption,
       subscriptions per session, the hub's service worker for notifications.
11. [ ] **Operations** – one PR each:
    1. [ ] **Switching apps on and off** – Administration › Apps and the
       gateway (`503`), Compose profiles per app.
    2. [ ] **Maintenance mode and banner** – for the hub and through the
       gateway for every app, the notice bar in the frame.
    3. [ ] **Update check** – the hub's releases, and the paired apps'
       versions.
    4. [ ] **Backups in the app** – the hub's database.
    5. [ ] **GeoIP** – places for sessions, the audit log and security
       notices.
