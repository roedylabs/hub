/**
 * axe-core in jsdom over the states of Administration › Branding (desktop and
 * mobile, light and dark): default, initial, icon grid, image with the file
 * card, a logo error and the bar "Unsaved changes" while saving. Colour
 * contrast is left to the browser audit (`scripts/a11y/scenarios/branding.mjs`).
 */
import { fireEvent, screen, within } from '@testing-library/svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { brandingOf, type AdminBranding } from '#lib/branding.svelte.ts';
import { prefs } from '#lib/state/prefs.svelte.ts';
import { BRAND0 } from '#lib/state/session.svelte.ts';
import { violations } from '#lib/test/axe.ts';
import { renderSignedIn, resetSignedIn, setPhone } from '#lib/test/signed-in.ts';
import Page from './+page.svelte';

vi.mock('$app/navigation', () => ({ goto: vi.fn(() => Promise.resolve()) }));

const ADMIN: AdminBranding = { ...BRAND0, imageName: '', imageMeta: '' };
const IMAGE: AdminBranding = {
  ...ADMIN,
  kind: 'image',
  image: '/branding/image/0123456789abcdef0123456789abcdef.png',
  imageName: 'example.png',
  imageMeta: 'PNG · 3 KB',
};

beforeEach(() => {
  vi.stubGlobal(
    'Image',
    class {
      naturalWidth = 512;
      naturalHeight = 512;
      onload: (() => void) | null = null;
      onerror: (() => void) | null = null;
      set src(_: string) {
        queueMicrotask(() => this.onload?.());
      }
    },
  );
});

afterEach(() => {
  resetSignedIn();
  delete document.documentElement.dataset.mode;
});

const radio = (group: string, name: string) =>
  within(screen.getByRole('radiogroup', { name: group })).getByRole('radio', { name });

async function show(branding: AdminBranding = ADMIN) {
  const view = renderSignedIn(Page, {
    role: 'admin',
    path: '/admin/branding',
    config: { branding: brandingOf(branding) },
    api: {
      'GET admin/branding': [{ body: branding }],
      // Never answers: the bar stays busy.
      'PUT admin/branding': [() => new Promise(() => {})],
    },
  });
  await vi.waitFor(() => expect(view.calls).toHaveLength(1));
  return view;
}

type Step = (view: Awaited<ReturnType<typeof show>>) => Promise<unknown>;

const CASES: [name: string, branding: AdminBranding, step: Step | null][] = [
  ['default', ADMIN, null],
  ['initial', ADMIN, () => fireEvent.click(radio('Type', 'Initials'))],
  ['icon grid', ADMIN, () => fireEvent.click(radio('Type', 'Icon'))],
  [
    'image drop zone with error',
    ADMIN,
    async (view) => {
      await fireEvent.click(radio('Type', 'Image'));
      const input = view.container.querySelector('input[type="file"]') as HTMLInputElement;
      await fireEvent.change(input, { target: { files: [new File(['x'], 'notes.txt', { type: 'text/plain' })] } });
      await view.findByRole('alert');
    },
  ],
  ['saved image', IMAGE, null],
  [
    'saving',
    ADMIN,
    async () => {
      await fireEvent.click(radio('Brand color', 'Sage'));
      await fireEvent.click(
        within(screen.getByRole('region', { name: 'Unsaved changes' })).getByRole('button', { name: 'Save' }),
      );
    },
  ],
];

describe('Branding page accessibility', () => {
  for (const phone of [false, true]) {
    for (const mode of ['light', 'dark'] as const) {
      for (const [name, branding, step] of CASES) {
        it(`${name} (${phone ? 'mobile' : 'desktop'}, ${mode})`, async () => {
          setPhone(phone);
          document.documentElement.dataset.mode = mode;
          prefs.set({ mode });
          const view = await show(branding);
          if (step) await step(view);
          expect(await violations(view)).toEqual([]);
        });
      }
    }
  }
});
