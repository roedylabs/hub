// Account › Security: with and without an authenticator app, loading failed, adding an app (QR code, error,
// replaced setup), removing the last app, new recovery codes, a lockout, a password error and the question before
// "Sign out all other devices".
export default ({ mock, button, ENROLL, CODES, PASSWORD }) => {
  const MFA = {
    devices: [{ id: 't1', name: 'iPhone', created_at: '2026-10-01T09:00:00Z', last_used_at: '2026-10-26T10:00:00Z' }],
    recovery: { total: 10, unused: 9, created_at: '2026-10-01T09:00:00Z' },
  };
  const NO_MFA = { devices: [], recovery: null };
  const SESSIONS = [
    {
      id: '01JSESSION',
      current: true,
      device: 'Firefox on Linux',
      device_type: 'desktop',
      ip: '198.51.x.x',
      created_at: '2026-10-26T08:00:00Z',
      last_seen_at: '2026-10-26T12:00:00Z',
    },
    {
      id: 's1',
      current: false,
      device: 'Safari on iPhone',
      device_type: 'mobile',
      ip: '203.0.x.x',
      created_at: '2026-10-25T08:00:00Z',
      last_seen_at: '2026-10-26T10:00:00Z',
    },
  ];
  const api = (mfa, routes = {}) =>
    mock({
      signedIn: true,
      routes: {
        'GET auth/totp/devices': { body: mfa },
        'GET auth/sessions': { body: { sessions: SESSIONS } },
        'GET auth/me': { body: { password_changed_at: '2026-10-20T12:00:00Z' } },
        ...routes,
      },
    });
  const loaded = (page) => page.getByText('Safari on iPhone').first().waitFor();
  /** The `n`-th visible field with this accessible name. */
  const field = (page, label, n) => page.getByLabel(label, { exact: true }).filter({ visible: true }).nth(n);
  const startApp = async (page) => {
    await loaded(page);
    await button(page, 'Add authenticator app');
    await page.getByRole('img', { name: 'QR code for the authenticator app' }).first().waitFor();
  };
  const confirmApp = async (page) => {
    await field(page, 'Current password', 1).fill(PASSWORD);
    await field(page, 'Verification code', 0).fill('123456');
    await button(page, 'Add');
  };
  const regen = async (page) => {
    await loaded(page);
    await button(page, 'Generate new codes');
    await field(page, 'Current password', 1).fill(PASSWORD);
    await field(page, 'Verification code', 1).fill('123456');
    await page
      .getByRole('button', { name: /^Generate( new codes)?$/ })
      .filter({ visible: true })
      .last()
      .dispatchEvent('click');
  };
  const path = '/account/security';
  return [
    { id: 'security', path, opts: api(MFA), prepare: loaded },
    { id: 'security without app', path, opts: api(NO_MFA), prepare: loaded },
    {
      id: 'security load failed',
      path,
      opts: api(MFA, { 'GET auth/totp/devices': { status: 500, body: { error: 'internal' } } }),
      prepare: (page) => page.getByRole('button', { name: 'Try again' }).first().waitFor(),
    },
    {
      id: 'security add app error',
      path,
      opts: api(NO_MFA, {
        'POST auth/totp/devices/enroll': { body: { id: 't2', ...ENROLL } },
        'POST auth/totp/devices/confirm': { status: 422, body: { error: 'invalid_code' } },
      }),
      prepare: async (page) => {
        await startApp(page);
        await confirmApp(page);
        await page.getByText('That code isn’t right.', { exact: false }).first().waitFor();
      },
    },
    {
      id: 'security add app replaced',
      path,
      opts: api(NO_MFA, {
        'POST auth/totp/devices/enroll': { body: { id: 't2', ...ENROLL } },
        'POST auth/totp/devices/confirm': { status: 404, body: { error: 'not_found' } },
      }),
      prepare: async (page) => {
        await startApp(page);
        await confirmApp(page);
        await page.getByRole('button', { name: 'Start over' }).first().waitFor();
      },
    },
    {
      id: 'security remove last app',
      path,
      opts: api(MFA),
      prepare: async (page) => {
        await loaded(page);
        await button(page, 'Remove iPhone');
        await page
          .getByText('Multi-factor authentication will then be turned off.', { exact: false })
          .first()
          .waitFor();
      },
    },
    {
      id: 'security new codes',
      path,
      opts: api(MFA, { 'POST auth/recovery-codes': { body: { ...MFA, recovery_codes: CODES } } }),
      prepare: async (page) => {
        await regen(page);
        await page.getByRole('list', { name: 'Recovery codes' }).first().waitFor();
      },
    },
    {
      id: 'security new codes locked',
      path,
      opts: api(MFA, {
        'POST auth/recovery-codes': { status: 429, body: { error: 'rate_limited', retry_after: 900 } },
      }),
      prepare: async (page) => {
        await regen(page);
        await page.getByText('Too many failed attempts.', { exact: false }).first().waitFor();
      },
    },
    {
      id: 'security password error',
      path,
      opts: api(NO_MFA, { 'PUT auth/password': { status: 422, body: { error: 'invalid_password' } } }),
      prepare: async (page) => {
        await loaded(page);
        await field(page, 'Current password', 0).fill('Wrong-Password-1');
        await field(page, 'New password', 0).fill(PASSWORD);
        await field(page, 'Confirm new password', 0).fill(PASSWORD);
        await button(page, 'Change password');
        await page.getByText('The password is incorrect.').first().waitFor();
      },
    },
    {
      id: 'security sign out others',
      path,
      opts: api(MFA),
      prepare: async (page) => {
        await loaded(page);
        await page
          .getByRole('button', { name: /^Sign out all other/ })
          .first()
          .dispatchEvent('click');
        await page.getByRole('alertdialog').first().waitFor();
      },
    },
  ];
};
