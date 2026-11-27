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
export COMPOSE_PROJECT_NAME=roedy
compose() { docker compose -f deploy/compose.yml -f deploy/compose.test.yml "$@"; }

fail() {
  echo "Smoke test failed: $*" >&2
  compose logs >&2 2>&1 || true
  exit 1
}
cleanup() { compose down -v --remove-orphans >/dev/null 2>&1 || true; }
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
check "Caddy does not reach the example app" sh -c "! docker exec roedy-caddy wget -q -T 5 -O /dev/null http://roedy-hub-example:8080/"

# An app on the network apps reaches the internal listener.
check "the apps' network reaches the internal listener" sh -c \
  "docker run --rm --network roedy_apps '$alpine' wget -q -T 5 -O - http://roedy-hub:8081/internal/v1/jwks | grep -q '\"keys\"'"
check "the example app is running" [ "$(docker inspect -f '{{.State.Running}}' roedy-hub-example)" = true ]

echo "Compose smoke test passed"
