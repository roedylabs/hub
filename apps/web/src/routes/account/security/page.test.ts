import { fireEvent, within } from '@testing-library/svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { goto } from '$app/navigation';
import type { MfaOverview } from '#lib/security.svelte.ts';
import { session } from '#lib/state/session.svelte.ts';
import { ui } from '@roedy/ui';
import { sessionOf } from '#lib/test/session.ts';
import { renderSignedIn, resetSignedIn, setPhone, type ApiRoutes } from '#lib/test/signed-in.ts';
import Page from './+page.svelte';

vi.mock('$app/navigation', () => ({ goto: vi.fn(() => Promise.resolve()) }));

const NOW = Date.parse('2026-10-26T12:00:00Z');
const MFA: MfaOverview = {
  devices: [{ id: 't1', name: 'iPhone', created_at: '2026-10-01T09:00:00Z', last_used_at: null }],
  recovery: { total: 10, unused: 10, created_at: '2026-10-01T09:00:00Z' },
};
const NO_MFA: MfaOverview = { devices: [], recovery: null };
const SESSIONS = [
  {
    id: 's0',
    current: true,
    device: 'Firefox on Linux',
    device_type: 'desktop',
    ip: '203.0.x.x',
    created_at: '2026-10-26T08:00:00Z',
    last_seen_at: '2026-10-26T12:00:00Z',
  },
  {
    id: 's1',
    current: false,
    device: 'Safari on iPhone',
    device_type: 'mobile',
    ip: '2001:db8:x:x:x:x:x:x',
    created_at: '2026-10-25T08:00:00Z',
    last_seen_at: '2026-10-26T10:00:00Z',
  },
];
const ENROLL = {
  id: 't2',
  secret: 'JBSWY3DPEHPK3PXPJBSWY3DPEHPK3PXP',
  uri: 'otpauth://totp/x?secret=JBSWY3DPEHPK3PXP',
};

beforeEach(() => {
  vi.useFakeTimers({ toFake: ['Date'] });
  vi.setSystemTime(NOW);
});

afterEach(() => {
  vi.useRealTimers();
  vi.mocked(goto).mockClear();
  resetSignedIn();
});

function renderPage(routes: ApiRoutes = {}, mfa = MFA) {
  return renderSignedIn(Page, {
    role: 'user',
    path: '/account/security',
    api: {
      'GET auth/totp/devices': [{ body: mfa }, { body: mfa }],
      'GET auth/sessions': [{ body: { sessions: SESSIONS } }, { body: { sessions: SESSIONS } }],
      'GET auth/me': [
        { body: { password_changed_at: '2026-10-20T12:00:00Z' } },
        { body: { password_changed_at: '2026-10-20T12:00:00Z' } },
      ],
      ...routes,
    },
  });
}

describe('Security page', () => {
  it('shows password, multi-factor, codes and devices', async () => {
    const view = renderPage();
    expect(view.getByRole('heading', { level: 1 })).toHaveTextContent('Security');
    expect(await view.findByText('Last changed 6 days ago')).toBeInTheDocument();
    expect(view.getByRole('button', { name: 'Multi-factor' })).toBeInTheDocument();
    expect(view.getByLabelText('Verification code')).toBeInTheDocument();
    expect(view.getByText('You’ll then be signed out on all other devices.')).toBeInTheDocument();
    expect(view.getByRole('button', { name: 'Change password' })).toHaveAttribute('aria-disabled', 'true');
    expect(view.getByText(/^Added on 1 Oct 2026/)).toBeInTheDocument();
    expect(view.getByRole('button', { name: 'Remove iPhone' })).toBeInTheDocument();
    expect(view.getByText('10 of 10 unused')).toBeInTheDocument();
    expect(view.getByText('IP 203.0.x.x · Active now')).toBeInTheDocument();
    expect(view.getByText('IP 2001:db8:x:x:x:x:x:x · Last active 2 hours ago')).toBeInTheDocument();
    expect(view.getByText('This device')).toBeInTheDocument();
    expect(view.getAllByRole('button', { name: 'Sign out' })).toHaveLength(1);
    expect(view.getByRole('button', { name: 'Sign out all other devices' })).toBeInTheDocument();
  });

  it('changes the password', async () => {
    const view = renderPage({
      'PUT auth/password': [{ body: { password_changed_at: '2026-10-26T12:00:00Z', sessions_ended: 1 } }],
    });
    await view.findByText('Safari on iPhone');
    const [current, code, next, confirm] = [
      'Current password',
      'Verification code',
      'New password',
      'Confirm new password',
    ].map((label) => view.getAllByLabelText(label)[0] as HTMLElement);
    await fireEvent.input(current as HTMLElement, { target: { value: 'Old-Password1' } });
    await fireEvent.input(code as HTMLElement, { target: { value: '123456' } });
    await fireEvent.input(next as HTMLElement, { target: { value: 'New-Password1' } });
    await fireEvent.input(confirm as HTMLElement, { target: { value: 'New-Password1' } });
    const go = view.getByRole('button', { name: 'Change password' });
    expect(go).toHaveAttribute('aria-disabled', 'false');
    await fireEvent.click(go);
    await vi.waitFor(() => expect(ui.toast?.text).toBe('Password changed. Other devices were signed out.'));
    expect(view.queryByText('Safari on iPhone')).toBeNull();
    expect(await view.findByText('Last changed just now')).toBeInTheDocument();
  });

  it('reports a wrong current password next to the button', async () => {
    const view = renderPage({ 'PUT auth/password': [{ status: 422, body: { error: 'invalid_password' } }] }, NO_MFA);
    await view.findByText('Safari on iPhone');
    await fireEvent.input(view.getAllByLabelText('Current password')[0] as HTMLElement, {
      target: { value: 'Wrong-Password1' },
    });
    await fireEvent.input(view.getByLabelText('New password'), { target: { value: 'New-Password1' } });
    await fireEvent.input(view.getByLabelText('Confirm new password'), { target: { value: 'New-Password1' } });
    await fireEvent.keyDown(view.getByLabelText('Confirm new password'), { key: 'Enter' });
    expect(await view.findByText('The password is incorrect.')).toHaveAttribute('role', 'alert');
    expect(view.getAllByLabelText('Current password')[0]).toHaveValue('');
  });

  it('sends the user to a fresh sign-in when the server asks for it', async () => {
    const view = renderPage({ 'PUT auth/password': [{ status: 403, body: { error: 'reauth_required' } }] }, NO_MFA);
    await view.findByText('Safari on iPhone');
    await fireEvent.input(view.getAllByLabelText('Current password')[0] as HTMLElement, {
      target: { value: 'Old-Password1' },
    });
    await fireEvent.input(view.getByLabelText('New password'), { target: { value: 'New-Password1' } });
    await fireEvent.input(view.getByLabelText('Confirm new password'), { target: { value: 'New-Password1' } });
    await fireEvent.click(view.getByRole('button', { name: 'Change password' }));
    await vi.waitFor(() => expect(goto).toHaveBeenCalledWith('/login?reauth=1&next=%2Faccount%2Fsecurity'));
  });

  it('adds an app with a QR code and then shows the codes', async () => {
    const codes = Array.from({ length: 10 }, (_, i) => `ABCD-EF${String(i + 2)}A`);
    const view = renderPage(
      {
        'POST auth/totp/devices/enroll': [{ body: ENROLL }],
        'POST auth/totp/devices/confirm': [{ body: { ...MFA, recovery_codes: codes } }],
      },
      NO_MFA,
    );
    expect(await view.findByText('No authenticator app set up yet')).toBeInTheDocument();
    expect(view.getByText('Generated once you add your first authenticator app.')).toBeInTheDocument();
    await fireEvent.click(view.getByRole('button', { name: 'Add authenticator app' }));
    expect(await view.findByRole('img', { name: 'QR code for the authenticator app' })).toBeInTheDocument();
    expect(view.getByText('JBSW Y3DP EHPK 3PXP JBSW Y3DP EHPK 3PXP')).toBeInTheDocument();
    await fireEvent.input(view.getByPlaceholderText('e.g. iPhone'), { target: { value: 'iPhone' } });
    await fireEvent.input(view.getAllByLabelText('Current password')[1] as HTMLElement, {
      target: { value: 'Secret-123' },
    });
    await fireEvent.input(view.getAllByLabelText('Verification code')[0] as HTMLElement, {
      target: { value: '123456' },
    });
    // The button goes with the prompt: the focus moves to the codes, read with their hint.
    const add = view.getByRole('button', { name: 'Add' });
    add.focus();
    await fireEvent.click(add);
    const list = await view.findByRole('list', { name: 'Recovery codes' });
    expect(within(list).getAllByRole('listitem')).toHaveLength(10);
    expect(ui.toast?.text).toBe('Multi-factor authentication turned on');
    expect(list).toHaveFocus();
    expect(list).toHaveAccessibleDescription(/They’re only shown once\./);
    await fireEvent.click(view.getByRole('button', { name: 'Done' }));
    expect(view.queryByRole('list', { name: 'Recovery codes' })).toBeNull();
  });

  it('offers "Start over" when the setup was replaced in another window', async () => {
    const next = { id: 't3', secret: 'KRSXG5CTMVRXEZLUKRSXG5CTMVRXEZLU', uri: 'otpauth://totp/x?secret=KRSX' };
    const view = renderPage(
      {
        'POST auth/totp/devices/enroll': [{ body: ENROLL }, { body: next }],
        'POST auth/totp/devices/confirm': [{ status: 404, body: { error: 'not_found' } }],
      },
      NO_MFA,
    );
    await view.findByText('Safari on iPhone');
    await fireEvent.click(view.getByRole('button', { name: 'Add authenticator app' }));
    await view.findByText('JBSW Y3DP EHPK 3PXP JBSW Y3DP EHPK 3PXP');
    await fireEvent.input(view.getByPlaceholderText('e.g. iPhone'), { target: { value: 'Laptop' } });
    await fireEvent.input(view.getAllByLabelText('Current password')[1] as HTMLElement, {
      target: { value: 'Secret-123' },
    });
    await fireEvent.input(view.getAllByLabelText('Verification code')[0] as HTMLElement, {
      target: { value: '123456' },
    });
    await fireEvent.click(view.getByRole('button', { name: 'Add' }));
    expect(await view.findByText('Setup was restarted in another window. Start it again here.')).toBeInTheDocument();
    await fireEvent.click(view.getByRole('button', { name: 'Start over' }));
    expect(await view.findByText('KRSX G5CT MVRX EZLU KRSX G5CT MVRX EZLU')).toBeInTheDocument();
    expect(view.getByPlaceholderText('e.g. iPhone')).toHaveValue('Laptop');
  });

  it('shows invalid characters in the app name at the name field before asking for the password', async () => {
    const view = renderPage({ 'POST auth/totp/devices/enroll': [{ body: ENROLL }] });
    await view.findByText('Safari on iPhone');
    await fireEvent.click(view.getByRole('button', { name: 'Add authenticator app' }));
    await view.findByText('JBSW Y3DP EHPK 3PXP JBSW Y3DP EHPK 3PXP');
    await fireEvent.input(view.getByPlaceholderText('e.g. iPhone'), {
      target: { value: `Lap${String.fromCodePoint(0x200b)}top` },
    });
    await fireEvent.click(view.getByRole('button', { name: 'Add' }));
    const label = view.getByPlaceholderText('e.g. iPhone').closest('label') as HTMLElement;
    expect(await within(label).findByRole('alert')).toHaveTextContent('The name contains invalid characters.');
    expect(view.calls.some((c) => c.path === 'auth/totp/devices/confirm')).toBe(false);
  });

  it('removes the last app after password and code', async () => {
    const view = renderPage({ 'DELETE auth/totp/devices/t1': [{ body: NO_MFA }] });
    await view.findByText('Safari on iPhone');
    await fireEvent.click(view.getByRole('button', { name: 'Remove iPhone' }));
    expect(view.getByText(/Multi-factor authentication will then be turned off\./)).toBeInTheDocument();
    await fireEvent.input(view.getAllByLabelText('Current password')[1] as HTMLElement, {
      target: { value: 'Secret-123' },
    });
    await fireEvent.input(view.getAllByLabelText('Verification code')[1] as HTMLElement, {
      target: { value: '123456' },
    });
    await fireEvent.click(view.getByRole('button', { name: 'Remove' }));
    expect(await view.findByText('No authenticator app set up yet')).toBeInTheDocument();
    expect(ui.toast?.text).toBe('Multi-factor authentication turned off');
    expect(view.queryByLabelText('Verification code')).toBeNull();
  });

  it('signs out a device right away', async () => {
    const view = renderPage({ 'DELETE auth/sessions/s1': [{ status: 204 }] });
    await view.findByText('Safari on iPhone');
    await fireEvent.click(view.getByRole('button', { name: 'Sign out' }));
    expect(view.queryByText('Safari on iPhone')).toBeNull();
    await vi.waitFor(() => expect(ui.toast?.text).toBe('Device signed out'));
    expect(view.queryByRole('button', { name: 'Sign out all other devices' })).toBeNull();
  });

  it('asks before signing out all other devices', async () => {
    const view = renderPage({ 'POST auth/sessions/revoke-others': [{ body: { sessions_ended: 1 } }] });
    await view.findByText('Safari on iPhone');
    await fireEvent.click(view.getByRole('button', { name: 'Sign out all other devices' }));
    expect(ui.confirm?.title).toBe('Sign out all other devices?');
    ui.confirmOk();
    await vi.waitFor(() => expect(view.queryByText('Safari on iPhone')).toBeNull());
    await vi.waitFor(() => expect(ui.toast?.text).toBe('Other devices signed out'));
  });

  it('shows sign-out as an icon and "Sign out all others" on mobile', async () => {
    setPhone(true);
    const view = renderPage();
    await view.findByText('Safari on iPhone');
    expect(view.getByText('Where you’re currently signed in.')).toBeInTheDocument();
    expect(view.getByRole('button', { name: 'Sign out all others' })).toBeInTheDocument();
    expect(view.getByRole('button', { name: 'Sign out' })).toBeInTheDocument();
    await fireEvent.click(view.getByRole('button', { name: 'Generate new codes' }));
    expect(view.getByRole('button', { name: 'Generate' })).toBeInTheDocument();
  });

  it('shows the load card instead of the sections when loading fails', async () => {
    const view = renderPage({ 'GET auth/totp/devices': [{ status: 500, body: {} }] });
    const card = await view.findByRole('alert');
    expect(card).toHaveTextContent('Couldn’t load security settings');
    expect(card).toHaveTextContent('That didn’t work. Please try again.');
    expect(view.getByRole('button', { name: 'Try again' })).toBeInTheDocument();
    expect(view.queryByRole('heading', { level: 2 })).toBeNull();
  });

  it('shows the data of another account signed in by another tab and drops what was open', async () => {
    const other = [{ ...SESSIONS[0], id: 'o0', device: 'Chrome on Android', device_type: 'mobile' }];
    const view = renderPage({
      'GET auth/totp/devices': [{ body: MFA }, { body: NO_MFA }],
      'GET auth/sessions': [{ body: { sessions: SESSIONS } }, { body: { sessions: other } }],
      'POST auth/totp/devices/enroll': [{ body: ENROLL }],
    });
    await view.findByText('Safari on iPhone');
    await fireEvent.click(view.getByRole('button', { name: 'Add authenticator app' }));
    await view.findByText('JBSW Y3DP EHPK 3PXP JBSW Y3DP EHPK 3PXP');
    await fireEvent.input(view.getAllByLabelText('Current password')[0] as HTMLElement, {
      target: { value: 'Old-Password1' },
    });
    session.user = sessionOf({ id: '01JOTHER', email: 'jordan@example.com', name: 'Jordan Hale' }).user;
    expect(await view.findByText('Chrome on Android')).toBeInTheDocument();
    expect(view.queryByText('Safari on iPhone')).toBeNull();
    expect(view.queryByText('JBSW Y3DP EHPK 3PXP JBSW Y3DP EHPK 3PXP')).toBeNull();
    expect(view.getByText('No authenticator app set up yet')).toBeInTheDocument();
    expect(view.getByLabelText('Current password')).toHaveValue('');
  });

  it('ignores answers that arrive after the page was left', async () => {
    let release: (reply: { body: unknown }) => void = () => {};
    const view = renderPage({
      'GET auth/totp/devices': [() => new Promise((resolve) => (release = resolve))],
    });
    view.unmount();
    release({ body: MFA });
    await Promise.resolve();
    expect(ui.toast).toBeNull();
  });
});
