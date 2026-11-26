import { afterEach, describe, expect, it, vi } from 'vitest';
import { ApiError, NetworkError } from '#lib/api.ts';
import { session } from '#lib/state/session.svelte.ts';
import { ui } from '#lib/state/ui.svelte.ts';
import { lostAccess } from './access.ts';

afterEach(() => {
  vi.restoreAllMocks();
  ui.reset();
});

describe('lostAccess', () => {
  it('reloads the account after 401 and 403', () => {
    const refresh = vi.spyOn(session, 'refreshUser').mockRejectedValue(new Error('offline'));
    expect(lostAccess(new ApiError(401, { error: 'unauthorized' }))).toBe(true);
    expect(lostAccess(new ApiError(403, { error: 'forbidden' }))).toBe(true);
    expect(refresh).toHaveBeenCalledTimes(2);
  });

  it('says so when the account cannot be reloaded', async () => {
    vi.spyOn(session, 'refreshUser').mockRejectedValue(new NetworkError(null));
    expect(lostAccess(new ApiError(403, { error: 'forbidden' }))).toBe(true);
    await vi.waitFor(() => expect(ui.toast).toMatchObject({ text: 'No connection. Please try again.', error: true }));
  });

  it('stays quiet when the account was reloaded', async () => {
    vi.spyOn(session, 'refreshUser').mockResolvedValue();
    expect(lostAccess(new ApiError(403, { error: 'forbidden' }))).toBe(true);
    await Promise.resolve();
    await Promise.resolve();
    expect(ui.toast).toBeNull();
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
