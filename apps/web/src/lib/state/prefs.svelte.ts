/**
 * State module `prefs`: the account's settings with the names and values the
 * server stores (`GET`/`PUT settings`: language, color scheme, accent;
 * `RESET0` holds the defaults).
 *
 * After sign-in `load()` loads `GET settings`. `set()` changes locally at once
 * and sends only the changed fields with the version via `PUT settings`; at
 * most one request runs, further changes collect until then. On
 * 409 `version_conflict` the own change is applied to the supplied state and
 * goes out again. Guests (without `load()`) change only in the browser.
 *
 * `lang` is also the UI language: Paraglide reads it here
 * (`overwriteGetLocale`); guests start with the device language
 * (`deviceLang()`), a new account takes it over on the first load. Language
 * and color scheme also go to the shared preferences (`#lib/shared-prefs.ts`)
 * that the apps read.
 *
 * Taken over from Roedy Mail (`apps/web/src/lib/state/prefs.svelte.ts`) and
 * trimmed to the hub's three settings; without a choice the color scheme
 * follows the system.
 */
import { ACCENTS, type AccentName } from '@roedy/tokens';
import { MediaQuery } from 'svelte/reactivity';
import { ApiError, api, isRecord } from '#lib/api.ts';
import { isHexColor, type HexColor } from '#lib/color.ts';
import { browserLang, deviceLang, rememberLang, type Lang } from '#lib/locale.ts';
import { baseLocale, isLocale, overwriteGetLocale } from '#lib/paraglide/runtime.js';
import { writeSharedPrefs } from '#lib/shared-prefs.ts';

export const MODES = ['light', 'dark', 'system'] as const;
export type ModeName = (typeof MODES)[number];
/** Named accent or a custom colour as `#rrggbb` (`#lib/color.ts`). */
export type AccentChoice = AccentName | HexColor;
/** Language setting: a UI language or `system` (follows the browser, `browserLang()`). */
export type LangChoice = Lang | 'system';

export interface Settings {
  /** `null`: no choice yet, the system's color scheme applies. */
  mode: ModeName | null;
  /** `null`: the brand colour. */
  accent: AccentChoice | null;
  lang: LangChoice;
}

/** Defaults like the server without a stored row. */
export const RESET0: Readonly<Settings> = Object.freeze({ mode: null, accent: null, lang: 'en' });

/** Drops values the hub does not offer (they keep the default). */
export function knownSettings(settings: Partial<Record<keyof Settings, unknown>>): Partial<Settings> {
  const known: Partial<Settings> = {};
  const { mode, accent, lang } = settings;
  if (mode === null || MODES.includes(mode as ModeName)) known.mode = mode as ModeName | null;
  if (accent === null || Object.hasOwn(ACCENTS, accent as string) || isHexColor(accent)) {
    known.accent = accent as AccentChoice | null;
  }
  if (lang === 'system' || (typeof lang === 'string' && isLocale(lang))) known.lang = lang;
  return known;
}

/** Response of `GET`/`PUT settings` and body of 409 `version_conflict`. */
export interface StoredSettings {
  version: number;
  settings: Settings;
}

/** Attempts per change on consecutive conflicts (afterwards it counts as failed). */
const MAX_CONFLICTS = 3;

/** Defaults with the device language (guests, before loading, after sign-out). */
function fresh(): Settings {
  return { ...RESET0, lang: deviceLang() };
}

export class Prefs {
  values = $state<Settings>(fresh());
  version = $state(0);
  /** Account settings loaded; before that (and for guests) only local. */
  remote = $state(false);
  /** Loading the account finished, also failed: from here on the values apply. */
  loaded = $state(false);
  /** Language of the browser for `system`; follows its changes (`languagechange`). */
  #browserLang = $state<Lang>(browserLang());

  /** Language of the browser (what `system` shows), e.g. for "Browser: English". */
  get browserLang(): Lang {
    return this.#browserLang;
  }

  /**
   * UI language: own `$derived` so that texts are only recreated on a change, not on every new state of `values`;
   * `system` is the browser's, an unknown value counts as the base language.
   */
  readonly lang: Lang = $derived(
    this.values.lang === 'system' ? this.#browserLang : isLocale(this.values.lang) ? this.values.lang : baseLocale,
  );
  /** Last error on save; the affected fields are then reset. */
  error = $state<ApiError | Error | null>(null);
  /** System setting "reduce motion" (`prefers-reduced-motion`). */
  #reduceMotion = new MediaQuery('(prefers-reduced-motion: reduce)');
  /** Animations of this page; the hub stores no such setting (Roedy Mail's "Animations"), tests turn them off. */
  animations = $state(true);

  /** Motion on: animations and no reduced motion in the system. */
  get motion(): boolean {
    return this.animations && !this.#reduceMotion.current;
  }

  constructor() {
    globalThis.addEventListener?.('languagechange', () => {
      this.#browserLang = browserLang();
    });
  }

  /** Last state confirmed by the server. */
  #confirmed: Settings = fresh();
  /** Changed, but not sent yet. */
  #pending: Partial<Settings> = {};
  /** Currently in flight. */
  #inflight: Partial<Settings> | null = null;
  #idle: Promise<void> = Promise.resolve();
  /** Counts `reset()`; responses from an earlier generation (e.g. before sign-out) expire. */
  #generation = 0;

  /**
   * Loads the settings of the signed-in account. `signal` ends the request early (the setup's sign-in does not wait
   * longer than its deadline for settings it can do without).
   */
  async load(signal?: AbortSignal): Promise<void> {
    const generation = this.#generation;
    const guestLang = this.values.lang;
    let stored: StoredSettings;
    try {
      stored = await api<StoredSettings>('GET', 'settings', undefined, signal ? { signal } : {});
    } finally {
      if (generation === this.#generation) this.loaded = true;
    }
    if (generation !== this.#generation) return;
    // Changes queued behind a running save stay: `#accept` lays them over the loaded state and they still go out.
    this.#accept(stored);
    this.remote = true;
    // Never saved (new account): it keeps the language it signed in with.
    if (stored.version === 0 && guestLang !== this.values.lang) this.set({ lang: guestLang });
    this.#share();
  }

  /** Back to the defaults without server (e.g. after sign-out). */
  reset(): void {
    this.#generation++;
    this.#inflight = null;
    this.#idle = Promise.resolve();
    this.values = fresh();
    this.version = 0;
    this.remote = false;
    this.loaded = false;
    this.error = null;
    this.#confirmed = fresh();
    this.#pending = {};
  }

  /** Changes settings; for signed-in accounts also on the server. */
  set(patch: Partial<Settings>): void {
    Object.assign(this.values, patch);
    if ('lang' in patch || 'mode' in patch) this.#share();
    if (!this.remote) return;
    this.#pending = { ...this.#pending, ...patch };
    // A running request then takes along what was added in the meantime.
    if (!this.#inflight) this.#idle = this.#flush();
  }

  /** Waits until all changes are sent (for tests and before sign-out). */
  async settled(): Promise<void> {
    let idle: Promise<void>;
    do {
      idle = this.#idle;
      await idle;
    } while (idle !== this.#idle);
  }

  /** Language and color scheme for the pages without sign-in and for the apps. */
  #share(): void {
    rememberLang(this.values.lang);
    writeSharedPrefs({ theme: this.values.mode ?? 'system' });
  }

  async #flush(): Promise<void> {
    if (this.#inflight || Object.keys(this.#pending).length === 0) return;
    const generation = this.#generation;
    const patch = this.#pending;
    this.#inflight = patch;
    this.#pending = {};
    try {
      for (let attempt = 1; ; attempt++) {
        let stored: StoredSettings;
        try {
          stored = await api<StoredSettings>('PUT', 'settings', { version: this.version, settings: patch });
        } catch (error) {
          if (generation !== this.#generation) return;
          const conflict = conflictOf(error);
          if (!conflict) throw error;
          // Newer state (other tab or device): take it over and send the own change on top of it again.
          this.#accept(conflict);
          if (attempt >= MAX_CONFLICTS) throw error;
          continue;
        }
        if (generation !== this.#generation) return;
        this.#accept(stored);
        this.error = null;
        break;
      }
    } catch (error) {
      for (const key of Object.keys(patch) as (keyof Settings)[]) {
        if (!(key in this.#pending)) this.#assign(key, this.#confirmed[key]);
      }
      // The apps and the pages without sign-in follow the rollback too.
      if ('lang' in patch || 'mode' in patch) this.#share();
      this.error = error instanceof Error ? error : new Error(String(error));
    } finally {
      if (generation === this.#generation) this.#inflight = null;
    }
    if (generation === this.#generation && Object.keys(this.#pending).length > 0) await this.#flush();
  }

  /** Takes over a server state; own changes not yet confirmed stay visible. */
  #accept(stored: StoredSettings): void {
    this.#confirmed = { ...fresh(), ...knownSettings(stored.settings) };
    this.version = stored.version;
    this.values = { ...this.#confirmed, ...this.#inflight, ...this.#pending };
  }

  #assign<K extends keyof Settings>(key: K, value: Settings[K]): void {
    this.values[key] = value;
  }
}

function conflictOf(error: unknown): StoredSettings | null {
  if (!(error instanceof ApiError) || error.status !== 409 || error.code !== 'version_conflict') return null;
  const { version, settings } = error.body;
  return typeof version === 'number' && isRecord(settings)
    ? { version, settings: settings as unknown as Settings }
    : null;
}

export const prefs = new Prefs();

/**
 * Language the texts render in for one pass of the setup's height fit (the language not chosen), otherwise `null`.
 * Only the texts follow it; `prefs.lang` and everything derived from it stay.
 */
export const fitLang = $state<{ value: Lang | null }>({ value: null });

// UI language from the settings; via `$state` templates and `$derived` follow a change.
overwriteGetLocale(() => fitLang.value ?? prefs.lang);
