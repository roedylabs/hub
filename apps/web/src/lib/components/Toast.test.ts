import { render } from '@testing-library/svelte';
import { describe, expect, it } from 'vitest';
import Toast from '#lib/components/Toast.svelte';

describe('toast', () => {
  it('announces a notice as status and an error as alert', () => {
    const ok = render(Toast, { toast: { id: 1, text: 'Saved', error: false }, isMobile: false });
    expect(ok.getByRole('status')).toHaveTextContent('Saved');
    ok.unmount();
    const err = render(Toast, { toast: { id: 2, text: 'Couldn’t save', error: true }, isMobile: true });
    expect(err.getByRole('alert')).toHaveTextContent('Couldn’t save');
  });

  it('sits above the bar "Unsaved changes" while there is one', async () => {
    const view = render(Toast, { toast: { id: 1, text: 'Saved', error: false }, isMobile: false, lift: 106 });
    expect(view.getByRole('status')).toHaveInlineStyle({ bottom: '106px' });
    await view.rerender({ toast: { id: 1, text: 'Saved', error: false }, isMobile: false, lift: null });
    expect(view.getByRole('status')).toHaveInlineStyle({ bottom: '' });
  });
});
