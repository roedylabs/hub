import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { mockApi, type Call } from '#lib/state/fetch-mock.ts';
import { session } from '#lib/state/session.svelte.ts';
import { ui } from '@roedy/ui';
import { resetSignedIn, signIn } from '#lib/test/signed-in.ts';
import { ProfilePage, profileValues, type ProfileMe, type ProfileValues } from './profile.svelte.ts';

const nav = vi.hoisted(() => ({ goto: vi.fn(() => Promise.resolve()) }));
vi.mock('$app/navigation', () => ({ goto: nav.goto }));

const ME: ProfileMe = {
  id: '01JUSER',
  username: 'alex',
  email: 'alex@example.com',
  display_name: 'Alex Example',
  role: 'user',
  mfa_enabled: false,
  password_changed_at: '2026-07-24T10:00:00Z',
  email_change: null,
  sole_admin: false,
};
const USER = { id: ME.id, username: 'alex', email: 'alex@example.com', display_name: 'Alex Example', role: 'user' };
const OPEN = { email: 'new@example.com', expires_at: '2026-10-27T12:00:00Z' };

type Routes = Parameters<typeof mockApi>[0];
type Reply = { status?: number; body?: unknown };

beforeEach(() => {
  signIn('user');
  nav.goto.mockClear();
});

afterEach(resetSignedIn);

/** A loaded page: `GET auth/me` answers `me` first, then the rest of `routes`. */
async function loaded(routes: Routes = {}, me: ProfileMe = ME) {
  const { 'GET auth/me': reads = [], ...rest } = routes;
  const api = mockApi({ 'GET auth/me': [{ body: me }, ...reads], ...rest });
  const p = new ProfilePage();
  await p.load();
  return { p, ...api };
}

/** A reply that waits until `release` is called. */
function held() {
  let release: (reply: Reply) => void = () => {};
  const reply = () => new Promise<Reply>((resolve) => (release = resolve));
  return { reply, release: (r: Reply) => release(r) };
}

const key = (k: string) => ({ key: k, preventDefault: vi.fn() });
const rowOf = (v: ProfileValues, k: string) => {
  const found = v.acctRows.find((r) => r.k === k);
  if (!found) throw new Error(`no row ${k}`);
  return found;
};
/** The page's requests, without the session reload of the header (`GET session`). */
const sent = (calls: Call[]) =>
  calls.filter((c) => c.path !== 'session').map((c) => ({ call: `${c.method} ${c.path}`, body: c.body }));
const paths = (calls: Call[]) => sent(calls).map((c) => c.call);

describe('loading', () => {
  it('loads `auth/me` and shows account, rows and initials', async () => {
    const { p, calls } = await loaded();
    expect(paths(calls)).toEqual(['GET auth/me']);
    expect(p.status).toBe('ready');
    const v = profileValues(p);
    expect(v.acct).toEqual({ name: 'Alex Example', initials: 'AE', sub: 'alex' });
    expect(v.acctRows.map((r) => [r.k, r.v, r.canEdit, r.noEdit])).toEqual([
      ['Username', 'alex', true, false],
      ['Email', 'alex@example.com', true, false],
      ['Sign-in', 'Password', false, true],
    ]);
    expect(rowOf(v, 'Username').editLabel).toBe('Edit Username');
    expect(v.acctNameRead).toBe(true);
    expect(v.nameDirty).toBe(false);
    expect(v.del).toMatchObject({ email: 'alex@example.com', soleAdm: false, dis: false, label: 'Delete account' });
  });

  it('shows the failure card and loads again with "Try again"', async () => {
    mockApi({
      'GET auth/me': [
        { status: 429, body: { error: 'rate_limited', retry_after: 120 } },
        { status: 500, body: {} },
      ],
    });
    const p = new ProfilePage();
    await p.load();
    expect(p.status).toBe('failed');
    expect(p.loadError).toBe('Too many requests. Try again in 2 minutes.');
    await p.load();
    expect(p.loadError).toBe('That didn’t work. Please try again.');
    // No answer at all.
    await p.load();
    expect(p.loadError).toBe('No connection. Please try again.');
  });

  it('applies only the latest of two overlapping loads', async () => {
    const first = held();
    const second = held();
    mockApi({ 'GET auth/me': [first.reply, second.reply] });
    const p = new ProfilePage();
    const a = p.load();
    const b = p.load();
    await vi.waitFor(() => expect(vi.mocked(fetch)).toHaveBeenCalledTimes(2));
    second.release({ body: { ...ME, display_name: 'Newer Name' } });
    await b;
    first.release({ body: { ...ME, display_name: 'Older Name' } });
    await a;
    expect(p.me?.display_name).toBe('Newer Name');
    expect(p.status).toBe('ready');
  });

  it('a load that started before a saved change does not overwrite it', async () => {
    const late = held();
    const { p } = await loaded({
      'GET auth/me': [late.reply],
      'PATCH auth/me': [{ body: { status: 'ok', user: { ...USER, display_name: 'Alex Rivera' } } }],
    });
    const reloading = p.load();
    p.editName();
    p.setName('Alex Rivera');
    await p.save();
    late.release({ body: ME });
    await reloading;
    expect(p.me?.display_name).toBe('Alex Rivera');
  });

  it('writes nothing after the page was left', async () => {
    const late = held();
    mockApi({ 'GET auth/me': [late.reply] });
    const p = new ProfilePage();
    const loading = p.load();
    p.dispose();
    late.release({ body: ME });
    await loading;
    expect(p.me).toBeNull();
    expect(p.status).toBe('loading');
  });

  it('follows another account signed in by another tab', async () => {
    const other = { ...ME, id: '01JOTHER', display_name: 'Jordan Hale', email: 'jordan@example.com' };
    const { p, calls } = await loaded({ 'GET auth/me': [{ body: other }] });
    p.edit('login');
    p.follow(ME.id);
    p.follow(undefined);
    expect(paths(calls)).toEqual(['GET auth/me']);
    p.follow('01JOTHER');
    expect(p.drafts).toEqual({});
    await vi.waitFor(() => expect(p.me?.display_name).toBe('Jordan Hale'));
    expect(paths(calls)).toEqual(['GET auth/me', 'GET auth/me']);
  });
});

describe('values', () => {
  it('edits display name and rows; empty or unchanged goes back to reading', async () => {
    const { p } = await loaded();
    profileValues(p).acctNameEdit();
    let v = profileValues(p);
    expect(v.acctNameEditing).toBe(true);
    expect(v.acctNameVal).toBe('Alex Example');
    expect(v.nameDirty).toBe(false);
    v.onNameDraft('Alex Rivera');
    expect(profileValues(p).nameDirty).toBe(true);
    v.onNameDraft('  ');
    profileValues(p).onNameBlur();
    expect(profileValues(p).acctNameRead).toBe(true);

    rowOf(profileValues(p), 'Username').edit();
    v = profileValues(p);
    expect(rowOf(v, 'Username')).toMatchObject({ editing: true, read: false, val: 'alex' });
    rowOf(v, 'Username').onChange('alex2');
    expect(profileValues(p).nameDirty).toBe(true);
    rowOf(profileValues(p), 'Username').onKey(key('Escape'));
    expect(rowOf(profileValues(p), 'Username').editing).toBe(false);
    expect(profileValues(p).nameDirty).toBe(false);
  });

  it('checks before saving', async () => {
    const { p, calls } = await loaded();
    p.edit('login');
    p.setField('login', 'Alex!');
    p.edit('email');
    p.setField('email', 'no-at');
    await p.save();
    const v = profileValues(p);
    expect(rowOf(v, 'Username')).toMatchObject({
      hasErr: true,
      err: 'Only lowercase letters, digits, periods, hyphens, and underscores, 2 to 32 characters.',
    });
    expect(rowOf(v, 'Email').err).toBe('Please enter a valid email address.');
    // A new input clears the message.
    rowOf(v, 'Username').onChange('alex2');
    expect(rowOf(profileValues(p), 'Username').hasErr).toBe(false);

    p.discardDrafts();
    p.editName();
    p.setName('   ');
    await p.save();
    expect(ui.toast?.text).toBe('Please enter a name');
    p.setName('Alex​Example');
    await p.save();
    expect(ui.toast).toMatchObject({ text: 'The name contains invalid characters.', error: true });
    expect(paths(calls)).toEqual(['GET auth/me']);
  });

  it('counts only changes that saving also sends; Escape counts as done', async () => {
    const { p } = await loaded();
    p.editName();
    p.setName('Alex  Example');
    expect(profileValues(p).nameDirty).toBe(false);
    p.blurName();
    expect(p.nameDraft).toBeNull();
    for (const handler of [
      (e: ReturnType<typeof key>) => p.nameKey(e),
      (e: ReturnType<typeof key>) => p.fieldKey('login', e),
      (e: ReturnType<typeof key>) => p.eaKey(e),
    ]) {
      const e = key('Escape');
      handler(e);
      expect(e.preventDefault).toHaveBeenCalled();
    }
  });
});

describe('saving', () => {
  it('saves name and username, shows a toast and reloads the session’s account', async () => {
    const info = {
      user: { ...session.user, name: 'Alex Rivera', username: 'ar' },
      session: session.current,
      hub: { name: 'Example Org', home: '/', account: '/account', admin: null },
    };
    const { p, calls } = await loaded({
      'PATCH auth/me': [{ body: { status: 'ok', user: { ...USER, display_name: 'Alex Rivera', username: 'ar' } } }],
      'GET session': [{ body: info }],
    });
    p.editName();
    p.setName(' Alex   Rivera ');
    p.edit('login');
    p.setField('login', 'ar');
    await p.save();
    expect(sent(calls).at(-1)).toEqual({
      call: 'PATCH auth/me',
      body: { display_name: 'Alex Rivera', username: 'ar' },
    });
    expect(p.me).toMatchObject({ display_name: 'Alex Rivera', username: 'ar', sole_admin: false });
    expect(ui.toast).toMatchObject({ text: 'Changes saved', error: false });
    expect(profileValues(p).nameDirty).toBe(false);
    await vi.waitFor(() => expect(session.user?.name).toBe('Alex Rivera'));
    expect(calls.map((c) => c.path)).toContain('session');
  });

  it('asks for the password after a new address (`reauth_required`)', async () => {
    const { p, calls } = await loaded({ 'PATCH auth/me': [{ body: { status: 'reauth_required', user: USER } }] });
    p.edit('email');
    p.setField('email', 'new@example.com');
    await p.save();
    expect(sent(calls).at(-1)).toEqual({ call: 'PATCH auth/me', body: { email: 'new@example.com' } });
    // Only the address changed: no toast.
    expect(ui.toast).toBeNull();
    const v = profileValues(p);
    expect(rowOf(v, 'Email')).toMatchObject({ auth: true, v: 'new@example.com', canEdit: false, pend: false });
    expect(v.eaIntro).toBe('Confirm the change with your password.');
    expect(v.eaFields.map((f) => [f.label, f.desc, f.ac, f.type, f.focus, f.hasEye])).toEqual([
      ['Current password', 'To confirm it’s you', 'current-password', 'password', true, true],
    ]);
    expect(v.eaDis).toBe(true);
    v.eaFields[0]?.onEye?.();
    expect(profileValues(p).eaFields[0]).toMatchObject({
      type: 'text',
      eyeIcon: 'ti-eye-off',
      eyeTip: 'Hide password',
    });
  });

  it('shows a taken address in the row', async () => {
    const { p } = await loaded({ 'PATCH auth/me': [{ status: 422, body: { error: 'email_taken' } }] });
    p.edit('email');
    p.setField('email', 'jordan@example.com');
    await p.save();
    expect(rowOf(profileValues(p), 'Email')).toMatchObject({
      editing: true,
      val: 'jordan@example.com',
      err: 'This email address is already used by another account.',
    });
  });

  it('reports server field errors in the row and unknown ones as an error toast', async () => {
    const { p } = await loaded({
      'PATCH auth/me': [
        { status: 422, body: { error: 'invalid', fields: { username: { code: 'invalid' } } } },
        { status: 422, body: { error: 'invalid', fields: { username: { code: 'taken' } } } },
        { status: 422, body: { error: 'invalid', fields: { display_name: { code: 'too_long' } } } },
        { status: 500, body: {} },
      ],
    });
    p.edit('login');
    p.setField('login', 'jordan');
    await p.save();
    expect(rowOf(profileValues(p), 'Username').err).toBe(
      'Only lowercase letters, digits, periods, hyphens, and underscores, 2 to 32 characters.',
    );
    p.setField('login', 'jordan');
    await p.save();
    expect(ui.toast).toBeNull();
    expect(rowOf(profileValues(p), 'Username')).toMatchObject({
      editing: true,
      val: 'jordan',
      err: 'This username is already taken.',
    });
    p.discardDrafts();
    p.editName();
    p.setName('Alex Rivera');
    await p.save();
    expect(ui.toast).toMatchObject({ text: 'The name is too long.', error: true });
    await p.save();
    expect(ui.toast).toMatchObject({ text: 'That didn’t work. Please try again.', error: true });
  });

  it('reloads the session without a toast after 401', async () => {
    const { p, calls } = await loaded({
      'PATCH auth/me': [{ status: 401, body: { error: 'unauthorized' } }],
      'GET session': [{ status: 401, body: { error: 'unauthorized' } }],
    });
    p.editName();
    p.setName('Alex Rivera');
    await p.save();
    await vi.waitFor(() => expect(session.user).toBeNull());
    expect(ui.toast).toBeNull();
    expect(calls.map((c) => c.path)).toEqual(['auth/me', 'auth/me', 'session']);
  });

  it('leads to a fresh sign-in when the server asks for one', async () => {
    const { p } = await loaded({ 'PATCH auth/me': [{ status: 403, body: { error: 'reauth_required' } }] });
    p.editName();
    p.setName('Alex Rivera');
    await p.save();
    expect(nav.goto).toHaveBeenCalledWith('/login?reauth=1&next=%2Faccount%2Fprofile');
    expect(ui.toast).toBeNull();
  });

  it('an answer after the page was left neither redirects nor shows a toast', async () => {
    const late = held();
    const { p } = await loaded({ 'PATCH auth/me': [late.reply] });
    p.editName();
    p.setName('Alex Rivera');
    const saving = p.save();
    p.dispose();
    late.release({ status: 403, body: { error: 'reauth_required' } });
    await saving;
    expect(nav.goto).not.toHaveBeenCalled();
    expect(ui.toast).toBeNull();
  });

  it('a name saved after the page was left only reloads the session’s account', async () => {
    const late = held();
    const { p, calls } = await loaded({ 'PATCH auth/me': [late.reply] });
    p.editName();
    p.setName('Alex Rivera');
    const saving = p.save();
    p.dispose();
    late.release({ body: { status: 'ok', user: { ...USER, display_name: 'Alex Rivera' } } });
    await saving;
    expect(ui.toast).toBeNull();
    expect(p.me?.display_name).toBe('Alex Example');
    expect(calls.map((c) => c.path)).toContain('session');
  });

  it('an answer for the previous account leaves the new account’s edits, prompt and messages alone', async () => {
    const other = { ...ME, id: '01JOTHER', username: 'jordan', email: 'jordan@example.com' };
    const ok = held();
    const fail = held();
    const { p, calls } = await loaded({
      'GET auth/me': [{ body: other }, { body: ME }],
      'PATCH auth/me': [ok.reply, fail.reply],
    });
    // Account A asks for a new address; another tab signs in B, who starts editing.
    p.edit('email');
    p.setField('email', 'new@example.com');
    const first = p.save();
    p.follow(other.id);
    await vi.waitFor(() => expect(p.me?.id).toBe(other.id));
    p.edit('login');
    p.setField('login', 'jordan2');
    ok.release({ body: { status: 'reauth_required', user: USER } });
    await first;
    expect([p.ea, p.drafts, p.saving]).toEqual([null, { login: 'jordan2' }, false]);

    // B saves, and A signs in again meanwhile: the failure (a fresh sign-in asked for) concerns B and neither marks
    // A's row nor redirects, although A is shown again.
    p.edit('email');
    p.setField('email', 'taken@example.com');
    const second = p.save();
    p.follow(ME.id);
    await vi.waitFor(() => expect(p.me?.id).toBe(ME.id));
    p.edit('email');
    p.setField('email', 'taken@example.com');
    fail.release({ status: 403, body: { error: 'reauth_required' } });
    await second;
    expect([p.errors, p.drafts, p.saving]).toEqual([{}, { email: 'taken@example.com' }, false]);
    expect(nav.goto).not.toHaveBeenCalled();
    expect(ui.toast).toBeNull();
    expect(paths(calls).filter((c) => c === 'PATCH auth/me')).toHaveLength(2);
  });

  it('an answer from before a switch to another account and back still counts as the previous account’s', async () => {
    const other = { ...ME, id: '01JOTHER', username: 'jordan', email: 'jordan@example.com' };
    const late = held();
    const { p } = await loaded({
      'GET auth/me': [{ body: other }, { body: ME }],
      'PATCH auth/me': [late.reply],
    });
    // A asks for a new address; while the request runs, another tab signs in B and then A again.
    p.edit('email');
    p.setField('email', 'new@example.com');
    const saving = p.save();
    p.follow(other.id);
    await vi.waitFor(() => expect(p.me?.id).toBe(other.id));
    p.follow(ME.id);
    await vi.waitFor(() => expect(p.me?.id).toBe(ME.id));
    p.edit('login');
    p.setField('login', 'alex2');
    late.release({ body: { status: 'reauth_required', user: USER } });
    await saving;
    // The page was set up anew for A: the old answer opens no prompt and leaves the new edits alone.
    expect([p.ea, p.drafts, p.saving]).toEqual([null, { login: 'alex2' }, false]);
    expect(nav.goto).not.toHaveBeenCalled();
  });
});

describe('send confirmation link', () => {
  async function asking(routes: Routes = {}, mfa = false) {
    const { 'PATCH auth/me': patches = [], ...rest } = routes;
    const api = await loaded(
      { 'PATCH auth/me': [{ body: { status: 'reauth_required', user: USER } }, ...patches], ...rest },
      { ...ME, mfa_enabled: mfa },
    );
    api.p.edit('email');
    api.p.setField('email', 'new@example.com');
    await api.p.save();
    return api;
  }

  it('sends with password and code and shows "Confirmation pending"', async () => {
    const { p, calls } = await asking(
      { 'PATCH auth/me': [{ body: { status: 'email_sent', user: USER, email_change: OPEN } }] },
      true,
    );
    let v = profileValues(p);
    expect(v.eaIntro).toBe('Confirm the change with your password and a code from your authenticator app.');
    expect(v.eaFields.map((f) => [f.label, f.desc, f.ac, f.im])).toEqual([
      ['Current password', 'To confirm it’s you', 'current-password', 'text'],
      ['Verification code', 'From your authenticator app', 'one-time-code', 'numeric'],
    ]);
    v.eaFields[0]?.onChange('Secret-123');
    expect(profileValues(p).eaDis).toBe(true);
    v.eaFields[1]?.onChange('12 3a45');
    expect(p.eaOtp).toBe('12345');
    v = profileValues(p);
    expect(v.eaDis).toBe(false);
    v.eaGo();
    expect(p.eaErr).toBe('The code must be 6 digits.');
    v.eaFields[1]?.onChange('123456');
    profileValues(p).eaKey(key('Enter'));
    await vi.waitFor(() => expect(ui.toast?.text).toBe('Confirmation link sent'));
    expect(sent(calls).at(-1)).toEqual({
      call: 'PATCH auth/me',
      body: { email: 'new@example.com', password: 'Secret-123', code: '123456' },
    });
    expect(rowOf(profileValues(p), 'Email')).toMatchObject({
      auth: false,
      pend: true,
      pendEmail: 'new@example.com',
      v: 'alex@example.com',
    });
    expect(p.me?.email_change).toEqual(OPEN);
  });

  it('reports password, code, lockout and sending failures', async () => {
    const { p } = await asking(
      {
        'PATCH auth/me': [
          { status: 422, body: { error: 'invalid_password' } },
          { status: 422, body: { error: 'invalid_code' } },
          { status: 422, body: { error: 'code_used' } },
          { status: 429, body: { error: 'rate_limited', retry_after: 900 } },
          { status: 502, body: { error: 'mail_failed' } },
        ],
      },
      true,
    );
    const fill = () => {
      p.setEaPw(p.eaPw || 'Secret-123');
      p.setEaOtp('123456');
    };
    fill();
    await p.sendLink();
    expect([p.eaErr, p.eaPw, p.eaOtp]).toEqual(['The password is incorrect.', '', '123456']);
    fill();
    await p.sendLink();
    expect([p.eaErr, p.eaPw, p.eaOtp]).toEqual([
      'That code isn’t right. Enter the current code from your authenticator app.',
      'Secret-123',
      '',
    ]);
    fill();
    await p.sendLink();
    expect(p.eaErr).toBe('This code has already been used. Wait for the next code in your app.');
    fill();
    await p.sendLink();
    expect([p.eaErr, p.eaPw, p.eaOtp]).toEqual([
      'Too many failed attempts. Try again in 15 minutes.',
      'Secret-123',
      '123456',
    ]);
    await p.sendLink();
    // Without a toast; the code is used up, the password stays.
    expect([p.eaErr, p.eaPw, p.eaOtp]).toEqual([
      'The email couldn’t be sent. Your email address wasn’t changed. Try again later with a new code.',
      'Secret-123',
      '',
    ]);
    expect(ui.toast).toBeNull();
    expect(profileValues(p).hasEaMsg).toBe(true);
  });

  it('without multi-factor: sending failed without the sentence about the code', async () => {
    const { p } = await asking({ 'PATCH auth/me': [{ status: 502, body: { error: 'mail_failed' } }] });
    p.setEaPw('Secret-123');
    await p.sendLink();
    expect(p.eaErr).toBe('The email couldn’t be sent. Your email address wasn’t changed. Try again later.');
    expect(rowOf(profileValues(p), 'Email').auth).toBe(true);
  });

  it('shows an address taken in the meantime in the row again', async () => {
    const { p } = await asking({ 'PATCH auth/me': [{ status: 422, body: { error: 'email_taken' } }] });
    p.setEaPw('Secret-123');
    await p.sendLink();
    expect(rowOf(profileValues(p), 'Email')).toMatchObject({
      auth: false,
      editing: true,
      val: 'new@example.com',
      err: 'This email address is already used by another account.',
    });
  });

  it('cancel and Escape close the prompt; a late message no longer applies', async () => {
    const late = held();
    const { p, calls } = await asking({ 'PATCH auth/me': [late.reply] });
    p.setEaPw('Secret-123');
    const sending = p.sendLink();
    expect(profileValues(p).eaBusy).toBe(true);
    await vi.waitFor(() => expect(paths(calls)).toHaveLength(3));
    profileValues(p).eaKey(key('Escape'));
    late.release({ status: 422, body: { error: 'invalid_password' } });
    await sending;
    expect(p.ea).toBeNull();
    expect(p.eaErr).toBeNull();
    expect(rowOf(profileValues(p), 'Email').auth).toBe(false);
  });

  it('a link sent for an earlier address keeps the prompt of a newer one', async () => {
    const late = held();
    const { p } = await asking({
      'PATCH auth/me': [late.reply, { body: { status: 'reauth_required', user: USER } }],
    });
    p.setEaPw('Secret-123');
    const sending = p.sendLink();
    p.cancelAuth();
    p.edit('email');
    p.setField('email', 'two@example.com');
    await p.save();
    expect(p.ea).toEqual({ email: 'two@example.com' });
    p.setEaPw('Other-456');
    late.release({ body: { status: 'email_sent', user: USER, email_change: OPEN } });
    await sending;
    expect(p.ea).toEqual({ email: 'two@example.com' });
    expect(p.eaPw).toBe('Other-456');
    expect(p.me?.email_change).toEqual(OPEN);
    expect(ui.toast?.text).toBe('Confirmation link sent');
  });

  it('reports the discarded change after `email_change_discarded` and reloads the account', async () => {
    const { p, calls } = await asking({
      'PATCH auth/me': [{ status: 409, body: { error: 'email_change_discarded' } }],
      'GET auth/me': [{ body: ME }],
    });
    p.setEaPw('Secret-123');
    await p.sendLink();
    expect(ui.toast).toMatchObject({ text: 'The email address change has since been discarded.', error: false });
    expect(p.ea).not.toBeNull();
    await vi.waitFor(() => expect(paths(calls).filter((c) => c === 'GET auth/me')).toHaveLength(2));
  });

  it('reports no link if the address already applies in the meantime', async () => {
    const { p, calls } = await asking({
      'PATCH auth/me': [{ body: { status: 'ok', user: { ...USER, email: 'new@example.com' } } }],
      'GET auth/me': [{ body: { ...ME, email: 'new@example.com' } }],
    });
    p.setEaPw('Secret-123');
    await p.sendLink();
    expect(ui.toast).toBeNull();
    expect(p.ea).toBeNull();
    expect(p.me?.email).toBe('new@example.com');
    await vi.waitFor(() => expect(paths(calls).at(-1)).toBe('GET auth/me'));
  });

  it('loads the account if the server requires a code the page did not ask for', async () => {
    const { p } = await asking({
      'PATCH auth/me': [{ status: 422, body: { error: 'invalid', fields: { code: { code: 'required' } } } }],
      'GET auth/me': [{ body: { ...ME, mfa_enabled: true } }],
    });
    p.setEaPw('Secret-123');
    await p.sendLink();
    expect(p.eaErr).toBe('The code must be 6 digits.');
    await vi.waitFor(() => expect(profileValues(p).eaFields).toHaveLength(2));
  });
});

describe('confirmation pending', () => {
  const PENDING = { ...ME, email_change: OPEN };

  it('shows the pending change from `auth/me` and resends the link', async () => {
    const renewed = { email: OPEN.email, expires_at: '2026-10-28T12:00:00Z' };
    const { p, calls } = await loaded({ 'POST auth/me/email/resend': [{ body: renewed }] }, PENDING);
    const row = rowOf(profileValues(p), 'Email');
    expect(row).toMatchObject({ pend: true, pendEmail: 'new@example.com', hasPendErr: false });
    row.resend();
    await vi.waitFor(() => expect(ui.toast?.text).toBe('Confirmation link resent'));
    expect(paths(calls)).toEqual(['GET auth/me', 'POST auth/me/email/resend']);
    expect(p.me?.email_change).toEqual(renewed);
  });

  it('reports a failed sending in the box, with the previous link only if it applies', async () => {
    const { p } = await loaded(
      {
        'POST auth/me/email/resend': [
          { status: 502, body: { error: 'mail_failed', previous_link_valid: true } },
          { status: 502, body: { error: 'mail_failed', previous_link_valid: false } },
          { status: 422, body: { error: 'email_taken' } },
          { status: 429, body: { error: 'rate_limited', retry_after: 600 } },
        ],
      },
      PENDING,
    );
    await p.resend();
    expect(rowOf(profileValues(p), 'Email')).toMatchObject({
      hasPendErr: true,
      pendErr: 'The email couldn’t be sent. The previous link is still valid. Try again later.',
    });
    await p.resend();
    expect(p.pendErr).toBe('The email couldn’t be sent. Try again later.');
    await p.resend();
    expect(p.pendErr).toBe('This email address is already used by another account.');
    await p.resend();
    expect(p.pendErr).toBe('Too many requests. Try again in 10 minutes.');
    expect(ui.toast).toBeNull();
    expect(p.me?.email_change).toEqual(OPEN);
  });

  it('"Resend link" after the change was discarded elsewhere: box gone, regular toast', async () => {
    const { p, calls } = await loaded(
      { 'POST auth/me/email/resend': [{ status: 404, body: { error: 'not_found' } }], 'GET auth/me': [{ body: ME }] },
      PENDING,
    );
    await p.resend();
    expect(paths(calls)).toEqual(['GET auth/me', 'POST auth/me/email/resend', 'GET auth/me']);
    expect(rowOf(profileValues(p), 'Email')).toMatchObject({ pend: false, hasPendErr: false });
    expect(ui.toast).toMatchObject({ text: 'The email address change has since been discarded.', error: false });
  });

  it('"Resend link" after the change was confirmed elsewhere: new address, no toast', async () => {
    const { p } = await loaded(
      {
        'POST auth/me/email/resend': [{ status: 404, body: { error: 'not_found' } }],
        'GET auth/me': [{ body: { ...ME, email: OPEN.email } }],
      },
      PENDING,
    );
    await p.resend();
    expect(rowOf(profileValues(p), 'Email')).toMatchObject({ pend: false, v: OPEN.email });
    expect(ui.toast).toBeNull();
  });

  it('"Resend link" without an answer to `auth/me` still drops the box', async () => {
    const { p } = await loaded(
      { 'POST auth/me/email/resend': [{ status: 404, body: { error: 'not_found' } }] },
      PENDING,
    );
    await p.resend();
    expect(p.me?.email_change).toBeNull();
    expect(ui.toast?.text).toBe('The email address change has since been discarded.');
  });

  it('drops a late "Resend link" answer after a newer link', async () => {
    const late = held();
    const newer = { ...OPEN, email: 'two@example.com' };
    const { p } = await loaded(
      {
        'POST auth/me/email/resend': [late.reply],
        'PATCH auth/me': [
          { body: { status: 'reauth_required', user: USER } },
          { body: { status: 'email_sent', user: USER, email_change: newer } },
        ],
      },
      PENDING,
    );
    const resending = p.resend();
    // Another page state replaced the change meanwhile (e.g. a load after another tab sent a new link).
    p.me = { ...PENDING, email_change: newer };
    late.release({ status: 502, body: { error: 'mail_failed', previous_link_valid: true } });
    await resending;
    expect(p.me?.email_change).toEqual(newer);
    expect(p.pendErr).toBeNull();
    expect(ui.toast).toBeNull();
  });

  it('a "Resend link" answer after the page was left shows nothing', async () => {
    const late = held();
    const { p } = await loaded({ 'POST auth/me/email/resend': [late.reply] }, PENDING);
    const resending = p.resend();
    p.dispose();
    late.release({ body: { ...OPEN, expires_at: '2026-10-28T12:00:00Z' } });
    await resending;
    expect(ui.toast).toBeNull();
    expect(p.me?.email_change).toEqual(OPEN);
  });

  it('a failed "Resend link" after the page was left shows nothing', async () => {
    const late = held();
    const { p } = await loaded({ 'POST auth/me/email/resend': [late.reply] }, PENDING);
    const resending = p.resend();
    p.dispose();
    late.release({ status: 500, body: {} });
    await resending;
    expect(ui.toast).toBeNull();
    expect(p.pendErr).toBeNull();
  });

  it('"Resend link" without an answer shows the server state and the error toast', async () => {
    const renewed = { ...OPEN, expires_at: '2026-10-28T12:00:00Z' };
    // No response for the POST: `fetch` fails.
    const { p } = await loaded({ 'GET auth/me': [{ body: { ...ME, email_change: renewed } }] }, PENDING);
    await p.resend();
    expect(p.me?.email_change).toEqual(renewed);
    expect(ui.toast).toMatchObject({ text: 'No connection. Please try again.', error: true });
    expect(p.resending).toBe(false);
  });

  it('"Resend link" and a new link wait for each other', async () => {
    const late = held();
    const { p, calls } = await loaded(
      {
        'PATCH auth/me': [{ body: { status: 'reauth_required', user: USER } }, late.reply],
        'POST auth/me/email/resend': [{ body: OPEN }],
      },
      PENDING,
    );
    p.edit('email');
    p.setField('email', 'two@example.com');
    await p.save();
    p.setEaPw('Secret-123');
    const sending = p.sendLink();
    await vi.waitFor(() => expect(paths(calls)).toHaveLength(3));
    // Prompt closed while sending: the box is back, but its buttons wait.
    p.cancelAuth();
    expect(rowOf(profileValues(p), 'Email')).toMatchObject({ pend: true, busy: true });
    await p.resend();
    expect(paths(calls)).toHaveLength(3);
    late.release({ body: { status: 'email_sent', user: USER, email_change: { ...OPEN, email: 'two@example.com' } } });
    await sending;
  });

  it('discards at once after `auth/me` still shows the change', async () => {
    const { p, calls } = await loaded(
      { 'GET auth/me': [{ body: PENDING }], 'DELETE auth/me/email': [{ status: 204 }] },
      PENDING,
    );
    const discarding = rowOf(profileValues(p), 'Email');
    discarding.discard();
    expect(rowOf(profileValues(p), 'Email').busy).toBe(true);
    await vi.waitFor(() => expect(ui.toast?.text).toBe('Change discarded'));
    expect(paths(calls)).toEqual(['GET auth/me', 'GET auth/me', 'DELETE auth/me/email']);
    expect(rowOf(profileValues(p), 'Email')).toMatchObject({ pend: false, busy: false });
  });

  it('"Discard change" counts 404 as done', async () => {
    const { p } = await loaded(
      {
        'GET auth/me': [{ body: PENDING }],
        'DELETE auth/me/email': [{ status: 404, body: { error: 'not_found' } }],
      },
      PENDING,
    );
    await p.discardChange();
    expect(p.me?.email_change).toBeNull();
    expect(ui.toast?.text).toBe('Change discarded');
  });

  it('"Discard change" after the change was discarded elsewhere: regular toast, nothing sent', async () => {
    const { p, calls } = await loaded({ 'GET auth/me': [{ body: ME }] }, PENDING);
    await p.discardChange();
    expect(rowOf(profileValues(p), 'Email').pend).toBe(false);
    expect(ui.toast).toMatchObject({ text: 'The email address change has since been discarded.', error: false });
    expect(paths(calls)).toEqual(['GET auth/me', 'GET auth/me']);
  });

  it('"Discard change" after the change was confirmed elsewhere: no toast', async () => {
    const { p } = await loaded({ 'GET auth/me': [{ body: { ...ME, email: OPEN.email } }] }, PENDING);
    await p.discardChange();
    expect(rowOf(profileValues(p), 'Email')).toMatchObject({ pend: false, v: OPEN.email });
    expect(ui.toast).toBeNull();
  });

  it('"Discard change" does not discard a change that replaced the clicked one', async () => {
    const newer = { ...OPEN, email: 'two@example.com' };
    const { p, calls } = await loaded({ 'GET auth/me': [{ body: { ...ME, email_change: newer } }] }, PENDING);
    await p.discardChange();
    expect(rowOf(profileValues(p), 'Email')).toMatchObject({ pend: true, pendEmail: 'two@example.com' });
    expect(ui.toast?.text).toBe('The email address change has since been discarded.');
    expect(paths(calls)).not.toContain('DELETE auth/me/email');
  });

  it('"Discard change" keeps a link sent while `auth/me` loads', async () => {
    const late = held();
    const newer = { ...OPEN, email: 'two@example.com' };
    const { p, calls } = await loaded({ 'GET auth/me': [late.reply] }, PENDING);
    const discarding = p.discardChange();
    await vi.waitFor(() => expect(paths(calls)).toHaveLength(2));
    p.me = { ...PENDING, email_change: newer };
    // The older read still shows the clicked change.
    late.release({ body: PENDING });
    await discarding;
    expect(p.me?.email_change).toEqual(newer);
    expect(ui.toast).toBeNull();
    expect(paths(calls)).not.toContain('DELETE auth/me/email');
  });

  it('"Discard change" discards nothing while a new link is being sent', async () => {
    const late = held();
    const { p, calls } = await loaded(
      {
        'GET auth/me': [{ body: PENDING }],
        'PATCH auth/me': [{ body: { status: 'reauth_required', user: USER } }, late.reply],
      },
      PENDING,
    );
    p.edit('email');
    p.setField('email', 'two@example.com');
    await p.save();
    p.setEaPw('Secret-123');
    const sending = p.sendLink();
    await vi.waitFor(() => expect(paths(calls)).toHaveLength(3));
    await p.discardChange();
    late.release({ body: { status: 'email_sent', user: USER, email_change: { ...OPEN, email: 'two@example.com' } } });
    await sending;
    expect(ui.toast?.text).toBe('Confirmation link sent');
    expect(paths(calls)).not.toContain('DELETE auth/me/email');
    expect(p.me?.email_change?.email).toBe('two@example.com');
  });

  it('sends no new link while "Discard change" is running', async () => {
    const late = held();
    const { p, calls } = await loaded(
      {
        'GET auth/me': [late.reply],
        'PATCH auth/me': [{ body: { status: 'reauth_required', user: USER } }],
        'DELETE auth/me/email': [{ status: 204 }],
      },
      PENDING,
    );
    p.edit('email');
    p.setField('email', 'two@example.com');
    await p.save();
    p.setEaPw('Secret-123');
    const discarding = p.discardChange();
    await vi.waitFor(() => expect(paths(calls)).toHaveLength(3));
    expect(profileValues(p).eaBusy).toBe(true);
    await p.sendLink();
    expect(paths(calls).filter((c) => c === 'PATCH auth/me')).toHaveLength(1);
    late.release({ body: PENDING });
    await discarding;
    expect(ui.toast?.text).toBe('Change discarded');
    expect(p.ea).toEqual({ email: 'two@example.com' });
    expect(profileValues(p).eaBusy).toBe(false);
  });

  it('keeps a name saved while "Discard change" reads `auth/me`', async () => {
    const late = held();
    const { p, calls } = await loaded(
      {
        'GET auth/me': [late.reply],
        'PATCH auth/me': [{ body: { status: 'ok', user: { ...USER, display_name: 'Alex Rivera' } } }],
        'DELETE auth/me/email': [{ status: 204 }],
      },
      PENDING,
    );
    const discarding = p.discardChange();
    await vi.waitFor(() => expect(paths(calls)).toHaveLength(2));
    p.editName();
    p.setName('Alex Rivera');
    await p.save();
    late.release({ body: PENDING });
    await discarding;
    expect(p.me?.display_name).toBe('Alex Rivera');
    expect(p.me?.email_change).toBeNull();
    expect(ui.toast?.text).toBe('Change discarded');
  });

  it('a discard check of a page that was left changes nothing', async () => {
    const late = held();
    const { p, calls } = await loaded({ 'GET auth/me': [late.reply] }, PENDING);
    const discarding = p.discardChange();
    await vi.waitFor(() => expect(paths(calls)).toHaveLength(2));
    p.dispose();
    late.release({ body: ME });
    await discarding;
    expect(p.me?.email_change).toEqual(OPEN);
    expect(ui.toast).toBeNull();
  });

  it('keeps the change and shows the error toast if `auth/me` cannot be read', async () => {
    const { p, calls } = await loaded({ 'GET auth/me': [{ status: 500, body: {} }] }, PENDING);
    await p.discardChange();
    expect(rowOf(profileValues(p), 'Email').pend).toBe(true);
    expect(ui.toast).toMatchObject({ text: 'That didn’t work. Please try again.', error: true });
    // No answer at all: `fetch` fails.
    await p.discardChange();
    expect(ui.toast).toMatchObject({ text: 'No connection. Please try again.', error: true });
    expect(p.me?.email_change).toEqual(OPEN);
    expect(paths(calls)).toEqual(['GET auth/me', 'GET auth/me', 'GET auth/me']);
  });

  it('keeps the change and reloads the account if the server rejects the discard', async () => {
    const { p, calls } = await loaded(
      {
        'GET auth/me': [{ body: PENDING }, { body: PENDING }],
        'DELETE auth/me/email': [{ status: 500, body: {} }],
      },
      PENDING,
    );
    await p.discardChange();
    expect(ui.toast).toMatchObject({ text: 'That didn’t work. Please try again.', error: true });
    expect(p.me?.email_change).toEqual(OPEN);
    await vi.waitFor(() =>
      expect(paths(calls)).toEqual(['GET auth/me', 'GET auth/me', 'DELETE auth/me/email', 'GET auth/me']),
    );
  });

  it('does not discard while "Resend link" is running', async () => {
    const late = held();
    const { p, calls } = await loaded({ 'POST auth/me/email/resend': [late.reply] }, PENDING);
    const resending = p.resend();
    await vi.waitFor(() => expect(paths(calls)).toHaveLength(2));
    expect(rowOf(profileValues(p), 'Email').busy).toBe(true);
    await p.discardChange();
    expect(paths(calls)).toHaveLength(2);
    late.release({ body: OPEN });
    await resending;
    expect(p.me?.email_change).toEqual(OPEN);
  });
});

describe('delete account', () => {
  it('asks with the confirmation word, then signs out and opens the sign-in', async () => {
    const { p, calls } = await loaded({ 'DELETE auth/me': [{ status: 204 }] });
    profileValues(p).del.onClick();
    expect(ui.confirm).toMatchObject({
      title: 'Permanently delete account?',
      text: 'Your account alex@example.com and all its data will be deleted. This can’t be undone.',
      icon: 'ti-trash',
      ok: 'Delete account',
      danger: true,
      word: 'DELETE',
    });
    ui.confirm?.run();
    expect(profileValues(p).del).toMatchObject({ label: 'Deleting …', dis: true, busy: true });
    await vi.waitFor(() => expect(nav.goto).toHaveBeenCalledWith('/login', { replace: true }));
    // The request names the account, so the server deletes nothing if the session belongs to another one by now.
    expect(sent(calls).slice(1)).toEqual([{ call: 'DELETE auth/me', body: { confirm: 'delete', id: ME.id } }]);
    expect(session.user).toBeNull();
    expect(p.deleting).toBe(false);
  });

  it('closes the confirmation when another tab signs in a different account', async () => {
    const other = { ...ME, id: '01JOTHER', display_name: 'Jordan Hale', email: 'jordan@example.com' };
    const { p, calls } = await loaded({ 'GET auth/me': [{ body: other }] });
    p.deleteAccount();
    const run = ui.confirm?.run;
    p.follow('01JOTHER');
    expect(ui.confirm).toBeNull();
    // A confirmation that was still open for the previous account deletes nothing.
    run?.();
    await vi.waitFor(() => expect(p.me?.id).toBe('01JOTHER'));
    expect(paths(calls)).toEqual(['GET auth/me', 'GET auth/me']);
    expect(p.deleting).toBe(false);
  });

  it('reports `account_mismatch`: the session already belongs to another account', async () => {
    const { p, calls } = await loaded({ 'DELETE auth/me': [{ status: 409, body: { error: 'account_mismatch' } }] });
    p.deleteAccount();
    ui.confirm?.run();
    await vi.waitFor(() =>
      expect(ui.toast).toMatchObject({
        text: 'Account not deleted. Another account has since signed in.',
        error: true,
      }),
    );
    expect(paths(calls)).toEqual(['GET auth/me', 'DELETE auth/me']);
    // The session's account reloads (then the layout follows the other account).
    expect(calls.map((c) => c.path)).toContain('session');
    expect(p.deleting).toBe(false);
    expect(nav.goto).not.toHaveBeenCalled();
  });

  it('closes the confirmation when the page is left', async () => {
    const { p } = await loaded();
    p.deleteAccount();
    expect(ui.confirm).not.toBeNull();
    p.dispose();
    expect(ui.confirm).toBeNull();
  });

  it('is locked for the only administrator', async () => {
    const { p } = await loaded({}, { ...ME, role: 'admin', sole_admin: true });
    const v = profileValues(p);
    expect(v.del).toMatchObject({
      soleAdm: true,
      dis: true,
      tip: 'Only possible once there’s another administrator',
      sub: 'You’re the only administrator. Make someone else an administrator first, then you can delete your account.',
    });
    expect(profileValues(p, true).del.tip).toBeUndefined();
    v.del.onClick();
    expect(ui.confirm).toBeNull();
  });

  it('locks the button after `last_admin`', async () => {
    const { p } = await loaded(
      { 'DELETE auth/me': [{ status: 409, body: { error: 'last_admin' } }] },
      {
        ...ME,
        role: 'admin',
      },
    );
    p.deleteAccount();
    ui.confirm?.run();
    await vi.waitFor(() => expect(ui.toast?.text).toBe('Account not deleted. You’re now the only administrator.'));
    expect(profileValues(p).del).toMatchObject({ soleAdm: true, dis: true, busy: false });
    expect(session.user).not.toBeNull();
    expect(nav.goto).not.toHaveBeenCalled();
  });

  it('leads to a fresh sign-in after `reauth_required`', async () => {
    const { p } = await loaded({ 'DELETE auth/me': [{ status: 403, body: { error: 'reauth_required' } }] });
    p.deleteAccount();
    ui.confirm?.run();
    await vi.waitFor(() => expect(nav.goto).toHaveBeenCalledWith('/login?reauth=1&next=%2Faccount%2Fprofile'));
    expect(session.user).not.toBeNull();
    expect(p.deleting).toBe(false);
  });

  it('keeps the account and shows the error toast if deleting fails', async () => {
    const { p } = await loaded({ 'DELETE auth/me': [{ status: 500, body: {} }] });
    p.deleteAccount();
    ui.confirm?.run();
    await vi.waitFor(() => expect(ui.toast).toMatchObject({ text: 'That didn’t work. Please try again.' }));
    expect(session.user).not.toBeNull();
    expect(profileValues(p).del).toMatchObject({ dis: false, label: 'Delete account' });
  });
});
