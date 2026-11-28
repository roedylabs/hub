/**
 * State module `ui`: window size and layout, dark theme and motion, the open
 * menu or sheet of the shell, a page's select list and dialog, the
 * confirmation, delayed loading indicators (skeletons), toasts and the
 * tooltip. The package's components read it; the app's shell sets it.
 *
 * Taken over from the hub's web app, which took it over from Roedy Mail
 * (`apps/web/src/lib/state/ui.svelte.ts`) without the sidebar, splash screen,
 * command palette and keyboard shortcuts, and without undo. There is no "Ask
 * before deleting" setting: `ask()` always asks.
 */
import { MOBILE_BP, NARROW } from '@roedy/tokens';
import type { Snippet } from 'svelte';
import { MediaQuery } from 'svelte/reactivity';
import { innerHeight, innerWidth } from 'svelte/reactivity/window';
import type { TipState } from '../actions/tooltip.ts';
import type { IconName } from '../icons.ts';
import type { PickerValues } from '../shell.ts';
import { settings } from '../settings.svelte.ts';
import { isDarkMode } from '../theme.ts';

/** Display duration of a toast and of an error toast (as in Roedy Mail's `undo.ts`). */
export const TOAST_MS = 3200;
export const ERROR_TOAST_MS = 4600;

/** Shown toast. */
export interface ToastItem {
  /** Counts up per toast so that a new toast with the same text fades in again. */
  id: number;
  text: string;
  /** Error toast: warning icon, announced as `alert`. */
  error: boolean;
}

/**
 * Open menu or sheet, named by the app: e.g. `account` for the account menu (a popover below the avatar on the
 * desktop, a sheet on mobile), `pick` for a page's select list (a popover on the desktop, on mobile a selection
 * sheet with `ui.pick`).
 */
export type MenuKey = string;

/** Delay after which a skeleton appears when there is no data in memory. */
export const SKELETON_DELAY = 150;

/** Request for the confirmation dialog (`ask()`). */
export interface ConfirmRequest {
  title: string;
  text: string;
  /** Icon of the tile; without it `ti-help-circle`. */
  icon?: IconName;
  /** Button label; without it "Confirm". */
  ok?: string;
  /** Red instead of dark (delete, remove). */
  danger?: boolean;
  /** Word that must be typed to confirm (in capitals). */
  word?: string;
  /** Without "Cancel"; then Escape and the backdrop do not close either. */
  noCancel?: boolean;
  run: () => void;
}

/**
 * Dialog of a page that the layout shows above the shell (e.g. the colour editor of Settings › Appearance): on
 * the desktop `content(false)` renders backdrop and dialog itself, on mobile `content(true)` sits in the sheet
 * with `title` – or draws its own sheet (`bare`). Escape closes via `close`.
 */
export interface PageDialog {
  title: string;
  close: () => void;
  content: Snippet<[boolean]>;
  /** Mobile: the content draws its own sheet (colour editor), without the layout's. */
  bare?: boolean;
}

/** One toast at a time; a new one replaces the shown one. */
export class Toasts {
  #show: (toast: ToastItem | null) => void;
  #id = 0;
  #timer: ReturnType<typeof setTimeout> | undefined;

  constructor(show: (toast: ToastItem | null) => void) {
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
  /** Dark theme from the app's colour scheme (`connectSettings`) and the operating system. */
  dark = $derived(isDarkMode(settings().mode(), this.systemDark.current));
  /** System setting "reduce motion" (`prefers-reduced-motion`). */
  #reduceMotion = new MediaQuery('(prefers-reduced-motion: reduce)');
  /** Animations of this page: an app's setting (Roedy Mail's "Animations") turns them off, as do tests. */
  animations = $state(true);

  /** Motion on: animations and no reduced motion in the system. */
  get motion(): boolean {
    return this.animations && !this.#reduceMotion.current;
  }
  #coarse = new MediaQuery('(pointer: coarse)');
  /** Tablet: desktop layout on a coarse primary pointer (touch). */
  isTablet = $derived(!this.isMobile && this.#coarse.current);
  /** Dragging sheets and dialogs down to close them: on mobile and tablet. */
  swipeOn = $derived(this.isMobile || this.isTablet);

  /** Open menu or sheet of the shell (`MenuKey`). */
  menu = $state<MenuKey | null>(null);
  /** Values of the open select list (`pick`) for the mobile sheet; the page sets them. */
  pick = $state.raw<(() => PickerValues) | null>(null);
  /**
   * Open dialog of the page (`null` while none is open); the page sets the function and clears it on leaving,
   * the layout shows it above everything (desktop) or as a sheet (mobile).
   */
  pageDialog = $state.raw<(() => PageDialog | null) | null>(null);
  /** Open confirmation with the typed word (`confirm`, `confirmIn`). */
  confirm = $state<ConfirmRequest | null>(null);
  confirmIn = $state('');

  /** Already navigated within the app: "Back" on the error page then goes back in history, otherwise home. */
  navigated = $state(false);

  /** Displayed toast. */
  toast = $state<ToastItem | null>(null);
  readonly toasts = new Toasts((toast) => (this.toast = toast));
  /** Displayed tooltip (`use:tooltip`, `actions/tooltip.ts`). */
  tip = $state<TipState | null>(null);

  toggleMenu(key: MenuKey): void {
    this.menu = this.menu === key ? null : key;
  }

  closeMenu(): void {
    this.menu = null;
  }

  /** Requests confirmation before `run`; closes an open menu. */
  ask(request: ConfirmRequest): void {
    this.confirm = request;
    this.confirmIn = '';
    this.menu = null;
  }

  /** Typed word matches, case-insensitively (or none is required). */
  get confirmReady(): boolean {
    const word = this.confirm?.word;
    return !!this.confirm && (!word || this.confirmIn.trim().toLocaleUpperCase('de') === word.toLocaleUpperCase('de'));
  }

  confirmOk(): void {
    const request = this.confirm;
    if (!request || !this.confirmReady) return;
    this.confirm = null;
    this.confirmIn = '';
    request.run();
  }

  confirmCancel(): void {
    if (!this.confirm || this.confirm.noCancel) return;
    this.confirm = null;
    this.confirmIn = '';
  }

  /** Discards the session state (sign-out): menus, dialogs, confirmation, toast. */
  reset(): void {
    this.menu = null;
    this.pick = null;
    this.pageDialog = null;
    this.confirm = null;
    this.confirmIn = '';
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

/**
 * Loading indicator that only appears after `SKELETON_DELAY`: `slow` becomes `true`
 * if the request takes longer, and `false` again after it ends.
 */
export class Loading {
  slow = $state(false);
  #running = 0;

  async run<T>(work: Promise<T>): Promise<T> {
    this.#running++;
    const timer = setTimeout(() => (this.slow = true), SKELETON_DELAY);
    try {
      return await work;
    } finally {
      clearTimeout(timer);
      if (--this.#running === 0) this.slow = false;
    }
  }
}

export const ui = new Ui();
