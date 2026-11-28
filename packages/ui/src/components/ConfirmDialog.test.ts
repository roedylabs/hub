// Taken over from Roedy Mail (`apps/web/src/lib/components/ConfirmDialog.test.ts`).
import { fireEvent, render } from '@testing-library/svelte';
import { describe, expect, it, vi } from 'vitest';
import ConfirmDialog from './ConfirmDialog.svelte';
import { confirmValues, type ConfirmDialogValues } from '../shell.ts';

function values(patch: Partial<ConfirmDialogValues> = {}): ConfirmDialogValues {
  return {
    isMobile: false,
    cf: confirmValues(
      { title: 'Remove Jordan Hale?', text: 'Jordan can no longer sign in.', danger: true, ok: 'Remove user' },
      true),
    confirmIn: '',
    onConfirmIn: () => {},
    confirmKey: () => {},
    confirmOk: () => {},
    confirmCancel: () => {},
    onConfirmOverlay: () => {},
    ...patch,
  };
}

describe('confirmation', () => {
  it('derives `cf` from the request', () => {
    expect(confirmValues({ title: 'T', text: 'X' }, true)).toEqual({
      title: 'T',
      text: 'X',
      icon: 'ti-help-circle',
      ok: 'Confirm',
      hasCancel: true,
      hasWord: false,
      word: '',
      danger: false,
      okDis: false,
    });
    expect(
      confirmValues({ title: 'T', text: 'X', word: 'DELETE', noCancel: true }, false),
    ).toMatchObject({
      hasWord: true,
      word: 'DELETE',
      hasCancel: false,
      okDis: true,
    });
  });

  it('asks and focuses the button', async () => {
    const confirmOk = vi.fn();
    const confirmCancel = vi.fn();
    const { getByRole } = render(ConfirmDialog, values({ confirmOk, confirmCancel }));
    const dialog = getByRole('alertdialog', { name: 'Remove Jordan Hale?' });
    expect(dialog).toHaveAccessibleDescription('Jordan can no longer sign in.');
    expect(getByRole('button', { name: 'Remove user' })).toHaveFocus();
    await fireEvent.click(getByRole('button', { name: 'Remove user' }));
    expect(confirmOk).toHaveBeenCalled();
    await fireEvent.click(getByRole('button', { name: 'Cancel' }));
    expect(confirmCancel).toHaveBeenCalled();
  });

  it('requires the confirmation word', async () => {
    const onConfirmIn = vi.fn();
    const cf = confirmValues(
      { title: 'Delete account?', text: 'This is final.', word: 'DELETE' },
      false);
    const { getByRole, getByText } = render(ConfirmDialog, values({ cf, onConfirmIn }));
    expect(getByText('Type DELETE to continue.')).toBeInTheDocument();
    const input = getByRole('textbox', { name: 'Confirmation word' });
    expect(input).toHaveFocus();
    await fireEvent.input(input, { target: { value: 'del' } });
    expect(onConfirmIn).toHaveBeenCalled();
    expect(getByRole('button', { name: 'Confirm' })).toHaveAttribute('aria-disabled', 'true');
  });
});
