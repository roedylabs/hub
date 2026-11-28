// Taken over from Roedy Mail (`apps/web/src/lib/components/DebugBar.test.ts`) with the props `text` and `bottom`.
import { cleanup, render } from '@testing-library/svelte';
import { afterEach, describe, expect, it } from 'vitest';
import type { DebugBarValues } from '../developer.ts';
import DebugBar from './DebugBar.svelte';

const VALUES: DebugBarValues = { text: 'Desktop · 1440×900 · /admin/developer', bottom: '18px' };

afterEach(cleanup);

describe('DebugBar', () => {
  it('shows the badge and the environment', () => {
    const { container } = render(DebugBar, VALUES);
    expect(container.querySelector('.tag')).toHaveTextContent('DEBUG');
    expect(container.querySelector('.text')).toHaveTextContent(VALUES.text);
  });

  it('stays hidden from screen readers', () => {
    const { container } = render(DebugBar, VALUES);
    expect(container.querySelector('.bar')).toHaveAttribute('aria-hidden', 'true');
  });

  it('sits at the `bottom` distance above the bottom edge', async () => {
    const { container, rerender } = render(DebugBar, VALUES);
    const bar = container.querySelector<HTMLElement>('.bar');
    expect(bar?.style.bottom).toBe('18px');
    await rerender({ ...VALUES, bottom: '90px' });
    expect(bar?.style.bottom).toBe('90px');
  });
});
