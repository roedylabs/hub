/**
 * axe-core in jsdom for the accessibility tests (`src/a11y.test.ts` and the
 * page tests). Not a test file.
 */
import type { RenderResult } from '@testing-library/svelte';
import axe from 'axe-core';

/**
 * Violations of axe in `view` (or an element), without colour contrast (jsdom has no layout; the browser audit
 * `scripts/a11y/audit.mjs` checks it) and without `region` (a page or component alone lies outside landmarks).
 */
export async function violations(view: Pick<RenderResult<never>, 'container'> | Element): Promise<string[]> {
  const root = view instanceof Element ? view : view.container;
  const result = await axe.run(root, {
    runOnly: { type: 'tag', values: ['wcag2a', 'wcag2aa', 'wcag21a', 'wcag21aa', 'wcag22aa', 'best-practice'] },
    rules: { 'color-contrast': { enabled: false }, region: { enabled: false } },
  });
  return result.violations.map((v) => `${v.id}: ${v.nodes.map((n) => n.target.join(' ')).join(', ')}`);
}
