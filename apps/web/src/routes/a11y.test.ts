/**
 * axe-core over the home page in every state (tiles with an offline app, the
 * pairing hint, empty for a user and for an administrator, skeleton, failure
 * card), desktop and mobile, light and dark. The colour contrast is left to
 * the browser audit (`scripts/a11y/scenarios/home.mjs`).
 */
import type { RenderResult } from '@testing-library/svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { HomeApp } from '#lib/apps.ts';
import { prefs } from '#lib/state/prefs.svelte.ts';
import { violations } from '@roedy/ui/testing';
import { renderSignedIn, resetSignedIn, setPhone, type ApiRoutes, type Role } from '#lib/test/signed-in.ts';
import Page from './+page.svelte';

vi.mock('$app/navigation', () => ({ goto: vi.fn(() => Promise.resolve()) }));

const APPS: HomeApp[] = [
  {
    slug: 'mail',
    name: 'Mail',
    description: 'Email for the team',
    url: '/mail/',
    icon: '/api/v1/apps/mail/icon.svg?v=0123456789abcdef',
    status: 'online',
  },
  {
    slug: 'notes',
    name: 'Notes',
    description: 'Shared notes',
    url: '/notes/',
    icon: '/api/v1/apps/notes/icon.svg?v=0123456789abcdef',
    status: 'offline',
  },
];

afterEach(() => {
  resetSignedIn();
  delete document.documentElement.dataset.mode;
});

type View = RenderResult<never>;

async function home(opts: { phone?: boolean; role?: Role; api: ApiRoutes; wait: (v: View) => Promise<unknown> }) {
  setPhone(opts.phone ?? false);
  const view = renderSignedIn(Page, { role: opts.role ?? 'admin', api: opts.api }) as unknown as View;
  await opts.wait(view);
  return view;
}

// Functions: `mockApi` uses its answers up.
const tiles = (): ApiRoutes => ({
  'GET apps': [{ body: { apps: APPS } }],
  'GET admin/apps': [{ body: { apps: [{ slug: 'wiki', state: 'pending', name: null }] } }],
});
const empty = (): ApiRoutes => ({ 'GET apps': [{ body: { apps: [] } }], 'GET admin/apps': [{ body: { apps: [] } }] });

const CASES: [string, () => Promise<View>][] = [
  ['tiles with pairing hint (desktop)', () => home({ api: tiles(), wait: (v) => v.findByRole('note') })],
  ['tiles with pairing hint (mobile)', () => home({ phone: true, api: tiles(), wait: (v) => v.findByRole('note') })],
  ['empty, user (desktop)', () => home({ role: 'user', api: empty(), wait: (v) => v.findByText('No apps yet') })],
  [
    'empty, administrator (mobile)',
    () => home({ phone: true, api: empty(), wait: (v) => v.findByRole('button', { name: 'Manage apps' }) }),
  ],
  [
    'skeleton (desktop)',
    () =>
      home({
        role: 'user',
        api: { 'GET apps': [() => new Promise(() => {})] },
        wait: (v) => v.findByRole('status'),
      }),
  ],
  [
    'failure card (mobile)',
    () =>
      home({
        phone: true,
        role: 'user',
        api: { 'GET apps': [{ status: 500, body: { error: 'internal' } }] },
        wait: (v) => v.findByRole('alert'),
      }),
  ],
];

describe('home page accessibility (axe)', () => {
  for (const mode of ['light', 'dark'] as const) {
    it.each(CASES)(`has no violations: %s (${mode})`, async (_name, show) => {
      document.documentElement.dataset.mode = mode;
      prefs.set({ mode });
      const view = await show();
      expect(await violations(view)).toEqual([]);
    });
  }
});
