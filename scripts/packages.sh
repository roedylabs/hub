#!/usr/bin/env bash
# The packages and crates the hub publishes for the Roedy apps (docs/plan.md
# § 9.4): `@roedy/tokens`, `@roedy/jmap` and `@roedy/ui` on npm,
# `roedy-hub-contract` and `roedy-jmap` on crates.io, all under the hub's
# version (package.json; scripts/release.sh sets it everywhere).
#
#   scripts/packages.sh pack <dir>       Build and pack the npm packages into <dir> (pnpm pack:
#                                        `workspace:` versions and publishConfig resolved).
#   scripts/packages.sh verify <dir>     Install the tarballs from <dir> into a throwaway SvelteKit
#                                        app outside the workspace (scripts/packages/consumer) with
#                                        npm, then type check, build, test and import them in Node.
#   scripts/packages.sh crates           `cargo package` of both crates: packs them like
#                                        `cargo publish` and builds them from the packed sources.
#   scripts/packages.sh publish-npm <dir> --dry-run
#                                        `npm publish --dry-run` of the tarballs: no sign-in, no upload.
#   scripts/packages.sh publish-npm <dir>    Release workflow only: publish the tarballs with provenance.
#   scripts/packages.sh publish-crates       Release workflow only: publish both crates.
#
# `just packages` runs everything but the publishing (CI job "Packages" on
# every pull request). Publishing skips what the registry already has, so a
# rerun of a release continues where it stopped; a version is never
# overwritten.
set -euo pipefail

die() {
  echo "Error: $*" >&2
  exit 1
}

# Publishing order: @roedy/ui depends on @roedy/tokens.
npm_packages=(tokens jmap ui)
crates=(roedy-hub-contract roedy-jmap)

cd "$(git rev-parse --show-toplevel)"
version=$(node -p 'require("./package.json").version')

# Field of a tarball's package.json: <tarball> <field>
tarball_field() { tar -xzOf "$1" package/package.json | node -e 'process.stdout.write(String(JSON.parse(require("fs").readFileSync(0, "utf8"))[process.argv[1]]))' "$2"; }
# The tarball of a package in <dir>: <dir> <name without scope>
tarball() { echo "$1/roedy-$2-$version.tgz"; }
# npm dist-tag: pre-releases (0.2.0-rc.1) must not become "latest".
dist_tag() { case $version in *-*) echo next ;; *) echo latest ;; esac; }

cmd=${1:-}
case $cmd in
pack)
  dir=${2:?usage: scripts/packages.sh pack <dir>}
  # The directory is emptied first: only one that holds nothing but tarballs.
  if [[ -d $dir ]] && find "$dir" -mindepth 1 ! -name '*.tgz' | grep -q .; then
    die "$dir holds more than tarballs – pick an empty directory"
  fi
  rm -rf "$dir"
  mkdir -p "$dir"
  dir=$(cd "$dir" && pwd)
  for name in "${npm_packages[@]}"; do
    # prepack builds dist/ and copies the licence in, postpack removes it.
    pnpm --filter "@roedy/$name" pack --pack-destination "$dir" >/dev/null
    file=$(tarball "$dir" "$name")
    [[ -f $file ]] || die "pnpm pack wrote no $file"
    [[ $(tarball_field "$file" version) == "$version" ]] || die "$file does not have version $version"
    # What the package ships: no tests, no sources, and the licence.
    files=$(tar -tzf "$file")
    ! grep -Eq '\.test\.|^package/src/' <<<"$files" || die "$file contains tests or sources"
    grep -qx package/LICENSE <<<"$files" || die "$file has no LICENSE"
    echo "$file: $(wc -l <<<"$files") files, $(($(wc -c <"$file") / 1024)) KiB"
  done
  ;;

verify)
  dir=${2:?usage: scripts/packages.sh verify <dir>}
  dir=$(cd "$dir" && pwd)
  for name in "${npm_packages[@]}"; do
    [[ -f $(tarball "$dir" "$name") ]] || die "$(tarball "$dir" "$name") missing – run 'scripts/packages.sh pack $dir' first"
  done
  work=$(mktemp -d "${TMPDIR:-/tmp}/roedy-consumer.XXXXXX")
  trap 'rm -rf "$work"' EXIT
  cp -R scripts/packages/consumer/. "$work"
  # Tooling in the versions the workspace uses; the packages from the
  # tarballs. The override makes @roedy/ui's dependency on @roedy/tokens use
  # the tarball too, since that version is not on npm yet.
  node - "$work" "$dir" "$version" <<'EOF'
const fs = require('fs');
const [work, dir, version] = process.argv.slice(2);
const web = JSON.parse(fs.readFileSync('apps/web/package.json', 'utf8'));
const ui = JSON.parse(fs.readFileSync('packages/ui/package.json', 'utf8'));
const pinned = { ...web.devDependencies, ...web.dependencies };
const pick = (...names) => Object.fromEntries(names.map((n) => [n, pinned[n] ?? ui.devDependencies[n]]));
const tgz = (name) => `file:${dir}/roedy-${name}-${version}.tgz`;
const pkg = {
  name: 'roedy-package-consumer',
  private: true,
  type: 'module',
  imports: { '#lib/*': './src/lib/*' },
  dependencies: {
    '@roedy/jmap': tgz('jmap'),
    '@roedy/tokens': tgz('tokens'),
    '@roedy/ui': tgz('ui'),
    ...pick('@sveltejs/kit', '@tabler/icons-svelte-runes', 'svelte'),
  },
  devDependencies: pick(
    '@sveltejs/adapter-static',
    '@sveltejs/vite-plugin-svelte',
    '@testing-library/svelte',
    '@types/node',
    'jsdom',
    'svelte-check',
    'typescript',
    'vite',
    'vitest',
  ),
  overrides: { '@roedy/tokens': '$@roedy/tokens' },
};
for (const [name, spec] of Object.entries({ ...pkg.dependencies, ...pkg.devDependencies })) {
  if (!spec) throw new Error(`no version for ${name} in the workspace`);
}
fs.writeFileSync(`${work}/package.json`, JSON.stringify(pkg, null, 2) + '\n');
EOF
  # The same cooldown as pnpm's minimumReleaseAge (pnpm-workspace.yaml) for
  # the dependencies npm resolves here; no install scripts.
  before=$(node -p 'new Date(Date.now() - 7 * 864e5).toISOString()')
  (
    cd "$work"
    npm install --no-audit --no-fund --ignore-scripts --before="$before" --loglevel=error
    npx --no-install svelte-kit sync
    npx --no-install svelte-check --tsconfig ./tsconfig.json --fail-on-warnings
    npx --no-install vite build --logLevel warn
    npx --no-install vitest run
    node node.mjs
  )
  echo "The packed packages work in an app outside the workspace"
  ;;

crates)
  # Packs both crates as `cargo publish` would and builds them from the
  # packed sources; warnings (e.g. missing metadata) fail it.
  pkgs=()
  for crate in "${crates[@]}"; do pkgs+=(-p "$crate"); done
  log=$(mktemp "${TMPDIR:-/tmp}/roedy-cargo-package.XXXXXX")
  trap 'rm -f "$log"' EXIT
  cargo package --locked "${pkgs[@]}" 2>&1 | tee "$log"
  # Expected: the contract's vector test reads docs/app-contract/vectors.json
  # of the repository, so it stays out of the package.
  # shellcheck disable=SC2016 # backticks of cargo's message
  ! grep '^warning' "$log" | grep -vq '^warning: ignoring test `vectors` as `tests/vectors.rs` is not included' ||
    die "cargo package warned"
  for crate in "${crates[@]}"; do
    file="${CARGO_TARGET_DIR:-target}/package/$crate-$version.crate"
    [[ -f $file ]] || die "$file missing"
    echo "$file: $(tar -tzf "$file" | wc -l) files"
  done
  ;;

publish-npm)
  dir=${2:?usage: scripts/packages.sh publish-npm <dir> [--dry-run]}
  dry_run=${3:-}
  [[ -z $dry_run || $dry_run == --dry-run ]] || die "unknown option $dry_run"
  # Absolute: npm reads a relative "dir/file.tgz" as a GitHub repository.
  dir=$(cd "$dir" && pwd)
  tag=$(dist_tag)
  for name in "${npm_packages[@]}"; do
    file=$(tarball "$dir" "$name")
    [[ -f $file ]] || die "$file missing"
    pkg=$(tarball_field "$file" name)
    if [[ -n $dry_run ]]; then
      # Checks the tarball and the options without signing in or uploading.
      npm publish "$file" --access public --tag "$tag" --dry-run
      continue
    fi
    if [[ $(npm view "$pkg@$version" version 2>/dev/null) == "$version" ]]; then
      echo "$pkg@$version is already on npm"
      continue
    fi
    # Provenance links the package to this workflow run (needs id-token: write).
    npm publish "$file" --access public --tag "$tag" --provenance
  done
  ;;

publish-crates)
  for crate in "${crates[@]}"; do
    # crates.io's API asks for a User-Agent that names the client.
    status=$(curl -s -o /dev/null -w '%{http_code}' -A "roedy-hub-release (github.com/roedylabs/hub)" \
      "https://crates.io/api/v1/crates/$crate/$version")
    case $status in
    200)
      echo "$crate $version is already on crates.io"
      continue
      ;;
    404) ;;
    *) die "could not check whether $crate $version is on crates.io (HTTP $status)" ;;
    esac
    cargo publish --locked -p "$crate"
  done
  ;;

*)
  die "usage: scripts/packages.sh pack|verify <dir> | crates | publish-npm <dir> [--dry-run] | publish-crates"
  ;;
esac
