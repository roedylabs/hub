import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { m } from '#lib/paraglide/messages.js';
import { mockApi } from '#lib/state/fetch-mock.ts';
import { session } from '#lib/state/session.svelte.ts';
import { ui } from '#lib/state/ui.svelte.ts';
import { resetSignedIn, signIn } from '#lib/test/signed-in.ts';
import {
  SecurityPage,
  securityValues,
  type ListedSession,
  type MfaOverview,
  type SecurityValues,
} from './security.svelte.ts';

const NOW = Date.parse('2026-10-26T12:00:00Z');
const minutesAgo = (n: number) => new Date(NOW - n * 60_000).toISOString();
const ch = (code: number) => String.fromCodePoint(code);

const NO_MFA: MfaOverview = { devices: [], recovery: null };
const IPHONE = { id: 't1', name: 'iPhone', created_at: '2026-10-01T09:00:00Z', last_used_at: minutesAgo(120) };
const LAPTOP = { id: 't2', name: 'Authenticator app 2', created_at: '2026-10-20T09:00:00Z', last_used_at: null };
const MFA: MfaOverview = {
  devices: [IPHONE, LAPTOP],
  recovery: { total: 10, unused: 8, created_at: minutesAgo(3 * 24 * 60) },
};
const CODES = Array.from({ length: 10 }, (_, i) => `ABCD-EF${String(10 + i)}`);
const ME = { password_changed_at: minutesAgo(94 * 24 * 60) };

const session0 = (id: string, extra: Partial<ListedSession> = {}): ListedSession => ({
  id,
  current: false,
  device: 'Firefox on Linux',
  device_type: 'desktop',
  ip: null,
  created_at: minutesAgo(600),
  last_seen_at: minutesAgo(600),
  ...extra,
});
const SESSIONS = [
  session0('s0', { current: true, ip: '198.51.x.x', last_seen_at: minutesAgo(0) }),
  session0('s1', { device: 'Safari on iPhone', device_type: 'mobile', last_seen_at: minutesAgo(120) }),
  session0('s2', { device: 'Chrome on iPad', device_type: 'tablet', ip: '2001:db8:x:x:x:x:x:x' }),
];

const ENROLL = {
  id: 't3',
  secret: 'JBSWY3DPEHPK3PXPJBSWY3DPEHPK3PXP',
  uri: 'otpauth://totp/Example%20Org:alex%40example.com?secret=JBSWY3DPEHPK3PXP',
};

const ctx = { scrollTo: vi.fn() };
type Routes = Parameters<typeof mockApi>[0];
type Reply = Routes[string][number];

beforeEach(() => {
  vi.useFakeTimers({ toFake: ['Date'] });
  vi.setSystemTime(NOW);
  signIn('user');
});

afterEach(() => {
  vi.useRealTimers();
  ctx.scrollTo.mockClear();
  resetSignedIn();
});

/** An answer that waits until `release` is called. */
function later() {
  let release: (reply: { status?: number; body?: unknown }) => void = () => {};
  const reply: Reply = () => new Promise((resolve) => (release = resolve));
  return { reply, release: (r: { status?: number; body?: unknown }) => release(r) };
}

function routes(extra: Routes = {}, mfa = MFA, sessions = SESSIONS): Routes {
  return {
    'GET auth/totp/devices': [{ body: mfa }, { body: mfa }, { body: mfa }],
    'GET auth/sessions': [{ body: { sessions } }, { body: { sessions } }, { body: { sessions } }],
    'GET auth/me': [{ body: ME }, { body: ME }, { body: ME }],
    ...extra,
  };
}

async function loaded(extra: Routes = {}, mfa = MFA, sessions = SESSIONS, hooks = {}) {
  const api = mockApi(routes(extra, mfa, sessions));
  const p = new SecurityPage(hooks);
  await p.load();
  return { p, v: () => securityValues(p, ctx, NOW, 'UTC'), ...api };
}

const key = (k: string) => ({ key: k, preventDefault: vi.fn() });
const sent = (calls: { method: string; path: string; body: unknown }[]) =>
  calls.map((c) => ({ call: `${c.method} ${c.path}`, body: c.body }));
const count = (calls: { path: string }[], path: string) => calls.filter((c) => c.path === path).length;
const field = (v: SecurityValues, ac: string, n = 0) => {
  const found = v.pwFields.filter((f) => f.ac === ac)[n];
  if (!found) throw new Error(`no field ${ac}`);
  return found;
};

describe('Loading', () => {
  it('shows password, apps, codes and devices', async () => {
    const { v, calls } = await loaded();
    expect(sent(calls).map((c) => c.call)).toEqual(['GET auth/totp/devices', 'GET auth/sessions', 'GET auth/me']);
    const values = v();
    expect(values.secTabs.map((t) => t.label)).toEqual(['Password', 'Multi-factor', 'Devices']);
    values.secTabs[1]?.onClick();
    expect(ctx.scrollTo).toHaveBeenCalledWith('2fa');
    expect(values.pwDesc).toBe('Last changed 94 days ago');
    expect(values.pwFields.map((f) => [f.label, f.desc, f.ac, f.sep, f.hasEye])).toEqual([
      ['Current password', 'To confirm it’s you', 'current-password', false, true],
      ['Verification code', 'From your authenticator app', 'one-time-code', true, false],
      [
        'New password',
        'At least 10 characters with upper- and lowercase letters and a number or special character',
        'new-password',
        true,
        true,
      ],
      ['Confirm new password', 'Enter it again to make sure', 'new-password', true, false],
    ]);
    expect(values.tfaEntries.map((e) => [e.name, e.sub, e.sep, e.delLabel])).toEqual([
      ['iPhone', 'Added on 1 Oct 2026 · Last used 2 hours ago', false, 'Remove iPhone'],
      ['Authenticator app 2', 'Added on 20 Oct 2026', true, 'Remove Authenticator app 2'],
    ]);
    expect([values.tfaEmpty, values.totpPanel, values.totpNoPanel]).toEqual([false, false, true]);
    expect([values.rcOn, values.rcOff, values.rcShow, values.rcForm, values.rcNoForm]).toEqual([
      true,
      false,
      false,
      false,
      true,
    ]);
    expect(values.rcSum).toBe('8 of 10 unused');
    expect(values.rcSub).toBe('Generated 3 days ago');
    expect(values.sessList.map((s) => [s.icon, s.name, s.sub, s.current, s.notCurrent])).toEqual([
      ['ti-device-desktop', 'Firefox on Linux', 'IP 198.51.x.x · Active now', true, false],
      ['ti-device-mobile', 'Safari on iPhone', 'Last active 2 hours ago', false, true],
      ['ti-device-tablet', 'Chrome on iPad', 'IP 2001:db8:x:x:x:x:x:x · Last active 10 hours ago', false, true],
    ]);
    expect(values.hasOtherSess).toBe(true);
  });

  it('without an app: no code field, no codes; only its own device', async () => {
    const { v } = await loaded({}, NO_MFA, [SESSIONS[0] as ListedSession]);
    const values = v();
    expect(values.pwFields.map((f) => f.ac)).toEqual(['current-password', 'new-password', 'new-password']);
    expect([values.tfaEmpty, values.rcOff, values.rcOn, values.hasOtherSess]).toEqual([true, true, false, false]);
  });

  it('shows the load card if the first load fails, and loads again', async () => {
    const { p } = await loaded({ 'GET auth/totp/devices': [{ status: 500, body: {} }, { body: MFA }] });
    expect([p.status, p.failure]).toEqual(['failed', m.error_act()]);
    p.retry();
    expect(p.status).toBe('loading');
    await vi.waitFor(() => expect(p.status).toBe('ready'));
  });

  it('offline without a response from the server', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn(() => Promise.reject(new TypeError('offline'))),
    );
    const p = new SecurityPage();
    await p.load();
    expect([p.status, p.failure]).toEqual(['failed', m.error_offline()]);
  });

  it('names the wait after too many requests on the first load', async () => {
    const { p } = await loaded({
      'GET auth/sessions': [{ status: 429, body: { error: 'rate_limited', retry_after: 30 } }],
    });
    expect([p.status, p.failure]).toEqual(['failed', 'Too many requests. Try again in 30 seconds.']);
  });

  it('reports a failed reload with an error toast and keeps the list', async () => {
    const { p, v } = await loaded({ 'GET auth/totp/devices': [{ body: MFA }, { status: 500, body: {} }] });
    await p.load();
    expect(p.status).toBe('ready');
    expect(ui.toast).toMatchObject({ text: 'Couldn’t update the list.', error: true });
    expect(v().sessList.map((s) => s.id)).toEqual(['s0', 's1', 's2']);
  });

  it('an ended session reloads the account instead of a toast', async () => {
    const { p, calls } = await loaded({
      'GET auth/totp/devices': [{ body: MFA }, { status: 401, body: { error: 'unauthorized' } }],
      'GET session': [{ status: 401, body: { error: 'unauthorized' } }],
    });
    await p.load();
    expect(ui.toast).toBeNull();
    await vi.waitFor(() => expect(session.user).toBeNull());
    expect(count(calls, 'session')).toBe(1);
  });

  it('only the latest of two overlapping loads applies', async () => {
    const first = later();
    const api = mockApi(
      routes({
        'GET auth/totp/devices': [first.reply, { body: NO_MFA }],
      }),
    );
    const p = new SecurityPage();
    const older = p.load();
    const newer = p.load();
    await newer;
    expect(p.devices).toEqual([]);
    first.release({ body: MFA });
    await older;
    expect([p.status, p.devices]).toEqual(['ready', []]);
    expect(count(api.calls, 'auth/totp/devices')).toBe(2);
  });

  it('a load that started before a removed app does not bring it back', async () => {
    const after: MfaOverview = { devices: [LAPTOP], recovery: MFA.recovery };
    const { p } = await loaded();
    const stale = later();
    mockApi(
      routes({
        'GET auth/totp/devices': [stale.reply, { body: after }],
        'DELETE auth/totp/devices/t1': [{ body: after }],
      }),
    );
    const load = p.load();
    p.askRemove('t1');
    p.setTPw('Secret-123');
    p.setTCode('123456');
    await p.confirmTotp();
    expect(p.devices.map((d) => d.id)).toEqual(['t2']);
    stale.release({ body: MFA });
    await load;
    await vi.waitFor(() => expect(p.devices.map((d) => d.id)).toEqual(['t2']));
    expect(ui.toast?.text).toBe('iPhone removed');
  });

  it('a load that started before "Sign out" does not bring the device back', async () => {
    const { p, v } = await loaded();
    const stale = later();
    const api = mockApi(
      routes({
        'GET auth/sessions': [stale.reply, { body: { sessions: SESSIONS.filter((s) => s.id !== 's1') } }],
        'DELETE auth/sessions/s1': [{ status: 204 }],
      }),
    );
    const load = p.load();
    await p.revoke('s1');
    stale.release({ body: { sessions: SESSIONS } });
    await load;
    await vi.waitFor(() => expect(count(api.calls, 'auth/sessions')).toBe(2));
    await vi.waitFor(() => expect(v().sessList.map((s) => s.id)).toEqual(['s0', 's2']));
  });

  it('writes nothing once the page is left', async () => {
    const stale = later();
    mockApi(routes({ 'GET auth/totp/devices': [stale.reply] }));
    const p = new SecurityPage();
    const load = p.load();
    p.dispose();
    stale.release({ body: MFA });
    await load;
    expect([p.status, p.devices, p.sessions]).toEqual(['loading', [], []]);
  });
});

describe('Change password', () => {
  async function filled(extra: Routes, mfa = MFA, hooks = {}) {
    const page = await loaded(extra, mfa, SESSIONS, hooks);
    const v = page.v();
    field(v, 'current-password').onChange('Old-Password1');
    if (mfa.devices.length) field(v, 'one-time-code').onChange('123456');
    field(v, 'new-password').onChange('New-Password1');
    field(v, 'new-password', 1).onChange('New-Password1');
    return page;
  }

  it('checks rules, repetition and code before sending', async () => {
    const { p, v, calls } = await loaded();
    expect(v().pwDis).toBe(true);
    field(v(), 'current-password').onChange('Old-Password1');
    field(v(), 'one-time-code').onChange('12345');
    field(v(), 'new-password').onChange('short');
    field(v(), 'new-password', 1).onChange('short');
    expect(v().pwDis).toBe(false);
    v().pwSubmit();
    expect(p.pwErr).toBe('The new password must be at least 10 characters long.');
    field(v(), 'new-password').onChange('new-password1');
    expect(p.pwErr).toBeNull();
    v().pwSubmit();
    expect(p.pwErr).toBe('The new password needs upper- and lowercase letters.');
    field(v(), 'new-password').onChange('New-Password1');
    v().pwSubmit();
    expect(p.pwErr).toBe('The passwords don’t match.');
    field(v(), 'new-password', 1).onChange('New-Password1');
    v().pwSubmit();
    expect(v().pwMsg).toBe('The code must be 6 digits.');
    expect(calls.filter((c) => c.method !== 'GET')).toEqual([]);
  });

  it('reports signed-out devices and keeps only its own', async () => {
    const { p, v, calls } = await filled({
      'PUT auth/password': [{ body: { password_changed_at: minutesAgo(0), sessions_ended: 2 } }],
    });
    v().pwKey(key('Enter'));
    await vi.waitFor(() => expect(ui.toast?.text).toBe('Password changed. Other devices were signed out.'));
    expect(sent(calls).at(-1)).toEqual({
      call: 'PUT auth/password',
      body: { current_password: 'Old-Password1', new_password: 'New-Password1', code: '123456' },
    });
    expect(v().sessList.map((s) => s.name)).toEqual(['Firefox on Linux']);
    expect(v().pwDesc).toBe('Last changed just now');
    expect([p.pwCur, p.pwNew, p.pwConf, p.pwOtp]).toEqual(['', '', '', '']);
  });

  it('without other devices only "Password changed"', async () => {
    const { v, calls } = await filled(
      { 'PUT auth/password': [{ body: { password_changed_at: minutesAgo(0), sessions_ended: 0 } }] },
      NO_MFA,
    );
    v().pwSubmit();
    await vi.waitFor(() => expect(ui.toast?.text).toBe('Password changed'));
    expect(sent(calls).at(-1)?.body).toEqual({ current_password: 'Old-Password1', new_password: 'New-Password1' });
  });

  it('reports server responses and clears only the affected field', async () => {
    const { p, v } = await filled({
      'PUT auth/password': [
        { status: 422, body: { error: 'invalid_password' } },
        { status: 422, body: { error: 'invalid', fields: { new_password: { code: 'policy', rules: ['num'] } } } },
        { status: 422, body: { error: 'invalid', fields: { new_password: { code: 'too_long' } } } },
        { status: 422, body: { error: 'code_used' } },
        { status: 422, body: { error: 'invalid_code' } },
        { status: 429, body: { error: 'rate_limited', retry_after: 900 } },
      ],
    });
    await p.changePassword();
    expect([v().pwMsg, p.pwCur, p.pwOtp]).toEqual(['The password is incorrect.', '', '123456']);
    p.setPw('pwCur', 'Old-Password1');
    await p.changePassword();
    expect(p.pwErr).toBe('The new password needs a number or special character.');
    await p.changePassword();
    expect(p.pwErr).toBe('The password is too long.');
    await p.changePassword();
    expect([p.pwErr, p.pwCur, p.pwOtp]).toEqual([
      'This code has already been used. Wait for the next code in your app.',
      'Old-Password1',
      '',
    ]);
    p.setPwOtp('654321');
    await p.changePassword();
    expect([p.pwErr, p.pwOtp]).toEqual([
      'That code isn’t right. Enter the current code from your authenticator app.',
      '',
    ]);
    p.setPwOtp('654321');
    await p.changePassword();
    expect([p.pwErr, p.pwCur, p.pwOtp]).toEqual([
      'Too many failed attempts. Try again in 15 minutes.',
      'Old-Password1',
      '654321',
    ]);
    expect(v().hasPwMsg).toBe(true);
  });

  it('reloads when the server asks for a code the page did not ask for', async () => {
    const { p, calls } = await filled(
      { 'PUT auth/password': [{ status: 422, body: { error: 'invalid', fields: { code: { code: 'required' } } } }] },
      NO_MFA,
    );
    await p.changePassword();
    expect(p.pwErr).toBe('The code must be 6 digits.');
    await vi.waitFor(() => expect(count(calls, 'auth/totp/devices')).toBe(2));
  });

  it('a fresh sign-in asked for by the server leads to the sign-in', async () => {
    const reauth = vi.fn();
    const { p } = await filled({ 'PUT auth/password': [{ status: 403, body: { error: 'reauth_required' } }] }, MFA, {
      reauth,
    });
    await p.changePassword();
    expect(reauth).toHaveBeenCalledOnce();
    expect(ui.toast).toBeNull();
  });

  it('busy or unexpected errors show the error toast and keep the inputs', async () => {
    const { p } = await filled({ 'PUT auth/password': [{ status: 503, body: { error: 'busy' } }] });
    await p.changePassword();
    expect(ui.toast).toMatchObject({ text: 'That didn’t work. Please try again.', error: true });
    expect([p.pwCur, p.pwOtp, p.pwBusy]).toEqual(['Old-Password1', '123456', false]);
  });
});

describe('Authenticator apps', () => {
  it('adds the first app, shows QR code, key and then the codes', async () => {
    const after: MfaOverview = {
      devices: [{ id: 't3', name: 'Authenticator app', created_at: minutesAgo(0), last_used_at: null }],
      recovery: { total: 10, unused: 10, created_at: minutesAgo(0) },
      recovery_codes: CODES,
    };
    const { p, v, calls } = await loaded(
      {
        'POST auth/totp/devices/enroll': [{ body: ENROLL }],
        'POST auth/totp/devices/confirm': [{ body: after }],
      },
      NO_MFA,
    );
    v().tfaAdd();
    expect([v().totpPanel, v().totpSetup, v().totpBusy]).toEqual([true, true, true]);
    await vi.waitFor(() => expect(v().totpSecret).toBe('JBSW Y3DP EHPK 3PXP JBSW Y3DP EHPK 3PXP'));
    expect(v().totpQr?.size).toBeGreaterThan(20);
    expect(v().totpIntro).toBe('Enter your password and the 6-digit code from the new app.');
    expect(v().totpGoLabel).toBe('Add');
    v().totpGo();
    expect(v().tErr).toBe('Please enter your password.');
    v().onTPw('Secret-123');
    v().totpGo();
    expect(v().tErr).toBe('Please enter the 6-digit code.');
    v().onTCode('123 456');
    v().tKey(key('Enter'));
    await vi.waitFor(() => expect(ui.toast?.text).toBe('Multi-factor authentication turned on'));
    expect(sent(calls).at(-1)).toEqual({
      call: 'POST auth/totp/devices/confirm',
      body: { id: 't3', password: 'Secret-123', code: '123456' },
    });
    const values = v();
    expect([values.totpPanel, values.rcShow, values.rcOn]).toEqual([false, true, true]);
    expect(values.rcCodes[0]).toEqual({ code: 'ABCD-EF10', n: '1' });
    expect(values.rcCodes).toHaveLength(10);
    expect(values.pwFields.some((f) => f.ac === 'one-time-code')).toBe(true);
    values.rcDone();
    expect(v().rcShow).toBe(false);
    expect(p.enrollment).toBeNull();
  });

  it('adds another app with a name', async () => {
    const after: MfaOverview = {
      devices: [...MFA.devices, { id: 't3', name: 'Laptop', created_at: minutesAgo(0), last_used_at: null }],
      recovery: MFA.recovery,
    };
    const { v, calls } = await loaded({
      'POST auth/totp/devices/enroll': [{ body: ENROLL }],
      'POST auth/totp/devices/confirm': [{ body: after }],
    });
    v().tfaAdd();
    await vi.waitFor(() => expect(v().totpSecret).not.toBe(''));
    v().onTName(' Laptop ');
    v().onTPw('Secret-123');
    v().onTCode('123456');
    v().totpGo();
    await vi.waitFor(() => expect(ui.toast?.text).toBe('Laptop added'));
    expect((sent(calls).at(-1)?.body as { name: string }).name).toBe('Laptop');
    expect(v().rcShow).toBe(false);
  });

  it('a failed enrolment closes the prompt with the error toast', async () => {
    const { v } = await loaded({ 'POST auth/totp/devices/enroll': [{ status: 500, body: {} }] });
    v().tfaAdd();
    await vi.waitFor(() => expect(ui.toast).toMatchObject({ error: true }));
    expect(v().totpPanel).toBe(false);
  });

  it('reports a wrong code of the new app, and cancel discards the enrolment', async () => {
    const { p, v } = await loaded({
      'POST auth/totp/devices/enroll': [{ body: ENROLL }],
      'POST auth/totp/devices/confirm': [{ status: 422, body: { error: 'invalid_code' } }],
    });
    await p.addApp();
    v().onTPw('Old-Password1');
    v().onTCode('123456');
    await p.confirmTotp();
    expect([v().tErr, p.tPw, p.tCode]).toEqual([
      'That code isn’t right. Enter the current code from the new app.',
      'Old-Password1',
      '',
    ]);
    expect(v().hasTErr).toBe(true);
    v().tKey(key('Escape'));
    expect([v().totpPanel, p.enrollment, p.tPw]).toEqual([false, null, '']);
  });

  it('offers "Start over" if the enrolment was replaced in the meantime, and keeps name and password', async () => {
    const NEXT = { ...ENROLL, id: 't4', secret: 'KRSXG5CTMVRXEZLUKRSXG5CTMVRXEZLU' };
    const { p, v, calls } = await loaded({
      'POST auth/totp/devices/enroll': [{ body: ENROLL }, { body: NEXT }],
      'POST auth/totp/devices/confirm': [{ status: 404, body: { error: 'not_found' } }],
    });
    await p.addApp();
    v().onTName('Laptop');
    v().onTPw('Secret-123');
    v().onTCode('123456');
    await p.confirmTotp();
    expect(v().tStale).toBe(true);
    expect([p.tCode, p.tName, p.tPw]).toEqual(['', 'Laptop', 'Secret-123']);
    expect(ui.toast).toBeNull();
    v().tRestart();
    expect(v().tStale).toBe(false);
    expect([p.tName, p.tPw, p.tCode]).toEqual(['Laptop', 'Secret-123', '']);
    await vi.waitFor(() => expect(p.enrollment?.id).toBe('t4'));
    expect(v().totpSecret).toBe('KRSX G5CT MVRX EZLU KRSX G5CT MVRX EZLU');
    expect(count(calls, 'auth/totp/devices/enroll')).toBe(2);
  });

  it('checks the app name before password and code, without a request', async () => {
    const { p, v, calls } = await loaded({ 'POST auth/totp/devices/enroll': [{ body: ENROLL }] });
    await p.addApp();
    v().onTName(`Lap${ch(0x200b)}top`);
    await p.confirmTotp();
    expect(v()).toMatchObject({
      tNameErr: 'The name contains invalid characters.',
      hasTNameErr: true,
      tErr: '',
      hasTErr: false,
    });
    v().onTName('Laptop');
    expect(v()).toMatchObject({ tNameErr: '', hasTNameErr: false });
    await p.confirmTotp();
    expect(v()).toMatchObject({ tErr: 'Please enter your password.', hasTNameErr: false });
    expect(count(calls, 'auth/totp/devices/confirm')).toBe(0);
  });

  it.each([
    ['a control character', `Lap${ch(7)}top`],
    ['a tab at the end', 'Laptop\t'],
    ['a C1 control character', `Lap${ch(0x85)}top`],
    ['a soft hyphen', `Lap${ch(0xad)}top`],
    ['a right-to-left mark', `Lap${ch(0x200f)}top`],
    ['a line separator', `Lap${ch(0x2028)}top`],
    ['a direction override', `Lap${ch(0x202e)}top`],
    ['a word joiner', `Lap${ch(0x2060)}top`],
    ['a byte order mark', `${ch(0xfeff)}Laptop`],
  ])('rejects an app name with %s before sending', async (_, name) => {
    const { p, v, calls } = await loaded({ 'POST auth/totp/devices/enroll': [{ body: ENROLL }] });
    await p.addApp();
    v().onTName(name);
    v().onTPw('Secret-123');
    v().onTCode('123456');
    await p.confirmTotp();
    expect(v().tNameErr).toBe('The name contains invalid characters.');
    expect(count(calls, 'auth/totp/devices/confirm')).toBe(0);
  });

  it('keeps the server check of the app name as a fallback', async () => {
    const { p, v } = await loaded({
      'POST auth/totp/devices/enroll': [{ body: ENROLL }],
      'POST auth/totp/devices/confirm': [
        { status: 422, body: { error: 'invalid', fields: { name: { code: 'invalid' } } } },
        { status: 422, body: { error: 'invalid', fields: { name: { code: 'too_long' } } } },
      ],
    });
    await p.addApp();
    v().onTName(`Lap${ch(0xfff9)}top`);
    v().onTPw('Secret-123');
    v().onTCode('123456');
    await p.confirmTotp();
    expect(v()).toMatchObject({ tNameErr: 'The name contains invalid characters.', hasTNameErr: true });
    await p.confirmTotp();
    expect(v().tNameErr).toBe('The name is too long.');
    expect(ui.toast).toBeNull();
  });

  it('removes an app with password and code', async () => {
    const after: MfaOverview = { devices: [LAPTOP], recovery: MFA.recovery };
    const { v, calls } = await loaded({ 'DELETE auth/totp/devices/t1': [{ body: after }] });
    v().tfaEntries[0]?.onRemove();
    let values = v();
    expect([values.totpPanel, values.totpSetup, values.totpGoLabel]).toEqual([true, false, 'Remove']);
    expect(values.tfaEntries[0]?.selected).toBe(true);
    expect(values.totpIntro).toBe('To remove “iPhone”, confirm with your password and a code from one of your apps.');
    values.onTPw('Secret-123');
    values.onTCode('654321');
    values.totpGo();
    await vi.waitFor(() => expect(ui.toast?.text).toBe('iPhone removed'));
    expect(sent(calls).at(-1)).toEqual({
      call: 'DELETE auth/totp/devices/t1',
      body: { password: 'Secret-123', code: '654321' },
    });
    values = v();
    expect(values.tfaEntries.map((e) => e.name)).toEqual(['Authenticator app 2']);
    expect(values.totpPanel).toBe(false);
  });

  it('the last app switches off multi-factor and the codes', async () => {
    const one: MfaOverview = { devices: [IPHONE], recovery: MFA.recovery };
    const { p, v } = await loaded({ 'DELETE auth/totp/devices/t1': [{ body: NO_MFA }] }, one);
    p.codes = CODES;
    v().tfaEntries[0]?.onRemove();
    expect(v().totpIntro).toBe(
      'To remove “iPhone”, confirm with your password and a code from one of your apps. Multi-factor authentication will then be turned off.',
    );
    p.setTPw('Secret-123');
    p.setTCode('123456');
    await p.confirmTotp();
    expect(ui.toast?.text).toBe('Multi-factor authentication turned off');
    const values = v();
    expect([values.tfaEmpty, values.rcOff, values.rcShow]).toEqual([true, true, false]);
    expect(values.pwFields.some((f) => f.ac === 'one-time-code')).toBe(false);
  });

  it('reports an already used code, a lockout, and reloads on 404', async () => {
    const { p, v, calls } = await loaded({
      'DELETE auth/totp/devices/t1': [
        { status: 422, body: { error: 'code_used' } },
        { status: 429, body: { error: 'rate_limited', retry_after: 61 } },
        { status: 404, body: { error: 'not_found' } },
      ],
    });
    p.askRemove('t1');
    p.setTPw('Secret-123');
    p.setTCode('123456');
    await p.confirmTotp();
    expect([v().tErr, p.tCode]).toEqual(['This code has already been used. Wait for the next code in your app.', '']);
    p.setTCode('654321');
    await p.confirmTotp();
    expect([v().tErr, p.tPw, p.tCode]).toEqual([
      'Too many failed attempts. Try again in 2 minutes.',
      'Secret-123',
      '654321',
    ]);
    await p.confirmTotp();
    expect(v().totpPanel).toBe(false);
    await vi.waitFor(() => expect(count(calls, 'auth/totp/devices')).toBe(2));
  });

  it('a late response neither closes nor locks the prompt opened in the meantime', async () => {
    const answer = later();
    const after: MfaOverview = {
      devices: [...MFA.devices, { id: 't3', name: 'Laptop', created_at: minutesAgo(0), last_used_at: null }],
      recovery: MFA.recovery,
    };
    const { p, v } = await loaded({
      'POST auth/totp/devices/enroll': [{ body: ENROLL }],
      'POST auth/totp/devices/confirm': [answer.reply],
    });
    await p.addApp();
    p.setTPw('Secret-123');
    p.setTCode('123456');
    const confirming = p.confirmTotp();
    p.cancelTotp();
    p.askRemove('t1');
    expect(v().totpBusy).toBe(false);
    p.setTPw('Other-1');
    answer.release({ body: after });
    await confirming;
    expect([p.totpMode, p.totpTarget, p.tPw]).toEqual(['off', 't1', 'Other-1']);
    expect(v().tfaEntries.map((e) => e.name)).toContain('Laptop');
    expect(ui.toast?.text).toBe('Laptop added');
  });

  it('copies the key without spaces', async () => {
    const writeText = vi.fn(() => Promise.resolve());
    vi.stubGlobal('navigator', { clipboard: { writeText } });
    const { p, v } = await loaded({ 'POST auth/totp/devices/enroll': [{ body: ENROLL }] });
    await p.addApp();
    v().copyTotp();
    await vi.waitFor(() => expect(ui.toast?.text).toBe('Key copied'));
    expect(writeText).toHaveBeenCalledWith(ENROLL.secret);
  });
});

describe('Recovery codes', () => {
  it('generates new codes with password and code and shows them once', async () => {
    const after: MfaOverview = {
      ...MFA,
      recovery: { total: 10, unused: 10, created_at: minutesAgo(0) },
      recovery_codes: CODES,
    };
    const { v, calls } = await loaded({ 'POST auth/recovery-codes': [{ body: after }] });
    v().rcRegen();
    let values = v();
    expect([values.rcForm, values.rcNoForm]).toEqual([true, false]);
    values.rcGo();
    expect(v().rcErr).toBe('Please enter your password.');
    values.onRcPw('Secret-123');
    v().rcGo();
    expect(v().rcErr).toBe('Please enter the 6-digit code.');
    v().onRcCode('123456');
    v().rcKey(key('Enter'));
    await vi.waitFor(() => expect(ui.toast?.text).toBe('New recovery codes generated'));
    expect(sent(calls).at(-1)).toEqual({
      call: 'POST auth/recovery-codes',
      body: { password: 'Secret-123', code: '123456' },
    });
    values = v();
    expect([values.rcShow, values.rcForm, values.rcSum, values.rcSub]).toEqual([
      true,
      false,
      '10 of 10 unused',
      'Generated just now',
    ]);
  });

  it('reports a lockout and keeps the inputs; Escape closes', async () => {
    const { p, v } = await loaded({
      'POST auth/recovery-codes': [{ status: 429, body: { error: 'rate_limited', retry_after: 900 } }],
    });
    p.openRegen();
    p.setRcPw('Old-Password1');
    p.setRcCode('123456');
    await p.regenerate();
    expect([v().rcErr, v().rcPwVal, v().rcCodeVal, v().rcForm]).toEqual([
      'Too many failed attempts. Try again in 15 minutes.',
      'Old-Password1',
      '123456',
      true,
    ]);
    v().rcKey(key('Escape'));
    expect(v().rcForm).toBe(false);
  });

  it('reloads when the last app was removed in the meantime', async () => {
    const { p, calls } = await loaded({
      'POST auth/recovery-codes': [{ status: 409, body: { error: 'mfa_disabled' } }],
      'GET auth/totp/devices': [{ body: MFA }, { body: NO_MFA }],
    });
    p.openRegen();
    p.setRcPw('Old-Password1');
    p.setRcCode('123456');
    await p.regenerate();
    expect(p.rcForm).toBe(false);
    await vi.waitFor(() => expect(p.devices).toEqual([]));
    expect(count(calls, 'auth/totp/devices')).toBe(2);
  });

  it('copies and downloads the codes', async () => {
    const writeText = vi.fn(() => Promise.resolve());
    vi.stubGlobal('navigator', { clipboard: { writeText } });
    // jsdom knows no blob URLs and would navigate when clicking the link.
    const created = vi.fn((blob: Blob) => (blob.size ? 'blob:codes' : ''));
    Object.assign(URL, { createObjectURL: created, revokeObjectURL: vi.fn() });
    const click = vi.spyOn(HTMLAnchorElement.prototype, 'click').mockImplementation(() => {});
    const { p, v } = await loaded();
    p.codes = CODES;
    v().rcCopy();
    await vi.waitFor(() => expect(ui.toast?.text).toBe('Codes copied'));
    expect(writeText).toHaveBeenCalledWith(CODES.join('\n'));
    v().rcDownload();
    expect(ui.toast?.text).toBe('Codes downloaded');
    expect(created).toHaveBeenCalled();
    expect(click).toHaveBeenCalled();
    click.mockRestore();
    Reflect.deleteProperty(URL, 'createObjectURL');
    Reflect.deleteProperty(URL, 'revokeObjectURL');
  });
});

describe('Signed-in devices', () => {
  it('signs out a device: hidden at once, gone after the answer', async () => {
    const answer = later();
    const { p, v, calls } = await loaded({ 'DELETE auth/sessions/s1': [answer.reply] });
    const revoking = p.revoke('s1');
    expect(v().sessList.map((s) => s.id)).toEqual(['s0', 's2']);
    answer.release({ status: 204 });
    await revoking;
    expect(ui.toast?.text).toBe('Device signed out');
    expect(p.sessions.map((s) => s.id)).toEqual(['s0', 's2']);
    expect(sent(calls).at(-1)).toEqual({ call: 'DELETE auth/sessions/s1', body: undefined });
  });

  it('an already ended device counts as signed out', async () => {
    const { p } = await loaded({ 'DELETE auth/sessions/s1': [{ status: 404, body: { error: 'not_found' } }] });
    await p.revoke('s1');
    expect(ui.toast?.text).toBe('Device signed out');
    expect(p.sessions.map((s) => s.id)).toEqual(['s0', 's2']);
  });

  it('does not sign out its own session here', async () => {
    const { p, calls } = await loaded();
    await p.revoke('s0');
    expect(calls.filter((c) => c.method === 'DELETE')).toEqual([]);
  });

  it('brings the device back if the server rejects it, and reloads', async () => {
    const { p, v, calls } = await loaded({ 'DELETE auth/sessions/s1': [{ status: 500, body: {} }] });
    await p.revoke('s1');
    expect(ui.toast).toMatchObject({ text: 'The device wasn’t signed out. Please try again.', error: true });
    expect(v().sessList.map((s) => s.id)).toEqual(['s0', 's1', 's2']);
    await vi.waitFor(() => expect(count(calls, 'auth/sessions')).toBe(2));
  });

  it('asks before signing out all others, then keeps only its own', async () => {
    const { v, calls } = await loaded({ 'POST auth/sessions/revoke-others': [{ body: { sessions_ended: 2 } }] });
    v().revokeOthers();
    expect(ui.confirm).toMatchObject({ title: 'Sign out all other devices?', ok: 'Sign out', danger: true });
    expect(calls.filter((c) => c.method === 'POST')).toEqual([]);
    ui.confirmOk();
    expect(v().sessList.map((s) => s.id)).toEqual(['s0']);
    await vi.waitFor(() => expect(ui.toast?.text).toBe('Other devices signed out'));
    expect(calls.filter((c) => c.method === 'POST').map((c) => c.path)).toEqual(['auth/sessions/revoke-others']);
    expect(v().hasOtherSess).toBe(false);
  });

  it('cancelling the question signs out nothing', async () => {
    const { v, calls } = await loaded();
    v().revokeOthers();
    ui.confirmCancel();
    expect(calls.filter((c) => c.method === 'POST')).toEqual([]);
    expect(v().sessList.map((s) => s.id)).toEqual(['s0', 's1', 's2']);
  });

  it('brings the others back if the server rejects "Sign out all others"', async () => {
    const { v, calls } = await loaded({ 'POST auth/sessions/revoke-others': [{ status: 500, body: {} }] });
    v().revokeOthers();
    ui.confirmOk();
    await vi.waitFor(() =>
      expect(ui.toast).toMatchObject({ text: 'The other devices weren’t signed out. Please try again.', error: true }),
    );
    expect(v().sessList.map((s) => s.id)).toEqual(['s0', 's1', 's2']);
    await vi.waitFor(() => expect(count(calls, 'auth/sessions')).toBe(2));
  });

  it('an ended own session reloads the account instead of a toast', async () => {
    const { p, calls } = await loaded({
      'POST auth/sessions/revoke-others': [{ status: 401, body: { error: 'unauthorized' } }],
      'GET session': [{ status: 401, body: { error: 'unauthorized' } }],
    });
    p.revokeOthers();
    ui.confirmOk();
    await vi.waitFor(() => expect(session.user).toBeNull());
    expect(ui.toast).toBeNull();
    expect(count(calls, 'auth/sessions')).toBe(1);
  });

  it('without other devices there is nothing to ask', async () => {
    const { p } = await loaded({}, MFA, [SESSIONS[0] as ListedSession]);
    p.revokeOthers();
    expect(ui.confirm).toBeNull();
  });
});
