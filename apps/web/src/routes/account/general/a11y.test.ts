/**
 * axe-core over Account › General (defaults, everything switched, German),
 * desktop and mobile, in the light and the dark theme. Colour contrast is
 * left to the browser audit.
 */
import { afterEach, describe, expect, it, vi } from 'vitest';
import { violations } from '@roedy/ui/testing';
import { prefs, type Settings } from '#lib/state/prefs.svelte.ts';
import { renderSignedIn, resetSignedIn, setPhone } from '#lib/test/signed-in.ts';
import Page from './+page.svelte';

vi.mock('$app/navigation', () => ({ goto: vi.fn(() => Promise.resolve()) }));

afterEach(() => {
  delete document.documentElement.dataset.mode;
  resetSignedIn();
});

const STATES: { name: string; values: Partial<Settings> }[] = [
  { name: 'defaults', values: {} },
  {
    name: 'everything switched',
    values: { confirm_delete: false, undo_delay: 10, shortcuts: false, developer: true },
  },
  { name: 'German', values: { lang: 'de' } },
];

describe('General page accessibility', () => {
  for (const mobile of [false, true]) {
    for (const mode of ['light', 'dark'] as const) {
      for (const state of STATES) {
        it(`${state.name} (${mobile ? 'mobile' : 'desktop'}, ${mode})`, async () => {
          setPhone(mobile);
          document.documentElement.dataset.mode = mode;
          prefs.set({ mode, ...state.values });
          const view = renderSignedIn(Page, { role: 'user' });
          expect(await violations(view)).toEqual([]);
        });
      }
    }
  }
});
