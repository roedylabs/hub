import { fireEvent } from '@testing-library/svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { page } from '#lib/test/page.svelte.ts';
import { renderSignedIn, resetSignedIn } from '#lib/test/signed-in.ts';
import { accountOf } from '#lib/test/stalwart.ts';
import Page from './+page.svelte';

const nav = vi.hoisted(() => ({ goto: vi.fn(() => Promise.resolve()) }));
vi.mock('$app/navigation', () => ({ goto: nav.goto }));
vi.mock('$app/state', async () => ({ page: (await import('#lib/test/page.svelte.ts')).page }));

const MAIL = { slug: 'mail', name: 'Mail', description: '', url: '/mail/', icon: '/i.svg', status: 'online' };

afterEach(() => {
  nav.goto.mockClear();
  resetSignedIn();
});

describe('No account yet', () => {
  it('names the app, what is needed, and leads to "Add account" with `next`', async () => {
    const view = renderSignedIn(Page, {
      role: 'user',
      path: '/account/no-account?next=%2Fmail%2Finbox',
      api: { 'GET stalwart/accounts': [{ body: { accounts: [] } }], 'GET apps': [{ body: { apps: [MAIL] } }] },
    });
    expect(
      await view.findByRole('heading', { level: 1, name: 'Connect your first email account' }),
    ).toBeInTheDocument();
    expect(view.getByText(/^Mail works with your email account/)).toBeInTheDocument();
    expect(view.getByRole('list', { name: 'What you’ll need' })).toHaveTextContent('Your email address');
    await fireEvent.click(view.getAllByRole('button', { name: /Add account/ })[0]!);
    expect(nav.goto).toHaveBeenLastCalledWith('/account/accounts/new?next=%2Fmail%2Finbox');
  });

  it('a user with an account goes straight on to `next`', async () => {
    page.url = new URL('http://localhost/account/no-account?next=%2Fcalendar%2F');
    const replace = vi.fn();
    vi.stubGlobal('location', { ...location, replace, origin: 'http://localhost' });
    renderSignedIn(Page, {
      role: 'user',
      api: { 'GET stalwart/accounts': [{ body: { accounts: [accountOf()] } }], 'GET apps': [{ body: { apps: [] } }] },
    });
    await vi.waitFor(() => expect(replace).toHaveBeenCalledWith('/calendar/'));
  });
});
