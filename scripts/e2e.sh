#!/usr/bin/env bash
# End-to-end suite (apps/e2e): starts the hub with the built-in TLS from a
# self-signed certificate for localhost (scripts/test-cert.sh) and the example
# app (crates/example-app), runs Playwright against https://localhost:8443 and
# stops everything again. Two ways to start them:
#
#   scripts/e2e.sh [arguments for playwright test …]
#       deploy/compose.yml with deploy/compose.test.yml and the images from
#       `just image` and `just image-example` (ROEDY_HUB_IMAGE,
#       ROEDY_EXAMPLE_IMAGE; default roedy-hub:dev, roedy-hub-example:dev).
#       Needs Docker with Compose (or Podman with `ROEDY_E2E_COMPOSE="podman
#       compose"`). The way CI runs it, against the image CI built.
#   scripts/e2e.sh --host [--no-build] [arguments for playwright test …]
#       Debug binaries on the host instead of containers (no Docker needed):
#       builds the web app, the hub with the embedded web app and the example
#       app into target/e2e/ unless --no-build.
#
# Once beforehand: the browser, `pnpm --filter @roedy/hub-e2e exec playwright
# install chromium-headless-shell` (or scripts/dev/browser.sh install, whose
# browser under $TMPDIR/roedy-hub-dev/ms-playwright is found as well).
# Ports 127.0.0.1:8443 and 127.0.0.1:8080 (with --host also 8081 and 8090)
# must be free. Needs curl and openssl.
#
# The suite gets the setup and the hub through environment variables
# (apps/e2e/env.ts). To pair the example app it calls `scripts/e2e.sh --pair`
# with the code in ROEDY_EXAMPLE_PAIRING_CODE, which (re)starts the example app
# the way this run started it (ROEDY_E2E_MODE).
set -euo pipefail

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO"
# Every request here and in the suite goes to the local hub: never through a
# proxy from the environment (Node's fetch uses one with NODE_USE_ENV_PROXY).
export NO_PROXY='*' no_proxy='*'

fail() {
  echo "e2e: $*" >&2
  exit 1
}
# The hub and the example app on the host talk to each other only: without a
# proxy from the environment (not every client honours NO_PROXY=*).
local_env=(env -u http_proxy -u https_proxy -u HTTP_PROXY -u HTTPS_PROXY -u all_proxy -u ALL_PROXY)

# The Compose command of compose mode: a project of its own, as in
# scripts/compose-smoke.sh; the fixed container names (roedy-hub,
# roedy-hub-example) refuse a run next to another one.
read -r -a engine <<<"${ROEDY_E2E_COMPOSE:-docker compose}"
compose=("${engine[@]}" -p roedy-e2e -f "$REPO/deploy/compose.yml" -f "$REPO/deploy/compose.test.yml")

# Called by the suite while it runs: restarts the example app with the pairing
# code in ROEDY_EXAMPLE_PAIRING_CODE – in compose mode by recreating its
# container, in host mode by restarting the binary and waiting until it is
# paired (its readyz answers 200 only then).
if [ "${1:-}" = --pair ]; then
  code=${ROEDY_EXAMPLE_PAIRING_CODE:?}
  case "${ROEDY_E2E_MODE:?only while scripts/e2e.sh runs the suite}" in
    compose)
      ROEDY_EXAMPLE_PAIRING_CODE="$code" "${compose[@]}" up -d --wait --wait-timeout 60 roedy-hub-example
      exit
      ;;
    host) ;;
    *) fail "unknown ROEDY_E2E_MODE $ROEDY_E2E_MODE" ;;
  esac
  run=${ROEDY_E2E_RUN:?}
  kill -INT "$(cat "$run/example.pid")" 2>/dev/null || true
  for _ in $(seq 1 50); do
    curl -s -o /dev/null --max-time 1 http://127.0.0.1:8090/example/healthz || break
    sleep 0.1
  done
  "${local_env[@]}" ROEDY_PAIRING_CODE="$code" ROEDY_PUBLIC_URL=https://localhost:8443/example \
    ROEDY_HUB_INTERNAL_URL=http://127.0.0.1:8081 ROEDY_LISTEN=127.0.0.1:8090 \
    nohup "$REPO/target/e2e/roedy-hub-example" >>"$run/example.log" 2>&1 &
  echo $! >"$run/example.pid"
  for _ in $(seq 1 100); do
    [ "$(curl -s -o /dev/null -w '%{http_code}' --max-time 1 http://127.0.0.1:8090/example/readyz)" = 200 ] && exit 0
    kill -0 "$(cat "$run/example.pid")" 2>/dev/null || break
    sleep 0.1
  done
  tail -n 20 "$run/example.log" >&2
  fail "the example app did not pair"
fi

mode=compose
build=1
while [ "$#" -gt 0 ]; do
  case "$1" in
    --host) mode=host ;;
    --no-build) build= ;;
    *) break ;;
  esac
  shift
done
[ "$mode" = host ] || [ -n "$build" ] || fail "--no-build only with --host (compose takes the images as they are)"
export ROEDY_E2E_MODE="$mode"

run=$(mktemp -d)
export ROEDY_E2E_RUN="$run"
scripts/test-cert.sh "$run/tls" localhost || fail "no test certificate (openssl missing?)"

export ROEDY_E2E_URL=https://localhost:8443
export ROEDY_E2E_HTTP_URL=http://127.0.0.1:8080
export ROEDY_E2E_CERT="$run/tls/cert.pem"
# Node (Playwright's request API) trusts the test certificate on top of its
# roots; the browser gets it in apps/e2e/playwright.config.ts.
export NODE_EXTRA_CA_CERTS="$ROEDY_E2E_CERT"

# Up to the readiness over HTTPS, trusting only the test certificate.
wait_ready() {
  for _ in $(seq 1 60); do
    curl -fs -o /dev/null --max-time 2 --cacert "$ROEDY_E2E_CERT" "$ROEDY_E2E_URL/readyz" && return 0
    sleep 1
  done
  return 1
}

if [ "$mode" = compose ]; then
  export ROEDY_PUBLIC_URL="$ROEDY_E2E_URL" ROEDY_TEST_TLS_DIR="$run/tls"
  export ROEDY_E2E_EXAMPLE_INTERNAL=http://roedy-hub-example:8080
  # shellcheck disable=SC2329 # invoked by the EXIT trap
  cleanup() {
    status=$?
    if [ "$status" -ne 0 ]; then
      "${compose[@]}" logs --no-color >"$REPO/apps/e2e/test-results/compose.log" 2>&1 || true
      echo "e2e: container logs in apps/e2e/test-results/compose.log" >&2
    fi
    "${compose[@]}" down -v --remove-orphans >/dev/null 2>&1 || true
    rm -rf "$run"
    exit "$status"
  }
  trap cleanup EXIT
  mkdir -p apps/e2e/test-results
  "${compose[@]}" up -d --wait --wait-timeout 60 || fail "the services did not start"
  wait_ready || fail "no /readyz over HTTPS after 60 s"
  ROEDY_E2E_SETUP_TOKEN=$("${engine[0]}" exec roedy-hub roedy-hub setup-token | sed -n 's/^Setup-Token: //p')
else
  bin="$REPO/target/e2e"
  if [ -n "$build" ]; then
    pnpm --filter @roedy/hub-web build
    # Separate builds: built together, Cargo would unify the features.
    cargo build --locked -p roedy-hub-server --features embed-web
    cargo build --locked -p roedy-hub-example
    # Own copies: `cargo test` rebuilds target/debug/roedy-hub without the web app.
    install -D -m 0755 target/debug/roedy-hub "$bin/roedy-hub"
    install -D -m 0755 target/debug/roedy-hub-example "$bin/roedy-hub-example"
  fi
  [ -x "$bin/roedy-hub" ] && [ -x "$bin/roedy-hub-example" ] || fail "$bin missing – run without --no-build"
  hub_env=(
    ROEDY_DATA_DIR="$run/data" ROEDY_PUBLIC_URL="$ROEDY_E2E_URL" ROEDY_LOG_FORMAT=text
    ROEDY_TLS=files ROEDY_TLS_CERT_FILE="$run/tls/cert.pem" ROEDY_TLS_KEY_FILE="$run/tls/key.pem"
    ROEDY_LISTEN=127.0.0.1:8443 ROEDY_HTTP_LISTEN=127.0.0.1:8080 ROEDY_INTERNAL_LISTEN=127.0.0.1:8081
  )
  mkdir -p "$run/data"
  "${local_env[@]}" "${hub_env[@]}" "$bin/roedy-hub" serve >"$run/hub.log" 2>&1 &
  echo $! >"$run/hub.pid"
  # The example app unpaired, as in the Compose setup; --pair restarts it.
  "${local_env[@]}" ROEDY_PUBLIC_URL="$ROEDY_E2E_URL/example" ROEDY_HUB_INTERNAL_URL=http://127.0.0.1:8081 \
    ROEDY_LISTEN=127.0.0.1:8090 "$bin/roedy-hub-example" >"$run/example.log" 2>&1 &
  echo $! >"$run/example.pid"
  # shellcheck disable=SC2329 # invoked by the EXIT trap
  cleanup() {
    status=$?
    kill -INT "$(cat "$run/example.pid")" 2>/dev/null || true
    kill "$(cat "$run/hub.pid")" 2>/dev/null || true
    if [ "$status" -ne 0 ]; then
      mkdir -p "$REPO/apps/e2e/test-results"
      cat "$run/hub.log" "$run/example.log" >"$REPO/apps/e2e/test-results/host.log" 2>/dev/null || true
      echo "e2e: hub and example logs in apps/e2e/test-results/host.log" >&2
    fi
    wait 2>/dev/null || true
    rm -rf "$run"
    exit "$status"
  }
  trap cleanup EXIT
  wait_ready || {
    cat "$run/hub.log" >&2
    fail "no /readyz over HTTPS after 60 s"
  }
  ROEDY_E2E_SETUP_TOKEN=$(env "${hub_env[@]}" "$bin/roedy-hub" setup-token | sed -n 's/^Setup-Token: //p')
  export ROEDY_E2E_EXAMPLE_INTERNAL=http://127.0.0.1:8090
fi
[ -n "$ROEDY_E2E_SETUP_TOKEN" ] || fail "no setup token"
export ROEDY_E2E_SETUP_TOKEN

dev_browsers="${TMPDIR:-/tmp}/roedy-hub-dev/ms-playwright"
if [ -z "${PLAYWRIGHT_BROWSERS_PATH:-}" ] && [ -d "$dev_browsers" ]; then
  export PLAYWRIGHT_BROWSERS_PATH="$dev_browsers"
fi

pnpm --filter @roedy/hub-e2e exec playwright test "$@"
