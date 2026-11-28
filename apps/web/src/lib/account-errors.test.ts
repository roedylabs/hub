import { ApiError, NetworkError } from '@roedy/ui';
import { describe, expect, it } from 'vitest';
import { accountError, checkFailed } from '#lib/account-errors.ts';

const c = { host: 'mail.example.com', email: 'alex@example.com' };
const err = (status: number, body: Record<string, unknown>) => new ApiError(status, body);

describe('Stalwart account errors', () => {
  it('puts field errors at their field', () => {
    expect(accountError(err(422, { error: 'invalid', fields: { email: { code: 'invalid' } } }), c)).toMatchObject({
      field: 'email',
      title: 'Check email address',
    });
    expect(accountError(err(422, { error: 'invalid', fields: { url: { code: 'private' } } }), c)).toMatchObject({
      field: 'url',
      title: 'Server not allowed',
    });
    expect(accountError(err(422, { error: 'invalid', fields: { url: { code: 'not_allowed' } } }), c)).toMatchObject({
      field: 'url',
      title: 'Custom servers are off',
    });
    expect(accountError(err(422, { error: 'invalid', fields: { url: { code: 'invalid' } } }), c).field).toBe('url');
    expect(accountError(err(422, { error: 'invalid', fields: { server_id: { code: 'no_client' } } }), c)).toMatchObject(
      { field: 'server', act: 'servers', title: 'Sign-in not set up' },
    );
    expect(accountError(err(422, { error: 'invalid', fields: { server_id: { code: 'not_found' } } }), c).title).toBe(
      'Server no longer available',
    );
    expect(accountError(err(422, { error: 'invalid', fields: { token: { code: 'required' } } }), c).title).toBe(
      'Access token missing',
    );
    expect(accountError(err(422, { error: 'invalid', fields: { token: { code: 'too_long' } } }), c).title).toBe(
      'Check access token',
    );
  });

  it('names the server and what to do', () => {
    expect(
      accountError(err(409, { error: 'dup', account: { id: '01JA', email: 'alex@example.com' } }), c),
    ).toMatchObject({ act: 'open', account: '01JA', text: 'alex@example.com is already connected.' });
    expect(accountError(err(409, { error: 'other_account', account: { email: 'sam@example.com' } }), c).text).toBe(
      'You signed in to mail.example.com as sam@example.com. Sign in as alex@example.com.',
    );
    expect(accountError(err(429, { error: 'rate_limited', retry_after: 90 }), c)).toMatchObject({ wait: 90 });
    expect(accountError(err(502, { error: 'sign_in_unavailable', reason: 'unreachable' }), c)).toMatchObject({
      code: 'unreachable',
      title: 'Sign-in page not available',
    });
    expect(accountError(err(409, { error: 'blocked' }), c).title).toBe('Server blocked');
    expect(accountError(err(409, { error: 'needs_sign_in' }), c).title).toBe('Sign-in needed');
    expect(accountError(err(409, { error: 'no_client' }), c).act).toBe('servers');
    expect(accountError(new NetworkError(new Error('x')), c).title).toBe('No connection');
    expect(accountError(new Error('x'), c).title).toBe('Couldn’t connect');
    expect(accountError(err(500, { error: 'internal' }), c)).toMatchObject({
      title: 'That didn’t work',
      code: 'internal',
    });
  });

  it('tells the reasons of a failed check apart, a rejected token at the field', () => {
    expect(checkFailed('auth_failed', c, true)).toMatchObject({ field: 'token', title: 'Invalid access token' });
    expect(checkFailed('auth_failed', c, false).title).toBe('Access denied');
    expect(checkFailed('unreachable', c, false).title).toBe('Server unreachable');
    expect(checkFailed('throttled', c, false).title).toBe('Too many attempts');
    expect(checkFailed('foreign_origin', c, false).title).toBe('Server misconfigured');
    expect(checkFailed('session_url_mismatch', c, false).title).toBe('Server misconfigured');
    expect(checkFailed('unsupported', c, false).title).toBe('No JMAP');
    expect(checkFailed('no_mail', c, false).title).toBe('No mailbox');
    expect(checkFailed('no_address', c, false).title).toBe('No email address');
    expect(checkFailed('weird', c, false)).toMatchObject({ code: 'weird', title: 'Couldn’t connect' });
    expect(accountError(err(422, { error: 'check_failed', reason: 'auth_failed' }), c, { token: true }).field).toBe(
      'token',
    );
  });

  it('explains the return from the sign-in page', () => {
    expect(accountError(err(400, { error: 'invalid_state' }), c).title).toBe('Sign-in not recognized');
    expect(accountError(err(409, { error: 'session_changed' }), c).title).toBe('Signed in again in the meantime');
    expect(accountError(err(410, { error: 'sign_in_expired' }), c).title).toBe('Sign-in expired');
    expect(accountError(err(422, { error: 'sign_in_failed', reason: 'access_denied' }), c).title).toBe(
      'Sign-in canceled',
    );
    expect(accountError(err(422, { error: 'sign_in_failed', reason: 'server_error' }), c)).toMatchObject({
      title: 'Sign-in not completed',
      code: 'server_error',
    });
    expect(accountError(err(400, { error: 'issuer_mismatch' }), c).title).toBe('Response from a different server');
    expect(accountError(err(409, { error: 'server_changed' }), c).title).toBe('Server changed');
    expect(
      accountError(err(422, { error: 'exchange_failed', reason: 'rejected', detail: 'unauthorized_client' }), c),
    ).toMatchObject({ code: 'unauthorized_client' });
    expect(
      accountError(err(502, { error: 'exchange_failed', reason: 'unreachable' }), { host: '', email: '' }).text,
    ).toBe(
      'The server didn’t confirm the sign-in to the hub. Try again; if it keeps happening, contact your administrator.',
    );
  });
});
