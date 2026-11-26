// Taken over from Roedy Mail (`apps/web/src/routes/admin/audit-log/page.test.ts`) with the hub's category and times.
import { fireEvent, screen, within } from '@testing-library/svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { AuditCounts, AuditEntry } from '#lib/audit.svelte.ts';
import { downloadBlob } from '#lib/download.ts';
import type { Call } from '#lib/state/fetch-mock.ts';
import { prefs } from '#lib/state/prefs.svelte.ts';
import { ui } from '#lib/state/ui.svelte.ts';
import { renderSignedIn, resetSignedIn, setPhone, type ApiRoutes } from '#lib/test/signed-in.ts';
import Page from './+page.svelte';

vi.mock('$app/navigation', () => ({ goto: vi.fn(() => Promise.resolve()) }));
// The browser's time zone of the tests: Berlin, one hour ahead of UTC in November.
vi.mock('#lib/audit-time.ts', async (original) => ({
  ...(await original<typeof import('#lib/audit-time.ts')>()),
  browserTimeZone: () => 'Europe/Berlin',
}));
vi.mock('#lib/download.ts', () => ({ downloadBlob: vi.fn(() => true) }));

/** Thursday, 12:00 in Berlin. */
const NOW = Date.parse('2026-11-05T11:00:00Z');

const entry = (id: number, at: string, patch: Partial<AuditEntry>): AuditEntry => ({
  id,
  at,
  cat: 'login',
  icon: 'ti-login',
  who: 'Jordan Hale',
  act: ' signed in',
  meta: '192.0.2.x',
  warn: false,
  ...patch,
});

const ENTRIES = [
  entry(4, '2026-11-05T08:26:00.000Z', {}),
  entry(3, '2026-11-05T07:00:00.000Z', {
    cat: 'users',
    icon: 'ti-user-shield',
    who: 'Sam Rivera',
    act: ' reset the administrator access',
    meta: '',
    warn: true,
  }),
  entry(2, '2026-11-04T15:10:00.000Z', {
    cat: 'settings',
    icon: 'ti-brush',
    who: 'Alex Example',
    act: ' changed the branding',
    meta: '',
  }),
  entry(1, '2026-11-02T09:00:00.000Z', {
    cat: 'apps',
    icon: 'ti-apps',
    who: 'Robin Lee',
    act: ' opened an app',
    meta: '',
  }),
];
const COUNTS: AuditCounts = { all: 4, login: 1, users: 1, apps: 1, settings: 1, system: 0 };
const DATA = { entries: ENTRIES, counts: COUNTS, next: null };
const NONE = { entries: [], counts: { all: 0, login: 0, users: 0, apps: 0, settings: 0, system: 0 }, next: null };

beforeEach(() => {
  vi.useFakeTimers({ toFake: ['Date'] });
  vi.setSystemTime(NOW);
});

afterEach(() => {
  vi.useRealTimers();
  vi.clearAllMocks();
  resetSignedIn();
  // A chosen language is remembered on the device (shared preferences).
  localStorage.clear();
  prefs.reset();
});

function renderPage(routes: ApiRoutes = {}) {
  return renderSignedIn(Page, {
    path: '/admin/audit-log',
    api: {
      'GET admin/audit': [{ body: DATA }],
      'GET admin/audit/retention': [{ body: { days: 90 } }],
      ...routes,
    },
  });
}

/** A response that only arrives on `release()`. */
function held(answer: { status?: number; body?: unknown }) {
  let release = () => {};
  const reply = new Promise<{ status?: number; body?: unknown }>((resolve) => {
    release = () => resolve(answer);
  });
  return { reply: () => reply, release: () => release() };
}

const filter = (label: string) => screen.getByRole('button', { name: new RegExp(`^${label}`) });
const keepGroup = () => screen.getByRole('group', { name: 'Keep entries' });
const keep = (label: string) => within(keepGroup()).getByRole('button', { name: label });
const row = (who: string) => screen.getByText(who).closest('.entry') as HTMLElement;
const listCalls = (calls: Call[]) => calls.filter((c) => c.method === 'GET' && !c.path.endsWith('retention'));

describe('Audit log page', () => {
  it('shows heading, search, filters with counts and the entries by day', async () => {
    const view = renderPage();
    await view.findByText('Jordan Hale');
    expect(view.getByRole('heading', { level: 1 })).toHaveTextContent('Audit log');
    expect(view.getByText('Track changes and sign-ins.')).toBeInTheDocument();
    const search = view.getByRole('textbox', { name: 'Search audit log' });
    expect(search).toHaveAttribute('placeholder', 'Person, action, or location');
    const exportButton = view.getByRole('button', { name: 'Export as CSV' });
    expect(exportButton).toHaveTextContent('Export as CSV');
    expect(exportButton).not.toHaveAttribute('aria-label');

    const pills = within(view.getAllByRole('group').at(0) as HTMLElement).getAllByRole('button');
    expect(pills.map((b) => b.textContent.trim())).toEqual([
      'All4',
      'Sign-in1',
      'Users1',
      'Apps1',
      'Settings1',
      'System0',
    ]);
    expect(filter('All')).toHaveAttribute('aria-pressed', 'true');
    expect(filter('Sign-in')).toHaveAttribute('aria-pressed', 'false');

    // Days as h2 directly below the h1, then "Retention".
    expect(view.getAllByRole('heading', { level: 2 }).map((h) => h.textContent)).toEqual([
      'Today',
      'Yesterday',
      'Monday 2 November',
      'Retention',
    ]);
    expect(view.queryByRole('heading', { level: 3 })).toBeNull();
    expect(row('Jordan Hale')).toHaveTextContent('Jordan Hale signed in');
    // Times in the browser's time zone.
    expect(within(row('Jordan Hale')).getByText('09:26 · 192.0.2.x')).toBeInTheDocument();
    expect(within(row('Jordan Hale')).getByText('Sign-in')).toBeInTheDocument();
    // Without a detail only the time; the warning's tile is red.
    expect(within(row('Sam Rivera')).getByText('08:00')).toBeInTheDocument();
    expect(within(row('Sam Rivera')).getByText('Users')).toBeInTheDocument();
    expect(row('Sam Rivera').querySelector('.tile')).toHaveClass('warn');
    expect(row('Jordan Hale').querySelector('.tile')).not.toHaveClass('warn');
    expect(within(row('Robin Lee')).getByText('Apps')).toBeInTheDocument();
    // Divider only between two entries of a day.
    expect(view.container.querySelectorAll('.sep')).toHaveLength(1);
    expect(view.queryByText('No entries found')).toBeNull();
    expect(view.queryByRole('button', { name: 'Load more entries' })).toBeNull();
    expect(view.queryByRole('button', { name: 'Clear search' })).toBeNull();
  });

  it('formats days and times in the UI language', async () => {
    prefs.set({ lang: 'de' });
    const view = renderPage();
    await view.findByText('Jordan Hale');
    expect(view.getAllByRole('heading', { level: 2 }).map((h) => h.textContent)).toEqual([
      'Heute',
      'Gestern',
      'Montag, 2. November',
      'Aufbewahrung',
    ]);
    expect(within(row('Jordan Hale')).getByText('Anmeldung')).toBeInTheDocument();
  });

  it('keeps counts, days and period empty before the first response', async () => {
    const list = held({ body: DATA });
    const days = held({ body: { days: 90 } });
    const view = renderPage({ 'GET admin/audit': [list.reply], 'GET admin/audit/retention': [days.reply] });
    expect(filter('All').textContent.trim()).toBe('All');
    const frame = view.getByRole('status', { name: 'Loading audit log' });
    expect(frame).toHaveAttribute('aria-busy', 'true');
    expect(frame).toHaveClass('empty', 'first');
    expect(view.queryByText('No entries found')).toBeNull();
    // Retention without selection, locked, without a row below.
    expect(keepGroup().closest('.box')).toHaveAttribute('aria-busy', 'true');
    for (const label of ['30 days', '90 days', '1 year']) {
      expect(keep(label)).toHaveAttribute('aria-disabled', 'true');
      expect(keep(label)).toHaveAttribute('aria-pressed', 'false');
    }
    await fireEvent.click(keep('1 year'));
    expect(view.calls.filter((c) => c.method === 'PUT')).toHaveLength(0);
    expect(view.queryByText(/are deleted/)).toBeNull();

    list.release();
    days.release();
    await view.findByText('Jordan Hale');
    expect(view.queryByRole('status', { name: 'Loading audit log' })).toBeNull();
    expect(view.container.querySelector('.empty')).toBeNull();
    await view.findByText('Entries older than 90 days are deleted');
    expect(view.container.querySelector('[aria-busy="true"]')).toBeNull();
    expect(keep('90 days')).toHaveAttribute('aria-disabled', 'false');
  });

  it('sends `cat` on a click on a filter and shows its entries', async () => {
    const list = held({ body: { ...DATA, entries: ENTRIES.slice(3, 4) } });
    const view = renderPage({ 'GET admin/audit?cat=apps': [list.reply] });
    await view.findByText('Jordan Hale');

    await fireEvent.click(filter('Apps'));
    expect(filter('Apps')).toHaveAttribute('aria-pressed', 'true');
    expect(filter('All')).toHaveAttribute('aria-pressed', 'false');
    // The previous list stays until the response.
    expect(view.getByText('Jordan Hale').closest('[aria-busy]')).toHaveAttribute('aria-busy', 'true');

    list.release();
    await vi.waitFor(() => expect(view.queryByText('Jordan Hale')).toBeNull());
    expect(view.getByText('Robin Lee').closest('[aria-busy]')).toHaveAttribute('aria-busy', 'false');
    expect(listCalls(view.calls).map((c) => c.path)).toEqual(['admin/audit', 'admin/audit?cat=apps']);
  });

  it('searches after the typing pause', async () => {
    const view = renderPage({ 'GET admin/audit?q=robin': [{ body: { ...DATA, entries: ENTRIES.slice(3) } }] });
    await view.findByText('Jordan Hale');
    await fireEvent.input(view.getByRole('textbox', { name: 'Search audit log' }), { target: { value: 'robin' } });
    await vi.waitFor(() => expect(view.queryByText('Jordan Hale')).toBeNull());
    expect(view.getByText('Robin Lee')).toBeInTheDocument();
    expect(listCalls(view.calls).map((c) => c.path)).toEqual(['admin/audit', 'admin/audit?q=robin']);
  });

  it('shows the empty state without days', async () => {
    const view = renderPage({ 'GET admin/audit': [{ body: NONE }] });
    expect(await view.findByText('No entries found')).toBeInTheDocument();
    expect(view.getAllByRole('heading', { level: 2 }).map((h) => h.textContent)).toEqual(['Retention']);
    expect(filter('All').textContent.trim()).toBe('All0');
  });

  it('shows the load card in place of the list and reloads with "Try again"', async () => {
    const view = renderPage({ 'GET admin/audit': [{ status: 500 }, { body: DATA }] });
    const card = await view.findByRole('alert');
    expect(card).toHaveTextContent('Couldn’t load the audit log');
    expect(card).toHaveTextContent('That didn’t work. Please try again.');
    expect(view.queryByText('No entries found')).toBeNull();
    expect(view.queryByRole('status', { name: 'Loading audit log' })).toBeNull();
    expect(filter('All').textContent.trim()).toBe('All');

    await fireEvent.click(within(card).getByRole('button', { name: 'Try again' }));
    expect(await view.findByText('Jordan Hale')).toBeInTheDocument();
    expect(view.queryByRole('alert')).toBeNull();
    expect(filter('All').textContent.trim()).toBe('All4');
  });

  it('after a failed switch the card stays and the counts are empty', async () => {
    const view = renderPage({ 'GET admin/audit?cat=users': [{ status: 500 }] });
    await view.findByText('Jordan Hale');
    await fireEvent.click(filter('Users'));
    const card = await view.findByRole('alert');
    expect(card).toHaveTextContent('Couldn’t load the audit log');
    expect(view.queryByText('Jordan Hale')).toBeNull();
    expect(filter('Users').textContent.trim()).toBe('Users');
  });

  it('appends the next page with "Load more entries"', async () => {
    const page = held({ body: { ...DATA, entries: ENTRIES.slice(3), next: null } });
    const view = renderPage({
      'GET admin/audit': [{ body: { ...DATA, entries: ENTRIES.slice(0, 3), next: 2 } }],
      'GET admin/audit?before=2': [page.reply],
    });
    const more = await view.findByRole('button', { name: 'Load more entries' });
    expect(more).toHaveAttribute('aria-busy', 'false');
    expect(more).toBeEnabled();
    expect(more).not.toHaveClass('busy');
    expect(view.queryByText('Robin Lee')).toBeNull();

    await fireEvent.click(more);
    // While loading more: "Loading …", `aria-busy`, not `disabled` (the focus stays).
    expect(more).toHaveAttribute('aria-busy', 'true');
    expect(more).toHaveTextContent('Loading …');
    expect(more).toHaveClass('busy');
    expect(more).toBeEnabled();
    await fireEvent.click(more);
    expect(view.calls.filter((c) => c.path.includes('before='))).toHaveLength(1);

    page.release();
    expect(await view.findByText('Robin Lee')).toBeInTheDocument();
    expect(view.getByText('Jordan Hale')).toBeInTheDocument();
    expect(view.queryByRole('button', { name: 'Load more entries' })).toBeNull();
  });

  it('moves the focus to the first new entry when the last page takes the button away', async () => {
    const view = renderPage({
      'GET admin/audit': [{ body: { ...DATA, entries: ENTRIES.slice(0, 3), next: 2 } }],
      'GET admin/audit?before=2': [{ body: { ...DATA, entries: ENTRIES.slice(3), next: null } }],
    });
    const more = await view.findByRole('button', { name: 'Load more entries' });
    more.focus();
    await fireEvent.click(more);
    await vi.waitFor(() => expect(document.activeElement).toBe(row('Robin Lee')));
    expect(more).not.toBeInTheDocument();
  });

  it('leaves the focus where the user moved it while the last page loads', async () => {
    const page = held({ body: { ...DATA, entries: ENTRIES.slice(3), next: null } });
    const view = renderPage({
      'GET admin/audit': [{ body: { ...DATA, entries: ENTRIES.slice(0, 3), next: 2 } }],
      'GET admin/audit?before=2': [page.reply],
    });
    const more = await view.findByRole('button', { name: 'Load more entries' });
    more.focus();
    await fireEvent.click(more);
    const search = view.getByRole('textbox', { name: 'Search audit log' });
    search.focus();
    page.release();
    await view.findByText('Robin Lee');
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(document.activeElement).toBe(search);
  });

  it('leaves the focus on the page if the user left the button there while the last page loads', async () => {
    const page = held({ body: { ...DATA, entries: ENTRIES.slice(3), next: null } });
    const view = renderPage({
      'GET admin/audit': [{ body: { ...DATA, entries: ENTRIES.slice(0, 3), next: 2 } }],
      'GET admin/audit?before=2': [page.reply],
    });
    const more = await view.findByRole('button', { name: 'Load more entries' });
    more.focus();
    await fireEvent.click(more);
    more.blur();
    expect(document.activeElement).toBe(document.body);
    page.release();
    await view.findByText('Robin Lee');
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(document.activeElement).toBe(document.body);
  });

  it('moves "Today" and "Yesterday" on at midnight', async () => {
    vi.useFakeTimers({ toFake: ['Date', 'setTimeout', 'clearTimeout'] });
    // 23:59 in Berlin.
    vi.setSystemTime(Date.parse('2026-11-05T22:59:00Z'));
    const view = renderPage();
    const days = () => view.getAllByRole('heading', { level: 2 }).map((h) => h.textContent);
    await vi.waitFor(() => expect(days()).toEqual(['Today', 'Yesterday', 'Monday 2 November', 'Retention']));
    await vi.advanceTimersByTimeAsync(60_000);
    expect(days()).toEqual(['Yesterday', 'Wednesday 4 November', 'Monday 2 November', 'Retention']);
    expect(listCalls(view.calls)).toHaveLength(1);
  });

  it('reports a failed load-more as an error toast; entries and button stay', async () => {
    const view = renderPage({
      'GET admin/audit': [{ body: { ...DATA, entries: ENTRIES.slice(0, 3), next: 2 } }],
      'GET admin/audit?before=2': [{ status: 500 }],
    });
    await fireEvent.click(await view.findByRole('button', { name: 'Load more entries' }));
    await vi.waitFor(() =>
      expect(ui.toast).toMatchObject({ error: true, text: 'That didn’t work. Please try again.' }),
    );
    expect(view.getByText('Jordan Hale')).toBeInTheDocument();
    expect(view.getByRole('button', { name: 'Load more entries' })).toHaveAttribute('aria-busy', 'false');
    expect(view.queryByRole('alert')).toBeNull();
  });

  it('exports the shown selection as CSV with the local date and shows the new entry', async () => {
    const view = renderPage({
      'GET admin/audit': [{ body: DATA }, { body: DATA }],
      // The file as the server writes it: header and four entries.
      'POST admin/audit/export': [{ raw: `\u{feff}"Time";"Who"\r\n${'"2026-11-05";"Jordan Hale"\r\n'.repeat(4)}` }],
    });
    await view.findByText('Jordan Hale');
    await fireEvent.click(view.getByRole('button', { name: 'Export as CSV' }));
    await vi.waitFor(() => expect(ui.toast).toMatchObject({ error: false, text: '4 entries exported' }));
    expect(vi.mocked(downloadBlob).mock.calls[0]![0]).toBe('audit-log-2026-11-05.csv');
    await vi.waitFor(() => expect(listCalls(view.calls)).toHaveLength(2));
  });

  it('shows the period with total and saves a new one at once', async () => {
    const view = renderPage({
      'GET admin/audit': [{ body: DATA }, { body: DATA }],
      'PUT admin/audit/retention': [{ body: { days: 365 } }],
    });
    expect(await view.findByText('Entries older than 90 days are deleted')).toBeInTheDocument();
    expect(view.getByRole('heading', { level: 2, name: 'Retention' })).toBeInTheDocument();
    expect(view.getByText('Applies to all areas of the audit log.')).toBeInTheDocument();
    expect(view.getByText('Keep entries')).toBeInTheDocument();
    expect(
      within(keepGroup())
        .getAllByRole('button')
        .map((b) => b.textContent),
    ).toEqual(['30 days', '90 days', '1 year']);
    expect(keep('90 days')).toHaveAttribute('aria-pressed', 'true');
    expect(keep('90 days')).toHaveClass('on');

    await fireEvent.click(keep('1 year'));
    expect(view.getByText('Entries older than one year are deleted')).toBeInTheDocument();
    expect(keep('1 year')).toHaveAttribute('aria-pressed', 'true');
    expect(keep('90 days')).toHaveAttribute('aria-pressed', 'false');
    expect(view.calls.filter((c) => c.method === 'PUT').map((c) => [c.path, c.body])).toEqual([
      ['admin/audit/retention', { days: 365 }],
    ]);
    // The period applies at once, the list is reloaded.
    await vi.waitFor(() => expect(listCalls(view.calls)).toHaveLength(2));
  });

  it('shows the load card in place of the retention and reloads the period', async () => {
    const view = renderPage({ 'GET admin/audit/retention': [{ status: 500 }, { body: { days: 30 } }] });
    const card = await view.findByRole('alert');
    expect(card).toHaveTextContent('Couldn’t load the retention setting');
    expect(view.queryByRole('group', { name: 'Keep entries' })).toBeNull();
    expect(view.getByRole('heading', { level: 2, name: 'Retention' })).toBeInTheDocument();

    await fireEvent.click(within(card).getByRole('button', { name: 'Try again' }));
    expect(await view.findByText('Entries older than 30 days are deleted')).toBeInTheDocument();
    expect(view.queryByRole('alert')).toBeNull();
    expect(keep('30 days')).toHaveAttribute('aria-pressed', 'true');
  });

  it('shows "Clear search" only with input and clears the field', async () => {
    const view = renderPage();
    await view.findByText('Jordan Hale');
    const search = view.getByRole('textbox', { name: 'Search audit log' });
    await fireEvent.input(search, { target: { value: 'alex' } });
    expect(search).toHaveValue('alex');

    await fireEvent.click(view.getByRole('button', { name: 'Clear search' }));
    expect(search).toHaveValue('');
    expect(search).toHaveFocus();
    expect(view.queryByRole('button', { name: 'Clear search' })).toBeNull();
    // The input never went out, so nothing to reload.
    expect(listCalls(view.calls)).toHaveLength(1);
  });

  it('aborts the list when leaving the page and sends nothing more', async () => {
    const list = held({ body: DATA });
    const view = renderPage({ 'GET admin/audit': [list.reply] });
    await fireEvent.input(view.getByRole('textbox', { name: 'Search audit log' }), { target: { value: 'alex' } });
    view.unmount();
    expect(vi.mocked(fetch).mock.calls[0]![1]!.signal!.aborted).toBe(true);
    list.release();
    await new Promise((resolve) => setTimeout(resolve, 500));
    expect(listCalls(view.calls)).toHaveLength(1);
  });

  it('shows the area in the meta line, no badge and the export as an icon on mobile', async () => {
    setPhone(true);
    const view = renderPage();
    await view.findByText('Jordan Hale');
    expect(ui.isMobile).toBe(true);
    expect(view.getByRole('heading', { level: 1 })).toHaveTextContent('Audit log');
    expect(view.getByText('Track changes and sign-ins.')).toBeInTheDocument();

    expect(within(row('Jordan Hale')).getByText('09:26 · Sign-in · 192.0.2.x')).toBeInTheDocument();
    expect(within(row('Jordan Hale')).queryByText('09:26 · 192.0.2.x')).toBeNull();
    expect(within(row('Sam Rivera')).getByText('08:00 · Users')).toBeInTheDocument();
    expect(view.container.querySelector('.cat')).toBeNull();

    const exportButton = view.getByRole('button', { name: 'Export as CSV' });
    expect(exportButton).toHaveAttribute('aria-label', 'Export as CSV');
    expect(exportButton.textContent.trim()).toBe('');
    expect(view.getByText('Applies to all areas.')).toBeInTheDocument();
    expect(view.queryByText('Applies to all areas of the audit log.')).toBeNull();
  });
});
