# Operating Roedy Hub

> Installation with Docker Compose and Caddy, configuration, data directory, backup and restore, updates, pairing an
> app, the gateway, events, health and the resource budgets. Installation, backup and update steps are taken over from
> Roedy Mail's `docs/ops.md` and adapted.

Roedy Hub is a single process with an embedded web app and an SQLite database. It listens on two ports: the
**public** one, which the reverse proxy forwards to, and the **internal** one, which only the apps reach over the
private network ([app contract › Internal API](app-contract.md#7-internal-api)). It is shipped as the Docker image
`ghcr.io/roedylabs/hub` for `linux/amd64` and `linux/arm64`.

```
Browser ──HTTPS──► Caddy :443 ──HTTP──► roedy-hub :8080 (public)   ──► /data (SQLite, key)
                   (network edge)          │ /mail/… ──► roedy-mail :8080
                     apps (network apps) ──HTTP──► roedy-hub :8081 (internal, never published)
```

## Requirements

- **Host:** Linux with Docker and Compose v2 (2.24 or later for the test override), `linux/amd64` or `linux/arm64`.
  The hub needs about 15 MB of memory when idle (budget: 40 MB); the image is about 12 MB (budget: 25 MB), see
  [Budgets](#budgets).
- **Own host name with HTTPS**, such as `app.example.com`, whose DNS points at the host, and ports 80 and 443 open
  for Caddy (certificates from Let's Encrypt). The hub needs the root `/` of its origin; the apps live under
  `/<slug>/` of the same origin, a sub-path for the hub does not work.
- **Outgoing connections:** HTTPS to the JMAP server for the hub's system mails (Administration › Email) and whatever
  the apps need (Roedy Mail: its Stalwart servers).
- **Time:** an accurate clock (NTP); authenticator app codes, sessions and the identities for the apps depend on it.

## Installation

### With Docker Compose

The basis is [`deploy/compose.yml`](../deploy/compose.yml): Caddy, the hub and, with the profile `mail`, Roedy Mail.
Only Caddy publishes ports (80, 443 and 443/udp). The containers talk over two private networks:

| Network | Members | What for |
|---|---|---|
| `edge` | Caddy, hub | Caddy forwards every request to the hub's public port `8080`. |
| `apps` | hub, apps | The hub reaches the apps at `http://<container>:8080`; the apps reach the hub's internal listener at `http://roedy-hub:8081`. |

The hub's internal listener binds to the hub's fixed address in `apps` (`ROEDY_INTERNAL_LISTEN`, default
`172.30.240.2:8081`), so Caddy cannot reach it, and the apps are not in `edge`, so only the hub reaches them. Every
container runs with a read-only file system (except Caddy), no capabilities (Caddy only `NET_BIND_SERVICE`) and
`no-new-privileges`.

1. Put `compose.yml` and [`Caddyfile`](../deploy/Caddyfile) into one directory, such as `/opt/roedy`, and create a
   `.env` file next to them from the template [`deploy/.env.example`](../deploy/.env.example):

   ```sh
   for f in compose.yml Caddyfile .env.example; do
     curl -fsSLO "https://raw.githubusercontent.com/roedylabs/hub/main/deploy/$f"
   done
   cp .env.example .env && chmod 600 .env
   ```

   It lists every variable of `compose.yml`. Replace `app.example.com` with your own host name; the minimum is:

   ```sh
   ROEDY_PUBLIC_URL=https://app.example.com
   ROEDY_IMAGE_TAG=0.1
   ```

   `ROEDY_PUBLIC_URL` is required: Caddy takes it as its site address and obtains the certificate for it, the hub
   uses it for links in mails and as the issuer of the identities. A fixed version in the tag (`0.1` follows the
   patches of 0.1.x, `0.1.0` is exactly that version) makes updates predictable; `latest` is the default. Pre-releases
   such as `0.2.0-rc.1` are only available under their own name.

2. Start and check the health (the image is public, no `docker login` needed):

   ```sh
   docker compose up -d
   docker compose ps                                # roedy-hub: healthy
   curl -fsS https://app.example.com/readyz         # {"status":"ok"}
   docker compose logs roedy-hub | grep Setup-Token
   ```

3. Open `https://app.example.com/setup` and enter the setup token to create the first administrator. Then pair the
   apps ([Pairing an app](#pairing-an-app)).

The commands here run in the directory of `compose.yml`; from another directory use `-f <path>/compose.yml`. The
project is called `roedy`, so the hub's volume is `roedy_roedy-hub-data` and the apps' network `roedy_apps`.

**Subnet:** the network `apps` has the fixed subnet `172.30.240.0/24` for the hub's fixed address. If it collides with
a network of the host or another Docker network (`Pool overlaps with other one on this address space`), set
`ROEDY_APPS_SUBNET` and `ROEDY_HUB_APPS_IP` in `.env` together, e.g. `10.250.40.0/24` and `10.250.40.2`.

**Bind mount instead of a volume:** a host directory must belong to the container's user (default 65532:65532),
otherwise the hub does not start (`Permission denied`):

```sh
sudo install -d -o 65532 -g 65532 -m 700 /srv/roedy-hub
```

and in `compose.yml` `- /srv/roedy-hub:/data` instead of `- roedy-hub-data:/data`. The data directory must be on a
local file system, not on NFS or SMB: SQLite in WAL mode needs shared memory and reliable locks.

**Container user:** the hub runs as user and group 65532 (the image's `USER`). For other IDs set `ROEDY_UID` and
`ROEDY_GID` in `.env`; never 0 (root). A new instance then needs a bind mount created with these IDs (a new named
volume takes owner 65532 from the image); for an existing volume change the owner once while the hub is stopped:

```sh
docker compose stop roedy-hub
docker run --rm -v roedy_roedy-hub-data:/data alpine:3.24 chown -R 1500:1500 /data
docker compose up -d
```

**Verifying the image:** every release carries an SBOM and provenance as attestations and is signed keylessly with
[cosign](https://github.com/sigstore/cosign) by the release workflow:

```sh
cosign verify ghcr.io/roedylabs/hub:0.1.0 \
  --certificate-identity https://github.com/roedylabs/hub/.github/workflows/release.yml@refs/tags/v0.1.0 \
  --certificate-oidc-issuer https://token.actions.githubusercontent.com
```

**Building it yourself:** `just dist` (needs [Zig](https://ziglang.org), `cargo-zigbuild`, `cargo-auditable` and the
Rust targets `x86_64-unknown-linux-musl` and `aarch64-unknown-linux-musl`; `just dist x86_64-unknown-linux-musl` for
one), then `just image` or `docker buildx build --platform linux/amd64,linux/arm64 .`. The Dockerfile only packages
the binaries from `dist/`. `just smoke` runs the image's smoke test, `just smoke-compose` the one of the Compose setup
(with `just dist-example` and `just image-example` for the example app).

### Reverse proxy

[`deploy/Caddyfile`](../deploy/Caddyfile) is all Caddy needs:

```
{$CADDY_SITE} {
	encode {
		zstd
		gzip
		match {
			header Content-Type text/html*
			# … CSS, JavaScript, JSON, SVG; never text/event-stream
		}
	}
	reverse_proxy roedy-hub:8080
}
```

`CADDY_SITE` is `ROEDY_PUBLIC_URL`. Caddy obtains and renews the certificate, redirects HTTP to HTTPS and speaks
HTTP/2 and HTTP/3 to the browser. Its certificates live in the volume `caddy-data`; keep it, otherwise every new
container requests new certificates and may run into Let's Encrypt's limits.

Another reverse proxy works as well, as long as it:

- **appends `X-Forwarded-For`** and the hub's port is reachable only through it. `compose.yml` sets
  `ROEDY_TRUST_PROXY=true` (one proxy); the hub takes the client IP for rate limits, sessions and the audit log from
  the entry the proxy appended. With another proxy in front (such as a CDN), use `2` and let the rear proxy trust the
  front one; the apps then use `3` (`ROEDY_MAIL_TRUST_PROXY`).
- **passes request and response headers through unchanged**, in particular `Cookie`, `Origin`, `Sec-Fetch-*`, the
  hub's Content Security Policy and the apps' headers.
- **streams** request and response bodies, Server-Sent Events and WebSockets (`Upgrade`) without buffering, without
  a body size limit (uploads to the apps) and with a read timeout of at least 300 seconds (the hub's own limit for an
  app's response, [Gateway](#gateway)); it does not compress `text/event-stream`.
- forwards **every path** of the origin to the hub, never the internal port `8081`.

## Pairing an app

An app connects to the hub once with a pairing code ([app contract › Pairing](app-contract.md#9-pairing)). For Roedy
Mail with `deploy/compose.yml` (needs a Roedy Mail version with hub mode):

1. In the hub: **Administration › Apps › Add app**, slug `mail`, internal address `http://roedy-mail:8080`. The hub shows
   a pairing code (`rhp_…`), valid for 15 minutes.
2. In `.env`: `COMPOSE_PROFILES=mail` and `ROEDY_MAIL_PAIRING_CODE=<code>`, then `docker compose up -d`. Mail runs at
   `<ROEDY_PUBLIC_URL>/mail` (its `ROEDY_PUBLIC_URL`, set by `compose.yml`), pairs with the hub's internal listener at
   `http://roedy-hub:8081` and sends its manifest.
3. Administration › Apps shows the app as connected once its health check answers. Remove the code from `.env`
   afterwards; it is used up.

Another app: add a service like `roedy-mail` in the network `apps` only (never in `edge`, never with `ports`), with
`ROEDY_PUBLIC_URL=<hub public URL>/<slug>`, `ROEDY_TRUST_PROXY` the hub's value plus one, the hub's internal address
`http://roedy-hub:8081` and the code, and choose the same slug in the hub. "New pairing code" pairs an app again, for
example after its data was lost.

## Configuration

All settings come from environment variables. Values are read as text without surrounding whitespace, so a key or
path made only of digits works as well. Empty values count as unset (except for `ROEDY_INTERNAL_LISTEN`, see below),
unknown `ROEDY_*` variables are ignored, invalid values prevent the start with a line `error: …` that names the
variable (such as `error: Invalid configuration – ROEDY_LISTEN: …`). The names and meanings are the same as in
Roedy Mail.

| Variable | Default | Meaning |
|---|---|---|
| `ROEDY_PUBLIC_URL` | – | **Required.** Public address of the hub, e.g. `https://app.example.com`: scheme, host and optionally a port, without path, credentials, query or fragment. The hub owns the root of its origin; the apps live under `/<slug>/`. Without it, `serve` exits with `error: ROEDY_PUBLIC_URL is not set – …` before it creates anything. |
| `ROEDY_LISTEN` | `0.0.0.0:8080` | Address and port of the public HTTP server: web app, `/api/v1`, `/healthz`, `/readyz` and the apps under `/<slug>/` ([Gateway](#gateway)). The reverse proxy handles TLS. |
| `ROEDY_INTERNAL_LISTEN` | `0.0.0.0:8081` | Address and port of the internal HTTP server for the apps (`/internal/v1/…`). **Never publish this port** and never forward it through the reverse proxy. Set to empty to turn it off; then no app can pair. It must not overlap with `ROEDY_LISTEN`. Apps pair, send their manifest and read users and keys here (docs/app-contract.md › Internal API); every other path answers `404`. |
| `ROEDY_DATA_DIR` | `data` relative to the working directory | Directory with the database `roedy.sqlite3` and the key `secret.key`; created with `0700`, files `0600`. Must be on a local file system (SQLite in WAL mode). |
| `ROEDY_LOG` | `info` | Log filter in the format of [`EnvFilter`](https://docs.rs/tracing-subscriber/latest/tracing_subscriber/filter/struct.EnvFilter.html), e.g. `warn` or `info,roedy_hub_server=debug`. `info,tower_http=debug` logs every request (method and path, tokens in paths replaced by `***`, never the query). The setup token always appears. |
| `ROEDY_LOG_FORMAT` | `json` | `json` (one line per event) or `text`. Logs go to stderr. |
| `ROEDY_WEB_DIR` | – | Directory with the web app build (e.g. `apps/web/build`). Takes precedence over the embedded web app; for development. |
| `ROEDY_TRUST_PROXY` | `false` | Reverse proxies in front of the server: `true` for one, otherwise their number (at most 10). The client IP is then the entry in `X-Forwarded-For` appended by the front-most proxy, and the server sends HSTS. Only set it when the public port is reachable exclusively via the proxy and every proxy appends to `X-Forwarded-For` instead of passing it through. The apps receive the trusted entries plus the hub's own peer; set their `ROEDY_TRUST_PROXY` to this value plus one. |
| `ROEDY_SECRET_KEY` | – | Instance key for secrets in the database: 32 bytes as 64 hex characters (`openssl rand -hex 32`) or Base64 (`openssl rand -base64 32`). If it is missing as well as `ROEDY_SECRET_KEY_FILE`, the first start creates `<ROEDY_DATA_DIR>/secret.key` (`0600`) and uses it afterwards. Without the key the encrypted values can no longer be read; keep a copy outside the data directory. The keys that sign the identities for the apps are encrypted with it as well: with another instance key the hub replaces them at the next start, and paired apps fetch the new one on their own. |
| `ROEDY_SECRET_KEY_FILE` | – | File with this key, e.g. a Docker secret (`/run/secrets/roedy_secret_key`). Not together with `ROEDY_SECRET_KEY`. |

**Initial setup:** as long as no administrator exists, every start writes a setup token to the log
(`Setup-Token: XXXX-XXXX-XXXX`). `roedy-hub setup-token` shows it again.

**Commands:** `roedy-hub serve` starts the server, `roedy-hub healthcheck` checks it ([Health](#health)),
`roedy-hub migrate` applies pending database migrations (`serve` does that as well) and `roedy-hub setup-token` shows
the setup token. All of them read the same variables. On `SIGTERM` or `SIGINT` the server stops accepting connections
on both ports, closes the open streams and WebSockets to the apps and gives open requests 8 seconds to finish.

**Lost administrator access:** `roedy-hub reset-admin` restores an administrator's access – for example when the only
administrator lost both password and authenticator app. It sets a new random password and prints it, removes the
authenticator apps and recovery codes, signs out every device, discards open links from mails and unlocks the
account; the audit log records it. With several administrators, choose one with `--user <email address or username>`.
`--link` prints a link to set a new password (valid for an hour, needs `ROEDY_PUBLIC_URL`) instead of a password, so the
operator never learns it; the old password stops working right away. Run it next to the server with the same
`ROEDY_DATA_DIR`, for example in its container.

**Cleanup:** the server deletes expired sessions and links from mails every hour, audit log entries older than the
retention chosen under Administration › Audit log (30, 90 or 365 days, default 90) and events for the apps older
than 30 days ([Events](#events)) once a day.

## Data directory

Everything the hub stores permanently lives in `/data` (`ROEDY_DATA_DIR`), owner 65532 (or `ROEDY_UID`/`ROEDY_GID`),
directory `0700`, files `0600`:

| Path | Content |
|---|---|
| `roedy.sqlite3` | Database: users, sessions, settings, branding, paired apps (secrets as SHA-256), access per app, events for the apps, audit log, the signing keys of the identities (encrypted) |
| `roedy.sqlite3-wal`, `roedy.sqlite3-shm` | Part of the database while it is open (WAL mode). Never delete or copy them individually while the hub is running. |
| `secret.key` | [Instance key](#instance-key), only without `ROEDY_SECRET_KEY` and `ROEDY_SECRET_KEY_FILE` |

SQLite's temporary files go to `/tmp` (a `tmpfs` in `compose.yml`). A copy of the files while the hub is running is
not a usable backup ([Backup and restore](#backup-and-restore)).

### Instance key

The key encrypts the secrets in the database (authenticator apps, open invitations, the sender account for system
mails, the signing keys of the identities). It does **not** belong next to the backups: whoever has a backup should
not be able to read the secrets in it. Store it separately right after the setup, for example in a password manager:

```sh
docker cp roedy-hub:/data/secret.key ./secret.key   # 64 hex characters
```

Delete the file afterwards once the key is in its place. The content also works as `ROEDY_SECRET_KEY`. With a
different key the hub starts, but authenticator apps, open invitations and the sender account no longer work, and it
replaces the signing keys (the apps fetch the new ones on their own). There is no command to change the key.

## Backup and restore

The hub has no backups of its own yet ([`plan.md`](plan.md) › Open points). Back up the data volume with the hub
stopped for a few seconds; Caddy answers `502` meanwhile. Without `secret.key`, which you keep separately:

```sh
docker compose stop roedy-hub
docker run --rm -v roedy_roedy-hub-data:/data:ro -v "$PWD":/backup alpine:3.24 \
  tar -czf "/backup/roedy-hub-$(date -u +%Y%m%dT%H%M%SZ).tar.gz" -C /data --exclude=./secret.key .
docker compose start roedy-hub
```

Copy the archive off the host (it holds password hashes and personal data, so keep it as protected as the server).
The apps back up their own data; for Roedy Mail see its `docs/ops.md`.

**Restore** (same key; the volume's content is replaced by the archive's) with
[`deploy/restore.sh`](../deploy/restore.sh), in the directory of `compose.yml`:

```sh
curl -fsSLO https://raw.githubusercontent.com/roedylabs/hub/main/deploy/restore.sh && chmod +x restore.sh
ROEDY_UID=65532 ROEDY_GID=65532 ./restore.sh roedy-hub-20261127T030000Z.tar.gz   # IDs as in .env
```

With the default named volume nothing else is needed. With a [bind mount](#with-docker-compose) such as
`/srv/roedy-hub:/data`, name its host directory, for the restore as well as for `--recover`:

```sh
ROEDY_HUB_VOLUME=/srv/roedy-hub ./restore.sh roedy-hub-20261127T030000Z.tar.gz
ROEDY_HUB_VOLUME=/srv/roedy-hub ./restore.sh --recover
```

Only one restore or recovery runs at a time: the script holds the lock directory `.roedy-restore.lock` next to
`compose.yml` until the hub has started again.

It stops the hub and works in a helper container (`alpine`, which installs `sqlite`, so it needs internet access): it
unpacks the archive there and checks the database with SQLite's `integrity_check`, copies it into the volume next to
the current data and then swaps them by renaming; the current data is moved to `.before-restore` and only deleted once
the new data is in place. Any failure – a missing or damaged archive, a full disk, a failed rename – puts the previous
data back and leaves the hub stopped; it only starts again after a successful restore. `secret.key` is never touched.
The volume needs free space for a second copy of the database.

**Interrupted restore** (the helper was killed, the host went down): the previous data is in `.before-restore` in the
volume, and a new restore refuses to start. `./restore.sh --recover` puts it back (and removes a half-copied
archive), then starts the hub; run the restore again afterwards.

On a new host start with the same key (`ROEDY_SECRET_KEY` in `.env`, or `secret.key` restored into the volume). If
the archive is older than the program version, the start applies the missing migrations. Everything since the backup
is lost: sessions and links from mails of that time are valid again, and apps paired since then must be paired
again. The apps have also processed events the restored hub will number anew; an app acknowledges a number it has
already processed without acting on it ([app contract › Delivery](app-contract.md#101-delivery)), so it misses the
changes behind them until the hub's numbering has passed its position. Back up often and restore the latest backup;
resetting the apps' position after a restore is an open point ([`plan.md`](plan.md) › Open points).

## Updates

The hub does not install updates by itself; what changes is described in the release notes and `CHANGELOG.md`.
Versions follow 0.x: a new minor version (0.1 → 0.2) may contain migrations and operational changes, a patch version
(0.1.0 → 0.1.1) fixes bugs. An update:

1. [Back up](#backup-and-restore) the data volume.
2. Read the release notes, raise `ROEDY_IMAGE_TAG` in `.env` if needed.
3. Pull the new images and recreate the containers:

   ```sh
   docker compose pull
   docker compose up -d
   docker compose logs roedy-hub | grep -E 'migration|Roedy Hub started'
   ```

The hub applies pending migrations by itself on start (`roedy-hub migrate` does the same without starting) and gives
open requests 8 seconds when stopping; the apps keep running and catch up on missed events. Contract version 1 stays
backwards compatible, so apps keep working across hub updates
([app contract › Versioning](app-contract.md#14-versioning-and-extensions)).

**Back to an older version:** an older version does not start once the database schema is newer than itself. Then set
the older tag and restore the backup from before the update; whatever happened since the update is lost.

## Gateway

Every path whose first segment is an app's slug goes to that app's internal address, path and query unchanged
([app contract › Routing](app-contract.md#3-routing)); the hub's own paths and every reserved name stay with the hub.
The hub speaks HTTP/1.1 to the apps (TLS for an `https` internal address, verified against the system's roots), keeps
idle connections for a minute and passes bodies through as they arrive, in both directions, without a size or total
time limit. It answers itself with JSON `{"error": "<code>"}` – for page loads with the web app showing the error –
when:

| Status | `error` | When |
|---|---|---|
| `404` | `not_found` | No app with this slug, or `/<slug>/_roedy/…` from outside |
| `403` | `no_app_access` | Signed in, but without access to the app |
| `403` | `origin_mismatch` | A WebSocket upgrade from another origin |
| `431` | `headers_too_large` | More than 64 KiB of request headers; always JSON, also for page loads |
| `502` | `app_unreachable` | No connection to the app within 5 seconds (name lookup, TCP and TLS), or the app breaks it off before its response |
| `503` | `app_not_ready` | The app waits for pairing and the user is no hub administrator |
| `504` | `app_timeout` | No response headers from the app within 300 seconds after the hub took the request body's last part to send it (an upload has no time limit; an app that stops reading its last bytes counts as not answering) |

Every `502` and `504` is logged as a warning with the slug and the request's `X-Roedy-Request-Id`, which the app
receives as well. A session is checked at most every 10 seconds per cookie; signing out, revoking a device, locking or
removing a user and changing the password take effect at once and close the session's open streams and WebSockets.
Granting, changing or removing a user's access to an app (Administration › Users or › Apps) applies to their next
request; removing it also closes their open streams and WebSockets to that app.
Sessions that end otherwise (expired, signed out by `roedy-hub reset-admin`) lose them within 5 seconds, as long as
the database answers within a second; a busier database delays it until a check gets through.

## Events

Changes to users, sessions and access reach the apps as signed events
([app contract › Events](app-contract.md#10-events)), written in the same database transaction as the change and sent
right after it to `POST <internal address>/<slug>/_roedy/v1/events`, one at a time in order per app. An app that does
not answer `204` within 10 seconds gets the same event again after 10 s, 30 s, 1 min, 5 min, 15 min and then every
hour; the first failure of a series is logged as a warning (`event delivery failed, retrying`, with the slug, `seq`
and the reason: `status <code>`, `timeout` or `not reachable`), the repetitions only at `debug`. After 7 days the hub
gives up (`event delivery given up until the app is back`) and tries again as soon as the app sends its manifest, reads
missed events, pairs again or answers its health check. Events written by `roedy-hub reset-admin` go out within a
minute. Events are kept for 30 days for apps that catch up; removing an app deletes its events.

## Health

| Endpoint | Response |
|---|---|
| `GET /healthz` | `200 {"status":"ok"}` as long as the process responds |
| `GET /readyz` | `200 {"status":"ok"}` if the database is readable within 2 seconds, otherwise `503 {"status":"unavailable"}` and a warning in the log |

Both are on the public port only, need no sign-in and reveal nothing but the health (`Cache-Control: no-store`). For
external monitoring, `/readyz` via the public address is the right choice:

```sh
curl -fsS https://app.example.com/readyz
```

`roedy-hub healthcheck` queries `/healthz` without curl (the image has none) and exits with `0` if it answers `200`
within 3 seconds. It reads `ROEDY_LISTEN` and connects to its port over loopback (`127.0.0.1`, for `[::]` first
`::1`), or to the address itself if `ROEDY_LISTEN` names one. Use it as the container's health check:

```yaml
healthcheck:
  test: ["CMD", "roedy-hub", "healthcheck"]
  interval: 30s
  timeout: 5s
  start_period: 10s
  retries: 3
```

## Budgets

The hub stays lightweight; CI checks both budgets on every pull request (`ci.yml`, job "Image") and the release
before it publishes (`release.yml`):

| Budget | Limit | Measured by |
|---|---|---|
| Image size (`linux/amd64`, uncompressed) | ≤ 25 MB | `docker image inspect`; CI fails above it |
| Idle memory of the process | ≤ 40 MB | `VmRSS` after start and a few requests ([`scripts/image-smoke.sh`](../scripts/image-smoke.sh)); CI fails above it |

MB means 10⁶ bytes. The step summary of each run shows the measured values. The image is a static musl binary with
the embedded web app on `scratch`, plus the CA certificates and the licence; mimalloc gives freed memory back at once
(`MIMALLOC_PURGE_DELAY=0` in the image).
