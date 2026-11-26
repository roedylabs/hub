/**
 * The system's keyboard conventions: Apple systems show ⌥ (Option) where the
 * others show Alt (the shortcut Alt+Shift+P of the password fields).
 *
 * Taken over from Roedy Mail (`apps/web/src/lib/keys.ts`): only the detection
 * of Apple systems and the key event of the select lists, without the
 * shortcuts of the shell.
 */

/** What the select lists need from a key event (`#lib/select.ts`; tests without a DOM). */
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
