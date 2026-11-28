/**
 * Language of the interface: English (the base language) or German. When signed
 * in, `lang` of the settings (`prefs`) applies; guests get the language last used
 * on this device, otherwise the first matching one of the browser, otherwise
 * English. Paraglide reads the language via `overwriteGetLocale` from `prefs`
 * (see there), so that templates and `$derived` follow a change without a reload.
 *
 * The language last used is kept in the shared preferences
 * (`#lib/shared-prefs.ts`, `language`), which the apps read as well; so are
 * colour scheme, font size and animations for the pages without sign-in
 * (`guestPrefs()`).
 *
 * Date and number formats in words follow the language (`intlLocale()`):
 * English uses `en-GB` – day before month ("Tue 10 November") and time zone
 * names like "CET" instead of "GMT+1"; the texts themselves are written in
 * American English. Date format in digits, time and week start stay settings
 * (Roedy Mail’s `#lib/date.ts`).
 *
 * Taken over from Roedy Mail (`apps/web/src/lib/locale.ts`).
 */
import { baseLocale, getLocale, isLocale, type Locale } from '#lib/paraglide/runtime.js';
import { readSharedPrefs } from '#lib/shared-prefs.ts';
import type { Settings } from '#lib/state/prefs.svelte.ts';

export type Lang = Locale;

/** Language of the device: remembered, otherwise the first matching one of the browser, otherwise the base language. */
export function deviceLang(): Lang {
  // Without a document (tests under Node, service worker) the base language applies.
  if (!('document' in globalThis)) return baseLocale;
  const stored = readSharedPrefs().language;
  if (stored && stored !== 'system' && isLocale(stored)) return stored;
  const tags = navigator.languages?.length ? navigator.languages : [navigator.language];
  for (const tag of tags) {
    const base = tag?.toLowerCase().split('-')[0];
    if (base && isLocale(base)) return base;
  }
  return baseLocale;
}

/** What the pages without sign-in take over from the shared preferences: colour scheme, font size, animations. */
export function guestPrefs(): Partial<Pick<Settings, 'mode' | 'font_size' | 'animations'>> {
  if (!('document' in globalThis)) return {};
  const { theme, font_size, animations } = readSharedPrefs();
  const out: Partial<Pick<Settings, 'mode' | 'font_size' | 'animations'>> = {};
  if (theme && theme !== 'system') out.mode = theme;
  if (font_size) out.font_size = font_size;
  if (animations !== undefined) out.animations = animations;
  return out;
}

/** Language tag for `Intl` (date in words, numbers, time zone abbreviations). */
export function intlLocale(lang: Lang = getLocale()): string {
  return lang === 'en' ? 'en-GB' : 'de-DE';
}
