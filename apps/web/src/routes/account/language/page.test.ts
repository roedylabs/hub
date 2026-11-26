// Taken over from Roedy Mail (`apps/web/src/routes/settings/language/page.test.ts`), the language section only, plus
// saving through `PUT settings`.
import { fireEvent, screen, within } from '@testing-library/svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { m } from '#lib/paraglide/messages.js';
import { prefs } from '#lib/state/prefs.svelte.ts';
import { renderSignedIn, resetSignedIn, setPhone } from '#lib/test/signed-in.ts';
import Page from './+page.svelte';

vi.mock('$app/navigation', () => ({ goto: vi.fn(() => Promise.resolve()) }));

const german = { locale: 'de' } as const;
const STORED = { version: 2, settings: { lang: 'en', mode: null, accent: null } };

function group(name: string) {
  return within(screen.getByRole('radiogroup', { name }));
}

function checked(name: string) {
  return group(name).getByRole('radio', { checked: true });
}

/** Sets the browser's languages and reports the change like the browser does. */
function browserLanguages(langs: string[]) {
  vi.spyOn(navigator, 'languages', 'get').mockReturnValue(langs);
  window.dispatchEvent(new Event('languagechange'));
}

afterEach(() => {
  vi.restoreAllMocks();
  window.dispatchEvent(new Event('languagechange'));
  resetSignedIn();
});

describe('Language page', () => {
  it('shows header and the language cards with the current language', () => {
    browserLanguages(['en-US']);
    prefs.set({ lang: 'en' });
    const view = renderSignedIn(Page, { role: 'user' });
    expect(view.getByRole('heading', { level: 1 })).toHaveTextContent('Language');
    expect(view.getByText('Language of the interface.')).toBeInTheDocument();
    // One section: no tabs, no section heading.
    expect(view.queryAllByRole('heading', { level: 2 })).toEqual([]);
    expect(view.queryByRole('button', { name: 'Language' })).toBeNull();
    expect(
      group('Language')
        .getAllByRole('radio')
        .map((r) => r.textContent.trim()),
    ).toEqual(['System Browser: English', 'DE Deutsch German', 'EN English']);
    expect(checked('Language')).toHaveTextContent('English');
    expect(checked('Language')).toHaveClass('on');
    expect(view.queryByText(/use the language of your last sign-in/)).toBeNull();
  });

  it('selects German: saves `lang`, the page switches the language at once', async () => {
    prefs.set({ lang: 'en' });
    const view = renderSignedIn(Page);
    const de = group('Language').getByRole('radio', { name: /Deutsch/ });
    expect(de).toHaveAttribute('aria-checked', 'false');
    expect(de.querySelector('.lang-name')).toHaveAttribute('lang', 'de');
    await fireEvent.click(de);
    expect(prefs.values.lang).toBe('de');
    // The page header follows too (`pageOf` in `$derived`).
    expect(view.getByRole('heading', { level: 1 })).toHaveTextContent(m.nav_language({}, german));
    expect(view.getByText(m.nav_language_desc({}, german))).toBeInTheDocument();
    expect(checked(m.nav_language({}, german))).toHaveTextContent('Deutsch');
    // Back to English.
    await fireEvent.click(group(m.nav_language({}, german)).getByRole('radio', { name: /^EN / }));
    expect(prefs.values.lang).toBe('en');
    expect(view.getByRole('heading', { level: 1 })).toHaveTextContent('Language');
  });

  it('a click on the chosen language changes nothing', async () => {
    prefs.set({ lang: 'en' });
    const view = renderSignedIn(Page, { api: { 'GET settings': [{ body: STORED }] } });
    await prefs.load();
    await fireEvent.click(group('Language').getByRole('radio', { name: /^EN / }));
    await prefs.settled();
    expect(view.calls.filter((c) => c.method === 'PUT')).toEqual([]);
  });

  it('selects "System": only its card is checked, the browser\'s language live and the note on mails', async () => {
    browserLanguages(['en-US']);
    prefs.set({ lang: 'en' });
    const view = renderSignedIn(Page);
    const system = group('Language').getByRole('radio', { name: 'System Browser: English' });
    expect(group('Language').getAllByRole('radio')[0]).toBe(system);
    expect(system.querySelector('.tile .tabler-icon-world')).not.toBeNull();
    expect(system.querySelector('.lang-name')).not.toHaveAttribute('lang');
    await fireEvent.click(system);
    expect(prefs.values.lang).toBe('system');
    expect(group('Language').getAllByRole('radio', { checked: true })).toEqual([system]);
    // The hub's name from the configuration.
    expect(view.getByText('Emails from Example Org use the language of your last sign-in.')).toBeInTheDocument();
    // A new browser language applies at once: the interface and the second line follow.
    browserLanguages(['de-DE']);
    await vi.waitFor(() =>
      expect(view.getByRole('heading', { level: 1 })).toHaveTextContent(m.nav_language({}, german)),
    );
    const cards = group(m.nav_language({}, german)).getAllByRole('radio');
    expect(cards[0]).toHaveTextContent('Browser: Deutsch');
    expect(group(m.nav_language({}, german)).getAllByRole('radio', { checked: true })).toEqual([cards[0]]);
    expect(view.getByText(m.region_lang_system_mail({ app: 'Example Org' }, german))).toBeInTheDocument();
  });

  it('moves with the arrow keys without choosing; Space and Enter choose', async () => {
    prefs.set({ lang: 'en' });
    renderSignedIn(Page);
    const [system, de, en] = group('Language').getAllByRole('radio') as [HTMLElement, HTMLElement, HTMLElement];
    en.focus();
    await fireEvent.keyDown(en, { key: 'ArrowRight' });
    expect(system).toHaveFocus();
    await fireEvent.keyDown(system, { key: 'ArrowRight' });
    expect(de).toHaveFocus();
    expect(prefs.values.lang).toBe('en');
    // Buttons: Space and Enter activate them (a click in jsdom).
    await fireEvent.click(de);
    expect(prefs.values.lang).toBe('de');
  });

  it('saves the language of a signed-in account with PUT settings', async () => {
    prefs.set({ lang: 'en' });
    const view = renderSignedIn(Page, {
      api: {
        'GET settings': [{ body: STORED }],
        'PUT settings': [{ body: { version: 3, settings: { ...STORED.settings, lang: 'de' } } }],
      },
    });
    await prefs.load();
    await fireEvent.click(group('Language').getByRole('radio', { name: /Deutsch/ }));
    await prefs.settled();
    expect(view.calls.filter((c) => c.method === 'PUT').map((c) => c.body)).toEqual([
      { version: 2, settings: { lang: 'de' } },
    ]);
    expect(prefs.version).toBe(3);
  });

  it('goes back to the confirmed language when saving fails', async () => {
    prefs.set({ lang: 'en' });
    const view = renderSignedIn(Page, {
      api: {
        'GET settings': [{ body: STORED }],
        'PUT settings': [{ status: 500, body: { error: 'internal' } }],
      },
    });
    await prefs.load();
    await fireEvent.click(group('Language').getByRole('radio', { name: /Deutsch/ }));
    expect(prefs.values.lang).toBe('de');
    await prefs.settled();
    // The layout shows the toast for `prefs.error`.
    expect(prefs.error).not.toBeNull();
    await vi.waitFor(() => expect(view.getByRole('heading', { level: 1 })).toHaveTextContent('Language'));
    expect(checked('Language')).toHaveTextContent('English');
  });

  it('shows header and cards on mobile', () => {
    setPhone(true);
    prefs.set({ lang: 'system' });
    const view = renderSignedIn(Page);
    expect(view.container.querySelector('.page')).toHaveClass('mobile');
    expect(view.container.querySelector('h1')).toHaveClass('m-title');
    expect(group('Language').getAllByRole('radio')).toHaveLength(3);
    expect(view.getByText(/use the language of your last sign-in/)).toBeInTheDocument();
  });
});
