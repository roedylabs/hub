import { fireEvent, waitFor, within } from '@testing-library/svelte';
import { ui } from '@roedy/ui';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { page } from '#lib/test/page.svelte.ts';
import { renderSignedIn, resetSignedIn } from '#lib/test/signed-in.ts';
import { accountOf, serversOf } from '#lib/test/stalwart.ts';
import Page from './+page.svelte';

const nav = vi.hoisted(() => ({ goto: vi.fn(() => Promise.resolve()) }));
vi.mock('$app/navigation', () => ({ goto: nav.goto }));
vi.mock('$app/state', async () => ({ page: (await import('#lib/test/page.svelte.ts')).page }));

afterEach(() => {
  nav.goto.mockClear();
  resetSignedIn();
});

function render(account = accountOf(), extra: Record<string, { status?: number; body?: unknown }[]> = {}) {
  page.params = { id: account.id };
  return renderSignedIn(Page, {
    role: 'user',
    path: `/account/accounts/${account.id}`,
    api: {
      'GET stalwart/accounts': [{ body: { accounts: [account] } }],
      'GET stalwart/servers': [{ body: serversOf() }],
      ...extra,
    },
  });
}

describe('One Stalwart account', () => {
  it('shows state, storage, colour, default and the connection', async () => {
    const view = render(accountOf({ default: false }), {
      'PATCH stalwart/accounts/01JACCT1': [{ body: accountOf({ color: 1, default: false }) }],
    });
    expect(await view.findByRole('heading', { level: 1, name: 'alex@example.com' })).toBeInTheDocument();
    expect(view.getAllByText('Mail · mail.example.com')).toHaveLength(2);
    expect(view.getByText('Connected')).toBeInTheDocument();
    expect(view.getByRole('progressbar', { name: 'Storage' })).toHaveAttribute('aria-valuenow', '25');
    expect(view.getByText('1.5 GB free')).toBeInTheDocument();
    const colors = within(view.getByRole('radiogroup', { name: 'Color' }));
    expect(colors.getAllByRole('radio').map((r) => r.getAttribute('aria-label'))).toEqual([
      'Apricot',
      'Sage',
      'Lilac',
      'Sky',
      'Rose',
      'Honey',
    ]);
    expect(colors.getByRole('radio', { name: 'Apricot' })).toHaveAttribute('aria-checked', 'true');
    await fireEvent.click(colors.getByRole('radio', { name: 'Sage' }));
    expect(colors.getByRole('radio', { name: 'Sage' })).toHaveAttribute('aria-checked', 'true');
    expect(view.calls.at(-1)?.body).toEqual({ color: 1 });
    const facts = view.getByRole('heading', { name: 'Connection' }).parentElement!;
    expect(facts).toHaveTextContent('Sign-in At the server');
    expect(facts).toHaveTextContent('Available for Mail, Contacts');
    expect(view.getByRole('button', { name: 'Set as default' })).toBeInTheDocument();
    await fireEvent.click(view.getByRole('button', { name: 'Accounts' }));
    expect(nav.goto).toHaveBeenLastCalledWith('/account/accounts');
  });

  it('an account needing a new sign-in leads with "Sign in again"', async () => {
    const view = render(accountOf({ state: 'needs_sign_in', auth: 'token' }));
    expect(await view.findByText('Sign-in needed')).toBeInTheDocument();
    expect(view.queryByRole('button', { name: /Check/ })).toBeNull();
    const again = view.getByRole('button', { name: 'Sign in again' });
    expect(again).toHaveAttribute('aria-expanded', 'false');
    await fireEvent.click(again);
    expect(again).toHaveAttribute('aria-expanded', 'true');
    expect(view.getByLabelText('New access token')).toBeInTheDocument();
  });

  it('"Remove" asks, removes and goes back', async () => {
    const view = render(accountOf(), { 'DELETE stalwart/accounts/01JACCT1': [{ status: 204 }] });
    await fireEvent.click(await view.findByRole('button', { name: 'Remove' }));
    expect(ui.confirm?.danger).toBe(true);
    ui.confirmOk();
    await waitFor(() => expect(nav.goto).toHaveBeenLastCalledWith('/account/accounts'));
  });

  it('an unknown account shows a card', async () => {
    page.params = { id: '01JX' };
    const view = renderSignedIn(Page, {
      role: 'user',
      api: {
        'GET stalwart/accounts': [{ body: { accounts: [] } }],
        'GET stalwart/servers': [{ body: serversOf() }],
      },
    });
    expect(await view.findByText('It was removed or belongs to another user.')).toBeInTheDocument();
  });
});
