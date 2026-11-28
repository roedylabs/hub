// Taken over from Roedy Mail (`apps/web/src/lib/components/ServerDialog.test.ts`) with the hub's registration on
// saving.
import { fireEvent, render } from '@testing-library/svelte';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { dialogValues, ServersPage, type AdminServer, type ServerDialogValues } from '#lib/servers.svelte.ts';
import { resetSignedIn, signIn } from '#lib/test/signed-in.ts';
import ServerDialog from './ServerDialog.svelte';

const S2: AdminServer = {
  id: 's2',
  label: 'VPS 2',
  url: 'https://mail.s2.test',
  host: 'mail.s2.test',
  domains: ['s2.test'],
  client_id: 'c2',
  has_secret: false,
  reach: 'ok',
  cors: 'ok',
  oauth: 'ok',
  tested_at: null,
  accounts: 1,
  created_at: '2026-11-01T00:00:00Z',
};

beforeEach(() => signIn('admin'));
afterEach(() => resetSignedIn());

/** Dialog with the values of a page; `sync()` takes over changes to the state. */
function open(server: AdminServer | null, isMobile = false, register = false) {
  const p = new ServersPage();
  p.redirectUri = 'https://hub.example.com/oauth/callback';
  p.openDialog(server, register);
  const sd = () => dialogValues(p) as ServerDialogValues;
  const view = render(ServerDialog, { sd: sd(), isMobile });
  const sync = () => view.rerender({ sd: sd(), isMobile });
  return { p, view, sync };
}

describe('ServerDialog', () => {
  it('shows fields, the credentials and the redirect URI on desktop; input goes to the state', async () => {
    const { p, view, sync } = open(null);
    expect(view.getByRole('dialog', { name: 'Add server' })).toBeInTheDocument();
    expect(view.getByText('Stalwart server that users can connect to')).toBeInTheDocument();
    const label = view.getByLabelText('Label');
    expect(document.activeElement).toBe(label);
    await fireEvent.input(label, { target: { value: 'New' } });
    await fireEvent.input(view.getByLabelText('Add domain'), { target: { value: 'new.test,' } });
    expect(p.dialog).toMatchObject({ label: 'New', domains: ['new.test'], domIn: '' });
    await sync();
    expect(view.getByRole('button', { name: 'Remove new.test' })).toBeInTheDocument();
    expect(view.getByRole('radiogroup', { name: 'OAuth client' })).toBeInTheDocument();
    expect(view.getByRole('radio', { name: 'Automatic' })).toHaveAttribute('aria-checked', 'true');
    expect(view.getByLabelText('Stalwart admin user')).toBeInTheDocument();
    await fireEvent.input(view.getByLabelText('Administrator’s app password'), { target: { value: 'pw' } });
    await sync();
    expect(view.getByText(/^Sign in once with the username and app password/)).toBeInTheDocument();
    expect(view.getByText('https://hub.example.com/oauth/callback')).toBeInTheDocument();
    await fireEvent.click(view.getByRole('button', { name: 'Add and test' }));
    await sync();
    expect(view.getAllByRole('alert').map((a) => a.textContent?.trim())).toEqual([
      'The URL must start with https://, e.g. https://mail.example.com.',
      'Please enter the Stalwart server’s admin user and app password, or leave both empty.',
    ]);
    await fireEvent.input(view.getByLabelText('Stalwart admin user'), { target: { value: 'admin' } });
    await sync();
    expect(view.getByRole('button', { name: 'Add and register' })).toBeInTheDocument();
    await fireEvent.click(view.getByRole('button', { name: 'Cancel' }));
    expect(p.dialog).toBeNull();
  });

  it('shows a manual client and toggles the secret', async () => {
    const { p, view, sync } = open(S2);
    expect(view.getByRole('dialog', { name: 'Edit server' })).toBeInTheDocument();
    expect(view.getByRole('radio', { name: 'Manual' })).toHaveAttribute('aria-checked', 'true');
    expect(view.getByLabelText('Client ID')).toHaveValue('c2');
    const secret = view.getByLabelText('Client secret (optional)');
    expect(secret).toHaveAttribute('type', 'password');
    await fireEvent.click(view.getByRole('button', { name: 'Show' }));
    await sync();
    expect(secret).toHaveAttribute('type', 'text');
    await fireEvent.input(view.getByLabelText('Client ID'), { target: { value: 'c3' } });
    await sync();
    expect(view.getByText('With a different client ID, the linked account has to sign in again.')).toBeInTheDocument();
    expect(view.getByRole('button', { name: 'Save and test' })).toBeInTheDocument();
    // A click in the dialog does not close, one on the backdrop does.
    await fireEvent.mouseDown(view.getByRole('dialog'));
    expect(p.dialog).not.toBeNull();
    await fireEvent.mouseDown(view.getByRole('presentation'));
    expect(p.dialog).toBeNull();
  });

  it('shows the stored client and "Register again"', async () => {
    const { p, view, sync } = open({ ...S2, has_secret: true });
    expect(view.getByText('OAuth client set up')).toBeInTheDocument();
    expect(view.getByText('Client ID c2 · Secret stored in the hub')).toBeInTheDocument();
    expect(view.queryByLabelText('Stalwart admin user')).toBeNull();
    await fireEvent.click(view.getByRole('button', { name: 'Register again' }));
    await sync();
    expect(view.getByLabelText('Stalwart admin user')).toBeInTheDocument();
    expect(view.getByText('With a new client, the linked account has to sign in again.')).toBeInTheDocument();
    await fireEvent.click(view.getByRole('button', { name: 'Keep client' }));
    await sync();
    expect(p.dialog?.again).toBe(false);
    expect(view.getByText('OAuth client set up')).toBeInTheDocument();
  });

  it('shows only the sheet content on mobile, with the unencrypted notice', async () => {
    const { p, view, sync } = open(null, true);
    expect(view.queryByRole('dialog')).toBeNull();
    expect(view.getByRole('heading', { name: 'Add server' })).toBeInTheDocument();
    expect(view.getByText('Suggested for addresses at these domains.')).toBeInTheDocument();
    expect(view.getByText(/^One-time sign-in with an administrator’s app password/)).toBeInTheDocument();
    p.setUrl('http://localhost:8080');
    await sync();
    expect(view.getByText('Unencrypted. Only for development on this machine.')).toBeInTheDocument();
    expect(view.getByRole('button', { name: 'Copy redirect URI' })).toBeInTheDocument();
  });

  it('shows the lock of the registration and the saved server after a failure', async () => {
    const { p, view, sync } = open(S2, false, true);
    p.setAdminUser('admin');
    p.setAdminPassword('pw');
    p.regLock.start('hub', 30);
    p.dialog = { ...(p.dialog as NonNullable<typeof p.dialog>), regFailed: true, aErr: 'Rejected.' };
    await sync();
    expect(view.getByText('Too many attempts.')).toBeInTheDocument();
    expect(view.getByRole('button', { name: 'Register in 0:30' })).toHaveAttribute('aria-disabled', 'true');
    expect(view.getByText('Rejected.')).toBeInTheDocument();
    expect(view.getByText(/^The server is saved without a new OAuth client/)).toBeInTheDocument();
    p.regLock.stop();
  });
});
