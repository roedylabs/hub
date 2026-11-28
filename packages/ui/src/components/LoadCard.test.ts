import { fireEvent, render } from '@testing-library/svelte';
import { describe, expect, it, vi } from 'vitest';
import LoadCard from './LoadCard.svelte';

describe('load card', () => {
  it('reports a failed load as an alert with "Try again"', async () => {
    const onRetry = vi.fn();
    const { getByRole } = render(LoadCard, { title: 'Couldn’t load sessions', text: 'No connection.', onRetry });
    const card = getByRole('alert');
    expect(card).toHaveTextContent('Couldn’t load sessions');
    expect(card).toHaveTextContent('No connection.');
    await fireEvent.click(getByRole('button', { name: 'Try again' }));
    expect(onRetry).toHaveBeenCalledOnce();
  });

  it('is a quiet empty state without `onRetry`', () => {
    const { queryByRole, getByText } = render(LoadCard, {
      icon: 'ti-users',
      title: 'No users yet',
      text: 'Invite one.',
    });
    expect(queryByRole('alert')).toBeNull();
    expect(queryByRole('button')).toBeNull();
    expect(getByText('No users yet')).toBeInTheDocument();
  });
});
