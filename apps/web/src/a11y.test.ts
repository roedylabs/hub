/**
 * axe-core over the pages before and around the sign-in, each in the light
 * and the dark theme (`data-mode` on `<html>`). jsdom draws nothing, so the
 * colour contrast is left to the browser audit (scripts/a11y/audit.mjs);
 * everything else axe checks (names, roles, labels, ARIA, landmarks inside
 * the page) runs here on every `pnpm test`.
 */
import { fireEvent, render, type RenderResult } from '@testing-library/svelte';
import axe from 'axe-core';
import { afterEach, describe, expect, it, vi } from 'vitest';
import ErrorView from '#lib/components/ErrorView.svelte';
import LoginPage from '#lib/components/LoginPage.svelte';
import type { LoginRoute } from '#lib/login.svelte.ts';
import { mockApi } from '#lib/state/fetch-mock.ts';
import { prefs } from '#lib/state/prefs.svelte.ts';
import { BRAND0, session, type PublicConfig } from '#lib/state/session.svelte.ts';
import { ui } from '#lib/state/ui.svelte.ts';
import { page } from '#lib/test/page.svelte.ts';
import { sessionOf } from '#lib/test/session.ts';
import ConfirmEmail from './routes/account/confirm-email/[token]/+page.svelte';
import Invite from './routes/invite/[token]/+page.svelte';
import Home from './routes/+page.svelte';
import Setup from './routes/setup/+page.svelte';

const nav = vi.hoisted(() => ({ goto: vi.fn(() => Promise.resolve()) }));
vi.mock('$app/navigation', () => ({ goto: nav.goto }));
// The stand-in is imported lazily: the pages above load `$app/state` before it.
vi.mock('$app/state', async () => ({ page: (await import('#lib/test/page.svelte.ts')).page }));

const CONFIG: PublicConfig = {
  name: 'Example Org',
  branding: BRAND0,
  org: {
    name: 'Example Org',
    website: null,
    help: null,
    email: 'support@example.com',
    imprint: 'https://example.com/imprint',
    privacy: 'https://example.com/privacy',
    terms: null,
    source: null,
  },
  setup_required: false,
};
const INVITE = {
  email: 'jordan@example.com',
  name: 'Jordan Hale',
  role: 'user',
  inviter: 'Alex Example',
  expires_at: '2026-12-03T10:00:00Z',
  expired: false,
};
const SECRET = 'JBSWY3DPEHPK3PXPJBSWY3DPEHPK3PXP';

afterEach(() => {
  vi.unstubAllGlobals();
  nav.goto.mockClear();
  session.config = null;
  session.user = null;
  session.status = 'idle';
  prefs.reset();
  ui.reset();
  sessionStorage.clear();
  delete document.documentElement.dataset.mode;
});

/** Violations of axe in `view`, without colour contrast (jsdom has no layout). */
async function violations(view: RenderResult<never>): Promise<string[]> {
  const result = await axe.run(view.container, {
    runOnly: { type: 'tag', values: ['wcag2a', 'wcag2aa', 'wcag21a', 'wcag21aa', 'wcag22aa', 'best-practice'] },
    rules: { 'color-contrast': { enabled: false }, region: { enabled: false } },
  });
  return result.violations.map((v) => `${v.id}: ${v.nodes.map((n) => n.target.join(' ')).join(', ')}`);
}

function ready(config: Partial<PublicConfig> = {}) {
  session.config = { ...CONFIG, ...config };
  session.status = 'ready';
}

type Case = [name: string, show: () => Promise<RenderResult<never>>];

const login = (route: LoginRoute, url = 'http://localhost/login') => {
  page.url = new URL(url);
  ready();
  return render(LoginPage, { route, navigate: () => {}, home: () => {} }) as RenderResult<never>;
};

async function invite(step: 1 | 2 | 3 | 'expired') {
  mockApi({
    'GET invites/tok': [{ body: { ...INVITE, expired: step === 'expired' } }],
    'POST invites/tok/totp': [{ body: { secret: SECRET, uri: `otpauth://totp/x?secret=${SECRET}` } }],
  });
  page.params = { token: 'tok' };
  ready();
  const view = render(Invite) as RenderResult<never>;
  await vi.waitFor(() => expect(view.container.querySelector('h1')).not.toBeNull());
  if (step === 1 || step === 'expired') return view;
  await fireEvent.click(view.getByRole('button', { name: 'Accept invitation' }));
  if (step === 2) return view;
  await fireEvent.input(view.getByLabelText('Choose a password'), { target: { value: 'Safe-enough1' } });
  await fireEvent.input(view.getByLabelText('Confirm password'), { target: { value: 'Safe-enough1' } });
  await fireEvent.click(view.getByRole('button', { name: 'Continue' }));
  await view.findByLabelText('Verification code');
  return view;
}

async function confirmEmail(state: string) {
  mockApi({ 'POST auth/email/check': [{ body: { state, email: 'new@example.com', own_session: false } }] });
  page.params = { token: 'tok' };
  ready();
  const view = render(ConfirmEmail) as RenderResult<never>;
  await vi.waitFor(() => expect(view.container.querySelector('h1')).not.toBeNull());
  await vi.waitFor(() => expect(view.container.querySelector('[aria-busy="true"]')).toBeNull());
  return view;
}

const CASES: Case[] = [
  ['setup: language and appearance', () => Promise.resolve(render(Setup) as RenderResult<never>)],
  [
    'setup: token',
    () => {
      sessionStorage.setItem('roedy.hub.setup.step', '2');
      return Promise.resolve(render(Setup) as RenderResult<never>);
    },
  ],
  ['sign-in', () => Promise.resolve(login({ kind: 'login' }))],
  [
    'sign-in again (reauth)',
    () => {
      session.user = sessionOf({ id: '01J', email: 'alex@example.com', name: 'Alex Example' }).user;
      return Promise.resolve(login({ kind: 'login' }, 'http://localhost/login?reauth=1&next=%2Fmail%2F'));
    },
  ],
  ['reset password', () => Promise.resolve(login({ kind: 'forgot' }, 'http://localhost/login/forgot'))],
  [
    'set a new password',
    () => Promise.resolve(login({ kind: 'newpw', token: 'tok' }, 'http://localhost/login/reset/tok')),
  ],
  ['invitation: welcome', () => invite(1)],
  ['invitation: account', () => invite(2)],
  ['invitation: authenticator app', () => invite(3)],
  ['invitation: expired', () => invite('expired')],
  ['confirm email: valid', () => confirmEmail('valid')],
  ['confirm email: invalid', () => confirmEmail('invalid')],
  ['confirm email: already confirmed', () => confirmEmail('already_confirmed')],
  ...(['404', '403', '500', '502', '503', '504', 'offline'] as const).map((code): Case => [
    `error ${code}`,
    () => {
      ready();
      return Promise.resolve(render(ErrorView, { code }) as RenderResult<never>);
    },
  ]),
  [
    'home page',
    () => {
      ready();
      session.user = sessionOf({ id: '01J', email: 'alex@example.com', name: 'Alex Example' }).user;
      return Promise.resolve(render(Home) as RenderResult<never>);
    },
  ],
];

describe('accessibility (axe)', () => {
  for (const mode of ['light', 'dark'] as const) {
    it.each(CASES)(`has no violations: %s (${mode})`, async (_name, show) => {
      prefs.animations = false;
      prefs.set({ mode });
      document.documentElement.dataset.mode = mode;
      const view = await show();
      expect(await violations(view)).toEqual([]);
      view.unmount();
    });
  }
});
