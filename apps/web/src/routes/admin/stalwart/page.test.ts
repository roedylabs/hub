// Taken over from Roedy Mail (`apps/web/src/routes/admin/mail-servers/page.test.ts`) without the list kept for the
// session and the command palette.
import { fireEvent, screen, within } from '@testing-library/svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { ui } from '@roedy/ui';
import type { AdminServer } from '#lib/servers.svelte.ts';
import { renderSignedIn, resetSignedIn, setPhone, type ApiRoutes } from '#lib/test/signed-in.ts';
import Page from './+page.svelte';

vi.mock('$app/navigation', () => ({ goto: vi.fn(() => Promise.resolve()) }));

const NOW = Date.now();
const base = {
  client_id: null,
  has_secret: false,
  reach: 'unk',
  cors: 'unk',
  oauth: 'unk',
  tested_at: null,
  accounts: 0,
  created_at: '2026-11-01T00:00:00Z',
} as const;
const SERVERS: AdminServer[] = [
  {
    ...base,
    id: 's1',
    label: 'VPS 1',
    url: 'https://mail.s1.test',
    host: 'mail.s1.test',
    domains: ['s1.test'],
    client_id: 'roedy-abc123',
    has_secret: true,
    reach: 'ok',
    cors: 'ok',
    oauth: 'ok',
    tested_at: new Date(NOW - 25 * 60_000).toISOString(),
    accounts: 1,
  },
  { ...base, id: 'sdev', label: 'Development', url: 'http://localhost:8080', host: 'localhost:8080', domains: [] },
];
const LIST = { servers: SERVERS, redirect_uri: 'https://hub.example.com/oauth/callback', system_mail_server: 's1' };
const POLICY = { allow_custom_servers: false };

afterEach(() => resetSignedIn());

function renderPage(routes: ApiRoutes = {}) {
  return renderSignedIn(Page, {
    path: '/admin/stalwart',
    api: {
      'GET admin/stalwart/servers': [{ body: LIST }, { body: LIST }],
      'GET admin/stalwart/policy': [{ body: POLICY }, { body: POLICY }],
      ...routes,
    },
  });
}

describe('Stalwart servers page', () => {
  it('shows heading, tabs, servers with badges and the policy', async () => {
    const view = renderPage();
    expect(view.getByRole('heading', { level: 1 })).toHaveTextContent('Stalwart servers');
    expect(view.getByText('Mail servers, OAuth clients, and custom servers.')).toBeInTheDocument();
    expect(await view.findByText('VPS 1')).toBeInTheDocument();
    expect(view.getByRole('button', { name: 'Policy' })).toBeInTheDocument();
    expect(view.getByText('2 servers')).toBeInTheDocument();
    expect(view.getByText('System emails')).toBeInTheDocument();
    expect(view.getByRole('img', { name: 'Unencrypted. Only for development on this machine.' })).toBeInTheDocument();
    expect(view.getByRole('img', { name: 'Reachable: OK' })).toBeInTheDocument();
    expect(view.getAllByRole('img', { name: 'Reachable: unknown' })).toHaveLength(1);
    expect(view.getByText('https://mail.s1.test · 1 linked account')).toBeInTheDocument();
    expect(view.getByText('None. Users choose the server themselves.')).toBeInTheDocument();
    expect(view.getByText('Last tested 25 min ago')).toBeInTheDocument();
    expect(view.getByRole('switch', { name: 'Users can connect accounts on other servers' })).toHaveAttribute(
      'aria-checked',
      'false',
    );
    expect(view.getByText('https://hub.example.com/oauth/callback')).toBeInTheDocument();
  });

  it('tests the connection and expands the diagnosis', async () => {
    const view = renderPage({
      'POST admin/stalwart/servers/sdev/test': [
        {
          body: {
            server: { ...SERVERS[1], reach: 'err', tested_at: new Date(NOW).toISOString() },
            reasons: { reach: 'unreachable' },
          },
        },
      ],
    });
    await view.findByText('Development');
    const buttons = view.getAllByRole('button', { name: 'Test connection' });
    await fireEvent.click(buttons[1] as HTMLElement);
    const region = await view.findByRole('region', { name: 'Diagnostics for Development' });
    await vi.waitFor(() => expect(within(region).getByText('JMAP session')).toBeInTheDocument());
    expect(within(region).getAllByText('Not checked because the server isn’t reachable.')).toHaveLength(2);
    expect(ui.toast?.text).toBe('Development: test finished with warnings');
    await fireEvent.click(view.getByRole('button', { name: 'Hide diagnostics' }));
    expect(view.queryByRole('region')).toBeNull();
  });

  it('asks before removing and toggles the policy', async () => {
    const view = renderPage({
      'DELETE admin/stalwart/servers/sdev': [{ status: 204 }],
      'PUT admin/stalwart/policy': [{ body: { allow_custom_servers: true } }],
    });
    await view.findByText('Development');
    await fireEvent.click(view.getByRole('button', { name: 'Remove Development' }));
    expect(ui.confirm?.title).toBe('Remove “Development”?');
    ui.confirmOk();
    await vi.waitFor(() => expect(view.queryByText('Development')).toBeNull());
    await fireEvent.click(view.getByRole('switch'));
    expect(view.getByRole('switch')).toHaveAttribute('aria-checked', 'true');
    expect(ui.toast?.text).toBe('Other servers allowed');
  });

  it('passes the dialog values to the layout', async () => {
    const view = renderPage();
    await view.findByText('VPS 1');
    expect(ui.pageDialog?.()).toBeNull();
    await fireEvent.click(view.getAllByRole('button', { name: 'Add server' })[0] as HTMLElement);
    expect(ui.pageDialog?.()?.title).toBe('Add server');
    ui.pageDialog?.()?.close();
    expect(ui.pageDialog?.()).toBeNull();
    await fireEvent.click(view.getByRole('button', { name: 'Edit VPS 1' }));
    expect(ui.pageDialog?.()?.title).toBe('Edit server');
    view.unmount();
    expect(ui.pageDialog).toBeNull();
  });

  it('shows "No Stalwart server yet" without servers', async () => {
    const empty = { body: { ...LIST, servers: [], system_mail_server: null } };
    const view = renderPage({ 'GET admin/stalwart/servers': [empty] });
    expect(await view.findByText('No Stalwart server yet')).toBeInTheDocument();
    await fireEvent.click(view.getByRole('button', { name: 'Add your first server' }));
    expect(ui.pageDialog?.()?.title).toBe('Add server');
  });

  it('shows host, "Unencrypted" notice and actions below the row on mobile', async () => {
    setPhone(true);
    const view = renderPage();
    await view.findByText('VPS 1');
    expect(view.getByText('mail.s1.test · 1 linked account')).toBeInTheDocument();
    expect(view.getByText('Unencrypted. Only for development on this machine.')).toBeInTheDocument();
    expect(view.getByText('Users connect their accounts to these servers.')).toBeInTheDocument();
    expect(view.getByText('Allow other servers')).toBeInTheDocument();
    expect(view.queryByText('2 servers')).toBeNull();
    expect(view.getAllByRole('button', { name: 'Add server' })).toHaveLength(1);
    // The system mail's server and its stored diagnosis, as on the desktop.
    expect(view.getByText('System emails')).toBeInTheDocument();
    const show = view.getByRole('button', { name: 'Show diagnostics' });
    expect(show).toHaveAttribute('aria-expanded', 'false');
    await fireEvent.click(show);
    expect(view.getByRole('button', { name: 'Hide diagnostics' })).toHaveAttribute('aria-expanded', 'true');
    expect(view.getByRole('region')).toBeInTheDocument();
  });

  it('shows the skeleton until the first response, without list and policy', () => {
    const view = renderSignedIn(Page, {
      path: '/admin/stalwart',
      api: {
        'GET admin/stalwart/servers': [() => new Promise(() => {})],
        'GET admin/stalwart/policy': [{ body: POLICY }],
      },
    });
    expect(view.getByRole('heading', { level: 1 })).toHaveTextContent('Stalwart servers');
    expect(view.queryByRole('heading', { level: 2 })).toBeNull();
    expect(view.queryByRole('button')).toBeNull();
  });

  it('shows the card instead of list and policy when the first load fails, and reloads', async () => {
    const view = renderPage({
      'GET admin/stalwart/servers': [{ status: 500, body: { error: 'internal' } }, { body: LIST }],
    });
    const card = await view.findByRole('alert');
    expect(card).toHaveTextContent('Couldn’t load Stalwart servers');
    expect(view.queryByRole('heading', { level: 2 })).toBeNull();
    await fireEvent.click(screen.getByRole('button', { name: 'Try again' }));
    expect(await view.findByText('VPS 1')).toBeInTheDocument();
  });
});
