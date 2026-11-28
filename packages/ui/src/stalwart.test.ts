import { afterEach, describe, expect, it } from 'vitest';
import {
  accountColor,
  accountColorName,
  accountLetter,
  accountStateText,
  byteText,
  quotaShare,
  quotaText,
} from './stalwart.ts';
import { connectSettings } from './settings.svelte.ts';

afterEach(() => connectSettings({}));

describe('Stalwart accounts', () => {
  it('maps the colour index to MA_COLORS, wrapping around', () => {
    expect(accountColor(0)).toEqual({ bg: '#f5a06e', ink: '#3a1b06' });
    expect(accountColor(5)).toEqual({ bg: '#f2c46b', ink: '#3d2a00' });
    expect(accountColor(6)).toEqual(accountColor(0));
    expect(accountColor(-1)).toEqual(accountColor(5));
    expect(accountColorName(1)).toBe('Sage');
    connectSettings({ lang: () => 'de' });
    expect(accountColorName(1)).toBe('Salbei');
  });

  it('takes the first letter or digit of the address for the tile', () => {
    expect(accountLetter('alex@example.com')).toBe('A');
    expect(accountLetter('_9x@example.com')).toBe('9');
    expect(accountLetter('ömer@example.com')).toBe('Ö');
    expect(accountLetter('')).toBe('?');
  });

  it('names states and storage', () => {
    expect(accountStateText('ok')).toBe('Connected');
    expect(accountStateText('needs_sign_in')).toBe('Sign-in needed');
    expect(accountStateText('blocked')).toBe('Blocked');
    expect(byteText(512, 'en')).toBe('512 B');
    expect(byteText(1536, 'en')).toBe('1.5 KB');
    expect(byteText(5 * 1024 ** 3, 'en')).toBe('5 GB');
    const checked_at = '2026-11-28T09:00:00.000Z';
    expect(quotaText(null)).toBeNull();
    expect(quotaText({ used: null, total: null, checked_at })).toBeNull();
    expect(quotaText({ used: 2048, total: null, checked_at }, 'en')).toBe('2 KB used');
    expect(quotaText({ used: 1024 ** 2, total: 100 * 1024 ** 2, checked_at }, 'en')).toBe('1 MB of 100 MB');
    expect(quotaShare({ used: 25, total: 100, checked_at })).toBe(0.25);
    expect(quotaShare({ used: 200, total: 100, checked_at })).toBe(1);
    expect(quotaShare({ used: 25, total: null, checked_at })).toBeNull();
    connectSettings({ lang: () => 'de' });
    expect(accountStateText('needs_sign_in')).toBe('Anmeldung nötig');
    expect(quotaText({ used: 1536, total: 1024 ** 2, checked_at }, 'de')).toBe('1,5 KB von 1 MB');
  });
});
