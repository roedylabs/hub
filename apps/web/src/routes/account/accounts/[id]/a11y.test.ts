/**
 * axe-core over one Stalwart account: connected, and needing a new access token with the form open, desktop and
 * mobile, in the light and the dark theme. Colour contrast is left to the browser audit.
 */
import { fireEvent } from '@testing-library/svelte';
import { violations } from '@roedy/ui/testing';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { prefs } from '#lib/state/prefs.svelte.ts';
import { page } from '#lib/test/page.svelte.ts';
import { renderSignedIn, resetSignedIn, setPhone } from '#lib/test/signed-in.ts';
import { accountOf, serversOf } from '#lib/test/stalwart.ts';
import Page from './+page.svelte';

vi.mock('$app/navigation', () => ({ goto: vi.fn(() => Promise.resolve()) }));
vi.mock('$app/state', async () => ({ page: (await import('#lib/test/page.svelte.ts')).page }));

afterEach(() => {
  delete document.documentElement.dataset.mode;
  resetSignedIn();
});

describe('Account page accessibility', () => {
  for (const mobile of [false, true]) {
    for (const mode of ['light', 'dark'] as const) {
      for (const needs of [false, true]) {
        it(`${needs ? 'needs a token' : 'connected'} (${mobile ? 'mobile' : 'desktop'}, ${mode})`, async () => {
          setPhone(mobile);
          document.documentElement.dataset.mode = mode;
          prefs.set({ mode });
          const account = needs ? accountOf({ auth: 'token', state: 'needs_sign_in', quota: null }) : accountOf();
          page.params = { id: account.id };
          const view = renderSignedIn(Page, {
            role: 'user',
            api: {
              'GET stalwart/accounts': [{ body: { accounts: [account] } }],
              'GET stalwart/servers': [{ body: serversOf() }],
            },
          });
          await view.findByRole('heading', { level: 1, name: account.email });
          if (needs) await fireEvent.click(view.getByRole('button', { name: 'Sign in again' }));
          expect(await violations(view)).toEqual([]);
        });
      }
    }
  }
});
