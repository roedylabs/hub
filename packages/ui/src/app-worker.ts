/**
 * The app's own service worker from the page's view: registration at start,
 * also for guests, and detection of new builds. Every app registers its own
 * worker under its prefix (docs/plan.md § 9.2). Long-open tabs and installed
 * apps never reload by themselves, so they check for a new build regularly
 * (SvelteKit: `updated.check()`, which polls `_app/version.json`) and reload
 * after the next navigation (`releaseWaiting`).
 *
 * Taken over from Roedy Mail (`apps/web/src/lib/app-worker.ts`) without
 * `$app/env` and `$app/state`: the app passes `dev`, the script and scope of
 * its worker and SvelteKit's `updated` in, and the pages that must not reload
 * (Mail: the composer) as `keep`. The functions became methods of `AppWorker`.
 */
import { pollVisible } from './state/network.svelte.ts';

/** Interval of the check for a new build while the page is visible. */
export const RELEASE_POLL_MS = 5 * 60_000;

/** New-build detection of the framework, e.g. SvelteKit's `updated` from `$app/state`. */
export interface BuildCheck {
  /** A new build has been found. */
  readonly current: boolean;
  /** Checks for a new build now; `true` if there is one. */
  check: () => Promise<boolean>;
}

export interface AppWorkerOptions {
  /** Development (`dev` from `$app/environment`): no registration at start; Vite serves the worker as a module. */
  dev: boolean;
  /** Address of the worker's script, e.g. `/mail/service-worker.js`. */
  script: string;
  /** Scope of the worker, the app's prefix, e.g. `/mail/`. */
  scope: string;
  /** New-build detection (`updated` from `$app/state`). */
  updated: BuildCheck;
  /**
   * Pages that neither reload into nor out of on a new build (Mail: the
   * composer, where reloading would leave an empty draft); without it none.
   */
  keep?: (path: string) => boolean;
}

/**
 * The registration once it has an active worker. Not `navigator.serviceWorker.ready`: that is the registration of the
 * page's own scope, which may be another app's (or the hub's) on a page outside this worker's scope.
 */
function activated(registration: ServiceWorkerRegistration): Promise<ServiceWorkerRegistration> {
  if (registration.active) return Promise.resolve(registration);
  const worker = registration.installing ?? registration.waiting;
  if (!worker) return Promise.resolve(registration);
  return new Promise((resolve, reject) => {
    const check = () => {
      if (worker.state === 'activated') {
        worker.removeEventListener('statechange', check);
        resolve(registration);
      } else if (worker.state === 'redundant') {
        worker.removeEventListener('statechange', check);
        reject(new Error('The service worker became redundant before it activated'));
      }
    };
    worker.addEventListener('statechange', check);
  });
}

export class AppWorker {
  readonly #options: AppWorkerOptions;
  #registering: Promise<ServiceWorkerRegistration> | null = null;

  constructor(options: AppWorkerOptions) {
    this.#options = options;
  }

  /** The registration of the app's scope, created on demand; a failure can be retried. */
  register(): Promise<ServiceWorkerRegistration> {
    const { dev, script, scope } = this.#options;
    this.#registering ??= navigator.serviceWorker
      .register(script, { scope, type: dev ? 'module' : 'classic' })
      .then(activated)
      .catch((error: unknown) => {
        this.#registering = null;
        throw error;
      });
    return this.#registering;
  }

  /** At app start. Without a service worker (private window, insecure context) the app runs as before, just not offline. */
  start(): void {
    if (this.#options.dev || !('serviceWorker' in navigator)) return;
    this.register().catch(() => undefined);
  }

  /** Checks for a new build while the page is visible, on return and after an offline phase; returns the cleanup. */
  watchRelease(target: Window = window): () => void {
    return pollVisible(
      target,
      RELEASE_POLL_MS,
      () => target.document.visibilityState === 'visible',
      () => {
        this.#options.updated
          .check()
          .then((found) => (found ? this.#fetchNewWorker() : undefined))
          .catch(() => undefined);
      },
    );
  }

  /** Whether the page should reload to the new build after a navigation from `from` to `to`. */
  releaseWaiting(from: string, to: string): boolean {
    const { updated, keep } = this.#options;
    return updated.current && !keep?.(from) && !keep?.(to);
  }

  /** Fetches the new service worker right away instead of waiting for the browser's next update check. */
  async #fetchNewWorker(): Promise<void> {
    const registration = await navigator.serviceWorker?.getRegistration(this.#options.scope);
    await registration?.update();
  }
}
