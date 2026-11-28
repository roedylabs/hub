// Administration › Stalwart servers: state and values. Cases taken over from Roedy Mail
// (`apps/web/src/lib/servers.test.ts`) where the flow is the same; the registration after saving is the hub's own.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { ui } from '@roedy/ui';
import { dialogValues, ServersPage, serversValues, testText, type AdminServer } from '#lib/servers.svelte.ts';
import { mockApi } from '#lib/state/fetch-mock.ts';
import { resetSignedIn, signIn } from '#lib/test/signed-in.ts';

const NOW = Date.parse('2026-11-28T12:00:00Z');
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
  cors: 'ok',
  oauth: 'ok',
  tested_at: new Date(NOW - 25 * 60_000).toISOString(),
  accounts: 3,
};
const DEV: AdminServer = {
  ...base,
  id: 'sdev',
  label: 'Development',
  url: 'http://localhost:8080',
  host: 'localhost:8080',
  domains: [],
};
const LIST = { servers: [S1, DEV], redirect_uri: 'https://hub.example.com/oauth/callback', system_mail_server: 's1' };
const POLICY = { allow_custom_servers: false };
const ctx = { scrollTo: vi.fn() };

beforeEach(() => {
  signIn('admin');
});

afterEach(() => {
  resetSignedIn();
  ctx.scrollTo.mockClear();
});

/** A loaded page with the routes; the list and policy answer as often as asked. */
async function loaded(routes: Parameters<typeof mockApi>[0] = {}) {
  const api = mockApi({
    'GET admin/stalwart/servers': Array.from({ length: 5 }, () => ({ body: LIST })),
    'GET admin/stalwart/policy': Array.from({ length: 5 }, () => ({ body: POLICY })),
    ...routes,
  });
  const p = new ServersPage();
  await p.load();
  return { p, ...api };
}

const tested = (server: AdminServer, patch: Partial<AdminServer> = {}) => ({
  body: { server: { ...server, tested_at: new Date(NOW).toISOString(), ...patch }, reasons: {} },
});

describe('list', () => {
  it('loads servers, redirect URI, system mail server and policy into the values', async () => {
    const { p } = await loaded();
    expect(p.status).toBe('ready');
    const v = serversValues(p, ctx, NOW);
    expect(v.srvSum).toBe('2 servers');
    expect(v.srvRedirect).toBe('https://hub.example.com/oauth/callback');
    const [vps, dev] = v.srvRows;
    expect(vps).toMatchObject({ isSys: true, accts: '3 linked accounts', tested: 'Last tested 25 min ago' });
    expect(vps?.chips.map((c) => c.aria)).toEqual(['Reachable: OK', 'CORS enabled: OK', 'OAuth client registered: OK']);
    expect(dev).toMatchObject({ plain: true, noDoms: true, tested: 'Not tested yet', canDiag: false });
    expect(v.tgSrvOwn.on).toBe(false);
    v.srvTabs[1]?.onClick();
    expect(ctx.scrollTo).toHaveBeenCalledWith('policy');
  });

  it('shows the card when the first load fails, a toast later', async () => {
    mockApi({
      'GET admin/stalwart/servers': [{ status: 500, body: { error: 'internal' } }, { body: LIST }, { status: 500 }],
      'GET admin/stalwart/policy': [{ body: POLICY }, { body: POLICY }, { body: POLICY }],
    });
    const p = new ServersPage();
    await p.load();
    expect(p.status).toBe('failed');
    expect(p.failure).toBe('That didn’t work. Please try again.');
    await p.retry();
    expect(p.status).toBe('ready');
    await p.load();
    expect(p.status).toBe('ready');
    expect(ui.toast).toMatchObject({ error: true });
  });

  it('counts warnings in the summary', async () => {
    const { p } = await loaded();
    p.rows = [{ ...S1, cors: 'err' }, DEV];
    expect(serversValues(p, ctx, NOW).srvSum).toBe('2 servers · 1 with warnings');
  });
});

describe('test connection', () => {
  it('expands the diagnosis with the reasons and reports warnings', async () => {
    const { p } = await loaded({
      'POST admin/stalwart/servers/sdev/test': [
        {
          body: {
            server: { ...DEV, reach: 'err', tested_at: new Date(NOW).toISOString() },
            reasons: { reach: 'unreachable', cors: 'not_reachable', oauth: 'not_reachable' },
          },
        },
      ],
    });
    await p.test('sdev');
    expect(p.diag.has('sdev')).toBe(true);
    expect(ui.toast?.text).toBe('Development: test finished with warnings');
    const row = serversValues(p, ctx, NOW).srvRows[1];
    expect(row?.diagRows.map((d) => d.text)).toEqual([
      'Can’t connect to this URL. Check the base URL, port, firewall, and TLS certificate.',
      'Not checked because the server isn’t reachable.',
      'Not checked because the server isn’t reachable.',
    ]);
    expect(row?.diagRows[0]?.actLabel).toBe('Edit URL');
    row?.diagRows[0]?.act();
    expect(p.dialog?.id).toBe('sdev');
  });

  it('offers to register the client from the OAuth row', async () => {
    const { p } = await loaded({
      'POST admin/stalwart/servers/sdev/test': [
        {
          body: {
            server: { ...DEV, reach: 'ok', cors: 'ok', oauth: 'err', tested_at: new Date(NOW).toISOString() },
            reasons: { oauth: 'no_client_id' },
          },
        },
      ],
    });
    await p.test('sdev');
    expect(ui.toast?.text).toBe('Development: test finished with warnings');
    const oauth = serversValues(p, ctx, NOW).srvRows[1]?.diagRows[2];
    expect(oauth?.actLabel).toBe('Register client');
    oauth?.act();
    expect(p.dialog).toMatchObject({ id: 'sdev', oa: 'auto', again: true });
    expect(dialogValues(p)?.aFields).toBe(true);
  });

  it('reports a successful test and runs queued tests one after the other', async () => {
    const { p, calls } = await loaded({
      'POST admin/stalwart/servers/s1/test': [tested(S1)],
      'POST admin/stalwart/servers/sdev/test': [tested(DEV, { reach: 'ok', cors: 'ok', oauth: 'ok' })],
    });
    const first = p.test('s1');
    void p.test('sdev', true);
    await first;
    await vi.waitFor(() => expect(p.testing).toBeNull());
    expect(calls.filter((c) => c.method === 'POST').map((c) => c.path)).toEqual([
      'admin/stalwart/servers/s1/test',
      'admin/stalwart/servers/sdev/test',
    ]);
    expect(ui.toast?.text).toBe('Development: connection OK');
  });

  it('closes a diagnosis opened by a failed test again', async () => {
    const { p } = await loaded({ 'POST admin/stalwart/servers/sdev/test': [{ status: 500 }] });
    await p.test('sdev');
    expect(p.diag.has('sdev')).toBe(false);
    expect(ui.toast?.error).toBe(true);
  });

  it('leaves a diagnosis the user toggled during a failed test as it is', async () => {
    let fail: (reply: { status: number }) => void = () => {};
    const { p } = await loaded({
      'POST admin/stalwart/servers/sdev/test': [() => new Promise((resolve) => (fail = resolve))],
    });
    const test = p.test('sdev');
    // Closed and opened again while the test runs.
    p.toggleDiag('sdev');
    p.toggleDiag('sdev');
    fail({ status: 500 });
    await test;
    expect(p.diag.has('sdev')).toBe(true);
  });

  it('says "connection OK" only when every check passed', () => {
    const at = { ...S1, reach: 'ok', cors: 'ok', oauth: 'ok' } as const;
    expect(testText(at, {})).toBe('VPS 1: connection OK');
    expect(testText({ ...at, oauth: 'unk' }, {})).toBe('VPS 1: not everything could be checked');
    expect(testText({ ...at, cors: 'err' }, {})).toBe('VPS 1: test finished with warnings');
    expect(testText(at, { oauth: 'throttled' })).toBe('VPS 1: test finished with warnings');
  });

  it('skips queued servers removed in the meantime', async () => {
    const S3: AdminServer = { ...DEV, id: 's3', label: 'Third', url: 'https://mail.s3.test', host: 'mail.s3.test' };
    let release: (reply: ReturnType<typeof tested>) => void = () => {};
    const { p, calls } = await loaded({
      'POST admin/stalwart/servers/s1/test': [() => new Promise((resolve) => (release = resolve))],
      'POST admin/stalwart/servers/s3/test': [tested(S3)],
    });
    p.rows = [...p.rows, S3];
    const first = p.test('s1');
    void p.test('sdev', true);
    void p.test('s3', true);
    // Another server's test waits: its buttons say so.
    const row = serversValues(p, ctx, NOW).srvRows.find((r) => r.id === 'sdev');
    expect(row).toMatchObject({ busy: false, testDis: true });
    p.rows = p.rows.filter((s) => s.id !== 'sdev');
    release(tested(S1));
    await first;
    await vi.waitFor(() => expect(p.testing).toBeNull());
    expect(calls.filter((c) => c.method === 'POST').map((c) => c.path)).toEqual([
      'admin/stalwart/servers/s1/test',
      'admin/stalwart/servers/s3/test',
    ]);
  });
});

describe('removal and policy', () => {
  it('asks with the linked accounts and the system mail, then removes', async () => {
    const { p, calls } = await loaded({ 'DELETE admin/stalwart/servers/s1': [{ status: 204 }] });
    p.remove(S1);
    expect(ui.confirm).toMatchObject({
      title: 'Remove “VPS 1”?',
      text: '3 linked accounts can’t get access tokens any more until the server is added again. System emails are sent through this server and will stop working.',
      danger: true,
    });
    ui.confirmOk();
    await vi.waitFor(() => expect(p.rows.map((s) => s.id)).toEqual(['sdev']));
    expect(p.systemServer).toBeNull();
    expect(ui.toast?.text).toBe('“VPS 1” removed');
    expect(calls.some((c) => c.method === 'DELETE')).toBe(true);
  });

  it('counts a server gone in the meantime as removed and keeps the row on errors', async () => {
    const { p } = await loaded({
      'DELETE admin/stalwart/servers/sdev': [{ status: 404, body: { error: 'not_found' } }],
      'DELETE admin/stalwart/servers/s1': [{ status: 500, body: { error: 'internal' } }],
    });
    p.remove(DEV);
    ui.confirmOk();
    await vi.waitFor(() => expect(p.rows.map((s) => s.id)).toEqual(['s1']));
    p.remove(S1);
    ui.confirmOk();
    await vi.waitFor(() => expect(ui.toast).toMatchObject({ error: true }));
    expect(ui.toast?.text).toBe('The server wasn’t removed. Please try again.');
    // The list is loaded again: the server is still there.
    expect(p.rows.map((s) => s.id)).toContain('s1');
  });

  it('toggles the policy at once and takes it back on errors, as the server knows it', async () => {
    const { p, calls } = await loaded({
      'GET admin/stalwart/policy': [{ body: POLICY }, { body: { allow_custom_servers: true } }],
      'PUT admin/stalwart/policy': [{ body: { allow_custom_servers: true } }, { status: 500 }],
    });
    p.toggleOwn();
    expect(p.allowOwn).toBe(true);
    expect(ui.toast?.text).toBe('Other servers allowed');
    await vi.waitFor(() => expect(calls.some((c) => c.method === 'PUT')).toBe(true));
    expect(calls.find((c) => c.method === 'PUT')?.body).toEqual({ allow_custom_servers: true });
    p.toggleOwn();
    expect(p.allowOwn).toBe(false);
    await vi.waitFor(() => expect(ui.toast?.error).toBe(true));
    expect(calls.filter((c) => c.method === 'PUT').map((c) => c.body)).toEqual([
      { allow_custom_servers: true },
      { allow_custom_servers: false },
    ]);
    // Back to the confirmed "on", not to the opposite of the failed "off"; the reload confirms it.
    expect(p.allowOwn).toBe(true);
    await vi.waitFor(() =>
      expect(calls.filter((c) => c.path === 'admin/stalwart/policy' && c.method === 'GET')).toHaveLength(2),
    );
    await vi.waitFor(() => expect(p.status).toBe('ready'));
    expect(p.allowOwn).toBe(true);
  });

  it('returns to the confirmed policy after two failed toggles', async () => {
    const { p } = await loaded({ 'PUT admin/stalwart/policy': [{ status: 403 }, { status: 403 }] });
    p.toggleOwn();
    p.toggleOwn();
    await vi.waitFor(() => expect(ui.toast?.error).toBe(true));
    await vi.waitFor(() => expect(p.allowOwn).toBe(false));
  });

  it('does not bring back a server removed while the list was loading', async () => {
    let list: (reply: { body: typeof LIST }) => void = () => {};
    const { p } = await loaded({
      'GET admin/stalwart/servers': [{ body: LIST }, () => new Promise((resolve) => (list = resolve))],
      'DELETE admin/stalwart/servers/sdev': [{ status: 204 }],
    });
    const reload = p.load();
    p.remove(DEV);
    ui.confirmOk();
    await vi.waitFor(() => expect(p.rows.map((s) => s.id)).toEqual(['s1']));
    list({ body: LIST });
    await reload;
    expect(p.rows.map((s) => s.id)).toEqual(['s1']);
  });
});

describe('dialog', () => {
  it('checks the fields before saving', async () => {
    const { p, calls } = await loaded();
    p.openDialog();
    p.setUrl('http://mail.example.com');
    p.domainInput('bad_domain s1.test ');
    p.setAdminUser('admin');
    await p.save();
    const sd = dialogValues(p);
    expect(sd).toMatchObject({
      eLabel: 'Please enter a label.',
      eUrl: 'Use an HTTPS URL. http:// only works for localhost.',
      eDom: '“bad_domain” isn’t a valid domain.',
      eAdm: 'Please enter the Stalwart server’s admin user and app password, or leave both empty.',
    });
    p.removeDomain('bad_domain');
    await p.save();
    expect(dialogValues(p)?.eDom).toBe('“s1.test” is already assigned to “VPS 1”.');
    expect(calls.filter((c) => c.method !== 'GET')).toHaveLength(0);
  });

  it('adds a server without a client and tests it', async () => {
    const NEW: AdminServer = {
      ...base,
      id: 'n1',
      label: 'New',
      url: 'https://mail.new.test',
      host: 'mail.new.test',
      domains: ['new.test'],
    };
    const { p, calls } = await loaded({
      'POST admin/stalwart/servers': [{ status: 201, body: NEW }],
      'POST admin/stalwart/servers/n1/test': [tested(NEW, { reach: 'ok', cors: 'ok', oauth: 'err' })],
    });
    p.openDialog();
    expect(dialogValues(p)).toMatchObject({ title: 'Add server', btn: 'Add and test', aFields: true, aDone: false });
    p.setLabel(' New ');
    p.setUrl('https://mail.new.test/');
    p.domainInput('@New.test,');
    await p.save();
    expect(calls.find((c) => c.path === 'admin/stalwart/servers' && c.method === 'POST')?.body).toEqual({
      label: 'New',
      url: 'https://mail.new.test',
      domains: ['new.test'],
    });
    expect(p.dialog).toBeNull();
    expect(p.rows.map((s) => s.id)).toEqual(['s1', 'sdev', 'n1']);
    await vi.waitFor(() => expect(p.testing).toBeNull());
    expect(calls.some((c) => c.path === 'admin/stalwart/servers/n1/test')).toBe(true);
  });

  it('saves first, then registers the client with the credentials, then tests', async () => {
    const NEW: AdminServer = {
      ...base,
      id: 'n1',
      label: 'New',
      url: 'https://mail.new.test',
      host: 'mail.new.test',
      domains: [],
    };
    const REG = { ...NEW, client_id: 'roedy-new', has_secret: true };
    const { p, calls } = await loaded({
      'POST admin/stalwart/servers': [{ status: 201, body: NEW }],
      'POST admin/stalwart/servers/n1/register-client': [{ body: REG }],
      'POST admin/stalwart/servers/n1/test': [tested(REG, { reach: 'ok', cors: 'ok', oauth: 'ok' })],
    });
    p.openDialog();
    p.setLabel('New');
    p.setUrl('https://mail.new.test');
    p.setAdminUser(' admin ');
    p.setAdminPassword('app pw');
    expect(dialogValues(p)).toMatchObject({ btn: 'Add and register', btnIcon: 'ti-wand' });
    await p.save();
    const posts = calls.filter((c) => c.method === 'POST').map((c) => c.path);
    expect(posts.slice(0, 2)).toEqual(['admin/stalwart/servers', 'admin/stalwart/servers/n1/register-client']);
    // With the address the dialog saved: the hub sends the credentials only there.
    expect(calls.find((c) => c.path.endsWith('register-client'))?.body).toEqual({
      url: 'https://mail.new.test',
      username: 'admin',
      password: 'app pw',
    });
    expect(p.dialog).toBeNull();
    expect(p.rows.find((s) => s.id === 'n1')?.client_id).toBe('roedy-new');
    await vi.waitFor(() => expect(ui.toast?.text).toBe('New: connection OK'));
  });

  it('keeps the dialog on the saved server when the registration fails, and retries with PATCH', async () => {
    const NEW: AdminServer = {
      ...base,
      id: 'n1',
      label: 'New',
      url: 'https://mail.new.test',
      host: 'mail.new.test',
      domains: [],
    };
    const { p, calls } = await loaded({
      'POST admin/stalwart/servers': [{ status: 201, body: NEW }],
      'PATCH admin/stalwart/servers/n1': [{ body: NEW }],
      'POST admin/stalwart/servers/n1/register-client': [
        { status: 422, body: { error: 'registration_failed', reason: 'auth_failed' } },
        { status: 502, body: { error: 'registration_failed', reason: 'rejected', detail: 'invalid_client_metadata' } },
      ],
    });
    p.openDialog();
    p.setLabel('New');
    p.setUrl('https://mail.new.test');
    p.setAdminUser('admin');
    p.setAdminPassword('wrong');
    await p.save();
    expect(p.dialog).toMatchObject({ id: 'n1', ap: '', au: 'admin', regFailed: true, saving: false });
    const sd = dialogValues(p);
    expect(sd).toMatchObject({
      title: 'Edit server',
      eAdm: 'Administrator sign-in failed. Check the username and app password.',
      aFields: true,
      regFailed: true,
    });
    expect(p.rows.some((s) => s.id === 'n1')).toBe(true);
    p.setAdminPassword('pw');
    await p.save();
    expect(calls.filter((c) => c.method === 'PATCH')).toHaveLength(1);
    expect(dialogValues(p)?.eAdm).toBe(
      'The Stalwart server rejected the registration (invalid_client_metadata). Choose “Manual” and create the client in the Stalwart web admin.',
    );
    // The credentials stay for "forbidden" and other reasons.
    expect(p.dialog?.ap).toBe('pw');
  });

  it('locks the registration after 429 of the hub and while Stalwart throttles', async () => {
    const { p } = await loaded({
      'PATCH admin/stalwart/servers/sdev': [{ body: DEV }, { body: DEV }],
      'POST admin/stalwart/servers/sdev/register-client': [
        { status: 429, body: { error: 'rate_limited', retry_after: 90 } },
      ],
    });
    p.openDialog(DEV);
    p.setAdminUser('admin');
    p.setAdminPassword('pw');
    await p.save();
    expect(dialogValues(p)).toMatchObject({
      aLocked: true,
      aLockText: 'Try again in 2 minutes.',
      saveLocked: true,
      btn: 'Register in 1:30',
    });
    p.regLock.stop();
  });

  it('shows the box of a throttling Stalwart only for its server', async () => {
    const { p } = await loaded({
      'PATCH admin/stalwart/servers/sdev': [{ body: DEV }],
      'POST admin/stalwart/servers/sdev/register-client': [
        { status: 502, body: { error: 'registration_failed', reason: 'throttled', retry_after: 45 } },
      ],
    });
    p.openDialog(DEV);
    p.setAdminUser('admin');
    p.setAdminPassword('pw');
    await p.save();
    expect(dialogValues(p)).toMatchObject({
      aThr: true,
      aThrText: 'The Stalwart server is throttling requests from Example Org. Try again in 45 seconds.',
    });
    p.openDialog(S1, true);
    p.setAdminUser('admin');
    p.setAdminPassword('pw');
    expect(dialogValues(p)).toMatchObject({ aThr: false, aLocked: false, saveLocked: false });
    p.regLock.stop();
  });

  it('keeps one server’s throttling while registering at another', async () => {
    const { p } = await loaded({
      'PATCH admin/stalwart/servers/sdev': [{ body: DEV }],
      'PATCH admin/stalwart/servers/s1': [{ body: S1 }],
      'POST admin/stalwart/servers/sdev/register-client': [
        { status: 502, body: { error: 'registration_failed', reason: 'throttled', retry_after: 45 } },
      ],
      'POST admin/stalwart/servers/s1/register-client': [
        { status: 422, body: { error: 'registration_failed', reason: 'auth_failed' } },
      ],
    });
    p.openDialog(DEV);
    p.setAdminUser('admin');
    p.setAdminPassword('pw');
    await p.save();
    p.openDialog(S1, true);
    p.setAdminUser('admin');
    p.setAdminPassword('pw');
    await p.save();
    p.openDialog(DEV, true);
    p.setAdminUser('admin');
    p.setAdminPassword('pw');
    expect(dialogValues(p)).toMatchObject({ aThr: true, saveLocked: true });
    p.regLock.stop();
  });

  it('locks the registration after 429 of the hub also when the dialog closed meanwhile', async () => {
    let reply: (r: { status: number; body: unknown }) => void = () => {};
    const { p } = await loaded({
      'PATCH admin/stalwart/servers/sdev': [{ body: DEV }],
      'POST admin/stalwart/servers/sdev/register-client': [() => new Promise((resolve) => (reply = resolve))],
    });
    p.openDialog(DEV);
    p.setAdminUser('admin');
    p.setAdminPassword('pw');
    const saving = p.save();
    await vi.waitFor(() => expect(p.dialog?.registering).toBe(true));
    p.closeDialog();
    p.openDialog(S1, true);
    reply({ status: 429, body: { error: 'rate_limited', retry_after: 60 } });
    await saving;
    expect(p.regBlocked('s1')).toBe(true);
    // The other dialog's fields stay as they are.
    expect(p.dialog).toMatchObject({ id: 's1', regFailed: false, aErr: null });
    p.regLock.stop();
  });

  it('takes no input while saving', async () => {
    let reply: (r: { body: AdminServer }) => void = () => {};
    const { p } = await loaded({
      'PATCH admin/stalwart/servers/sdev': [() => new Promise((resolve) => (reply = resolve))],
      'POST admin/stalwart/servers/sdev/test': [tested(DEV)],
    });
    p.openDialog(DEV);
    const saving = p.save();
    expect(p.dialog?.saving).toBe(true);
    p.setLabel('Changed');
    p.setUrl('https://other.test');
    p.domainInput('x.test,');
    p.setOa('manual');
    expect(p.dialog).toMatchObject({ label: 'Development', url: 'http://localhost:8080', domains: [], oa: 'auto' });
    reply({ body: DEV });
    await saving;
    await vi.waitFor(() => expect(p.testing).toBeNull());
  });

  it('shows the stored client under "Automatic" and keeps it without credentials', async () => {
    const { p, calls } = await loaded({
      'PATCH admin/stalwart/servers/s1': [{ body: { ...S1, label: 'VPS one' } }],
      'POST admin/stalwart/servers/s1/test': [tested(S1, { label: 'VPS one' })],
    });
    p.openDialog(S1);
    expect(dialogValues(p)).toMatchObject({
      isAuto: true,
      aDone: true,
      aFields: false,
      aDoneSub: 'Client ID roedy-abc123 · Secret stored in the hub',
      btn: 'Save and test',
    });
    p.setAgain(true);
    expect(dialogValues(p)).toMatchObject({ aFields: true, canKeep: true, hasLinkHint: true });
    expect(dialogValues(p)?.linkHint).toBe('With a new client, the 3 linked accounts have to sign in again.');
    p.setAgain(false);
    p.setUrl('https://mail.s1-new.test');
    expect(dialogValues(p)).toMatchObject({ aFields: true, moved: true, canKeep: false });
    p.setUrl('https://mail.s1.test/');
    p.setLabel('VPS one');
    await p.save();
    expect(calls.find((c) => c.method === 'PATCH')?.body).toEqual({
      label: 'VPS one',
      url: 'https://mail.s1.test',
      domains: ['s1.test'],
    });
    await vi.waitFor(() => expect(ui.toast?.text).toBe('VPS one: connection OK'));
    expect(p.testing).toBeNull();
  });

  it('sends client ID and secret under "Manual" and checks them', async () => {
    const { p, calls } = await loaded({
      'PATCH admin/stalwart/servers/sdev': [{ body: { ...DEV, client_id: 'c1' } }],
      'POST admin/stalwart/servers/sdev/test': [tested(DEV, { client_id: 'c1' })],
    });
    p.openDialog(DEV);
    p.setOa('manual');
    p.setSecret('s3cret');
    await p.save();
    expect(dialogValues(p)?.eCred).toBe('A secret needs a client ID.');
    p.setCid('bad\nid');
    await p.save();
    expect(dialogValues(p)?.eCred).toBe('The client ID contains invalid characters or is too long.');
    p.setCid(' c1 ');
    await p.save();
    expect(calls.find((c) => c.method === 'PATCH')?.body).toEqual({
      label: 'Development',
      url: 'http://localhost:8080',
      domains: [],
      client_id: 'c1',
      client_secret: 's3cret',
    });
    await vi.waitFor(() => expect(calls.some((c) => c.path === 'admin/stalwart/servers/sdev/test')).toBe(true));
    await vi.waitFor(() => expect(p.testing).toBeNull());
    expect(ui.toast?.error).toBeFalsy();
  });

  it('asks for the secret again under "Manual" when the address changes', async () => {
    const { p } = await loaded();
    p.openDialog({ ...S1, has_secret: true });
    p.setOa('manual');
    expect(dialogValues(p)?.secPh).toBe('The secret is saved. Leave empty to keep it. It stays in the hub only.');
    p.setUrl('https://mail.s1-new.test');
    expect(dialogValues(p)?.secPh).toBe(
      'The address changes, so the saved secret no longer applies. Enter the client secret again.',
    );
  });

  it('shows the field errors of the server', async () => {
    const { p } = await loaded({
      'POST admin/stalwart/servers': [
        {
          status: 422,
          body: {
            error: 'invalid',
            fields: {
              label: { code: 'too_long' },
              url: { code: 'taken', server: 'VPS 1' },
              domains: { code: 'taken', domain: 'x.test', server: 'Other' },
            },
          },
        },
      ],
    });
    p.openDialog();
    p.setLabel('New');
    p.setUrl('https://mail.s1.test');
    await p.save();
    expect(dialogValues(p)).toMatchObject({
      eLabel: 'The label can have at most 100 characters.',
      eUrl: 'This address is already used by “VPS 1”.',
      eDom: '“x.test” is already assigned to “Other”.',
    });
  });

  it('takes domains as chips and closes on the backdrop', async () => {
    const { p } = await loaded();
    p.openDialog();
    p.domainInput('a.test, b.test c');
    expect(p.dialog).toMatchObject({ domains: ['a.test', 'b.test'], domIn: 'c' });
    p.domainKey({ key: 'Enter', preventDefault: () => {} });
    expect(p.dialog?.domains).toEqual(['a.test', 'b.test', 'c']);
    p.domainKey({ key: 'Backspace', preventDefault: () => {} });
    expect(p.dialog?.domains).toEqual(['a.test', 'b.test']);
    const sd = dialogValues(p);
    const target = {};
    sd?.overlay({ target, currentTarget: target, preventDefault: () => {} } as never);
    expect(p.dialog).toBeNull();
  });
});
