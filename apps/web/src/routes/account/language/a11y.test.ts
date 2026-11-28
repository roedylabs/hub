/**
 * axe-core over Account › Language & region (a language chosen, "System"
 * with the note on mails, German; a country with own formats, a chosen time
 * zone and an open pick list), desktop and mobile, in the light and the dark
 * theme. Colour contrast is left to the browser audit
 * (`scripts/a11y/scenarios/language.mjs`).
 */
import { fireEvent } from '@testing-library/svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { prefs, type Settings } from '#lib/state/prefs.svelte.ts';
import { violations } from '@roedy/ui/testing';
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
      it(`a country, own formats, a chosen zone and an open list (${mobile ? 'mobile' : 'desktop'}, ${mode})`, async () => {
        setPhone(mobile);
        document.documentElement.dataset.mode = mode;
        prefs.set({ mode, region: 'CH' });
        prefs.set({ date_format: 'iso', time_zone_auto: false, time_zone: 'Asia/Tokyo' });
        const view = renderSignedIn(Page, { role: 'user' });
        await fireEvent.click(view.getByRole(mobile ? 'button' : 'combobox', { name: /^Number format / }));
        expect(await violations(view)).toEqual([]);
      });
    }
  }
});
