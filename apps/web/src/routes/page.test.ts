import { fireEvent, render, screen } from '@testing-library/svelte';
import { afterEach, describe, expect, test, vi } from 'vitest';
import { mockApi } from '#lib/state/fetch-mock.ts';
import { session } from '#lib/state/session.svelte.ts';
import { ui } from '#lib/state/ui.svelte.ts';
import { sessionOf } from '#lib/test/session.ts';
import Page from './+page.svelte';

const nav = vi.hoisted(() => ({ goto: vi.fn(() => Promise.resolve()) }));
vi.mock('$app/navigation', () => ({ goto: nav.goto }));

afterEach(() => {
  vi.unstubAllGlobals();
  nav.goto.mockClear();
  session.user = null;
  ui.reset();
});

describe('home page', () => {
  test('names the hub', () => {
    render(Page);
    expect(screen.getByRole('heading', { level: 1, name: 'Roedy Hub' })).toBeInTheDocument();
  });

  test('signs out and opens the sign-in', async () => {
    session.user = sessionOf({ id: '01J', email: 'alex@example.com', name: 'Alex Example' }).user;
    const { calls } = mockApi({ 'POST auth/logout': [{ status: 204 }] });
    render(Page);
    expect(screen.getByText('Signed in as Alex Example')).toBeInTheDocument();
    await fireEvent.click(screen.getByRole('button', { name: 'Sign out' }));
    await vi.waitFor(() => expect(nav.goto).toHaveBeenCalledWith('/login', { replace: true }));
    expect(calls.map((c) => [c.method, c.path, c.headers['X-Requested-With']])).toEqual([
      ['POST', 'auth/logout', 'fetch'],
    ]);
    expect(session.user).toBeNull();
  });

  test('stays signed in and says so when signing out fails', async () => {
    session.user = sessionOf({ id: '01J', email: 'alex@example.com', name: 'Alex Example' }).user;
    mockApi({});
    render(Page);
    await fireEvent.click(screen.getByRole('button', { name: 'Sign out' }));
    await vi.waitFor(() => expect(ui.toast?.error).toBe(true));
    expect(nav.goto).not.toHaveBeenCalled();
    expect(session.user).not.toBeNull();
  });
});
