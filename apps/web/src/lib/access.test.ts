import { afterEach, describe, expect, it, vi } from 'vitest';
import { ApiError, NetworkError } from '#lib/api.ts';
import { session } from '#lib/state/session.svelte.ts';
import { lostAccess } from './access.ts';

afterEach(() => vi.restoreAllMocks());

describe('lostAccess', () => {
  it('reloads the account after 401 and 403', () => {
    const refresh = vi.spyOn(session, 'refreshUser').mockRejectedValue(new Error('offline'));
    expect(lostAccess(new ApiError(401, { error: 'unauthorized' }))).toBe(true);
    expect(lostAccess(new ApiError(403, { error: 'forbidden' }))).toBe(true);
    expect(refresh).toHaveBeenCalledTimes(2);
  });

  it('leaves other errors to the caller', () => {
    const refresh = vi.spyOn(session, 'refreshUser').mockResolvedValue();
    expect(lostAccess(new ApiError(404, { error: 'not_found' }))).toBe(false);
    expect(lostAccess(new ApiError(429, { error: 'rate_limited' }))).toBe(false);
    expect(lostAccess(new NetworkError(null))).toBe(false);
    expect(lostAccess(new Error('x'))).toBe(false);
    expect(refresh).not.toHaveBeenCalled();
  });
});
