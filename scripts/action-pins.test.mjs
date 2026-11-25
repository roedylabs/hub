// Actions pinned by commit (node --test scripts/): every `uses:` in the
// workflows and in composite actions names a full 40-character commit SHA, so
// that a moved or replaced tag cannot change what runs in CI and in the
// release. Local actions (`./…`) are part of the repository; Docker actions
// (`docker://…`) need a digest. Stands in for the repository setting that
// enforces SHA pinning; Dependabot (github-actions) updates SHA and comment.

import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { describe, test } from 'node:test';

const root = join(import.meta.dirname, '..');

/** Tracked and new (not ignored) files of the repository. */
function repoFiles() {
  return execFileSync('git', ['ls-files', '-z', '--cached', '--others', '--exclude-standard'], {
    cwd: root,
    encoding: 'utf8',
  })
    .split('\0')
    .filter(Boolean);
}

/** Workflows and composite actions (action.yml anywhere). */
function isWorkflowOrAction(path) {
  return /^\.github\/workflows\/[^/]+\.ya?ml$/.test(path) || /(^|\/)action\.ya?ml$/.test(path);
}

/** The key `uses`, plain or quoted. */
const KEY = String.raw`(?:uses|"uses"|'uses')\s*:`;
/** `uses: <ref>` as a key, also as the first key of a list item; the value may be quoted and followed by a comment. */
const USES = new RegExp(String.raw`^\s*(?:-\s+)?${KEY}\s*(['"]?)([^\s'"#]*)\1\s*(?:#.*)?$`);
/** A `uses` key at the start of the line or inside a flow mapping. */
const USES_KEY = new RegExp(String.raw`^\s*(?:-\s+)?${KEY}|[{,]\s*${KEY}`);
/** A key whose value is a block scalar (`run: |`, `script: >-`); the lines below it are text. */
const BLOCK_SCALAR = /^(\s*(?:-\s+)*)[^\s#][^:#]*:\s*[|>][-+0-9]*\s*(?:#.*)?$/;

/**
 * Problems of a workflow or action file: references without a commit SHA,
 * and `uses:` lines the check cannot read (e.g. flow mappings). Text inside
 * block scalars (shell scripts of `run: |`) is not YAML and is skipped.
 * @param {string} text
 * @returns {string[]}
 */
function usesProblems(text) {
  const problems = [];
  /** Column of the key that opened the current block scalar, or -1. */
  let block = -1;
  for (const [i, line] of text.split(/\r?\n/).entries()) {
    if (block >= 0) {
      if (line.trim() === '' || line.search(/\S/) > block) continue;
      block = -1;
    }
    const scalar = BLOCK_SCALAR.exec(line);
    if (scalar) {
      block = scalar[1].length;
      continue;
    }
    if (!USES_KEY.test(line) || /^\s*#/.test(line)) continue;
    const m = USES.exec(line);
    if (!m) {
      problems.push(`line ${i + 1}: unreadable uses: ${line.trim()}`);
      continue;
    }
    const ref = m[2];
    if (ref.startsWith('./')) continue;
    if (ref.startsWith('docker://')) {
      if (!/^docker:\/\/[^\s@]+@sha256:[0-9a-f]{64}$/.test(ref)) problems.push(`line ${i + 1}: ${ref} without digest`);
    } else if (!/^[\w.-]+\/[\w./-]+@[0-9a-f]{40}$/.test(ref)) {
      problems.push(`line ${i + 1}: ${ref} not pinned to a commit SHA`);
    }
  }
  return problems;
}

describe('usesProblems', () => {
  const sha = 'a'.repeat(40);

  test('accepts commit SHAs, local actions and Docker digests', () => {
    const text = [
      'jobs:',
      '  call:',
      `    uses: owner/repo/.github/workflows/reusable.yml@${sha}`,
      '  build:',
      '    steps:',
      `      - uses: actions/checkout@${sha} # v7.0.1`,
      `      - uses: "github/codeql-action/init@${sha}" # v4`,
      `      - uses: 'owner/repo@${sha}'`,
      '      - uses: ./.github/actions/setup',
      `      - uses: docker://alpine:3.24@sha256:${'b'.repeat(64)}`,
      `      - "uses": actions/checkout@${sha}`,
      `      - 'uses': actions/checkout@${sha}`,
      '      # - uses: owner/repo@v1 (commented out)',
      '      - name: Script that mentions a flow mapping',
      '        run: |',
      "          echo '{ uses: example }'",
      '          echo "uses: owner/repo@v1"',
      '',
      '          echo done',
      '      - name: Text that mentions uses: in a string',
      '        run: echo "no key"',
    ].join('\n');
    assert.deepEqual(usesProblems(text), []);
  });

  test('reports tags, branches, short SHAs, Docker tags and unreadable lines', () => {
    const text = [
      '      - uses: actions/checkout@v7',
      '      - uses: actions/checkout@main',
      `      - uses: actions/checkout@${sha.slice(0, 7)}`,
      `      - uses: actions/checkout@${sha.toUpperCase()}`,
      '      - uses: actions/checkout',
      '      - uses: docker://alpine:3.24',
      `      - { name: x, uses: actions/checkout@${sha} }`,
      '      - "uses": actions/checkout@v7',
      "      - 'uses': actions/checkout@v7",
      '      - run: >-',
      '          echo text',
      '      - uses: actions/checkout@v7',
    ].join('\n');
    assert.deepEqual(usesProblems(text), [
      'line 1: actions/checkout@v7 not pinned to a commit SHA',
      'line 2: actions/checkout@main not pinned to a commit SHA',
      `line 3: actions/checkout@${sha.slice(0, 7)} not pinned to a commit SHA`,
      `line 4: actions/checkout@${sha.toUpperCase()} not pinned to a commit SHA`,
      'line 5: actions/checkout not pinned to a commit SHA',
      'line 6: docker://alpine:3.24 without digest',
      `line 7: unreadable uses: - { name: x, uses: actions/checkout@${sha} }`,
      'line 8: actions/checkout@v7 not pinned to a commit SHA',
      'line 9: actions/checkout@v7 not pinned to a commit SHA',
      'line 12: actions/checkout@v7 not pinned to a commit SHA',
    ]);
  });
});

describe('repository', () => {
  test('every uses: in workflows and composite actions is pinned to a commit SHA', () => {
    const files = repoFiles().filter(isWorkflowOrAction);
    assert.ok(files.includes('.github/workflows/ci.yml'), 'ci.yml not found');
    const problems = files.flatMap((f) => usesProblems(readFileSync(join(root, f), 'utf8')).map((p) => `${f} ${p}`));
    assert.deepEqual(problems, []);
  });
});
