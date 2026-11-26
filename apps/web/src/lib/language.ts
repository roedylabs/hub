/**
 * The interface languages to choose from: "System" (follows the browser,
 * `prefs.browserLang`) and the languages themselves, as cards (Settings ›
 * Language) or as the segment of the account menu (`langSeg`). A choice
 * applies at once (`prefs.set`).
 *
 * Taken over from Roedy Mail (`apps/web/src/lib/language.ts`), so far only
 * the language choices the account menu needs.
 */
import { m } from '#lib/paraglide/messages.js';
import type { Locale } from '#lib/paraglide/runtime.js';
import type { Settings } from '#lib/state/prefs.svelte.ts';

interface Lang {
  id: Locale;
  code: string;
  /** Name in the language itself ("Deutsch", "English"). */
  name: () => string;
  /** Name in the UI language; only shown if it differs from `name` ("German" under "Deutsch"). */
  sub: () => string;
}

/** Languages (`LANGS`). */
const LANGS: readonly Lang[] = [
  { id: 'de', code: 'DE', name: m.region_lang_de, sub: m.region_lang_de_sub },
  { id: 'en', code: 'EN', name: m.region_lang_en, sub: m.region_lang_en_sub },
];

/** Name of a language in itself ("Deutsch", "English"). */
function langName(lang: Locale): string {
  return lang === 'de' ? m.region_lang_de() : m.region_lang_en();
}

/** Language as a card (page) or segment (account menu, `langSeg`). */
export interface LangChoice {
  id: Settings['lang'];
  /** Code on the tile and in the segment ("DE"); empty for "System", which shows `ti-world`. */
  code: string;
  name: string;
  /**
   * Second line: the name in the UI language if it differs ("Deutsch" – "German"), for "System" always the
   * browser's language ("Browser: English"); otherwise empty.
   */
  sub: string;
  /** Name in the account menu: code and name ("EN English"), for "System" with the browser's language. */
  label: string;
  on: boolean;
  onClick: () => void;
}

/**
 * Languages with selection, "System" first (follows the browser, `browser`); a change applies at once
 * (`prefs.set`), without undo. With `system` only "System" is chosen, not the language it resolves to.
 */
export function langChoices(
  lang: Settings['lang'],
  browser: Locale,
  set: (patch: Partial<Settings>) => void,
): LangChoice[] {
  const pick = (id: Settings['lang']) => () => {
    if (id !== lang) set({ lang: id });
  };
  const browserName = langName(browser);
  return [
    {
      id: 'system',
      code: '',
      name: m.region_lang_system(),
      sub: m.region_lang_system_sub({ lang: browserName }),
      label: m.account_language_system({ lang: browserName }),
      on: lang === 'system',
      onClick: pick('system'),
    },
    ...LANGS.map((l) => {
      const name = l.name();
      const sub = l.sub();
      return {
        id: l.id,
        code: l.code,
        name,
        sub: sub === name ? '' : sub,
        label: `${l.code} ${name}`,
        on: l.id === lang,
        onClick: pick(l.id),
      };
    }),
  ];
}
