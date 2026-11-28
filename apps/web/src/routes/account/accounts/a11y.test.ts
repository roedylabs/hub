/**
 * axe-core over Account › Accounts: the list, and the form with an error, desktop and mobile, in the light and the
 * dark theme. Colour contrast is left to the browser audit.
 */
import { fireEvent, within } from '@testing-library/svelte';
import { violations } from '@roedy/ui/testing';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { prefs } from '#lib/state/prefs.svelte.ts';
import { renderSignedIn, resetSignedIn, setPhone } from '#lib/test/signed-in.ts';
import { accountOf, serversOf } from '#lib/test/stalwart.ts';
import Page from './+page.svelte';
import NewPage from './new/+page.svelte';

vi.mock('$app/navigation', () => ({ goto: vi.fn(() => Promise.resolve()) }));
vi.mock('$app/state', async () => ({ page: (await import('#lib/test/page.svelte.ts')).page }));

afterEach(() => {
  delete document.documentElement.dataset.mode;
  resetSignedIn();
});

const api = () => ({
  'GET stalwart/accounts': [
    {
      body: {
        accounts: [
          accountOf(),
          accountOf({ id: '01JA2', email: 'b@example.org', default: false, state: 'needs_sign_in' }),
        ],
      },
    },
  ],
  'GET stalwart/servers': [{ body: serversOf(undefined, true) }],
});

describe('Accounts page accessibility', () => {
  for (const mobile of [false, true]) {
    for (const mode of ['light', 'dark'] as const) {
      it(`list (${mobile ? 'mobile' : 'desktop'}, ${mode})`, async () => {
        setPhone(mobile);
        document.documentElement.dataset.mode = mode;
        prefs.set({ mode });
        const view = renderSignedIn(Page, { role: 'user', api: api() });
        await view.findByRole('button', { name: /b@example\.org/ });
        expect(await violations(view)).toEqual([]);
      });

      it(`form with an error (${mobile ? 'mobile' : 'desktop'}, ${mode})`, async () => {
        setPhone(mobile);
        document.documentElement.dataset.mode = mode;
        prefs.set({ mode });
        const view = renderSignedIn(NewPage, { role: 'user', path: '/account/accounts/new', api: api() });
        const form = within(await view.findByRole('group', { name: 'New account' }));
        await fireEvent.click(form.getByRole('button', { name: 'Continue to sign-in' }));
        await form.findByRole('alert');
        expect(await violations(view)).toEqual([]);
      });
    }
  }
});
