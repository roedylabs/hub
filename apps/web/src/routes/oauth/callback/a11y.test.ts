/**
 * axe-core over the return from the sign-in page, while it finishes and with an error, desktop and mobile, in the
 * light and the dark theme. Colour contrast is left to the browser audit.
 */
import { violations } from '@roedy/ui/testing';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { prefs } from '#lib/state/prefs.svelte.ts';
import { renderSignedIn, resetSignedIn, setPhone } from '#lib/test/signed-in.ts';
import Page from './+page.svelte';

vi.mock('$app/navigation', () => ({
  goto: vi.fn(() => Promise.resolve()),
  replaceState: vi.fn(),
  afterNavigate: vi.fn(),
}));

afterEach(() => {
  delete document.documentElement.dataset.mode;
  history.replaceState(null, '', '/');
  resetSignedIn();
});

describe('Sign-in return accessibility', () => {
  for (const mobile of [false, true]) {
    for (const mode of ['light', 'dark'] as const) {
      for (const phase of ['busy', 'error'] as const) {
        it(`${phase} (${mobile ? 'mobile' : 'desktop'}, ${mode})`, async () => {
          setPhone(mobile);
          document.documentElement.dataset.mode = mode;
          prefs.set({ mode });
          history.replaceState(null, '', '/oauth/callback?state=s&code=c');
          const view = renderSignedIn(Page, {
            role: 'user',
            api: {
              'POST stalwart/sign-in/complete': [
                phase === 'busy'
                  ? () => new Promise(() => {})
                  : { status: 410, body: { error: 'sign_in_expired', return_to: '/account/accounts' } },
              ],
            },
          });
          if (phase === 'error') await view.findByRole('heading', { level: 1, name: 'Sign-in expired' });
          expect(await violations(view)).toEqual([]);
        });
      }
    }
  }
});
