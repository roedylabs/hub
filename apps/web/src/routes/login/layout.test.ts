// Taken over from Roedy Mail (`apps/web/src/routes/(auth)/layout.test.ts`).
import { fireEvent, render } from '@testing-library/svelte';
import { createRawSnippet } from 'svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { session } from '#lib/state/session.svelte.ts';
import { ui } from '@roedy/ui';
import { sessionOf } from '#lib/test/session.ts';
import Layout from './+layout.svelte';

const nav = vi.hoisted(() => ({
  goto: vi.fn(() => Promise.resolve()),
  page: { url: new URL('http://localhost/login'), params: {} },
}));
vi.mock('$app/navigation', () => ({ goto: nav.goto }));
vi.mock('$app/state', () => ({ page: nav.page }));

const empty = createRawSnippet(() => ({ render: () => '<span></span>' }));

function renderAt(path: string, params: Record<string, string> = {}) {
  nav.page.url = new URL(`http://localhost${path}`);
  nav.page.params = params;
  return render(Layout, { children: empty, data: {}, params } as never);
}

afterEach(() => {
  nav.goto.mockClear();
  session.user = null;
  ui.reset();
});

describe('Login routes', () => {
  it('selects the step by the address', () => {
    expect(renderAt('/login').getByRole('button', { name: 'Sign in' })).toBeInTheDocument();
    expect(renderAt('/login/forgot').getByRole('heading', { name: 'Reset password' })).toBeInTheDocument();
    const token = 'd'.repeat(64);
    expect(
      renderAt(`/login/reset/${token}`, { token }).getByRole('heading', { name: 'Set a new password' }),
    ).toBeInTheDocument();
  });

  it('follows step changes with the address', async () => {
    const view = renderAt('/login');
    await fireEvent.click(view.getByRole('link', { name: 'Forgot password?' }));
    // Without resetting the focus (`reset: false`), otherwise it would go to <body>.
    expect(nav.goto).toHaveBeenCalledWith('/login/forgot', { reset: false });
  });

  it('keeps `next` and `reauth` on the way to "Reset password" and back', async () => {
    const view = renderAt('/login?next=%2Fmail%2F');
    await fireEvent.click(view.getByRole('link', { name: 'Forgot password?' }));
    expect(nav.goto).toHaveBeenCalledWith('/login/forgot?next=%2Fmail%2F', { reset: false });
  });

  it('leads from /login/forgot to the home page when signed in', async () => {
    session.user = sessionOf({ id: '01J', email: 'alex@example.net', name: 'Alex Example' }).user;
    const view = renderAt('/login/forgot');
    await fireEvent.click(view.getByRole('button', { name: 'Back to Roedy Hub' }));
    expect(nav.goto).toHaveBeenCalledWith('/');
  });
});
