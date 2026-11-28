// The return from the sign-in page end to end in the browser: address in, `POST stalwart/sign-in/complete`, on to
// `return_to` – or the error with its actions.
import { fireEvent } from '@testing-library/svelte';
import { ui } from '@roedy/ui';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { guest } from '#lib/state/guest.svelte.ts';
import { signInNav } from '#lib/stalwart-sign-in.ts';
import { renderSignedIn, resetSignedIn } from '#lib/test/signed-in.ts';
import { accountOf } from '#lib/test/stalwart.ts';
import Page from './+page.svelte';

const nav = vi.hoisted(() => ({ goto: vi.fn(() => Promise.resolve()), replaceState: vi.fn(), afterNavigate: vi.fn() }));
vi.mock('$app/navigation', () => nav);

const ATTEMPT = {
  server_id: '01JSRV1',
  email: 'alex@example.com',
  return_to: '/account/accounts',
  host: 'mail.example.com',
};

function at(query: string) {
  history.replaceState(null, '', `/oauth/callback?${query}`);
  sessionStorage.setItem('roedy-hub.stalwart-sign-in', JSON.stringify(ATTEMPT));
}

afterEach(() => {
  nav.goto.mockClear();
  nav.replaceState.mockClear();
  vi.restoreAllMocks();
  sessionStorage.clear();
  history.replaceState(null, '', '/');
  resetSignedIn();
});

describe('Return from the sign-in page', () => {
  it('finishes the sign-in, removes code and state from the address and goes back with a toast', async () => {
    at('state=s1&code=c1');
    let finish: (reply: { status: number; body: unknown }) => void = () => {};
    const view = renderSignedIn(Page, {
      role: 'user',
      api: {
        'POST stalwart/sign-in/complete': [() => new Promise((resolve) => (finish = resolve))],
      },
    });
    expect(nav.replaceState).toHaveBeenCalledWith('/oauth/callback', {});
    expect(view.getByRole('heading', { level: 1 })).toHaveTextContent('Finishing sign-in');
    expect(view.getByText('Signed in at mail.example.com. The hub is connecting the account.')).toBeInTheDocument();
    expect(guest.title).toBe('Sign-in – Example Org');
    finish({ status: 201, body: { account: accountOf(), return_to: '/account/accounts' } });
    await vi.waitFor(() => expect(nav.goto).toHaveBeenCalledWith('/account/accounts', { replace: true }));
    expect(view.calls[0]?.body).toEqual({ state: 's1', code: 'c1' });
    expect(ui.toast?.text).toBe('alex@example.com connected');
  });

  it('shows an error with "Try again" and "Back"', async () => {
    at('state=s1&code=c1');
    const open = vi.spyOn(signInNav, 'open').mockImplementation(() => {});
    const view = renderSignedIn(Page, {
      role: 'user',
      api: {
        'POST stalwart/sign-in/complete': [
          {
            status: 422,
            body: { error: 'exchange_failed', reason: 'invalid_grant', return_to: '/account/accounts/new' },
          },
        ],
        'POST stalwart/sign-in': [{ body: { authorize_url: 'https://mail.example.com/authorize' } }],
      },
    });
    expect(await view.findByRole('heading', { level: 1, name: 'Sign-in not completed' })).toBeInTheDocument();
    expect(view.getByText(/mail\.example\.com didn’t confirm the sign-in/)).toBeInTheDocument();
    expect(view.getByText('Server message: invalid_grant')).toBeInTheDocument();
    const back = view.getAllByRole('button', { name: 'Back' })[0]!;
    await fireEvent.click(back);
    expect(nav.goto).toHaveBeenLastCalledWith('/account/accounts/new', { replace: true });
    await fireEvent.click(view.getAllByRole('button', { name: 'Try again' })[0]!);
    await vi.waitFor(() => expect(open).toHaveBeenCalledWith('https://mail.example.com/authorize'));
  });

  it('the server’s refusal is handed over as it is', async () => {
    at('state=s1&error=access_denied&error_description=x');
    const view = renderSignedIn(Page, {
      role: 'user',
      api: {
        'POST stalwart/sign-in/complete': [
          { status: 422, body: { error: 'sign_in_failed', reason: 'access_denied', return_to: '/account/accounts' } },
        ],
      },
    });
    expect(await view.findByRole('heading', { level: 1, name: 'Sign-in canceled' })).toBeInTheDocument();
    expect(view.calls[0]?.body).toEqual({ state: 's1', error: 'access_denied' });
  });
});
