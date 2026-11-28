// One version for everything the hub releases (node --test scripts/): the
// root package.json is the source, and `scripts/release.sh prepare` writes it
// to Cargo.toml (the crates, also `roedy-hub-contract` and `roedy-jmap` on
// crates.io) and to the published npm packages (packages/*). A copy that
// drifts would publish a package under another version than the release.

import assert from 'node:assert/strict';
import { existsSync, readdirSync, readFileSync } from 'node:fs';
import { join } from 'node:path';
import { test } from 'node:test';

const root = join(import.meta.dirname, '..');
const json = (path) => JSON.parse(readFileSync(join(root, path), 'utf8'));
const version = json('package.json').version;

/** The workspace packages under packages/, which are all published. */
const packages = readdirSync(join(root, 'packages'), { withFileTypes: true })
  .filter((entry) => entry.isDirectory())
  .map((entry) => `packages/${entry.name}/package.json`)
  .filter((path) => existsSync(join(root, path)));

test('the release version is SemVer 0.x', () => {
  assert.match(version, /^0\.(0|[1-9]\d*)\.(0|[1-9]\d*)(-[0-9A-Za-z]+(\.[0-9A-Za-z]+)*)?$/);
});

test('Cargo.toml has the release version', () => {
  const cargo = readFileSync(join(root, 'Cargo.toml'), 'utf8');
  const section = cargo.slice(cargo.indexOf('[workspace.package]'));
  assert.equal(/^version = "([^"]+)"$/m.exec(section)?.[1], version);
});

test('the published packages have the release version and are publishable', () => {
  assert.deepEqual(
    packages.map((path) => json(path).name),
    ['@roedy/jmap', '@roedy/tokens', '@roedy/ui'],
  );
  for (const path of packages) {
    const pkg = json(path);
    assert.equal(pkg.version, version, path);
    assert.equal(pkg.private, undefined, `${path} must not be private`);
    assert.equal(pkg.license, 'AGPL-3.0-only', path);
    assert.equal(pkg.repository?.url, 'git+https://github.com/roedylabs/hub.git', path);
    assert.equal(pkg.repository?.directory, path.replace('/package.json', ''), path);
    assert.equal(pkg.publishConfig?.access, 'public', path);
    // The workspace uses the sources, the published package the build.
    assert.ok(pkg.publishConfig?.exports, `${path}: publishConfig.exports`);
    for (const [entry, target] of Object.entries(pkg.publishConfig.exports)) {
      const files = typeof target === 'string' ? [target] : Object.values(target);
      for (const file of files) assert.doesNotMatch(file, /^\.\/src\//, `${path}: ${entry}`);
    }
  }
});

test('the npm packages depend on each other in the release version', () => {
  for (const path of packages) {
    const pkg = json(path);
    for (const [name, spec] of Object.entries({ ...pkg.dependencies, ...pkg.peerDependencies })) {
      // `workspace:*` becomes the exact version when packing.
      if (name.startsWith('@roedy/')) assert.equal(spec, 'workspace:*', `${path}: ${name}`);
    }
  }
});
