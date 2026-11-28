import { createEvent, fireEvent, screen, within } from '@testing-library/svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { ui } from '@roedy/ui';
import { renderSignedIn, resetSignedIn, setPhone, type ApiRoutes } from '#lib/test/signed-in.ts';
import { NO_MAIL, type MailServer, type SystemMail } from '#lib/system-mail.svelte.ts';
import Page from './+page.svelte';

vi.mock('$app/navigation', () => ({ goto: vi.fn(() => Promise.resolve()) }));
const sections = vi.hoisted(() => ({ scrollToSection: vi.fn() }));
vi.mock('#lib/sections.ts', () => sections);

const IDS = [{ id: 'i1', name: 'Example Org', email: 'noreply@example.com' }];
const SERVERS: MailServer[] = [
  { id: 's1', label: 'Example mail', url: 'https://mail.example.com/', host: 'mail.example.com' },
  { id: 's2', label: 'Other mail', url: 'https://other.example.com/', host: 'other.example.com' },
];
/** `GET admin/stalwart/servers` (the page reads only `servers`). */
const REGISTRY = { body: { servers: SERVERS, redirect_uri: 'https://hub.example.com/cb', system_mail_server: 's1' } };
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
  checked_at: new Date(Date.now() - 5 * 60_000).toISOString(),
  identities: IDS,
};

afterEach(() => {
  sections.scrollToSection.mockClear();
  resetSignedIn();
});

function renderPage(mail: SystemMail | null = STORED, routes: ApiRoutes = {}, servers: MailServer[] = SERVERS) {
  return renderSignedIn(Page, {
    path: '/admin/email',
    api: {
      'GET admin/system-mail': mail ? [{ body: mail }] : [() => new Promise(() => {})],
      'GET admin/stalwart/servers': [{ body: { ...REGISTRY.body, servers } }],
      ...routes,
    },
  });
}

const field = (label: string) => screen.getByLabelText(label, { selector: 'input' });
/** "Mail server": named by the row's label and the chosen server. */
const server = (value: string) => screen.getByRole('combobox', { name: `Mail server ${value}` });
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
    expect(server('Example mail')).toHaveAttribute('aria-expanded', 'false');
    expect(view.getByText('JMAP session via mail.example.com')).toBeInTheDocument();
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
    expect(server('Choose server')).toBeInTheDocument();
    expect(view.getByText('The sending account’s Stalwart server')).toBeInTheDocument();
  });

  it('chooses the server in the pick list with the mouse and the keyboard', async () => {
    const view = renderPage();
    await ready();
    const btn = server('Example mail');
    await fireEvent.click(btn);
    const list = view.getByRole('listbox', { name: 'Mail server' });
    expect(btn).toHaveAttribute('aria-expanded', 'true');
    expect(btn).toHaveAttribute('aria-controls', list.id);
    const [first, other] = within(list).getAllByRole('option');
    expect(first).toHaveAttribute('aria-selected', 'true');
    expect(other).toHaveTextContent('other.example.com');
    const down = createEvent.mouseDown(other as HTMLElement);
    await fireEvent(other as HTMLElement, down);
    expect(down.defaultPrevented).toBe(true);
    await fireEvent.click(other as HTMLElement);
    expect(view.queryByRole('listbox')).toBeNull();
    expect(server('Other mail')).toBeInTheDocument();
    expect(view.getByText('Not checked')).toBeInTheDocument();
    // The stored password belongs to the previous server.
    expect(field('Password')).toHaveAttribute('placeholder', '');
    expect(bar()).not.toBeNull();

    await fireEvent.keyDown(server('Other mail'), { key: 'ArrowDown' });
    const again = view.getByRole('listbox', { name: 'Mail server' });
    const opts = within(again).getAllByRole('option');
    expect(server('Other mail')).toHaveAttribute('aria-activedescendant', opts[1]?.id);
    await fireEvent.keyDown(server('Other mail'), { key: 'ArrowDown' });
    expect(server('Other mail')).toHaveAttribute('aria-activedescendant', opts[0]?.id);
    await fireEvent.keyDown(server('Other mail'), { key: 'Enter' });
    expect(view.queryByRole('listbox')).toBeNull();
    expect(server('Example mail')).toBeInTheDocument();
    expect(field('Password')).toHaveAttribute('placeholder', '••••••••');
    expect(bar()).toBeNull();
  });

  it('opens the sheet for the server on mobile', async () => {
    setPhone(true);
    const view = renderPage();
    await ready();
    const btn = view.getByRole('button', { name: 'Mail server Example mail' });
    expect(btn).toHaveAttribute('aria-haspopup', 'dialog');
    await fireEvent.click(btn);
    // On mobile the layout shows the list as a sheet (`ui.pick`).
    expect(ui.menu).toBe('pick');
    expect(view.queryByRole('listbox')).toBeNull();
    const pick = ui.pick?.();
    expect(pick?.title).toBe('Mail server');
    pick?.opts[1]?.onClick();
    expect(ui.menu).toBeNull();
    await vi.waitFor(() => expect(view.getByRole('button', { name: 'Mail server Other mail' })).toBeInTheDocument());
    view.unmount();
    expect(ui.pick).toBeNull();
  });

  it('shows a hint instead of the pick list without a server, and asks for one on saving', async () => {
    const view = renderPage(NO_MAIL, {}, []);
    await ready();
    expect(screen.queryByRole('combobox')).toBeNull();
    expect(
      view.getByText('No Stalwart server yet. Add one under Administration › Stalwart servers.'),
    ).toBeInTheDocument();
    await fireEvent.input(field('Username'), { target: { value: 'noreply@example.com' } });
    await fireEvent.input(field('Password'), { target: { value: 'pw' } });
    await fireEvent.click(within(bar() as HTMLElement).getByRole('button', { name: 'Save' }));
    expect(ui.toast).toMatchObject({ text: 'Please check the highlighted fields', error: false });
    expect(sections.scrollToSection.mock.calls.at(-1)?.[1]).toBe('conn');
    expect(view.getByRole('alert')).toHaveTextContent('Choose the mail server.');
    expect(view.calls.filter((c) => c.method === 'PUT')).toHaveLength(0);
  });

  it('marks the server when it was removed from the registry', async () => {
    const view = renderPage({ ...STORED, server_id: null, has_secret: false, status: 'err', reason: 'unreachable' });
    await ready();
    await fireEvent.input(field('Password'), { target: { value: 'pw' } });
    await fireEvent.click(within(bar() as HTMLElement).getByRole('button', { name: 'Save' }));
    const btn = server('Choose server');
    const alert = view.getByRole('alert');
    expect(alert).toHaveTextContent('Choose the mail server.');
    expect(btn).toHaveAttribute('aria-invalid', 'true');
    expect(btn).toHaveAttribute('aria-describedby', alert.id);
    await fireEvent.click(btn);
    await fireEvent.click(within(view.getByRole('listbox')).getAllByRole('option')[0] as HTMLElement);
    expect(view.queryByRole('alert')).toBeNull();
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
    await fireEvent.input(field('Username'), { target: { value: 'no reply' } });
    const input = field('Username');
    expect(input).toHaveAttribute('aria-invalid', 'true');
    const alert = view.getByRole('alert');
    expect(alert).toHaveTextContent('Enter a username without spaces, at most 254 characters.');
    expect(input).toHaveAttribute('aria-describedby', alert.id);
    expect(input.parentElement?.contains(alert)).toBe(false);

    await fireEvent.input(field('Username'), { target: { value: 'noreply@example.com' } });
    expect(view.queryByRole('alert')).toBeNull();
    await fireEvent.click(server('Example mail'));
    await fireEvent.click(within(view.getByRole('listbox')).getByRole('option', { name: /Other mail/ }));
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
      api: {
        'GET admin/system-mail': [{ status: 500, body: { error: 'internal' } }, { body: STORED }],
        'GET admin/stalwart/servers': [REGISTRY, REGISTRY],
      },
    });
    await vi.waitFor(() => expect(view.getByText('Couldn’t load the email settings')).toBeInTheDocument());
    expect(view.queryByRole('button', { name: 'Connection' })).toBeNull();
    await fireEvent.click(view.getByRole('button', { name: 'Try again' }));
    await ready();
    expect(view.getByText('Connected')).toBeInTheDocument();
  });
});
