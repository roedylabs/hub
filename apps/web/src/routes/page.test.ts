import { fireEvent } from '@testing-library/svelte';
import { afterEach, describe, expect, test, vi } from 'vitest';
import type { HomeApp } from '#lib/apps.ts';
import { renderSignedIn, resetSignedIn } from '#lib/test/signed-in.ts';
import Page from './+page.svelte';

const nav = vi.hoisted(() => ({ goto: vi.fn(() => Promise.resolve()) }));
vi.mock('$app/navigation', () => ({ goto: nav.goto }));

const MAIL: HomeApp = {
  slug: 'mail',
  name: 'Mail',
  description: 'Email for the team',
  url: '/mail/',
  icon: '/api/v1/apps/mail/icon.svg?v=0123456789abcdef',
  status: 'online',
};
const NOTES: HomeApp = { ...MAIL, slug: 'notes', name: 'Notes', description: 'Shared notes', url: '/notes/' };

afterEach(() => {
  resetSignedIn();
  nav.goto.mockClear();
});

describe('home page', () => {
  test('greets and shows the apps as links to them, offline ones marked', async () => {
    const view = renderSignedIn(Page, {
      role: 'user',
      api: { 'GET apps': [{ body: { apps: [MAIL, { ...NOTES, status: 'offline' }] } }] },
    });
    expect(view.getByRole('heading', { level: 1, name: 'Hello, Alex' })).toBeInTheDocument();
    const mail = await view.findByRole('link', { name: 'Mail' });
    expect(mail).toHaveAttribute('href', '/mail/');
    expect(mail).toHaveAccessibleDescription('Email for the team');
    expect(mail.closest('li')?.querySelector('img')).toHaveAttribute('src', MAIL.icon);
    expect(view.getByRole('link', { name: 'Notes' })).toHaveAccessibleDescription('Shared notes Offline');
    expect(view.getAllByRole('listitem')).toHaveLength(2);
  });

  test('shows the empty state for a user without apps', async () => {
    const view = renderSignedIn(Page, { role: 'user', api: { 'GET apps': [{ body: { apps: [] } }] } });
    expect(await view.findByText('No apps yet')).toBeInTheDocument();
    expect(view.queryByRole('button', { name: 'Manage apps' })).toBeNull();
  });

  test('leads an administrator to the administration from the empty state and the pairing hint', async () => {
    const view = renderSignedIn(Page, {
      api: {
        'GET apps': [{ body: { apps: [] } }],
        'GET admin/apps': [{ body: { apps: [{ slug: 'wiki', state: 'pending', name: null }] } }],
      },
    });
    const note = await view.findByRole('note');
    expect(note).toHaveTextContent('wiki is waiting for pairing.');
    await fireEvent.click(view.getByRole('button', { name: 'Open' }));
    expect(nav.goto).toHaveBeenCalledWith('/admin/apps');
    await fireEvent.click(view.getByRole('button', { name: 'Manage apps' }));
    expect(nav.goto).toHaveBeenCalledTimes(2);
  });

  test('offers "Try again" when the apps cannot be loaded', async () => {
    const view = renderSignedIn(Page, {
      role: 'user',
      api: { 'GET apps': [{ status: 500, body: { error: 'internal' } }, { body: { apps: [MAIL] } }] },
    });
    const alert = await view.findByRole('alert');
    expect(alert).toHaveTextContent('Couldn’t load your apps');
    await fireEvent.click(view.getByRole('button', { name: 'Try again' }));
    expect(await view.findByRole('link', { name: 'Mail' })).toBeInTheDocument();
  });

  test('loads again when the tab becomes visible', async () => {
    const view = renderSignedIn(Page, {
      role: 'user',
      api: { 'GET apps': [{ body: { apps: [MAIL] } }, { body: { apps: [MAIL, NOTES] } }] },
    });
    await view.findByRole('link', { name: 'Mail' });
    document.dispatchEvent(new Event('visibilitychange'));
    expect(await view.findByRole('link', { name: 'Notes' })).toBeInTheDocument();
  });
});
