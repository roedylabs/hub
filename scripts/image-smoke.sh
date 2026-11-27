#!/bin/sh
# Smoke test and budgets of the image (ci.yml, release.yml before and after the
# push, locally `just smoke`); taken over from Roedy Mail (../mail). Starts two
# containers with named volumes and the restrictions of deploy/compose.yml:
#
# - TLS off (ROEDY_TLS=off): waits for the HEALTHCHECK and checks user and
#   privileges of the process, the endpoints, the embedded SPA, the internal
#   listener (listening, but neither exposed nor on the public port), setup
#   token, version and the idle memory.
# - TLS from files (ROEDY_TLS=files, a self-signed certificate from
#   scripts/test-cert.sh): the HEALTHCHECK over TLS, user and privileges,
#   HTTPS on 8443 with that certificate for the public host only (SNI), HSTS,
#   the 308 from 8080 to ROEDY_PUBLIC_URL, the ACME HTTP-01 path on 8080, no
#   web app and no API on plain HTTP, and the idle memory with TLS.
#
# Needs no setup of the hub; needs curl and openssl.
#
#   scripts/image-smoke.sh <image> [<version>]
#
# Budgets (docs/plan.md): idle RSS ≤ 40 MB here, in both modes; image size
# ≤ 25 MB in the workflows. MB = 10^6 bytes.
set -eu

image=${1:?usage: scripts/image-smoke.sh <image> [<version>]}
version=${2:-}
here=$(dirname "$0")
name="roedy-hub-smoke-$$"
volume="$name-data"
tls="$name-tls"
tls_volume="$tls-data"
certs=

fail() {
  echo "Smoke test failed: $*" >&2
  for container in "$name" "$tls"; do
    if docker container inspect "$container" >/dev/null 2>&1; then
      echo "--- docker logs $container" >&2
      docker logs "$container" >&2 2>&1 || true
    fi
  done
  exit 1
}
cleanup() {
  docker rm -f "$name" "$tls" >/dev/null 2>&1 || true
  docker volume rm "$volume" "$tls_volume" >/dev/null 2>&1 || true
  [ -z "$certs" ] || rm -rf "$certs"
}
trap cleanup EXIT

check() { # <description> <command …>
  what=$1
  shift
  "$@" || fail "$what"
}

check "USER is 65532:65532" [ "$(docker image inspect -f '{{.Config.User}}' "$image")" = 65532:65532 ]
# Only the public port: `docker run -P` must never publish the internal one.
exposed=$(docker image inspect -f '{{range $port, $_ := .Config.ExposedPorts}}{{$port}} {{end}}' "$image")
check "only 8080/tcp and 8443/tcp exposed (actual: $exposed)" [ "$exposed" = "8080/tcp 8443/tcp " ]

# HEALTHCHECK (roedy-hub healthcheck) must report "healthy" within 30 s.
healthy() { # <container>
  i=0
  while :; do
    status=$(docker inspect -f '{{.State.Health.Status}}' "$1")
    [ "$status" = healthy ] && break
    [ "$(docker inspect -f '{{.State.Running}}' "$1")" = true ] || fail "$1: container exited"
    i=$((i + 1))
    [ "$i" -le 30 ] || fail "$1: not healthy after 30 s (status $status)"
    sleep 1
  done
}

# With user namespace remapping (userns-remap) the host sees the remapped IDs,
# not 65532: then only Config.User applies; not on GitHub's runners.
userns=$(docker info --format '{{json .SecurityOptions}}' 2>/dev/null | grep -c 'name=userns' || true)

# The server process as the host sees it (the image has no shell): UID and GID
# 65532 (real, effective, saved, file system), no capabilities, no_new_privs.
# Without /proc (Docker Desktop) only a note; in CI the check is mandatory.
privileges() { # <container>
  proc="/proc/$(docker inspect -f '{{.State.Pid}}' "$1")/status"
  if [ -r "$proc" ]; then
    field() { sed -n "s/^$1:[[:space:]]*//p" "$proc" | tr -s '[:space:]' ' ' | sed 's/ $//'; }
    if [ "$userns" = 0 ] || [ -n "${GITHUB_ACTIONS:-}" ]; then
      check "$1: UID 65532 (Uid: $(field Uid))" [ "$(field Uid)" = "65532 65532 65532 65532" ]
      check "$1: GID 65532 (Gid: $(field Gid))" [ "$(field Gid)" = "65532 65532 65532 65532" ]
    else
      echo "UID and GID of the process not checked: the Docker daemon remaps user namespaces"
    fi
    check "$1: no effective capabilities (CapEff: $(field CapEff))" [ "$(field CapEff)" = 0000000000000000 ]
    check "$1: empty capability bounding set (CapBnd: $(field CapBnd))" [ "$(field CapBnd)" = 0000000000000000 ]
    check "$1: no_new_privs (NoNewPrivs: $(field NoNewPrivs))" [ "$(field NoNewPrivs)" = 1 ]
  elif [ -n "${GITHUB_ACTIONS:-}" ]; then
    fail "$proc missing, user and privileges of the process not checked"
  else
    echo "User and privileges of the process not checked: $proc missing (e.g. Docker Desktop)"
  fi
}

# Idle memory after the requests made so far (budget: RSS ≤ 40 MB): VmRSS of
# the server process via the host's /proc, in MB = 10^6 bytes like the image
# size in the workflows. Without /proc (Docker Desktop) only a note.
idle_rss() { # <container> <mode>
  sleep 2
  proc="/proc/$(docker inspect -f '{{.State.Pid}}' "$1")/status"
  if [ -r "$proc" ]; then
    kib() { sed -n "s/^$1:[[:space:]]*\([0-9]*\) kB\$/\1/p" "$proc"; }
    mb() { awk -v k="$1" 'BEGIN { printf "%.1f", k * 1024 / 1000000 }'; }
    rss=$(kib VmRSS)
    peak=$(kib VmHWM)
    check "$1: VmRSS in $proc" [ -n "$rss" ]
    check "$1: VmHWM in $proc" [ -n "$peak" ]
    line="Idle RSS ($2): $(mb "$rss") MB (peak since start $(mb "$peak") MB)"
    echo "$line"
    [ -z "${GITHUB_STEP_SUMMARY:-}" ] || echo "$line" >>"$GITHUB_STEP_SUMMARY"
    check "idle RSS ($2) ≤ 40 MB (actual: $(mb "$rss") MB)" [ $((rss * 1024)) -le 40000000 ]
  else
    echo "RSS ($2) not measured: $proc missing (e.g. Docker Desktop)"
  fi
}

# The same restrictions as deploy/compose.yml: read-only file system apart from
# /data and a tmpfs on /tmp, no capabilities, no privilege escalation.
restrictions="--read-only --tmpfs /tmp --cap-drop ALL --security-opt no-new-privileges:true"

# --- TLS off. Only the public port is published; the internal listener keeps
# its default 0.0.0.0:8081 inside the container.
# shellcheck disable=SC2086 # $restrictions are separate options
docker run -d --name "$name" -p 127.0.0.1::8080 -v "$volume:/data" $restrictions \
  -e ROEDY_PUBLIC_URL=https://app.example.com \
  "$image" >/dev/null
url="http://$(docker port "$name" 8080/tcp | head -n 1)"
healthy "$name"

body() { curl -fsS "$url$1"; }
status() { curl -s -o /dev/null -w '%{http_code}' "$url$1"; }

privileges "$name"

check "/healthz" sh -c "curl -fsS '$url/healthz' | grep -q '\"ok\"'"
check "/readyz" sh -c "curl -fsS '$url/readyz' | grep -q '\"ok\"'"
check "index.html embedded" sh -c "curl -fsS '$url/' | grep -q '<html'"
check "SPA fallback" sh -c "curl -fsS '$url/login' | grep -q '<html'"
check "missing asset → 404" [ "$(status /_hub/immutable/missing.js)" = 404 ]
check "/api → 404" [ "$(status /api/v1/does-not-exist)" = 404 ]
asset=$(body / | grep -o '/_hub/immutable/[^"]*\.js' | head -n 1)
check "an asset in index.html" [ -n "$asset" ]
check "asset from index.html ($asset)" sh -c "curl -fsS -D - -o /dev/null '$url$asset' | grep -qi 'cache-control: public, max-age=31536000, immutable'"
# SvelteKit's startup script is inline; the CSP must name its hash.
check "CSP with the hash of the startup script" sh -c "curl -fsS -D - -o /dev/null '$url/' | grep -qi \"^content-security-policy: .*script-src 'self' 'sha256-\""
check "key file created in the data directory" sh -c "docker logs '$name' 2>&1 | grep -q 'secret.key (newly created)'"

# The internal listener: not published, not on the public port, but listening
# for the apps on the container's network (its address is reachable from the
# host on Linux; without it, e.g. Docker Desktop, only a note).
check "internal port not published" sh -c "! docker port '$name' 8081/tcp >/dev/null 2>&1"
no_jwks() { ! printf '%s' "$1" | grep -q '"keys"'; }
check "no internal API on the public port" no_jwks "$(curl -s "$url/internal/v1/jwks" || true)"
ip=$(docker inspect -f '{{range .NetworkSettings.Networks}}{{.IPAddress}}{{end}}' "$name")
if [ -n "$ip" ] && curl -fsS -m 5 -o /dev/null "http://$ip:8080/healthz" 2>/dev/null; then
  check "internal listener on $ip:8081" sh -c "curl -fsS -m 5 'http://$ip:8081/internal/v1/jwks' | grep -q '\"keys\"'"
  check "internal listener: other paths → 404" [ "$(curl -s -m 5 -o /dev/null -w '%{http_code}' "http://$ip:8081/healthz")" = 404 ]
elif [ -n "${GITHUB_ACTIONS:-}" ]; then
  fail "container address '$ip' not reachable, internal listener not checked"
else
  echo "Internal listener not checked: the container's address is not reachable from here (e.g. Docker Desktop)"
fi

logged=$(docker logs "$name" 2>&1 | sed -n 's/.*Setup-Token: \([A-Z2-9-]*\).*/\1/p' | head -n 1)
shown=$(docker exec "$name" roedy-hub setup-token | sed -n 's/^Setup-Token: //p')
check "setup token in the log" [ -n "$logged" ]
check "setup token via 'docker exec … roedy-hub setup-token'" [ "$logged" = "$shown" ]

if [ -n "$version" ]; then
  actual=$(docker exec "$name" roedy-hub --version)
  check "version $version (actual: $actual)" [ "$actual" = "roedy-hub $version" ]
fi

idle_rss "$name" "TLS off"

# --- TLS from files: a self-signed certificate for the public host
# hub.example.test, which exists nowhere – curl connects to the published
# ports with that name (SNI) and trusts only this certificate.
certs=$(mktemp -d)
"$here/test-cert.sh" "$certs" hub.example.test || fail "no test certificate (openssl missing?)"
host=hub.example.test
public="https://$host"
# shellcheck disable=SC2086 # $restrictions are separate options
docker run -d --name "$tls" -p 127.0.0.1::8443 -p 127.0.0.1::8080 -v "$tls_volume:/data" \
  -v "$certs:/run/roedy-tls:ro" $restrictions \
  -e ROEDY_PUBLIC_URL="$public" -e ROEDY_TLS=files \
  -e ROEDY_TLS_CERT_FILE=/run/roedy-tls/cert.pem -e ROEDY_TLS_KEY_FILE=/run/roedy-tls/key.pem \
  "$image" >/dev/null
https_port=$(docker port "$tls" 8443/tcp | head -n 1 | sed 's/.*://')
http_port=$(docker port "$tls" 8080/tcp | head -n 1 | sed 's/.*://')
check "HTTPS port published" [ -n "$https_port" ]
check "HTTP port published" [ -n "$http_port" ]
# The healthcheck speaks TLS with the public host as server name.
healthy "$tls"
privileges "$tls"

https() { curl --cacert "$certs/cert.pem" --connect-to "$host:443:127.0.0.1:$https_port" "$@"; }
has() { printf '%s' "$1" | grep -qi -- "$2"; }
check "HTTPS /readyz with the test certificate" has "$(https -fsS "$public/readyz" || true)" '"ok"'
check "the web app over HTTPS" has "$(https -fsS "$public/login" || true)" '<html'
served=$(openssl s_client -connect "127.0.0.1:$https_port" -servername "$host" </dev/null 2>/dev/null |
  openssl x509 -noout -fingerprint -sha256 2>/dev/null || true)
expected=$(openssl x509 -in "$certs/cert.pem" -noout -fingerprint -sha256)
check "the certificate from ROEDY_TLS_CERT_FILE is served (actual: ${served:-none})" [ "$served" = "$expected" ]
check "HSTS over HTTPS" has "$(https -fsS -D - -o /dev/null "$public/" || true)" '^strict-transport-security: max-age=31536000'
check "no certificate for another server name" sh -c "! curl -sk -o /dev/null --connect-to 'other.example.test:443:127.0.0.1:$https_port' https://other.example.test/healthz"
check "no certificate without a server name" sh -c "! curl -sk -o /dev/null 'https://127.0.0.1:$https_port/healthz'"
check "no plain HTTP on the HTTPS port" [ "$(curl -s -o /dev/null -w '%{http_code}' "http://127.0.0.1:$https_port/healthz" || true)" != 200 ]
check "no internal API over HTTPS" no_jwks "$(https -s "$public/internal/v1/jwks" || true)"
check "internal port not published (TLS)" sh -c "! docker port '$tls' 8081/tcp >/dev/null 2>&1"

# Plain HTTP: a 308 to the same path and query under ROEDY_PUBLIC_URL for
# everything – no web app, no API, no health – except ACME HTTP-01.
for target in "/" "/login?next=%2Fadmin" "/api/v1/session" "/healthz" "/readyz" "/example/" "/internal/v1/jwks"; do
  actual=$(curl -s -o /dev/null -w '%{http_code} %{redirect_url}' "http://127.0.0.1:$http_port$target" || true)
  check "HTTP $target → 308 $public$target (actual: $actual)" [ "$actual" = "308 $public$target" ]
done
check "HTTP: the ACME challenge path is answered, not redirected" \
  [ "$(curl -s -o /dev/null -w '%{http_code}' "http://127.0.0.1:$http_port/.well-known/acme-challenge/smoke-test-token")" = 404 ]

idle_rss "$tls" "TLS files"

for container in "$name" "$tls"; do
  docker stop -t 10 "$container" >/dev/null
  check "$container: clean shutdown (exit code 0)" [ "$(docker inspect -f '{{.State.ExitCode}}' "$container")" = 0 ]
done
echo "Smoke test passed: $image"
