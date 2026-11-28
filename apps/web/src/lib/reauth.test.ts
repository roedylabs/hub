import { describe, expect, it } from 'vitest';
import { ApiError, NetworkError } from '@roedy/ui';
import { needsReauth, reauthError } from './reauth.ts';

const err = (status: number, body: Record<string, unknown>) => new ApiError(status, body);
const invalid = (fields: Record<string, unknown>) => err(422, { error: 'invalid', fields });

describe('reauthError', () => {
  it('clears the password after a wrong password', () => {
    expect(reauthError(err(422, { error: 'invalid_password' }))).toEqual({
      text: 'The password is incorrect.',
      clear: 'password',
    });
    expect(reauthError(invalid({ password: { code: 'too_long' } }))).toEqual({
      text: 'The password is incorrect.',
      clear: 'password',
    });
    expect(reauthError(invalid({ current_password: { code: 'too_long' } }), false, 'current_password')?.clear).toBe(
      'password',
    );
  });

  it('clears the code after a wrong, used or malformed code', () => {
    expect(reauthError(err(422, { error: 'invalid_code' }))).toEqual({
      text: 'That code isn’t right. Enter the current code from your authenticator app.',
      clear: 'code',
    });
    expect(reauthError(err(422, { error: 'invalid_code' }), true)).toEqual({
      text: 'That code isn’t right. Enter the current code from the new app.',
      clear: 'code',
    });
    expect(reauthError(err(422, { error: 'code_used' }))).toEqual({
      text: 'This code has already been used. Wait for the next code in your app.',
      clear: 'code',
    });
    expect(reauthError(invalid({ code: { code: 'invalid' } }))).toEqual({
      text: 'The code must be 6 digits.',
      clear: 'code',
    });
  });

  it('keeps the inputs during a lockout', () => {
    expect(reauthError(err(429, { error: 'rate_limited', retry_after: 30 }))).toEqual({
      text: 'Too many failed attempts. Try again in 30 seconds.',
      clear: null,
    });
  });

  it('leaves other errors to the page', () => {
    expect(reauthError(new Error('x'))).toBeNull();
    expect(reauthError(new NetworkError(null))).toBeNull();
    expect(reauthError(invalid({ email: { code: 'invalid' } }))).toBeNull();
    expect(reauthError(invalid({ new_password: { code: 'policy' } }))).toBeNull();
    expect(reauthError(err(404, { error: 'not_found' }))).toBeNull();
    expect(reauthError(err(502, { error: 'mail_failed' }))).toBeNull();
  });
});

describe('needsReauth', () => {
  it('recognises the request for a fresh sign-in', () => {
    expect(needsReauth(err(403, { error: 'reauth_required' }))).toBe(true);
    expect(needsReauth(err(401, { error: 'unauthorized' }))).toBe(false);
    expect(needsReauth(new Error('reauth_required'))).toBe(false);
  });
});
