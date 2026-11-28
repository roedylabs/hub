/**
 * The settings of the app that the package's components follow: the
 * interface language and the colour scheme. Each app connects its own state
 * once (`connectSettings`), as getters so that templates and `$derived`
 * follow a change; without it the package uses the browser's language and
 * the system's colour scheme.
 */

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
}

/** German if the browser's preferred language starts with `de`, otherwise English (also without a browser). */
export function browserLang(): Lang {
  if (!('navigator' in globalThis)) return 'en';
  const tag = navigator.languages?.[0] ?? navigator.language ?? '';
  return tag.toLowerCase().startsWith('de') ? 'de' : 'en';
}

const DEFAULTS: UiSettings = { lang: browserLang, mode: () => null };

const current = $state<{ settings: UiSettings }>({ settings: DEFAULTS });

/** Connects the app's settings; missing getters keep the defaults. */
export function connectSettings(settings: Partial<UiSettings>): void {
  current.settings = { ...DEFAULTS, ...settings };
}

/** The connected settings. */
export function settings(): UiSettings {
  return current.settings;
}
