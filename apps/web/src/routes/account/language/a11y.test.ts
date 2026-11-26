/**
 * axe-core over Account › Language (a language chosen, "System" with the
 * note on mails, German), desktop and mobile, in the light and the dark
 * theme. Colour contrast is left to the browser audit
 * (`scripts/a11y/scenarios/language.mjs`).
 */
import { afterEach, describe, expect, it, vi } from 'vitest';
import { prefs, type Settings } from '#lib/state/prefs.svelte.ts';
import { violations } from '#lib/test/axe.ts';
import { renderSignedIn, resetSignedIn, setPhone } from '#lib/test/signed-in.ts';
import Page from './+page.svelte';

vi.mock('$app/navigation', () => ({ goto: vi.fn(() => Promise.resolve()) }));

afterEach(() => {
  delete document.documentElement.dataset.mode;
  resetSignedIn();
});

const LANGS: Settings['lang'][] = ['en', 'system', 'de'];

describe('Language page accessibility', () => {
  for (const mobile of [false, true]) {
    for (const mode of ['light', 'dark'] as const) {
      for (const lang of LANGS) {
        it(`${lang} (${mobile ? 'mobile' : 'desktop'}, ${mode})`, async () => {
          setPhone(mobile);
          document.documentElement.dataset.mode = mode;
          prefs.set({ mode, lang });
          const view = renderSignedIn(Page, { role: 'user' });
          expect(await violations(view)).toEqual([]);
        });
      }
    }
  }
});
