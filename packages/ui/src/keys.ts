/**
 * The system's keyboard conventions: Apple systems show ⌥ (Option) where the
 * others show Alt (the shortcut Alt+Shift+P of the password fields) and ⌘
 * where the others show Ctrl; the names of keys in the shortcut lists
 * (`keyLabel`) and a shortcut of such a list (`shortcut`).
 *
 * Taken over from Roedy Mail (`apps/web/src/lib/keys.ts`): the detection
 * of Apple systems, the key event of the select lists, the modifier key and
 * the key names; without Mail's shortcuts, their groups and tabs and the
 * mapping of a key to a command, which stay with each app.
 */
import { texts } from './i18n.ts';
import type { Shortcut } from './shell.ts';

const t = texts({
  en: { ctrl: 'Ctrl', enter: 'Enter', esc: 'Esc' },
  de: { ctrl: 'Strg', enter: 'Enter', esc: 'Esc' },
});

/** What the select lists need from a key event (an app's select lists; tests without a DOM). */
export interface KeyInput {
  key: string;
  preventDefault: () => void;
}

/** Part of `navigator` for detecting the system (tests without a DOM). */
export interface PlatformInfo {
  platform?: string;
  userAgentData?: { platform?: string };
}

/**
 * Apple system (macOS, iOS, iPadOS):
 * `userAgentData.platform` "macOS" or "iOS", otherwise `navigator.platform` with
 * Mac, iPhone, iPad or iPod – iPadOS with a keyboard reports "MacIntel". If the
 * access throws (e.g. a getter of a privacy extension), no Apple system applies.
 */
export function isApple(nav: PlatformInfo | undefined = globalThis.navigator): boolean {
  try {
    const p = nav?.userAgentData?.platform;
    return p ? /^(macos|ios)$/i.test(p) : /^(mac|iphone|ipad|ipod)/i.test(nav?.platform ?? '');
  } catch {
    return false;
  }
}

/**
 * Shown modifier key of the shortcuts: "⌘"
 * on Apple systems, otherwise "Ctrl" ("Strg"). Both work on all systems.
 */
export function modKey(nav?: PlatformInfo): string {
  return isApple(nav) ? '⌘' : t.ctrl();
}

/** Keys with a name in the interface language (`ctrl`: "Ctrl", "Strg"). */
export type KeyName = 'ctrl' | 'enter' | 'esc';

/** Name of a key in the shortcut lists and the palette ("Esc", "Enter", "Ctrl"). */
export function keyLabel(name: KeyName): string {
  return t[name]();
}

/** Shortcut of the shortcut list: description and the keys in order (`shortcut('Open palette', modKey(), 'K')`). */
export function shortcut(d: string, ...keys: string[]): Shortcut {
  return { d, keys: keys.map((k) => ({ k })) };
}
