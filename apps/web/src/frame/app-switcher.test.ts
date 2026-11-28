import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { violations } from '#lib/test/axe.ts';
import de from '../../messages/de.json';
import en from '../../messages/en.json';
import {
  APPS_URL,
  RoedyAppSwitcher,
  SESSION_URL,
  STALE_MS,
  TAG,
  parseApps,
  parseLinks,
  typeahead,
  vertical,
} from './app-switcher.ts';
import { RoedyAppSwitcher as FromFrame } from './frame.ts';
import { localeOf } from './messages.ts';
import { CSS } from './styles.ts';

const APPS = {
  apps: [
    {
      slug: 'mail',
      name: 'Mail',
      description: 'Email for the team',
      url: '/mail/',
      icon: '/api/v1/apps/mail/icon.svg?v=1',
      status: 'online',
    },
    {
      slug: 'contacts',
      name: 'Contacts',
      description: '',
      url: '/contacts/',
      icon: '/api/v1/apps/contacts/icon.svg?v=1',
      status: 'offline',
    },
  ],
};
const SESSION = {
  user: { role: 'admin' },
  hub: { name: 'Example Org', home: '/', account: '/account', admin: '/admin' },
};

type Reply = { status: number; body?: unknown };

/** Answers the hub's endpoints; the replies may change between calls. */
function mockHub(replies: { apps: Reply; session?: Reply }) {
  const fetch = vi.fn((url: string) => {
    const reply = url === APPS_URL ? replies.apps : url === SESSION_URL ? (replies.session ?? { status: 401 }) : null;
    if (!reply) return Promise.reject(new TypeError('unexpected request'));
    return Promise.resolve(
      new Response(reply.body === undefined ? null : JSON.stringify(reply.body), { status: reply.status }),
    );
  });
  vi.stubGlobal('fetch', fetch);
  return fetch;
}

async function mount(attributes: Record<string, string> = {}): Promise<RoedyAppSwitcher> {
  const element = document.createElement(TAG);
  for (const [name, value] of Object.entries(attributes)) element.setAttribute(name, value);
  document.body.append(element);
  await element.refresh();
  return element;
}

const shadow = (element: RoedyAppSwitcher) => element.shadowRoot!;
const toggle = (element: RoedyAppSwitcher) => shadow(element).querySelector<HTMLButtonElement>('.toggle')!;
const items = (element: RoedyAppSwitcher) => [...shadow(element).querySelectorAll<HTMLElement>('[role="menuitem"]')];
const focused = (element: RoedyAppSwitcher) => shadow(element).activeElement as HTMLElement | null;
const label = (item: HTMLElement | null | undefined) => item?.dataset.label;
const key = (element: RoedyAppSwitcher, name: string, init: KeyboardEventInit = {}) =>
  (focused(element) ?? toggle(element)).dispatchEvent(
    new KeyboardEvent('keydown', { key: name, bubbles: true, composed: true, cancelable: true, ...init }),
  );

beforeEach(() => {
  history.replaceState(null, '', '/mail/inbox?q=1#top');
});

afterEach(() => {
  document.body.replaceChildren();
  document.documentElement.removeAttribute('lang');
  vi.unstubAllGlobals();
  vi.useRealTimers();
});

describe('parsing', () => {
  it('keeps the apps with the needed fields and only paths on this origin', () => {
    const apps = parseApps({
      apps: [
        ...APPS.apps,
        { slug: 'x', name: 'X', url: '//evil.example/', icon: '/i.svg' },
        { slug: 'y', name: 'Y', url: 'javascript:alert(1)', icon: '/i.svg' },
        { slug: 'z', name: 'Z', url: '/z/', icon: '/\\evil.example/i.svg' },
        { name: 'No slug', url: '/n/', icon: '/i.svg' },
        null,
      ],
    });
    expect(apps?.map((app) => app.slug)).toEqual(['mail', 'contacts']);
    expect(parseApps({ apps: 'none' })).toBeNull();
    expect(parseApps(null)).toBeNull();
  });

  it('takes the hub links from the session, with defaults', () => {
    expect(parseLinks(SESSION)).toEqual({ home: '/', account: '/account', admin: '/admin' });
    expect(parseLinks({ hub: { home: 'https://evil.example/', admin: null } })).toEqual({
      home: '/',
      account: '/account',
      admin: null,
    });
    expect(parseLinks(null)).toEqual({ home: '/', account: '/account', admin: null });
  });

  it('picks German for every German tag, else English', () => {
    expect(localeOf('de')).toBe('de');
    expect(localeOf('de-AT')).toBe('de');
    expect(localeOf('DE_ch')).toBe('de');
    expect(localeOf('en-GB')).toBe('en');
    expect(localeOf('fr')).toBe('en');
    expect(localeOf('')).toBe('en');
    expect(localeOf(undefined)).toBe('en');
  });

  it('has every switcher text in English and German', () => {
    const keys = (messages: Record<string, string>) => Object.keys(messages).filter((k) => k.startsWith('switcher_'));
    expect(keys(de)).toEqual(keys(en));
    expect(keys(en).length).toBeGreaterThan(0);
  });
});

describe('frame', () => {
  it('defines the switcher when the frame is loaded', () => {
    expect(FromFrame).toBe(RoedyAppSwitcher);
    expect(customElements.get(TAG)).toBe(RoedyAppSwitcher);
  });

  it('takes the design tokens of the page where it sets them, else its own', () => {
    // Light fallbacks on `.root`, dark ones on `.root.dark`.
    expect(CSS).toContain('--_pop:var(--pop,#fffdfa)');
    expect(CSS).toContain('--_pop:var(--pop,#322b25)');
    expect(CSS).toContain('--_acc:var(--acc,#f5a06e)');
    expect(CSS).toMatch(/--_accsoft:var\(--accsoft,color-mix\(in oklab, var\(--_acc\) 26%, var\(--_surface\)\)\)/);
    // The rules use only the private names, never a page's token directly.
    const rules = CSS.replace(/--_[a-z0-9]+:var\(--[a-z0-9-]+,/g, '');
    expect(rules).not.toMatch(/var\(--(?!_)/);
  });
});

describe('navigation helpers', () => {
  /** Items at fixed positions: a grid of three columns, then full-width rows. */
  function laidOut(rects: [number, number, number, number][]): HTMLElement[] {
    return rects.map(([left, top, width, height], i) => {
      const item = document.createElement('a');
      item.dataset.label = `Item ${i}`;
      item.getBoundingClientRect = () => DOMRect.fromRect({ x: left, y: top, width, height });
      return item;
    });
  }

  it('moves by row in a grid and wraps around', () => {
    // Row 1: 0 1 2, row 2: 3, then two full rows 4 and 5.
    const items = laidOut([
      [0, 0, 90, 90],
      [100, 0, 90, 90],
      [200, 0, 90, 90],
      [0, 100, 90, 90],
      [0, 200, 290, 44],
      [0, 250, 290, 44],
    ]);
    expect(vertical(items, 1, 1)).toBe(items[3]);
    expect(vertical(items, 3, 1)).toBe(items[4]);
    expect(vertical(items, 4, 1)).toBe(items[5]);
    // Wrapping: the item of the top row closest to the full-width row's middle.
    expect(vertical(items, 5, 1)).toBe(items[1]);
    expect(vertical(items, 4, -1)).toBe(items[3]);
    expect(vertical(items, 2, -1)).toBe(items[5]);
    expect(vertical(items, -1, 1)).toBe(items[0]);
    expect(vertical(items, -1, -1)).toBe(items[5]);
  });

  it('moves one by one without layout', () => {
    const items = laidOut([
      [0, 0, 0, 0],
      [0, 0, 0, 0],
    ]);
    expect(vertical(items, 0, 1)).toBe(items[1]);
    expect(vertical(items, 1, 1)).toBe(items[0]);
    expect(vertical(items, 0, -1)).toBe(items[1]);
  });

  it('finds the next item by its first letter', () => {
    const items = ['Mail', 'Contacts', 'Home', 'Manage account'].map((name) => {
      const item = document.createElement('a');
      item.dataset.label = name;
      return item;
    });
    expect(typeahead(items, 0, 'm')).toBe(items[3]);
    expect(typeahead(items, 3, 'M')).toBe(items[0]);
    expect(typeahead(items, 0, 'x')).toBeUndefined();
  });
});

describe('<roedy-app-switcher>', () => {
  it('lists the apps and the hub links, the current one marked', async () => {
    mockHub({ apps: { status: 200, body: APPS }, session: { status: 200, body: SESSION } });
    const element = await mount();
    const button = toggle(element);
    expect(button).toHaveAttribute('aria-haspopup', 'menu');
    expect(button).toHaveAttribute('aria-expanded', 'false');
    expect(button).toHaveAccessibleName('Apps');

    button.click();
    expect(element.open).toBe(true);
    expect(button).toHaveAttribute('aria-expanded', 'true');
    const menu = shadow(element).querySelector('[role="menu"]')!;
    expect(menu).toHaveAccessibleName('Apps');
    expect(items(element).map(label)).toEqual(['Mail', 'Contacts', 'Home', 'Manage account', 'Administration']);
    const [mail, contacts, home, account, admin] = items(element);
    // The current app comes from the path (`/mail/…`).
    expect(mail).toHaveAttribute('aria-current', 'page');
    expect(mail).toHaveAttribute('href', '/mail/');
    expect(mail).toHaveAccessibleName('Mail');
    expect(mail).toHaveAttribute('title', 'Email for the team');
    expect(mail?.querySelector('img')).toHaveAttribute('src', '/api/v1/apps/mail/icon.svg?v=1');
    expect(contacts).not.toHaveAttribute('aria-current');
    expect(contacts).toHaveAccessibleName('Contacts');
    expect(contacts).toHaveAccessibleDescription('Offline');
    expect(home).toHaveAttribute('href', '/');
    expect(account).toHaveAttribute('href', '/account');
    expect(admin).toHaveAttribute('href', '/admin');
    // Focus on the first item; the items are reached with the arrow keys, not Tab.
    expect(focused(element)).toBe(mail);
    expect(items(element).every((item) => item.tabIndex === -1)).toBe(true);
  });

  it('marks the app named in `current` and leaves out administration for users', async () => {
    mockHub({
      apps: { status: 200, body: APPS },
      session: { status: 200, body: { ...SESSION, hub: { ...SESSION.hub, admin: null } } },
    });
    const element = await mount({ current: 'contacts' });
    toggle(element).click();
    expect(items(element).map(label)).toEqual(['Mail', 'Contacts', 'Home', 'Manage account']);
    expect(items(element)[1]).toHaveAttribute('aria-current', 'page');
    expect(items(element)[0]).not.toHaveAttribute('aria-current');
  });

  it('works with the keyboard (menu button pattern)', async () => {
    mockHub({ apps: { status: 200, body: APPS }, session: { status: 200, body: SESSION } });
    const element = await mount();
    const button = toggle(element);
    const toggles = vi.fn();
    element.addEventListener('toggle', toggles);

    button.focus();
    key(element, 'ArrowUp');
    expect(element.open).toBe(true);
    expect(label(focused(element))).toBe('Administration');
    key(element, 'ArrowDown');
    expect(label(focused(element))).toBe('Mail');
    key(element, 'ArrowRight');
    expect(label(focused(element))).toBe('Contacts');
    key(element, 'ArrowLeft');
    key(element, 'ArrowLeft');
    expect(label(focused(element))).toBe('Administration');
    key(element, 'Home');
    expect(label(focused(element))).toBe('Mail');
    key(element, 'End');
    expect(label(focused(element))).toBe('Administration');
    key(element, 'm');
    expect(label(focused(element))).toBe('Mail');
    key(element, 'm');
    expect(label(focused(element))).toBe('Manage account');

    // Space activates the item like Enter.
    const clicked = vi.fn((event: Event) => event.preventDefault());
    focused(element)!.addEventListener('click', clicked);
    key(element, ' ');
    expect(clicked).toHaveBeenCalledOnce();

    // Escape closes and returns focus to the button.
    key(element, 'Escape');
    expect(element.open).toBe(false);
    expect(button).toHaveAttribute('aria-expanded', 'false');
    expect(focused(element)).toBe(button);
    expect(toggles).toHaveBeenCalledTimes(2);

    // ArrowDown on the button opens at the first item; Tab closes.
    key(element, 'ArrowDown');
    expect(label(focused(element))).toBe('Mail');
    key(element, 'Tab');
    expect(element.open).toBe(false);
  });

  it('closes on a click outside, the button and close()', async () => {
    mockHub({ apps: { status: 200, body: APPS }, session: { status: 200, body: SESSION } });
    const element = await mount();
    const outside = document.createElement('p');
    document.body.append(outside);

    toggle(element).click();
    items(element)[0]!.dispatchEvent(new PointerEvent('pointerdown', { bubbles: true, composed: true }));
    expect(element.open).toBe(true);
    outside.dispatchEvent(new PointerEvent('pointerdown', { bubbles: true, composed: true }));
    expect(element.open).toBe(false);

    toggle(element).click();
    toggle(element).click();
    expect(element.open).toBe(false);

    toggle(element).click();
    element.close();
    expect(element.open).toBe(false);
  });

  it('offers to sign in when signed out, coming back to this page', async () => {
    mockHub({ apps: { status: 401, body: { error: 'unauthorized' } } });
    const element = await mount();
    toggle(element).click();
    const menu = shadow(element).querySelector('[role="menu"]')!;
    expect(menu).toHaveAccessibleDescription('You’re not signed in');
    expect(items(element).map(label)).toEqual(['Sign in', 'Home']);
    expect(items(element)[0]).toHaveAttribute('href', `/login?next=${encodeURIComponent('/mail/inbox?q=1#top')}`);
  });

  it('shows an error with a retry, and keeps shown apps when a refresh fails', async () => {
    const replies: { apps: Reply; session?: Reply } = { apps: { status: 503 } };
    const fetch = mockHub(replies);
    const element = await mount();
    toggle(element).click();
    expect(shadow(element).querySelector('[role="menu"]')).toHaveAccessibleDescription('Couldn’t load your apps');
    expect(items(element).map(label)).toEqual(['Try again', 'Home']);

    replies.apps = { status: 200, body: APPS };
    replies.session = { status: 200, body: SESSION };
    items(element)[0]!.click();
    expect(shadow(element).querySelector('[role="status"]')).toHaveTextContent('Loading apps …');
    await vi.waitFor(() => expect(items(element).map(label)).toContain('Mail'));
    expect(label(focused(element))).toBe('Mail');

    // Stale data is fetched again on opening; a failure keeps the apps.
    element.close();
    replies.apps = { status: 500 };
    vi.spyOn(Date, 'now').mockReturnValue(Date.now() + STALE_MS + 1);
    const calls = fetch.mock.calls.length;
    toggle(element).click();
    await vi.waitFor(() => expect(fetch.mock.calls.length).toBeGreaterThan(calls));
    await element.refresh();
    expect(items(element).map(label)).toContain('Mail');
  });

  it('says when there are no apps', async () => {
    mockHub({ apps: { status: 200, body: { apps: [] } }, session: { status: 200, body: SESSION } });
    const element = await mount();
    toggle(element).click();
    expect(shadow(element).querySelector('[role="menu"]')).toHaveAccessibleDescription('No apps yet');
    expect(items(element).map(label)).toEqual(['Home', 'Manage account', 'Administration']);
  });

  it('speaks the language of the page or its own lang', async () => {
    mockHub({ apps: { status: 200, body: APPS }, session: { status: 200, body: SESSION } });
    document.documentElement.lang = 'de-DE';
    const element = await mount();
    toggle(element).click();
    expect(items(element).map(label)).toEqual(['Mail', 'Contacts', 'Startseite', 'Konto verwalten', 'Administration']);
    expect(items(element)[1]).toHaveAccessibleDescription('Offline');
    expect(shadow(element).querySelector('.pop')).toHaveAttribute('lang', 'de');

    element.setAttribute('lang', 'en');
    expect(items(element).map(label)).toContain('Home');
    expect(toggle(element)).toHaveAccessibleName('Apps');
  });

  it('follows the system theme unless the app sets one', async () => {
    mockHub({ apps: { status: 200, body: APPS } });
    let dark = true;
    const listeners = new Set<() => void>();
    vi.stubGlobal('matchMedia', (query: string) => ({
      get matches() {
        return dark && query.includes('dark');
      },
      addEventListener: (_: string, listener: () => void) => listeners.add(listener),
      removeEventListener: (_: string, listener: () => void) => listeners.delete(listener),
    }));
    const element = await mount();
    const root = shadow(element).querySelector('.root')!;
    expect(root).toHaveClass('dark');
    dark = false;
    for (const listener of listeners) listener();
    expect(root).not.toHaveClass('dark');

    element.setAttribute('theme', 'dark');
    expect(root).toHaveClass('dark');
    element.setAttribute('theme', 'light');
    dark = true;
    for (const listener of listeners) listener();
    expect(root).not.toHaveClass('dark');

    element.remove();
    expect(listeners.size).toBe(0);
  });

  it('asks only the hub, on the same origin', async () => {
    const fetch = mockHub({ apps: { status: 200, body: APPS }, session: { status: 200, body: SESSION } });
    await mount();
    expect(fetch.mock.calls.map(([url]) => url).sort()).toEqual([APPS_URL, SESSION_URL].sort());
    for (const [, init] of fetch.mock.calls as unknown as [string, RequestInit][]) {
      expect(init).toMatchObject({ credentials: 'same-origin', cache: 'no-store' });
    }
  });

  it('has no axe violations, closed and open', async () => {
    mockHub({ apps: { status: 200, body: APPS }, session: { status: 200, body: SESSION } });
    const main = document.createElement('main');
    document.body.append(main);
    const element = document.createElement(TAG);
    main.append(element);
    await element.refresh();
    expect(await violations(main)).toEqual([]);
    toggle(element).click();
    expect(await violations(main)).toEqual([]);
  });
});
