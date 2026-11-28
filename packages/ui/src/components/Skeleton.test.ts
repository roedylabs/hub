// Taken over from Roedy Mail (`apps/web/src/lib/components/Skeleton.test.ts`), trimmed to the settings kind.
import { render } from '@testing-library/svelte';
import { describe, expect, it } from 'vitest';
import Skeleton from './Skeleton.svelte';

describe('skeleton', () => {
  it.each([false, true])('reports loading as a status (mobile: %s)', (isMobile) => {
    const { getAllByRole } = render(Skeleton, { isMobile });
    expect(getAllByRole('status').map((s) => s.getAttribute('aria-label'))).toEqual(['Loading data from the server']);
  });

  it('shows three rows and "Loading data …" by default', () => {
    const { getByText, container } = render(Skeleton, { set: 'list' });
    expect(getByText('Loading data …')).toBeInTheDocument();
    expect(container.querySelectorAll('.list-row')).toHaveLength(3);
  });

  it('shows a custom hint and the overview', () => {
    const { getByText, container } = render(Skeleton, { set: 'ueb', note: 'Loading users …' });
    expect(getByText('Loading users …')).toBeInTheDocument();
    expect(container.querySelector('.ueb-card')).not.toBeNull();
    expect(container.querySelectorAll('.list-row')).toHaveLength(0);
  });
});
