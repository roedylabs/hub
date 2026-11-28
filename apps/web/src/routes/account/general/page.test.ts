// Taken over from Roedy Mail (`apps/web/src/routes/settings/general/page.test.ts`) with the behaviour section and
// developer mode, plus saving through `PUT settings`.
import { fireEvent, screen, within } from '@testing-library/svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { ui } from '@roedy/ui';
import { prefs, RESET0 } from '#lib/state/prefs.svelte.ts';
import { renderSignedIn, resetSignedIn, setPhone } from '#lib/test/signed-in.ts';
import Page from './+page.svelte';

vi.mock('$app/navigation', () => ({ goto: vi.fn(() => Promise.resolve()) }));

// The tabs scroll to the section; jsdom has no layout (`offsetParent`), hence only the call.
const scrollToSection = vi.hoisted(() => vi.fn());
vi.mock('#lib/sections.ts', () => ({ scrollToSection }));

const STORED = { version: 4, settings: { ...RESET0 } };

function undo() {
  return within(screen.getByRole('radiogroup', { name: 'Undo period' }));
}

afterEach(() => {
  scrollToSection.mockClear();
  resetSignedIn();
});

describe('General page', () => {
  it('shows header, tabs, the sections and the defaults', () => {
    const view = renderSignedIn(Page, { role: 'user' });
    expect(view.getByRole('heading', { level: 1 })).toHaveTextContent('General');
    expect(view.getByText('Behavior in the hub and all apps.')).toBeInTheDocument();
    const tabs = ['Behavior', 'Developer'];
    expect(view.getAllByRole('heading', { level: 2 }).map((h) => h.textContent)).toEqual(tabs);
    for (const t of tabs) expect(view.getByRole('button', { name: t })).toBeInTheDocument();
    expect(
      view.getAllByRole('switch').map((s) => [s.getAttribute('aria-label'), s.getAttribute('aria-checked')]),
    ).toEqual([
      ['Ask before deleting', 'true'],
      ['Keyboard shortcuts', 'true'],
      ['Developer mode', 'false'],
    ]);
    expect(
      undo()
        .getAllByRole('radio')
        .map((r) => r.textContent.trim()),
    ).toEqual(['3 s', '5 s', '10 s']);
    expect(undo().getByRole('radio', { checked: true })).toHaveTextContent('5 s');
  });

  it('switches the settings; `@roedy/ui` follows at once', async () => {
    const view = renderSignedIn(Page);
    await fireEvent.click(view.getByRole('switch', { name: 'Ask before deleting' }));
    expect(prefs.values.confirm_delete).toBe(false);
    expect(ui.confirmDelete).toBe(false);
    await fireEvent.click(undo().getByRole('radio', { name: '10 s' }));
    expect(prefs.values.undo_delay).toBe(10);
    expect(ui.undoMs).toBe(10_000);
    await fireEvent.click(view.getByRole('switch', { name: 'Keyboard shortcuts' }));
    expect(ui.shortcuts).toBe(false);
    const dev = view.getByRole('switch', { name: 'Developer mode' });
    await fireEvent.click(dev);
    expect(prefs.values.developer).toBe(true);
    expect(ui.developer).toBe(true);
    expect(dev).toHaveAttribute('aria-checked', 'true');
    expect(dev).toHaveClass('on');
  });

  it('saves a change of a signed-in account with PUT settings', async () => {
    const view = renderSignedIn(Page, {
      api: {
        'GET settings': [{ body: STORED }],
        'PUT settings': [{ body: { version: 5, settings: { ...RESET0, undo_delay: 3 } } }],
      },
    });
    await prefs.load();
    await fireEvent.click(undo().getByRole('radio', { name: '3 s' }));
    await prefs.settled();
    expect(view.calls.filter((c) => c.method === 'PUT').map((c) => c.body)).toEqual([
      { version: 4, settings: { undo_delay: 3 } },
    ]);
    expect(prefs.version).toBe(5);
  });

  it('scrolls to the sections via the tabs', async () => {
    const view = renderSignedIn(Page);
    await fireEvent.click(view.getByRole('button', { name: 'Developer' }));
    expect(scrollToSection).toHaveBeenLastCalledWith(expect.any(HTMLElement), 'developer', false);
  });

  it('leaves out the keyboard shortcuts on mobile', () => {
    setPhone(true);
    const view = renderSignedIn(Page);
    expect(view.container.querySelector('.page')).toHaveClass('mobile');
    expect(view.getAllByRole('switch').map((s) => s.getAttribute('aria-label'))).toEqual([
      'Ask before deleting',
      'Developer mode',
    ]);
    expect(view.getByText('Confirm before removing')).toBeInTheDocument();
  });
});
