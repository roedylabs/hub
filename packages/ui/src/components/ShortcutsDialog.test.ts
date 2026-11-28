// Taken over from Roedy Mail (`apps/web/src/lib/components/ShortcutsDialog.test.ts`) with a shortcut list of the
// test instead of Mail's, plus the backdrop, the German texts and axe.
import { fireEvent, render } from '@testing-library/svelte';
import { describe, expect, it, vi } from 'vitest';
import { keyLabel, modKey, shortcut } from '../keys.ts';
import { connectSettings } from '../settings.svelte.ts';
import type { ShortcutsDialogValues } from '../shell.ts';
import { violations } from '../testing/axe.ts';
import ShortcutsDialog from './ShortcutsDialog.svelte';

function props(patch: Partial<ShortcutsDialogValues> = {}): ShortcutsDialogValues {
  return {
    shortcuts: [
      shortcut('Search', '/'),
      shortcut('Open command palette', modKey(), 'K'),
      shortcut('Show keyboard shortcuts', '?'),
      shortcut('Open', keyLabel('enter')),
      shortcut('Close', keyLabel('esc')),
    ],
    closeHelp: () => {},
    onHelpOverlay: () => {},
    ...patch,
  };
}

describe('keyboard shortcuts', () => {
  it('lists the shortcuts and closes', async () => {
    const closeHelp = vi.fn();
    const { getByRole, getByText } = render(ShortcutsDialog, props({ closeHelp }));
    expect(getByRole('dialog', { name: 'Keyboard shortcuts' })).toHaveFocus();
    // jsdom reports no Apple system: Ctrl instead of ⌘.
    expect(getByText('Open command palette').nextElementSibling).toHaveTextContent('CtrlK');
    expect(getByText('Open').nextElementSibling).toHaveTextContent('Enter');
    // The list scrolls and is therefore reachable via keyboard.
    const list = getByRole('region', { name: 'Keyboard shortcuts' });
    expect(list).toHaveAttribute('tabindex', '0');
    expect(list).toContainElement(getByText('Open command palette'));
    await fireEvent.click(getByRole('button', { name: 'Close' }));
    expect(closeHelp).toHaveBeenCalled();
  });

  it('passes a backdrop click to the app', async () => {
    const onHelpOverlay = vi.fn();
    const view = render(ShortcutsDialog, props({ onHelpOverlay }));
    await fireEvent.mouseDown(view.container.querySelector('.scrim')!);
    expect(onHelpOverlay).toHaveBeenCalled();
  });

  it('has its German texts', () => {
    connectSettings({ lang: () => 'de' });
    try {
      const view = render(ShortcutsDialog, props({ shortcuts: [shortcut('Befehlspalette öffnen', modKey(), 'K')] }));
      expect(view.getByRole('dialog', { name: 'Tastenkürzel' })).toBeInTheDocument();
      expect(view.getByRole('button', { name: 'Schließen' })).toBeInTheDocument();
      expect(view.getByText('Befehlspalette öffnen').nextElementSibling).toHaveTextContent('StrgK');
    } finally {
      connectSettings({});
    }
  });

  it('has no axe violations', async () => {
    expect(await violations(render(ShortcutsDialog, props()))).toEqual([]);
  });
});
