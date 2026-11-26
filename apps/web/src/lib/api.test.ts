// Taken over from Roedy Mail (`apps/web/src/lib/api.test.ts`).
import { afterEach, describe, expect, it, vi } from 'vitest';
import { ApiError, KEEPALIVE_MAX, NetworkError, api, apiBlob, isApiError } from './api.ts';

describe('api', () => {
  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it('reports missing responses as `NetworkError`, error responses as `ApiError`', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn(() => Promise.reject(new TypeError('Failed to fetch'))),
    );
    await expect(api('GET', 'auth/me')).rejects.toBeInstanceOf(NetworkError);
    vi.stubGlobal(
      'fetch',
      vi.fn(() => Promise.resolve(new Response(JSON.stringify({ error: 'unauthorized' }), { status: 401 }))),
    );
    await expect(api('GET', 'auth/me')).rejects.toBeInstanceOf(ApiError);
  });

  it('reports a body that breaks off after the headers as `NetworkError`', async () => {
    const broken = new ReadableStream({ start: (c) => c.error(new DOMException('timed out', 'TimeoutError')) });
    vi.stubGlobal(
      'fetch',
      vi.fn(() => Promise.resolve(new Response(broken, { status: 200 }))),
    );
    await expect(api('POST', 'auth/me/email/resend')).rejects.toBeInstanceOf(NetworkError);
  });

  it('returns the body as a file with `apiBlob`', async () => {
    const fetch = vi.fn<(url: string, init?: RequestInit) => Promise<Response>>(() =>
      Promise.resolve(new Response('a;b', { headers: { 'Content-Type': 'text/csv' } })),
    );
    vi.stubGlobal('fetch', fetch);
    const blob = await apiBlob('POST', 'admin/audit/export');
    expect(await blob.text()).toBe('a;b');
    expect(blob.type).toBe('text/csv');
    expect(fetch.mock.calls[0]![0]).toBe('/api/v1/admin/audit/export');
    expect(fetch.mock.calls[0]![1]).toMatchObject({
      method: 'POST',
      credentials: 'same-origin',
      headers: { 'X-Requested-With': 'fetch' },
    });
  });

  it('reports errors of `apiBlob` like `api`', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn(() => Promise.resolve(new Response('{"error":{"code":"forbidden"}}', { status: 403 }))),
    );
    await expect(apiBlob('POST', 'x')).rejects.toSatisfy((error) => isApiError(error, 403));
    vi.stubGlobal(
      'fetch',
      vi.fn(() => Promise.resolve(new Response('no JSON', { status: 502 }))),
    );
    await expect(apiBlob('POST', 'x')).rejects.toSatisfy((error) => isApiError(error, 502));
    vi.stubGlobal(
      'fetch',
      vi.fn(() => Promise.reject(new TypeError('Failed to fetch'))),
    );
    await expect(apiBlob('POST', 'x')).rejects.toBeInstanceOf(NetworkError);
  });

  it('sends keepalive only up to KEEPALIVE_MAX', async () => {
    const fetch = vi.fn<(url: string, init?: RequestInit) => Promise<Response>>(() =>
      Promise.resolve(new Response('{}')),
    );
    vi.stubGlobal('fetch', fetch);
    await api('PUT', 'admin/branding', { tilt: false }, { keepalive: true });
    await api('PUT', 'admin/branding', { image: 'x'.repeat(KEEPALIVE_MAX) }, { keepalive: true });
    await api('DELETE', 'auth/sessions', undefined, { keepalive: true });
    expect(fetch.mock.calls.map(([, init]) => init?.keepalive ?? false)).toEqual([true, false, true]);
  });
});
