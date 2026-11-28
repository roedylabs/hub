import { ui } from '@roedy/ui';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { CallbackFlow, returnedOf } from '#lib/callback.svelte.ts';
import { mockApi } from '#lib/state/fetch-mock.ts';
import { signInNav, type SignInAttempt } from '#lib/stalwart-sign-in.ts';
import { accountOf } from '#lib/test/stalwart.ts';

const ATTEMPT: SignInAttempt = {
  server_id: '01JSRV1',
  email: 'alex@example.com',
  return_to: '/mail/',
  host: 'mail.example.com',
};

function flow(attempt: SignInAttempt | null = ATTEMPT) {
  const nav = { clean: vi.fn(), leave: vi.fn() };
  return { f: new CallbackFlow(nav, attempt), nav };
}

const at = (query: string) => new URL(`https://hub.example.com/oauth/callback?${query}`);

afterEach(() => {
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
  sessionStorage.clear();
  ui.reset();
});

describe('Return from the sign-in page', () => {
  it('reads state, code, error and iss', () => {
    expect(returnedOf(at('state=s&code=c&iss=https%3A%2F%2Fmail.example.com'))).toEqual({
      state: 's',
      code: 'c',
      error: null,
      iss: 'https://mail.example.com',
    });
    expect(returnedOf(at('code=c'))).toBeNull();
  });

  it('completes the sign-in and goes on to `return_to`, with a toast', async () => {
    const { f, nav } = flow();
    const api = mockApi({
      'POST stalwart/sign-in/complete': [{ status: 201, body: { account: accountOf(), return_to: '/mail/' } }],
    });
    sessionStorage.setItem('roedy-hub.stalwart-sign-in', JSON.stringify(ATTEMPT));
    await f.run(at('state=s1&code=c1&iss=https%3A%2F%2Fmail.example.com'));
    expect(nav.clean).toHaveBeenCalled();
    expect(api.calls[0]?.body).toEqual({ state: 's1', code: 'c1', iss: 'https://mail.example.com' });
    expect(api.calls[0]?.headers['X-Requested-With']).toBe('fetch');
    expect(nav.leave).toHaveBeenCalledWith('/mail/');
    expect(ui.toast?.text).toBe('alex@example.com connected');
    expect(sessionStorage.getItem('roedy-hub.stalwart-sign-in')).toBeNull();
  });

  it('says "signed in again" for an account, and never leaves the origin', async () => {
    const { f, nav } = flow({ account_id: '01JACCT1', return_to: '/account/accounts/01JACCT1', host: 'h' });
    mockApi({
      'POST stalwart/sign-in/complete': [{ body: { account: accountOf(), return_to: '//evil.example/x' } }],
    });
    await f.run(at('state=s&code=c'));
    expect(ui.toast?.text).toBe('Signed in to alex@example.com again');
    expect(nav.leave).toHaveBeenCalledWith('/account/accounts');
  });

  it('hands the server’s error over and shows it with "Try again" and "Back"', async () => {
    const { f } = flow();
    const api = mockApi({
      'POST stalwart/sign-in/complete': [
        { status: 422, body: { error: 'sign_in_failed', reason: 'access_denied', return_to: '/account/accounts/new' } },
      ],
    });
    await f.run(at('state=s&error=access_denied'));
    expect(api.calls[0]?.body).toEqual({ state: 's', error: 'access_denied' });
    expect(f.phase).toBe('error');
    expect(f.error?.title).toBe('Sign-in canceled');
    expect(f.error?.text).toBe('The sign-in at mail.example.com was canceled. Nothing was changed.');
    expect(f.returnTo).toBe('/account/accounts/new');
    expect(f.canRetry).toBe(true);
  });

  it('without `state` it reports the sign-in as unknown without asking the hub', async () => {
    const { f, nav } = flow(null);
    const api = mockApi({});
    await f.run(at('code=c'));
    expect(api.calls).toEqual([]);
    expect(f.error?.title).toBe('Sign-in not recognized');
    // Without an attempt there is nothing to try again.
    expect(f.canRetry).toBe(false);
    f.back();
    expect(nav.leave).toHaveBeenCalledWith('/account/accounts');
  });

  it('an account connected already offers "Open account"', async () => {
    const { f, nav } = flow();
    mockApi({
      'POST stalwart/sign-in/complete': [
        {
          status: 409,
          body: { error: 'dup', account: { id: '01JA', email: 'alex@example.com' }, return_to: '/mail/' },
        },
      ],
    });
    await f.run(at('state=s&code=c'));
    expect(f.error).toMatchObject({ act: 'open', account: '01JA' });
    f.openAccount();
    expect(nav.leave).toHaveBeenCalledWith('/account/accounts/01JA');
  });

  it('"Try again" starts the same sign-in anew', async () => {
    const { f } = flow();
    mockApi({
      'POST stalwart/sign-in/complete': [{ status: 410, body: { error: 'sign_in_expired', return_to: '/mail/' } }],
    });
    await f.run(at('state=s&code=c'));
    expect(f.canRetry).toBe(true);
    const open = vi.spyOn(signInNav, 'open').mockImplementation(() => {});
    const api = mockApi({
      'POST stalwart/sign-in': [
        { status: 422, body: { error: 'invalid', fields: { server_id: { code: 'no_client' } } } },
        { body: { authorize_url: 'https://mail.example.com/authorize' } },
      ],
    });
    await f.retry();
    expect(f.error?.title).toBe('Sign-in not set up');
    expect(f.canRetry).toBe(false);
    expect(f.retrying).toBe(false);
    f.code = 'sign_in_expired';
    await f.retry();
    expect(api.calls[1]?.body).toEqual({
      server_id: '01JSRV1',
      email: 'alex@example.com',
      return_to: '/mail/',
    });
    expect(open).toHaveBeenCalledWith('https://mail.example.com/authorize');
  });

  it('without a connection it says so', async () => {
    const { f } = flow();
    vi.stubGlobal(
      'fetch',
      vi.fn(() => Promise.reject(new TypeError('offline'))),
    );
    await f.run(at('state=s&code=c'));
    expect(f.error?.title).toBe('No connection');
    expect(f.canRetry).toBe(true);
  });
});
