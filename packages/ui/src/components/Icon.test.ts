// Taken over from Roedy Mail (`apps/web/src/lib/components/Icon.test.ts`).
import { render } from '@testing-library/svelte';
import IconPuzzle from '@tabler/icons-svelte-runes/icons/puzzle';
import { describe, expect, it } from 'vitest';
import { isIconName } from '../icons.ts';
import Icon from './Icon.svelte';

describe('Icon', () => {
  it('is decorative without a label', () => {
    const { container } = render(Icon, { name: 'ti-lock' });
    const svg = container.querySelector('svg')!;
    expect(svg).toHaveAttribute('aria-hidden', 'true');
    expect(svg).toHaveAttribute('width', '1em');
    expect(svg).toHaveAttribute('stroke-width', '2');
    expect(svg).toHaveClass('tabler-icon', 'tabler-icon-lock');
  });

  it('is announced as an image with a label', () => {
    const { getByRole } = render(Icon, { name: 'ti-bug', label: 'Error', size: 24 });
    expect(getByRole('img', { name: 'Error' })).toHaveAttribute('width', '24');
  });

  it('shows an icon outside the registry from its component', () => {
    const { container } = render(Icon, { name: IconPuzzle });
    expect(container.querySelector('svg')).toHaveClass('tabler-icon-puzzle');
  });
});

describe('isIconName', () => {
  it('knows only registered icons', () => {
    expect(isIconName('ti-mountain')).toBe(true);
    expect(isIconName('ti-does-not-exist')).toBe(false);
    expect(isIconName('toString')).toBe(false);
  });
});
