import { afterEach, describe, expect, it, vi } from 'vitest';
import { readSharedPrefs, writeSharedPrefs } from '#lib/shared-prefs.ts';

/** The key the apps read (docs/app-contract.md), written out so that a rename of the constant fails here. */
const KEY = 'roedy.hub.prefs';

afterEach(() => {
  vi.unstubAllGlobals();
  localStorage.clear();
});

describe('shared preferences (docs/app-contract.md › Namespaces in the browser)', () => {
  it('stores theme and language under `roedy.hub.prefs`', () => {
    writeSharedPrefs({ theme: 'dark' });
    writeSharedPrefs({ locale: 'de' });
    expect(JSON.parse(localStorage.getItem(KEY) ?? '')).toEqual({ theme: 'dark', locale: 'de' });
    expect(readSharedPrefs()).toEqual({ theme: 'dark', locale: 'de' });
    // `null` forgets the language: it follows the browser.
    writeSharedPrefs({ locale: null });
    expect(readSharedPrefs()).toEqual({ theme: 'dark' });
  });

  it('ignores broken and unknown values', () => {
    localStorage.setItem(KEY, '{');
    expect(readSharedPrefs()).toEqual({});
    localStorage.setItem(KEY, JSON.stringify({ theme: 'sepia', locale: 'fr', other: 1 }));
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
    expect(() => writeSharedPrefs({ theme: 'dark' })).not.toThrow();
  });
});
