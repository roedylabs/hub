/**
 * axe-core over the page of an app in every state (paired with roles and
 * who has access, switched off, pending with a new pairing code, offline
 * with failing events, not found, failure card), desktop and mobile, light and dark. The
 * colour contrast is left to the browser audit
 * (`scripts/a11y/scenarios/apps.mjs`).
 */
import { fireEvent, type RenderResult } from '@testing-library/svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { prefs } from '#lib/state/prefs.svelte.ts';
import { disabledApp, offlineApp, pairedApp, PAIRING, pendingApp } from '#lib/test/apps.ts';
import { violations } from '@roedy/ui/testing';
import { page } from '#lib/test/page.svelte.ts';
import { renderSignedIn, resetSignedIn, setPhone, type ApiRoutes } from '#lib/test/signed-in.ts';
import Page from './+page.svelte';

vi.mock('$app/navigation', () => ({ goto: vi.fn(() => Promise.resolve()) }));
vi.mock('$app/state', async () => ({ page: (await import('#lib/test/page.svelte.ts')).page }));

const USERS = {
  users: [
    { id: 'u1', status: 'active', name: 'Alex Example', email: 'alex@example.com', role: 'admin', app_role: 'admin' },
    { id: 'u3', status: 'active', name: null, email: 'robin@example.com', role: 'user', app_role: null },
  ],
  invites: [{ id: 'i1', status: 'invited', name: null, email: 'kim@example.com', role: 'user', app_role: 'user' }],
};

afterEach(() => {
  resetSignedIn();
  delete document.documentElement.dataset.mode;
});

type View = RenderResult<never>;

async function app(slug: string, api: ApiRoutes, then: (v: View) => Promise<unknown>, phone = false) {
  setPhone(phone);
  page.params = { slug };
  const view = renderSignedIn(Page, { path: `/admin/apps/${slug}`, api }) as unknown as View;
  await then(view);
  return view;
}

const paired = (): ApiRoutes => ({
  'GET admin/apps/mail': [{ body: { app: pairedApp() } }],
  'GET admin/apps/mail/users': [{ body: USERS }],
});
const off = (): ApiRoutes => ({
  'GET admin/apps/mail': [{ body: { app: disabledApp() } }],
  'GET admin/apps/mail/users': [{ body: USERS }],
});
const pending = (): ApiRoutes => ({
  'GET admin/apps/wiki': [{ body: { app: pendingApp('2099-01-01T00:00:00Z') } }],
  'POST admin/apps/wiki/pairing-code': [{ body: { app: pendingApp('2099-01-01T00:00:00Z'), pairing: PAIRING } }],
});
const withAccess = (v: View) => v.findByText('2 of 3 with access, including open invitations.');
const withCode = async (v: View) => {
  await fireEvent.click(await v.findByRole('button', { name: 'New pairing code' }));
  await v.findByRole('region', { name: 'Pairing code for wiki' });
};

const CASES: [string, () => Promise<View>][] = [
  ['paired (desktop)', () => app('mail', paired(), withAccess)],
  ['paired (mobile)', () => app('mail', paired(), withAccess, true)],
  ['switched off (desktop)', () => app('mail', off(), withAccess)],
  ['switched off (mobile)', () => app('mail', off(), withAccess, true)],
  ['pending with pairing code (desktop)', () => app('wiki', pending(), withCode)],
  ['pending with pairing code (mobile)', () => app('wiki', pending(), withCode, true)],
  [
    'offline with failing events (desktop)',
    () =>
      app(
        'notes',
        {
          'GET admin/apps/notes': [{ body: { app: offlineApp() } }],
          'GET admin/apps/notes/users': [{ status: 500, body: { error: 'internal' } }],
        },
        (v) => v.findByText('Couldn’t load who has access.'),
      ),
  ],
  [
    'not found (mobile)',
    () =>
      app(
        'nope',
        { 'GET admin/apps/nope': [{ status: 404, body: { error: 'not_found' } }] },
        (v) => v.findByText('It may have been removed in the meantime.'),
        true,
      ),
  ],
  [
    'failure card (desktop)',
    () =>
      app('mail', { 'GET admin/apps/mail': [{ status: 500, body: { error: 'internal' } }] }, (v) =>
        v.findByRole('alert'),
      ),
  ],
];

describe('Administration › Apps › an app accessibility (axe)', () => {
  for (const mode of ['light', 'dark'] as const) {
    it.each(CASES)(`has no violations: %s (${mode})`, async (_name, show) => {
      document.documentElement.dataset.mode = mode;
      prefs.set({ mode });
      const view = await show();
      expect(await violations(view)).toEqual([]);
    });
  }
});
