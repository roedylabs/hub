/**
 * axe-core over every state of Account › Profile (desktop and mobile), each in
 * the light and the dark theme; the colour contrast is left to the browser
 * audit (`scripts/a11y/scenarios/profile.mjs`).
 */
import { fireEvent, within } from '@testing-library/svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { ProfileMe } from '#lib/profile.svelte.ts';
import { prefs } from '#lib/state/prefs.svelte.ts';
import { violations } from '@roedy/ui/testing';
import { renderSignedIn, resetSignedIn, setPhone, type ApiRoutes } from '#lib/test/signed-in.ts';
import Page from './+page.svelte';

vi.mock('$app/navigation', () => ({ goto: vi.fn(() => Promise.resolve()) }));

const ME: ProfileMe = {
  id: '01JUSER',
  username: 'alex',
  email: 'alex@example.com',
  display_name: 'Alex Example',
  role: 'admin',
  mfa_enabled: true,
  password_changed_at: '2026-10-20T10:00:00Z',
  email_change: null,
  sole_admin: false,
};
const USER = { id: ME.id, username: 'alex', email: 'alex@example.com', display_name: 'Alex Example', role: 'admin' };
const OPEN = { email: 'new@example.com', expires_at: '2026-10-27T10:00:00Z' };

afterEach(() => {
  resetSignedIn();
  delete document.documentElement.dataset.mode;
});

type View = ReturnType<typeof renderSignedIn>;
type Case = [name: string, show: () => Promise<View>];

function open(me: ProfileMe | null, api: ApiRoutes = {}): View {
  const first = me ? { body: me } : { status: 500, body: {} };
  return renderSignedIn(Page, { path: '/account/profile', api: { 'GET auth/me': [first], ...api } });
}

const STATES: Case[] = [
  [
    'ready',
    async () => {
      const view = open(ME);
      await view.findByText('AE');
      return view;
    },
  ],
  [
    'load failed',
    async () => {
      const view = open(null);
      await view.findByRole('button', { name: 'Try again' });
      return view;
    },
  ],
  [
    'editing with unsaved changes and an error',
    async () => {
      const view = open(ME);
      await fireEvent.click(await view.findByRole('button', { name: 'Edit display name' }));
      await fireEvent.input(view.getByRole('textbox', { name: 'Display name' }), { target: { value: 'Alex R' } });
      await fireEvent.click(view.getByRole('button', { name: 'Edit Username' }));
      await fireEvent.input(view.getByRole('textbox', { name: 'Username' }), { target: { value: 'Alex!' } });
      const bar = view.getByRole('region', { name: 'Unsaved changes' });
      await fireEvent.click(within(bar).getByRole('button', { name: 'Save' }));
      await view.findByRole('alert');
      return view;
    },
  ],
  [
    'password and code prompt with a message',
    async () => {
      const view = open(ME, {
        'PATCH auth/me': [
          { body: { status: 'reauth_required', user: USER } },
          { status: 422, body: { error: 'invalid_password' } },
        ],
      });
      await fireEvent.click(await view.findByRole('button', { name: 'Edit Email' }));
      const input = view.getByRole('textbox', { name: 'Email' });
      await fireEvent.input(input, { target: { value: 'new@example.com' } });
      await fireEvent.keyDown(input, { key: 'Enter' });
      const group = await view.findByRole('group', { name: 'Confirm email address change' });
      await fireEvent.input(within(group).getByLabelText('Current password'), { target: { value: 'wrong' } });
      await fireEvent.input(within(group).getByLabelText('Verification code'), { target: { value: '123456' } });
      await fireEvent.click(within(group).getByRole('button', { name: 'Send confirmation link' }));
      await within(group).findByText('The password is incorrect.');
      return view;
    },
  ],
  [
    'confirmation pending with a failure',
    async () => {
      const view = open(
        { ...ME, email_change: OPEN },
        { 'POST auth/me/email/resend': [{ status: 502, body: { error: 'mail_failed', previous_link_valid: true } }] },
      );
      const status = await view.findByRole('status');
      await fireEvent.click(within(status).getByRole('button', { name: 'Resend link' }));
      await within(status).findByRole('alert');
      return view;
    },
  ],
  [
    'only administrator',
    async () => {
      const view = open({ ...ME, sole_admin: true });
      await view.findByRole('link', { name: 'Manage users' });
      return view;
    },
  ],
];

describe('accessibility (axe)', () => {
  for (const phone of [false, true]) {
    for (const mode of ['light', 'dark'] as const) {
      it.each(STATES)(`has no violations: %s (${phone ? 'mobile' : 'desktop'}, ${mode})`, async (_name, show) => {
        prefs.set({ mode });
        document.documentElement.dataset.mode = mode;
        setPhone(phone);
        const view = await show();
        expect(await violations(view)).toEqual([]);
        view.unmount();
      });
    }
  }
});
