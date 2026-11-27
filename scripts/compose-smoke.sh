#!/bin/sh
# Smoke test of deploy/compose.yml (ci.yml, locally `just smoke-compose`):
# validates the compose files, starts Caddy, the hub and the example app with
# deploy/compose.test.yml and checks the network layout – only Caddy published,
# the hub reachable through Caddy, its internal listener reachable from the
# apps' network but not from Caddy, the apps not reachable from Caddy. Needs no
# setup of the hub.
#
#   scripts/compose-smoke.sh
#
# Images: ROEDY_HUB_IMAGE (default roedy-hub:dev) and ROEDY_EXAMPLE_IMAGE
# (default roedy-hub-example:dev), built with `just image` and
# `just image-example`.
set -eu

cd "$(dirname "$0")/.."
# alpine as in the Dockerfile: wget from the apps' network.
# shellcheck disable=SC2016 # $BUILDPLATFORM is the Dockerfile's text
alpine=$(sed -n 's/^FROM --platform=$BUILDPLATFORM \(alpine:[^ ]*\) AS rootfs$/\1/p' Dockerfile)
[ -n "$alpine" ] || {
  echo "Smoke test failed: no alpine image in the Dockerfile" >&2
  exit 1
}

export ROEDY_PUBLIC_URL=http://localhost:8080
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
check "compose.yml is valid" env ROEDY_PUBLIC_URL=https://app.example.com \
  docker compose -f deploy/compose.yml config -q
check "compose.yml with the profile mail is valid" env ROEDY_PUBLIC_URL=https://app.example.com \
  docker compose -f deploy/compose.yml --profile mail config -q
check "compose.test.yml is valid" compose config -q
published=$(env ROEDY_PUBLIC_URL=https://app.example.com docker compose -f deploy/compose.yml --profile mail \
  config --format json | jq -r '.services | to_entries[] | select(.value.ports) | .key')
check "only Caddy publishes ports (actual: $published)" [ "$published" = caddy ]

for container in roedy-caddy roedy-hub roedy-hub-example roedy-mail; do
  if docker container inspect "$container" >/dev/null 2>&1 &&
    [ "$(docker inspect -f '{{index .Config.Labels "com.docker.compose.project"}}' "$container")" != "$project" ]; then
    echo "Smoke test not run: container $container exists outside this test; remove it or run the test elsewhere" >&2
    exit 1
  fi
done
trap cleanup EXIT
cleanup
compose up -d --wait --wait-timeout 60 || fail "the services did not start"

# Through Caddy at 127.0.0.1:8080.
url=http://127.0.0.1:8080
i=0
until curl -fsS -o /dev/null "$url/readyz" 2>/dev/null; do
  i=$((i + 1))
  [ "$i" -le 30 ] || fail "no /readyz through Caddy after 30 s"
  sleep 1
done
check "/readyz through Caddy" sh -c "curl -fsS '$url/readyz' | grep -q '\"ok\"'"
check "the web app through Caddy" sh -c "curl -fsS '$url/login' | grep -q '<html'"
check "the hub's CSP passes Caddy" sh -c "curl -fsS -D - -o /dev/null '$url/' | grep -qi '^content-security-policy: '"
check "compressed by Caddy" sh -c "curl -fsS -H 'Accept-Encoding: gzip' -D - -o /dev/null '$url/' | grep -qi '^content-encoding: gzip'"
check "no internal API through Caddy" sh -c "! curl -s '$url/internal/v1/jwks' | grep -q '\"keys\"'"

# Only Caddy is published.
for container in roedy-hub roedy-hub-example; do
  check "$container publishes no port" [ -z "$(docker port "$container")" ]
done

# Caddy (network edge) reaches the hub's public port, but neither its internal
# listener nor the apps.
check "Caddy reaches the hub's public port" docker exec roedy-caddy wget -q -T 5 -O /dev/null http://roedy-hub:8080/healthz
check "Caddy does not reach the internal listener" sh -c "! docker exec roedy-caddy wget -q -T 5 -O /dev/null http://roedy-hub:8081/internal/v1/jwks"
check "the apps' network reaches the example app" \
  docker run --rm --network "${project}_apps" "$alpine" wget -q -T 5 -O /dev/null http://roedy-hub-example:8080/example/healthz
check "Caddy does not reach the example app" sh -c "! docker exec roedy-caddy wget -q -T 5 -O /dev/null http://roedy-hub-example:8080/example/healthz"

# An app on the network apps reaches the internal listener.
check "the apps' network reaches the internal listener" sh -c \
  "docker run --rm --network '${project}_apps' '$alpine' wget -q -T 5 -O - http://roedy-hub:8081/internal/v1/jwks | grep -q '\"keys\"'"
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
