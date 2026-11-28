// Taken over from Roedy Mail (`apps/web/src/routes/settings/language/page.test.ts`) with "Not set", the formats as
// pick lists with "Automatic", plus saving through `PUT settings`.
import { fireEvent, screen, within } from '@testing-library/svelte';
import { tick } from 'svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { ui } from '@roedy/ui';
import { m } from '#lib/paraglide/messages.js';
import { prefs, RESET0 } from '#lib/state/prefs.svelte.ts';
import { renderSignedIn, resetSignedIn, setPhone } from '#lib/test/signed-in.ts';
import Page from './+page.svelte';

vi.mock('$app/navigation', () => ({ goto: vi.fn(() => Promise.resolve()) }));

// The tabs scroll to the section; jsdom has no layout (`offsetParent`), hence only the call.
const scrollToSection = vi.hoisted(() => vi.fn());
vi.mock('#lib/sections.ts', () => ({ scrollToSection }));

// Fixed device time zone, independent of the test run's environment.
const device = vi.hoisted(() => ({ tz: 'Europe/Berlin' }));
vi.mock('@roedy/ui', async (importOriginal) => {
  const mod = await importOriginal<typeof import('@roedy/ui')>();
  return { ...mod, deviceZone: () => device.tz };
});

const german = { locale: 'de' } as const;
const STORED = { version: 2, settings: { ...RESET0, lang: 'en' } };
/** 1 November 2026, 10:00 UTC – 11:00 CET in Berlin. */
const NOW = Date.UTC(2026, 10, 1, 10, 0);
const REGIONS = [
  'Not set',
  'Germany',
  'Austria',
  'Switzerland',
  'France',
  'Italy',
  'Netherlands',
  'Spain',
  'United Kingdom',
  'United States',
];
const NBSP = ' ';

function group(name: string) {
  return within(screen.getByRole('radiogroup', { name }));
}

function checked(name: string) {
  return group(name).getByRole('radio', { checked: true });
}

function preview(container: HTMLElement) {
  return Array.from(container.querySelectorAll('.pv'), (p) => [
    p.querySelector('.pv-k')?.textContent,
    p.querySelector('.pv-v')?.textContent,
  ]);
}

function hiLabel(list: HTMLElement) {
  return list.querySelector('.opt.hi .opt-label')?.textContent;
}

/** Opens a pick list by its row label and returns its options' texts and the listbox. */
async function open(name: string) {
  await fireEvent.click(screen.getByRole('combobox', { name: new RegExp(`^${name} `) }));
  const list = screen.getByRole('listbox', { name });
  return { list, opts: within(list).getAllByRole('option') };
}

/** Sets the browser's languages and reports the change like the browser does. */
function browserLanguages(langs: string[]) {
  vi.spyOn(navigator, 'languages', 'get').mockReturnValue(langs);
  window.dispatchEvent(new Event('languagechange'));
}

beforeEach(() => {
  vi.useFakeTimers({ toFake: ['Date'] });
  vi.setSystemTime(NOW);
  device.tz = 'Europe/Berlin';
});

afterEach(() => {
  vi.useRealTimers();
  vi.restoreAllMocks();
  window.dispatchEvent(new Event('languagechange'));
  scrollToSection.mockClear();
  resetSignedIn();
});

describe('Language & region page', () => {
  it('shows header, tabs, sections and the defaults', () => {
    browserLanguages(['en-US']);
    prefs.set({ lang: 'en' });
    const view = renderSignedIn(Page, { role: 'user' });
    expect(view.getByRole('heading', { level: 1 })).toHaveTextContent('Language & region');
    expect(view.getByText('Language, time zone and formats.')).toBeInTheDocument();
    const tabs = ['Language', 'Region', 'Formats'];
    expect(view.getAllByRole('heading', { level: 2 }).map((h) => h.textContent)).toEqual(tabs);
    for (const t of tabs) expect(view.getByRole('button', { name: t })).toBeInTheDocument();
    expect(
      group('Language')
        .getAllByRole('radio')
        .map((r) => r.textContent.trim()),
    ).toEqual(['System Browser: English', 'DE Deutsch German', 'EN English']);
    expect(checked('Language')).toHaveTextContent('English');
    expect(view.queryByText(/use the language of your last sign-in/)).toBeNull();

    // No region: the formats follow the language.
    const region = view.getByRole('combobox', { name: 'Country or region Not set' });
    expect(region).toHaveAttribute('aria-expanded', 'false');
    expect(view.getByRole('switch', { name: 'Set time zone automatically' })).toHaveAttribute('aria-checked', 'true');
    expect(view.getByRole('combobox', { name: 'Time zone Berlin, Vienna, Zurich (CET)' })).toHaveAttribute(
      'aria-disabled',
      'true',
    );
    expect(view.getByRole('combobox', { name: /^Date format / })).toHaveTextContent('Automatic (2026-11-01)');
    expect(view.getByRole('combobox', { name: /^Time format / })).toHaveTextContent('Automatic (24-hour · 11:00)');
    expect(view.getByRole('combobox', { name: /^Week starts on / })).toHaveTextContent('Automatic (Monday)');
    expect(view.getByRole('combobox', { name: /^Number format / })).toHaveTextContent('Automatic (12,345.67)');
    expect(preview(view.container)).toEqual([
      ['Date', '2026-11-01'],
      ['Time', '11:00'],
      ['Number', '12,345.67'],
    ]);
    expect(view.getByText(/“Automatic” follows the country or region/)).toBeInTheDocument();
  });

  it('selects German: saves `lang`, the page switches the language at once', async () => {
    prefs.set({ lang: 'en' });
    const view = renderSignedIn(Page);
    const de = group('Language').getByRole('radio', { name: /Deutsch/ });
    expect(de).toHaveAttribute('aria-checked', 'false');
    expect(de.querySelector('.lang-name')).toHaveAttribute('lang', 'de');
    await fireEvent.click(de);
    expect(prefs.values.lang).toBe('de');
    // The page header follows too (`pageOf` in `$derived`), and "Automatic" follows the language.
    expect(view.getByRole('heading', { level: 1 })).toHaveTextContent(m.nav_language({}, german));
    expect(checked(m.region_tab_lang({}, german))).toHaveTextContent('Deutsch');
    expect(preview(view.container)[0]).toEqual([m.region_preview_date({}, german), '01.11.2026']);
    // Back to English.
    await fireEvent.click(group(m.region_tab_lang({}, german)).getByRole('radio', { name: /^EN / }));
    expect(prefs.values.lang).toBe('en');
    expect(view.getByRole('heading', { level: 1 })).toHaveTextContent('Language & region');
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
    expect(system.querySelector('.tile .tabler-icon-world')).not.toBeNull();
    expect(system.querySelector('.lang-name')).not.toHaveAttribute('lang');
    await fireEvent.click(system);
    expect(prefs.values.lang).toBe('system');
    expect(group('Language').getAllByRole('radio', { checked: true })).toEqual([system]);
    expect(view.getByText('Emails from Example Org use the language of your last sign-in.')).toBeInTheDocument();
    // A new browser language applies at once: the interface and the second line follow.
    browserLanguages(['de-DE']);
    await vi.waitFor(() =>
      expect(view.getByRole('heading', { level: 1 })).toHaveTextContent(m.nav_language({}, german)),
    );
    const cards = group(m.region_tab_lang({}, german)).getAllByRole('radio');
    expect(cards[0]).toHaveTextContent('Browser: Deutsch');
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
    await fireEvent.click(de);
    expect(prefs.values.lang).toBe('de');
  });

  it('selects a country in the popover; its formats apply and a new country resets own formats', async () => {
    const view = renderSignedIn(Page);
    const btn = view.getByRole('combobox', { name: /^Country or region / });
    const { list, opts } = await open('Country or region');
    expect(ui.menu).toBe('pick');
    expect(btn).toHaveAttribute('aria-expanded', 'true');
    expect(opts.map((o) => o.textContent.trim())).toEqual(REGIONS);
    expect(within(list).getByRole('option', { selected: true })).toHaveTextContent('Not set');
    await fireEvent.mouseMove(opts[1]!);
    expect(hiLabel(list)).toBe('Germany');
    await fireEvent.click(opts[1]!);
    expect(prefs.values.region).toBe('DE');
    expect(ui.menu).toBeNull();
    expect(btn).toHaveTextContent('Germany');
    expect(preview(view.container)).toEqual([
      ['Date', '01.11.2026'],
      ['Time', '11:00'],
      ['Number', '12.345,67'],
    ]);

    // An own format, then a new country: back to automatic, the United States' formats.
    await fireEvent.click((await open('Date format')).opts[4]!);
    expect(prefs.values.date_format).toBe('us');
    await fireEvent.click((await open('Country or region')).opts[9]!);
    expect(prefs.values).toMatchObject({ region: 'US', date_format: null, time_format: null });
    expect(preview(view.container)).toEqual([
      ['Date', '11/01/2026'],
      ['Time', '11:00 AM'],
      ['Number', '12,345.67'],
    ]);
    expect(view.getByRole('combobox', { name: /^Week starts on / })).toHaveTextContent('Automatic (Sunday)');
    // "Not set" again.
    await fireEvent.click((await open('Country or region')).opts[0]!);
    expect(prefs.values.region).toBeNull();
  });

  it('operates a pick list with arrow keys, Enter, Space and Tab', async () => {
    prefs.set({ region: 'DE' });
    const view = renderSignedIn(Page);
    const btn = view.getByRole('combobox', { name: /^Country or region / });
    await fireEvent.keyDown(btn, { key: 'ArrowDown' });
    const list = view.getByRole('listbox', { name: 'Country or region' });
    expect(hiLabel(list)).toBe('Germany');
    await fireEvent.keyDown(btn, { key: 'ArrowDown' });
    expect(hiLabel(list)).toBe('Austria');
    expect(btn).toHaveAttribute('aria-activedescendant', list.querySelector('.opt.hi')?.id);
    await fireEvent.keyDown(btn, { key: 'Enter' });
    expect(prefs.values.region).toBe('AT');
    expect(view.queryByRole('listbox')).toBeNull();

    await fireEvent.keyDown(btn, { key: 'ArrowDown' });
    await fireEvent.keyDown(btn, { key: 'ArrowDown' });
    await fireEvent.keyDown(btn, { key: ' ' });
    expect(view.getByRole('listbox')).toBeInTheDocument();
    await fireEvent.keyUp(btn, { key: ' ' });
    expect(prefs.values.region).toBe('CH');

    await fireEvent.keyDown(btn, { key: 'ArrowDown' });
    await fireEvent.keyDown(btn, { key: 'ArrowDown' });
    await fireEvent.keyDown(btn, { key: 'Tab' });
    expect(view.queryByRole('listbox')).toBeNull();
    expect(ui.menu).toBeNull();
    expect(prefs.values.region).toBe('CH');
  });

  it('locks the time zone while it is automatic and unlocks it with the switch', async () => {
    const view = renderSignedIn(Page);
    const tz = view.getByRole('combobox', { name: /^Time zone / });
    expect(tz.closest('.pick')).toHaveClass('disabled');
    await fireEvent.click(tz);
    expect(ui.menu).toBeNull();

    const auto = view.getByRole('switch', { name: 'Set time zone automatically' });
    await fireEvent.click(auto);
    expect(prefs.values).toMatchObject({ time_zone_auto: false, time_zone: 'Europe/Berlin' });
    expect(auto).toHaveAttribute('aria-checked', 'false');
    expect(tz).toHaveAttribute('aria-disabled', 'false');

    const { list, opts } = await open('Time zone');
    expect(opts).toHaveLength(6);
    expect(opts[0]?.querySelector('.opt-sub')).toHaveTextContent('CET');
    await fireEvent.click(within(list).getByRole('option', { name: /^Tokyo, Seoul/ }));
    expect(prefs.values.time_zone).toBe('Asia/Tokyo');
    expect(tz).toHaveTextContent('Tokyo, Seoul (GMT+9)');
    expect(preview(view.container)[1]).toEqual(['Time', '19:00']);

    // Automatic again: the device zone applies and is reported.
    await fireEvent.click(auto);
    expect(prefs.values).toMatchObject({ time_zone_auto: true, time_zone: 'Europe/Berlin' });
    expect(tz).toHaveTextContent('Berlin, Vienna, Zurich (CET)');
  });

  it('puts a zone outside the selection at the front', async () => {
    device.tz = 'America/Argentina/Buenos_Aires';
    prefs.set({ time_zone_auto: false, time_zone: 'America/Argentina/Buenos_Aires' });
    const view = renderSignedIn(Page);
    const tz = view.getByRole('combobox', { name: /^Time zone / });
    expect(tz).toHaveTextContent(/^Buenos Aires/);
    const { opts } = await open('Time zone');
    expect(opts).toHaveLength(7);
    expect(opts[0]).toHaveAttribute('aria-selected', 'true');
    expect(preview(view.container)[1]).toEqual(['Time', '07:00']);
  });

  it('chooses each format and "Automatic" again; the preview follows', async () => {
    prefs.set({ region: 'DE' });
    const view = renderSignedIn(Page);
    let { opts } = await open('Date format');
    expect(opts.map((o) => o.textContent.trim())).toEqual([
      'Automatic (01.11.2026)',
      '01.11.2026',
      '01/11/2026',
      '2026-11-01',
      '11/01/2026',
    ]);
    await fireEvent.click(opts[2]!);
    expect(prefs.values.date_format).toBe('dmy');
    ({ opts } = await open('Time format'));
    expect(opts.map((o) => o.textContent.trim())).toEqual([
      'Automatic (24-hour · 11:00)',
      '24-hour · 11:00',
      '12-hour · 11:00 AM',
    ]);
    await fireEvent.click(opts[2]!);
    expect(prefs.values.time_format).toBe('12h');
    ({ opts } = await open('Week starts on'));
    await fireEvent.click(opts[2]!);
    expect(prefs.values.week_start).toBe('sunday');
    ({ opts } = await open('Number format'));
    expect(opts.map((o) => o.textContent.trim())).toEqual([
      'Automatic (12.345,67)',
      '12.345,67',
      '12,345.67',
      `12${NBSP}345,67`,
      '12’345.67',
    ]);
    await fireEvent.click(opts[4]!);
    expect(prefs.values.number_format).toBe('1’234.56');
    expect(preview(view.container)).toEqual([
      ['Date', '01/11/2026'],
      ['Time', '11:00 AM'],
      ['Number', '12’345.67'],
    ]);
    await fireEvent.click((await open('Date format')).opts[0]!);
    expect(prefs.values.date_format).toBeNull();
    expect(preview(view.container)[0]).toEqual(['Date', '01.11.2026']);
  });

  it('saves the changes of a signed-in account with PUT settings; a new country resets the formats', async () => {
    const view = renderSignedIn(Page, {
      api: {
        'GET settings': [{ body: { version: 2, settings: { ...STORED.settings, date_format: 'iso' } } }],
        'PUT settings': [{ body: { version: 3, settings: { ...STORED.settings, region: 'FR' } } }],
      },
    });
    await prefs.load();
    await fireEvent.click((await open('Country or region')).opts[4]!);
    await prefs.settled();
    expect(view.calls.filter((c) => c.method === 'PUT').map((c) => c.body)).toEqual([
      {
        version: 2,
        settings: { region: 'FR', date_format: null, time_format: null, week_start: null, number_format: null },
      },
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
    expect(prefs.error).not.toBeNull();
    await vi.waitFor(() => expect(view.getByRole('heading', { level: 1 })).toHaveTextContent('Language & region'));
    expect(checked('Language')).toHaveTextContent('English');
  });

  it('updates the preview across the day change', async () => {
    vi.useFakeTimers({ toFake: ['Date', 'setInterval', 'clearInterval'] });
    // 23:59:50 in Berlin.
    vi.setSystemTime(Date.UTC(2026, 10, 1, 22, 59, 50));
    prefs.set({ region: 'DE' });
    const view = renderSignedIn(Page);
    expect(preview(view.container)[0]).toEqual(['Date', '01.11.2026']);
    vi.advanceTimersByTime(30_000);
    await tick();
    expect(preview(view.container)).toEqual([
      ['Date', '02.11.2026'],
      ['Time', '00:00'],
      ['Number', '12.345,67'],
    ]);
  });

  it('scrolls to the section via the tabs', async () => {
    const view = renderSignedIn(Page);
    await fireEvent.click(view.getByRole('button', { name: 'Region' }));
    expect(scrollToSection).toHaveBeenLastCalledWith(expect.any(HTMLElement), 'region', false);
    await fireEvent.click(view.getByRole('button', { name: 'Formats' }));
    expect(scrollToSection).toHaveBeenLastCalledWith(expect.any(HTMLElement), 'formats', false);
  });

  it('shows the pick lists in the sheet on mobile (ui.pick)', async () => {
    setPhone(true);
    prefs.set({ lang: 'system' });
    const view = renderSignedIn(Page);
    expect(view.container.querySelector('.page')).toHaveClass('mobile');
    expect(view.container.querySelector('h1')).toHaveClass('m-title');
    expect(group('Language').getAllByRole('radio')).toHaveLength(3);
    expect(view.getByText(/use the language of your last sign-in/)).toBeInTheDocument();

    const region = view.getByRole('button', { name: /^Country or region / });
    expect(region).toHaveAttribute('aria-haspopup', 'dialog');
    await fireEvent.click(region);
    expect(ui.menu).toBe('pick');
    expect(view.queryByRole('listbox')).toBeNull();
    const pick = ui.pick?.();
    expect(pick?.title).toBe('Country or region');
    expect(pick?.opts.map((o) => o.label)).toEqual(REGIONS);
    pick?.opts[2]?.onClick();
    await tick();
    expect(prefs.values.region).toBe('AT');
    expect(region).toHaveTextContent('Austria');

    await fireEvent.click(view.getByRole('button', { name: /^Number format / }));
    expect(ui.pick?.().title).toBe('Number format');

    view.unmount();
    expect(ui.menu).toBeNull();
    expect(ui.pick).toBeNull();
  });
});
