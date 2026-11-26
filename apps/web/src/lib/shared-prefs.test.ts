import { afterEach, describe, expect, it } from 'vitest';
import { readSharedPrefs, SHARED_PREFS_KEY, writeSharedPrefs } from './shared-prefs.ts';

afterEach(() => localStorage.clear());

describe('shared preferences (docs/app-contract.md › Namespaces in the browser)', () => {
  it('stores theme and language under `roedy.hub.prefs`', () => {
    writeSharedPrefs({ theme: 'dark' });
    writeSharedPrefs({ locale: 'de' });
    expect(JSON.parse(localStorage.getItem(SHARED_PREFS_KEY) ?? '')).toEqual({ theme: 'dark', locale: 'de' });
    expect(readSharedPrefs()).toEqual({ theme: 'dark', locale: 'de' });
    // `null` forgets the language: it follows the browser.
    writeSharedPrefs({ locale: null });
    expect(readSharedPrefs()).toEqual({ theme: 'dark' });
  });

  it('ignores broken and unknown values', () => {
    localStorage.setItem(SHARED_PREFS_KEY, '{');
    expect(readSharedPrefs()).toEqual({});
    localStorage.setItem(SHARED_PREFS_KEY, JSON.stringify({ theme: 'sepia', locale: 'fr', other: 1 }));
    expect(readSharedPrefs()).toEqual({});
    localStorage.setItem(SHARED_PREFS_KEY, 'null');
    expect(readSharedPrefs()).toEqual({});
  });
});
