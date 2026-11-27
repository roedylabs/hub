// Taken over from Roedy Mail (`apps/web/src/lib/users.test.ts`) without undo and the kept list; with the details,
// the hub's domain errors and the request-sequencing races.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { mockApi, type Call } from '#lib/state/fetch-mock.ts';
import { prefs } from '#lib/state/prefs.svelte.ts';
import { session } from '#lib/state/session.svelte.ts';
import { ui } from '#lib/state/ui.svelte.ts';
import { offlineApp, pairedApp, pendingApp } from '#lib/test/apps.ts';
import { signIn, TEST_USER } from '#lib/test/signed-in.ts';
import { UsersPage, rowsValues, usersValues, type ListedUser } from './users.svelte.ts';

const NOW = Date.parse('2026-10-26T12:00:00Z');
const before = (hours: number) => new Date(NOW - hours * 3600_000).toISOString();

const user = (id: string, name: string, extra: Partial<ListedUser> = {}): ListedUser => ({
  id,
  status: 'active',
  name,
  email: `${name.toLowerCase().replace(' ', '.')}@example.com`,
  username: name.split(' ')[0]?.toLowerCase() ?? null,
  role: 'user',
  mfa: false,
  last_seen_at: before(2),
  created_at: before(500),
  ...extra,
});
const invite = (id: string, email: string, extra: Partial<ListedUser> = {}): ListedUser => ({
  id,
  status: 'invited',
  name: null,
  email,
  role: 'user',
  created_at: before(1),
  invited_by: { id: TEST_USER.id, name: 'Alex Example' },
  mail_sent_at: before(1),
  expires_at: before(-167),
  expired: false,
  requested_at: null,
  ...extra,
});

const USERS: ListedUser[] = [
  user(TEST_USER.id, 'Alex Example', { email: TEST_USER.email, role: 'admin', last_seen_at: before(0) }),
  user('u1', 'Anna Baker', { role: 'admin', mfa: true }),
  user('u2', 'Luke Walker', { mfa: true, last_seen_at: before(26) }),
  user('u3', 'Sophie Hamilton', { status: 'locked', last_seen_at: before(120) }),
  invite('i1', 'jane.smith@example.com', { name: 'Jane Smith' }),
  invite('i2', 'paul.long@example.com', { mail_sent_at: null }),
];

const ctx = { editMe: vi.fn(), setupMail: vi.fn() };

type Reply = Parameters<typeof mockApi>[0][string][number];
/** The same answer many times (lists load again after every action). */
const always = (reply: Reply, n = 30): Reply[] => Array.from({ length: n }, () => reply);
/** An answer that waits for `release`. */
function held() {
  let release: (answer: { status?: number; body?: unknown }) => void = () => {};
  const reply: Reply = () => new Promise((resolve) => (release = resolve));
  return { reply, release: (answer: { status?: number; body?: unknown }) => release(answer) };
}

beforeEach(() => {
  // Dates in words use the host's time zone: pinned, so "26 October" holds everywhere.
  vi.stubEnv('TZ', 'UTC');
  vi.useFakeTimers({ toFake: ['Date'] });
  vi.setSystemTime(NOW);
  signIn('admin');
});

afterEach(() => {
  vi.unstubAllGlobals();
  vi.unstubAllEnvs();
  vi.useRealTimers();
  session.config = null;
  session.user = null;
  prefs.reset();
  ui.reset();
});

/** Loaded page with the test data; `GET users` answers with `server.users` (changeable by the test). */
async function loaded(routes: Parameters<typeof mockApi>[0] = {}, users = USERS, mail = 'ok') {
  const server = { users };
  const api = mockApi({
    'GET users': always(() => ({ body: { users: server.users } })),
    'GET admin/system-mail': [{ body: { status: mail } }],
    ...routes,
  });
  const p = new UsersPage();
  p.start();
  await vi.waitFor(() => expect(p.status).toBe('ready'));
  await tick();
  return { p, v: () => usersValues(p, ctx, NOW), server, ...api };
}

const tick = () => new Promise((resolve) => setTimeout(resolve, 0));
const row = (v: ReturnType<typeof usersValues>, name: string) => {
  const found = v.uRows.find((r) => r.name === name);
  if (!found) throw new Error(`no row ${name}`);
  return found;
};
const listed = (p: UsersPage, id: string) => p.rows.find((u) => u.id === id);
const sent = (calls: Call[]) => calls.map((c) => `${c.method} ${c.path}`);
const loads = (calls: Call[]) => sent(calls).filter((c) => c === 'GET users').length;

describe('List', () => {
  it('loads users and invitations, sorted by name', async () => {
    const { v, calls } = await loaded();
    expect(v().uRows.map((r) => r.name)).toEqual([
      'Alex Example',
      'Anna Baker',
      'Jane Smith',
      'Luke Walker',
      'paul.long@example.com',
      'Sophie Hamilton',
    ]);
    expect(v().uPills.map((o) => [o.label, o.n, o.on])).toEqual([
      ['All', 6, true],
      ['Active', 3, false],
      ['Invited', 2, false],
      ['Suspended', 1, false],
    ]);
    expect(v()).toMatchObject({ uHasRows: true, uNoMail: false });
    expect(sent(calls)).toEqual(['GET users', 'GET admin/system-mail', 'GET admin/apps']);
  });

  it('shows the own account with "You", without selection and actions', async () => {
    const { p, v } = await loaded({ [`GET users/${TEST_USER.id}`]: [{ body: { user: { sessions: [] } } }] });
    const me = row(v(), 'Alex Example');
    expect(me).toMatchObject({
      isMe: true,
      online: true,
      last: 'Now',
      lastLong: 'Active now',
      ckLabel: 'You can’t select your own account',
      isActive: false,
      notMe: false,
      canMfaReset: false,
    });
    p.toggleSelect(TEST_USER.id);
    expect(p.selected).toEqual([]);
    p.toggleOpen(TEST_USER.id);
    expect(row(v(), 'Alex Example').roles.every((r) => r.dis)).toBe(true);
    row(v(), 'Alex Example').editMe();
    expect(ctx.editMe).toHaveBeenCalled();
  });

  it('describes rows (time, status, multi-factor)', async () => {
    const { v } = await loaded();
    expect(row(v(), 'Anna Baker')).toMatchObject({
      initials: 'AB',
      sub: 'anna.baker@example.com',
      role: 'Administrator',
      roleIcon: 'ti-crown',
      mfa: true,
      canMfaReset: true,
      st: 'Active',
      last: '2 hours ago',
      lastLong: 'Last active 2 hours ago',
    });
    expect(row(v(), 'Luke Walker').last).toBe('26 hours ago');
    expect(row(v(), 'Sophie Hamilton')).toMatchObject({
      st: 'Suspended',
      isLockedSt: true,
      isLocked: true,
      last: 'Last seen 5 days ago',
      canMfaReset: false,
    });
    expect(row(v(), 'paul.long@example.com')).toMatchObject({
      sub: 'Invitation pending',
      initials: 'P',
      isInvited: true,
      sendErr: true,
      hasSendNote: true,
      canResend: true,
      mfa: false,
      removeLabel: 'Revoke invitation',
      lastLong: 'Invited 1 hour ago',
      sendNote: 'The invitation email couldn’t be sent. Copy the link and share it yourself, or resend the invitation.',
    });
    expect(row(v(), 'Jane Smith')).toMatchObject({ sendErr: false, hasSendNote: false, sendNote: '' });
  });

  it('offers no "Resend" without system mail and explains the invitation', async () => {
    const { v } = await loaded({}, USERS, 'none');
    expect(v()).toMatchObject({ uNoMail: true, uHasMail: false });
    expect(row(v(), 'paul.long@example.com')).toMatchObject({
      canResend: false,
      sendNote: 'Without system emails, no invitation is sent. Copy the link and share it yourself.',
    });
    v().uNoMailGo();
    expect(ctx.setupMail).toHaveBeenCalled();
  });

  it('assumes a system mail if its state cannot be loaded', async () => {
    const { v } = await loaded({ 'GET admin/system-mail': [{ status: 500, body: { error: 'internal' } }] });
    expect(v().uNoMail).toBe(false);
  });

  it('marks expired invitations and names the expiry date for valid ones', async () => {
    // `expired` of the API only applies on loading: `expires_at` is what counts.
    const old = invite('i3', 'eve.ross@example.com', { name: 'Eve Ross', expires_at: before(5), expired: false });
    const { v } = await loaded({}, [...USERS, old]);
    expect(row(v(), 'Eve Ross')).toMatchObject({
      st: 'Expired',
      expired: true,
      requested: false,
      last: 'on 26 October',
      lastLong: 'Expired on 26 October',
      canExtend: true,
      canCopy: false,
      canResend: true,
      hasValid: false,
      hasSendNote: true,
      sendNote:
        'This invitation has expired and the link no longer works. Resend it, or extend it and share the link yourself.',
    });
    expect(row(v(), 'Jane Smith')).toMatchObject({
      st: 'Invited',
      expired: false,
      canExtend: false,
      canCopy: true,
      hasValid: true,
      validUntil: 'Valid until 2 November',
    });
    expect(row(v(), 'Anna Baker')).toMatchObject({ expired: false, canExtend: false, canCopy: false, hasValid: false });
  });

  it('shows that the invitee asked for a new invitation (`requested_at`)', async () => {
    const asked = invite('i3', 'eve.ross@example.com', {
      expires_at: before(30),
      expired: true,
      requested_at: before(3),
    });
    const { v } = await loaded({}, [...USERS, asked]);
    expect(row(v(), 'eve.ross@example.com')).toMatchObject({
      requested: true,
      last: 'New invitation requested',
      sendNote:
        'The invitee asked for a new invitation 3 hours ago. Resend it, or extend it and share the link yourself.',
    });
  });

  it('explains requested and expired invitations without system mail without "Resend"', async () => {
    const asked = invite('i3', 'eve.ross@example.com', {
      expires_at: before(30),
      expired: true,
      requested_at: before(3),
    });
    const old = invite('i4', 'tim.ross@example.com', { expires_at: before(5), expired: true, mail_sent_at: null });
    const { v } = await loaded({}, [...USERS, asked, old], 'none');
    expect(row(v(), 'eve.ross@example.com').sendNote).toBe(
      'The invitee asked for a new invitation 3 hours ago. Extend it and share the link yourself.',
    );
    expect(row(v(), 'tim.ross@example.com')).toMatchObject({
      canExtend: true,
      canResend: false,
      sendNote: 'This invitation has expired and the link no longer works. Extend it and share the link yourself.',
    });
  });

  it('searches, filters, sorts and resets locally', async () => {
    const { p, v, calls } = await loaded();
    p.setQuery('BAKER');
    expect(v().uRows.map((r) => r.name)).toEqual(['Anna Baker']);
    p.setQuery('long@');
    expect(v().uRows.map((r) => r.name)).toEqual(['paul.long@example.com']);
    expect(v().uPills.map((o) => o.n)).toEqual([1, 0, 1, 0]);
    p.setQuery('xyz');
    expect(v()).toMatchObject({ uEmpty: true, uHasRows: false, uEmptyText: 'No results for “xyz”', uCanReset: true });
    v().uReset();
    p.setFilter('invited');
    expect(v().uRows.map((r) => r.name)).toEqual(['Jane Smith', 'paul.long@example.com']);
    p.setFilter('locked');
    p.setFilter('active');
    expect(v().uEmptyText).toBe('No users in this view');
    p.resetFilter();
    p.sortBy('status');
    expect(v().uRows.map((r) => r.status)).toEqual(['active', 'active', 'active', 'invited', 'invited', 'locked']);
    expect(v().uHdStatus).toMatchObject({ on: true, desc: false, aria: 'Sort by Status' });
    p.sortBy('status');
    expect(v().uHdStatus.desc).toBe(true);
    expect(v().uRows[0]?.status).toBe('locked');
    p.sortBy('role');
    expect(
      v()
        .uRows.slice(0, 2)
        .map((r) => r.role),
    ).toEqual(['Administrator', 'Administrator']);
    // Typing and sorting send nothing: the search works on the loaded list.
    expect(loads(calls)).toBe(1);
  });

  it('shows the card if the first load fails, with the reason', async () => {
    mockApi({ 'GET users': [{ status: 500, body: { error: 'internal' } }] });
    const p = new UsersPage();
    await p.load();
    expect(p.status).toBe('failed');
    expect(p.failure).toBe('That didn’t work. Please try again.');
    vi.stubGlobal(
      'fetch',
      vi.fn(() => Promise.reject(new TypeError('offline'))),
    );
    const retry = p.retry();
    expect(p.status).toBe('loading');
    await retry;
    expect(p.failure).toBe('No connection. Please try again.');
  });

  it('names the wait time if the first load is rate-limited', async () => {
    mockApi({ 'GET users': [{ status: 429, body: { error: 'rate_limited', retry_after: 20 } }] });
    const p = new UsersPage();
    await p.load();
    expect(p.failure).toBe('Too many requests. Try again in 20 seconds.');
  });

  it('keeps the list with an error toast if a reload fails', async () => {
    const { p } = await loaded({ 'GET users': [{ body: { users: USERS } }, { status: 500, body: {} }] });
    await p.load();
    expect(p.status).toBe('ready');
    expect(p.rows).toHaveLength(6);
    expect(ui.toast).toMatchObject({ text: 'Couldn’t update the list.', error: true });
  });

  it('reloads the account without a session', async () => {
    mockApi({
      'GET users': [{ status: 401, body: { error: 'unauthorized' } }],
      'GET session': [{ status: 401, body: { error: 'unauthorized' } }],
    });
    const p = new UsersPage();
    await p.load();
    await vi.waitFor(() => expect(session.authed).toBe(false));
  });

  it('discards an older list that arrives after a newer one', async () => {
    const slow = held();
    mockApi({ 'GET users': [slow.reply, { body: { users: USERS.slice(0, 2) } }] });
    const p = new UsersPage();
    const first = p.load();
    await p.load();
    slow.release({ body: { users: USERS } });
    await first;
    expect(p.rows).toHaveLength(2);
  });

  it('writes nothing after leaving the page', async () => {
    const slow = held();
    const mail = held();
    mockApi({ 'GET users': [slow.reply], 'GET admin/system-mail': [mail.reply] });
    const p = new UsersPage();
    p.start();
    p.dispose();
    slow.release({ body: { users: USERS } });
    mail.release({ body: { status: 'none' } });
    await tick();
    expect(p.rows).toEqual([]);
    expect(p.status).toBe('loading');
    expect(p.hasMail).toBe(true);
  });
});

describe('Details', () => {
  const details = (sessions: { created_at: string; last_seen_at: string | null }[], codes = null as unknown) => ({
    body: { user: { ...USERS[1], sessions, recovery_codes: codes } },
  });

  it('loads devices and recovery codes when a user opens, not for invitations', async () => {
    const { p, v, calls } = await loaded({
      'GET users/u1': [
        details(
          [
            { created_at: before(50), last_seen_at: before(1) },
            { created_at: before(10), last_seen_at: null },
          ],
          { unused: 8, total: 10 },
        ),
      ],
    });
    p.toggleOpen('u1');
    expect(row(v(), 'Anna Baker').details).toMatchObject({ loading: true, failed: false, devices: '' });
    await vi.waitFor(() => expect(p.details?.status).toBe('ready'));
    expect(row(v(), 'Anna Baker').details).toMatchObject({
      loading: false,
      devices: 'Signed in on 2 devices · last active 1 hour ago',
      codes: '8 of 10 recovery codes left',
    });
    p.toggleOpen('i1');
    expect(p.details).toBeNull();
    expect(row(v(), 'Jane Smith').details).toBeNull();
    expect(sent(calls)).not.toContain('GET users/i1');
  });

  it('names a single device, none and missing codes', async () => {
    const { p, v } = await loaded({
      'GET users/u1': [details([{ created_at: before(5), last_seen_at: before(5) }]), details([])],
    });
    p.toggleOpen('u1');
    await vi.waitFor(() => expect(p.details?.status).toBe('ready'));
    expect(row(v(), 'Anna Baker').details).toMatchObject({
      devices: 'Signed in on 1 device · active 5 hours ago',
      codes: 'No recovery codes',
    });
    await p.loadDetails('u1');
    expect(row(v(), 'Anna Baker').details?.devices).toBe('Not signed in on any device');
  });

  it('ignores the details of a row closed or switched in the meantime', async () => {
    const a = held();
    const { p } = await loaded({ 'GET users/u1': [a.reply], 'GET users/u2': [details([])] });
    p.toggleOpen('u1');
    p.toggleOpen('u2');
    await vi.waitFor(() => expect(p.details).toMatchObject({ id: 'u2', status: 'ready' }));
    a.release(details([{ created_at: before(1), last_seen_at: before(1) }]));
    await tick();
    expect(p.details).toMatchObject({ id: 'u2', data: { sessions: [] } });
    // Closed while loading: nothing comes back.
    const b = held();
    mockApi({ 'GET users/u1': [b.reply] });
    p.toggleOpen('u1');
    p.toggleOpen('u1');
    b.release(details([]));
    await tick();
    expect(p.open).toBeNull();
    expect(p.details).toBeNull();
  });

  it('keeps the details while the list reloads', async () => {
    const slow = held();
    const { p, server } = await loaded({ 'GET users/u1': [details([])] });
    p.toggleOpen('u1');
    await vi.waitFor(() => expect(p.details?.status).toBe('ready'));
    mockApi({ 'GET users': [slow.reply] });
    const reload = p.load();
    slow.release({ body: { users: server.users } });
    await reload;
    expect(p.open).toBe('u1');
    expect(p.details?.status).toBe('ready');
  });

  it('closes the row and drops its details when a reload no longer has the user', async () => {
    const slow = held();
    const { p, server } = await loaded({ 'GET users/u1': [slow.reply] });
    p.toggleOpen('u1');
    server.users = USERS.filter((u) => u.id !== 'u1');
    await p.load();
    expect(p.open).toBeNull();
    slow.release(details([]));
    await tick();
    expect(p.details).toBeNull();
  });

  it('offers "Try again" if the details fail', async () => {
    const { p, v } = await loaded({
      'GET users/u1': [{ status: 500, body: { error: 'internal' } }, details([])],
    });
    p.toggleOpen('u1');
    await vi.waitFor(() => expect(p.details?.status).toBe('failed'));
    expect(row(v(), 'Anna Baker').details).toMatchObject({ failed: true, loading: false });
    row(v(), 'Anna Baker').details?.retry();
    await vi.waitFor(() => expect(p.details?.status).toBe('ready'));
  });

  it('reloads the list if the user is gone (`not_found`)', async () => {
    const { p, calls, server } = await loaded({ 'GET users/u1': [{ status: 404, body: { error: 'not_found' } }] });
    server.users = USERS.filter((u) => u.id !== 'u1');
    p.toggleOpen('u1');
    await vi.waitFor(() => expect(p.open).toBeNull());
    expect(ui.toast).toMatchObject({ text: 'This entry no longer exists. The list has been updated.', error: true });
    expect(loads(calls)).toBe(2);
    expect(listed(p, 'u1')).toBeUndefined();
  });
});

describe('Selection', () => {
  it('selects all except the own account and shows matching actions', async () => {
    const { p, v } = await loaded();
    expect(v().uAllCk).toMatchObject({ on: false, label: 'Select all', icon: 'ti-check', vis: 1 });
    p.toggleSelect('u2');
    expect(v()).toMatchObject({ uSelOn: true, uSelOff: false, uSelText: '1 selected', uBulkCanLock: true });
    expect(v().uAllCk).toMatchObject({ on: 'mixed', icon: 'ti-minus' });
    expect(row(v(), 'Luke Walker')).toMatchObject({ sel: true, isSel: true, ckLabel: 'Deselect Luke Walker' });
    v().uAllCk.onClick();
    expect(p.selected).not.toContain(TEST_USER.id);
    expect(v().uAllCk).toMatchObject({ on: true, label: 'Clear selection' });
    expect(v()).toMatchObject({ uBulkCanLock: true, uBulkCanUnlock: true });
    v().uAllCk.onClick();
    expect(p.selected).toEqual([]);
  });

  it('changes the role of the selection (`users/bulk`) and reloads', async () => {
    const { p, v, calls } = await loaded({ 'POST users/bulk': [{ body: { changed: ['u2', 'u3'] } }] });
    p.toggleSelect('u2');
    p.toggleSelect('u3');
    p.toggleSelect('u1');
    expect(v().selBulkRole.title).toBe('Role for 3 people');
    v().selBulkRole.toggle();
    expect(ui.menu).toBe('pick');
    expect(v().selBulkRole.opts.map((o) => [o.label, o.on])).toEqual([
      ['Administrator', false],
      ['User', false],
    ]);
    v().selBulkRole.opts[0]?.onClick();
    expect(ui.menu).toBeNull();
    // Shown at once, sent at once.
    expect(listed(p, 'u2')?.role).toBe('admin');
    expect(calls.at(-1)).toMatchObject({
      path: 'users/bulk',
      body: { action: 'role', ids: ['u2', 'u3'], role: 'admin' },
    });
    await vi.waitFor(() => expect(ui.toast?.text).toBe('Role changed for 2 people'));
    await vi.waitFor(() => expect(loads(calls)).toBe(2));
  });

  it('operates the select list "Change role" with the keyboard', async () => {
    const { p, v } = await loaded({ 'POST users/bulk': [{ body: { changed: ['u2'] } }] });
    p.toggleSelect('u2');
    const key = (k: string) => v().selBulkRole.onKey({ key: k, preventDefault: () => {} });
    key('ArrowDown');
    expect(v().selBulkRole.open).toBe(true);
    expect(v().selBulkRole.opts.map((o) => o.hi)).toEqual([false, true]);
    key('ArrowDown');
    expect(v().selBulkRole.opts[0]?.hi).toBe(true);
    key('Tab');
    expect(v().selBulkRole.open).toBe(false);
    key('ArrowUp');
    key('ArrowUp');
    const space = { key: ' ', preventDefault: vi.fn() };
    v().selBulkRole.onKey(space);
    expect(space.preventDefault).toHaveBeenCalled();
    expect(listed(p, 'u2')?.role).toBe('user');
    v().selBulkRole.onKeyUp({ key: ' ', preventDefault: () => {} });
    expect(listed(p, 'u2')?.role).toBe('admin');
    expect(v().selBulkRole.open).toBe(false);
    await vi.waitFor(() => expect(ui.toast?.text).toBe('Role changed'));
  });

  it('locks after confirmation, unlocks and removes the selection', async () => {
    const { p, v, calls, server } = await loaded({
      'POST users/bulk': [
        { body: { changed: ['u1', 'u2'] } },
        { body: { changed: ['u1'] } },
        { body: { changed: ['u1', 'i2'] } },
      ],
    });
    p.toggleSelect('u1');
    p.toggleSelect('u2');
    v().uBulkLock();
    expect(ui.confirm).toMatchObject({
      title: 'Suspend 2 people?',
      text: 'The selected people will be signed out immediately and can’t sign in again until you reactivate their accounts.',
      ok: 'Suspend',
      danger: true,
    });
    server.users = USERS.map((u) => (u.id === 'u1' || u.id === 'u2' ? { ...u, status: 'locked' as const } : u));
    ui.confirmOk();
    expect(listed(p, 'u1')?.status).toBe('locked');
    expect(calls.at(-1)?.body).toEqual({ action: 'lock', ids: ['u1', 'u2'] });
    await vi.waitFor(() => expect(ui.toast?.text).toBe('2 people suspended'));
    await vi.waitFor(() => expect(loads(calls)).toBe(2));
    p.selected = ['u1'];
    v().uBulkUnlock();
    expect(calls.at(-1)?.body).toEqual({ action: 'unlock', ids: ['u1'] });
    await vi.waitFor(() => expect(ui.toast?.text).toBe('Anna Baker reactivated'));
    p.selected = ['u1', 'i2'];
    v().uBulkRemove();
    expect(ui.confirm).toMatchObject({
      title: 'Remove 2 people?',
      text: 'The selected people will be signed out and lose access to Example Org immediately. Pending invitations are revoked.',
      ok: 'Remove',
    });
    ui.confirmOk();
    expect(p.selected).toEqual([]);
    expect(listed(p, 'u1')).toBeUndefined();
    expect(calls.at(-1)?.body).toEqual({ action: 'remove', ids: ['u1', 'i2'] });
    await vi.waitFor(() => expect(ui.toast?.text).toBe('2 people removed'));
  });

  it('brings back rows and selection with the error toast after a failed bulk removal', async () => {
    const { p, v } = await loaded({ 'POST users/bulk': [{ status: 500, body: { error: 'internal' } }] });
    p.selected = ['u1', 'u2'];
    v().uBulkRemove();
    ui.confirmOk();
    expect(p.selected).toEqual([]);
    await vi.waitFor(() =>
      expect(ui.toast).toMatchObject({ text: 'That didn’t work. Please try again.', error: true }),
    );
    expect([listed(p, 'u1')?.id, listed(p, 'u2')?.id]).toEqual(['u1', 'u2']);
    expect(p.selected).toEqual(['u1', 'u2']);
  });

  it('says so when a bulk action changed nothing (e.g. only the last administrator)', async () => {
    const { p, v, calls } = await loaded({ 'POST users/bulk': [{ body: { changed: [] } }] });
    p.selected = ['u1'];
    v().uBulkLock();
    ui.confirmOk();
    await vi.waitFor(() => expect(ui.toast).toMatchObject({ text: 'Nothing was changed.', error: true }));
    // The reload shows the server's state.
    await vi.waitFor(() => expect(loads(calls)).toBe(2));
    expect(listed(p, 'u1')?.status).toBe('active');
  });

  it('keeps only what the server changed and names it, even if the reload fails', async () => {
    const { p, v } = await loaded();
    const { calls } = mockApi({
      'POST users/bulk': [{ body: { changed: ['u1'] } }, { body: { changed: ['i2'] } }],
      'GET users': always({ status: 500, body: { error: 'internal' } }),
    });
    const show = vi.spyOn(ui.toasts, 'show');
    p.selected = ['u1', 'u2'];
    v().uBulkLock();
    ui.confirmOk();
    await vi.waitFor(() => expect(ui.toast).toMatchObject({ error: true }));
    expect(loads(calls)).toBe(1);
    // The server skipped Luke Walker: his row is back before the (failing) reload.
    expect(show).toHaveBeenCalledWith('Anna Baker suspended');
    expect([listed(p, 'u1')?.status, listed(p, 'u2')?.status]).toEqual(['locked', 'active']);
    // A skipped removal comes back with its selection.
    p.selected = ['u2', 'i2'];
    v().uBulkRemove();
    ui.confirmOk();
    await vi.waitFor(() => expect(loads(calls)).toBe(2));
    expect(show).toHaveBeenLastCalledWith('paul.long@example.com removed');
    expect(listed(p, 'i2')).toBeUndefined();
    expect(listed(p, 'u2')?.status).toBe('active');
    expect(p.selected).toEqual(['u2']);
    show.mockRestore();
  });

  it('refreshes the open details only if a bulk status change changed that account', async () => {
    const details = { body: { user: { ...USERS[2], sessions: [], recovery_codes: null } } };
    const { p, v, calls } = await loaded({
      'POST users/bulk': [{ body: { changed: ['u1'] } }, { body: { changed: ['u2'] } }],
      'GET users/u2': always(details, 3),
    });
    const detailLoads = () => sent(calls).filter((c) => c === 'GET users/u2').length;
    p.toggleOpen('u2');
    await vi.waitFor(() => expect(p.details?.status).toBe('ready'));
    p.selected = ['u1', 'u2'];
    v().uBulkLock();
    ui.confirmOk();
    await vi.waitFor(() => expect(loads(calls)).toBe(2));
    await tick();
    expect(detailLoads()).toBe(1);
    p.selected = ['u2'];
    v().uBulkLock();
    ui.confirmOk();
    await vi.waitFor(() => expect(detailLoads()).toBe(2));
  });

  it('asks before revoking a single invitation and reports it as removed', async () => {
    const { p, v } = await loaded({ 'POST users/bulk': [{ body: { changed: ['i2'] } }] });
    p.toggleSelect('i2');
    v().uBulkRemove();
    expect(ui.confirm).toMatchObject({
      title: 'Revoke invitation?',
      text: 'The link in the invitation to paul.long@example.com will stop working.',
      ok: 'Revoke',
    });
    ui.confirmOk();
    await vi.waitFor(() => expect(ui.toast?.text).toBe('paul.long@example.com removed'));
  });
});

describe('Inviting', () => {
  it('turns separators into chips, checks addresses and locks the name with several', async () => {
    const { p, v } = await loaded();
    v().openInvite();
    expect(v()).toMatchObject({
      inviteOpen: true,
      inviteClosed: false,
      invPh: 'name@example.com',
      invBtn: { dis: true },
    });
    v().onInvEmail('eve.ross@example.com');
    expect(v()).toMatchObject({ invSendLabel: 'Send invitation', invSendShort: 'Send', invBtn: { dis: false } });
    v().onInvEmail('eve.ross@example.com, Eve.Ross@example.com;anna.baker@example.com broken@ x');
    expect(p.invite?.list).toEqual(['eve.ross@example.com', 'anna.baker@example.com', 'broken@']);
    expect(v().invEmail).toBe('x');
    expect(v().invChips.map((c) => [c.label, c.bad, c.tip])).toEqual([
      ['eve.ross@example.com', false, undefined],
      ['anna.baker@example.com', true, 'Already has an account'],
      ['broken@', true, 'Not a valid email address'],
    ]);
    expect(v()).toMatchObject({
      invErr: 'Check 2 addresses. Remove or fix the entries marked in red.',
      hasInvErr: true,
      invBtn: { dis: true },
      invNameDis: true,
      invNamePh: 'Only for a single address',
      invPh: '',
    });
    v().invChips[2]?.remove();
    expect(v().invErr).toBe('anna.baker@example.com already belongs to an account.');
    v().invChips[1]?.remove();
    v().onInvEmail('broken');
    v().onInvBlur();
    expect(v().invErr).toBe('“broken” isn’t a valid email address.');
    const key = (k: string) => {
      const event = { key: k, preventDefault: vi.fn() };
      v().invKey(event);
      return event;
    };
    expect(key('Backspace').preventDefault).not.toHaveBeenCalled();
    expect(p.invite?.list).toEqual(['eve.ross@example.com']);
    v().onInvEmail('tim.brown@example.com');
    expect(key('Enter').preventDefault).toHaveBeenCalled();
    expect(p.invite?.list).toEqual(['eve.ross@example.com', 'tim.brown@example.com']);
    expect(v()).toMatchObject({ invSendLabel: 'Send 2 invitations', invSendShort: 'Send 2' });
    key('Escape');
    expect(v().inviteOpen).toBe(false);
  });

  it('says "create" instead of "send" without system mail', async () => {
    const { p, v } = await loaded({}, USERS, 'none');
    p.openInvite();
    p.inviteInput('tim.brown@example.com eve.ross@example.com');
    expect(v()).toMatchObject({ invSendLabel: 'Create 2 invitations', invSendShort: 'Create 2' });
    p.removeChip('tim.brown@example.com');
    p.inviteInput('');
    expect(v()).toMatchObject({ invSendLabel: 'Create invitation', invSendShort: 'Create' });
  });

  it('sends addresses, role and name and reports success', async () => {
    const { p, v, calls } = await loaded({
      'POST invites': [
        { status: 201, body: { invites: [{ id: 'n1', email: 'jane@example.com', sent: true }], system_mail: true } },
      ],
    });
    p.openInvite();
    p.inviteInput('jane@example.com');
    p.inviteName('  Jane\tSmith ');
    p.inviteRole('admin');
    p.setFilter('locked');
    await p.sendInvites();
    expect(calls.find((c) => c.path === 'invites')?.body).toEqual({
      emails: ['jane@example.com'],
      role: 'admin',
      name: 'Jane Smith',
    });
    expect(v()).toMatchObject({ inviteOpen: false, invRes: { show: false } });
    expect(p.filter).toBe('all');
    expect(ui.toast).toMatchObject({ text: 'Invitation sent', error: false });
    expect(sent(calls).at(-1)).toBe('GET users');
  });

  it('shows the result with "Copy link" if a mail did not go out (`sent: false`)', async () => {
    const writeText = vi.fn(() => Promise.resolve());
    vi.stubGlobal('navigator', { ...navigator, clipboard: { writeText } });
    const nina = invite('n2', 'nina@example.com', { mail_sent_at: null });
    const added = [...USERS, invite('n1', 'eve@example.com'), nina];
    const { p, v, server } = await loaded({
      'POST invites': [
        {
          status: 201,
          body: {
            invites: [
              { id: 'n1', email: 'eve@example.com', sent: true },
              { id: 'n2', email: 'nina@example.com', sent: false },
            ],
            system_mail: true,
          },
        },
      ],
      'GET invites/n2/link': [{ body: { url: 'https://hub.example.com/invite/abc' } }],
      'POST invites/n2/resend': [{ body: { invite: { ...nina, mail_sent_at: before(0) } } }],
    });
    server.users = added;
    p.openInvite();
    p.inviteInput('eve@example.com nina@example.com');
    await p.sendInvites();
    expect(v().invRes).toMatchObject({
      show: true,
      off: false,
      icon: 'ti-alert-triangle',
      title: '1 invitation sent, 1 couldn’t be sent',
      text: 'The invitation to this address has been created. Copy the link and share it yourself, or resend the invitation later.',
    });
    expect(v().invRes.rows.map((r) => r.email)).toEqual(['nina@example.com']);
    expect(ui.toast).toBeNull();
    v().invRes.rows[0]?.copy();
    await vi.waitFor(() => expect(writeText).toHaveBeenCalledWith('https://hub.example.com/invite/abc'));
    await vi.waitFor(() => expect(ui.toast?.text).toBe('Invitation link copied'));
    // After a successful resend the row disappears from the result, and with it the result.
    server.users = [...USERS, invite('n1', 'eve@example.com'), { ...nina, mail_sent_at: before(0) }];
    await p.resend(listed(p, 'n2') as ListedUser);
    expect(v().invRes.show).toBe(false);
  });

  it('shows "created" without system mail and takes over `system_mail`', async () => {
    const { p, v, server } = await loaded({
      'POST invites': [
        { status: 201, body: { invites: [{ id: 'n1', email: 'tim@example.com', sent: false }], system_mail: false } },
      ],
    });
    server.users = [...USERS, invite('n1', 'tim@example.com', { mail_sent_at: null })];
    p.openInvite();
    p.inviteInput('tim@example.com');
    await p.sendInvites();
    expect(v().invRes).toMatchObject({
      show: true,
      off: true,
      icon: 'ti-mail-off',
      title: 'Invitation created',
      text: 'Without system emails, no invitation is sent. Copy the link and share it yourself.',
    });
    expect(v().uNoMail).toBe(true);
    v().invRes.close();
    expect(v().invRes.show).toBe(false);
  });

  it('shows unsent invitations with "Copy link" even if the reload fails', async () => {
    const { p, v } = await loaded();
    mockApi({
      'POST invites': [
        { status: 201, body: { invites: [{ id: 'n1', email: 'tim@example.com', sent: false }], system_mail: true } },
      ],
      'GET users': [{ status: 500, body: { error: 'internal' } }, { body: { users: USERS } }],
    });
    p.openInvite();
    p.inviteInput('tim@example.com');
    await p.sendInvites();
    expect(v().invRes).toMatchObject({ show: true, title: 'Invitation couldn’t be sent' });
    expect(v().invRes.rows.map((r) => [r.id, r.email])).toEqual([['n1', 'tim@example.com']]);
    // A list loaded since no longer has it (revoked in the meantime): the result ends.
    await p.load();
    expect(v().invRes.show).toBe(false);
  });

  it('keeps `system_mail` of an invitation over an older mail state arriving later', async () => {
    const mail = held();
    const { p, v, server } = await loaded({
      'GET admin/system-mail': [mail.reply],
      'POST invites': [
        { status: 201, body: { invites: [{ id: 'n1', email: 'tim@example.com', sent: false }], system_mail: false } },
      ],
    });
    server.users = [...USERS, invite('n1', 'tim@example.com', { mail_sent_at: null })];
    p.openInvite();
    p.inviteInput('tim@example.com');
    await p.sendInvites();
    expect(p.hasMail).toBe(false);
    mail.release({ body: { status: 'ok' } });
    await tick();
    expect(p.hasMail).toBe(false);
    expect(row(v(), 'tim@example.com').canResend).toBe(false);
  });

  it('names several failed invitations', async () => {
    const { p, v, server } = await loaded({
      'POST invites': [
        {
          status: 201,
          body: {
            invites: [
              { id: 'n1', email: 'a@example.com', sent: false },
              { id: 'n2', email: 'b@example.com', sent: false },
            ],
            system_mail: true,
          },
        },
      ],
    });
    server.users = [
      ...USERS,
      invite('n1', 'a@example.com', { mail_sent_at: null }),
      invite('n2', 'b@example.com', { mail_sent_at: null }),
    ];
    p.openInvite();
    p.inviteInput('a@example.com b@example.com');
    await p.sendInvites();
    expect(v().invRes).toMatchObject({
      title: '2 invitations couldn’t be sent',
      text: 'The invitations to these addresses have been created. Copy the links and share them yourself, or resend the invitations later.',
    });
  });

  it('marks addresses reported by the server as taken or invalid (`invalid` with `fields`)', async () => {
    const { p, v } = await loaded({
      'POST invites': [
        {
          status: 422,
          body: { error: 'invalid', fields: { 'emails.1': { code: 'taken' }, 'emails.2': { code: 'invalid' } } },
        },
      ],
    });
    p.openInvite();
    p.inviteInput('eve@example.com new@example.com x@example.com');
    await p.sendInvites();
    expect(v().inviteOpen).toBe(true);
    expect(v().invChips.map((c) => c.tip)).toEqual([undefined, 'Already has an account', 'Not a valid email address']);
    expect(v().invErr).toBe('Check 2 addresses. Remove or fix the entries marked in red.');
    expect(ui.toast).toBeNull();
    // A new form forgets them.
    p.closeInvite();
    p.openInvite();
    p.inviteInput('new@example.com ');
    expect(v().invChips[0]?.bad).toBe(false);
  });

  it('reports more than 50 addresses in the form and locks the button without sending', async () => {
    const { p, v, calls } = await loaded();
    p.openInvite();
    p.inviteInput(Array.from({ length: 51 }, (_, i) => `a${String(i)}@example.com`).join(' ') + ' ');
    expect(v()).toMatchObject({ invErr: 'Up to 50 addresses at a time.', hasInvErr: true, invBtn: { dis: true } });
    await p.sendInvites();
    expect(sent(calls)).not.toContain('POST invites');
    p.removeChip('a50@example.com');
    expect(v()).toMatchObject({ invErr: '', invBtn: { dis: false } });
  });

  it('reports `emails: too_many` of the server with an error toast without reloading', async () => {
    const { p, v, calls } = await loaded({
      'POST invites': [{ status: 422, body: { error: 'invalid', fields: { emails: { code: 'too_many' } } } }],
    });
    p.openInvite();
    p.inviteInput(Array.from({ length: 50 }, (_, i) => `a${String(i)}@example.com`).join(' '));
    await p.sendInvites();
    expect(ui.toast).toMatchObject({ text: 'Up to 50 addresses at a time.', error: true });
    expect(v()).toMatchObject({ inviteOpen: true, invBusy: false });
    expect(loads(calls)).toBe(1);
  });

  it('leaves a newly opened form alone when the response of the old one arrives', async () => {
    const first = held();
    const { p, v } = await loaded({
      'POST invites': [
        first.reply,
        { status: 422, body: { error: 'invalid', fields: { 'emails.0': { code: 'taken' } } } },
      ],
    });
    p.openInvite();
    p.inviteInput('eve@example.com');
    const sending = p.sendInvites();
    p.closeInvite();
    p.openInvite();
    p.inviteInput('tim@example.com');
    first.release({
      status: 201,
      body: { invites: [{ id: 'n1', email: 'eve@example.com', sent: true }], system_mail: true },
    });
    await sending;
    expect(v()).toMatchObject({ inviteOpen: true, invEmail: 'tim@example.com' });
    expect(ui.toast?.text).toBe('Invitation sent');
    // If the old form fails, a toast reports it instead of marks in the new one.
    const second = p.sendInvites();
    p.closeInvite();
    p.openInvite();
    await second;
    expect(ui.toast).toMatchObject({ text: 'That didn’t work. Please try again.', error: true });
    expect(v().invChips).toEqual([]);
  });

  it('reports a lock (`rate_limited`) in the form with a running wait time', async () => {
    const { p, v, calls } = await loaded({
      'POST invites': [{ status: 429, body: { error: 'rate_limited', retry_after: 70 } }],
    });
    vi.useFakeTimers();
    vi.setSystemTime(NOW);
    p.openInvite();
    p.inviteInput('eve@example.com');
    await p.sendInvites();
    expect(v()).toMatchObject({
      invErr: 'Too many requests. Try again in 2 minutes.',
      hasInvErr: true,
      invBtn: { dis: true },
    });
    vi.advanceTimersByTime(30_000);
    expect(v().invErr).toBe('Too many requests. Try again in 40 seconds.');
    await p.sendInvites();
    expect(sent(calls).filter((c) => c === 'POST invites')).toHaveLength(1);
    vi.advanceTimersByTime(40_000);
    expect(v()).toMatchObject({ invErr: '', hasInvErr: false, invBtn: { dis: false } });
    p.dispose();
  });

  it('reports other errors with an error toast and leaves the form open', async () => {
    const { p, v } = await loaded({ 'POST invites': [{ status: 500, body: { error: 'internal' } }] });
    p.openInvite();
    p.inviteInput('eve@example.com');
    await p.sendInvites();
    expect(ui.toast).toMatchObject({ text: 'That didn’t work. Please try again.', error: true });
    expect(v()).toMatchObject({ inviteOpen: true, invBusy: false });
  });

  it('checks the name only with one address and shows the server’s name errors until it changes', async () => {
    const { p, v, calls } = await loaded({
      'POST invites': [{ status: 422, body: { error: 'invalid', fields: { name: { code: 'invalid' } } } }],
    });
    p.openInvite();
    p.inviteInput('eve@example.com');
    p.inviteName('E'.repeat(101));
    expect(v()).toMatchObject({ invErr: 'The name is too long.', invBtn: { dis: true } });
    await p.sendInvites();
    expect(sent(calls)).not.toContain('POST invites');
    p.inviteName('Eve Ross');
    await p.sendInvites();
    expect(v()).toMatchObject({ inviteOpen: true, invErr: 'The name contains invalid characters.' });
    p.inviteName('Eve Rossi');
    expect(v().invErr).toBe('');
  });
});

describe('Actions per row', () => {
  it('changes the role at once (`PATCH users/:id`, `invites/:id`)', async () => {
    const { p, v, calls } = await loaded({
      'PATCH users/u2': [{ body: { user: { ...USERS[2], role: 'admin' } } }],
      'PATCH invites/i1': [{ body: { invite: { ...USERS[4], role: 'admin' } } }],
    });
    p.toggleOpen('i1');
    row(v(), 'Jane Smith').roles[0]?.onClick();
    expect(listed(p, 'i1')?.role).toBe('admin');
    expect(calls.at(-1)).toMatchObject({ method: 'PATCH', path: 'invites/i1', body: { role: 'admin' } });
    await vi.waitFor(() => expect(ui.toast?.text).toBe('Role changed'));
    p.setRole(listed(p, 'u2') as ListedUser, 'admin');
    expect(calls.at(-1)).toMatchObject({ method: 'PATCH', path: 'users/u2', body: { role: 'admin' } });
    // The same role or the own account: nothing.
    const before = calls.length;
    p.setRole(listed(p, TEST_USER.id) as ListedUser, 'user');
    p.setRole(listed(p, 'u1') as ListedUser, 'admin');
    expect(calls).toHaveLength(before);
  });

  it('keeps a change on its way when an older list arrives in the meantime', async () => {
    const patch = held();
    const slow = held();
    const { p, server } = await loaded({ 'PATCH users/u2': [patch.reply] });
    mockApi({
      'GET users': [slow.reply, ...always(() => ({ body: { users: server.users } }))],
      'PATCH users/u2': [patch.reply],
    });
    const reload = p.load();
    p.setRole(listed(p, 'u2') as ListedUser, 'admin');
    // The list the server sent before the change.
    slow.release({ body: { users: USERS } });
    await reload;
    expect(listed(p, 'u2')?.role).toBe('admin');
    server.users = USERS.map((u) => (u.id === 'u2' ? { ...u, role: 'admin' as const } : u));
    patch.release({ body: { user: server.users[2] } });
    await vi.waitFor(() => expect(ui.toast?.text).toBe('Role changed'));
    await tick();
    expect(listed(p, 'u2')?.role).toBe('admin');
  });

  it('does not let a list loaded before a confirmed change overwrite it', async () => {
    const slow = held();
    const { p, server } = await loaded({ 'PATCH users/u2': [{ body: { user: null } }] });
    mockApi({
      'GET users': [slow.reply, ...always(() => ({ body: { users: server.users } }))],
      'PATCH users/u2': [{ body: { user: null } }],
    });
    const reload = p.load();
    server.users = USERS.map((u) => (u.id === 'u2' ? { ...u, role: 'admin' as const } : u));
    p.setRole(listed(p, 'u2') as ListedUser, 'admin');
    await vi.waitFor(() => expect(ui.toast?.text).toBe('Role changed'));
    // The reload after the change is newer: the old list arriving now is dropped.
    slow.release({ body: { users: USERS } });
    await reload;
    await tick();
    expect(listed(p, 'u2')?.role).toBe('admin');
  });

  it('reverts a failed change, names it and reloads', async () => {
    const { p, calls } = await loaded({ 'PATCH users/u2': [{ status: 500, body: { error: 'internal' } }] });
    p.setRole(listed(p, 'u2') as ListedUser, 'admin');
    await vi.waitFor(() =>
      expect(ui.toast).toMatchObject({ text: 'The role wasn’t changed. Please try again.', error: true }),
    );
    expect(listed(p, 'u2')?.role).toBe('user');
    await vi.waitFor(() => expect(loads(calls)).toBe(2));
  });

  it('returns to the confirmed role, not a pending one, when overlapping role changes fail', async () => {
    const first = held();
    const second = held();
    const { p } = await loaded();
    mockApi({
      'PATCH users/u2': [first.reply, second.reply],
      'GET users': always({ status: 500, body: { error: 'internal' } }),
    });
    p.setRole(listed(p, 'u2') as ListedUser, 'admin');
    p.setRole(listed(p, 'u2') as ListedUser, 'user');
    expect(listed(p, 'u2')?.role).toBe('user');
    first.release({ status: 500, body: { error: 'internal' } });
    await vi.waitFor(() => expect(ui.toast).toMatchObject({ error: true }));
    // The newer change is still on its way.
    expect(listed(p, 'u2')?.role).toBe('user');
    second.release({ status: 500, body: { error: 'internal' } });
    await tick();
    await tick();
    expect(listed(p, 'u2')?.role).toBe('user');
  });

  it('takes a settled role change as the baseline of a later one', async () => {
    const first = held();
    const second = held();
    const { p } = await loaded();
    const { calls } = mockApi({
      'PATCH users/u2': [first.reply, second.reply],
      'GET users': always({ status: 500, body: { error: 'internal' } }),
    });
    p.setRole(listed(p, 'u2') as ListedUser, 'admin');
    p.setRole(listed(p, 'u2') as ListedUser, 'user');
    first.release({ body: { user: null } });
    // Settled: its reload (failing here) follows.
    await vi.waitFor(() => expect(loads(calls)).toBe(1));
    second.release({ status: 500, body: { error: 'internal' } });
    await vi.waitFor(() => expect(loads(calls)).toBe(2));
    expect(listed(p, 'u2')?.role).toBe('admin');
  });

  it('keeps the baseline of a later confirmed change when an earlier one is confirmed late', async () => {
    const first = held();
    const second = held();
    const third = held();
    const { p } = await loaded();
    const { calls } = mockApi({
      'PATCH users/u2': [first.reply, second.reply, third.reply],
      'GET users': always({ status: 500, body: { error: 'internal' } }),
    });
    p.setRole(listed(p, 'u2') as ListedUser, 'admin');
    p.setRole(listed(p, 'u2') as ListedUser, 'user');
    // The later change is confirmed first: the server has "user" (it applied "admin" before).
    second.release({ body: { user: null } });
    await vi.waitFor(() => expect(loads(calls)).toBe(1));
    p.setRole(listed(p, 'u2') as ListedUser, 'admin');
    // The earlier change's late confirmation is older than "user".
    first.release({ body: { user: null } });
    await vi.waitFor(() => expect(loads(calls)).toBe(2));
    // Rejected with 429 (no reload): back to what the server has.
    third.release({ status: 429, body: { error: 'rate_limited', retry_after: 30 } });
    await vi.waitFor(() => expect(ui.toast?.text).toContain('30 seconds'));
    expect(listed(p, 'u2')?.role).toBe('user');
    expect(loads(calls)).toBe(2);
  });

  it('takes a field changed by someone else from a newer list when the server skips the row', async () => {
    const bulk = held();
    const { p, v, server } = await loaded();
    const { calls } = mockApi({
      'POST users/bulk': [bulk.reply],
      'GET users': [() => ({ body: { users: server.users } }), ...always({ status: 500, body: { error: 'internal' } })],
    });
    p.selected = ['u1', 'u2'];
    v().uBulkLock();
    ui.confirmOk();
    // Another administrator locked Luke Walker meanwhile; a list loaded now says so.
    server.users = USERS.map((u) => (u.id === 'u2' ? { ...u, status: 'locked' as const } : u));
    await p.load();
    expect(listed(p, 'u2')?.status).toBe('locked');
    bulk.release({ body: { changed: ['u1'] } });
    await vi.waitFor(() => expect(loads(calls)).toBe(2));
    expect([listed(p, 'u1')?.status, listed(p, 'u2')?.status]).toEqual(['locked', 'locked']);
  });

  it('brings back a skipped removal without a change of it rejected meanwhile', async () => {
    const role = held();
    const bulk = held();
    const { p, v } = await loaded();
    const { calls } = mockApi({
      'PATCH users/u2': [role.reply],
      'POST users/bulk': [bulk.reply],
      'GET users': always({ status: 500, body: { error: 'internal' } }),
    });
    p.setRole(listed(p, 'u2') as ListedUser, 'admin');
    p.selected = ['u2', 'u3'];
    v().uBulkRemove();
    ui.confirmOk();
    expect(listed(p, 'u2')).toBeUndefined();
    role.release({ status: 429, body: { error: 'rate_limited', retry_after: 30 } });
    await vi.waitFor(() => expect(ui.toast?.text).toContain('30 seconds'));
    bulk.release({ body: { changed: ['u3'] } });
    await vi.waitFor(() => expect(loads(calls)).toBe(1));
    expect(listed(p, 'u3')).toBeUndefined();
    expect(listed(p, 'u2')?.role).toBe('user');
  });

  it('names the last administrator (`last_admin`)', async () => {
    const { p } = await loaded({ 'PATCH users/u1': [{ status: 409, body: { error: 'last_admin' } }] });
    p.setRole(listed(p, 'u1') as ListedUser, 'user');
    await vi.waitFor(() =>
      expect(ui.toast).toMatchObject({ text: 'At least one active administrator must remain.', error: true }),
    );
    expect(listed(p, 'u1')?.role).toBe('admin');
  });

  it('names "No connection" instead of the action text without a connection', async () => {
    const { p } = await loaded();
    vi.stubGlobal(
      'fetch',
      vi.fn(() => Promise.reject(new TypeError('offline'))),
    );
    p.unlock(listed(p, 'u3') as ListedUser);
    await vi.waitFor(() => expect(ui.toast).toMatchObject({ text: 'No connection. Please try again.', error: true }));
  });

  it('locks after confirmation, unlocks and refreshes the open details', async () => {
    const { p, calls } = await loaded({
      'POST users/u2/lock': [{ body: { user: { ...USERS[2], status: 'locked' } } }],
      'POST users/u3/unlock': [{ status: 500, body: { error: 'internal' } }],
      'GET users/u2': always({ body: { user: { ...USERS[2], sessions: [], recovery_codes: null } } }, 3),
    });
    p.toggleOpen('u2');
    p.lock(listed(p, 'u2') as ListedUser);
    expect(ui.confirm).toMatchObject({
      title: 'Suspend Luke Walker?',
      text: 'Luke Walker will be signed out immediately and can’t sign in until you reactivate the account.',
      danger: true,
    });
    ui.confirmOk();
    expect(listed(p, 'u2')?.status).toBe('locked');
    await vi.waitFor(() => expect(ui.toast?.text).toBe('Luke Walker suspended'));
    await vi.waitFor(() => expect(sent(calls).filter((c) => c === 'GET users/u2')).toHaveLength(2));
    p.unlock(listed(p, 'u3') as ListedUser);
    await vi.waitFor(() =>
      expect(ui.toast).toMatchObject({ text: 'The account wasn’t reactivated. Please try again.', error: true }),
    );
    expect(listed(p, 'u3')?.status).toBe('locked');
  });

  it('removes after confirmation and brings the row back on an error', async () => {
    const { p, v, calls } = await loaded({
      'DELETE users/u2': [{ status: 500, body: { error: 'internal' } }, { status: 204 }],
      'GET users/u2': always({ body: { user: { ...USERS[2], sessions: [], recovery_codes: null } } }, 3),
    });
    p.toggleOpen('u2');
    p.toggleSelect('u2');
    row(v(), 'Luke Walker').remove();
    expect(ui.confirm).toMatchObject({
      title: 'Remove Luke Walker?',
      text: 'Luke Walker will be signed out and lose access to Example Org immediately.',
      ok: 'Remove',
    });
    ui.confirmOk();
    expect(listed(p, 'u2')).toBeUndefined();
    expect(p.open).toBeNull();
    await vi.waitFor(() => expect(ui.toast).toMatchObject({ error: true }));
    expect(listed(p, 'u2')).toBeDefined();
    expect(p.open).toBe('u2');
    expect(p.selected).toEqual(['u2']);
    row(v(), 'Luke Walker').remove();
    ui.confirmOk();
    await vi.waitFor(() => expect(ui.toast?.text).toBe('Luke Walker removed'));
    expect(sent(calls).filter((c) => c === 'DELETE users/u2')).toHaveLength(2);
  });

  it('counts a removal of someone already gone (`not_found`) as done', async () => {
    const { p, v } = await loaded({ 'DELETE invites/i1': [{ status: 404, body: { error: 'not_found' } }] });
    row(v(), 'Jane Smith').remove();
    expect(listed(p, 'i1')).toBeDefined();
    expect(ui.confirm).toMatchObject({ title: 'Revoke invitation?', ok: 'Revoke' });
    ui.confirmOk();
    expect(listed(p, 'i1')).toBeUndefined();
    await vi.waitFor(() => expect(ui.toast).toMatchObject({ text: 'Invitation revoked', error: false }));
  });

  it('resets multi-factor after confirmation and names `mfa_disabled`', async () => {
    const { p, calls } = await loaded({
      'POST users/u1/reset-mfa': [
        { body: { user: { ...USERS[1], mfa: false } } },
        { status: 409, body: { error: 'mfa_disabled' } },
      ],
    });
    p.resetMfa(listed(p, 'u1') as ListedUser);
    expect(ui.confirm).toMatchObject({ title: 'Reset multi-factor for Anna Baker?', ok: 'Reset', danger: true });
    ui.confirmOk();
    expect(listed(p, 'u1')?.mfa).toBe(false);
    await vi.waitFor(() => expect(ui.toast?.text).toBe('Multi-factor reset for Anna Baker'));
    expect(sent(calls)).toContain('POST users/u1/reset-mfa');
    await vi.waitFor(() => expect(loads(calls)).toBe(2));
    p.resetMfa(listed(p, 'u1') as ListedUser);
    ui.confirmOk();
    await vi.waitFor(() =>
      expect(ui.toast).toMatchObject({ text: 'Multi-factor is no longer set up for Anna Baker.', error: true }),
    );
  });

  it('resends an invitation and shows a failed mail (`mail_failed`) in the row', async () => {
    const { p, v } = await loaded({
      'POST invites/i2/resend': [
        { status: 502, body: { error: 'mail_failed' } },
        { body: { invite: { ...USERS[5], mail_sent_at: before(0) } } },
      ],
    });
    p.toggleOpen('i2');
    await p.resend(listed(p, 'i2') as ListedUser);
    expect(row(v(), 'paul.long@example.com')).toMatchObject({
      hasSendNote: true,
      canCopy: true,
      sendNote: 'The invitation couldn’t be resent. Copy the link and share it yourself, or try again later.',
    });
    expect(ui.toast).toBeNull();
    await p.resend(listed(p, 'i2') as ListedUser);
    expect(ui.toast?.text).toBe('Invitation resent');
    expect(row(v(), 'paul.long@example.com')).toMatchObject({ hasSendNote: false, sendErr: false });
  });

  it('points to "Extend" instead of "Copy link" when resending an expired invitation fails', async () => {
    const old = invite('i3', 'eve.ross@example.com', { expires_at: before(5), expired: true });
    const { p, v } = await loaded({ 'POST invites/i3/resend': [{ status: 502, body: { error: 'mail_failed' } }] }, [
      ...USERS,
      old,
    ]);
    await p.resend(listed(p, 'i3') as ListedUser);
    expect(row(v(), 'eve.ross@example.com')).toMatchObject({
      hasSendNote: true,
      canCopy: false,
      canExtend: true,
      sendNote:
        'The invitation couldn’t be resent, and its link no longer works. Extend it and share the link yourself, or try again later.',
    });
  });

  it('names the wait time of a rate-limited "Resend" and a gone invitation', async () => {
    const { p, calls } = await loaded({
      'POST invites/i2/resend': [
        { status: 429, body: { error: 'rate_limited', retry_after: 72 } },
        { status: 404, body: { error: 'not_found' } },
      ],
    });
    await p.resend(listed(p, 'i2') as ListedUser);
    expect(ui.toast).toMatchObject({ text: 'Too many requests. Try again in 2 minutes.', error: true });
    expect(loads(calls)).toBe(1);
    await p.resend(listed(p, 'i2') as ListedUser);
    expect(ui.toast).toMatchObject({ text: 'This entry no longer exists. The list has been updated.', error: true });
    await vi.waitFor(() => expect(loads(calls)).toBe(2));
  });

  it('extends an expired invitation and shows the new row', async () => {
    const old = invite('i3', 'eve.ross@example.com', { expires_at: before(5), expired: true });
    const { p, v } = await loaded(
      {
        'POST invites/i3/renew': [
          { body: { invite: { ...old, expires_at: before(-168), expired: false, mail_sent_at: old.mail_sent_at } } },
        ],
      },
      [...USERS, old],
    );
    p.sendFail = 'i3';
    await p.extend(listed(p, 'i3') as ListedUser);
    expect(ui.toast?.text).toBe('Invitation extended until 2 November');
    expect(p.sendFail).toBeNull();
    expect(row(v(), 'eve.ross@example.com')).toMatchObject({ expired: false, canCopy: true });
  });

  it('shows an invitation as expired as soon as its time is up, without reloading', async () => {
    const soon = invite('i3', 'eve.ross@example.com', { expires_at: before(-1) });
    const { p } = await loaded({}, [...USERS, soon]);
    const later = usersValues(p, ctx, NOW + 2 * 3600_000);
    expect(row(later, 'eve.ross@example.com')).toMatchObject({ expired: true, canExtend: true });
  });

  it('reports "Couldn’t copy" if the clipboard refuses', async () => {
    vi.stubGlobal('navigator', {
      ...navigator,
      clipboard: { writeText: vi.fn(() => Promise.reject(new Error('no'))) },
    });
    const { p } = await loaded({ 'GET invites/i1/link': [{ body: { url: 'https://hub.example.com/invite/x' } }] });
    await p.copyLink({ id: 'i1' });
    expect(ui.toast).toMatchObject({ text: 'Couldn’t copy.', error: true });
  });

  it('reloads the account without a session or role instead of the list', async () => {
    const { p, calls } = await loaded({
      'POST users/u3/unlock': [{ status: 403, body: { error: 'forbidden' } }],
      'GET session': [{ status: 401, body: { error: 'unauthorized' } }],
    });
    p.unlock(listed(p, 'u3') as ListedUser);
    await vi.waitFor(() => expect(sent(calls)).toContain('GET session'));
    expect(loads(calls)).toBe(1);
  });

  it('after leaving the page only reports a failed action, without writing state', async () => {
    const patch = held();
    const { p, calls } = await loaded({ 'PATCH users/u2': [patch.reply] });
    p.setRole(listed(p, 'u2') as ListedUser, 'admin');
    p.dispose();
    patch.release({ status: 500, body: { error: 'internal' } });
    await vi.waitFor(() => expect(ui.toast).toMatchObject({ error: true }));
    expect(loads(calls)).toBe(1);
  });

  it('rows derive without the form or the menu', async () => {
    const { p } = await loaded();
    expect(rowsValues(p, ctx, NOW)).toHaveLength(6);
  });
});

describe('App access', () => {
  const APPS_ROUTE = (apps = [pairedApp(), pendingApp(), offlineApp()]) => always({ body: { apps } }, 5);
  const withApps = (u: ListedUser, apps: Record<string, string>): ListedUser => ({ ...u, apps });
  const ACCESS_USERS = USERS.map((u) =>
    u.id === 'u2' ? withApps(u, { mail: 'user' }) : u.id === 'i1' ? withApps(u, { notes: 'user' }) : u,
  );

  async function withAccess(routes: Parameters<typeof mockApi>[0] = {}) {
    const ready = await loaded({ 'GET admin/apps': APPS_ROUTE(), ...routes }, ACCESS_USERS);
    await vi.waitFor(() => expect(ready.p.paired.map((a) => a.slug)).toEqual(['mail', 'notes']));
    return ready;
  }

  it('offers "No access" or a role per paired app in the expanded row', async () => {
    const { p, v } = await withAccess({ 'GET users/u2': [{ body: { user: { sessions: [] } } }] });
    expect(row(v(), 'Luke Walker').access).toEqual([]);
    p.toggleOpen('u2');
    const access = row(v(), 'Luke Walker').access;
    expect(access.map((a) => [a.name, a.label, a.options.filter((o) => o.on).map((o) => o.label)])).toEqual([
      ['Mail', 'Access to Mail', ['User']],
      ['Notes', 'Access to Notes', ['No access']],
    ]);
  });

  it('changes a user’s role and access at once, and keeps the choice over a list loaded meanwhile', async () => {
    const put = held();
    const { p, v, calls, server } = await withAccess({
      'GET users/u2': always({ body: { user: { sessions: [] } } }),
      'PUT users/u2/apps/mail': [put.reply],
      'DELETE users/u2/apps/notes': [{ body: { user: withApps(USERS[2] as ListedUser, { mail: 'admin' }) } }],
    });
    p.toggleOpen('u2');
    row(v(), 'Luke Walker').access[0]?.pick('admin');
    expect(listed(p, 'u2')?.apps).toEqual({ mail: 'admin' });
    // A list from before the response does not take it back.
    await p.load();
    expect(listed(p, 'u2')?.apps).toEqual({ mail: 'admin' });
    put.release({ body: { user: withApps(USERS[2] as ListedUser, { mail: 'admin' }) } });
    await vi.waitFor(() => expect(ui.toast?.text).toBe('Access to Mail changed'));
    expect(calls.find((c) => c.method === 'PUT')).toMatchObject({
      path: 'users/u2/apps/mail',
      body: { app_role: 'admin' },
    });
    // Nothing to do for the same value.
    row(v(), 'Luke Walker').access[0]?.pick('admin');
    expect(calls.filter((c) => c.method === 'PUT')).toHaveLength(1);
    server.users = ACCESS_USERS;
    p.setAccess(listed(p, 'u2') as ListedUser, 'notes', 'user');
    row(v(), 'Luke Walker').access[1]?.pick(null);
    expect(calls.at(-1)).toMatchObject({ method: 'DELETE', path: 'users/u2/apps/notes' });
  });

  it('changes an invitation’s access through the invitation', async () => {
    const { p, calls } = await withAccess({
      'PATCH invites/i1': [{ body: { invite: withApps(USERS[4] as ListedUser, {}) } }],
    });
    p.setAccess(listed(p, 'i1') as ListedUser, 'notes', null);
    await vi.waitFor(() => expect(ui.toast?.text).toBe('Access to Notes removed'));
    expect(calls.at(-1)).toMatchObject({ method: 'PATCH', path: 'invites/i1', body: { apps: { notes: null } } });
    expect(listed(p, 'i1')?.apps).toEqual({});
  });

  it('goes back and reloads the apps when the app is pending, the role gone, or the request failed', async () => {
    const { p, calls } = await withAccess({
      'PUT users/u1/apps/mail': [
        { status: 409, body: { error: 'app_pending' } },
        { status: 422, body: { error: 'invalid', fields: { app_role: { code: 'invalid' } } } },
        { status: 500, body: { error: 'internal' } },
      ],
    });
    const appsLoads = () => calls.filter((c) => c.path === 'admin/apps').length;
    const anna = () => listed(p, 'u1') as ListedUser;
    p.setAccess(anna(), 'mail', 'user');
    await vi.waitFor(() => expect(ui.toast?.text).toBe('This app isn’t paired yet. The list was updated.'));
    expect(anna().apps ?? {}).toEqual({});
    expect(appsLoads()).toBe(2);
    p.setAccess(anna(), 'mail', 'admin');
    await vi.waitFor(() => expect(ui.toast?.text).toBe('This role no longer exists in the app. The list was updated.'));
    p.setAccess(anna(), 'mail', 'admin');
    await vi.waitFor(() => expect(ui.toast?.text).toBe('Access wasn’t changed. Please try again.'));
    expect(anna().apps ?? {}).toEqual({});
  });

  it('a later change of the same app wins over an earlier response', async () => {
    const first = held();
    const { p } = await withAccess({
      'PUT users/u1/apps/mail': [first.reply, { body: { user: withApps(USERS[1] as ListedUser, { mail: 'admin' }) } }],
    });
    p.setAccess(listed(p, 'u1') as ListedUser, 'mail', 'user');
    p.setAccess(listed(p, 'u1') as ListedUser, 'mail', 'admin');
    await vi.waitFor(() => expect(ui.toast?.text).toBe('Access to Mail changed'));
    first.release({ status: 500, body: { error: 'internal' } });
    await tick();
    expect(listed(p, 'u1')?.apps).toEqual({ mail: 'admin' });
    // The earlier failure reports nothing: the later change decided.
    expect(ui.toast?.text).toBe('Access to Mail changed');
  });

  it('presets the invitation form from each app’s default and sends only the changed apps', async () => {
    const { p, v, calls } = await withAccess({
      'POST invites': [
        {
          status: 422,
          body: { error: 'invalid', fields: { 'apps.notes': { code: 'invalid' } } },
        },
        { status: 201, body: { invites: [{ id: 'i9', email: 'tim@example.com', sent: true }], system_mail: true } },
      ],
    });
    p.openInvite();
    expect(v().invApps.map((a) => [a.slug, a.options.find((o) => o.on)?.value])).toEqual([
      ['mail', 'user'],
      ['notes', null],
    ]);
    p.inviteInput('tim@example.com ');
    await p.sendInvites();
    expect(calls.filter((c) => c.method === 'POST').at(-1)?.body).not.toHaveProperty('apps');
    // The first send failed on the role in Notes: shown at the app, sending locked until it changes.
    expect(v().invApps[1]?.err).toBe('This role no longer exists in the app. Choose another one.');
    expect(v().invErr).toBe('Please check the app access.');
    v().invApps[1]?.pick('admin');
    expect(v().invApps[1]?.err).toBe('');
    v().invApps[0]?.pick(null);
    await p.sendInvites();
    expect(calls.filter((c) => c.method === 'POST').at(-1)?.body).toMatchObject({
      emails: ['tim@example.com'],
      apps: { mail: null, notes: 'admin' },
    });
  });

  it('gives access to the selection per app, leaving unchanged apps alone', async () => {
    const { p, v, calls } = await withAccess({
      'POST users/bulk': [{ body: { changed: ['u2', 'u3'] } }, { body: { changed: ['u3'] } }],
    });
    p.toggleSelect('u2');
    p.toggleSelect('u3');
    expect(v().uCanBulkAccess).toBe(true);
    v().uBulkAccessToggle();
    const b = () => v().uBulkAccess;
    expect(b()?.title).toBe('App access for 2 people');
    expect(b()?.rows.map((r) => r.unchanged.on)).toEqual([true, true]);
    expect(b()?.canApply).toBe(false);
    b()?.rows[0]?.pick('admin');
    b()?.rows[1]?.pick(null);
    expect(b()?.canApply).toBe(true);
    b()?.rows[1]?.unchanged.pick();
    b()?.rows[1]?.pick(null);
    b()?.apply();
    await vi.waitFor(() => expect(ui.toast?.text).toBe('App access changed for 2 people'));
    expect(calls.filter((c) => c.path === 'users/bulk').map((c) => c.body)).toEqual([
      { action: 'access', ids: ['u2', 'u3'], slug: 'mail', access: true, app_role: 'admin' },
      { action: 'access', ids: ['u2', 'u3'], slug: 'notes', access: false },
    ]);
    expect(v().uBulkAccessOpen).toBe(false);
  });

  it('closes the bulk action with the selection and offers it only with paired apps', async () => {
    const { p, v } = await withAccess();
    p.toggleSelect('u2');
    p.openBulkAccess();
    p.toggleSelect('u2');
    expect(p.bulkAccess).toBeNull();
    p.apps = [pendingApp()];
    expect(v().uCanBulkAccess).toBe(false);
  });
});
