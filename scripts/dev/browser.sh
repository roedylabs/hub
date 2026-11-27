#!/usr/bin/env bash
# Runs a Playwright script against the Vite dev server of the web app, for
# visual checks, screenshots and the accessibility audit (scripts/a11y/).
# Taken over from Roedy Mail (scripts/dev/browser.sh), with the Playwright
# version of the end-to-end suite (apps/e2e/package.json). Playwright with Chromium
# (headless) and axe-core are installed under $TMPDIR, outside the repository.
# `run` starts the dev server, runs the script and stops the server again.
#
#   scripts/dev/browser.sh install
#   scripts/dev/browser.sh run <script.mjs> [arguments …]
#
# The script imports `playwright` (`import { chromium } from 'playwright'`,
# for the accessibility audit also `axe-core`), finds the app at
# process.env.BASE (http://127.0.0.1:5173) and runs in the caller's directory
# (relative arguments apply there). It answers the API itself
# (`page.route('**/api/v1/**', …)`); requests it does not intercept go to an
# address without a server, never to a running backend. After a change of
# dependencies Vite may reload the first page load ("optimized dependencies
# changed"): then load the page once more.
#
# Network: `install` needs registry.npmjs.org, cdn.playwright.dev,
# playwright.download.prss.microsoft.com and storage.googleapis.com; `run`
# needs pnpm and the installed workspace dependencies.
set -euo pipefail

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
# The version of the end-to-end suite, so that both use the same browser
# (scripts/e2e.sh finds the one installed here).
PLAYWRIGHT_VERSION="$(node -p 'require(process.argv[1]).devDependencies["@playwright/test"]' "$REPO/apps/e2e/package.json")"
# axe-core (MPL-2.0) for the accessibility audit (scripts/a11y/); not part of the bundle.
# The same version as the component tests' (apps/web/package.json), so both report the same rules.
AXE_VERSION="$(node -p 'require(process.argv[1]).devDependencies["axe-core"]' "$REPO/apps/web/package.json")"
PORT=5173
DEV="${TMPDIR:-/tmp}/roedy-hub-dev"
DIR="$DEV/browser"
export PLAYWRIGHT_BROWSERS_PATH="$DEV/ms-playwright"
export npm_config_cache="$DEV/npm-cache"

fail() {
  echo "browser.sh: $*" >&2
  exit 1
}


case "${1:-}" in
  install)
    mkdir -p "$DIR"
    cd "$DIR"
    [ -f package.json ] || echo '{ "name": "roedy-hub-dev-browser", "private": true }' >package.json
    npm install --no-audit --no-fund "playwright@$PLAYWRIGHT_VERSION" "axe-core@$AXE_VERSION"
    npx playwright install chromium-headless-shell
    ;;
  run)
    shift
    script="${1:?usage: $0 run <script.mjs> [arguments …]}"
    shift
    [ -f "$script" ] || fail "script not found: $script"
    if ! [ -d "$DIR/node_modules/playwright" ] || ! compgen -G "$PLAYWRIGHT_BROWSERS_PATH/chromium_headless_shell-*" >/dev/null; then
      fail "Playwright or Chromium missing – first: $0 install"
    fi
    installed="$(node -p 'require(process.argv[1]).version' "$DIR/node_modules/playwright/package.json")"
    [ "$installed" = "$PLAYWRIGHT_VERSION" ] ||
      fail "Playwright $installed instead of $PLAYWRIGHT_VERSION – first: $0 install"
    caller="$PWD"
    script="$(cd "$(dirname "$script")" && pwd)/$(basename "$script")"

    # Make `import 'playwright'` resolvable next to the script without moving it.
    link="$(dirname "$script")/node_modules"
    made_link=
    if [ -L "$link" ] && [ "$(readlink "$link")" = "$DIR/node_modules" ]; then
      # Leftover of an aborted run: take it over and clean up afterwards.
      made_link=1
    elif [ ! -e "$link" ]; then
      ln -s "$DIR/node_modules" "$link"
      made_link=1
    fi

    cd "$REPO"
    command -v pnpm >/dev/null 2>&1 || fail "pnpm not found (see packageManager in package.json)"

    # Another server on the port would answer the readiness check in place of this run's Vite.
    if curl -s -o /dev/null --connect-timeout 2 --max-time 5 "http://127.0.0.1:$PORT/"; then
      fail "port $PORT is already in use – stop the other server first"
    fi

    log="$(mktemp "$DEV/vite.XXXXXX.log")"
    parent=$$
    # Own session and process group; a watcher ends the group even when this
    # script ends without the EXIT trap (e.g. SIGKILL on a timeout).
    (
      cd apps/web
      export ROEDY_DEV_BACKEND=http://127.0.0.1:9
      exec setsid bash -c '
        pnpm exec vite dev --host 127.0.0.1 --port '"$PORT"' --strictPort &
        while kill -0 '"$parent"' 2>/dev/null && kill -0 $! 2>/dev/null; do sleep 1; done
        kill -- -$$ 2>/dev/null'
    ) >"$log" 2>&1 &
    vite=$!
    # shellcheck disable=SC2329 # invoked by the EXIT trap
    cleanup() {
      kill -- -"$vite" 2>/dev/null || kill "$vite" 2>/dev/null || true
      [ -n "$made_link" ] && rm -f "$link"
      rm -f "$log"
    }
    trap cleanup EXIT
    trap 'exit 130' INT TERM HUP

    # Ready as soon as the server responds (independent of colors and old logs).
    ready=
    for _ in $(seq 1 120); do
      # Only while this run's Vite is alive (it exits at once with --strictPort if the port was taken meanwhile).
      kill -0 "$vite" 2>/dev/null || break
      if curl -sf -o /dev/null --connect-timeout 2 --max-time 5 "http://127.0.0.1:$PORT/" && kill -0 "$vite" 2>/dev/null; then
        ready=1
        break
      fi
      sleep 0.25
    done
    if [ -z "$ready" ]; then
      cat "$log" >&2
      fail "Vite does not respond on port $PORT (log above)"
    fi

    cd "$caller"
    status=0
    BASE="http://127.0.0.1:$PORT" node "$script" "$@" || status=$?
    # Errors and warnings from Vite (compiling, proxy), otherwise invisible.
    if grep -qiE 'error|warn' "$log"; then
      echo "--- Vite reports:" >&2
      grep -iE 'error|warn' "$log" >&2 || true
    fi
    exit "$status"
    ;;
  *)
    echo "usage: $0 install | run <script.mjs> [arguments …]" >&2
    exit 2
    ;;
esac
