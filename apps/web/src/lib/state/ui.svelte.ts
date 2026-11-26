/**
 * State module `ui`: window size and layout, dark theme, toasts, tooltip and
 * the document title of the guest pages.
 *
 * Taken over from Roedy Mail (`apps/web/src/lib/state/ui.svelte.ts`) without
 * the shell (sidebar, menus, sheets, command palette, confirmations) and
 * without undo: the hub's toasts only inform.
 */
import { MOBILE_BP, NARROW } from '@roedy/tokens';
import { MediaQuery } from 'svelte/reactivity';
import { innerHeight, innerWidth } from 'svelte/reactivity/window';
import type { TipState } from '#lib/actions/tooltip.ts';
import { prefs } from '#lib/state/prefs.svelte.ts';
import { isDarkMode } from '#lib/theme.ts';

/** Display duration of a toast and of an error toast (as in Roedy Mail's `undo.ts`). */
export const TOAST_MS = 3200;
export const ERROR_TOAST_MS = 4600;

/** Shown toast. */
export interface Toast {
  /** Counts up per toast so that a new toast with the same text fades in again. */
  id: number;
  text: string;
  /** Error toast: warning icon, announced as `alert`. */
  error: boolean;
}

/** One toast at a time; a new one replaces the shown one. */
export class Toasts {
  #show: (toast: Toast | null) => void;
  #id = 0;
  #timer: ReturnType<typeof setTimeout> | undefined;

  constructor(show: (toast: Toast | null) => void) {
    this.#show = show;
  }

  show(text: string): void {
    this.#open(text, false, TOAST_MS);
  }

  error(text: string): void {
    this.#open(text, true, ERROR_TOAST_MS);
  }

  /** Closes the shown toast. */
  dismiss(): void {
    clearTimeout(this.#timer);
    this.#show(null);
  }

  /** Nothing waits to be sent in the hub; kept for the callers taken over from Roedy Mail. */
  flush(): Promise<void> {
    return Promise.resolve();
  }

  #open(text: string, error: boolean, ms: number): void {
    clearTimeout(this.#timer);
    this.#show({ id: ++this.#id, text, error });
    this.#timer = setTimeout(() => this.#show(null), ms);
  }
}

export class Ui {
  /** Window width; without a window (tests without jsdom) like desktop. */
  width = $derived(innerWidth.current ?? NARROW);
  /** Window height; without a window 0. */
  height = $derived(innerHeight.current ?? 0);
  isMobile = $derived(this.width < MOBILE_BP);
  /** Dark mode of the operating system (`prefers-color-scheme`), for "System". */
  readonly systemDark = new MediaQuery('(prefers-color-scheme: dark)');
  /** Dark theme from `mode` and the operating system (theme button of the guest pages, error page). */
  dark = $derived(isDarkMode(prefs.values.mode, this.systemDark.current));
  #coarse = new MediaQuery('(pointer: coarse)');
  /** Tablet: desktop layout on a coarse primary pointer (touch). */
  isTablet = $derived(!this.isMobile && this.#coarse.current);

  /**
   * Document title of the open guest page's screen ("Setup token · Setup"), set by its shell; `null` outside the
   * guest pages (setup, sign-in, password reset, invitation).
   */
  authTitle = $state<string | null>(null);
  /** The setup has signed the administrator in and opens the home page: `/setup` stays until then. */
  setupLeaving = $state(false);
  /** Already navigated within the app: "Back" on the error page then goes back in history, otherwise home. */
  navigated = $state(false);

  /** Displayed toast. */
  toast = $state<Toast | null>(null);
  readonly toasts = new Toasts((toast) => (this.toast = toast));
  /** Displayed tooltip (`use:tooltip`, `#lib/actions/tooltip.ts`). */
  tip = $state<TipState | null>(null);

  /** Discards the session state (sign-out). */
  reset(): void {
    this.authTitle = null;
    this.setupLeaving = false;
    this.toasts.dismiss();
  }

  /**
   * A previous page of the app in the history: exact with the Navigation API (same origin), otherwise whether the
   * app has already navigated.
   */
  canGoBack(): boolean {
    const navigation = (globalThis as { navigation?: { canGoBack?: unknown } }).navigation;
    return typeof navigation?.canGoBack === 'boolean' ? navigation.canGoBack : this.navigated;
  }
}

export const ui = new Ui();
