import { render } from '@testing-library/svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { page } from '#lib/test/page.svelte.ts';
import ErrorPage from './+error.svelte';

vi.mock('$app/state', () => ({ page }));

afterEach(() => {
  page.status = 200;
  document.head.querySelector('meta[name="roedy-error"]')?.remove();
});

function gateway(code: string) {
  const meta = document.createElement('meta');
  meta.name = 'roedy-error';
  meta.content = code;
  document.head.append(meta);
}

describe('error page of the router and the gateway', () => {
  it('shows the router’s 404 for an unknown address', () => {
    page.status = 404;
    const view = render(ErrorPage);
    expect(view.getByRole('heading', { level: 1, name: 'Page not found' })).toBeInTheDocument();
  });

  it('shows any other router error as 500', () => {
    page.status = 418;
    const view = render(ErrorPage);
    expect(view.getByRole('heading', { level: 1, name: 'Something went wrong' })).toBeInTheDocument();
  });

  it.each([
    ['no_app_access', 'No access to this app'],
    ['origin_mismatch', 'Access denied'],
    ['app_unreachable', 'App not reachable'],
    ['app_not_ready', 'App not ready yet'],
    ['app_disabled', 'App switched off'],
    ['app_timeout', 'The app took too long'],
    ['not_found', 'Page not found'],
  ])('shows the gateway’s error %s at the same address', (code, title) => {
    gateway(code);
    page.status = 404;
    const view = render(ErrorPage);
    expect(view.getByRole('heading', { level: 1, name: title })).toBeInTheDocument();
  });

  it('ignores an unknown code of the gateway', () => {
    gateway('<script>');
    page.status = 404;
    const view = render(ErrorPage);
    expect(view.getByRole('heading', { level: 1, name: 'Page not found' })).toBeInTheDocument();
  });
});
