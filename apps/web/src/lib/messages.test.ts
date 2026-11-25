import { describe, expect, test } from 'vitest';
import de from '../../messages/de.json';
import en from '../../messages/en.json';

describe('messages', () => {
  test('every key exists in English and German', () => {
    expect(Object.keys(de).sort()).toEqual(Object.keys(en).sort());
  });

  test('no message is empty', () => {
    for (const [key, text] of [...Object.entries(en), ...Object.entries(de)]) {
      expect(text, key).not.toBe('');
    }
  });
});
