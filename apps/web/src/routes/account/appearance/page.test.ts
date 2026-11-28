// Taken over from Roedy Mail (`apps/web/src/routes/settings/appearance/page.test.ts`) without the sidebar and the
// mailbox design, plus saving through `PUT settings`.
import { fireEvent, screen, within } from '@testing-library/svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { prefs } from '#lib/state/prefs.svelte.ts';
import { BRAND0, ui } from '@roedy/ui';
import { renderSignedIn, resetSignedIn, setPhone } from '#lib/test/signed-in.ts';
import Page from './+page.svelte';
import DialogHost from './DialogHost.svelte';

vi.mock('$app/navigation', () => ({ goto: vi.fn(() => Promise.resolve()) }));

// The tabs scroll to the section; jsdom has no layout (`offsetParent`), hence only the call.
const scrollToSection = vi.hoisted(() => vi.fn());
vi.mock('#lib/sections.ts', () => ({ scrollToSection }));

const LOCKED = 'Set by your administrator';
const STORED = { version: 3, settings: { lang: 'en', mode: null, accent: null } };

function group(name: string) {
  return within(screen.getByRole('radiogroup', { name }));
}

function checked(name: string) {
  return group(name).getByRole('radio', { checked: true });
}

afterEach(() => {
  scrollToSection.mockClear();
  resetSignedIn();
});

describe('Appearance page', () => {
  it('shows header, tabs, the sections and the defaults', () => {
    const view = renderSignedIn(Page, { role: 'user' });
    expect(view.getByRole('heading', { level: 1 })).toHaveTextContent('Appearance');
    expect(view.getByText('Color scheme, accent, text size and layout.')).toBeInTheDocument();
    const tabs = ['Color scheme', 'Accent color', 'Text & animation', 'Layout'];
    expect(view.getAllByRole('heading', { level: 2 }).map((h) => h.textContent)).toEqual(tabs);
    for (const t of tabs) expect(view.getByRole('button', { name: t })).toBeInTheDocument();
    expect(view.getByText('Light, dark, or matching your system.')).toBeInTheDocument();
    expect(view.getByText('For icons, highlights, and selection.')).toBeInTheDocument();
    expect(view.queryByText(LOCKED)).toBeNull();
    // Nothing of Roedy Mail's own settings (sidebar, mailbox design).
    expect(view.getAllByRole('switch')).toHaveLength(1);
    expect(view.container.querySelectorAll('[data-sec]')).toHaveLength(4);
    expect(checked('Font size')).toHaveTextContent('Medium');
    expect(checked('Density')).toHaveTextContent('Normal');
    expect(view.getByRole('switch', { name: 'Animations' })).toHaveAttribute('aria-checked', 'true');

    expect(
      group('Color scheme')
        .getAllByRole('radio')
        .map((r) => r.textContent.trim()),
    ).toEqual(['Light', 'Dark', 'System']);
    // Without a choice the system's colour scheme applies.
    expect(checked('Color scheme')).toHaveTextContent('System');
    expect(
      group('Accent color')
        .getAllByRole('radio')
        .map((r) => r.textContent.trim()),
    ).toEqual(['Apricot', 'Sage', 'Lavender', 'Sky', 'Rose', 'More']);
    // Without a choice the brand colour.
    expect(checked('Accent color')).toHaveTextContent('Apricot');
  });

  it('shows the preview of each mode in its fixed colours', () => {
    renderSignedIn(Page);
    const light = group('Color scheme').getByRole('radio', { name: 'Light' });
    expect(light.querySelector<HTMLElement>('.prev')?.style.background).toBe('rgb(243, 238, 231)');
    expect(light.querySelector<HTMLElement>('.prev-panel')?.style.background).toBe('rgb(255, 253, 250)');
    expect(group('Accent color').getByRole('radio', { name: 'Sky' }).style.getPropertyValue('--swatch')).toMatch(/^#/);
  });

  it('selects colour scheme and accent colour via prefs', async () => {
    renderSignedIn(Page);
    await fireEvent.click(group('Color scheme').getByRole('radio', { name: 'Light' }));
    expect(prefs.values.mode).toBe('light');
    expect(checked('Color scheme')).toHaveTextContent('Light');
    await fireEvent.click(group('Accent color').getByRole('radio', { name: 'Sky' }));
    expect(prefs.values.accent).toBe('sky');
    expect(checked('Accent color')).toHaveTextContent('Sky');
  });

  it('selects font size, animations and density via prefs', async () => {
    const view = renderSignedIn(Page);
    expect(
      group('Font size')
        .getAllByRole('radio')
        .map((r) => r.textContent.trim()),
    ).toEqual(['Small', 'Medium', 'Large']);
    await fireEvent.click(group('Font size').getByRole('radio', { name: 'Large' }));
    expect(prefs.values.font_size).toBe('large');
    expect(checked('Font size')).toHaveTextContent('Large');
    const anim = view.getByRole('switch', { name: 'Animations' });
    await fireEvent.click(anim);
    expect(prefs.values.animations).toBe(false);
    expect(anim).toHaveAttribute('aria-checked', 'false');
    expect(
      group('Density')
        .getAllByRole('radio')
        .map((r) => r.textContent.trim()),
    ).toEqual(['Compact', 'Normal', 'Comfortable']);
    await fireEvent.click(group('Density').getByRole('radio', { name: 'Compact' }));
    expect(prefs.values.density).toBe('compact');
    await fireEvent.click(view.getByRole('button', { name: 'Layout' }));
    expect(scrollToSection).toHaveBeenLastCalledWith(expect.any(HTMLElement), 'layout', false);
  });

  it('saves the choices of a signed-in account with PUT settings, one request after the other', async () => {
    const view = renderSignedIn(Page, {
      api: {
        'GET settings': [{ body: STORED }],
        'PUT settings': [
          { body: { version: 4, settings: { ...STORED.settings, mode: 'dark' } } },
          { body: { version: 5, settings: { ...STORED.settings, mode: 'dark', accent: 'rose' } } },
        ],
      },
    });
    await prefs.load();
    await fireEvent.click(group('Color scheme').getByRole('radio', { name: 'Dark' }));
    await fireEvent.click(group('Accent color').getByRole('radio', { name: 'Rose' }));
    await prefs.settled();
    expect(view.calls.filter((c) => c.method === 'PUT').map((c) => c.body)).toEqual([
      { version: 3, settings: { mode: 'dark' } },
      { version: 4, settings: { accent: 'rose' } },
    ]);
    expect(prefs.version).toBe(5);
    expect(checked('Color scheme')).toHaveTextContent('Dark');
    expect(checked('Accent color')).toHaveTextContent('Rose');
  });

  it('takes a newer state over on a version conflict and sends the own choice on top of it', async () => {
    const view = renderSignedIn(Page, {
      api: {
        'GET settings': [{ body: STORED }],
        'PUT settings': [
          {
            status: 409,
            body: { error: 'version_conflict', version: 7, settings: { ...STORED.settings, mode: 'light' } },
          },
          { body: { version: 8, settings: { ...STORED.settings, mode: 'light', accent: 'sage' } } },
        ],
      },
    });
    await prefs.load();
    await fireEvent.click(group('Accent color').getByRole('radio', { name: 'Sage' }));
    await prefs.settled();
    expect(view.calls.filter((c) => c.method === 'PUT').map((c) => c.body)).toEqual([
      { version: 3, settings: { accent: 'sage' } },
      { version: 7, settings: { accent: 'sage' } },
    ]);
    // The other device's colour scheme shows up too.
    expect(checked('Color scheme')).toHaveTextContent('Light');
    expect(checked('Accent color')).toHaveTextContent('Sage');
  });

  it('goes back to the confirmed choice when saving fails', async () => {
    renderSignedIn(Page, {
      api: {
        'GET settings': [{ body: { ...STORED, settings: { ...STORED.settings, mode: 'dark' } } }],
        'PUT settings': [{ status: 500, body: { error: 'internal' } }],
      },
    });
    await prefs.load();
    await fireEvent.click(group('Color scheme').getByRole('radio', { name: 'Light' }));
    expect(checked('Color scheme')).toHaveTextContent('Light');
    await prefs.settled();
    // The layout shows the toast for `prefs.error`.
    expect(prefs.error).not.toBeNull();
    await vi.waitFor(() => expect(checked('Color scheme')).toHaveTextContent('Dark'));
  });

  it('locks the accent colour when the branding sets it', async () => {
    const view = renderSignedIn(Page, { config: { branding: { ...BRAND0, userAccent: false, color: 'rose' } } });
    expect(view.getByText(LOCKED)).toBeInTheDocument();
    const accents = view.getByRole('radiogroup', { name: 'Accent color' });
    expect(accents).toHaveClass('locked');
    expect(accents).toHaveAttribute('aria-disabled', 'true');
    expect(checked('Accent color')).toHaveTextContent('Rose');
    const sky = within(accents).getByRole('radio', { name: 'Sky' });
    expect(sky).toBeDisabled();
    await fireEvent.click(sky);
    expect(prefs.values.accent).toBeNull();
    expect(checked('Accent color')).toHaveTextContent('Rose');
    // "More" is locked too and opens nothing.
    const more = within(accents).getByRole('radio', { name: 'More' });
    expect(more).toBeDisabled();
    await fireEvent.click(more);
    expect(ui.pageDialog?.()).toBeNull();
    // The colour scheme stays the user's.
    await fireEvent.click(group('Color scheme').getByRole('radio', { name: 'Dark' }));
    expect(prefs.values.mode).toBe('dark');
  });

  it('a locked custom accent shows the brand colour and "More"', () => {
    prefs.set({ accent: '#3a7bd5' });
    renderSignedIn(Page, { config: { branding: { ...BRAND0, userAccent: false, color: 'sage' } } });
    expect(checked('Accent color')).toHaveTextContent('Sage');
    expect(group('Accent color').getByRole('radio', { name: 'More' })).not.toBeChecked();
  });

  it('"More" opens the colour editor at the tile; Apply saves the custom accent at once', async () => {
    const view = renderSignedIn(DialogHost);
    const more = group('Accent color').getByRole('radio', { name: 'More' });
    // A radio like the setup's tile: no `aria-haspopup` (not allowed on `radio`).
    expect(more).not.toHaveAttribute('aria-haspopup');
    expect(more).not.toBeChecked();
    // Ring of the palette's hues with + while no custom accent is active.
    expect(more.querySelector('.swatch .tabler-icon-plus')).not.toBeNull();
    expect(ui.pageDialog?.()).toBeNull();

    await fireEvent.click(more);
    expect(ui.pageDialog?.()).toMatchObject({ title: 'More colors', bare: true });
    const editor = await view.findByRole('dialog', { name: 'More colors' });
    // Starts with the base of the active accent (apricot).
    expect(within(editor).getByLabelText('Or your own hex code')).toHaveValue('#F5A06E');
    await fireEvent.input(within(editor).getByLabelText('Or your own hex code'), { target: { value: '3a7bd5' } });
    await fireEvent.click(within(editor).getByRole('button', { name: 'Apply' }));
    expect(prefs.values.accent).toBe('#3a7bd5');
    expect(view.queryByRole('dialog', { name: 'More colors' })).toBeNull();

    // The tile shows the colour with the check, like a named one, and gets the focus back.
    const custom = group('Accent color').getByRole('radio', { name: 'Custom #3A7BD5' });
    expect(custom).toBe(more);
    // Visible "Custom"; the code only in the name, which contains the visible text (WCAG 2.5.3).
    expect(more.querySelector('.accent-label')).toHaveTextContent(/^Custom$/);
    expect(checked('Accent color')).toBe(more);
    expect(more.style.getPropertyValue('--swatch')).toBe('#3a7bd5');
    expect(more.querySelector('.swatch > .tabler-icon-check')).not.toBeNull();
    expect(more.querySelector('.ring')).toBeNull();
    await vi.waitFor(() => expect(more).toHaveFocus());

    // Opened again it starts with the custom colour; Escape changes nothing.
    await fireEvent.click(more);
    const again = await view.findByRole('dialog', { name: 'More colors' });
    expect(within(again).getByLabelText('Or your own hex code')).toHaveValue('#3A7BD5');
    await fireEvent.click(within(again).getByRole('radio', { name: 'Mint' }));
    await fireEvent.keyDown(again, { key: 'Escape' });
    expect(view.queryByRole('dialog', { name: 'More colors' })).toBeNull();
    expect(prefs.values.accent).toBe('#3a7bd5');
    await vi.waitFor(() => expect(more).toHaveFocus());

    // A named accent turns the tile back into "More".
    await fireEvent.click(group('Accent color').getByRole('radio', { name: 'Sky' }));
    expect(prefs.values.accent).toBe('sky');
    expect(more).toHaveAccessibleName('More');
    expect(more).not.toBeChecked();
  });

  it('shows a preset as the custom accent under its name; the arrows do not open the editor', async () => {
    prefs.set({ accent: '#78c895' });
    renderSignedIn(DialogHost);
    const mint = group('Accent color').getByRole('radio', { name: 'Mint' });
    expect(mint).toBeChecked();
    expect(mint).not.toHaveAttribute('aria-label');
    expect(mint.querySelector('.accent-label')).toHaveTextContent(/^Mint$/);
    const rose = group('Accent color').getByRole('radio', { name: 'Rose' });
    rose.focus();
    await fireEvent.keyDown(rose, { key: 'ArrowRight' });
    expect(mint).toHaveFocus();
    expect(ui.pageDialog?.()).toBeNull();
    await fireEvent.keyDown(mint, { key: 'ArrowRight' });
    expect(prefs.values.accent).toBe('apricot');
  });

  it('clears its dialog when the page is left', async () => {
    const view = renderSignedIn(Page);
    await fireEvent.click(group('Accent color').getByRole('radio', { name: 'More' }));
    expect(ui.pageDialog?.()).not.toBeNull();
    view.unmount();
    expect(ui.pageDialog).toBeNull();
  });

  it('scrolls to the section via the tabs, instantly without animations', async () => {
    const view = renderSignedIn(Page);
    ui.animations = true;
    await fireEvent.click(view.getByRole('button', { name: 'Accent color' }));
    expect(scrollToSection).toHaveBeenLastCalledWith(expect.any(HTMLElement), 'accent', true);
    ui.animations = false;
    await fireEvent.click(view.getByRole('button', { name: 'Color scheme' }));
    expect(scrollToSection).toHaveBeenLastCalledWith(expect.any(HTMLElement), 'mode', false);
  });

  it('shows header, tabs and the small previews on mobile; the colour editor as its own sheet', async () => {
    setPhone(true);
    const view = renderSignedIn(DialogHost);
    expect(view.getByRole('heading', { level: 1 })).toHaveTextContent('Appearance');
    expect(view.getByRole('button', { name: 'Accent color' })).toBeInTheDocument();
    expect(view.container.querySelector('.page')).toHaveClass('mobile');
    expect(group('Color scheme').getAllByRole('radio')).toHaveLength(3);
    expect(view.container.querySelector('.m-prev')).not.toBeNull();
    expect(view.container.querySelector('.prev')).toBeNull();
    expect(group('Accent color').getAllByRole('radio')).toHaveLength(6);
    await fireEvent.click(group('Accent color').getByRole('radio', { name: 'More' }));
    const editor = await view.findByRole('dialog', { name: 'More colors' });
    expect(editor).toHaveClass('sheet');
  });
});
