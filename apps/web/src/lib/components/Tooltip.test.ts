// Taken over from Roedy Mail (`apps/web/src/lib/components/Tooltip.test.ts`).
import { render } from '@testing-library/svelte';
import { describe, expect, it, vi } from 'vitest';
import Tooltip from '#lib/components/Tooltip.svelte';

describe('Tooltip', () => {
  it('measures itself and reports the position', () => {
    const onplace = vi.fn();
    const tip = {
      text: 'Edit',
      side: 'top' as const,
      phase: 'measure' as const,
      warm: false,
      anchor: { x: 100, y: 100, w: 40, h: 40 },
    };
    const { getByRole } = render(Tooltip, { tip, onplace });
    expect(getByRole('tooltip')).toHaveTextContent('Edit');
    expect(getByRole('tooltip')).toHaveAttribute('id', 'sn-tip');
    // jsdom measures 0 × 0: centered above the element.
    expect(onplace).toHaveBeenCalledWith(expect.objectContaining({ phase: 'show', side: 'top', x: 120, y: 92 }));
  });
});
