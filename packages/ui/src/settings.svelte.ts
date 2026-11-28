/**
 * The settings of the app that the package's components and helpers follow:
 * the interface language, the colour scheme and the hub's shared settings
 * (font size, density, animations, region, time zone, formats, confirmations,
 * undo delay, keyboard shortcuts, developer mode; `prefs.ts`). Each app
 * connects them once – the hub's web app its own state (`connectSettings`),
 * the other apps the hub's settings (`connectHub()`) – as getters so that
 * templates and `$derived` follow a change; without it the package uses the
 * browser's language, the system's colour scheme and the hub's defaults.
 */
import {
  DEFAULT_PREFS,
  deviceZone,
  resolveFormats,
  type DateFormat,
  type Density,
  type FontSize,
  type NumberFormat,
  type Region,
  type TimeFormat,
  type UndoDelay,
  type WeekStart,
} from './prefs.ts';

/** Languages of the package's texts; English is the base language. */
export const LANGS = ['en', 'de'] as const;
export type Lang = (typeof LANGS)[number];

/** Colour schemes; `system` and `null` (no choice) follow the operating system. */
export const MODES = ['light', 'dark', 'system'] as const;
export type ModeName = (typeof MODES)[number];

export interface UiSettings {
  /** Language of the package's texts. */
  lang: () => Lang;
  /** Colour scheme of the app (`ui.dark`). */
  mode: () => ModeName | null;
  fontSize: () => FontSize;
  density: () => Density;
  /** Animations on (`ui.motion`, together with the system's "reduce motion"). */
  animations: () => boolean;
  region: () => Region | null;
  /** The time zone that applies (IANA name): with "automatic" the device's. */
  timeZone: () => string;
  /** The formats that apply ("Automatic" resolved). */
  dateFormat: () => DateFormat;
  timeFormat: () => TimeFormat;
  weekStart: () => WeekStart;
  numberFormat: () => NumberFormat;
  /** Ask before deleting (`ui.confirmDelete`). */
  confirmDelete: () => boolean;
  /** Undo delay in seconds (`ui.undoMs`). */
  undoDelay: () => UndoDelay;
  /** Keyboard shortcuts on (`ui.shortcuts`). */
  shortcuts: () => boolean;
  /** Developer mode (`ui.developer`). */
  developer: () => boolean;
}

/** German if the browser's preferred language starts with `de`, otherwise English (also without a browser). */
export function browserLang(): Lang {
  if (!('navigator' in globalThis)) return 'en';
  const tag = navigator.languages?.[0] ?? navigator.language ?? '';
  return tag.toLowerCase().startsWith('de') ? 'de' : 'en';
}

/** Formats without a region in the connected language. */
const auto = () => resolveFormats(null, current.settings.lang());

const DEFAULTS: UiSettings = {
  lang: browserLang,
  mode: () => null,
  fontSize: () => DEFAULT_PREFS.font_size,
  density: () => DEFAULT_PREFS.density,
  animations: () => DEFAULT_PREFS.animations,
  region: () => null,
  timeZone: () => deviceZone() ?? DEFAULT_PREFS.time_zone,
  dateFormat: () => auto().date_format,
  timeFormat: () => auto().time_format,
  weekStart: () => auto().week_start,
  numberFormat: () => auto().number_format,
  confirmDelete: () => DEFAULT_PREFS.confirm_delete,
  undoDelay: () => DEFAULT_PREFS.undo_delay,
  shortcuts: () => DEFAULT_PREFS.shortcuts,
  developer: () => DEFAULT_PREFS.developer,
};

const current = $state<{ settings: UiSettings }>({ settings: DEFAULTS });

/** Connects the app's settings; missing getters keep the defaults. */
export function connectSettings(settings: Partial<UiSettings>): void {
  current.settings = { ...DEFAULTS, ...settings };
}

/** The connected settings. */
export function settings(): UiSettings {
  return current.settings;
}
