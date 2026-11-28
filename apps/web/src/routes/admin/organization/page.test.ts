import { fireEvent, screen, within } from '@testing-library/svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { session, type Org } from '#lib/state/session.svelte.ts';
import { ui } from '@roedy/ui';
import { renderSignedIn, resetSignedIn, setPhone, TEST_CONFIG, type ApiRoutes } from '#lib/test/signed-in.ts';
import Page from './+page.svelte';
import SOURCE from './+page.svelte?raw';

vi.mock('$app/navigation', () => ({ goto: vi.fn(() => Promise.resolve()) }));
const sections = vi.hoisted(() => ({ scrollToSection: vi.fn() }));
vi.mock('#lib/sections.ts', () => sections);

/** State from `public/config`. */
const ORG: Org = { ...TEST_CONFIG.org, website: 'https://example.com', privacy: null };
/** State from `GET admin/org`, extended in the meantime. */
const ADMIN_ORG: Org = { ...ORG, help: 'https://example.com/help', source: 'https://git.example.com/example/hub' };

afterEach(() => {
  sections.scrollToSection.mockClear();
  resetSignedIn();
});

function renderPage(routes: ApiRoutes = {}) {
  return renderSignedIn(Page, {
    path: '/admin/organization',
    config: { org: ORG },
    api: { 'GET admin/org': [{ body: ADMIN_ORG }], ...routes },
  });
}

const field = (label: string) => screen.getByRole('textbox', { name: label });
const bar = () => screen.queryByRole('region', { name: 'Unsaved changes' });
const loaded = () => vi.waitFor(() => expect(field('Help center')).toHaveValue('https://example.com/help'));

describe('Organization page', () => {
  it('shows heading, tabs, notice and the fields with the state from public/config and admin/org', async () => {
    const view = renderPage();
    expect(view.getByRole('heading', { level: 1 })).toHaveTextContent('Organization');
    expect(view.getByText('Company details, support, and legal notices.')).toBeInTheDocument();
    for (const tab of ['Company', 'Support', 'Legal', 'Source code']) {
      expect(view.getByRole('button', { name: tab })).toBeInTheDocument();
      expect(view.getByRole('heading', { level: 2, name: tab })).toBeInTheDocument();
    }
    expect(view.getByText(/The name is also the hub’s name/)).toBeInTheDocument();
    expect(view.queryByRole('button', { name: 'View page' })).toBeNull();
    expect(view.getByText('Provider identification')).toBeInTheDocument();

    // Immediately the state from public/config, without bar.
    expect(field('Name')).toHaveValue('Example Org');
    expect(field('Name')).toHaveAttribute('placeholder', 'Example Inc.');
    expect(field('Website')).toHaveAttribute('type', 'url');
    expect(field('Website')).toHaveAttribute('placeholder', 'https://');
    expect(field('Support email')).toHaveAttribute('type', 'email');
    expect(field('Support email')).toHaveAttribute('placeholder', 'support@example.com');
    expect(field('Help center')).toHaveValue('');
    expect(view.getAllByRole('textbox')).toHaveLength(8);
    expect(bar()).toBeNull();

    // Then the state from admin/org.
    await loaded();
    expect(field('Source code')).toHaveValue('https://git.example.com/example/hub');
    expect(view.calls.map((c) => `${c.method} ${c.path}`)).toEqual(['GET admin/org']);
    expect(bar()).toBeNull();
  });

  it('shows the bar after an input and discards the changes', async () => {
    renderPage();
    await loaded();
    await fireEvent.input(field('Name'), { target: { value: 'Example Inc' } });
    const region = bar();
    expect(region).not.toBeNull();
    expect(within(region as HTMLElement).getByText('Unsaved changes')).toBeInTheDocument();
    await fireEvent.click(within(region as HTMLElement).getByRole('button', { name: 'Discard' }));
    expect(bar()).toBeNull();
    expect(field('Name')).toHaveValue('Example Org');
  });

  it('saves only the changed fields with "Saving …" and takes over the new hub name', async () => {
    const saved: Org = { ...ADMIN_ORG, name: 'Example Inc', imprint: null };
    let answer: (reply: { body: unknown }) => void = () => {};
    const view = renderPage({ 'PUT admin/org': [() => new Promise((resolve) => (answer = resolve))] });
    await loaded();
    await fireEvent.input(field('Name'), { target: { value: ' Example Inc ' } });
    await fireEvent.input(field('Legal notice'), { target: { value: '' } });
    await fireEvent.click(within(bar() as HTMLElement).getByRole('button', { name: 'Save' }));
    await vi.waitFor(() =>
      expect(within(bar() as HTMLElement).getByRole('button', { name: /Saving/ })).toHaveAttribute('aria-busy', 'true'),
    );
    const puts = view.calls.filter((c) => c.method === 'PUT');
    expect(puts.map((c) => [c.path, c.body])).toEqual([['admin/org', { name: 'Example Inc', imprint: null }]]);
    answer({ body: saved });
    await vi.waitFor(() => expect(bar()).toBeNull());
    expect(ui.toast?.text).toBe('Changes saved');
    expect(session.config?.org).toEqual(saved);
    expect(session.appName).toBe('Example Inc');
    expect(field('Name')).toHaveValue('Example Inc');
    expect(field('Legal notice')).toHaveValue('');
  });

  it('scrolls to the section via the tabs', async () => {
    const view = renderPage();
    await fireEvent.click(view.getByRole('button', { name: 'Legal' }));
    expect(sections.scrollToSection).toHaveBeenCalledTimes(1);
    const [root, key] = sections.scrollToSection.mock.calls[0] as [HTMLElement, string, boolean];
    expect(key).toBe('legal');
    expect(root.querySelector('[data-sec="legal"]')).toHaveTextContent('Legal');
    expect([...root.querySelectorAll('section')].map((s) => s.dataset.sec)).toEqual([
      'firm',
      'support',
      'legal',
      'source',
    ]);
  });

  it('shows field errors live at the field and does not save with an error', async () => {
    const view = renderPage();
    await loaded();
    expect(field('Website')).toHaveAttribute('aria-invalid', 'false');
    expect(view.queryByRole('alert')).toBeNull();

    await fireEvent.input(field('Website'), { target: { value: 'example.com' } });
    const input = field('Website');
    expect(input).toHaveAttribute('aria-invalid', 'true');
    expect(input).toHaveClass('bad');
    const alert = view.getByRole('alert');
    expect(alert).toHaveTextContent('Please enter a complete URL, e.g. https://example.com.');
    expect(input).toHaveAttribute('aria-describedby', alert.id);
    // On desktop below the row, not next to the field.
    expect(input.parentElement?.contains(alert)).toBe(false);

    await fireEvent.click(within(bar() as HTMLElement).getByRole('button', { name: 'Save' }));
    expect(ui.toast).toMatchObject({ text: 'Please check the highlighted fields', error: false });
    expect(sections.scrollToSection.mock.calls.at(-1)?.[1]).toBe('firm');
    expect(bar()).not.toBeNull();
    expect(view.calls.filter((c) => c.method === 'PUT')).toHaveLength(0);

    await fireEvent.input(field('Website'), { target: { value: '' } });
    expect(view.queryByRole('alert')).toBeNull();
    expect(field('Website')).not.toHaveAttribute('aria-describedby');
  });

  it('fields animate the focus ring and show the red ring', () => {
    const rule = (selector: string) =>
      new RegExp(`^ {2}${selector.replace(/[\\^$.*+?()[\]{}|]/g, '\\$&')} \\{([^}]*)\\}`, 'm').exec(SOURCE)?.[1] ?? '';
    expect(rule('input')).toMatch(/transition: box-shadow var\(--dur, 220ms\);/);
    expect(rule('input.bad')).toMatch(/box-shadow: 0 0 0 2px var\(--danger\);/);
  });

  it('shows a 422 from the server at the field until it is changed', async () => {
    const view = renderPage({
      'PUT admin/org': [{ status: 422, body: { error: 'invalid', fields: { source: { code: 'too_long' } } } }],
    });
    await loaded();
    await fireEvent.input(field('Source code'), { target: { value: 'https://git.example.com/example/other' } });
    await fireEvent.click(within(bar() as HTMLElement).getByRole('button', { name: 'Save' }));
    await vi.waitFor(() =>
      expect(view.getByRole('alert')).toHaveTextContent('The URL can’t be longer than 2048 characters.'),
    );
    expect(field('Source code')).toHaveAttribute('aria-invalid', 'true');
    expect(field('Source code')).toHaveValue('https://git.example.com/example/other');
    expect(bar()).not.toBeNull();
    expect(sections.scrollToSection.mock.calls.at(-1)?.[1]).toBe('source');
    await fireEvent.input(field('Source code'), { target: { value: 'https://git.example.com/example' } });
    expect(view.queryByRole('alert')).toBeNull();
  });

  it('shows heading, fields and the bar with two buttons on mobile', async () => {
    setPhone(true);
    const view = renderPage();
    expect(ui.isMobile).toBe(true);
    expect(view.getByRole('heading', { level: 1 })).toHaveTextContent('Organization');
    expect(view.getByRole('heading', { level: 2, name: 'Support' })).toBeInTheDocument();
    await loaded();
    await fireEvent.input(field('Support email'), { target: { value: 'help@example.com' } });
    const region = bar() as HTMLElement;
    expect(within(region).queryByText('Unsaved changes')).toBeNull();
    expect(
      within(region)
        .getAllByRole('button')
        .map((b) => b.textContent?.trim()),
    ).toEqual(['Discard', 'Save']);
    await fireEvent.click(within(region).getByRole('button', { name: 'Discard' }));
    expect(field('Support email')).toHaveValue('support@example.com');

    // On mobile the field error sits directly below the field.
    await fireEvent.input(field('Support email'), { target: { value: 'help' } });
    const alert = view.getByRole('alert');
    expect(alert).toHaveTextContent('Please enter a valid email address.');
    expect(field('Support email').parentElement?.contains(alert)).toBe(true);
  });
});
