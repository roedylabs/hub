// The plain packages in Node without a bundler: ES modules with their exports.
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { CORE, JmapClient, capabilityOf } from '@roedy/jmap';
import { DEFAULT_ACCENT, MODES, renderTokensCss } from '@roedy/tokens';

assert.equal(CORE, 'urn:ietf:params:jmap:core');
assert.equal(typeof JmapClient, 'function');
assert.equal(capabilityOf('Core/echo'), CORE);
assert.equal(typeof DEFAULT_ACCENT, 'string');
assert.ok('dark' in MODES && 'light' in MODES);
// The published style sheet is the one the constants render.
const css = readFileSync(createRequire(import.meta.url).resolve('@roedy/tokens/tokens.css'), 'utf8');
assert.equal(css, renderTokensCss());
console.log('@roedy/jmap and @roedy/tokens load in Node');
