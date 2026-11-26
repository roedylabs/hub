// Taken over from Roedy Mail (`apps/web/src/lib/components/ErrorView.test.ts`).
import { fireEvent, render } from '@testing-library/svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { BRAND0, session } from '#lib/state/session.svelte.ts';
import { ui } from '#lib/state/ui.svelte.ts';
import ErrorView from './ErrorView.svelte';

const nav = vi.hoisted(() => ({ assign: vi.fn(), reload: vi.fn() }));

afterEach(() => {
  vi.unstubAllGlobals();
  vi.useRealTimers();
  vi.restoreAllMocks();
  nav.assign.mockClear();
  nav.reload.mockClear();
  session.config = null;
  ui.navigated = false;
  ui.reset();
});

/** `location` with recorded `assign` and `reload`. */
function stubLocation() {
  vi.stubGlobal('location', { ...window.location, assign: nav.assign, reload: nav.reload });
}

function withOrgEmail(email: string | null) {
  session.config = {
    name: 'Roedy Hub',
    branding: BRAND0,
    org: { name: null, website: null, help: null, email, imprint: null, privacy: null, terms: null, source: null },
    setup_required: false,
  };
}

describe('error page', () => {
  it('shows 404 with "Go to overview" and "Back" in its own main landmark', async () => {
    stubLocation();
    const view = render(ErrorView, { code: '404' });
    expect(view.container.querySelector('main')).not.toBeNull();
    expect(view.getByText('Error 404')).toBeInTheDocument();
    expect(view.getByRole('heading', { level: 1, name: 'Page not found' })).toBeInTheDocument();
    expect(view.getByText('The URL is wrong or the page has moved.')).toBeInTheDocument();
    expect(view.queryByRole('button', { name: 'Try again' })).toBeNull();
    expect(document.title).toBe('Page not found – Roedy Hub');
    // A full page load: the address may belong to an app behind the gateway.
    await fireEvent.click(view.getByRole('button', { name: 'Go to overview' }));
    expect(nav.assign).toHaveBeenLastCalledWith('/');
    // Without history in the hub "Back" leads to the overview, otherwise back in the history.
    await fireEvent.click(view.getByRole('button', { name: 'Back' }));
    expect(nav.assign).toHaveBeenCalledTimes(2);
    const back = vi.spyOn(history, 'back').mockImplementation(() => {});
    ui.navigated = true;
    await fireEvent.click(view.getByRole('button', { name: 'Back' }));
    expect(back).toHaveBeenCalledOnce();
    // The Navigation API knows exactly: without a previous page you stay in the hub.
    vi.stubGlobal('navigation', { canGoBack: false });
    await fireEvent.click(view.getByRole('button', { name: 'Back' }));
    expect(back).toHaveBeenCalledOnce();
    expect(nav.assign).toHaveBeenCalledTimes(3);
  });

  it('shows the gateway’s 403 without "Try again"', () => {
    const view = render(ErrorView, { code: '403' });
    expect(view.getByText('Error 403')).toBeInTheDocument();
    expect(view.getByRole('heading', { name: 'Access denied' })).toBeInTheDocument();
    expect(view.queryByRole('button', { name: 'Try again' })).toBeNull();
    expect(view.getByRole('button', { name: 'Back' })).toBeInTheDocument();
  });

  it.each([
    ['502', 'App not reachable', 'The app isn’t responding right now. Try again in a moment.'],
    ['503', 'App not ready yet', 'This app is still being set up. Try again later or contact your administrator.'],
    ['504', 'The app took too long', 'The app didn’t answer in time. Try again.'],
  ] as const)('shows the gateway’s %s with "Try again", reloading the page', async (code, title, text) => {
    vi.useFakeTimers();
    stubLocation();
    const view = render(ErrorView, { code });
    expect(view.getByText(`Error ${code}`)).toBeInTheDocument();
    expect(view.getByRole('heading', { name: title })).toBeInTheDocument();
    expect(view.getByText(text)).toBeInTheDocument();
    await fireEvent.click(view.getByRole('button', { name: 'Try again' }));
    await vi.advanceTimersByTimeAsync(900);
    expect(nav.reload).toHaveBeenCalledOnce();
  });

  it('retries on 500 and shows "Loading" meanwhile', async () => {
    vi.useFakeTimers();
    const retry = vi.fn(() => Promise.resolve());
    const view = render(ErrorView, { code: '500', retry });
    const button = view.getByRole('button', { name: 'Try again' });
    expect(button).toHaveAttribute('aria-disabled', 'false');
    await fireEvent.click(button);
    expect(view.getByRole('button', { name: 'Loading' })).toHaveAttribute('aria-disabled', 'true');
    await fireEvent.click(view.getByRole('button', { name: 'Loading' }));
    await vi.advanceTimersByTimeAsync(900);
    expect(retry).toHaveBeenCalledOnce();
    expect(view.getByRole('button', { name: 'Try again' })).toBeInTheDocument();
    expect(view.getByRole('button', { name: 'Go to overview' })).toBeInTheDocument();
  });

  it('shows "No connection" offline', () => {
    const view = render(ErrorView, { code: 'offline' });
    expect(view.getByRole('heading', { name: 'No connection' })).toBeInTheDocument();
    expect(view.getByRole('button', { name: 'Try again' })).toBeInTheDocument();
  });

  it('offers support only with an organization address', () => {
    withOrgEmail('support@example.com');
    const view = render(ErrorView, { code: '500' });
    expect(view.getByText('Still having trouble?')).toBeInTheDocument();
    expect(view.getByRole('link', { name: 'Contact support' })).toHaveAttribute(
      'href',
      'mailto:support@example.com?subject=Error%20500%20in%20Roedy%20Hub',
    );
    view.unmount();
    withOrgEmail(null);
    const plain = render(ErrorView, { code: '500' });
    expect(plain.queryByRole('link', { name: 'Contact support' })).toBeNull();
  });
});
