# Roedy Hub app contract – version 1

> **Status: version 1, agreed on 2026-11-25.** The hub implements it in the
> milestones of [`plan.md`](plan.md); until the first release, details may
> still be corrected here, and every correction is noted in the PR that makes it.

This document is everything an app needs to run behind Roedy Hub. It is written
so that an app can implement it without reading the hub's code. The key words
MUST, MUST NOT, SHOULD and MAY are used as in RFC 2119.

## Contents

1. [Terms](#1-terms)
2. [Trust model](#2-trust-model)
3. [Routing](#3-routing)
4. [Identity](#4-identity)
5. [Signing in and out](#5-signing-in-and-out)
6. [Browser API of the hub](#6-browser-api-of-the-hub)
7. [Internal API](#7-internal-api)
8. [Manifest](#8-manifest)
9. [Pairing](#9-pairing)
10. [Events](#10-events)
11. [Access and roles](#11-access-and-roles)
12. [Errors](#12-errors)
13. [Obligations of an app (checklist)](#13-obligations-of-an-app-checklist)
14. [Versioning and extensions](#14-versioning-and-extensions)
15. [Examples](#15-examples)

## 1. Terms

| Term | Meaning |
|---|---|
| **Hub** | Roedy Hub: gateway, sign-in and home page at the root of the public origin, e.g. `https://app.example.com`. |
| **App** | A Roedy app (Mail, Contacts, …) that runs in its own container and is reached through the hub. |
| **Slug** | Path prefix of an app, e.g. `mail`. Format `^[a-z][a-z0-9-]{0,30}[a-z0-9]$` (2–32 characters), not one of the [reserved names](#reserved-paths). Chosen by the administrator in the hub. |
| **Public URL of the app** | `<hub public URL>/<slug>`, e.g. `https://app.example.com/mail`. The app's `ROEDY_PUBLIC_URL` MUST be exactly this. |
| **Internal address** | Scheme, host and port at which the hub reaches the app, e.g. `http://roedy-mail:8080`. No path. |
| **Hub internal address** | Address of the hub's internal listener, e.g. `http://roedy-hub:8081` (`ROEDY_INTERNAL_LISTEN` on the hub). |
| **Hub mode** | The app runs behind the hub and leaves sign-in, users and permissions to it. The opposite is **standalone**. |
| **Issuer** | The hub's public URL without trailing slash, e.g. `https://app.example.com`. Value of `iss`. |

## 2. Trust model

- **One origin for all apps.** The hub and every app share the origin
  `https://app.example.com`. The browser does not isolate them: script running
  in one app (including an XSS) can read and change the client-side storage of
  every other app and call every app's API with the user's session. **Only
  install apps you trust as much as the hub itself.** The namespace rules in
  [§ 3.6](#36-namespaces-in-the-browser) prevent accidents, not attacks.
- **The hub is the only way in.** In hub mode the app's port MUST be reachable
  only by the hub (a private container network), never from the internet or
  from other untrusted hosts. The app nevertheless verifies every identity
  header cryptographically ([§ 4](#4-identity)).
- **The internal listener is private.** The hub's internal API
  ([§ 7](#7-internal-api)) is reachable only on the container network and is
  never published.
- **The hub decides who may use an app; the app decides what they may do in
  it,** based on `app_role` ([§ 11](#11-access-and-roles)).

## 3. Routing

### 3.1 Path prefix

The hub forwards every request whose path is `/<slug>` or starts with
`/<slug>/` to the app's internal address **with the path unchanged**:

```
https://app.example.com/mail/api/v1/messages?x=1  →  http://roedy-mail:8080/mail/api/v1/messages?x=1
```

The app MUST serve everything under its prefix: pages, assets, API, service
worker, web app manifest, health endpoints. It derives the prefix from the path
of its `ROEDY_PUBLIC_URL`. A request for `/<slug>` without trailing slash is
the app's business; it SHOULD redirect to `/<slug>/`.

### Reserved paths

These first path segments belong to the hub and can never be a slug:

`login`, `account`, `admin`, `setup`, `invite`, `api`, `internal`, `healthz`,
`readyz`, `.well-known`, `_hub` (the hub's web app assets), `favicon.ico`,
`robots.txt`, and, kept free for later: `apps`, `auth`, `branding`, `error`,
`help`, `hub`, `logout`, `oauth`, `settings`, `static`.

Inside an app's prefix, `/<slug>/_roedy/` is reserved for calls from the hub
to the app ([§ 10](#10-events)). The hub answers 404 to any request from
outside for this path and never forwards it – also in the spellings a server
may normalize to it (percent-encoded, with empty, `.` or `..` segments).

### 3.2 Request headers the hub sends to the app

| Header | What the hub does |
|---|---|
| `X-Roedy-*` (any) | Removes all of them from the client's request. Then sets its own (below). |
| `X-Roedy-Identity` | Signed identity, only when the request carries a valid hub session ([§ 4](#4-identity)). |
| `X-Roedy-Request-Id` | A ULID per request. The app SHOULD log it. |
| `Cookie` | Forwards only cookies in the app's namespace (names starting with `__Host-roedy-<slug>_`, [§ 3.6](#36-namespaces-in-the-browser)). The hub's session cookie `__Host-roedy_hub_session` and other apps' cookies never reach the app. |
| `Host` | The public host, e.g. `app.example.com`. |
| `X-Forwarded-Host`, `X-Forwarded-Proto` | Public host and scheme from the hub's `ROEDY_PUBLIC_URL`; values from the client are replaced. |
| `X-Forwarded-For` | The entries the hub trusts (per its `ROEDY_TRUST_PROXY`) plus the address of the hub's TCP peer; untrusted entries are dropped. The app sets **its** `ROEDY_TRUST_PROXY` to the hub's value plus one (e.g. TLS proxy → hub → app: `2`). |
| `Forwarded` | Removed (RFC 7239; not used). |
| Hop-by-hop headers | Removed as in RFC 9110 § 7.6.1, except for WebSocket upgrades ([§ 3.5](#35-streaming-and-websocket)). |
| Everything else | Unchanged, in particular `Origin`, `Referer`, `Sec-Fetch-*`, `X-Requested-With`, `Authorization`, `Accept-Language`. |

### 3.3 Response headers the hub changes

The app's response passes through unchanged except:

| Header | What the hub does |
|---|---|
| `Set-Cookie` | Drops every cookie whose name is not in the app's namespace (logged as a warning). |
| `Clear-Site-Data` | Removed: it would clear every app on the origin. |
| `Service-Worker-Allowed` | Removed: an app's service worker scope stays inside its prefix. |
| `Strict-Transport-Security` | Replaced by the hub's own (sent when the hub terminates TLS itself or runs behind a TLS proxy). |
| Hop-by-hop headers | Removed. |
| `X-Content-Type-Options` | Set to `nosniff` if the app sent none. |

Content Security Policy, `Cross-Origin-Opener-Policy`, `Referrer-Policy`,
`Permissions-Policy` and cache headers are the app's own; the hub does not add
or replace them. The app SHOULD send a CSP with `frame-ancestors 'self'` or
stricter.

### 3.4 Limits and timeouts

| Limit | Value |
|---|---|
| Connect to the app (name lookup, TCP, TLS) | 5 s, then 502 `app_unreachable` |
| Until the app's response headers | 300 s after the hub has taken the request body's last part to send it (an earlier response counts at once), then 504 `app_timeout`. The upload itself has no time limit; an app that stops reading the last bytes counts as not answering. |
| Response and request bodies | Streamed in both directions without buffering; no total time limit, no size limit at the hub |
| Request headers | 64 KiB in total, then 431 `headers_too_large` |
| Protocol to the app | HTTP/1.1 (the hub does not speak h2c to apps) |
| Compression | None by the hub; `Accept-Encoding` and `Content-Encoding` pass through, so an app compresses its responses itself if it wants (a TLS proxy in front, if any, may compress as well) |

The hub, or a TLS proxy in front of it, may close idle connections. Apps SHOULD send
a keep-alive at least every 30 s on long-lived streams (SSE comment line,
WebSocket ping).

### 3.5 Streaming and WebSocket

- **Server-Sent Events and downloads:** the hub passes every chunk on as soon
  as it arrives.
- **Uploads:** the hub streams the request body to the app as it arrives;
  `Expect: 100-continue` is passed on.
- **WebSocket:** the hub forwards `GET` requests with `Connection: upgrade` and
  `Upgrade: websocket` including all `Sec-WebSocket-*` headers, and after the
  app's `101` relays both directions. The hub refuses an upgrade whose `Origin`
  is present and is not the hub's public origin (403). The app MUST check
  `Origin` as well.
- **End of a session:** when a hub session ends (sign-out, revoked, user
  locked or deleted), the hub closes the session's open streams and WebSocket
  connections to all apps within 5 seconds. Sessions that end without the
  hub's own action (they expire, or another process such as the hub's
  command line ends them) are found by a periodic check; while the hub's
  database is too busy to answer that check, their streams stay open until
  it does. Apps learn about every ended session from `session.revoked`
  ([§ 10](#10-events)) as well.
- **End of an access:** when an administrator removes a user's access to an
  app, the hub closes that user's open streams and WebSocket connections to
  this app at once (their other apps' stay open); the app learns about it
  from `access.changed` with `access: false`. A new role applies to the
  next request, whose identity carries it.

### 3.6 Namespaces in the browser

All apps share one origin, so everything the browser stores per origin MUST
carry the app's slug. `<slug>` is the app's slug as configured in the hub
(not its manifest `id`): two instances of one app get separate namespaces.
Every prefix ends with a character no slug can contain (`.` or `_`), so that
a name belongs to exactly one slug: `roedy.mail.` is not a prefix of
`roedy.mail-archive.…`.

| What | Rule |
|---|---|
| URLs | Only below `/<slug>/`, plus the hub endpoints of [§ 6](#6-browser-api-of-the-hub) and the links of [§ 5](#5-signing-in-and-out). |
| `localStorage`, `sessionStorage` | Keys start with `roedy.<slug>.` |
| IndexedDB | Database names start with `roedy.<slug>.` |
| Cache Storage | Cache names start with `roedy.<slug>.` |
| `BroadcastChannel`, Web Locks | Names start with `roedy.<slug>.` |
| Service worker | Script under `/<slug>/`, scope `/<slug>/` (the default for a script there) |
| Web app manifest | Served under `/<slug>/`; `id`, `scope` and `start_url` under `/<slug>/` |
| Cookies | Apps SHOULD NOT set cookies in hub mode. If they must: name starts with `__Host-roedy-<slug>_` (e.g. `__Host-roedy-mail_token`), `Path=/`, `Secure`, `HttpOnly`, `SameSite=Lax` or `Strict`. Other cookies are dropped by the hub ([§ 3.3](#33-response-headers-the-hub-changes)). |

The hub itself uses the namespace `hub` (`roedy.hub.`; its session cookie
`__Host-roedy_hub_session` lies outside every app's namespace). Apps MAY
read `localStorage["roedy.hub.prefs"]` – JSON `{"theme": "system"|"light"|"dark",
"locale": "en"|"de"}` – to apply theme and language before their first request.
They MUST NOT write any key outside their namespace.

## 4. Identity

### 4.1 The header

Every forwarded request with a valid hub session carries exactly one
`X-Roedy-Identity` header: a JWT (RFC 7519) in compact JWS serialization,
signed with Ed25519.

Requests without a session, with an expired or revoked session, or from a
locked user carry **no** identity header. The hub never forwards a request of
a signed-in user who has no access to the app ([§ 11](#11-access-and-roles));
it answers 403 itself.

### 4.2 Format

JOSE header:

```json
{ "alg": "EdDSA", "typ": "roedy-id+jwt", "kid": "01JDX3…" }
```

Header, claims and signature are base64url without padding, joined by dots
(RFC 7515 compact serialization); header and claims are JSON objects.

Claims:

| Claim | Type | Meaning |
|---|---|---|
| `iss` | string | Issuer: the hub's public URL without trailing slash, e.g. `https://app.example.com` |
| `aud` | string | The app's slug, e.g. `mail` (always a string, never an array) |
| `sub` | string | User ID: a ULID, stable for the user's lifetime; never reused |
| `username` | string | Login name, lowercase, 2–32 characters; may change |
| `email` | string | Confirmed email address; may change |
| `name` | string | Display name; may change |
| `role` | string | Hub role: `admin` or `user` |
| `app_role` | string | The user's role in this app, an `id` from the app's manifest |
| `locale` | string | The user's language, `en` or `de` |
| `sid` | string | ID of the hub session (ULID); not secret |
| `auth_time` | number | When the user last entered their password (and second factor) in this session, Unix seconds |
| `iat` | number | Issued at, Unix seconds |
| `exp` | number | Expires, Unix seconds; `exp - iat` ≤ 60 |

Apps MUST ignore claims they do not know.

### 4.3 Verification

For every request, the app:

1. takes the `X-Roedy-Identity` header; none, or more than one, means **not
   signed in**;
2. parses the compact JWS; the header MUST have `alg` = `EdDSA`,
   `typ` = `roedy-id+jwt` and a `kid`. Any other `alg` (including `none`) is
   rejected, regardless of the key. Values compare exactly (case-sensitive).
   A header with `crit` is rejected: the contract defines no extensions;
3. looks up `kid` in its cached JWKS; if unknown, refreshes the JWKS
   ([§ 4.4](#44-keys-jwks)) at most once per 30 s and tries again;
4. verifies the Ed25519 signature over the signing input;
5. checks `iss` = the issuer stored at pairing, `aud` = its own slug,
   `exp` > now − 10 s, `iat` ≤ now + 10 s and 0 ≤ `exp − iat` ≤ 60.

The test vectors ([§ 15](#15-examples)) cover each step.

If any step fails, the request counts as **not signed in** – the app does not
answer 400 for it. It SHOULD log the reason at most once a minute (a failing
signature usually means the hub was re-installed or the app needs re-pairing).

### 4.4 Keys (JWKS)

The hub signs with Ed25519 keys and publishes their public parts as a JWK Set
(RFC 7517, OKP keys per RFC 8037):

```json
{
  "keys": [
    { "kty": "OKP", "crv": "Ed25519", "x": "11qYAYKxCrfVS_7TyWQHOg7hcvPapiMlrwIaaPcHURo",
      "kid": "01JDX3…", "use": "sig", "alg": "EdDSA" }
  ]
}
```

- The app gets the JWKS in the pairing response and from
  `GET /internal/v1/jwks` ([§ 7](#7-internal-api)). It keeps a copy across
  restarts and refreshes it at least every hour and whenever it meets an
  unknown `kid`.
- **Rotation:** the hub publishes a new key at least 10 minutes before it
  signs with it, and keeps a retired key in the set for at least 10 minutes
  after its last signature. An app that refreshes as above never rejects a
  valid header. The hub rotates every 90 days; apps MUST NOT rely on that
  interval.
- The one exception: if the hub can no longer decrypt its key (its instance
  key was replaced), it signs with a new key at once. Apps pick it up through
  the refresh for an unknown `kid`.
- Only `EdDSA` keys with `use` = `sig` are valid.

### 4.5 Users in the app

- The app identifies users **only by `sub`**. Username and email can change.
- On the first request of an unknown `sub`, the app creates its local user
  record (just in time). When `username`, `email`, `name`, `locale` or
  `app_role` differ from the stored values, it updates them.
- Permissions inside the app come from `app_role` alone. The hub role `admin`
  does **not** make anyone an administrator of the app.
- In hub mode the app MUST NOT accept any sign-in of its own (passwords,
  tokens, its own session cookies): otherwise a user locked in the hub could
  still get in. Features that belong to the identity – profile, password,
  two-factor authentication, signed-in devices, users, invitations – are
  hidden in the app and link to the hub instead ([§ 5.4](#54-links-to-the-hub)).

## 5. Signing in and out

### 5.1 Not signed in

The hub forwards requests without a session without identity. The app decides
what is public (for example its HTML shell, assets, health endpoints, images
that mail clients load) and answers everything else with **401**.

When the app learns that the user is not signed in – a 401 from its own API or
a page that needs a user – it navigates the whole page to:

```
/login?next=<path, query and fragment of the current page, URL-encoded>
```

Example: `/login?next=%2Fmail%2Fmailbox%2Finbox`. The hub only follows `next`
when it is a path on the same origin (starts with exactly one `/`, contains no
`\`, no control characters); otherwise it goes to `/` after sign-in.

### 5.2 Fresh sign-in for sensitive actions

For an action that needs proof of a recent sign-in (such as deleting data
irreversibly), the app compares `auth_time` with the current time. If it is
too old (the app's choice, e.g. 10 minutes), it navigates to
`/login?reauth=1&next=…`; the hub asks for the password and the second factor
again and returns with a new `auth_time`. While the session is still valid,
it stays the same, with the same `sid`; if it ended in the meantime (expired,
revoked), the sign-in starts a new session with a new `sid`.

### 5.3 Signing out

The app signs out through the hub:

```http
POST /api/v1/auth/logout
X-Requested-With: fetch
```

→ `204`. The hub ends the session, removes its cookie, sends
`Clear-Site-Data: "cache", "storage"` for the whole origin, sends
`session.revoked` to all apps ([§ 10](#10-events)) and closes the session's
open streams. The app then navigates to `/login`. Apps MUST cope with their
client-side storage being cleared at sign-out.

### 5.4 Links to the hub

The app MUST show a visible link to the hub's home page `/` in its main
navigation. It SHOULD offer the app switcher of the hub's
frame ([§ 5.5](#55-the-hubs-frame)) rather than building its own from
`GET /api/v1/apps` ([§ 6](#6-browser-api-of-the-hub)). Other links into the hub:

| Purpose | Link |
|---|---|
| Home page with all apps | `/` |
| Profile, language, theme | `/account` |
| Password, two-factor authentication, signed-in devices | `/account/security` |
| Administration (only for `role` = `admin`) | `/admin` |
| Access to this app (only for `role` = `admin`) | `/admin/apps/<slug>` |

### 5.5 The hub's frame

What every app needs around its own content belongs to the hub, so that the
apps look and behave alike and contain only what is their own. The hub serves
these shared parts as web components in one dependency-free script, the
**frame**. Including it defines the elements; the app places them in its
header:

```html
<script type="module" src="/api/v1/frame.js"></script>
…
<roedy-app-switcher></roedy-app-switcher>
```

For the frame as a whole:

- **CSP:** it needs `script-src 'self'`, `connect-src 'self'` (it reads the
  endpoints of [§ 6](#6-browser-api-of-the-hub) with the session cookie) and
  `img-src 'self'` (the apps' icons). It has no inline script or style and
  needs no `style-src` (its styles are constructed style sheets in shadow
  roots) and no `font-src`; it makes no request to another origin. This is
  the policy of the example app:
  `default-src 'none'; script-src 'self'; img-src 'self'; connect-src 'self'; frame-ancestors 'self'`.
- **Look:** the design tokens of `@roedy/tokens`. Custom properties reach
  into the shadow DOM: an app that sets the tokens' properties on its page
  (`--pop`, `--fg`, `--acc`, `--font-sans` …) gets its colours, accent, mode
  and fonts in the frame as well. Where a property is not set, the frame uses
  the tokens' values for light or dark: those of `theme="light"` or
  `theme="dark"` on the element, else of `prefers-color-scheme`. Further
  adjustments go through `::part()`.
- **Language:** English or German (`de…`) from `lang` on the element, else
  the closest `lang` in the page, else the browser's language.
- **Updates:** the address stays; the hub revalidates the script on every
  load (`Cache-Control: no-cache` with an `ETag`), so that every app shows
  the hub's current frame without a release of its own.

#### App switcher: `<roedy-app-switcher>`

A 40 × 40 px button that opens a menu with the apps the user can open (icon
and name, the current app marked, `Offline` for an app whose health check
failed) and the links of [§ 5.4](#54-links-to-the-hub) to the home page, the
account and, for administrators, the administration. Signed out, it offers
`/login?next=<the current page>`; when the hub cannot be reached, a retry.

- **Attributes,** all optional:

  | Attribute | Meaning |
  |---|---|
  | `current` | Slug of the current app; default the first segment of the page's path. |
  | `theme` | `light` or `dark` for the tokens the page does not set; default `prefers-color-scheme`. |
  | `lang` | Language of its texts (see above). |
  | `align` | `end` (default) or `start`: the edge of the button the menu lines up with. |

- **Script interface:** the property `open` (whether the menu is open), the
  methods `close()` and `refresh()` (fetch the apps again; the switcher does
  so by itself when the menu opens and its data is older than 30 s), and the
  event `toggle` (does not bubble) when the menu opens or closes.
- **Parts:** `button` and `menu`.
- **Behaviour:** the menu button pattern of WAI-ARIA: Enter, Space or ↓ open
  the menu on its first item, ↑ on its last; in the menu the arrow keys move
  (↑/↓ by row, ←/→ one by one), Home and End go to the ends, a letter to the
  next item starting with it, Escape closes it and puts focus back on the
  button, Tab or a click outside closes it.

## 6. Browser API of the hub

These endpoints are for the app's web front end on the same origin. They use
the hub's session cookie, which the browser sends by itself. The hub does not
allow CORS. Modifying requests need a non-empty `X-Requested-With` header
(otherwise 403 `csrf`). Errors are JSON `{"error": "<code>"}`.

Only the endpoints listed here are part of the contract; other paths under
`/api/v1` belong to the hub's own web app and may change.

### `GET /api/v1/session`

`200` while signed in, otherwise `401 {"error": "unauthorized"}`.

```json
{
  "user": {
    "id": "01JDX2Q7…", "username": "alex", "email": "alex@example.com",
    "name": "Alex Example", "role": "user", "locale": "de", "theme": "system"
  },
  "session": { "id": "01JDX3…", "auth_time": 1795600000 },
  "hub": { "name": "Example Org", "home": "/", "account": "/account", "admin": null }
}
```

`hub.name` is the organization's name from the hub's administration
(`"Roedy Hub"` while none is set); `hub.admin` is `"/admin"` for
administrators, else `null`.

### `GET /api/v1/apps`

The apps the signed-in user has access to, in the order the administrator set,
names and descriptions in the user's language (`user.locale` of
`GET /api/v1/session`); `401 {"error": "unauthorized"}` when signed out:

```json
{
  "apps": [
    { "slug": "mail", "name": "Mail", "description": "Email for the team",
      "url": "/mail/", "icon": "/api/v1/apps/mail/icon.svg", "status": "online" }
  ]
}
```

`status`: `online` (the last health check succeeded) or `offline` (it failed,
or there was none yet). `url` is `/<slug><home>` with `home` from the manifest.
`icon` is the address of the icon below, with its version.

Only paired apps are listed. An app that waits for pairing
([§ 9](#9-pairing)) is not, not even for hub administrators: it has no name or
icon yet, and they reach it through the hub's administration. A hub
administrator sees the apps they have access to, like everyone else.

### `GET /api/v1/apps/<slug>/icon.svg`

The app's icon from its manifest, sanitized by the hub, with
`Content-Security-Policy: default-src 'none'; style-src 'unsafe-inline'; sandbox`.
For use in `<img>`. Public, cacheable, the URL changes when the icon changes
(`?v=` with a hash).

`Content-Type: image/svg+xml`, an `ETag`, and with the current `v` (as
`GET /api/v1/apps` names it) `Cache-Control: public, max-age=31536000,
immutable`; without `v` or with an earlier one `no-cache`, so that the
browser revalidates. An app that waits for pairing, or an unknown slug, gets
`404 {"error": "not_found"}`. The icon has a square `viewBox` and scales to
any size.

### `GET /api/v1/frame.js`

The hub's frame ([§ 5.5](#55-the-hubs-frame)), an ES module for
`<script type="module">`. Public, `Content-Type: text/javascript;
charset=utf-8`, `Cache-Control: no-cache` with an `ETag`,
`Cross-Origin-Resource-Policy: same-origin`.

### `POST /api/v1/auth/logout`

See [§ 5.3](#53-signing-out).

## 7. Internal API

The hub serves the internal API on its own listener (`ROEDY_INTERNAL_LISTEN`,
default `0.0.0.0:8081`), separate from the public one, under `/internal/v1`.
The public listener answers 404 for `/internal/…`. All bodies are JSON
(`Content-Type: application/json`) unless stated otherwise; errors are
`{"error": "<code>"}` with optional further fields.

**Authentication:** except for `pair` and `jwks`, every request carries the
app secret from pairing:

```http
Authorization: Bearer rhs_01JDX4…_k3J9…
```

An unknown or revoked secret gets `401 {"error": "invalid_app_credentials"}`.
The app should then report "connection to the hub lost, pair again" in its
logs and its admin area.

| Method and path | Auth | Purpose |
|---|---|---|
| `POST /internal/v1/pair` | pairing code | Pair the app ([§ 9](#9-pairing)) |
| `GET /internal/v1/jwks` | – | Public signing keys ([§ 4.4](#44-keys-jwks)), `Cache-Control: max-age=300` |
| `PUT /internal/v1/manifest` | secret | Send the current manifest, on every start ([§ 8](#8-manifest)) |
| `GET /internal/v1/events?after=<seq>&limit=<n>` | secret | Read missed events ([§ 10.3](#103-catching-up)) |
| `GET /internal/v1/users?cursor=<c>&limit=<n>` | secret | Users with access to this app |
| `GET /internal/v1/users/<sub>` | secret | One user: with access, without access or deleted |

### `PUT /internal/v1/manifest`

Body: the manifest ([§ 8](#8-manifest)). Response `200`:

```json
{ "app_id": "01JDX4…", "slug": "mail", "issuer": "https://app.example.com",
  "public_url": "https://app.example.com/mail" }
```

If `public_url` differs from the app's `ROEDY_PUBLIC_URL` (the administrator
changed the slug), the app logs an error and SHOULD refuse to start.
Errors: `400 invalid_manifest` with `fields` ([§ 8](#8-manifest)),
`400 unsupported_contract` with `supported`, `401 invalid_app_credentials`.

### `GET /internal/v1/users`

For work without a request from the user (background mails, directories,
reconciling after missed events). Ordered by `sub`, `limit` 1–500 (default
200):

```json
{
  "users": [
    { "sub": "01JDX2Q7…", "username": "alex", "email": "alex@example.com",
      "name": "Alex Example", "role": "user", "app_role": "user",
      "locale": "de", "status": "active" }
  ],
  "next_cursor": null
}
```

`status`: `active` or `locked`. For the next page the app passes
`next_cursor` as `cursor`; it is `null` on the last page. A `limit` outside
1–500 gets `400 bad_request`. Users without access to the app are not
listed. **Absence from this list never means that a user was deleted** – only
that they have no access (now); an app MUST NOT delete a user's data because
of it.

### `GET /internal/v1/users/<sub>`

The authoritative answer for one user:

| Response | Meaning | The app should |
|---|---|---|
| `200` with the user as in the list | The user has access | update its copy; if `status` is `locked`, act as for `user.locked` |
| `404 {"error": "no_access"}` | The user exists but has no access to this app | keep the data, treat the user like a locked one in this app |
| `410 {"error": "user_deleted", "deleted_at": "…"}` | The user was deleted; the hub keeps this record (only `sub` and the time) permanently | delete the user's data as for `user.deleted` |

An unknown `sub` (never issued by this hub) also gets `404 no_access`.

## 8. Manifest

The app describes itself with a manifest. It sends it when pairing and on every
start (`PUT /internal/v1/manifest`), so that name, version and roles in the
hub stay current.

```json
{
  "contract": 1,
  "id": "roedy.mail",
  "version": "0.6.0",
  "name": { "en": "Mail", "de": "Mail" },
  "description": { "en": "Email for the team", "de": "E-Mail für das Team" },
  "icon": "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 24 24\">…</svg>",
  "home": "/",
  "roles": [
    { "id": "user", "name": { "en": "User", "de": "Benutzer" },
      "description": { "en": "Reads and sends email", "de": "Liest und sendet E-Mails" },
      "default": true },
    { "id": "admin", "name": { "en": "Administrator", "de": "Administrator" },
      "description": { "en": "Also manages mail servers", "de": "Verwaltet auch die Mailserver" },
      "admin": true }
  ]
}
```

| Field | Rules |
|---|---|
| `contract` | Integer, the contract version the app implements: `1`. The hub rejects versions it does not support (`400 unsupported_contract`). |
| `id` | Identifies the kind of app, `^[a-z][a-z0-9.-]{1,63}$`, e.g. `roedy.mail`. Several installations of the same app share it. |
| `version` | SemVer 2.0 of the app. |
| `name`, `description` | Objects by language; `en` and `de` required, further languages optional. `name` 1–40 characters, `description` up to 200. |
| `icon` | SVG document as a string, up to 64 KiB, square `viewBox`. The hub sanitizes it (no scripts, no event handlers, no external references, no `foreignObject`) and only ever shows it as an image. An SVG it cannot sanitize is rejected. |
| `home` | Start path relative to the prefix, default `/`. The tile links to `/<slug><home>`. |
| `roles` | 1–16 roles. `id` `^[a-z][a-z0-9_-]{0,31}$`, unique; `name` and `description` as above. Exactly one role has `"default": true`; at most one has `"admin": true` (the role that gives full control of the app). |

Unknown fields are ignored; names starting with `x-` are reserved for
experiments.

**Errors.** A `contract` version the hub does not implement gets
`400 unsupported_contract` with `supported` (the versions it does), whatever
else the manifest holds. Any other manifest that breaks these rules gets
`400 invalid_manifest` with `fields`: per broken field its path (`id`,
`name.de`, `roles.1.id`, `icon`, …) and `{"code": "<code>"}`, for example
`{"error": "invalid_manifest", "fields": {"roles": {"code": "default_role"}}}`.
The codes: `required`, `invalid`, `too_long`, `too_many` (more than 16
roles), `duplicate` (a role `id` twice), `default_role` (not exactly one
default role), `admin_role` (more than one admin role); for `icon` also
`too_large` (over 64 KiB, or too complex to draw), `not_square` (no square
`viewBox`), `svg_text` (text elements: the hub draws icons without fonts)
and `unreadable` (not an SVG the hub can sanitize, or nothing left to draw
after sanitizing, such as only external images). The hub stores the icon
as it sanitized it.

**Changed roles:** when a manifest no longer contains a role, users who had it
fall back to the default role (with an entry in the hub's audit log and a
`access.changed` event).

## 9. Pairing

Pairing connects an app installation to the hub once and gives it its
credentials.

```
Administrator          Hub                                  App
     │ Admin › Apps › Add │                                   │
     │ (slug, internal    │                                   │
     │  address) ────────►│ app "waiting for pairing",        │
     │◄── pairing code ───│ code valid 15 min                 │
     │                                                        │
     │ gives the code to the app (environment or the app's setup)
     │ ──────────────────────────────────────────────────────►│
     │                    │◄── POST /internal/v1/pair ─────────│
     │                    │    code, public_url, manifest      │
     │                    │── app_id, app_secret, issuer, ────►│
     │                    │   JWKS                             │ stores them
     │                    │── GET /<slug>/readyz (health) ────►│
     │◄── "Connected" ────│                                    │
```

### Pairing code

- Shown once to the administrator, valid for **15 minutes** and for a single
  successful use. Generating a new code invalidates the previous one.
- Format: `rhp_` followed by 32 characters of Crockford Base32 (160 random
  bits), displayed in groups of four; the app accepts it with or without
  spaces and hyphens, in any case.
- The hub stores only its SHA-256.

### `POST /internal/v1/pair`

```json
{
  "code": "rhp_7K2M…",
  "public_url": "https://app.example.com/mail",
  "manifest": { "contract": 1, "id": "roedy.mail", … }
}
```

The hub checks, in this order: rate limit (10 attempts per 15 minutes per
source address, 30 in total), the code (unknown, used and expired codes give
the same error), the manifest, and that `public_url` equals
`<hub public URL>/<slug>`.

Every attempt counts towards the rate limit, successful or not; an attempt
refused by the limit per source address does not count towards the total.
Only a pairing that succeeds
uses up the code: after `invalid_manifest`, `unsupported_contract` or
`public_url_mismatch` the app can correct what is wrong and try again with
the same code while it is valid. `public_url` is compared exactly, without
a trailing slash.

Response `200`:

```json
{
  "app_id": "01JDX4…",
  "slug": "mail",
  "app_secret": "rhs_01JDX4…_k3J9…",
  "issuer": "https://app.example.com",
  "public_url": "https://app.example.com/mail",
  "jwks": { "keys": [ … ] },
  "contract": 1
}
```

The app MUST store `app_secret` like a password (encrypted at rest with its
instance key); the hub keeps only a hash and cannot show it again. It stores
`app_id`, `issuer`, its slug and the JWKS.

| Error | Meaning |
|---|---|
| `401 invalid_code` | Code unknown, used or expired |
| `409 public_url_mismatch` | `public_url` is wrong; the response names `expected` |
| `400 invalid_manifest` | Manifest breaks the rules; `fields` names them |
| `400 unsupported_contract` | Contract version not supported; `supported` lists the versions |
| `429 rate_limited` | Too many attempts; `retry_after` in seconds |

### Before and after pairing

- **Health:** right after pairing and then about every 30 seconds the hub
  sends `GET /<slug>/readyz` to the app's internal address. A `2xx` answer
  within 10 seconds counts as online
  ([§ 6](#6-browser-api-of-the-hub) `status`), anything else as offline.
- While an app waits for pairing, the hub forwards requests to it only for
  signed-in hub administrators (so that an administrator can complete the
  app's own setup through the hub, for example to enter the pairing code),
  without an identity header – the app does not know the hub's keys yet – and
  answers `503 app_not_ready` to everyone else.
- **Pairing again** (Admin › Apps › New pairing code) replaces `app_secret`
  once the app has used the new code; the old secret stops working at that
  moment. That is the way to recover an app that lost its data or its secret.
- **Removing an app** in the hub revokes its secret, deletes its access
  entries and stops forwarding. The app gets `401 invalid_app_credentials`
  from then on.

## 10. Events

The hub tells apps about changes to users and sessions, so that they can
clean up their own data.

### 10.1 Delivery

```http
POST /<slug>/_roedy/v1/events      (to the app's internal address)
Content-Type: application/jwt

eyJhbGciOiJFZERTQSIsInR5cCI6InJvZWR5LWV2ZW50K2p3dCIsImtpZCI6IjAxSkRYMy4uLiJ9.eyJ…
```

The body is one event as a JWT signed like the identity, with `typ` =
`roedy-event+jwt`:

| Claim | Meaning |
|---|---|
| `iss`, `aud` | As in [§ 4.2](#42-format) |
| `jti` | Event ID (ULID) |
| `seq` | Position in this app's event stream: an integer that grows by one per event for this app |
| `type` | Event type ([§ 10.2](#102-event-types)) |
| `data` | Object, depends on `type` |
| `iat`, `exp` | Signed at, expires; `exp − iat` ≤ 300. A retry is signed anew. |

The app verifies it as in [§ 4.3](#43-verification) with `typ` =
`roedy-event+jwt` and a lifetime of up to 300 s, processes it and answers
**`204`**. Any other answer, or none within 10 s, counts as a failure.

- **Order and retries:** the hub delivers an app's events one at a time in
  `seq` order; the next one only after the previous one was acknowledged.
  After a failure it retries after 10 s, 30 s, 1 min, 5 min, 15 min and then
  every hour, for up to 7 days. After that it waits until the app shows that
  it is back – it sends its manifest, catches up ([§ 10.3](#103-catching-up)),
  pairs again or answers its health check – and then tries the same event
  again at once.
- **Gaps:** an event the hub could not deliver within the retention of 30
  days is deleted, and the stream continues after it. A `seq` more than one
  above the highest processed therefore means that the app missed events: it
  catches up ([§ 10.3](#103-catching-up)) before it processes the delivery,
  or answers `503` until it can.
- **At least once:** an event can arrive more than once. The app stores the
  highest `seq` it has processed and acknowledges anything at or below it
  without acting again.
- **Unknown types** are acknowledged with `204` and otherwise ignored.
- Requests from outside to `/<slug>/_roedy/…` never reach the app
  ([§ 3.1](#31-path-prefix)); the signature check still applies.

### 10.2 Event types

All user events go to every paired app, whether or not the user has access to
it: the app may hold data from earlier access.

| `type` | `data` | The app should |
|---|---|---|
| `user.updated` | `{"sub", "username", "email", "name", "role", "locale"}` – the new values | update its copy of the user |
| `user.locked` | `{"sub"}` | end everything that runs for the user without a request: open streams, push subscriptions, scheduled sending. Keep the data. |
| `user.unlocked` | `{"sub"}` | record the user as no longer locked, so that writes for them are allowed again (also when replaying: if the user was unlocked after a replayed `user.locked`, the replay contains that `user.unlocked` as well) |
| `user.deleted` | `{"sub"}` | delete the user's data (or anonymize it, as the app documents) |
| `session.revoked` | `{"sub", "sid", "reason"}`; `reason`: `logout`, `revoked`, `expired`, `password_changed`, `locked` | end server-side state bound to this session (push subscriptions, streams) |
| `access.changed` | `{"sub", "access": true\|false, "app_role": "<id>"\|null}` – for this app only | apply the new role; on `false` treat the user like a locked one in this app, on `true` lift that again |

- Events start with pairing: the first ones are `access.changed` for the
  access granted then ([§ 11](#11-access-and-roles)). A pending app gets none.
- A new user reaches the apps they get access to as `access.changed` with
  `access: true`; there is no separate event for a new user. Their details
  come with the identity or from `GET /internal/v1/users/<sub>`.
- An administrator can grant, change and remove a user's access at any
  time; each change gives `access.changed` to that app alone – removal with
  `"access": false, "app_role": null`.
- `user.updated` follows a change of any of its fields, `locale` included
  (the user's language, or the language of their last sign-in while the
  setting follows the browser).
- `session.revoked` with `reason` `expired` follows when the hub cleans up
  expired sessions, up to an hour after the expiry; `revoked` also covers the
  least recently used session of a user who signs in with 20 sessions open.
- After `user.deleted` nothing more follows for the `sub`: the end of the
  user's sessions and access is part of it.

**Requests racing with events.** A request can check its identity before a
`session.revoked`, `user.locked` or `user.deleted` event is processed and try
to write afterwards. So that it cannot recreate what the event ended, the rule
below applies to **state that these events end**: push subscriptions,
scheduled work such as delayed sending, and anything else that acts for the
user without a request. Ordinary data the user saves (a draft, a setting) is
not affected; a locked or deleted user's sessions end, so no request of
theirs arrives afterwards.

- The app keeps the `sid`s of ended sessions (at least as long as the hub
  keeps events, 30 days) and whether each user is locked or deleted.
- It creates such state only in a transaction that checks that the `sid` has
  not ended, the user is neither locked nor deleted, and the request's
  identity has not expired yet (`exp` with the leeway of § 4.3). Every such
  write therefore happens within about a minute of a check by the hub, long
  before an ended `sid` is forgotten or leaves the event retention.
- Processing an event records the change and removes the state in one
  transaction.
- The two kinds of transaction MUST be serialized against each other: the
  check and the write of a request must not interleave with the processing of
  an event for the same `sid` or user. Serializable isolation, a write
  transaction that holds the database's write lock from its first read (SQLite
  `BEGIN IMMEDIATE`), or a lock per `sid` and user taken by both sides is
  enough; a plain read-then-write in a weaker isolation level is not.
- **Long-lived connections:** a stream or WebSocket keeps the identity of its
  opening request, which expires after at most 60 s. To create such state
  later, the client sends an ordinary HTTP request, which the hub gives a
  fresh identity. If the app receives the wish over the connection after the
  identity has expired, it does not create the state silently or drop it: it
  answers with an explicit error, such as a message with
  `"error": "identity_expired"`, and the client repeats it as an HTTP
  request.

### 10.3 Catching up

The hub keeps events for 30 days. On every start, after `PUT manifest`, the
app SHOULD read what it missed:

```http
GET /internal/v1/events?after=<highest seq processed>&limit=100
Authorization: Bearer rhs_…
```

```json
{ "events": ["<event JWT>", "…"], "more": false }
```

Each entry is an event JWT as in a delivery, oldest first, **signed anew when
it is read**: `jti`, `seq`, `type` and `data` are those of the original event,
`iat` is the time of the request and `exp − iat` ≤ 300. The app verifies it
exactly like a delivery. `after` is 0 or more (default 0), `limit` 1–100
(default 100), anything else gets `400 bad_request`; `more` is `true` when
further events follow. An `after` at or past the newest event gives an empty
list. Reading counts as acknowledging every event up to `after`: deliveries
continue after it, and one that failed is tried again at once.

If `after` lies before the retention, the hub answers
`410 {"error": "events_expired", "resume_after": <n>}`: `resume_after + 1` is
the oldest event the hub still keeps. The app has missed the events before it,
which it cannot get back, and **reconciles**, in this order:

1. It stops processing events: until step 5 it answers every delivery with
   `503`, so that the hub retries later and no event overtakes the
   reconciliation (deliveries are strictly ordered, § 10.1).
2. It ends **all** state bound to hub sessions, for every user – push
   subscriptions, open streams, anything else keyed by `sid` – because it
   cannot know which `session.revoked` events it missed. Users set it up
   again on their next visit with a valid identity.
3. It asks `GET /internal/v1/users/<sub>` for every user it holds data for
   and acts on the answer as described there.
4. It stores `resume_after` as the highest `seq` processed.
5. It processes deliveries again; the hub sends every event it still keeps,
   from `resume_after + 1` on. Events that the lookups in step 3 already
   reflect are applied once more, which is harmless: they come in order and
   end with the current state, and deletion is final (below).

Requests from browsers need no pause during reconciliation: their identity is
at most 60 s old, so its session was still valid long after every event the
app missed. If that session has been revoked since, the `session.revoked`
event lies within the retention and step 5 delivers it; together with the
rule for [requests racing with events](#102-event-types), nothing the request
sets up survives it.

**Deletion is final.** Once an app has deleted a user's data – after
`user.deleted` or a `410 user_deleted` – it keeps the `sub` as deleted and
never recreates data for it, whatever it receives later: an identity header
or a lookup answer for that `sub` that arrives afterwards can only be stale,
because the hub never issues a deleted `sub` again.

## 11. Access and roles

- **Hub roles:** `admin` (manages the hub, its users and apps) and `user`.
- **Per app and user:** access yes/no and, with access, one `app_role` from
  the app's manifest.
- **Default per app:** "everyone" – every new user gets access with the
  default role – or "selected only" – new users get no access until an
  administrator grants it. The invitation form shows the defaults and lets the
  administrator change them per invitation. The setting affects new users
  only: changing it leaves every existing access as it is, in both
  directions. An invitation keeps the access chosen for it; for apps it does
  not name, the default at the time it is accepted applies. A role the
  manifest no longer has when the invitation is accepted becomes the default
  role.
- Access is given in paired apps only: an app that waits for pairing has no
  roles yet.
- **After pairing,** every hub administrator gets access with the manifest's
  `admin` role (or the default role, if there is none); with "everyone", all
  other users get the default role.
- A signed-in user without access gets **403** from the hub for every path
  under `/<slug>/` ([§ 12](#12-errors)), even for paths the app treats as
  public.
- Groups are not part of version 1.

## 12. Errors

Errors the hub generates for paths under `/<slug>/` instead of the app. For
page loads (`Sec-Fetch-Mode: navigate`, or `Accept` with `text/html` and no
`Sec-Fetch-Mode`) the hub shows an error page; otherwise it answers JSON
`{"error": "<code>"}` with `Cache-Control: no-store`.

| Status | `error` | When |
|---|---|---|
| 403 | `no_app_access` | Signed in, but no access to this app |
| 403 | `origin_mismatch` | WebSocket upgrade from another origin |
| 404 | `not_found` | No app with this slug; `/<slug>/_roedy/…` from outside |
| 431 | `headers_too_large` | Request headers beyond 64 KiB ([§ 3.4](#34-limits-and-timeouts)); always JSON |
| 502 | `app_unreachable` | The app does not accept the connection |
| 503 | `app_not_ready` | The app waits for pairing, and the user is not a hub administrator |
| 504 | `app_timeout` | No response headers within 300 s after the request body ([§ 3.4](#34-limits-and-timeouts)) |

Errors of the app itself (its 401, 403, 404, 5xx) reach the browser unchanged.

## 13. Obligations of an app (checklist)

An app in hub mode MUST:

- [ ] use `ROEDY_PUBLIC_URL` = `<hub public URL>/<slug>` and serve everything below `/<slug>/`;
- [ ] be reachable only by the hub (private network);
- [ ] set `ROEDY_TRUST_PROXY` to the hub's value plus one;
- [ ] verify `X-Roedy-Identity` on every request as in [§ 4.3](#43-verification) and treat a failure as "not signed in";
- [ ] identify users by `sub` and grant permissions by `app_role` alone;
- [ ] accept no sign-in of its own and hide identity features, linking to the hub instead;
- [ ] answer 401 to unauthenticated requests for non-public resources and send the browser to `/login?next=…`;
- [ ] sign out via `POST /api/v1/auth/logout` and cope with cleared client storage;
- [ ] keep all client-side storage, service worker and manifest in its namespace ([§ 3.6](#36-namespaces-in-the-browser));
- [ ] check `Origin` on WebSocket upgrades;
- [ ] show a link to the hub's home page `/`;
- [ ] serve `GET /<slug>/healthz` and `GET /<slug>/readyz` without sign-in (`200` when healthy or ready);
- [ ] pair with a code, store `app_secret` encrypted, and send its manifest on every start;
- [ ] accept events at `POST /<slug>/_roedy/v1/events`, idempotently, answering `204`.

It SHOULD:

- [ ] use the app switcher of the hub's frame ([§ 5.5](#55-the-hubs-frame));
- [ ] catch up on events at start;
- [ ] send keep-alives at least every 30 s on long-lived streams;
- [ ] log `X-Roedy-Request-Id`;
- [ ] send a CSP with `frame-ancestors 'self'` or stricter.

## 14. Versioning and extensions

- This is **version 1**. The manifest names it in `contract`, the internal API
  in its path (`/internal/v1`), the JWTs in `typ`.
- **Compatible additions** within version 1: new claims, new event types, new
  optional manifest fields, new internal endpoints, new fields in responses.
  Apps ignore what they do not know.
- **Incompatible changes** get version 2 (`/internal/v2`, `"contract": 2`);
  the hub then supports both versions for at least one minor release.
- **Planned platform services:** the hub is becoming the shared platform for
  all Roedy apps ([`plan.md`](plan.md) § 9): what several apps need is built
  in the hub, and an app should not build its own. Planned as compatible
  additions: Stalwart servers, accounts and access tokens for the browser and
  for the app's backend (an optional manifest field, endpoints in the browser
  and internal API, events); shared settings such as time zone and formats;
  notifications with topics from the manifest; a frame of web components
  served by the hub (app switcher, notification bell, notice bar); switching
  an app off in the administration. Until each is in this document, nothing
  here depends on it.

## 15. Examples

The example app in this repository (`crates/example-app`) implements the whole
contract and is what the hub's end-to-end tests run against.

**Test vectors** are in [`app-contract/vectors.json`](app-contract/vectors.json):
the JWKS (the public key of RFC 8037, appendix A.1, as in § 4.4), the
published test keys (never use them for anything else), and cases with a
token, the `typ` to verify it as, `now` and the result – the claims of a valid
token, or for an invalid one an `error` naming the first check that fails in
the order of [§ 4.3](#43-verification): `malformed`, `alg`, `typ`,
`unknown_kid`, `signature`, `claims` (not the expected shape, e.g. `aud` as an
array), `iss`, `aud`, `expired`, `iat`, `lifetime`. An app only has to reject
an invalid token; the code helps to find out why it did not. The hub's own
implementation (`crates/contract`) makes and checks the file.

Decoded identity:

```json
{
  "header": { "alg": "EdDSA", "typ": "roedy-id+jwt", "kid": "01JDX3M8Q4V6S2T0R9P7N5K3H1" },
  "claims": {
    "iss": "https://app.example.com",
    "aud": "mail",
    "sub": "01JDX2Q7B5W3Y1Z9X7V5T3R1P0",
    "username": "alex",
    "email": "alex@example.com",
    "name": "Alex Example",
    "role": "user",
    "app_role": "user",
    "locale": "de",
    "sid": "01JDX3A1C3E5G7J9K1M3N5P7Q9",
    "auth_time": 1795600000,
    "iat": 1795600123,
    "exp": 1795600183
  }
}
```

Decoded event:

```json
{
  "header": { "alg": "EdDSA", "typ": "roedy-event+jwt", "kid": "01JDX3M8Q4V6S2T0R9P7N5K3H1" },
  "claims": {
    "iss": "https://app.example.com",
    "aud": "mail",
    "jti": "01JDX5R2T4V6X8Z0B2D4F6H8J0",
    "seq": 42,
    "type": "session.revoked",
    "data": { "sub": "01JDX2Q7B5W3Y1Z9X7V5T3R1P0", "sid": "01JDX3A1C3E5G7J9K1M3N5P7Q9", "reason": "logout" },
    "iat": 1795600200,
    "exp": 1795600500
  }
}
```
