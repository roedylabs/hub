import { afterEach, describe, expect, it, vi } from 'vitest';
import { BRAND0, DEFAULT_PREFS } from '@roedy/ui';
import { readSharedPrefs, sharedPrefsOf, trimSharedPrefs, writeSharedPrefs } from '#lib/shared-prefs.ts';
import { RESET0 } from '#lib/state/prefs.svelte.ts';

/** The key the apps read (docs/app-contract.md), written out so that a rename of the constant fails here. */
const KEY = 'roedy.hub.prefs';

afterEach(() => {
  vi.unstubAllGlobals();
  localStorage.clear();
});

describe('shared preferences (docs/app-contract.md › Namespaces in the browser)', () => {
  it('hands the apps the settings with resolved formats and the accent they may use', () => {
    expect(sharedPrefsOf(RESET0, 'en', BRAND0)).toEqual(DEFAULT_PREFS);
    const values = {
      ...RESET0,
      mode: 'dark',
      accent: 'rose',
      lang: 'system',
      region: 'AT',
      time_format: '12h',
    } as const;
    expect(sharedPrefsOf(values, 'de', BRAND0)).toMatchObject({
      theme: 'dark',
      accent: 'rose',
      language: 'system',
      region: 'AT',
      date_format: 'de',
      time_format: '12h',
      week_start: 'monday',
      number_format: '1.234,56',
    });
    // The branding sets the accent: the apps show the brand colour, as the hub does.
    expect(sharedPrefsOf(values, 'de', { ...BRAND0, userAccent: false }).accent).toBeNull();
  });

  it('stores the whole object under `roedy.hub.prefs`; sign-out keeps what the guest pages use', () => {
    const prefs = { ...DEFAULT_PREFS, theme: 'dark', font_size: 'large', developer: true } as const;
    writeSharedPrefs(prefs);
    expect(JSON.parse(localStorage.getItem(KEY) ?? '')).toEqual(prefs);
    expect(readSharedPrefs()).toEqual(prefs);
    trimSharedPrefs();
    expect(JSON.parse(localStorage.getItem(KEY) ?? '')).toEqual({
      theme: 'dark',
      language: 'en',
      font_size: 'large',
      animations: true,
    });
    // Nothing stored: nothing to trim.
    localStorage.clear();
    trimSharedPrefs();
    expect(localStorage.getItem(KEY)).toBeNull();
  });

  it('ignores broken and unknown values', () => {
    localStorage.setItem(KEY, '{');
    expect(readSharedPrefs()).toEqual({});
    localStorage.setItem(KEY, JSON.stringify({ theme: 'sepia', language: 'fr', locale: 'de', other: 1 }));
    expect(readSharedPrefs()).toEqual({});
    localStorage.setItem(KEY, 'null');
    expect(readSharedPrefs()).toEqual({});
  });

  it('neither throws nor stores when the browser blocks the storage', () => {
    vi.stubGlobal('localStorage', undefined);
    Object.defineProperty(globalThis, 'localStorage', {
      configurable: true,
      get() {
        throw new DOMException('Blocked', 'SecurityError');
      },
    });
    expect(readSharedPrefs()).toEqual({});
    expect(() => writeSharedPrefs(DEFAULT_PREFS)).not.toThrow();
    expect(() => trimSharedPrefs()).not.toThrow();
  });
});
