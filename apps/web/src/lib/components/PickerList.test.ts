// Taken over from Roedy Mail (`apps/web/src/lib/components/PickerList.test.ts`).
import { fireEvent, render } from '@testing-library/svelte';
import { describe, expect, it, vi } from 'vitest';
import PickerList from '#lib/components/PickerList.svelte';

describe('picker list', () => {
  it('shows title and options and selects', async () => {
    const choose = vi.fn();
    const closeSheet = vi.fn();
    const view = render(PickerList, {
      pick: {
        title: 'Role for 2 people',
        opts: [
          {
            label: 'Administrator',
            sub: 'Manages users and settings',
            hasSub: true,
            on: false,
            fw: 600,
            onClick: choose,
          },
          { label: 'User', sub: '', hasSub: false, on: true, fw: 800, onClick: () => {} },
        ],
      },
      closeSheet,
    });
    const list = view.getByRole('listbox', { name: 'Role for 2 people' });
    expect(list).toBeInTheDocument();
    expect(view.getByRole('option', { name: /^User/ })).toHaveAttribute('aria-selected', 'true');
    expect(view.getByText('Manages users and settings')).toBeInTheDocument();
    await fireEvent.click(view.getByRole('option', { name: /Administrator/ }));
    expect(choose).toHaveBeenCalled();
    await fireEvent.click(view.getByRole('button', { name: 'Close' }));
    expect(closeSheet).toHaveBeenCalled();
  });

  it('moves the focus with the arrow keys, Home and End from the chosen option', async () => {
    const choose = vi.fn();
    const opts = ['Administrator', 'User', 'Guest'].map((label) => ({
      label,
      sub: '',
      hasSub: false,
      on: label === 'User',
      fw: 600,
      onClick: choose,
    }));
    const view = render(PickerList, { pick: { title: 'Role', opts }, closeSheet: () => {} });
    const [admin, user, guest] = view.getAllByRole('option');
    // One tab stop: the chosen option.
    expect(admin).toHaveAttribute('tabindex', '-1');
    expect(user).toHaveAttribute('tabindex', '0');
    expect(guest).toHaveAttribute('tabindex', '-1');
    user?.focus();
    await fireEvent.keyDown(user as HTMLElement, { key: 'ArrowDown' });
    expect(guest).toHaveFocus();
    // Stops at the ends.
    await fireEvent.keyDown(guest as HTMLElement, { key: 'ArrowDown' });
    expect(guest).toHaveFocus();
    await fireEvent.keyDown(guest as HTMLElement, { key: 'Home' });
    expect(admin).toHaveFocus();
    await fireEvent.keyDown(admin as HTMLElement, { key: 'ArrowUp' });
    expect(admin).toHaveFocus();
    await fireEvent.keyDown(admin as HTMLElement, { key: 'End' });
    expect(guest).toHaveFocus();
    // Moving the focus does not choose.
    expect(choose).not.toHaveBeenCalled();
  });

  it('puts the tab stop on the first option while none is chosen', () => {
    const opts = ['A', 'B'].map((label) => ({ label, sub: '', hasSub: false, on: false, fw: 600, onClick: () => {} }));
    const view = render(PickerList, { pick: { title: 'Pick', opts }, closeSheet: () => {} });
    expect(view.getAllByRole('option').map((o) => o.getAttribute('tabindex'))).toEqual(['0', '-1']);
  });
});
