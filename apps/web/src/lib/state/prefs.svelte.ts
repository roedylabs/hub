/**
 * State module `prefs`: the account's settings with the names and values the
 * server stores (`GET`/`PUT settings`: colour scheme, accent, font size,
 * density, animations, language, region, time zone, formats, confirmations,
 * undo delay, keyboard shortcuts, developer mode; `RESET0` holds the
 * defaults, the choices come from `@roedy/ui`'s `prefs.ts`).
 *
 * After sign-in `load()` loads `GET settings`. `set()` changes locally at once
 * and sends only the changed fields with the version via `PUT settings`; at
 * most one request runs, further changes collect until then. On
 * 409 `version_conflict` the own change is applied to the supplied state and
 * goes out again. Guests (without `load()`) change only in the browser.
 *
 * `lang` is also the UI language: Paraglide reads it here
 * (`overwriteGetLocale`); guests start with the device language
 * (`deviceLang()`), a new account takes it over on the first load, together
 * with the region of the browser's language tag. While the time zone is
 * automatic, `load()` reports the device's zone when it differs, so that the
 * hub's mails and the apps' backends use it. A new region resets the four
 * formats to "Automatic" (`null`), as the server does. Every change also goes
 * to the shared preferences (`#lib/shared-prefs.ts`) that the apps read, and
 * to `@roedy/ui` (`connectSettings`).
 *
 * Taken over from Roedy Mail (`apps/web/src/lib/state/prefs.svelte.ts`) with
 * the hub's settings: Mail's own (list design, swipe gestures, external
 * content, start page, notifications) stay with Mail; without a choice the
 * colour scheme follows the system.
 */
import { ACCENTS } from '@roedy/tokens';
import {
  api,
  ApiError,
  browserLang,
  connectSettings,
  DATE_FORMATS,
  DEFAULT_PREFS,
  deviceZone,
  effectiveZone,
  isHexColor,
  isRecord,
  knownPrefs,
  MODES,
  NUMBER_FORMATS,
  regionOfTag,
  resolveFormats,
  TIME_FORMATS,
  WEEK_STARTS,
  type AccentChoice,
  type DateFormat,
  type Density,
  type FontSize,
  type Formats,
  type ModeName,
  type NumberFormat,
  type Region,
  type TimeFormat,
  type UndoDelay,
  type WeekStart,
} from '@roedy/ui';
import { deviceLang, guestPrefs, type Lang } from '#lib/locale.ts';
import { baseLocale, isLocale, overwriteGetLocale } from '#lib/paraglide/runtime.js';
import { sharedPrefsOf, trimSharedPrefs, writeSharedPrefs } from '#lib/shared-prefs.ts';
import { session } from '#lib/state/session.svelte.ts';

/** Language setting: a UI language or `system` (follows the browser, `browserLang()`). */
export type LangChoice = Lang | 'system';

export interface Settings {
  /** `null`: no choice yet, the system's color scheme applies. */
  mode: ModeName | null;
  /** `null`: the brand colour. */
  accent: AccentChoice | null;
  font_size: FontSize;
  density: Density;
  animations: boolean;
  lang: LangChoice;
  /** `null`: not set (the formats follow the language). */
  region: Region | null;
  time_zone_auto: boolean;
  /** IANA name; while `time_zone_auto` the device's, as last reported. */
  time_zone: string;
  /** `null`: automatic (the region's, without one the language's; `resolveFormats`). */
  date_format: DateFormat | null;
  time_format: TimeFormat | null;
  week_start: WeekStart | null;
  number_format: NumberFormat | null;
  confirm_delete: boolean;
  undo_delay: UndoDelay;
  shortcuts: boolean;
  developer: boolean;
}

/** The four formats; `null` in `Settings` means automatic. */
export const FORMAT_KEYS = [
  'date_format',
  'time_format',
  'week_start',
  'number_format',
] as const satisfies readonly (keyof Formats)[];

/** Defaults like the server without a stored row. */
export const RESET0: Readonly<Settings> = Object.freeze({
  mode: null,
  accent: null,
  font_size: DEFAULT_PREFS.font_size,
  density: DEFAULT_PREFS.density,
  animations: DEFAULT_PREFS.animations,
  lang: 'en',
  region: null,
  time_zone_auto: DEFAULT_PREFS.time_zone_auto,
  time_zone: DEFAULT_PREFS.time_zone,
  date_format: null,
  time_format: null,
  week_start: null,
  number_format: null,
  confirm_delete: DEFAULT_PREFS.confirm_delete,
  undo_delay: DEFAULT_PREFS.undo_delay,
  shortcuts: DEFAULT_PREFS.shortcuts,
  developer: DEFAULT_PREFS.developer,
});

/** Choices of the formats, for `knownSettings`. */
const FORMAT_CHOICES: Readonly<Record<(typeof FORMAT_KEYS)[number], readonly string[]>> = {
  date_format: DATE_FORMATS,
  time_format: TIME_FORMATS,
  week_start: WEEK_STARTS,
  number_format: NUMBER_FORMATS,
};

/** Drops values the hub does not offer (they keep the default). */
export function knownSettings(settings: Partial<Record<keyof Settings, unknown>>): Partial<Settings> {
  const { mode, accent, lang } = settings;
  // The fields with the same names and values as the shared settings of the apps.
  const shared = knownPrefs(settings);
  const known: Partial<Settings> = {};
  for (const key of [
    'font_size',
    'density',
    'animations',
    'region',
    'time_zone_auto',
    'time_zone',
    'confirm_delete',
    'undo_delay',
    'shortcuts',
    'developer',
  ] as const) {
    if (key in shared) (known as Record<string, unknown>)[key] = shared[key];
  }
  for (const key of FORMAT_KEYS) {
    const value = settings[key];
    if (value === null || (typeof value === 'string' && FORMAT_CHOICES[key].includes(value))) {
      (known as Record<string, unknown>)[key] = value;
    }
  }
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

/**
 * Defaults with the device language and what the shared preferences keep for the pages without sign-in (colour
 * scheme, font size, animations): guests, before loading, after sign-out.
 */
function fresh(): Settings {
  return { ...RESET0, ...guestPrefs(), lang: deviceLang() };
}

/** The browser's preferred language tag (`de-AT`). */
function browserTag(): string | undefined {
  if (!('navigator' in globalThis)) return undefined;
  return navigator.languages?.[0] ?? navigator.language;
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
  /** The formats that apply: the chosen ones, "Automatic" resolved (region, without one the UI language). */
  readonly formats: Formats = $derived.by(() => {
    const auto = resolveFormats(this.values.region, this.lang);
    return {
      date_format: this.values.date_format ?? auto.date_format,
      time_format: this.values.time_format ?? auto.time_format,
      week_start: this.values.week_start ?? auto.week_start,
      number_format: this.values.number_format ?? auto.number_format,
    };
  });
  /** The time zone that applies: while automatic the device's. */
  readonly timeZone: string = $derived(effectiveZone(this.values));
  /** Last error on save; the affected fields are then reset. */
  error = $state<ApiError | Error | null>(null);
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
    const patch: Partial<Settings> = {};
    if (stored.version === 0) {
      // Never saved (new account): it keeps the language it signed in with and gets the browser's region.
      if (guestLang !== this.values.lang) patch.lang = guestLang;
      const region = regionOfTag(browserTag());
      if (region && this.values.region === null) patch.region = region;
    }
    // Automatic time zone: the device's, so that mails and the apps' backends use it too.
    const zone = deviceZone();
    if (this.values.time_zone_auto && zone && zone !== this.values.time_zone) patch.time_zone = zone;
    if (Object.keys(patch).length > 0) this.set(patch);
    else this.share();
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
    // The pages without sign-in keep only what they use (colour scheme, language, font size, animations).
    trimSharedPrefs();
  }

  /** Changes settings; for signed-in accounts also on the server. A new region resets the formats to automatic. */
  set(change: Partial<Settings>): void {
    const patch: Partial<Settings> =
      'region' in change && change.region !== this.values.region
        ? { ...Object.fromEntries(FORMAT_KEYS.map((key) => [key, null])), ...change }
        : change;
    Object.assign(this.values, patch);
    this.share();
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

  /** The shared preferences for the apps and the pages without sign-in (`roedy.hub.prefs`). */
  share(): void {
    writeSharedPrefs(sharedPrefsOf(this.values, this.lang, session.branding));
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
      this.share();
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

// UI language from the settings; via `$state` templates and `$derived` follow a change. `@roedy/ui` follows the
// same language and the other settings: its dark theme, motion, undo delay, formats and time zone.
overwriteGetLocale(() => fitLang.value ?? prefs.lang);
connectSettings({
  lang: () => fitLang.value ?? prefs.lang,
  mode: () => prefs.values.mode,
  fontSize: () => prefs.values.font_size,
  density: () => prefs.values.density,
  animations: () => prefs.values.animations,
  region: () => prefs.values.region,
  timeZone: () => prefs.timeZone,
  dateFormat: () => prefs.formats.date_format,
  timeFormat: () => prefs.formats.time_format,
  weekStart: () => prefs.formats.week_start,
  numberFormat: () => prefs.formats.number_format,
  confirmDelete: () => prefs.values.confirm_delete,
  undoDelay: () => prefs.values.undo_delay,
  shortcuts: () => prefs.values.shortcuts,
  developer: () => prefs.values.developer,
});
