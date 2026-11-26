// Account › Profile in the Chromium audit (scripts/a11y/audit.mjs): every
// state of the page – loaded, load failed, unsaved changes with an error,
// password and code prompt with a message, confirmation pending with a
// failure, and the only administrator with "Delete account" locked.
export default ({ mock, fill, button, USER, PASSWORD }) => {
  const ME = {
    id: USER.id,
    username: USER.username,
    email: USER.email,
    display_name: USER.name,
    role: USER.role,
    mfa_enabled: true,
    password_changed_at: '2026-10-20T10:00:00Z',
    email_change: null,
    sole_admin: false,
    system_mail_configured: true,
  };
  const PROFILE_USER = {
    id: USER.id,
    username: USER.username,
    email: USER.email,
    display_name: USER.name,
    role: USER.role,
  };
  const OPEN = { email: 'new@example.com', expires_at: '2026-12-03T10:00:00Z' };
  const path = '/account/profile';
  const at = (routes) => mock({ signedIn: true, routes });

  return [
    {
      id: 'profile',
      path,
      opts: at({ 'GET auth/me': { body: ME } }),
      prepare: (page) => page.getByText('Shown to other people').waitFor(),
    },
    {
      id: 'profile load failed',
      path,
      opts: at({ 'GET auth/me': { status: 500, body: { error: 'internal' } } }),
      prepare: (page) => page.getByRole('button', { name: 'Try again' }).waitFor(),
    },
    {
      id: 'profile unsaved',
      path,
      opts: at({ 'GET auth/me': { body: ME } }),
      prepare: async (page) => {
        await button(page, 'Edit display name');
        await fill(page, 'input[aria-label="Display name"]', 'Alex Rivera');
        await button(page, 'Edit Username');
        await fill(page, 'input[aria-label="Username"]', 'Alex!');
        await page.getByRole('region', { name: 'Unsaved changes' }).getByRole('button', { name: 'Save' }).click();
        await page.locator('.row-err').waitFor();
      },
    },
    {
      id: 'profile email prompt',
      path,
      opts: at({
        'GET auth/me': { body: ME },
        'PATCH auth/me': [
          { body: { status: 'reauth_required', user: PROFILE_USER } },
          { status: 422, body: { error: 'invalid_password' } },
        ],
      }),
      prepare: async (page) => {
        await button(page, 'Edit Email');
        const email = page.locator('input[aria-label="Email"]');
        await email.fill('new@example.com');
        await email.press('Enter');
        await page.locator('input[autocomplete="current-password"]').waitFor();
        await fill(page, 'input[autocomplete="current-password"]', PASSWORD);
        await fill(page, 'input[autocomplete="one-time-code"]', '123456');
        await button(page, 'Send confirmation link');
        await page.getByText('The password is incorrect.').waitFor();
      },
    },
    {
      id: 'profile email pending',
      path,
      opts: at({
        'GET auth/me': { body: { ...ME, email_change: OPEN } },
        'POST auth/me/email/resend': { status: 502, body: { error: 'mail_failed', previous_link_valid: true } },
      }),
      prepare: async (page) => {
        await button(page, 'Resend link');
        await page.locator('.pend-err').waitFor();
      },
    },
    {
      id: 'profile delete confirmation',
      path,
      opts: at({ 'GET auth/me': { body: ME } }),
      prepare: async (page) => {
        await button(page, 'Delete account');
        await page.getByRole('alertdialog').waitFor();
      },
    },
    {
      id: 'profile only administrator',
      path,
      opts: at({ 'GET auth/me': { body: { ...ME, sole_admin: true } } }),
      prepare: (page) => page.getByRole('link', { name: 'Manage users' }).waitFor(),
    },
  ];
};
