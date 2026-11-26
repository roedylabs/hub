/**
 * axe-core over Account › Appearance in every state (defaults, custom accent,
 * accent locked by the branding, colour editor open), desktop and mobile, in
 * the light and the dark theme. Colour contrast is left to the browser audit
 * (`scripts/a11y/scenarios/appearance.mjs`).
 */
import { fireEvent, screen, within } from '@testing-library/svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { prefs, type Settings } from '#lib/state/prefs.svelte.ts';
import { BRAND0, type PublicConfig } from '#lib/state/session.svelte.ts';
import { violations } from '#lib/test/axe.ts';
import { renderSignedIn, resetSignedIn, setPhone } from '#lib/test/signed-in.ts';
import DialogHost from './DialogHost.svelte';

vi.mock('$app/navigation', () => ({ goto: vi.fn(() => Promise.resolve()) }));

afterEach(() => {
  delete document.documentElement.dataset.mode;
  resetSignedIn();
});

interface State {
  name: string;
  values?: Partial<Settings>;
  config?: Partial<PublicConfig>;
  editor?: boolean;
}

const STATES: State[] = [
  { name: 'defaults' },
  { name: 'custom accent', values: { accent: '#3a7bd5' } },
  { name: 'accent locked', config: { branding: { ...BRAND0, userAccent: false, color: 'sage' } } },
  { name: 'colour editor open', editor: true },
];

describe('Appearance page accessibility', () => {
  for (const mobile of [false, true]) {
    for (const mode of ['light', 'dark'] as const) {
      for (const state of STATES) {
        it(`${state.name} (${mobile ? 'mobile' : 'desktop'}, ${mode})`, async () => {
          setPhone(mobile);
          document.documentElement.dataset.mode = mode;
          prefs.set({ mode, ...state.values });
          const view = renderSignedIn(DialogHost, { role: 'user', config: state.config ?? {} });
          if (state.editor) {
            const accents = within(screen.getByRole('radiogroup', { name: 'Accent color' }));
            await fireEvent.click(accents.getByRole('radio', { name: 'More' }));
            await view.findByRole('dialog', { name: 'More colors' });
          }
          expect(await violations(document.body)).toEqual([]);
        });
      }
    }
  }
});
