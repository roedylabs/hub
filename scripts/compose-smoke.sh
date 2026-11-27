#!/bin/sh
# Smoke test of deploy/compose.yml (ci.yml, locally `just smoke-compose`):
# validates the compose files and the production settings (only the hub
# publishes ports, 443 → 8443 and 80 → 8080, ROEDY_TLS=acme, no trusted
# proxy), starts the hub and the example app with deploy/compose.test.yml –
# the hub with the built-in TLS from a self-signed certificate
# (scripts/test-cert.sh) – and checks HTTPS with that certificate for the
# public host only, HSTS, the redirect and the ACME path on plain HTTP, the
# healthcheck over TLS and the network layout: the hub's internal listener
# reachable from the apps' network only, never published, the apps reachable
# from the apps' network only. Then a backup and restore round trip with
# deploy/restore.sh. Needs no setup of the hub; needs curl, jq and openssl.
#
#   scripts/compose-smoke.sh
#
# Images: ROEDY_HUB_IMAGE (default roedy-hub:dev) and ROEDY_EXAMPLE_IMAGE
# (default roedy-hub-example:dev), built with `just image` and
# `just image-example`. Ports 127.0.0.1:8443 and 127.0.0.1:8080 must be free.
set -eu

cd "$(dirname "$0")/.."
# alpine as in the Dockerfile: wget from the apps' network.
# shellcheck disable=SC2016 # $BUILDPLATFORM is the Dockerfile's text
alpine=$(sed -n 's/^FROM --platform=$BUILDPLATFORM \(alpine:[^ ]*\) AS rootfs$/\1/p' Dockerfile)
[ -n "$alpine" ] || {
  echo "Smoke test failed: no alpine image in the Dockerfile" >&2
  exit 1
}

export ROEDY_PUBLIC_URL=https://localhost:8443
# The test certificate for compose.test.yml, made below.
certs=$(mktemp -d)
export ROEDY_TEST_TLS_DIR="$certs"
remove_certs() { rm -rf "$certs"; }
trap remove_certs EXIT
# A project of its own: its volumes and networks never touch a deployment's
# (project roedy). The fixed container names would collide, so the test
# refuses to run next to one (see below) instead of replacing it.
project=roedy-smoke
compose() { docker compose -p "$project" -f deploy/compose.yml -f deploy/compose.test.yml "$@"; }

fail() {
  echo "Smoke test failed: $*" >&2
  compose logs >&2 2>&1 || true
  exit 1
}
backup=
# The restore lock this run created itself, if any (never someone else's).
test_lock=
cleanup() {
  [ -z "$test_lock" ] || rmdir "$test_lock" 2>/dev/null || true
  compose down -v --remove-orphans >/dev/null 2>&1 || true
  [ -z "$backup" ] || docker run --rm -v "$backup:/backup" "$alpine" sh -c 'rm -rf /backup/*' >/dev/null 2>&1 || true
}
check() { # <description> <command …>
  what=$1
  shift
  "$@" || fail "$what"
}

# The production file alone, also with Roedy Mail, and with the override.
production() { env ROEDY_PUBLIC_URL=https://app.example.com docker compose -f deploy/compose.yml --profile mail "$@"; }
check "compose.yml is valid" env ROEDY_PUBLIC_URL=https://app.example.com \
  docker compose -f deploy/compose.yml config -q
check "compose.yml with the profile mail is valid" production config -q
check "compose.test.yml is valid" compose config -q
config=$(production config --format json)
# Only the hub publishes, TCP 443 → 8443 and 80 → 8080; never 8081.
# "<service> [<host_ip>:]<published>:<target>/<protocol>" per published port.
ports_of='.services | to_entries[] | {name: .key, port: (.value.ports // [])[]} |
  "\(.name) \(if .port.host_ip then .port.host_ip + ":" else "" end)\(.port.published):\(.port.target)/\(.port.protocol // "tcp")"'
published=$(printf '%s' "$config" | jq -r "$ports_of" | sort | tr '\n' ' ')
check "only the hub publishes 443:8443 and 80:8080 (actual: $published)" \
  [ "$published" = "roedy-hub 443:8443/tcp roedy-hub 80:8080/tcp " ]
setting() { printf '%s' "$config" | jq -r ".services[\"$1\"].environment.$2 // \"\""; }
check "the hub gets its certificate by ACME (actual: $(setting roedy-hub ROEDY_TLS))" [ "$(setting roedy-hub ROEDY_TLS)" = acme ]
check "the hub trusts no proxy (actual: $(setting roedy-hub ROEDY_TRUST_PROXY))" [ -z "$(setting roedy-hub ROEDY_TRUST_PROXY)" ]
check "Mail trusts the hub only (actual: $(setting roedy-mail ROEDY_TRUST_PROXY))" [ "$(setting roedy-mail ROEDY_TRUST_PROXY)" = 1 ]
check "the hub needs no capability" [ "$(printf '%s' "$config" | jq -c '.services["roedy-hub"].cap_add // []')" = "[]" ]
test_published=$(compose config --format json | jq -r "$ports_of" | sort | tr '\n' ' ')
check "the test setup publishes the hub on loopback only (actual: $test_published)" \
  [ "$test_published" = "roedy-hub 127.0.0.1:8080:8080/tcp roedy-hub 127.0.0.1:8443:8443/tcp " ]

for container in roedy-hub roedy-hub-example roedy-mail; do
  if docker container inspect "$container" >/dev/null 2>&1 &&
    [ "$(docker inspect -f '{{index .Config.Labels "com.docker.compose.project"}}' "$container")" != "$project" ]; then
    echo "Smoke test not run: container $container exists outside this test; remove it or run the test elsewhere" >&2
    exit 1
  fi
done
trap 'cleanup; remove_certs' EXIT
cleanup
scripts/test-cert.sh "$certs" localhost || fail "no test certificate (openssl missing?)"
compose up -d --wait --wait-timeout 60 || fail "the services did not start"

# The healthcheck speaks TLS with the public host as server name.
check "roedy-hub healthy with TLS" [ "$(docker inspect -f '{{.State.Health.Status}}' roedy-hub)" = healthy ]

# HTTPS at 127.0.0.1:8443 as https://localhost:8443, trusting only the test
# certificate.
url=$ROEDY_PUBLIC_URL
https() { curl --cacert "$certs/cert.pem" "$@"; }
has() { printf '%s' "$1" | grep -qi -- "$2"; }
i=0
until https -fsS -o /dev/null "$url/readyz" 2>/dev/null; do
  i=$((i + 1))
  [ "$i" -le 30 ] || fail "no /readyz over HTTPS after 30 s"
  sleep 1
done
check "/readyz over HTTPS" has "$(https -fsS "$url/readyz" || true)" '"ok"'
check "the web app over HTTPS" has "$(https -fsS "$url/login" || true)" '<html'
check "the hub's CSP over HTTPS" has "$(https -fsS -D - -o /dev/null "$url/" || true)" '^content-security-policy: '
check "HSTS over HTTPS" has "$(https -fsS -D - -o /dev/null "$url/" || true)" '^strict-transport-security: max-age=31536000'
check "no certificate for another server name" sh -c "! curl -sk -o /dev/null --connect-to other.example.test:443:127.0.0.1:8443 https://other.example.test/healthz"
check "no internal API over HTTPS" sh -c "! curl -sk '$url/internal/v1/jwks' | grep -q '\"keys\"'"

# Plain HTTP at 127.0.0.1:8080: only the redirect and ACME HTTP-01.
for target in "/" "/login?next=%2Fadmin" "/api/v1/session" "/readyz" "/example/healthz"; do
  actual=$(curl -s -o /dev/null -w '%{http_code} %{redirect_url}' "http://127.0.0.1:8080$target" || true)
  check "HTTP $target → 308 $url$target (actual: $actual)" [ "$actual" = "308 $url$target" ]
done
check "HTTP: the ACME challenge path is answered, not redirected" \
  [ "$(curl -s -o /dev/null -w '%{http_code}' http://127.0.0.1:8080/.well-known/acme-challenge/smoke-test-token)" = 404 ]

# Published: the hub's 8443 and 8080, nothing else – never its internal 8081.
ports=$(docker port roedy-hub | sort | tr '\n' ' ')
check "the hub publishes 8080 and 8443 only (actual: $ports)" \
  [ "$ports" = "8080/tcp -> 127.0.0.1:8080 8443/tcp -> 127.0.0.1:8443 " ]
check "roedy-hub-example publishes no port" [ -z "$(docker port roedy-hub-example)" ]

# The apps' network reaches the example app and the hub's internal listener; a
# container outside it reaches neither, not even at the hub's address there.
hub_ip=$(docker inspect -f "{{(index .NetworkSettings.Networks \"${project}_apps\").IPAddress}}" roedy-hub)
check "the hub has its fixed address on the apps' network (actual: $hub_ip)" [ "$hub_ip" = 172.30.240.2 ]
check "the apps' network reaches the example app" \
  docker run --rm --network "${project}_apps" "$alpine" wget -q -T 5 -O /dev/null http://roedy-hub-example:8080/example/healthz
check "the apps' network reaches the internal listener" sh -c \
  "docker run --rm --network '${project}_apps' '$alpine' wget -q -T 5 -O - http://roedy-hub:8081/internal/v1/jwks | grep -q '\"keys\"'"
check "outside the apps' network the internal listener is unreachable" sh -c \
  "! docker run --rm '$alpine' wget -q -T 5 -O /dev/null http://$hub_ip:8081/internal/v1/jwks"
example_ip=$(docker inspect -f "{{(index .NetworkSettings.Networks \"${project}_apps\").IPAddress}}" roedy-hub-example)
check "outside the apps' network the example app is unreachable" sh -c \
  "! docker run --rm '$alpine' wget -q -T 5 -O /dev/null http://$example_ip:8080/example/healthz"
check "the example app is running" [ "$(docker inspect -f '{{.State.Running}}' roedy-hub-example)" = true ]

# Backup and restore as in docs/ops.md, with deploy/restore.sh: the setup token
# lives in the database, so it shows which database the hub runs on.
volume="${project}_roedy-hub-data"
backup=$(mktemp -d)
token() { docker exec roedy-hub roedy-hub setup-token | sed -n 's/^Setup-Token: //p'; }
healthy() {
  i=0
  until [ "$(docker inspect -f '{{.State.Health.Status}}' roedy-hub)" = healthy ]; do
    i=$((i + 1))
    [ "$i" -le 30 ] || fail "roedy-hub not healthy after 30 s"
    sleep 1
  done
}
restore() { (cd deploy && COMPOSE_PROJECT_NAME="$project" ROEDY_HUB_VOLUME="$volume" ./restore.sh "$@"); }
original=$(token)
check "setup token before the backup" [ -n "$original" ]
compose stop roedy-hub >/dev/null
docker run --rm -v "$volume:/data:ro" -v "$backup:/backup" "$alpine" \
  tar -czf /backup/hub.tar.gz -C /data --exclude=./secret.key .
# Another database: the hub starts with a new token.
docker run --rm -v "$volume:/data" "$alpine" sh -c 'rm -f /data/roedy.sqlite3 /data/roedy.sqlite3-wal /data/roedy.sqlite3-shm'
compose start roedy-hub >/dev/null
healthy
other=$(token)
check "setup token of the new database" [ -n "$other" ]
check "a new database after removing it" [ "$other" != "$original" ]
# A damaged archive is refused, the data stays as it was.
mkdir "$backup/bad"
head -c 4096 /dev/urandom >"$backup/bad/roedy.sqlite3"
tar -czf "$backup/bad.tar.gz" -C "$backup/bad" .
if out=$(restore "$backup/bad.tar.gz" 2>&1); then
  fail "restore.sh accepted a damaged archive"
fi
printf '%s\n' "$out"
check "restore.sh refuses a damaged archive" sh -c "printf '%s' \"\$1\" | grep -q 'missing or damaged'" sh "$out"
check "the hub stays stopped after a failed restore" [ "$(docker inspect -f '{{.State.Running}}' roedy-hub)" = false ]
compose start roedy-hub >/dev/null
healthy
check "the data is unchanged after a failed restore" [ "$(token)" = "$other" ]
# The backup restores the original database.
check "restore.sh restores the backup" restore "$backup/hub.tar.gz"
healthy
check "the original database after the restore" [ "$(token)" = "$original" ]
# One restore at a time: a held lock refuses a second one before any change.
mkdir deploy/.roedy-restore.lock
test_lock=deploy/.roedy-restore.lock
if out=$(restore --recover 2>&1); then
  fail "restore.sh ran while the lock was held"
fi
rmdir "$test_lock"
test_lock=
check "restore.sh refuses to run while locked" sh -c "printf '%s' \"\$1\" | grep -q 'another restore'" sh "$out"
check "the hub keeps running when refused" [ "$(docker inspect -f '{{.State.Running}}' roedy-hub)" = true ]
check "restore.sh --recover with nothing to recover" restore --recover
healthy

echo "Compose smoke test passed"
