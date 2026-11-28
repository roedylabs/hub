import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { mockApi } from '#lib/state/fetch-mock.ts';
import { session } from '#lib/state/session.svelte.ts';
import { ApiError, ui } from '@roedy/ui';
import { resetSignedIn, signIn, TEST_USER } from '#lib/test/signed-in.ts';
import { sessionOf } from '#lib/test/session.ts';
import {
  changedBody,
  connOf,
  draftOf,
  firstBadSection,
  liveErrors,
  mailErrorsOf,
  mailErrorText,
  NO_MAIL,
  precheck,
  reasonText,
  requiredErrors,
  SystemMailPage,
  systemMailValues,
  type MailServer,
  type SystemMail,
  type SystemMailInput,
} from './system-mail.svelte.ts';

const IDS = [{ id: 'i1', name: 'Example Org', email: 'noreply@example.com' }];
/** A stored, checked account. */
const STORED: SystemMail = {
  server_id: 's1',
  auth: 'password',
  user: 'noreply@example.com',
  has_secret: true,
  identity_id: 'i1',
  sender_name: null,
  reply_to: null,
  keep_sent: true,
  status: 'ok',
  reason: null,
  checked_at: '2026-11-26T10:00:00Z',
  identities: IDS,
};

/** Servers of the Stalwart registry. */
const SERVERS: MailServer[] = [
  { id: 's1', label: 'Example mail', url: 'https://mail.example.com', host: 'mail.example.com' },
  { id: 's2', label: 'Other mail', url: 'https://other.example.com:8443', host: 'other.example.com:8443' },
];
/** `GET admin/stalwart/servers` with the fields the page does not use. */
const registry = (servers: MailServer[] = SERVERS) => ({
  body: {
    servers: servers.map((s) => ({
      ...s,
      domains: [],
      client_id: null,
      has_secret: false,
      reach: 'ok',
      cors: 'ok',
      oauth: 'ok',
      tested_at: null,
      accounts: 0,
      created_at: '2026-11-01T10:00:00Z',
    })),
    redirect_uri: 'https://hub.example.com/oauth/stalwart',
    system_mail_server: servers[0]?.id ?? null,
  },
});

const scrollTo = vi.fn();
const show = vi.fn();
const mark = vi.fn();

beforeEach(() => {
  signIn('admin');
});

afterEach(() => {
  scrollTo.mockClear();
  show.mockClear();
  mark.mockClear();
  resetSignedIn();
});

const sent = (calls: { method: string; path: string }[]) => calls.map((c) => `${c.method} ${c.path}`);
type Reply = { status?: number; body?: unknown };

/** A response that waits until `answer` is called. */
function later() {
  let answer: (reply: Reply) => void = () => {};
  const reply = () => new Promise<Reply>((resolve) => (answer = resolve));
  return { reply, answer: (r: Reply) => answer(r) };
}

async function loaded(
  mail: SystemMail = STORED,
  routes: Parameters<typeof mockApi>[0] = {},
  servers: MailServer[] = SERVERS,
) {
  const api = mockApi({
    'GET admin/system-mail': [{ body: mail }],
    'GET admin/stalwart/servers': [registry(servers)],
    ...routes,
  });
  const p = new SystemMailPage(scrollTo);
  await p.load();
  return { p, api };
}

const values = (p: SystemMailPage, now?: number, pick: Partial<SystemMailInput> = {}) =>
  systemMailValues(p, { now, open: null, hi: null, isMobile: false, ...pick }, { scrollTo, show, mark });

describe('draft, connection and body', () => {
  it('draft from the stored account without secrets', () => {
    expect(draftOf(STORED)).toEqual({
      srv: 's1',
      auth: 'password',
      user: 'noreply@example.com',
      pass: '',
      token: '',
      name: '',
      replyTo: '',
      keepSent: true,
    });
    expect(draftOf(NO_MAIL).srv).toBe('');
  });

  it('connection: server, user only with password, sender does not count', () => {
    const d = draftOf(STORED);
    expect(connOf({ ...d, srv: 's2' })).not.toBe(connOf(d));
    expect(connOf({ ...d, user: ' noreply@example.com ' })).toBe(connOf(d));
    expect(connOf({ ...d, name: 'Other' })).toBe(connOf(d));
    expect(connOf({ ...d, auth: 'token', user: 'x' })).toBe(connOf({ ...d, auth: 'token', user: 'y' }));
    expect(connOf({ ...d, pass: 'pw' })).not.toBe(connOf(d));
  });

  it('PUT sends only what changed, secrets only when entered', () => {
    const base = draftOf(STORED);
    expect(changedBody({ ...base, user: ' noreply@example.com ' }, base)).toEqual({});
    expect(changedBody({ ...base, name: ' Example ', replyTo: '', keepSent: false }, base)).toEqual({
      sender_name: 'Example',
      keep_sent: false,
    });
    expect(changedBody({ ...base, srv: 's2', pass: ' pw ' }, base)).toEqual({
      server_id: 's2',
      app_password: ' pw ',
    });
    expect(changedBody({ ...base, auth: 'token', token: ' tok ' }, base)).toEqual({ auth: 'token', token: 'tok' });
    // A new secret names the address of the server the page shows.
    expect(changedBody({ ...base, pass: 'pw' }, base, SERVERS)).toEqual({
      app_password: 'pw',
      server_url: 'https://mail.example.com',
    });
    expect(changedBody({ ...base, name: 'Example' }, base, SERVERS)).toEqual({ sender_name: 'Example' });
  });

  it('a moved server is an error of the server field', () => {
    const error = new ApiError(422, {
      error: 'invalid',
      fields: { server_id: { code: 'changed', url: 'https://mail.example.net' } },
    });
    expect(mailErrorsOf(error)).toEqual({ srv: 'changed' });
    expect(mailErrorText('srv', 'changed')).toContain('address has changed');
  });

  it('checks fields live like the backend', () => {
    const d = draftOf(STORED);
    expect(liveErrors(d)).toEqual({});
    expect(liveErrors({ ...d, user: 'no reply', name: 'x'.repeat(101), replyTo: 'no-at' })).toEqual({
      user: 'invalid',
      name: 'too_long',
      replyTo: 'invalid',
    });
    expect(liveErrors({ ...d, name: `Ex${String.fromCharCode(0x200b)}ample` }).name).toBe('invalid');
    expect(liveErrors({ ...d, pass: 'x'.repeat(1025) }).pass).toBe('invalid');
    expect(liveErrors({ ...d, auth: 'token', token: `a${String.fromCharCode(7)}b` }).token).toBe('invalid');
    expect(liveErrors({ ...d, replyTo: `${'a'.repeat(250)}@example.org` }).replyTo).toBe('too_long');
    // No server chosen is not a live error (only on saving or testing).
    expect(liveErrors({ ...d, srv: '' })).toEqual({});
  });

  it('required: a server of the registry, user with password, a secret – a stored one of the same method on the same server counts', () => {
    const d = draftOf(STORED);
    expect(requiredErrors(d, STORED, SERVERS)).toEqual({});
    expect(requiredErrors({ ...d, srv: 's2' }, STORED, SERVERS)).toEqual({ pass: 'required' });
    expect(requiredErrors({ ...d, auth: 'token' }, STORED, SERVERS)).toEqual({ token: 'required' });
    expect(requiredErrors(draftOf(NO_MAIL), NO_MAIL, SERVERS)).toEqual({
      srv: 'required',
      user: 'required',
      pass: 'required',
    });
    // Removed from the registry meanwhile; an empty registry needs a server too.
    expect(requiredErrors({ ...d, srv: 'gone' }, STORED, SERVERS)).toEqual({ srv: 'not_found', pass: 'required' });
    expect(requiredErrors({ ...draftOf(NO_MAIL), user: 'a', pass: 'pw' }, NO_MAIL, [])).toEqual({ srv: 'required' });
    expect(firstBadSection({ replyTo: 'invalid' })).toBe('sender');
    expect(firstBadSection({ replyTo: 'invalid', token: 'required' })).toBe('conn');
    expect(firstBadSection({})).toBeNull();
  });

  it('checks beforehand', () => {
    const d = draftOf(STORED);
    expect(precheck(d, STORED, SERVERS)).toBeNull();
    expect(precheck({ ...d, srv: '' }, STORED, SERVERS)).toBe('Choose the mail server.');
    expect(precheck({ ...d, srv: 'gone' }, STORED, SERVERS)).toBe(
      'The server is no longer in the server list. Choose another one.',
    );
    expect(precheck(d, STORED, [])).toBe('The server is no longer in the server list. Choose another one.');
    expect(precheck({ ...d, auth: 'token' }, STORED, SERVERS)).toBe('Sign-in failed. No token provided.');
    expect(precheck({ ...d, user: '' }, STORED, SERVERS)).toBe('Sign-in failed. Enter a username and password.');
    expect(precheck({ ...d, srv: 's2' }, STORED, SERVERS)).toBe('Sign-in failed. Enter a username and password.');
    expect(precheck({ ...d, srv: 's2', pass: 'pw' }, STORED, SERVERS)).toBeNull();
  });

  it('texts per reason', () => {
    expect(reasonText('unreachable')).toBe('Can’t reach the server. Check the address.');
    expect(reasonText('auth_failed')).toBe('Sign-in failed. The username or password is incorrect.');
    expect(reasonText('auth_failed', { tok: true })).toBe(
      'Sign-in failed. The access token is incorrect, expired, or revoked.',
    );
    expect(reasonText('auth_locked', { wait: 900 })).toBe('Too many failed attempts. Try again in 15 minutes.');
    expect(reasonText('auth_locked')).toBe('Too many failed attempts. Try again later.');
    expect(reasonText('throttled')).toBe(
      'The mail server is throttling requests from Example Org. Test again in a few minutes.',
    );
    expect(reasonText('no_mailbox', { role: 'sent' })).toMatch(/“Sent” mailbox/);
    expect(reasonText('no_mailbox')).toMatch(/“Drafts” mailbox/);
    expect(reasonText('foreign_origin')).toMatch(/different address/);
    expect(reasonText('something_new')).toBe('That didn’t work. Please try again.');
  });

  it('reads field errors of a 422 per field of the draft', () => {
    const err = new ApiError(422, {
      error: 'invalid',
      fields: {
        server_id: { code: 'not_found' },
        app_password: { code: 'required' },
        reply_to: { code: 'too_long' },
        to: { code: 'invalid' },
      },
    });
    expect(mailErrorsOf(err)).toEqual({ srv: 'not_found', pass: 'required', replyTo: 'too_long' });
    expect(mailErrorsOf(new ApiError(400, { error: 'bad_request' }))).toEqual({});
  });
});

describe('loading and status', () => {
  it('connected: status, rows, sender', async () => {
    const { p, api } = await loaded();
    expect(sent(api.calls)).toEqual(['GET admin/system-mail', 'GET admin/stalwart/servers']);
    expect(p.status).toBe('ready');
    expect(p.servers).toEqual(SERVERS);
    const v = values(p, Date.parse('2026-11-26T12:00:00Z'));
    expect(v.mSt).toMatchObject({
      key: 'ok',
      title: 'Connected',
      sub: 'JMAP session with mail.example.com is active.',
    });
    expect(v.mailRows.map((r) => [r.k, r.v])).toEqual([
      ['Session', 'https://mail.example.com/.well-known/jmap'],
      ['Account', 'noreply@example.com'],
      ['Last checked', '2 hours ago'],
    ]);
    expect(v.mailFrom).toBe('Example Org <noreply@example.com>');
    expect(v.mailNamePh).toBe('Example Org');
    expect(v.mailSecret).toMatchObject({ label: 'Password', ph: '••••••••', type: 'password', bad: false });
    expect(v.mailSrvSub).toBe('JMAP session via mail.example.com');
    expect(v.mailSrvHas).toBe(true);
    expect(v.selMailSrv).toMatchObject({ title: 'Mail server', label: 'Example mail', open: false, disabled: false });
    expect(v.selMailSrv.opts.map((o) => [o.label, o.sub, o.on])).toEqual([
      ['Example mail', 'mail.example.com', true],
      ['Other mail', 'other.example.com:8443', false],
    ]);
    expect(v.mailDirty).toBe(false);
  });

  it('without an account "Not set up"; a chosen server is "Not checked"', async () => {
    const { p } = await loaded(NO_MAIL);
    expect(values(p).mSt).toMatchObject({ key: 'none', title: 'Not set up' });
    expect(values(p).mailSecret.ph).toBe('');
    expect(values(p).mailSrvSub).toBe('The sending account’s Stalwart server');
    expect(values(p).selMailSrv.label).toBe('Choose server');
    p.set('srv', 's1');
    expect(values(p).mSt).toMatchObject({ key: 'new', title: 'Not checked' });
    expect(values(p).selMailSrv.label).toBe('Example mail');
  });

  it('the pick list opens on the desktop, marks a row and chooses with a click', async () => {
    const { p } = await loaded(STORED);
    values(p).selMailSrv.toggle();
    expect(show).toHaveBeenLastCalledWith('mailsrv');
    let v = values(p, undefined, { open: 'mailsrv', hi: 1 });
    expect(v.selMailSrv).toMatchObject({ open: true, active: 1 });
    expect(values(p, undefined, { open: 'mailsrv', isMobile: true }).selMailSrv.open).toBe(false);
    v.selMailSrv.opts[1]?.onClick();
    expect(show).toHaveBeenLastCalledWith(null);
    expect(p.draft.srv).toBe('s2');
    v = values(p);
    expect(v.selMailSrv.label).toBe('Other mail');
    expect(v.mailSrvSub).toBe('JMAP session via other.example.com:8443');
    expect(v.mSt.key).toBe('new');
  });

  it('a server removed from the registry asks for another one', async () => {
    const { p } = await loaded({ ...STORED, server_id: null, status: 'err', reason: 'unreachable' });
    const v = values(p);
    expect(v.selMailSrv.label).toBe('Choose server');
    expect(v.selMailSrv.opts.some((o) => o.on)).toBe(false);
    // The stored password belonged to the removed server.
    expect(v.mailSecret.ph).toBe('');
  });

  it('an empty registry: no pick list, saving and testing ask for the server', async () => {
    const { p, api } = await loaded(NO_MAIL, {}, []);
    expect(values(p)).toMatchObject({ mailSrvHas: false, mailSrvSub: 'The sending account’s Stalwart server' });
    expect(values(p).selMailSrv.disabled).toBe(true);
    p.set('user', 'noreply@example.com');
    p.set('pass', 'pw');
    await p.save();
    expect(p.errors).toEqual({ srv: 'required' });
    expect(values(p).mailErr.srv).toMatchObject({ bad: true, err: 'Choose the mail server.' });
    expect(scrollTo).toHaveBeenCalledWith('conn');
    await p.test();
    expect(values(p).mSt).toMatchObject({ key: 'err', sub: 'Choose the mail server.' });
    expect(sent(api.calls)).toEqual(['GET admin/system-mail', 'GET admin/stalwart/servers']);
  });

  it('a stored error shows the reason and names `role`', async () => {
    const { p } = await loaded({ ...STORED, status: 'err', reason: 'no_mailbox', role: 'sent' });
    expect(values(p).mSt).toMatchObject({ key: 'err', title: 'No connection' });
    expect(values(p).mSt.sub).toMatch(/“Sent” mailbox/);
  });

  it('unchecked checks silently with the stored account after loading', async () => {
    const { p, api } = await loaded(
      { ...STORED, status: 'unchecked', checked_at: null, identities: [] },
      { 'POST admin/system-mail/test': [{ body: { identities: IDS, sent: false } }] },
    );
    await vi.waitFor(() => expect(p.key).toBe('ok'));
    expect(api.calls[2]).toMatchObject({
      path: 'admin/system-mail/test',
      body: { server_id: 's1', auth: 'password', user: 'noreply@example.com' },
    });
    expect(ui.toast).toBeNull();
  });

  it('unchecked without a response stays "Not checked" with its own text, without a toast', async () => {
    const { p } = await loaded({ ...STORED, status: 'unchecked', checked_at: null });
    await vi.waitFor(() => expect(p.busy).toBe(false));
    expect(values(p).mSt).toMatchObject({
      key: 'unchecked',
      sub: 'The sending account is saved but not checked yet. Test the connection.',
    });
    expect(ui.toast).toBeNull();
  });

  it('an error while loading shows the card; 403 reloads the account', async () => {
    const api = mockApi({
      'GET admin/system-mail': [{ status: 403, body: { error: 'forbidden' } }],
      'GET admin/stalwart/servers': [{ status: 403, body: { error: 'forbidden' } }],
      'GET session': [{ body: sessionOf({ ...TEST_USER, role: 'user' }) }],
    });
    const p = new SystemMailPage(scrollTo);
    await p.load();
    expect(p.status).toBe('failed');
    expect(p.failure).toBe('That didn’t work. Please try again.');
    await vi.waitFor(() => expect(session.isAdmin).toBe(false));
    expect(sent(api.calls)).toEqual(['GET admin/system-mail', 'GET admin/stalwart/servers', 'GET session']);
  });

  it('"Try again" shows the skeleton again and loads', async () => {
    mockApi({
      'GET admin/system-mail': [{ status: 500, body: { error: 'internal' } }, { body: STORED }],
      'GET admin/stalwart/servers': [registry(), registry()],
    });
    const p = new SystemMailPage(scrollTo);
    await p.load();
    expect(p.status).toBe('failed');
    const run = p.retry();
    expect(p.status).toBe('loading');
    await run;
    expect(p.status).toBe('ready');
  });

  it('only the latest of two overlapping loads applies; nothing after leaving', async () => {
    const first = later();
    mockApi({
      'GET admin/system-mail': [first.reply, { body: { ...STORED, sender_name: 'Newer' } }],
      'GET admin/stalwart/servers': [registry(), registry()],
    });
    const p = new SystemMailPage(scrollTo);
    const a = p.load();
    await p.load();
    first.answer({ body: { ...STORED, sender_name: 'Older' } });
    await a;
    expect(p.draft.name).toBe('Newer');

    const late = later();
    mockApi({ 'GET admin/system-mail': [late.reply], 'GET admin/stalwart/servers': [registry()] });
    const q = new SystemMailPage(scrollTo);
    const run = q.load();
    q.dispose();
    late.answer({ body: STORED });
    await run;
    expect(q.status).toBe('loading');
  });
});

describe('test connection', () => {
  it('checks the draft with the entered secret and remembers the connection', async () => {
    const { p, api } = await loaded(NO_MAIL, {
      'POST admin/system-mail/test': [{ body: { identities: IDS, sent: false } }],
    });
    p.set('srv', 's1');
    p.set('user', ' noreply@example.com ');
    p.set('pass', 'pw');
    await p.test();
    expect(api.calls[2]).toMatchObject({
      method: 'POST',
      path: 'admin/system-mail/test',
      body: { server_id: 's1', auth: 'password', user: 'noreply@example.com', app_password: 'pw' },
    });
    expect(p.key).toBe('ok');
    expect(ui.toast).toMatchObject({ text: 'Connection established', error: false });
    expect(values(p).mailRows[0]?.v).toBe('https://mail.example.com/.well-known/jmap');
  });

  it('pre-check without a request', async () => {
    const { p, api } = await loaded(NO_MAIL);
    await p.test();
    expect(sent(api.calls)).toEqual(['GET admin/system-mail', 'GET admin/stalwart/servers']);
    expect(values(p).mSt).toMatchObject({ key: 'err', sub: 'Choose the mail server.' });
    expect(ui.toast).toMatchObject({ text: 'Connection failed', error: true });
  });

  it('rejected sign-in and lock with wait time; a good connection is no longer good', async () => {
    const { p } = await loaded(STORED, {
      'POST admin/system-mail/test': [
        { status: 422, body: { error: 'mail_failed', reason: 'auth_failed' } },
        { status: 429, body: { error: 'mail_failed', reason: 'auth_locked', retry_after: 600 } },
      ],
    });
    await p.test();
    expect(values(p).mSt).toMatchObject({ key: 'err', sub: 'Sign-in failed. The username or password is incorrect.' });
    await p.test();
    expect(values(p).mSt.sub).toBe('Too many failed attempts. Try again in 10 minutes.');
    expect(p.good).toBeNull();
  });

  it('server error, 429 and no response keep the status, only an error toast', async () => {
    const { p } = await loaded(STORED, {
      'POST admin/system-mail/test': [
        { status: 500, body: { error: 'internal' } },
        { status: 429, body: { error: 'rate_limited', retry_after: 120 } },
      ],
    });
    await p.test();
    expect(p.key).toBe('ok');
    expect(ui.toast).toMatchObject({ text: 'That didn’t work. Please try again.', error: true });
    await p.test();
    expect(p.key).toBe('ok');
    expect(ui.toast).toMatchObject({ text: 'Too many requests. Try again in 2 minutes.', error: true });
    await p.test();
    expect(p.key).toBe('ok');
    expect(ui.toast).toMatchObject({ text: 'No connection. Please try again.', error: true });
  });

  it('a 422 per field names the field in the status', async () => {
    const { p } = await loaded(STORED, {
      'POST admin/system-mail/test': [
        { status: 422, body: { error: 'invalid', fields: { server_id: { code: 'not_found' } } } },
      ],
    });
    await p.test();
    expect(values(p).mSt.sub).toBe('The server is no longer in the server list. Choose another one.');
  });

  it('a second test while one runs does nothing; a result after leaving writes nothing', async () => {
    const res = later();
    const { p, api } = await loaded(STORED, { 'POST admin/system-mail/test': [res.reply] });
    const run = p.test();
    await vi.waitFor(() => expect(p.busy).toBe(true));
    await p.test();
    expect(api.calls.filter((c) => c.method === 'POST')).toHaveLength(1);
    p.dispose();
    res.answer({ status: 422, body: { error: 'mail_failed', reason: 'auth_failed' } });
    await run;
    expect(p.result).toBe('ok');
    expect(ui.toast).toBeNull();
  });

  it('another account is checked with the first identity unless the check knew the stored one', async () => {
    const { p, api } = await loaded(STORED, {
      'POST admin/system-mail/test': [
        { body: { identities: IDS, sent: false } },
        { body: { identities: IDS, sent: false } },
      ],
      'PUT admin/system-mail': [{ body: { ...STORED, user: 'other@example.com' } }],
    });
    p.set('user', 'other@example.com');
    p.set('pass', 'pw');
    await p.test();
    expect(api.calls[2]?.body).toMatchObject({ identity_id: null });
    await p.test();
    // The last check of this connection returned the stored identity: it stays.
    expect(api.calls[3]?.body).not.toHaveProperty('identity_id');
  });
});

describe('saving', () => {
  it('sends only what changed, shows "Checking" for a new connection and takes over the status', async () => {
    const put = later();
    const { p, api } = await loaded(STORED, { 'PUT admin/system-mail': [put.reply] });
    p.set('pass', 'new-pw');
    p.set('name', ' Example ');
    const run = p.save();
    expect(p.saving).toBe(true);
    expect(p.key).toBe('busy');
    expect(api.calls[2]).toMatchObject({
      method: 'PUT',
      path: 'admin/system-mail',
      body: { app_password: 'new-pw', sender_name: 'Example' },
    });
    put.answer({ body: { ...STORED, sender_name: 'Example', checked_at: '2026-11-26T12:00:00Z' } });
    await run;
    expect(p.saving).toBe(false);
    expect(p.dirty).toBe(false);
    expect(p.draft.pass).toBe('');
    expect(p.key).toBe('ok');
    expect(ui.toast).toMatchObject({ text: 'Changes saved', error: false });
  });

  it('a new connection that fails: saved, status with the reason, error toast', async () => {
    const { p } = await loaded(STORED, {
      'PUT admin/system-mail': [{ body: { ...STORED, server_id: 's2', status: 'err', reason: 'unreachable' } }],
    });
    p.set('srv', 's2');
    p.set('pass', 'pw');
    await p.save();
    expect(values(p).mSt).toMatchObject({ key: 'err', sub: 'Can’t reach the server. Check the address.' });
    expect(ui.toast).toMatchObject({ text: 'Changes saved, but the connection failed', error: true });
  });

  it('with a missing secret or a field error: toast, jump, fields marked, nothing sent', async () => {
    const { p, api } = await loaded(STORED);
    p.set('srv', 's2');
    await p.save();
    expect(ui.toast).toMatchObject({ text: 'Please check the highlighted fields', error: false });
    expect(scrollTo).toHaveBeenCalledWith('conn');
    expect(p.errors).toEqual({ pass: 'required' });
    expect(values(p).mailSecret).toMatchObject({ bad: true, err: 'Enter the password.' });
    p.set('pass', 'pw');
    expect(p.errors).toEqual({});
    p.set('replyTo', 'no-at');
    await p.save();
    expect(scrollTo).toHaveBeenLastCalledWith('sender');
    expect(values(p).mailErr.replyTo).toMatchObject({ bad: true, err: 'Please enter a valid email address.' });
    expect(sent(api.calls)).toEqual(['GET admin/system-mail', 'GET admin/stalwart/servers']);
  });

  it('required errors follow the draft: the stored server chosen again brings the stored secret back', async () => {
    const { p, api } = await loaded(STORED, {
      'PUT admin/system-mail': [{ body: { ...STORED, sender_name: 'Example' } }],
    });
    p.set('srv', 's2');
    await p.save();
    expect(p.errors).toEqual({ pass: 'required' });
    // Back to the stored server: the stored password applies again.
    p.set('srv', 's1');
    p.set('name', 'Example');
    expect(p.errors).toEqual({});
    await p.save();
    expect(sent(api.calls)).toEqual(['GET admin/system-mail', 'GET admin/stalwart/servers', 'PUT admin/system-mail']);
    expect(api.calls[2]?.body).toEqual({ sender_name: 'Example' });
  });

  it('a hidden field of the other sign-in method does not block saving', async () => {
    const { p, api } = await loaded(STORED, {
      'PUT admin/system-mail': [
        { status: 422, body: { error: 'invalid', fields: { app_password: { code: 'invalid' } } } },
        { body: { ...STORED, auth: 'token', status: 'ok' } },
      ],
    });
    p.set('srv', 's2');
    await p.save();
    expect(p.errors).toEqual({ pass: 'required' });
    p.set('srv', 's1');
    p.set('pass', 'pw');
    await p.save();
    expect(p.errors).toEqual({ pass: 'invalid' });
    // Token: the password field is hidden, its errors do not count.
    p.set('auth', 'token');
    p.set('token', 'tok');
    expect(p.errors).toEqual({});
    await p.save();
    expect(sent(api.calls)).toEqual([
      'GET admin/system-mail',
      'GET admin/stalwart/servers',
      'PUT admin/system-mail',
      'PUT admin/system-mail',
    ]);
    expect(api.calls[3]?.body).toMatchObject({ auth: 'token', token: 'tok' });
    expect(api.calls[3]?.body).not.toHaveProperty('app_password');
  });

  it('a saved connection change that fails its check clears the identities of the previous account', async () => {
    const { p } = await loaded(STORED, {
      'PUT admin/system-mail': [
        {
          body: {
            ...STORED,
            server_id: 's2',
            user: 'sender@example.com',
            status: 'err',
            reason: 'auth_failed',
            identities: [],
          },
        },
      ],
    });
    expect(values(p).mailFrom).toBe('Example Org <noreply@example.com>');
    p.set('srv', 's2');
    p.set('user', 'sender@example.com');
    p.set('pass', 'pw');
    await p.save();
    expect(p.identities).toEqual([]);
    expect(values(p).mailFrom).toBe('sender@example.com');
  });

  it('a check that started before a save changed the connection is ignored', async () => {
    const post = later();
    const { p, api } = await loaded(
      { ...STORED, status: 'err', reason: 'unreachable' },
      {
        'POST admin/system-mail/test': [post.reply],
        'PUT admin/system-mail': [{ body: { ...STORED, status: 'err', reason: 'auth_failed' } }],
      },
    );
    const testing = p.test();
    await vi.waitFor(() => expect(sent(api.calls)).toContain('POST admin/system-mail/test'));
    // While the check of the stored password runs, a wrong new one is saved and fails its check.
    p.set('pass', 'wrong');
    await p.save();
    expect(p.result).toBe('err');
    post.answer({ body: { identities: IDS, sent: false } });
    await testing;
    expect(p.key).toBe('err');
    expect(values(p).mailSendDis).toBe(true);
    expect(p.busy).toBe(false);
  });

  it('a send that fails after a save changed the connection keeps the saved status', async () => {
    const post = later();
    const { p, api } = await loaded(STORED, {
      'POST admin/system-mail/test': [post.reply],
      'PUT admin/system-mail': [{ body: { ...STORED, checked_at: '2026-11-26T12:00:00Z' } }],
    });
    const sending = p.send();
    await vi.waitFor(() => expect(sent(api.calls)).toContain('POST admin/system-mail/test'));
    p.set('pass', 'new-pw');
    await p.save();
    expect(p.key).toBe('ok');
    post.answer({ status: 502, body: { error: 'mail_failed', reason: 'auth_failed' } });
    await sending;
    expect(p.key).toBe('ok');
    expect(ui.toast).toMatchObject({ text: 'Sign-in failed. The username or password is incorrect.', error: true });
  });

  it('a 422 shows the text at the field while it shows the sent value', async () => {
    const put = later();
    const { p } = await loaded(STORED, { 'PUT admin/system-mail': [put.reply] });
    p.set('name', 'Example');
    p.set('replyTo', 'help@example.com');
    const run = p.save();
    p.set('name', 'Changed');
    put.answer({
      status: 422,
      body: { error: 'invalid', fields: { sender_name: { code: 'invalid' }, reply_to: { code: 'too_long' } } },
    });
    await run;
    expect(p.errors).toEqual({ replyTo: 'too_long' });
    expect(scrollTo).toHaveBeenCalledWith('sender');
    expect(p.dirty).toBe(true);
    expect(ui.toast).toBeNull();
  });

  it('a 422 on a field hidden meanwhile (other sign-in method) shows the error toast', async () => {
    const put = later();
    const { p } = await loaded(STORED, { 'PUT admin/system-mail': [put.reply] });
    p.set('pass', 'pw');
    const run = p.save();
    // The password field is hidden with a token, but still holds the sent value.
    p.set('auth', 'token');
    put.answer({ status: 422, body: { error: 'invalid', fields: { app_password: { code: 'invalid' } } } });
    await run;
    expect(p.serverErrors).toEqual({});
    expect(scrollTo).not.toHaveBeenCalled();
    expect(ui.toast).toMatchObject({ text: 'That didn’t work. Please try again.', error: true });
  });

  it('a server error keeps the inputs with the secret and the bar', async () => {
    const { p } = await loaded(STORED, { 'PUT admin/system-mail': [{ status: 500, body: { error: 'internal' } }] });
    p.set('pass', 'pw');
    await p.save();
    expect(p.draft.pass).toBe('pw');
    expect(p.dirty).toBe(true);
    expect(p.busy).toBe(false);
    expect(ui.toast).toMatchObject({ text: 'That didn’t work. Please try again.', error: true });
  });

  it('inputs made while saving stay; a second save does nothing meanwhile; discard is blocked', async () => {
    const put = later();
    const { p, api } = await loaded(STORED, { 'PUT admin/system-mail': [put.reply] });
    p.set('name', 'Example');
    const run = p.save();
    p.set('replyTo', 'help@example.com');
    await p.save();
    p.discard();
    expect(api.calls.filter((c) => c.method === 'PUT')).toHaveLength(1);
    put.answer({ body: { ...STORED, sender_name: 'Example' } });
    await run;
    expect(p.edits).toEqual({ replyTo: 'help@example.com' });
    expect(p.dirty).toBe(true);
  });

  it('a test waits for a running save and then uses the stored secret', async () => {
    const put = later();
    const { p, api } = await loaded(STORED, {
      'PUT admin/system-mail': [put.reply],
      'POST admin/system-mail/test': [{ body: { identities: IDS, sent: false } }],
    });
    p.set('name', 'Example');
    const saving = p.save();
    const testing = p.test();
    expect(api.calls.filter((c) => c.method === 'POST')).toHaveLength(0);
    put.answer({ body: { ...STORED, sender_name: 'Example' } });
    await Promise.all([saving, testing]);
    expect(sent(api.calls)).toEqual([
      'GET admin/system-mail',
      'GET admin/stalwart/servers',
      'PUT admin/system-mail',
      'POST admin/system-mail/test',
    ]);
    expect(api.calls[3]?.body).not.toHaveProperty('app_password');
  });

  it('a load that started before a save does not overwrite its result', async () => {
    const get = later();
    const { p } = await loaded(STORED, {
      'PUT admin/system-mail': [{ body: { ...STORED, sender_name: 'Example' } }],
    });
    mockApi({
      'GET admin/system-mail': [get.reply],
      'GET admin/stalwart/servers': [registry()],
      'PUT admin/system-mail': [{ body: { ...STORED, sender_name: 'Example' } }],
    });
    const loading = p.load();
    p.set('name', 'Example');
    await p.save();
    get.answer({ body: STORED });
    await loading;
    expect(p.saved.sender_name).toBe('Example');
  });

  it('only whitespace changed: toast without a request', async () => {
    const { p, api } = await loaded({ ...STORED, sender_name: 'Example' });
    p.set('name', 'Example ');
    expect(p.dirty).toBe(true);
    await p.save();
    expect(ui.toast?.text).toBe('Changes saved');
    expect(p.dirty).toBe(false);
    expect(sent(api.calls)).toEqual(['GET admin/system-mail', 'GET admin/stalwart/servers']);
  });

  it('401 when saving reloads the account', async () => {
    const { p, api } = await loaded(STORED, {
      'PUT admin/system-mail': [{ status: 401, body: { error: 'unauthorized' } }],
      'GET session': [{ status: 401, body: { error: 'unauthorized' } }],
    });
    p.set('name', 'Example');
    await p.save();
    await vi.waitFor(() => expect(session.user).toBeNull());
    expect(sent(api.calls)).toContain('GET session');
  });
});

describe('test email', () => {
  it('sends with the draft to the own address', async () => {
    const { p, api } = await loaded(STORED, {
      'POST admin/system-mail/test': [{ body: { identities: IDS, sent: true } }],
    });
    p.set('name', 'Example');
    // The sender name does not change the connection: still connected.
    expect(p.key).toBe('ok');
    await p.send();
    expect(api.calls[2]?.body).toEqual({
      server_id: 's1',
      auth: 'password',
      user: 'noreply@example.com',
      sender_name: 'Example',
      reply_to: null,
      keep_sent: true,
      to: 'alex@example.com',
    });
    expect(ui.toast).toMatchObject({ text: 'Test email sent to alex@example.com', error: false });
  });

  it('invalid address, not connected and the limit', async () => {
    const { p, api } = await loaded(STORED, {
      'POST admin/system-mail/test': [{ status: 429, body: { error: 'rate_limited', retry_after: 300 } }],
    });
    p.to = 'nobody';
    await p.send();
    expect(ui.toast?.text).toBe('Please enter a valid email address');
    p.to = 'help@example.com';
    await p.send();
    expect(ui.toast).toMatchObject({ text: 'Too many requests. Try again in 5 minutes.', error: true });
    expect(p.key).toBe('ok');
    p.set('pass', 'new');
    await p.send();
    expect(api.calls.filter((c) => c.method === 'POST')).toHaveLength(1);
  });

  it('if only the mail fails, the connection stays good; a sign-in failure ends it', async () => {
    const { p } = await loaded(STORED, {
      'POST admin/system-mail/test': [
        { status: 502, body: { error: 'mail_failed', reason: 'not_created' } },
        { status: 502, body: { error: 'mail_failed', reason: 'no_mailbox', role: 'sent' } },
      ],
    });
    await p.send();
    expect(ui.toast).toMatchObject({
      text: 'The test email to alex@example.com wasn’t sent. Please try again.',
      error: true,
    });
    expect(p.key).toBe('ok');
    await p.send();
    expect(p.key).toBe('err');
    expect(ui.toast?.text).toMatch(/“Sent” mailbox/);
  });

  it('a rejected recipient is a toast; a rejected reply-to address is marked at the field', async () => {
    const { p } = await loaded(STORED, {
      'POST admin/system-mail/test': [
        { status: 422, body: { error: 'invalid', fields: { to: { code: 'invalid' } } } },
        { status: 422, body: { error: 'invalid', fields: { reply_to: { code: 'invalid' } } } },
      ],
    });
    await p.send();
    expect(ui.toast).toMatchObject({ text: 'Please enter a valid email address', error: true });
    p.set('replyTo', 'help@example.com');
    await p.send();
    expect(p.errors).toEqual({ replyTo: 'invalid' });
    expect(p.key).toBe('ok');
    expect(scrollTo).toHaveBeenCalledWith('sender');
  });
});

describe('values', () => {
  it('token mode hides the user and names the access token; the eye shows the secret', async () => {
    const { p } = await loaded(STORED);
    let v = values(p);
    expect(v.mailAuthOpts.map((o) => [o.label, o.on])).toEqual([
      ['Password', true],
      ['Token', false],
    ]);
    v.mailAuthOpts[1]?.onClick();
    v = values(p);
    expect(v.mailIsPw).toBe(false);
    expect(v.mailSecret).toMatchObject({ label: 'Access token', ph: '', ac: 'off', field: 'token' });
    expect(v.mailRows[1]?.v).toBe('Via access token');
    expect(v.mSt.key).toBe('new');
    v.mailSecret.onEye();
    expect(values(p).mailSecret).toMatchObject({ type: 'text', eyeTip: 'Hide', eyeIcon: 'ti-eye-off' });
    v.tgMailSent.onClick();
    expect(p.draft.keepSent).toBe(false);
    v.mailDiscard();
    expect(p.dirty).toBe(false);
  });

  it('tabs scroll to their section; Enter in the recipient sends', async () => {
    const { p } = await loaded(STORED, { 'POST admin/system-mail/test': [{ body: { identities: IDS, sent: true } }] });
    const v = values(p);
    expect(v.mailTabs.map((t) => t.label)).toEqual(['Connection', 'Sender', 'Delivery', 'Test']);
    v.mailTabs[3]?.onClick();
    expect(scrollTo).toHaveBeenCalledWith('test');
    const prevent = vi.fn();
    v.mailToKey({ key: 'Enter', preventDefault: prevent });
    expect(prevent).toHaveBeenCalled();
    await vi.waitFor(() => expect(ui.toast?.text).toBe('Test email sent to alex@example.com'));
  });
});
