import { fireEvent, render, screen } from '@testing-library/svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { mockApi } from '#lib/state/fetch-mock.ts';
import { prefs } from '#lib/state/prefs.svelte.ts';
import { BRAND0, session } from '#lib/state/session.svelte.ts';
import { ui } from '#lib/state/ui.svelte.ts';
import { sessionOf } from '#lib/test/session.ts';
import Page from './+page.svelte';

const nav = vi.hoisted(() => ({ goto: vi.fn(() => Promise.resolve()) }));
vi.mock('$app/navigation', () => ({ goto: nav.goto }));
// The reactive stand-in, so that a new token reaches the mounted page like a navigation.
vi.mock('$app/state', async () => ({ page: (await import('#lib/test/page.svelte.ts')).page }));
const { page } = await import('#lib/test/page.svelte.ts');

const SECRET = 'JBSWY3DPEHPK3PXPJBSWY3DPEHPK3PXP';

afterEach(() => {
  vi.unstubAllGlobals();
  nav.goto.mockClear();
  session.config = null;
  session.user = null;
  session.status = 'idle';
  prefs.reset();
  ui.reset();
});

/** Answers for an invitation that is accepted without an app ("Set up later"). */
function invitation(token: string, email: string) {
  return {
    [`GET invites/${token}`]: [
      {
        body: {
          email,
          name: 'Jonas Schulz',
          role: 'user',
          inviter: null,
          expires_at: '2026-12-03T10:00:00Z',
          expired: false,
        },
      },
    ],
    [`POST invites/${token}/totp`]: [{ body: { secret: SECRET, uri: `otpauth://totp/x?secret=${SECRET}` } }],
    [`POST invites/${token}/accept`]: [
      { status: 201, body: { user: { id: token, email, display_name: 'Jonas Schulz' }, recovery_codes: null } },
    ],
  };
}

async function acceptWithoutApp() {
  await fireEvent.click(await screen.findByRole('button', { name: 'Accept invitation' }));
  await fireEvent.input(screen.getByLabelText('Choose a password'), { target: { value: 'Safe-enough1' } });
  await fireEvent.input(screen.getByLabelText('Confirm password'), { target: { value: 'Safe-enough1' } });
  await fireEvent.click(screen.getByRole('button', { name: 'Continue' }));
  await fireEvent.click(await screen.findByRole('button', { name: 'Set up later' }));
  await vi.waitFor(() => expect(screen.getByRole('heading', { level: 1 })).toHaveTextContent('Welcome, Jonas'));
}

describe('Invitation page: the accounts of several invitations', () => {
  it('adopts the second account after the first one’s load has failed', async () => {
    let first: (a: { status: number; body: unknown }) => void = () => {};
    const { calls } = mockApi({
      ...invitation('a', 'a@example.com'),
      ...invitation('b', 'b@example.com'),
      'GET settings': [{ body: { version: 1, settings: {} } }, { body: { version: 1, settings: {} } }],
      'GET session': [
        () => new Promise((resolve) => (first = resolve)),
        { body: sessionOf({ id: 'b', email: 'b@example.com', name: 'Jonas Schulz' }) },
      ],
    });
    session.config = { name: 'Roedy Hub', branding: BRAND0, org: {} as never, setup_required: false };
    page.params = { token: 'a' };
    const view = render(Page);
    await acceptWithoutApp();

    // Another invitation in the same page: A's account starts to load, B is accepted meanwhile, then the page goes.
    page.params = { token: 'b' };
    await vi.waitFor(() => expect(calls.map((c) => c.path)).toContain('session'));
    await acceptWithoutApp();
    view.unmount();

    // A's load fails: B's adoption follows instead of being lost.
    first({ status: 500, body: { error: 'internal' } });
    await vi.waitFor(() => expect(session.user?.email).toBe('b@example.com'));
    expect(calls.filter((c) => c.path === 'session')).toHaveLength(2);
  });

  it('keeps a queued account when a third invitation opens before the pending load ends', async () => {
    let first: (a: { status: number; body: unknown }) => void = () => {};
    const { calls } = mockApi({
      ...invitation('a', 'a@example.com'),
      ...invitation('b', 'b@example.com'),
      ...invitation('c', 'c@example.com'),
      'GET settings': [{ body: { version: 1, settings: {} } }, { body: { version: 1, settings: {} } }],
      'GET session': [
        () => new Promise((resolve) => (first = resolve)),
        { body: sessionOf({ id: 'b', email: 'b@example.com', name: 'Jonas Schulz' }) },
      ],
    });
    session.config = { name: 'Roedy Hub', branding: BRAND0, org: {} as never, setup_required: false };
    page.params = { token: 'a' };
    const view = render(Page);
    await acceptWithoutApp();
    page.params = { token: 'b' };
    await vi.waitFor(() => expect(calls.map((c) => c.path)).toContain('session'));
    await acceptWithoutApp();

    // Invitation C opens while A's load still runs: B's account stays queued.
    page.params = { token: 'c' };
    await screen.findByRole('button', { name: 'Accept invitation' });
    first({ status: 500, body: { error: 'internal' } });
    await vi.waitFor(() => expect(session.user?.email).toBe('b@example.com'));
    expect(calls.filter((c) => c.path === 'session')).toHaveLength(2);
    view.unmount();
  });

  it('loads an account accepted while the previous one’s load still runs', async () => {
    let first: (a: { status: number; body: unknown }) => void = () => {};
    let second: (a: { status: number; body: unknown }) => void = () => {};
    const settings = { body: { version: 1, settings: {} } };
    const { calls } = mockApi({
      ...invitation('a', 'a@example.com'),
      ...invitation('b', 'b@example.com'),
      ...invitation('c', 'c@example.com'),
      'GET settings': [settings, settings, settings],
      'GET session': [
        () => new Promise((resolve) => (first = resolve)),
        () => new Promise((resolve) => (second = resolve)),
        { body: sessionOf({ id: 'c', email: 'c@example.com', name: 'Jonas Schulz' }) },
      ],
    });
    session.config = { name: 'Roedy Hub', branding: BRAND0, org: {} as never, setup_required: false };
    page.params = { token: 'a' };
    const view = render(Page);
    await acceptWithoutApp();
    page.params = { token: 'b' };
    await vi.waitFor(() => expect(calls.filter((c) => c.path === 'session')).toHaveLength(1));
    await acceptWithoutApp();
    page.params = { token: 'c' };
    await screen.findByRole('button', { name: 'Accept invitation' });

    // A's load fails: B's load starts. C is accepted while it runs, then the page goes.
    first({ status: 500, body: { error: 'internal' } });
    await vi.waitFor(() => expect(calls.filter((c) => c.path === 'session')).toHaveLength(2));
    await acceptWithoutApp();
    view.unmount();

    // B's load brings B's account, but C came after it started: C's load follows.
    second({ status: 200, body: sessionOf({ id: 'b', email: 'b@example.com', name: 'Jonas Schulz' }) });
    await vi.waitFor(() => expect(session.user?.email).toBe('c@example.com'));
  });
});
