// Taken over from Roedy Mail (`apps/web/src/lib/components/CommandPalette.test.ts`), plus the palette without a
// search command, its keyboard (`paletteKey`, from Mail's layout), the German texts and axe.
import { fireEvent, render } from '@testing-library/svelte';
import { describe, expect, it, vi } from 'vitest';
import { connectSettings } from '../settings.svelte.ts';
import { paletteKey, paletteRows, type Command, type CommandPaletteValues, type PaletteRow } from '../shell.ts';
import { violations } from '../testing/axe.ts';
import CommandPalette from './CommandPalette.svelte';

const run = vi.fn();
const COMMANDS: Command[] = [
  { g: 'Actions', label: 'New email', icon: 'ti-pencil-plus', k: 'C', run },
  { g: 'Settings', label: 'Open settings', icon: 'ti-settings', k: ',', run },
  { g: 'Settings', label: 'Switch to light theme', icon: 'ti-sun', run },
  { g: 'Help', label: 'Show keyboard shortcuts', icon: 'ti-keyboard', k: '?', run },
];

const ctx = (close = vi.fn(), select = vi.fn()) => ({
  search: (q: string): Command => ({ g: 'Search', label: `Search Roedy Mail for "${q}"`, icon: 'ti-search', run }),
  close,
  select,
});

function props(patch: Partial<CommandPaletteValues> = {}): CommandPaletteValues {
  return {
    palQ: '',
    palRows: paletteRows(COMMANDS, '', 0, ctx()),
    palEmpty: false,
    onPalQ: () => {},
    onPalKey: () => {},
    onPalOverlay: () => {},
    ...patch,
  };
}

function key(k: string) {
  return { key: k, preventDefault: vi.fn() };
}

describe('command palette', () => {
  it('groups, filters and puts the search first', () => {
    const rows = paletteRows(COMMANDS, '', 0, ctx());
    expect(rows.map((r) => r.hasHdr)).toEqual([true, true, false, true]);
    expect(rows[0]).toMatchObject({ sel: true, hasKey: true, k: 'C' });
    const found = paletteRows(COMMANDS, ' sett ', 5, ctx());
    expect(found.map((r) => r.label)).toEqual(['Search Roedy Mail for "sett"', 'Open settings']);
    // Selection stays in the list.
    expect(found[1]?.sel).toBe(true);
  });

  it('only filters without a search command', () => {
    const { close, select } = ctx();
    expect(paletteRows(COMMANDS, 'sett', 0, { close, select }).map((r) => r.label)).toEqual(['Open settings']);
    expect(paletteRows(COMMANDS, 'xyz', 0, { close, select })).toEqual([]);
  });

  it('closes before running and selects on hover', () => {
    const close = vi.fn();
    const select = vi.fn();
    const rows = paletteRows(COMMANDS, '', 0, ctx(close, select));
    rows[1]?.run();
    expect(close).toHaveBeenCalledBefore(run);
    rows[2]?.hover();
    expect(select).toHaveBeenCalledWith(2);
    rows[0]?.hover();
    expect(select).toHaveBeenCalledTimes(1);
  });

  it('marks with the arrow keys, wrapping around, and runs with Enter', () => {
    const select = vi.fn();
    const rows = paletteRows(COMMANDS, '', 0, ctx());
    const down = key('ArrowDown');
    paletteKey(down, rows, 0, select);
    expect(down.preventDefault).toHaveBeenCalled();
    expect(select).toHaveBeenLastCalledWith(1);
    paletteKey(key('ArrowDown'), rows, 3, select);
    expect(select).toHaveBeenLastCalledWith(0);
    paletteKey(key('ArrowUp'), rows, 0, select);
    expect(select).toHaveBeenLastCalledWith(3);
    // A mark beyond a shorter list counts as its last row.
    paletteKey(key('ArrowUp'), rows, 9, select);
    expect(select).toHaveBeenLastCalledWith(2);

    const row = { ...rows[2], run: vi.fn() } as PaletteRow;
    const enter = key('Enter');
    paletteKey(enter, [rows[0]!, rows[1]!, row], 2, select);
    expect(enter.preventDefault).toHaveBeenCalled();
    expect(row.run).toHaveBeenCalled();

    // Without rows: the mark goes to 0, Enter does nothing; other keys stay with the field.
    select.mockClear();
    paletteKey(key('ArrowDown'), [], 0, select);
    expect(select).toHaveBeenCalledWith(0);
    expect(() => paletteKey(key('Enter'), [], 0, select)).not.toThrow();
    const a = key('a');
    paletteKey(a, rows, 0, select);
    expect(a.preventDefault).not.toHaveBeenCalled();
  });

  it('shows the search field with focus, rows and the empty state', async () => {
    const onPalQ = vi.fn();
    const view = render(CommandPalette, props({ onPalQ }));
    const input = view.getByPlaceholderText('What would you like to do?');
    expect(input).toHaveFocus();
    expect(view.getByRole('dialog', { name: 'Command palette' })).toBeInTheDocument();
    expect(view.getAllByText('Settings')).toHaveLength(1);
    expect(view.getByText('Esc')).toBeInTheDocument();
    await fireEvent.input(input, { target: { value: 'x' } });
    expect(onPalQ).toHaveBeenCalled();
    await view.rerender(props({ palQ: 'xyz', palRows: [], palEmpty: true, onPalQ }));
    expect(view.getByText('No commands found')).toBeInTheDocument();
  });

  it('passes keys and backdrop clicks to the app and returns the focus on closing', async () => {
    const before = document.createElement('button');
    document.body.append(before);
    before.focus();
    const onPalKey = vi.fn();
    const onPalOverlay = vi.fn();
    const view = render(CommandPalette, props({ onPalKey, onPalOverlay }));
    await fireEvent.keyDown(view.getByRole('textbox', { name: 'Command palette' }), { key: 'ArrowDown' });
    expect(onPalKey).toHaveBeenCalled();
    await fireEvent.mouseDown(view.container.querySelector('.scrim')!);
    expect(onPalOverlay).toHaveBeenCalled();
    view.unmount();
    expect(before).toHaveFocus();
    before.remove();
  });

  it('has its German texts', () => {
    connectSettings({ lang: () => 'de' });
    try {
      const view = render(CommandPalette, props({ palRows: [], palEmpty: true }));
      expect(view.getByRole('dialog', { name: 'Befehlspalette' })).toBeInTheDocument();
      expect(view.getByPlaceholderText('Was möchten Sie tun?')).toBeInTheDocument();
      expect(view.getByText('Keine Befehle gefunden')).toBeInTheDocument();
    } finally {
      connectSettings({});
    }
  });

  it('has no axe violations', async () => {
    expect(await violations(render(CommandPalette, props()))).toEqual([]);
  });
});
