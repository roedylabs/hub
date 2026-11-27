import { fireEvent, within } from '@testing-library/svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { ui } from '#lib/state/ui.svelte.ts';
import { offlineApp, pairedApp, PAIRING, pendingApp } from '#lib/test/apps.ts';
import { page } from '#lib/test/page.svelte.ts';
import { renderSignedIn, resetSignedIn, type ApiRoutes } from '#lib/test/signed-in.ts';
import Page from './+page.svelte';

const nav = vi.hoisted(() => ({ goto: vi.fn(() => Promise.resolve()) }));
vi.mock('$app/navigation', () => ({ goto: nav.goto }));
vi.mock('$app/state', async () => ({ page: (await import('#lib/test/page.svelte.ts')).page }));

const USERS = {
  users: [
    { id: 'u1', status: 'active', name: 'Alex Example', email: 'alex@example.com', role: 'admin', app_role: 'admin' },
    { id: 'u2', status: 'locked', name: 'Sam Rivera', email: 'sam@example.com', role: 'user', app_role: 'user' },
    { id: 'u3', status: 'active', name: null, email: 'robin@example.com', role: 'user', app_role: null },
  ],
  invites: [{ id: 'i1', status: 'invited', name: null, email: 'kim@example.com', role: 'user', app_role: 'user' }],
};

afterEach(() => {
  resetSignedIn();
  nav.goto.mockClear();
});

function open(slug: string, api: ApiRoutes) {
  page.params = { slug };
  return renderSignedIn(Page, { path: `/admin/apps/${slug}`, api });
}

describe('Administration › Apps › an app', () => {
  it('shows a paired app: status, facts, roles and who has access', async () => {
    const view = open('mail', {
      'GET admin/apps/mail': [{ body: { app: pairedApp() } }],
      'GET admin/apps/mail/users': [{ body: USERS }],
    });
    expect(await view.findByRole('heading', { level: 1, name: 'Mail' })).toBeInTheDocument();
    expect(view.getByText('Email for the team')).toBeInTheDocument();
    expect(view.getByText('Online')).toBeInTheDocument();
    expect(view.getByRole('link', { name: '/mail/' })).toHaveAttribute('href', '/mail/');
    expect(view.getByText('http://mail.internal:8080')).toBeInTheDocument();
    expect(view.getByText('roedy.mail')).toBeInTheDocument();
    // Roles with their marks.
    expect(view.getByText('Default')).toBeInTheDocument();
    expect(view.getByText('Full control')).toBeInTheDocument();
    // Who has access: three of four, the locked user and the invitation marked.
    expect(await view.findByText('3 of 4 with access, including open invitations.')).toBeInTheDocument();
    const people = view.getAllByRole('list')[1] as HTMLElement;
    expect(
      within(people)
        .getAllByRole('listitem')
        .map((li) => li.textContent?.replace(/\s+/g, ' ').trim()),
    ).toEqual([
      'Alex Example alex@example.com Administrator',
      'Sam Rivera Suspended sam@example.com User',
      'kim@example.com Invited User',
    ]);
    expect(view.getByText('Without access (1)')).toBeInTheDocument();
    await fireEvent.click(view.getByRole('button', { name: 'Manage access' }));
    expect(nav.goto).toHaveBeenCalledWith('/admin/users');
  });

  it('changes the access default with the keyboard-accessible radio group', async () => {
    const view = open('mail', {
      'GET admin/apps/mail': [{ body: { app: pairedApp() } }],
      'GET admin/apps/mail/users': [{ body: USERS }],
      'PATCH admin/apps/mail': [{ body: { app: pairedApp({ access_default: 'selected' }) } }],
    });
    const group = await view.findByRole('radiogroup', { name: 'New users' });
    const all = within(group).getByRole('radio', { name: /Everyone/ });
    expect(all).toHaveAttribute('aria-checked', 'true');
    expect(all).toHaveTextContent('New users get access as User.');
    await fireEvent.click(within(group).getByRole('radio', { name: /Selected people only/ }));
    expect(within(group).getByRole('radio', { name: /Selected people only/ })).toHaveAttribute('aria-checked', 'true');
    await vi.waitFor(() => expect(ui.toast?.text).toBe('Default for new users saved'));
    expect(view.getByText(/Applies to new users only/)).toBeInTheDocument();
  });

  it('shows a pending app without access sections and makes a new pairing code', async () => {
    const view = open('wiki', {
      'GET admin/apps/wiki': [{ body: { app: pendingApp('2099-01-01T00:00:00Z') } }],
      'POST admin/apps/wiki/pairing-code': [{ body: { app: pendingApp('2099-01-01T00:00:00Z'), pairing: PAIRING } }],
    });
    expect(await view.findByRole('heading', { level: 1, name: 'wiki' })).toBeInTheDocument();
    expect(view.getByText('Waiting for pairing: give the app a pairing code.')).toBeInTheDocument();
    expect(view.queryByRole('heading', { name: 'Access' })).toBeNull();
    await fireEvent.click(view.getByRole('button', { name: 'New pairing code' }));
    const code = await view.findByRole('region', { name: 'Pairing code for wiki' });
    expect(code).toHaveTextContent(PAIRING.display);
  });

  it('marks an offline app and its failing event delivery', async () => {
    const view = open('notes', {
      'GET admin/apps/notes': [{ body: { app: offlineApp() } }],
      'GET admin/apps/notes/users': [{ body: { users: [], invites: [] } }],
    });
    expect(await view.findByText('Offline')).toBeInTheDocument();
    expect(view.getByRole('note')).toHaveTextContent(/Events can’t be delivered/);
  });

  it('removes the app after confirmation and goes back to the list', async () => {
    const view = open('mail', {
      'GET admin/apps/mail': [{ body: { app: pairedApp() } }],
      'GET admin/apps/mail/users': [{ body: USERS }],
      'DELETE admin/apps/mail': [{ status: 204 }],
    });
    const remove = await view.findAllByRole('button', { name: 'Remove app' });
    await fireEvent.click(remove[0] as HTMLElement);
    expect(ui.confirm?.title).toBe('Remove Mail?');
    ui.confirmOk();
    await vi.waitFor(() => expect(nav.goto).toHaveBeenCalledWith('/admin/apps'));
  });

  it('says when the app does not exist, and leads back to the list', async () => {
    const view = open('nope', { 'GET admin/apps/nope': [{ status: 404, body: { error: 'not_found' } }] });
    expect(await view.findByText('It may have been removed in the meantime.')).toBeInTheDocument();
    await fireEvent.click(view.getByRole('button', { name: 'Apps' }));
    expect(nav.goto).toHaveBeenCalledWith('/admin/apps');
  });

  it('offers "Try again" when the app cannot be loaded', async () => {
    const view = open('mail', {
      'GET admin/apps/mail': [{ status: 500, body: { error: 'internal' } }, { body: { app: pairedApp() } }],
      'GET admin/apps/mail/users': [{ body: USERS }],
    });
    expect(await view.findByRole('alert')).toHaveTextContent('Couldn’t load the app');
    await fireEvent.click(view.getByRole('button', { name: 'Try again' }));
    expect(await view.findByRole('heading', { level: 1, name: 'Mail' })).toBeInTheDocument();
  });
});
