import { fireEvent, waitFor, within } from '@testing-library/svelte';
import { ui } from '@roedy/ui';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { signInNav } from '#lib/stalwart-sign-in.ts';
import { renderSignedIn, resetSignedIn, setPhone } from '#lib/test/signed-in.ts';
import { accountOf, serversOf } from '#lib/test/stalwart.ts';
import Page from './+page.svelte';
import NewPage from './new/+page.svelte';

const nav = vi.hoisted(() => ({ goto: vi.fn(() => Promise.resolve()) }));
vi.mock('$app/navigation', () => ({ goto: nav.goto }));
vi.mock('$app/state', async () => ({ page: (await import('#lib/test/page.svelte.ts')).page }));

afterEach(() => {
  nav.goto.mockClear();
  vi.restoreAllMocks();
  sessionStorage.clear();
  resetSignedIn();
});

function api(
  accounts = [
    accountOf(),
    accountOf({
      id: '01JACCT2',
      email: 'team@example.org',
      default: false,
      color: 2,
      state: 'needs_sign_in',
      quota: null,
      server: { id: '01JSRV2', label: 'Archive' },
      url: 'https://archive.example.org',
    }),
  ],
) {
  return {
    'GET stalwart/accounts': [{ body: { accounts } }],
    'GET stalwart/servers': [{ body: serversOf() }],
  };
}

describe('Account › Accounts', () => {
  it('lists the accounts with default, server, state and storage; a row opens the account', async () => {
    const view = renderSignedIn(Page, { role: 'user', api: api() });
    expect(view.getByRole('heading', { level: 1 })).toHaveTextContent('Accounts');
    const first = await view.findByRole('button', { name: /alex@example\.com/ });
    expect(first).toHaveTextContent('Default');
    expect(first).toHaveTextContent('Mail · mail.example.com');
    expect(first).toHaveTextContent('Connected');
    expect(first).toHaveTextContent('512 MB of 2 GB');
    const second = view.getByRole('button', { name: /team@example\.org/ });
    expect(second).toHaveTextContent('Sign-in needed');
    expect(second).toHaveTextContent('Not checked yet');
    await fireEvent.click(second);
    expect(nav.goto).toHaveBeenLastCalledWith('/account/accounts/01JACCT2');
    await fireEvent.click(view.getByRole('button', { name: 'Add account' }));
    expect(nav.goto).toHaveBeenLastCalledWith('/account/accounts/new');
  });

  it('without accounts says so', async () => {
    const view = renderSignedIn(Page, { role: 'user', api: api([]) });
    expect(await view.findByText('No account connected yet.')).toBeInTheDocument();
  });

  it('the form picks the server from the address and signs in there', async () => {
    const open = vi.spyOn(signInNav, 'open').mockImplementation(() => {});
    const view = renderSignedIn(NewPage, {
      role: 'user',
      path: '/account/accounts/new?next=%2Fmail%2F',
      api: {
        ...api([]),
        'POST stalwart/sign-in': [{ body: { authorize_url: 'https://mail.example.com/authorize' } }],
      },
    });
    const form = within(await view.findByRole('group', { name: 'New account' }));
    const server = form.getByRole('combobox', { name: /Server/ });
    expect(server).toHaveTextContent('Automatic');
    await fireEvent.input(form.getByRole('textbox', { name: 'Email address' }), {
      target: { value: 'sam@example.com' },
    });
    expect(server).toHaveTextContent('Automatic: Mail');
    expect(form.getByText('Server: mail.example.com')).toBeInTheDocument();
    expect(form.getByRole('radio', { name: 'At the server' })).toHaveAttribute('aria-checked', 'true');
    await fireEvent.click(form.getByRole('button', { name: 'Continue to sign-in' }));
    await waitFor(() => expect(open).toHaveBeenCalledWith('https://mail.example.com/authorize'));
    expect(view.calls.find((c) => c.path === 'stalwart/sign-in')?.body).toEqual({
      server_id: '01JSRV1',
      email: 'sam@example.com',
      return_to: '/mail/',
    });
  });

  it('the form is read-only while a request runs, and editable again after it failed', async () => {
    let answer!: (reply: { status: number; body: unknown }) => void;
    const view = renderSignedIn(NewPage, {
      role: 'user',
      path: '/account/accounts/new',
      api: {
        ...api([]),
        'POST stalwart/sign-in': [() => new Promise((resolve) => (answer = resolve))],
      },
    });
    const form = within(await view.findByRole('group', { name: 'New account' }));
    const email = form.getByRole('textbox', { name: 'Email address' });
    await fireEvent.input(email, { target: { value: 'sam@example.com' } });
    await fireEvent.click(form.getByRole('button', { name: 'Continue to sign-in' }));
    await waitFor(() => expect(email).toHaveAttribute('readonly'));
    expect(form.getByRole('radio', { name: 'Access token' })).toBeDisabled();
    answer({ status: 502, body: { error: 'sign_in_unavailable', reason: 'unreachable' } });
    await waitFor(() => expect(email).not.toHaveAttribute('readonly'));
    expect(form.getByRole('radio', { name: 'Access token' })).toBeEnabled();
  });

  it('shows errors in the form and marks the field', async () => {
    const view = renderSignedIn(NewPage, {
      role: 'user',
      path: '/account/accounts/new',
      api: {
        ...api([]),
        'POST stalwart/accounts': [{ status: 422, body: { error: 'check_failed', reason: 'auth_failed' } }],
      },
    });
    const form = within(await view.findByRole('group', { name: 'New account' }));
    await fireEvent.click(form.getByRole('button', { name: 'Continue to sign-in' }));
    expect(form.getByRole('alert')).toHaveTextContent('Check email address');
    const email = form.getByRole('textbox', { name: 'Email address' });
    expect(email).toHaveAttribute('aria-invalid', 'true');
    // The archive has no OAuth client: access tokens only.
    await fireEvent.input(email, { target: { value: 'sam@example.org' } });
    expect(form.queryByRole('radiogroup')).toBeNull();
    expect(form.getByText(/doesn’t offer the sign-in here yet/)).toBeInTheDocument();
    await fireEvent.input(form.getByLabelText('Access token'), { target: { value: 'bad' } });
    await fireEvent.click(form.getByRole('button', { name: 'Connect' }));
    expect(await form.findByRole('alert')).toHaveTextContent(
      'archive.example.org rejected the access token. Check it or create a new one.',
    );
    expect(form.getByLabelText('Access token')).toHaveAttribute('aria-invalid', 'true');
    // Escape cancels.
    await fireEvent.keyDown(email, { key: 'Escape' });
    expect(nav.goto).toHaveBeenLastCalledWith('/account/accounts');
  });

  it('on mobile the server picker opens the sheet', async () => {
    setPhone(true);
    const view = renderSignedIn(NewPage, { role: 'user', path: '/account/accounts/new', api: api([]) });
    const form = within(await view.findByRole('group', { name: 'New account' }));
    await fireEvent.click(form.getByRole('button', { name: /Server/ }));
    expect(ui.menu).toBe('pick');
    expect(ui.pick?.().opts.map((o) => o.label)).toEqual(['Automatic', 'Mail', 'Archive']);
  });
});
