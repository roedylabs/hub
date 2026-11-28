import { render } from '@testing-library/svelte';
import { describe, expect, it } from 'vitest';
import AccountTile from './AccountTile.svelte';

describe('AccountTile', () => {
  it('shows the first letter in the account colour, hidden from assistive technology by default', () => {
    const { container } = render(AccountTile, { email: 'sam@example.com', color: 1, size: 32 });
    const tile = container.querySelector('.tile');
    expect(tile).toHaveTextContent('S');
    expect(tile).toHaveAttribute('aria-hidden', 'true');
    expect(tile).toHaveStyle({ background: '#94c48d', color: '#13300f' });
  });

  it('is an image with a name when given a label', () => {
    const { getByRole } = render(AccountTile, { email: 'sam@example.com', color: 0, label: 'sam@example.com' });
    expect(getByRole('img', { name: 'sam@example.com' })).toHaveTextContent('S');
  });
});
