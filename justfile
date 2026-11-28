# Tasks for development, image and release (https://just.systems). `just`
# lists all recipes. Without just installed: `pnpm exec just …` (package
# rust-just, pinned in package.json). Image and release recipes taken over from
# Roedy Mail (../mail).

set shell := ["bash", "-euo", "pipefail", "-c"]

# Target platforms of the image: statically linked with musl.
targets := "x86_64-unknown-linux-musl aarch64-unknown-linux-musl"

# List recipes
default:
    @just --list

# All checks as in the pre-push hook
check:
    cargo fmt --all -- --check
    cargo clippy --workspace --all-targets -- -D warnings
    cargo test --workspace
    pnpm lint
    pnpm check
    pnpm test

# Build the web app (apps/web/build, embedded by `just dist`)
web:
    pnpm build

# Hub binaries with embedded web app to dist/<amd64|arm64>/roedy-hub (needs zig, cargo-zigbuild, cargo-auditable and the rustup targets; default both targets)
dist *targets=targets: web
    rm -rf dist/amd64/roedy-hub dist/arm64/roedy-hub
    # One call for all targets: the LTO steps (codegen-units = 1) run in parallel.
    cargo auditable zigbuild --release --locked -p roedy-hub-server --features embed-web \
      {{ prepend("--target ", targets) }}
    for target in {{ targets }}; do \
      install -D -m 0755 "target/$target/release/roedy-hub" "dist/$({{ just_executable() }} _arch "$target")/roedy-hub"; \
    done
    ls -l dist/*/roedy-hub

# Example app binaries to dist/<amd64|arm64>/roedy-hub-example for deploy/compose.test.yml (debug build, not part of the image)
dist-example *targets=targets:
    rm -rf dist/amd64/roedy-hub-example dist/arm64/roedy-hub-example
    # A separate call: built together with the hub, Cargo would unify the
    # features and put the example's WebSocket crates into the hub's binary.
    cargo zigbuild --locked -p roedy-hub-example {{ prepend("--target ", targets) }}
    for target in {{ targets }}; do \
      install -D -m 0755 "target/$target/debug/roedy-hub-example" "dist/$({{ just_executable() }} _arch "$target")/roedy-hub-example"; \
    done
    ls -l dist/*/roedy-hub-example

# Image architecture of a Rust target
[private]
_arch target:
    @case "{{ target }}" in x86_64-*) echo amd64 ;; aarch64-*) echo arm64 ;; *) echo "unknown target {{ target }}" >&2; exit 1 ;; esac

# Build the image for the local platform from dist/ (Docker with buildx)
image tag="roedy-hub:dev":
    docker buildx build --load -t {{ tag }} .

# Build the example app's image for the local platform from dist/ (Docker with buildx)
image-example tag="roedy-hub-example:dev":
    docker buildx build --load -f deploy/example-app.Dockerfile -t {{ tag }} .

# Smoke test and budgets of an image (version optional, e.g. 0.2.0)
smoke image="roedy-hub:dev" version="":
    scripts/image-smoke.sh {{ image }} {{ version }}

# Smoke test of deploy/compose.yml with the test override (images from `just image` and `just image-example`)
smoke-compose:
    scripts/compose-smoke.sh

# End-to-end suite against deploy/compose.test.yml with the images from `just image` and `just image-example` (Docker or Podman; arguments go to `playwright test`)
e2e *args:
    scripts/e2e.sh {{ args }}

# End-to-end suite against debug binaries on the host, without containers (arguments go to `playwright test`)
e2e-host *args:
    scripts/e2e.sh --host {{ args }}

# Prepare a release: version, CHANGELOG.md and PR (version without v, e.g. 0.2.0)
release version:
    scripts/release.sh prepare {{ version }}

# After merging the release PR: set tag v<version> on main and push it
release-tag version:
    scripts/release.sh tag {{ version }}
