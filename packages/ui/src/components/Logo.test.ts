// Taken over from Roedy Mail (`apps/web/src/lib/components/Logo.test.ts`).
import { render } from '@testing-library/svelte';
import { describe, expect, it } from 'vitest';
import Logo from './Logo.svelte';

describe('Logo', () => {
  it('shows the hub’s mark by default: four tiles in the ink colour, the first one in full', () => {
    const { container } = render(Logo);
    const svg = container.querySelector('svg.mark');
    expect(svg).toHaveAttribute('viewBox', '0 0 48 48');
    expect(svg).toHaveAttribute('fill', 'currentColor');
    expect(svg).toHaveAttribute('aria-hidden', 'true');
    const tiles = [...container.querySelectorAll('svg.mark > rect')];
    expect(tiles.map((t) => [t.getAttribute('x'), t.getAttribute('y'), t.getAttribute('opacity')])).toEqual([
      ['8', '8', null],
      ['26', '8', '.55'],
      ['8', '26', '.55'],
      ['26', '26', '.55'],
    ]);
    // Mail's letters with their masks are gone.
    expect(container.querySelector('mask')).toBeNull();
  });

  it('shows the initial in the given size', () => {
    const { getByText, container } = render(Logo, { logo: { isInit: true, text: 'RM', fs: '47cqmin' } });
    expect(getByText('RM')).toHaveInlineStyle({ fontSize: '47cqmin' });
    expect(container.querySelector('svg')).toBeNull();
  });

  it('shows known icons and skips unknown ones', () => {
    expect(
      render(Logo, { logo: { isIcon: true, icon: 'ti-mountain' } }).container.querySelector('.tabler-icon-mountain'),
    ).not.toBeNull();
    expect(render(Logo, { logo: { isIcon: true, icon: 'ti-unknown' } }).container.querySelector('svg')).toBeNull();
  });

  it('sizes the icon when asked', () => {
    const { container } = render(Logo, { logo: { isIcon: true, icon: 'ti-mountain', iconSize: '76px' } });
    expect(container.querySelector('.glyph')).toHaveInlineStyle({ width: '76px', height: '76px' });
  });

  it('shows images filled or with a border', () => {
    const { container } = render(Logo, { logo: { isPad: true, src: '/logo.png' } });
    expect(container.querySelector('img')).toHaveClass('pad');
    expect(container.querySelector('img')).toHaveAttribute('src', '/logo.png');
  });
});
