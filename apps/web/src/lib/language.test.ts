import { describe, expect, it, vi } from 'vitest';
import { langChoices } from '#lib/language.ts';

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
