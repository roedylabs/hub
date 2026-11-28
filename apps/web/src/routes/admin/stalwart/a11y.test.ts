/**
 * axe-core over Administration › Stalwart servers in every state (loading,
 * load failed, list, diagnosis open, empty, dialog for adding with errors,
 * dialog for editing a server with a client), desktop and mobile, light and
 * dark. The dialog is rendered by the layout in the app; here the test renders
 * it next to the page. Colour contrast is left to the browser audit
 * (scripts/a11y/audit.mjs).
 */
import { fireEvent, render, screen, within } from '@testing-library/svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { ui } from '@roedy/ui';
import ServerDialog from '#lib/components/ServerDialog.svelte';
import { dialogValues, ServersPage, type AdminServer, type ServerDialogValues } from '#lib/servers.svelte.ts';
import { prefs } from '#lib/state/prefs.svelte.ts';
import { violations } from '@roedy/ui/testing';
import { renderSignedIn, resetSignedIn, setPhone, signIn, type ApiRoutes } from '#lib/test/signed-in.ts';
import Page from './+page.svelte';

vi.mock('$app/navigation', () => ({ goto: vi.fn(() => Promise.resolve()) }));

afterEach(() => {
  resetSignedIn();
  delete document.documentElement.dataset.mode;
});

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
const S1: AdminServer = {
  ...base,
  id: 's1',
  label: 'VPS 1',
  url: 'https://mail.s1.test',
  host: 'mail.s1.test',
  domains: ['s1.test'],
  client_id: 'roedy-abc123',
  has_secret: true,
  reach: 'ok',
  cors: 'err',
  oauth: 'ok',
  tested_at: new Date(Date.now() - 60_000).toISOString(),
  accounts: 2,
};
const DEV: AdminServer = {
  ...base,
  id: 'sdev',
  label: 'Development',
  url: 'http://localhost:8080',
  host: 'localhost:8080',
  domains: [],
};
const list = (servers: AdminServer[]) => ({
  body: { servers, redirect_uri: 'https://hub.example.com/oauth/callback', system_mail_server: 's1' },
});
const routes = (servers: AdminServer[] = [S1, DEV]): ApiRoutes => ({
  'GET admin/stalwart/servers': [list(servers)],
  'GET admin/stalwart/policy': [{ body: { allow_custom_servers: true } }],
});
const ready = () =>
  vi.waitFor(() => expect(screen.getByRole('heading', { level: 2, name: 'Policy' })).toBeInTheDocument());
const never = () => new Promise<never>(() => {});

const STATES: Record<string, { api: () => ApiRoutes; prepare?: () => Promise<unknown> }> = {
  loading: { api: () => ({ 'GET admin/stalwart/servers': [never], 'GET admin/stalwart/policy': [never] }) },
  failed: {
    api: () => ({
      'GET admin/stalwart/servers': [{ status: 500, body: { error: 'internal' } }],
      'GET admin/stalwart/policy': [{ body: { allow_custom_servers: false } }],
    }),
    prepare: () => vi.waitFor(() => expect(screen.getByRole('button', { name: 'Try again' })).toBeInTheDocument()),
  },
  list: { api: () => routes(), prepare: ready },
  diagnosis: {
    api: () => ({
      ...routes(),
      'POST admin/stalwart/servers/s1/test': [{ body: { server: S1, reasons: { cors: 'authorization_not_allowed' } } }],
    }),
    prepare: async () => {
      await ready();
      await fireEvent.click(screen.getAllByRole('button', { name: 'Test connection' })[0] as HTMLElement);
      // The finished diagnosis, not its "Testing …" state.
      await vi.waitFor(() => {
        const region = screen.getByRole('region');
        expect(within(region).queryByRole('status')).toBeNull();
        expect(within(region).getAllByText(/./).length).toBeGreaterThan(1);
      });
    },
  },
  empty: { api: () => routes([]), prepare: ready },
};

describe('Stalwart servers page – axe', () => {
  for (const mobile of [false, true])
    for (const mode of ['light', 'dark'] as const)
      for (const [state, { api, prepare }] of Object.entries(STATES))
        it(`${state}, ${mobile ? 'mobile' : 'desktop'}, ${mode}`, async () => {
          document.documentElement.dataset.mode = mode;
          prefs.set({ mode });
          setPhone(mobile);
          const view = renderSignedIn(Page, { path: '/admin/stalwart', api: api() });
          await prepare?.();
          expect(await violations(view)).toEqual([]);
        });
});

/** A page with the dialog open; on mobile the test renders only its content, as the layout's sheet shows it. */
function dialog(server: AdminServer | null): ServersPage {
  const p = new ServersPage();
  p.redirectUri = 'https://hub.example.com/oauth/callback';
  p.rows = [S1, DEV];
  p.openDialog(server);
  return p;
}

describe('Stalwart server dialog – axe', () => {
  const CASES: Record<string, [AdminServer | null, (p: ServersPage) => void | Promise<void>]> = {
    'add with errors': [
      null,
      async (p) => {
        p.setUrl('http://mail.example.com');
        p.domainInput('bad_domain s1.test ');
        p.setAdminUser('admin');
        await p.save();
      },
    ],
    'edit with client': [S1, () => {}],
    'register again': [S1, (p) => p.setAgain(true)],
    manual: [DEV, (p) => p.setOa('manual')],
  };
  for (const mobile of [false, true])
    for (const mode of ['light', 'dark'] as const)
      for (const [name, [server, change]] of Object.entries(CASES))
        it(`${name}, ${mobile ? 'mobile' : 'desktop'}, ${mode}`, async () => {
          signIn('admin');
          document.documentElement.dataset.mode = mode;
          prefs.set({ mode });
          setPhone(mobile);
          const p = dialog(server);
          await change(p);
          const view = render(ServerDialog, { sd: dialogValues(p) as ServerDialogValues, isMobile: mobile });
          expect(await violations(view)).toEqual([]);
          expect(ui.toast).toBeNull();
        });
});
