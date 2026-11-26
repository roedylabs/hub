import { describe, expect, it, vi } from 'vitest';
import { langChoices, languageValues } from '#lib/language.ts';

describe('langChoices', () => {
  it('offers "System" with the browser’s language first, then the languages by their own names', () => {
    const choices = langChoices('de', 'en', () => {});
    expect(choices.map((c) => [c.id, c.code, c.name, c.sub, c.label, c.on])).toEqual([
      ['system', '', 'System', 'Browser: English', 'System – Browser: English', false],
      ['de', 'DE', 'Deutsch', 'German', 'DE Deutsch', true],
      ['en', 'EN', 'English', '', 'EN English', false],
    ]);
  });

  it('selects only "System" while it follows the browser', () => {
    expect(
      langChoices('system', 'de', () => {})
        .filter((c) => c.on)
        .map((c) => c.id),
    ).toEqual(['system']);
  });

  it('saves a different language at once and leaves the chosen one alone', () => {
    const set = vi.fn();
    const [system, de, en] = langChoices('en', 'en', set);
    en!.onClick();
    expect(set).not.toHaveBeenCalled();
    de!.onClick();
    system!.onClick();
    expect(set.mock.calls).toEqual([[{ lang: 'de' }], [{ lang: 'system' }]]);
  });
});

describe('languageValues', () => {
  it('returns the cards with a second line only where it differs', () => {
    const v = languageValues({ lang: 'en', browserLang: 'de', appName: 'Example Org' }, () => {});
    expect(v.langOpts.map((l) => [l.id, l.sub, l.hasSub, l.on])).toEqual([
      ['system', 'Browser: Deutsch', true, false],
      ['de', 'German', true, false],
      ['en', '', false, true],
    ]);
    expect(v.systemNote).toBeNull();
  });

  it('notes under "System" that mails use the language of the last sign-in', () => {
    const v = languageValues({ lang: 'system', browserLang: 'en', appName: 'Example Org' }, () => {});
    expect(v.systemNote).toBe('Emails from Example Org use the language of your last sign-in.');
  });

  it('saves a choice through `set`', () => {
    const set = vi.fn();
    languageValues({ lang: 'en', browserLang: 'en', appName: 'Example Org' }, set).langOpts[1]!.onClick();
    expect(set).toHaveBeenCalledWith({ lang: 'de' });
  });
});
