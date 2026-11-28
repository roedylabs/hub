/**
 * axe-core over Account › Security in every state of the page (loaded with
 * and without an authenticator app, loading failed, adding an app with QR
 * code and an error, removing the last app, new recovery codes shown, the
 * form for new codes with an error, a password error), desktop and mobile, in
 * the light and the dark theme. Colour contrast is left to the browser audit
 * (`scripts/a11y/scenarios/security.mjs`).
 */
import { fireEvent, type RenderResult } from '@testing-library/svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { MfaOverview } from '#lib/security.svelte.ts';
import { prefs } from '#lib/state/prefs.svelte.ts';
import { violations } from '@roedy/ui/testing';
import { renderSignedIn, resetSignedIn, setPhone, type ApiRoutes } from '#lib/test/signed-in.ts';
import Page from './+page.svelte';

vi.mock('$app/navigation', () => ({ goto: vi.fn(() => Promise.resolve()) }));

afterEach(() => {
  delete document.documentElement.dataset.mode;
  resetSignedIn();
});

const MFA: MfaOverview = {
  devices: [{ id: 't1', name: 'iPhone', created_at: '2026-10-01T09:00:00Z', last_used_at: '2026-10-26T10:00:00Z' }],
  recovery: { total: 10, unused: 9, created_at: '2026-10-01T09:00:00Z' },
};
const NO_MFA: MfaOverview = { devices: [], recovery: null };
const CODES = Array.from({ length: 10 }, (_, i) => `K7PX-${String(4000 + i * 137).slice(0, 4)}`);
const SESSIONS = [
  {
    id: 's0',
    current: true,
    device: 'Firefox on Linux',
    device_type: 'desktop',
    ip: '198.51.x.x',
    created_at: '2026-10-26T08:00:00Z',
    last_seen_at: '2026-10-26T12:00:00Z',
  },
  {
    id: 's1',
    current: false,
    device: 'Safari on iPhone',
    device_type: 'mobile',
    ip: null,
    created_at: '2026-10-25T08:00:00Z',
    last_seen_at: '2026-10-26T10:00:00Z',
  },
];
const ENROLL = {
  id: 't2',
  secret: 'JBSWY3DPEHPK3PXPJBSWY3DPEHPK3PXP',
  uri: 'otpauth://totp/x?secret=JBSWY3DPEHPK3PXP',
};

type View = RenderResult<never>;

interface State {
  name: string;
  mfa?: MfaOverview;
  api?: ApiRoutes;
  prepare?: (view: View) => Promise<void>;
}

const input = (view: View, label: string, n: number, value: string) =>
  fireEvent.input(view.getAllByLabelText(label)[n] as HTMLElement, { target: { value } });

const STATES: State[] = [
  { name: 'with an app', prepare: (view) => view.findByText('Safari on iPhone').then(() => {}) },
  { name: 'without an app', mfa: NO_MFA, prepare: (view) => view.findByText('Safari on iPhone').then(() => {}) },
  {
    name: 'loading failed',
    api: { 'GET auth/totp/devices': [{ status: 500, body: {} }] },
    prepare: (view) => view.findByRole('button', { name: 'Try again' }).then(() => {}),
  },
  {
    name: 'adding an app with an error',
    mfa: NO_MFA,
    api: {
      'POST auth/totp/devices/enroll': [{ body: ENROLL }],
      'POST auth/totp/devices/confirm': [{ status: 422, body: { error: 'invalid_code' } }],
    },
    prepare: async (view) => {
      await view.findByText('Safari on iPhone');
      await fireEvent.click(view.getByRole('button', { name: 'Add authenticator app' }));
      await view.findByRole('img', { name: 'QR code for the authenticator app' });
      await input(view, 'Current password', 1, 'Secret-123');
      await input(view, 'Verification code', 0, '123456');
      await fireEvent.click(view.getByRole('button', { name: 'Add' }));
      await view.findByText('That code isn’t right. Enter the current code from the new app.');
    },
  },
  {
    name: 'setup replaced in another window',
    mfa: NO_MFA,
    api: {
      'POST auth/totp/devices/enroll': [{ body: ENROLL }],
      'POST auth/totp/devices/confirm': [{ status: 404, body: { error: 'not_found' } }],
    },
    prepare: async (view) => {
      await view.findByText('Safari on iPhone');
      await fireEvent.click(view.getByRole('button', { name: 'Add authenticator app' }));
      await view.findByRole('img', { name: 'QR code for the authenticator app' });
      await fireEvent.input(view.getByPlaceholderText('e.g. iPhone'), {
        target: { value: `Lap${String.fromCodePoint(0x200b)}top` },
      });
      await fireEvent.click(view.getByRole('button', { name: 'Add' }));
      await view.findByText('The name contains invalid characters.');
      await fireEvent.input(view.getByPlaceholderText('e.g. iPhone'), { target: { value: 'Laptop' } });
      await input(view, 'Current password', 1, 'Secret-123');
      await input(view, 'Verification code', 0, '123456');
      await fireEvent.click(view.getByRole('button', { name: 'Add' }));
      await view.findByRole('button', { name: 'Start over' });
    },
  },
  {
    name: 'removing the last app',
    prepare: async (view) => {
      await view.findByText('Safari on iPhone');
      await fireEvent.click(view.getByRole('button', { name: 'Remove iPhone' }));
    },
  },
  {
    name: 'new recovery codes shown',
    api: { 'POST auth/recovery-codes': [{ body: { ...MFA, recovery_codes: CODES } }] },
    prepare: async (view) => {
      await view.findByText('Safari on iPhone');
      await fireEvent.click(view.getByRole('button', { name: 'Generate new codes' }));
      await input(view, 'Current password', 1, 'Secret-123');
      await input(view, 'Verification code', 1, '123456');
      await fireEvent.click(view.getAllByRole('button', { name: /^Generate/ }).at(-1) as HTMLElement);
      await view.findByRole('list', { name: 'Recovery codes' });
    },
  },
  {
    name: 'new codes with a lockout',
    api: { 'POST auth/recovery-codes': [{ status: 429, body: { error: 'rate_limited', retry_after: 900 } }] },
    prepare: async (view) => {
      await view.findByText('Safari on iPhone');
      await fireEvent.click(view.getByRole('button', { name: 'Generate new codes' }));
      await input(view, 'Current password', 1, 'Secret-123');
      await input(view, 'Verification code', 1, '123456');
      await fireEvent.click(view.getAllByRole('button', { name: /^Generate/ }).at(-1) as HTMLElement);
      await view.findByText('Too many failed attempts. Try again in 15 minutes.');
    },
  },
  {
    name: 'password error',
    mfa: NO_MFA,
    prepare: async (view) => {
      await view.findByText('Safari on iPhone');
      await input(view, 'Current password', 0, 'Old-Password1');
      await input(view, 'New password', 0, 'short');
      await input(view, 'Confirm new password', 0, 'short');
      await fireEvent.click(view.getByRole('button', { name: 'Change password' }));
      await view.findByText('The new password must be at least 10 characters long.');
    },
  },
];

describe('Security page accessibility', () => {
  for (const mobile of [false, true]) {
    for (const mode of ['light', 'dark'] as const) {
      for (const state of STATES) {
        it(`${state.name} (${mobile ? 'mobile' : 'desktop'}, ${mode})`, async () => {
          setPhone(mobile);
          document.documentElement.dataset.mode = mode;
          prefs.set({ mode });
          const mfa = state.mfa ?? MFA;
          const view = renderSignedIn(Page, {
            role: 'user',
            api: {
              'GET auth/totp/devices': [{ body: mfa }],
              'GET auth/sessions': [{ body: { sessions: SESSIONS } }],
              'GET auth/me': [{ body: { password_changed_at: '2026-10-20T12:00:00Z' } }],
              // A copy: `mockApi` uses the answers up.
              ...structuredClone(state.api),
            },
          }) as unknown as View;
          await state.prepare?.(view);
          expect(await violations(view)).toEqual([]);
        });
      }
    }
  }
});
