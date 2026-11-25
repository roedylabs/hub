// Tests of the sandbox environment (node --test scripts/): scripts/dev/sandbox-env.sh
// must redirect every place npm, pnpm and cargo write to into $TMPDIR –
// including PNPM_HOME, which the profile sets to ~/.local/share/pnpm, read-only
// in the sandbox.

import assert from 'node:assert/strict';
import { chmodSync, mkdirSync, mkdtempSync, rmSync, statSync, symlinkSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { after, describe, test } from 'node:test';
import { execFileSync, spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';

const script = resolve(dirname(fileURLToPath(import.meta.url)), 'sandbox-env.sh');
const VARS = [
  'npm_config_cache',
  'COREPACK_HOME',
  'XDG_RUNTIME_DIR',
  'XDG_CACHE_HOME',
  'XDG_DATA_HOME',
  'XDG_STATE_HOME',
  'PNPM_HOME',
  'CARGO_HOME',
];

describe('sandbox-env.sh', () => {
  const tmp = mkdtempSync(join(tmpdir(), 'roedy-sandbox-env-'));
  after(() => rmSync(tmp, { recursive: true, force: true }));

  // A pnpm placeholder in PATH, so that the corepack branch does not go online.
  const bin = join(tmp, 'bin');
  const sandboxTmp = join(tmp, 'tmp');
  for (const dir of [bin, sandboxTmp]) mkdirSync(dir);
  writeFileSync(join(bin, 'pnpm'), '#!/bin/sh\nexit 0\n');
  chmodSync(join(bin, 'pnpm'), 0o755);

  const printf = VARS.map((name) => `"${name}=$${name}"`).join(' ');
  const run = spawnSync(
    'bash',
    ['-c', `. "$1" && printf '%s\\n' ${printf} "_rm_base=\${_rm_base-unset}"`, 'bash', script],
    {
      encoding: 'utf8',
      env: {
        ...process.env,
        TMPDIR: sandboxTmp,
        PATH: `${bin}:${process.env.PATH}`,
        PNPM_HOME: join(tmp, 'home', '.local', 'share', 'pnpm'),
        CARGO_HOME: join(tmp, 'home', '.cargo'),
      },
    },
  );
  const env = Object.fromEntries(
    (run.stdout ?? '')
      .trim()
      .split('\n')
      .map((line) => {
        const at = line.indexOf('=');
        return [line.slice(0, at), line.slice(at + 1)];
      }),
  );
  const base = join(sandboxTmp, 'roedy-hub-dev');

  test('runs without errors', () => {
    assert.equal(run.error, undefined, String(run.error));
    assert.equal(run.status, 0, run.stderr);
  });

  test('redirects all write locations to $TMPDIR/roedy-hub-dev', () => {
    for (const name of VARS) {
      assert.ok(env[name]?.startsWith(`${base}/`), `${name}=${env[name]}`);
    }
  });

  test('sets PNPM_HOME even if the profile already set it', () => {
    assert.equal(env.PNPM_HOME, join(base, 'pnpm-home'));
    assert.ok(statSync(env.PNPM_HOME).isDirectory());
  });

  test('creates XDG_RUNTIME_DIR with mode 700', () => {
    assert.equal(statSync(env.XDG_RUNTIME_DIR).mode & 0o777, 0o700);
  });

  test('cleans up the helper variable', () => {
    assert.equal(env._rm_base, 'unset');
  });

  test('makes the base directory private', () => {
    assert.equal(statSync(base).mode & 0o777, 0o700);
  });

  test('creates a private base even with a permissive umask', () => {
    const otherTmp = join(tmp, 'umask-tmp');
    mkdirSync(otherTmp);
    const umask = spawnSync('bash', ['-c', 'umask 0002 && . "$1"', 'bash', script], {
      encoding: 'utf8',
      env: { ...process.env, TMPDIR: otherTmp, PATH: `${bin}:${process.env.PATH}` },
    });
    assert.equal(umask.status, 0, umask.stderr);
    assert.equal(statSync(join(otherTmp, 'roedy-hub-dev')).mode & 0o777, 0o700);
  });
});

describe('sandbox-env.sh failures', () => {
  const tmp = mkdtempSync(join(tmpdir(), 'roedy-sandbox-env-fail-'));
  after(() => rmSync(tmp, { recursive: true, force: true }));

  const bash = execFileSync('bash', ['-c', 'command -v bash'], { encoding: 'utf8' }).trim();

  /** Sources the script with exactly the tools in `bin` (no pnpm) and reports the status. */
  function source(name, prepare) {
    const dir = join(tmp, name);
    const bin = join(dir, 'bin');
    const sandboxTmp = join(dir, 'tmp');
    for (const d of [bin, sandboxTmp]) mkdirSync(d, { recursive: true });
    for (const tool of ['mkdir', 'find', 'id', 'chmod', 'sh', 'realpath']) {
      symlinkSync(execFileSync('bash', ['-c', `command -v ${tool}`], { encoding: 'utf8' }).trim(), join(bin, tool));
    }
    // npm that fails: the corepack bootstrap cannot succeed.
    writeFileSync(join(bin, 'npm'), '#!/bin/sh\nexit 1\n');
    chmodSync(join(bin, 'npm'), 0o755);
    prepare?.(join(sandboxTmp, 'roedy-hub-dev'), bin);
    return spawnSync(bash, ['-c', '. "$1" && echo sourced', 'bash', script], {
      encoding: 'utf8',
      env: { TMPDIR: sandboxTmp, PATH: bin, HOME: dir },
    });
  }

  test('fails when installing corepack fails', () => {
    const run = source('npm-fails');
    assert.notEqual(run.status, 0);
    assert.doesNotMatch(run.stdout, /sourced/);
    assert.match(run.stderr, /installing corepack failed/);
  });

  test('refuses executables that another user planted in the base', () => {
    // Files of another user cannot be created without root; a find that reports
    // one for the ownership probe stands in for them.
    const run = source('planted', (base, bin) => {
      mkdirSync(join(base, 'corepack'), { recursive: true, mode: 0o700 });
      chmodSync(base, 0o700);
      const real = join(bin, 'find');
      const target = execFileSync('readlink', [real], { encoding: 'utf8' }).trim();
      rmSync(real);
      writeFileSync(
        real,
        `#!${bash}\ncase " $* " in *" -user "*) echo "$1/node_modules/.bin/corepack" ;; *) exec ${target} "$@" ;; esac\n`,
      );
      chmodSync(real, 0o755);
    });
    assert.notEqual(run.status, 0);
    assert.doesNotMatch(run.stdout, /sourced/);
    assert.match(run.stderr, /contains files of other users/);
  });

  for (const [name, dirMode, fileMode] of [
    ['a world-writable target', 0o755, 0o777],
    ['a target in a directory others can write to', 0o777, 0o700],
    ['a group-writable target', 0o755, 0o770],
  ]) {
    test(`refuses a link out of the base to ${name}`, () => {
      const dir = `link-${dirMode.toString(8)}-${fileMode.toString(8)}`;
      const run = source(dir, (base) => {
        const outside = join(tmp, dir, 'outside');
        mkdirSync(outside, { recursive: true });
        writeFileSync(join(outside, 'corepack'), '#!/bin/sh\nexit 0\n');
        chmodSync(join(outside, 'corepack'), fileMode);
        chmodSync(outside, dirMode);
        mkdirSync(join(base, 'corepack', 'node_modules', '.bin'), { recursive: true, mode: 0o700 });
        chmodSync(base, 0o700);
        symlinkSync(join(outside, 'corepack'), join(base, 'corepack', 'node_modules', '.bin', 'corepack'));
      });
      assert.notEqual(run.status, 0);
      assert.doesNotMatch(run.stdout, /sourced/);
      assert.match(run.stderr, /links out of it/);
    });
  }

  test('refuses a link out of the base behind a directory link', () => {
    const run = source('dir-link', (base) => {
      const outside = join(tmp, 'dir-link', 'outside');
      mkdirSync(outside, { recursive: true });
      writeFileSync(join(outside, 'corepack'), '#!/bin/sh\nexit 0\n');
      chmodSync(join(outside, 'corepack'), 0o700);
      mkdirSync(join(base, 'alternate', 'node_modules', '.bin'), { recursive: true, mode: 0o700 });
      chmodSync(base, 0o700);
      symlinkSync(join(outside, 'corepack'), join(base, 'alternate', 'node_modules', '.bin', 'corepack'));
      symlinkSync(join(base, 'alternate'), join(base, 'corepack'));
    });
    assert.notEqual(run.status, 0);
    assert.doesNotMatch(run.stdout, /sourced/);
    assert.match(run.stderr, /links out of it/);
  });

  test('stops without realpath instead of skipping the link check', () => {
    const run = source('no-realpath', (base, bin) => {
      rmSync(join(bin, 'realpath'));
    });
    assert.notEqual(run.status, 0);
    assert.match(run.stderr, /realpath is required/);
  });

  test('accepts links that stay inside the base', () => {
    const run = source('link-in', (base) => {
      const bin = join(base, 'corepack', 'node_modules', '.bin');
      mkdirSync(join(base, 'corepack', 'dist'), { recursive: true, mode: 0o700 });
      mkdirSync(bin, { recursive: true, mode: 0o700 });
      writeFileSync(join(base, 'corepack', 'dist', 'corepack.js'), '#!/bin/sh\nexit 0\n');
      chmodSync(join(base, 'corepack', 'dist', 'corepack.js'), 0o700);
      chmodSync(base, 0o700);
      symlinkSync('../../dist/corepack.js', join(bin, 'corepack'));
    });
    assert.equal(run.status, 0, run.stderr);
    assert.match(run.stdout, /sourced/);
  });

  test('refuses a base directory that its group can write to', () => {
    const run = source('group-base', (base) => {
      mkdirSync(base);
      chmodSync(base, 0o770);
    });
    assert.notEqual(run.status, 0);
    assert.match(run.stderr, /must not be writable by others/);
  });

  test('refuses a base directory that others can write to', () => {
    const run = source('open-base', (base) => {
      mkdirSync(base);
      chmodSync(base, 0o777);
    });
    assert.notEqual(run.status, 0);
    assert.doesNotMatch(run.stdout, /sourced/);
    assert.match(run.stderr, /must not be writable by others/);
  });
});
