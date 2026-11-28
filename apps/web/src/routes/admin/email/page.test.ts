import { fireEvent, screen, within } from '@testing-library/svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { ui } from '@roedy/ui';
import { renderSignedIn, resetSignedIn, setPhone, type ApiRoutes } from '#lib/test/signed-in.ts';
import { NO_MAIL, type SystemMail } from '#lib/system-mail.svelte.ts';
import Page from './+page.svelte';

vi.mock('$app/navigation', () => ({ goto: vi.fn(() => Promise.resolve()) }));
const sections = vi.hoisted(() => ({ scrollToSection: vi.fn() }));
vi.mock('#lib/sections.ts', () => sections);

const IDS = [{ id: 'i1', name: 'Example Org', email: 'noreply@example.com' }];
const STORED: SystemMail = {
  url: 'https://mail.example.com',
  auth: 'password',
  user: 'noreply@example.com',
  has_secret: true,
  identity_id: 'i1',
  sender_name: null,
  reply_to: null,
  keep_sent: true,
  status: 'ok',
  reason: null,
  checked_at: new Date(Date.now() - 5 * 60_000).toISOString(),
  identities: IDS,
};

afterEach(() => {
  sections.scrollToSection.mockClear();
  resetSignedIn();
});

function renderPage(mail: SystemMail | null = STORED, routes: ApiRoutes = {}) {
  return renderSignedIn(Page, {
    path: '/admin/email',
    api: { 'GET admin/system-mail': mail ? [{ body: mail }] : [() => new Promise(() => {})], ...routes },
  });
}

const field = (label: string) => screen.getByLabelText(label, { selector: 'input' });
const bar = () => screen.queryByRole('region', { name: 'Unsaved changes' });
const ready = () =>
  vi.waitFor(() => expect(screen.getByRole('heading', { level: 2, name: 'Status' })).toBeInTheDocument());

describe('Administration › Email page', () => {
  it('shows heading, description and the skeleton until the first response, without tabs', () => {
    const view = renderPage(null);
    expect(view.getByRole('heading', { level: 1 })).toHaveTextContent('Email');
    expect(view.getByText('Sending system emails via JMAP.')).toBeInTheDocument();
    expect(view.queryByRole('button', { name: 'Connection' })).toBeNull();
    expect(view.queryByRole('heading', { level: 2 })).toBeNull();
  });

  it('shows "Connected" with session, account and "Last checked" and the fields', async () => {
    const view = renderPage();
    await ready();
    for (const tab of ['Connection', 'Sender', 'Delivery', 'Test'])
      expect(view.getByRole('button', { name: tab })).toBeInTheDocument();
    expect(view.getByText('Connected')).toBeInTheDocument();
    expect(view.getByText('JMAP session with mail.example.com is active.')).toBeInTheDocument();
    expect(view.getByText('https://mail.example.com/.well-known/jmap')).toBeInTheDocument();
    expect(view.getByText('5 min ago')).toBeInTheDocument();
    expect(field('Server address')).toHaveValue('https://mail.example.com');
    expect(field('Server address')).toHaveAttribute('type', 'url');
    expect(field('Username')).toHaveValue('noreply@example.com');
    expect(field('Password')).toHaveValue('');
    expect(field('Password')).toHaveAttribute('placeholder', '••••••••');
    expect(field('Sender name')).toHaveAttribute('placeholder', 'Example Org');
    expect(view.getByText('Example Org <noreply@example.com>')).toBeInTheDocument();
    expect(view.getByRole('switch', { name: 'Save a copy to Sent' })).toHaveAttribute('aria-checked', 'true');
    expect(field('Recipient')).toHaveValue('alex@example.com');
    expect(view.getByRole('button', { name: 'Send' })).toHaveAttribute('aria-disabled', 'false');
    expect(view.queryByText('Available after a successful connection test.')).toBeNull();
    expect(bar()).toBeNull();
    // No template preview in the hub.
    expect(view.queryByRole('tablist')).toBeNull();
  });

  it('shows "Not set up" without an account and locks "Send" with a notice', async () => {
    const view = renderPage(NO_MAIL);
    await ready();
    expect(view.getByText('Not set up')).toBeInTheDocument();
    expect(view.getByRole('button', { name: 'Send' })).toHaveAttribute('aria-disabled', 'true');
    expect(view.getByText('Available after a successful connection test.')).toBeInTheDocument();
    expect(field('Server address')).toHaveAttribute('placeholder', 'https://mail.example.com');
  });

  it('tests the connection and shows "No connection" with the reason', async () => {
    const view = renderPage(STORED, {
      'POST admin/system-mail/test': [{ status: 422, body: { error: 'mail_failed', reason: 'auth_failed' } }],
    });
    await ready();
    await fireEvent.click(view.getByRole('button', { name: 'Test connection' }));
    await vi.waitFor(() => expect(view.getByText('No connection')).toBeInTheDocument());
    expect(view.getByText('Sign-in failed. The username or password is incorrect.')).toBeInTheDocument();
    expect(ui.toast).toMatchObject({ text: 'Connection failed', error: true });
  });

  it('switches to the token, shows "Not checked" and the bar, and discards', async () => {
    const view = renderPage();
    await ready();
    const group = view.getByRole('group', { name: 'Sign-in' });
    await fireEvent.click(within(group).getByRole('button', { name: 'Token' }));
    expect(within(group).getByRole('button', { name: 'Token' })).toHaveAttribute('aria-pressed', 'true');
    expect(screen.queryByLabelText('Username', { selector: 'input' })).toBeNull();
    expect(field('Access token')).toHaveValue('');
    expect(view.getByText('Not checked')).toBeInTheDocument();
    await fireEvent.click(within(bar() as HTMLElement).getByRole('button', { name: 'Discard' }));
    expect(bar()).toBeNull();
    expect(field('Username')).toBeInTheDocument();
  });

  it('shows and hides the secret', async () => {
    const view = renderPage();
    await ready();
    expect(field('Password')).toHaveAttribute('type', 'password');
    await fireEvent.click(view.getByRole('button', { name: 'Show' }));
    expect(field('Password')).toHaveAttribute('type', 'text');
    expect(view.getByRole('button', { name: 'Hide' })).toBeInTheDocument();
  });

  it('shows field errors at the field, marks a missing password on saving and sends nothing', async () => {
    const view = renderPage();
    await ready();
    await fireEvent.input(field('Server address'), { target: { value: 'mail.example.com' } });
    const input = field('Server address');
    expect(input).toHaveAttribute('aria-invalid', 'true');
    const alert = view.getByRole('alert');
    expect(alert).toHaveTextContent('Enter the address as https://mail.example.com, without a path.');
    expect(input).toHaveAttribute('aria-describedby', alert.id);
    expect(input.parentElement?.contains(alert)).toBe(false);

    await fireEvent.input(field('Server address'), { target: { value: 'https://other.example.com' } });
    expect(view.queryByRole('alert')).toBeNull();
    await fireEvent.click(within(bar() as HTMLElement).getByRole('button', { name: 'Save' }));
    expect(ui.toast).toMatchObject({ text: 'Please check the highlighted fields', error: false });
    expect(sections.scrollToSection.mock.calls.at(-1)?.[1]).toBe('conn');
    expect(view.getByRole('alert')).toHaveTextContent('Enter the password.');
    expect(field('Password')).toHaveAttribute('aria-invalid', 'true');
    expect(view.calls.filter((c) => c.method === 'PUT')).toHaveLength(0);
    await fireEvent.input(field('Password'), { target: { value: 'pw' } });
    expect(view.queryByRole('alert')).toBeNull();
  });

  it('saves with "Saving …" and "Checking …", then shows the result', async () => {
    let answer: (reply: { body: unknown }) => void = () => {};
    const view = renderPage(STORED, { 'PUT admin/system-mail': [() => new Promise((resolve) => (answer = resolve))] });
    await ready();
    await fireEvent.input(field('Password'), { target: { value: 'new-pw' } });
    await fireEvent.input(field('Reply-to address'), { target: { value: 'help@example.com' } });
    await fireEvent.click(within(bar() as HTMLElement).getByRole('button', { name: 'Save' }));
    await vi.waitFor(() => expect(view.getByText('Checking …')).toBeInTheDocument());
    expect(within(bar() as HTMLElement).getByRole('button', { name: /Saving/ })).toHaveAttribute('aria-busy', 'true');
    expect(view.calls.filter((c) => c.method === 'PUT').map((c) => c.body)).toEqual([
      { app_password: 'new-pw', reply_to: 'help@example.com' },
    ]);
    answer({ body: { ...STORED, reply_to: 'help@example.com' } });
    await vi.waitFor(() => expect(bar()).toBeNull());
    expect(view.getByText('Connected')).toBeInTheDocument();
    expect(field('Password')).toHaveValue('');
    expect(ui.toast?.text).toBe('Changes saved');
  });

  it('sends a test email', async () => {
    const view = renderPage(STORED, {
      'POST admin/system-mail/test': [{ body: { identities: IDS, sent: true } }],
    });
    await ready();
    await fireEvent.input(field('Recipient'), { target: { value: 'help@example.com' } });
    await fireEvent.click(view.getByRole('button', { name: 'Send' }));
    await vi.waitFor(() => expect(ui.toast?.text).toBe('Test email sent to help@example.com'));
    expect(view.calls.at(-1)?.body).toMatchObject({ to: 'help@example.com' });
  });

  it('scrolls to the section via the tabs', async () => {
    const view = renderPage();
    await ready();
    await fireEvent.click(view.getByRole('button', { name: 'Delivery' }));
    const [root, key] = sections.scrollToSection.mock.calls.at(-1) as [HTMLElement, string, boolean];
    expect(key).toBe('delivery');
    expect([...root.querySelectorAll('section')].map((s) => s.dataset.sec)).toEqual([
      'status',
      'conn',
      'sender',
      'delivery',
      'test',
    ]);
  });

  it('shows send below the field and errors directly below the field on mobile', async () => {
    setPhone(true);
    const view = renderPage(NO_MAIL);
    await ready();
    const send = view.getByRole('button', { name: 'Send' });
    expect(send.closest('.m-send')).not.toBeNull();
    await fireEvent.input(field('Reply-to address'), { target: { value: 'no-at' } });
    const alert = view.getByRole('alert');
    expect(alert).toHaveTextContent('Please enter a valid email address.');
    expect(field('Reply-to address').parentElement?.contains(alert)).toBe(true);
    const region = bar() as HTMLElement;
    expect(within(region).queryByText('Unsaved changes')).toBeNull();
  });

  it('shows the card instead of tabs and sections when the first load fails, and reloads', async () => {
    const view = renderSignedIn(Page, {
      path: '/admin/email',
      api: { 'GET admin/system-mail': [{ status: 500, body: { error: 'internal' } }, { body: STORED }] },
    });
    await vi.waitFor(() => expect(view.getByText('Couldn’t load the email settings')).toBeInTheDocument());
    expect(view.queryByRole('button', { name: 'Connection' })).toBeNull();
    await fireEvent.click(view.getByRole('button', { name: 'Try again' }));
    await ready();
    expect(view.getByText('Connected')).toBeInTheDocument();
  });
});
