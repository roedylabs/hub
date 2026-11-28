/**
 * axe-core over "No account yet", desktop and mobile, in the light and the dark theme. Colour contrast is left to
 * the browser audit.
 */
import { violations } from '@roedy/ui/testing';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { prefs } from '#lib/state/prefs.svelte.ts';
import { renderSignedIn, resetSignedIn, setPhone } from '#lib/test/signed-in.ts';
import Page from './+page.svelte';

vi.mock('$app/navigation', () => ({ goto: vi.fn(() => Promise.resolve()) }));
vi.mock('$app/state', async () => ({ page: (await import('#lib/test/page.svelte.ts')).page }));

afterEach(() => {
  delete document.documentElement.dataset.mode;
  resetSignedIn();
});

describe('No account yet accessibility', () => {
  for (const mobile of [false, true]) {
    for (const mode of ['light', 'dark'] as const) {
      it(`${mobile ? 'mobile' : 'desktop'}, ${mode}`, async () => {
        setPhone(mobile);
        document.documentElement.dataset.mode = mode;
        prefs.set({ mode });
        const view = renderSignedIn(Page, {
          role: 'user',
          path: '/account/no-account?next=%2Fmail%2F',
          api: { 'GET stalwart/accounts': [{ body: { accounts: [] } }], 'GET apps': [{ body: { apps: [] } }] },
        });
        await view.findByRole('heading', { level: 1 });
        expect(await violations(view)).toEqual([]);
      });
    }
  }
});
