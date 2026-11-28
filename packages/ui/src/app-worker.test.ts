// Taken over from Roedy Mail (`apps/web/src/lib/app-worker.test.ts`) with `dev`, script, scope, `updated` and the
// pages that do not reload passed in instead of mocking `$app/env` and `$app/state`.
import { afterEach, describe, expect, it, vi } from 'vitest';
import { AppWorker, type AppWorkerOptions } from './app-worker.ts';

/** `navigator.serviceWorker` as a stub; without it the browser has none. */
function stubWorker() {
  const registration = { update: vi.fn(() => Promise.resolve()) };
  const container = {
    register: vi.fn(() => Promise.resolve(registration)),
    ready: Promise.resolve(registration),
    getRegistration: vi.fn(() => Promise.resolve(registration)),
  };
  Object.defineProperty(navigator, 'serviceWorker', { configurable: true, value: container });
  return { container, registration };
}

function worker(options: Partial<AppWorkerOptions> = {}) {
  const updated = { current: false, check: vi.fn(() => Promise.resolve(false)) };
  const app = new AppWorker({ dev: false, script: '/mail/service-worker.js', scope: '/mail/', updated, ...options });
  return { app, updated };
}

afterEach(() => {
  Reflect.deleteProperty(navigator, 'serviceWorker');
});

describe('service worker of the app', () => {
  it('registers once at start under its prefix, even for several callers', async () => {
    const { container } = stubWorker();
    const { app } = worker();
    app.start();
    await app.register();
    expect(container.register).toHaveBeenCalledOnce();
    expect(container.register).toHaveBeenCalledWith('/mail/service-worker.js', { scope: '/mail/', type: 'classic' });
  });

  it('registers a module worker in development on demand', async () => {
    const { container } = stubWorker();
    await worker({ dev: true }).app.register();
    expect(container.register).toHaveBeenCalledWith('/mail/service-worker.js', { scope: '/mail/', type: 'module' });
  });

  it('tries again after a failure', async () => {
    const { container } = stubWorker();
    container.register.mockRejectedValueOnce(new Error('blocked'));
    const { app } = worker();
    await expect(app.register()).rejects.toThrow('blocked');
    await expect(app.register()).resolves.toBeDefined();
  });

  it('does not start in development and keeps running without a service worker', () => {
    expect(() => worker().app.start()).not.toThrow();
    const { container } = stubWorker();
    worker({ dev: true }).app.start();
    expect(container.register).not.toHaveBeenCalled();
  });

  it('fetches the new worker of its scope as soon as a new build appears', async () => {
    const { container, registration } = stubWorker();
    const { app, updated } = worker();
    const stop = app.watchRelease(window);
    document.dispatchEvent(new Event('visibilitychange'));
    await vi.waitFor(() => expect(updated.check).toHaveBeenCalledOnce());
    expect(registration.update).not.toHaveBeenCalled();

    updated.check.mockResolvedValue(true);
    document.dispatchEvent(new Event('visibilitychange'));
    await vi.waitFor(() => expect(registration.update).toHaveBeenCalledOnce());
    expect(container.getRegistration).toHaveBeenCalledWith('/mail/');

    stop();
    document.dispatchEvent(new Event('visibilitychange'));
    expect(updated.check).toHaveBeenCalledTimes(2);
  });

  it('stays quiet when the check fails', async () => {
    stubWorker();
    const { app, updated } = worker();
    updated.check.mockRejectedValue(new TypeError('offline'));
    const stop = app.watchRelease(window);
    document.dispatchEvent(new Event('visibilitychange'));
    await vi.waitFor(() => expect(updated.check).toHaveBeenCalled());
    stop();
  });

  it('reloads only with a new build and not into or out of the pages it keeps', () => {
    const { app, updated } = worker({ keep: (path) => path.startsWith('/mail/compose/') });
    expect(app.releaseWaiting('/mail/start', '/mail/inbox')).toBe(false);
    updated.current = true;
    expect(app.releaseWaiting('/mail/start', '/mail/inbox')).toBe(true);
    expect(app.releaseWaiting('/mail/start', '/mail/compose/new')).toBe(false);
    expect(app.releaseWaiting('/mail/compose/new', '/mail/start')).toBe(false);
    // Without `keep` every page reloads.
    const plain = worker();
    plain.updated.current = true;
    expect(plain.app.releaseWaiting('/mail/compose/new', '/mail/start')).toBe(true);
  });
});
