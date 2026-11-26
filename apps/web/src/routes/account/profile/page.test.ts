import { fireEvent, within } from '@testing-library/svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { ProfileMe } from '#lib/profile.svelte.ts';
import { session } from '#lib/state/session.svelte.ts';
import { ui } from '#lib/state/ui.svelte.ts';
import { renderSignedIn, resetSignedIn, setPhone, type ApiRoutes } from '#lib/test/signed-in.ts';
import { sessionOf } from '#lib/test/session.ts';
import Page from './+page.svelte';

const nav = vi.hoisted(() => ({ goto: vi.fn(() => Promise.resolve()) }));
vi.mock('$app/navigation', () => ({ goto: nav.goto }));

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
  nav.goto.mockClear();
});

function show(me: ProfileMe = ME, api: ApiRoutes = {}) {
  const { 'GET auth/me': reads = [], ...rest } = api;
  return renderSignedIn(Page, { path: '/account/profile', api: { 'GET auth/me': [{ body: me }, ...reads], ...rest } });
}

describe('Profile page', () => {
  it('shows account, display name, username, email, sign-in and "Delete account"', async () => {
    const view = show();
    expect(view.getByRole('heading', { level: 1 })).toHaveTextContent('Profile');
    expect(await view.findByText('AE')).toBeInTheDocument();
    expect(view.getByText('Shown to other people')).toBeInTheDocument();
    expect(view.getAllByText('alex@example.com')).toHaveLength(2);
    expect(view.getByText('Password')).toBeInTheDocument();
    expect(view.getByRole('button', { name: 'Edit display name' })).toBeInTheDocument();
    expect(view.getByRole('button', { name: 'Edit Username' })).toBeInTheDocument();
    expect(view.getByRole('button', { name: 'Edit Email' })).toBeInTheDocument();
    expect(view.queryByRole('button', { name: 'Edit Sign-in' })).toBeNull();
    expect(view.getByRole('heading', { level: 2, name: 'Delete account' })).toBeInTheDocument();
    expect(view.getByRole('button', { name: 'Delete account' })).toHaveAttribute('aria-disabled', 'false');
    expect(view.queryByRole('link', { name: 'Manage users' })).toBeNull();
  });

  it('shows the failure card and loads again with "Try again"', async () => {
    const view = renderSignedIn(Page, {
      path: '/account/profile',
      api: { 'GET auth/me': [{ status: 500, body: {} }, { body: ME }] },
    });
    const card = await view.findByRole('alert');
    expect(card).toHaveTextContent('Couldn’t load your profile');
    expect(card).toHaveTextContent('That didn’t work. Please try again.');
    await fireEvent.click(within(card).getByRole('button', { name: 'Try again' }));
    expect(await view.findByText('AE')).toBeInTheDocument();
  });

  it('edits the name with the "Unsaved changes" bar and saves', async () => {
    const { calls, ...view } = show(ME, {
      'PATCH auth/me': [{ body: { status: 'ok', user: { ...USER, display_name: 'Alex Rivera' } } }],
    });
    await fireEvent.click(await view.findByRole('button', { name: 'Edit display name' }));
    const input = view.getByRole('textbox', { name: 'Display name' });
    expect(input).toHaveFocus();
    await fireEvent.input(input, { target: { value: 'Alex Rivera' } });
    const bar = view.getByRole('region', { name: 'Unsaved changes' });
    await fireEvent.click(within(bar).getByRole('button', { name: 'Save' }));
    await vi.waitFor(() => expect(ui.toast?.text).toBe('Changes saved'));
    expect(calls.filter((c) => c.method === 'PATCH').map((c) => c.body)).toEqual([{ display_name: 'Alex Rivera' }]);
    expect(view.queryByRole('region', { name: 'Unsaved changes' })).toBeNull();
    expect(view.getAllByText('Alex Rivera')).toHaveLength(2);
  });

  it('asks for password and code for a new address and then shows "Confirmation pending"', async () => {
    const { calls, ...view } = show(ME, {
      'PATCH auth/me': [
        { body: { status: 'reauth_required', user: USER } },
        { status: 502, body: { error: 'mail_failed' } },
        { body: { status: 'email_sent', user: USER, email_change: OPEN } },
      ],
    });
    await fireEvent.click(await view.findByRole('button', { name: 'Edit Email' }));
    const input = view.getByRole('textbox', { name: 'Email' });
    await fireEvent.input(input, { target: { value: 'new@example.com' } });
    await fireEvent.keyDown(input, { key: 'Enter' });
    const group = await view.findByRole('group', { name: 'Confirm email address change' });
    expect(
      within(group).getByText('Confirm the change with your password and a code from your authenticator app.'),
    ).toBeInTheDocument();
    const send = within(group).getByRole('button', { name: 'Send confirmation link' });
    expect(send).toHaveAttribute('aria-disabled', 'true');
    const pw = within(group).getByLabelText('Current password');
    expect(pw).toHaveFocus();
    await fireEvent.input(pw, { target: { value: 'Secret-123' } });
    await fireEvent.input(within(group).getByLabelText('Verification code'), { target: { value: '123456' } });
    expect(send).toHaveAttribute('aria-disabled', 'false');
    await fireEvent.click(send);
    await vi.waitFor(() =>
      expect(
        within(group).getByText(
          'The email couldn’t be sent. Your email address wasn’t changed. Try again later with a new code.',
        ),
      ).toBeInTheDocument(),
    );
    expect(within(group).getByLabelText('Verification code')).toHaveValue('');
    await fireEvent.input(within(group).getByLabelText('Verification code'), { target: { value: '654321' } });
    await fireEvent.click(send);
    const status = await view.findByRole('status');
    expect(status).toHaveTextContent('Confirmation pending');
    expect(status).toHaveTextContent('We sent a confirmation link to new@example.com.');
    expect(within(status).getByRole('button', { name: 'Resend link' })).toBeInTheDocument();
    expect(within(status).getByRole('button', { name: 'Discard change' })).toBeInTheDocument();
    expect(calls.filter((c) => c.method === 'PATCH')).toHaveLength(3);
  });

  it('discards a pending change', async () => {
    const pending = { ...ME, email_change: OPEN };
    const view = show(pending, { 'GET auth/me': [{ body: pending }], 'DELETE auth/me/email': [{ status: 204 }] });
    const status = await view.findByRole('status');
    await fireEvent.click(within(status).getByRole('button', { name: 'Discard change' }));
    await vi.waitFor(() => expect(view.queryByRole('status')).toBeNull());
    expect(ui.toast?.text).toBe('Change discarded');
  });

  it('locks "Delete account" for the only administrator and links to the users', async () => {
    const view = show({ ...ME, sole_admin: true });
    const button = await view.findByRole('button', { name: 'Delete account' });
    expect(button).toHaveAttribute('aria-disabled', 'true');
    expect(view.getByRole('link', { name: 'Manage users' })).toHaveAttribute('href', '/admin/users');
    await fireEvent.click(button);
    expect(ui.confirm).toBeNull();
  });

  it('deletes the account after the confirmation and opens the sign-in', async () => {
    const { calls, ...view } = show(ME, { 'GET auth/me': [{ body: ME }], 'DELETE auth/me': [{ status: 204 }] });
    await fireEvent.click(await view.findByRole('button', { name: 'Delete account' }));
    expect(ui.confirm?.word).toBe('DELETE');
    ui.confirm?.run();
    await vi.waitFor(() => expect(nav.goto).toHaveBeenCalledWith('/login', { replace: true }));
    expect(calls.at(-1)).toMatchObject({ method: 'DELETE', path: 'auth/me', body: { confirm: 'delete' } });
    expect(session.user).toBeNull();
  });

  it('shows the profile of another account signed in by another tab', async () => {
    const other = { ...ME, id: '01JOTHER', display_name: 'Jordan Hale', username: 'jordan' };
    const view = show(ME, { 'GET auth/me': [{ body: other }] });
    expect(await view.findByText('AE')).toBeInTheDocument();
    await fireEvent.click(view.getByRole('button', { name: 'Delete account' }));
    expect(ui.confirm).not.toBeNull();
    session.user = sessionOf({ id: '01JOTHER', email: 'jordan@example.com', name: 'Jordan Hale' }).user;
    expect(await view.findByText('JH')).toBeInTheDocument();
    // The confirmation named the previous account.
    expect(ui.confirm).toBeNull();
  });

  it('shows the field below the row and the bar with two buttons on mobile', async () => {
    setPhone(true);
    const view = show({ ...ME, email_change: OPEN });
    expect(view.getByRole('heading', { level: 1 })).toHaveTextContent('Profile');
    expect(await view.findByRole('status')).toHaveTextContent('Confirmation pending');
    await fireEvent.click(view.getByRole('button', { name: 'Edit Username' }));
    await fireEvent.input(view.getByRole('textbox', { name: 'Username' }), { target: { value: 'Alex!' } });
    const bar = view.getByRole('region', { name: 'Unsaved changes' });
    expect(within(bar).queryByText('Unsaved changes')).toBeNull();
    await fireEvent.click(within(bar).getByRole('button', { name: 'Save' }));
    expect(view.getByRole('alert')).toHaveTextContent(
      'Only lowercase letters, digits, periods, hyphens, and underscores, 2 to 32 characters.',
    );
    await fireEvent.click(within(bar).getByRole('button', { name: 'Discard' }));
    expect(view.queryByRole('alert')).toBeNull();
  });
});
