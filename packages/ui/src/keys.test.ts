// Taken over from Roedy Mail (`apps/web/src/lib/keys.test.ts`): only the modifier key and the detection of Apple
// systems, plus the key names and `shortcut`.
import { afterEach, describe, expect, it, vi } from 'vitest';
import { isApple, keyLabel, modKey, shortcut, type PlatformInfo } from './keys.ts';
import { connectSettings } from './settings.svelte.ts';

afterEach(() => {
  vi.unstubAllGlobals();
  connectSettings({});
});

describe('keys', () => {
  it('shows the modifier key per system: ⌘ on macOS, iOS and iPadOS, otherwise Ctrl', () => {
    // First `userAgentData.platform`, otherwise `navigator.platform`.
    expect(modKey({ userAgentData: { platform: 'macOS' }, platform: 'MacIntel' })).toBe('⌘');
    expect(modKey({ userAgentData: { platform: 'iOS' } })).toBe('⌘');
    expect(modKey({ userAgentData: { platform: 'Windows' }, platform: 'MacIntel' })).toBe('Ctrl');
    expect(modKey({ userAgentData: { platform: 'Chrome OS' } })).toBe('Ctrl');
    expect(modKey({ userAgentData: { platform: '' }, platform: 'iPhone' })).toBe('⌘');
    // iPadOS with a keyboard reports "MacIntel"; Android reports "Linux …".
    for (const platform of ['MacIntel', 'Macintosh', 'iPad', 'iPhone', 'iPod touch']) {
      expect(isApple({ platform })).toBe(true);
    }
    for (const platform of ['Win32', 'Linux x86_64', 'Linux armv81', '']) expect(isApple({ platform })).toBe(false);
    expect(isApple({})).toBe(false);
    const throwing = Object.defineProperty({}, 'platform', {
      get() {
        throw new Error('blocked');
      },
    }) as PlatformInfo;
    expect(modKey(throwing)).toBe('Ctrl');

    // Without an argument the browser's `navigator`.
    vi.stubGlobal('navigator', { platform: 'MacIntel' });
    expect(modKey()).toBe('⌘');
  });

  it('names the keys in the interface language', () => {
    expect([keyLabel('ctrl'), keyLabel('enter'), keyLabel('esc')]).toEqual(['Ctrl', 'Enter', 'Esc']);
    connectSettings({ lang: () => 'de' });
    expect([keyLabel('ctrl'), keyLabel('enter'), keyLabel('esc')]).toEqual(['Strg', 'Enter', 'Esc']);
    expect(modKey({ platform: 'Win32' })).toBe('Strg');
  });

  it('builds a shortcut from its description and keys', () => {
    expect(shortcut('Open command palette', '⌘', 'K')).toEqual({
      d: 'Open command palette',
      keys: [{ k: '⌘' }, { k: 'K' }],
    });
    expect(shortcut('Nothing')).toEqual({ d: 'Nothing', keys: [] });
  });
});
