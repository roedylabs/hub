/**
 * axe-core over Administration › Organization in every state (loaded, unsaved
 * changes, field errors, saving), desktop and mobile, light and dark. Colour
 * contrast is left to the browser audit (scripts/a11y/audit.mjs).
 */
import { fireEvent, screen } from '@testing-library/svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { prefs } from '#lib/state/prefs.svelte.ts';
import { violations } from '#lib/test/axe.ts';
import { renderSignedIn, resetSignedIn, setPhone, TEST_CONFIG } from '#lib/test/signed-in.ts';
import Page from './+page.svelte';

vi.mock('$app/navigation', () => ({ goto: vi.fn(() => Promise.resolve()) }));

afterEach(() => {
  resetSignedIn();
  delete document.documentElement.dataset.mode;
});

const field = (label: string) => screen.getByRole('textbox', { name: label });

type State = 'loaded' | 'dirty' | 'errors' | 'saving';

async function show(state: State) {
  const view = renderSignedIn(Page, {
    path: '/admin/organization',
    api: {
      'GET admin/org': [{ body: { ...TEST_CONFIG.org, help: 'https://example.com/help' } }],
      'PUT admin/org': [() => new Promise(() => {})],
    },
  });
  await vi.waitFor(() => expect(field('Help center')).toHaveValue('https://example.com/help'));
  if (state === 'dirty' || state === 'saving')
    await fireEvent.input(field('Name'), { target: { value: 'Example Inc' } });
  if (state === 'errors') {
    await fireEvent.input(field('Website'), { target: { value: 'example.com' } });
    await fireEvent.input(field('Support email'), { target: { value: 'support' } });
  }
  if (state === 'saving') {
    await fireEvent.click(screen.getByRole('button', { name: 'Save' }));
    await vi.waitFor(() => expect(view.container.querySelector('button[aria-busy="true"]')).not.toBeNull());
  }
  return view;
}

describe('Organization page – axe', () => {
  for (const mobile of [false, true])
    for (const mode of ['light', 'dark'] as const)
      for (const state of ['loaded', 'dirty', 'errors', 'saving'] as const)
        it(`${state}, ${mobile ? 'mobile' : 'desktop'}, ${mode}`, async () => {
          document.documentElement.dataset.mode = mode;
          prefs.set({ mode });
          setPhone(mobile);
          const view = await show(state);
          expect(await violations(view)).toEqual([]);
        });
});
