/**
 * axe-core over Administration › Users in every state (list, invitation form
 * with marked addresses, result notice, selection with the role list,
 * expanded user with details, expired invitation, no system mail, empty
 * search, skeleton, failure card, app access in the row, the invitation form
 * and the bulk action), desktop and mobile, light and dark. The
 * colour contrast is left to the browser audit (`scripts/a11y/scenarios/users.mjs`).
 */
import { fireEvent, type RenderResult } from '@testing-library/svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { prefs } from '#lib/state/prefs.svelte.ts';
import { offlineApp, pairedApp, pendingApp } from '#lib/test/apps.ts';
import { violations } from '#lib/test/axe.ts';
import { renderSignedIn, resetSignedIn, setPhone, TEST_USER, type ApiRoutes } from '#lib/test/signed-in.ts';
import type { ListedUser } from '#lib/users.svelte.ts';
import Page from './+page.svelte';

vi.mock('$app/navigation', () => ({ goto: vi.fn(() => Promise.resolve()) }));

const NOW = Date.now();
const before = (hours: number) => new Date(NOW - hours * 3600_000).toISOString();
const base = { mfa: false, created_at: before(500), last_seen_at: before(2) };
const USERS: ListedUser[] = [
  { ...base, id: TEST_USER.id, status: 'active', name: TEST_USER.name, email: TEST_USER.email, role: 'admin' },
  { ...base, id: 'u1', status: 'active', name: 'Alice Brown', email: 'alice@example.com', role: 'admin', mfa: true },
  { ...base, id: 'u3', status: 'locked', name: 'Sophie Taylor', email: 'sophie@example.com', role: 'user' },
  {
    id: 'i2',
    status: 'invited',
    name: null,
    email: 'paul@example.com',
    role: 'user',
    created_at: before(1),
    mail_sent_at: null,
    expires_at: before(-100),
    expired: false,
  },
  {
    id: 'i3',
    status: 'invited',
    name: 'Eve Moore',
    email: 'eve@example.com',
    role: 'user',
    created_at: before(300),
    mail_sent_at: before(200),
    expires_at: before(30),
    expired: true,
    requested_at: before(4),
  },
];
const DETAILS = {
  body: {
    user: {
      ...USERS[1],
      sessions: [{ created_at: before(30), last_seen_at: before(2) }],
      recovery_codes: { unused: 9, total: 10 },
    },
  },
};

afterEach(() => {
  resetSignedIn();
  delete document.documentElement.dataset.mode;
});

type View = RenderResult<never>;

async function users(opts: { phone?: boolean; mail?: string; routes?: ApiRoutes; then?: (v: View) => unknown } = {}) {
  setPhone(opts.phone ?? false);
  const view = renderSignedIn(Page, {
    path: '/admin/users',
    api: {
      'GET users': Array.from({ length: 5 }, () => ({ body: { users: USERS } })),
      'GET admin/system-mail': [{ body: { status: opts.mail ?? 'ok' } }],
      'GET users/u1': [DETAILS],
      ...opts.routes,
    },
  }) as unknown as View;
  await view.findByText('Alice Brown');
  await opts.then?.(view);
  return view;
}

const expand = (name: RegExp) => async (view: View) => {
  await fireEvent.click(view.getByRole('button', { name, expanded: false }));
};

type Case = [name: string, show: () => Promise<View>];

const APPS = () => ({
  'GET admin/apps': [{ body: { apps: [pairedApp(), pendingApp(), offlineApp()] } }],
});
const accessCase = (phone: boolean) => () =>
  users({
    phone,
    routes: APPS(),
    then: async (view) => {
      await expand(/Alice Brown/)(view);
      await view.findByRole('radiogroup', { name: 'Access to Mail' });
    },
  });
const bulkCase = (phone: boolean) => () =>
  users({
    phone,
    routes: APPS(),
    then: async (view) => {
      await fireEvent.click(view.getByRole('checkbox', { name: 'Select Sophie Taylor' }));
      await fireEvent.click(await view.findByRole('button', { name: 'App access' }));
      await view.findByRole('group', { name: 'App access for 1 person' });
    },
  });

const CASES: Case[] = [
  ['expanded user with app access (desktop)', accessCase(false)],
  ['expanded user with app access (mobile)', accessCase(true)],
  ['bulk app access (desktop)', bulkCase(false)],
  ['bulk app access (mobile)', bulkCase(true)],
  [
    'invitation form with app access error (desktop)',
    () =>
      users({
        routes: {
          ...APPS(),
          'POST invites': [{ status: 422, body: { error: 'invalid', fields: { 'apps.mail': { code: 'invalid' } } } }],
        },
        then: async (view) => {
          await fireEvent.click(view.getByRole('button', { name: 'Invite' }));
          await view.findByRole('radiogroup', { name: 'Access to Mail' });
          await fireEvent.input(view.getByLabelText('Email addresses'), { target: { value: 'tim@example.com' } });
          await fireEvent.click(view.getByRole('button', { name: 'Send invitation' }));
          await view.findByText('This role no longer exists in the app. Choose another one.');
        },
      }),
  ],
  ['list (desktop)', () => users()],
  ['list (mobile)', () => users({ phone: true })],
  [
    'expanded user with details (desktop)',
    () =>
      users({
        then: async (view) => {
          await expand(/Alice Brown/)(view);
          await view.findByText('9 of 10 recovery codes left');
        },
      }),
  ],
  [
    'expanded user with details (mobile)',
    () =>
      users({
        phone: true,
        then: async (view) => {
          await expand(/Alice Brown/)(view);
          await view.findByText('9 of 10 recovery codes left');
        },
      }),
  ],
  [
    'details failed (desktop)',
    () =>
      users({
        routes: { 'GET users/u1': [{ status: 500, body: { error: 'internal' } }] },
        then: async (view) => {
          await expand(/Alice Brown/)(view);
          await view.findByText('Couldn’t load devices and recovery codes.');
        },
      }),
  ],
  ['expired, requested invitation (desktop)', () => users({ then: expand(/Eve Moore/) })],
  ['unsent invitation (mobile)', () => users({ phone: true, then: expand(/paul@example\.com/) })],
  [
    'own account (desktop)',
    () =>
      users({
        routes: {
          [`GET users/${TEST_USER.id}`]: [{ body: { user: { ...USERS[0], sessions: [], recovery_codes: null } } }],
        },
        then: expand(/Alex Example/),
      }),
  ],
  [
    'invitation form with marked addresses (desktop)',
    () =>
      users({
        then: async (view) => {
          await fireEvent.click(view.getByRole('button', { name: 'Invite' }));
          await fireEvent.input(view.getByLabelText('Email addresses'), {
            target: { value: 'alice@example.com new@example.com broken@ ' },
          });
        },
      }),
  ],
  [
    'invitation form (mobile)',
    () =>
      users({
        phone: true,
        then: async (view) => {
          await fireEvent.click(view.getByRole('button', { name: 'Invite' }));
          await fireEvent.input(view.getByLabelText('Email addresses'), { target: { value: 'broken@ ' } });
        },
      }),
  ],
  [
    'result notice after inviting (desktop)',
    () =>
      users({
        routes: {
          'POST invites': [
            {
              status: 201,
              body: { invites: [{ id: 'i2', email: 'paul@example.com', sent: false }], system_mail: true },
            },
          ],
        },
        then: async (view) => {
          await fireEvent.click(view.getByRole('button', { name: 'Invite' }));
          await fireEvent.input(view.getByLabelText('Email addresses'), { target: { value: 'tim@example.com' } });
          await fireEvent.click(view.getByRole('button', { name: 'Send invitation' }));
          await view.findByText('Invitation couldn’t be sent');
        },
      }),
  ],
  [
    'selection with the role list open (desktop)',
    () =>
      users({
        then: async (view) => {
          await fireEvent.click(view.getByRole('checkbox', { name: 'Select Sophie Taylor' }));
          await fireEvent.keyDown(view.getByRole('combobox', { name: 'Change role' }), { key: 'ArrowDown' });
        },
      }),
  ],
  [
    'selection (mobile)',
    () =>
      users({
        phone: true,
        then: (view) => fireEvent.click(view.getByRole('checkbox', { name: 'Select Sophie Taylor' })),
      }),
  ],
  ['without system mail (desktop)', () => users({ mail: 'none', then: (view) => view.findByRole('note') })],
  ['without system mail (mobile)', () => users({ phone: true, mail: 'none', then: (view) => view.findByRole('note') })],
  [
    'empty search (desktop)',
    () =>
      users({
        then: (view) => fireEvent.input(view.getByLabelText('Search users'), { target: { value: 'xyz' } }),
      }),
  ],
  [
    'skeleton (desktop)',
    async () => {
      const view = renderSignedIn(Page, {
        api: { 'GET users': [() => new Promise(() => {})], 'GET admin/system-mail': [{ body: { status: 'ok' } }] },
      }) as unknown as View;
      await view.findByText('Loading data …');
      return view;
    },
  ],
  [
    'failure card (mobile)',
    async () => {
      setPhone(true);
      const view = renderSignedIn(Page, {
        api: {
          'GET users': [{ status: 500, body: { error: 'internal' } }],
          'GET admin/system-mail': [{ body: { status: 'ok' } }],
        },
      }) as unknown as View;
      await view.findByRole('alert');
      return view;
    },
  ],
];

describe('Administration › Users accessibility (axe)', () => {
  for (const mode of ['light', 'dark'] as const) {
    it.each(CASES)(`has no violations: %s (${mode})`, async (_name, show) => {
      document.documentElement.dataset.mode = mode;
      prefs.set({ mode });
      const view = await show();
      expect(await violations(view)).toEqual([]);
    });
  }
});
