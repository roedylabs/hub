/**
 * The hub's shared settings (docs/app-contract.md › `GET /api/v1/session`,
 * `prefs`): the choices, the formats of each region, the `prefs` object the
 * hub hands to the apps and the copy in `localStorage["roedy.hub.prefs"]`,
 * and the attributes on `<html>` for font size, density and animations.
 *
 * The values are the ones the hub stores (`crate::settings` of the server);
 * the region table is the same on both sides, so that a page and the API
 * resolve "Automatic" alike. Taken over in part from Roedy Mail
 * (`apps/web/src/lib/settings-choices.ts`, `lib/state/prefs.svelte.ts`), with
 * the region's formats new in the hub.
 */
import { ACCENTS } from '@roedy/tokens';
import { isHexColor } from './color.ts';
import type { AccentChoice } from './theme.ts';
import type { Lang, ModeName } from './settings.svelte.ts';

/** Countries and regions (ISO 3166 codes). */
export const REGIONS = ['DE', 'AT', 'CH', 'FR', 'IT', 'NL', 'ES', 'GB', 'US'] as const;
export type Region = (typeof REGIONS)[number];

/** Dates in digits: `de` 25.10.2026, `dmy` 25/10/2026, `iso` 2026-10-25, `us` 10/25/2026. */
export const DATE_FORMATS = ['de', 'dmy', 'iso', 'us'] as const;
export type DateFormat = (typeof DATE_FORMATS)[number];

export const TIME_FORMATS = ['24h', '12h'] as const;
export type TimeFormat = (typeof TIME_FORMATS)[number];

export const WEEK_STARTS = ['monday', 'sunday'] as const;
export type WeekStart = (typeof WEEK_STARTS)[number];

/** Numbers: the group and decimal separators, as an example. */
export const NUMBER_FORMATS = ['1.234,56', '1,234.56', '1 234,56', '1’234.56'] as const;
export type NumberFormat = (typeof NUMBER_FORMATS)[number];

export const FONT_SIZES = ['small', 'medium', 'large'] as const;
export type FontSize = (typeof FONT_SIZES)[number];

export const DENSITIES = ['compact', 'normal', 'comfortable'] as const;
export type Density = (typeof DENSITIES)[number];

/** Undo delay: how long a toast with "Undo" stays, in seconds. */
export const UNDO_DELAYS = [3, 5, 10] as const;
export type UndoDelay = (typeof UNDO_DELAYS)[number];

/** Language choice: a language, or `system` (follows the browser). */
export const LANGUAGES = ['en', 'de', 'system'] as const;
export type LanguageChoice = (typeof LANGUAGES)[number];

/** The four formats; `null` in the settings means "Automatic" (`resolveFormats`). */
export interface Formats {
  date_format: DateFormat;
  time_format: TimeFormat;
  week_start: WeekStart;
  number_format: NumberFormat;
}

const formats = (
  date_format: DateFormat,
  time_format: TimeFormat,
  week_start: WeekStart,
  number_format: NumberFormat,
): Readonly<Formats> => Object.freeze({ date_format, time_format, week_start, number_format });

/** The usual formats of each region. */
export const REGION_FORMATS: Readonly<Record<Region, Readonly<Formats>>> = Object.freeze({
  DE: formats('de', '24h', 'monday', '1.234,56'),
  AT: formats('de', '24h', 'monday', '1.234,56'),
  CH: formats('de', '24h', 'monday', '1’234.56'),
  FR: formats('dmy', '24h', 'monday', '1 234,56'),
  IT: formats('dmy', '24h', 'monday', '1.234,56'),
  NL: formats('dmy', '24h', 'monday', '1.234,56'),
  ES: formats('dmy', '24h', 'monday', '1.234,56'),
  GB: formats('dmy', '24h', 'monday', '1,234.56'),
  US: formats('us', '12h', 'sunday', '1,234.56'),
});

/** Without a region the language decides. */
const LANG_FORMATS: Readonly<Record<Lang, Readonly<Formats>>> = Object.freeze({
  de: formats('de', '24h', 'monday', '1.234,56'),
  en: formats('iso', '24h', 'monday', '1,234.56'),
});

/** The formats "Automatic" stands for: the region's, without a region the language's. */
export function resolveFormats(region: Region | null, lang: Lang): Formats {
  return { ...(region ? REGION_FORMATS[region] : LANG_FORMATS[lang]) };
}

/** Region of a browser language tag if it is one of `REGIONS`: `de-AT` → `AT`, `de` → `null`. */
export function regionOfTag(tag: string | undefined): Region | null {
  const part = tag?.split(/[-_]/).find((p, i) => i > 0 && /^[a-z]{2}$/i.test(p));
  const code = part?.toUpperCase();
  return code && (REGIONS as readonly string[]).includes(code) ? (code as Region) : null;
}

/** Time zone of the device (IANA name); `undefined` if the browser reports none. */
export function deviceZone(): string | undefined {
  try {
    return Intl.DateTimeFormat().resolvedOptions().timeZone || undefined;
  } catch {
    return undefined;
  }
}

/** A time zone the browser knows. */
export function isZone(tz: unknown): tz is string {
  if (typeof tz !== 'string' || !tz || tz.length > 64) return false;
  try {
    new Intl.DateTimeFormat('en-US', { timeZone: tz });
    return true;
  } catch {
    return false;
  }
}

/** The zone that applies: while `time_zone_auto` the device's, otherwise the chosen one. */
export function effectiveZone(prefs: Pick<HubPrefs, 'time_zone' | 'time_zone_auto'>): string {
  return (prefs.time_zone_auto ? deviceZone() : undefined) ?? prefs.time_zone;
}

/**
 * The shared settings as the hub hands them to the apps (`prefs` of `GET /api/v1/session` and
 * `localStorage["roedy.hub.prefs"]`): formats resolved, the accent `null` while the branding sets it.
 */
export interface HubPrefs extends Formats {
  theme: ModeName;
  accent: AccentChoice | null;
  font_size: FontSize;
  density: Density;
  animations: boolean;
  language: LanguageChoice;
  region: Region | null;
  time_zone: string;
  time_zone_auto: boolean;
  confirm_delete: boolean;
  undo_delay: UndoDelay;
  shortcuts: boolean;
  developer: boolean;
}

/** Default time zone of the hub (`DEFAULT_TZ` of the server). */
export const DEFAULT_ZONE = 'Europe/Berlin';

/** The hub's defaults (formats of English without a region). */
export const DEFAULT_PREFS: Readonly<HubPrefs> = Object.freeze({
  theme: 'system',
  accent: null,
  font_size: 'medium',
  density: 'normal',
  animations: true,
  language: 'en',
  region: null,
  time_zone: DEFAULT_ZONE,
  time_zone_auto: true,
  ...resolveFormats(null, 'en'),
  confirm_delete: true,
  undo_delay: 5,
  shortcuts: true,
  developer: false,
});

const oneOf =
  <T>(values: readonly T[]) =>
  (value: unknown): value is T =>
    values.includes(value as T);
const isBool = (value: unknown): value is boolean => typeof value === 'boolean';

/** Check per field of `HubPrefs`. */
const CHECKS: { readonly [K in keyof HubPrefs]: (value: unknown) => value is HubPrefs[K] } = {
  theme: oneOf(['light', 'dark', 'system'] as const),
  accent: (value): value is AccentChoice | null =>
    value === null || (typeof value === 'string' && (Object.hasOwn(ACCENTS, value) || isHexColor(value))),
  font_size: oneOf(FONT_SIZES),
  density: oneOf(DENSITIES),
  animations: isBool,
  language: oneOf(LANGUAGES),
  region: (value): value is Region | null => value === null || oneOf(REGIONS)(value),
  time_zone: isZone,
  time_zone_auto: isBool,
  date_format: oneOf(DATE_FORMATS),
  time_format: oneOf(TIME_FORMATS),
  week_start: oneOf(WEEK_STARTS),
  number_format: oneOf(NUMBER_FORMATS),
  confirm_delete: isBool,
  undo_delay: oneOf(UNDO_DELAYS),
  shortcuts: isBool,
  developer: isBool,
};

/** The fields of `data` that are valid shared settings; unknown fields and values are left out. */
export function knownPrefs(data: unknown): Partial<HubPrefs> {
  if (typeof data !== 'object' || data === null || Array.isArray(data)) return {};
  const known: Record<string, unknown> = {};
  for (const [key, check] of Object.entries(CHECKS)) {
    const value = (data as Record<string, unknown>)[key];
    if (Object.hasOwn(data, key) && check(value)) known[key] = value;
  }
  return known;
}

/** Key of the shared settings in `localStorage` (docs/app-contract.md › Namespaces in the browser). */
export const PREFS_KEY = 'roedy.hub.prefs';

/** The browser's `localStorage`; `undefined` where the getter throws (storage blocked by a policy). */
export function browserStorage(): Storage | undefined {
  try {
    return globalThis.localStorage;
  } catch {
    return undefined;
  }
}

/** The shared settings stored on this device; broken or unknown values are left out. */
export function readHubPrefs(storage: Pick<Storage, 'getItem'> | undefined = browserStorage()): Partial<HubPrefs> {
  let raw: string | null = null;
  try {
    raw = storage?.getItem(PREFS_KEY) ?? null;
  } catch {
    // No storage (private window, blocked).
  }
  if (!raw) return {};
  try {
    return knownPrefs(JSON.parse(raw));
  } catch {
    return {};
  }
}

/** Font size, density and animations as attributes on `<html>` (`@roedy/tokens`: `data-font-size` …). */
export interface Display {
  fontSize: FontSize;
  density: Density;
  animations: boolean;
}

/** Sets the attributes of `display` on `root`, as Roedy Mail's `applyTheme()` did. */
export function applyDisplay(root: HTMLElement, display: Display): void {
  root.dataset.fontSize = display.fontSize;
  root.dataset.density = display.density;
  if (display.animations) delete root.dataset.animations;
  else root.dataset.animations = 'off';
}
