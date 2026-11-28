/**
 * The app switcher of the hub's frame (`frame.ts`, served as
 * `GET /api/v1/frame.js`; docs/app-contract.md › App switcher). Its texts
 * are the `switcher_*` keys of `messages/{en,de}.json`.
 *
 * ```html
 * <script type="module" src="/api/v1/frame.js"></script>
 * <roedy-app-switcher></roedy-app-switcher>
 * ```
 *
 * A button with the apps the signed-in user can open (`GET /api/v1/apps`),
 * the current one marked, and the links into the hub (home page, account,
 * administration for administrators; `GET /api/v1/session`). Plain DOM in a
 * shadow root without dependencies, with a constructed style sheet and no
 * inline script or style, so that it works under an app's strict CSP
 * (`script-src 'self'`, no `style-src` needed). Only same-origin requests.
 *
 * Menu button pattern (WAI-ARIA APG): the button opens the menu with Enter,
 * Space or the arrow keys and focuses its first (↑: last) item; in the menu
 * the arrow keys move (↑/↓ by row, ←/→ one item), Home/End go to the ends,
 * a letter to the next item starting with it, Escape closes it and returns
 * focus to the button, Tab or a click outside closes it.
 *
 * Attributes: `current` (slug of the current app, default: first segment of
 * the path), `theme` (`light`, `dark`; default: the system's), `lang`
 * (default: the closest `lang` in the page, then the browser's language),
 * `align` (`end` (default) or `start`: which edge of the button the menu
 * lines up with). Property `open`, methods `close()` and `refresh()`, event
 * `toggle` (does not bubble) when the menu opens or closes.
 */
import { icon, type IconName } from './icons.ts';
import { MESSAGES, localeOf, type Locale, type Messages } from './messages.ts';
import { CSS } from './styles.ts';

export const TAG = 'roedy-app-switcher';
export const APPS_URL = '/api/v1/apps';
export const SESSION_URL = '/api/v1/session';
/** Data older than this is fetched again when the menu opens. */
export const STALE_MS = 30_000;
/** Distance of the menu from the button and from the edge of the window, in px. */
const GAP = 10;
const EDGE = 12;

/** An app of `GET /api/v1/apps` (contract § 6). */
export interface HubApp {
  slug: string;
  name: string;
  description: string;
  url: string;
  icon: string;
  status: string;
}

/** The hub's links of `GET /api/v1/session` (`hub`). */
export interface HubLinks {
  home: string;
  account: string;
  admin: string | null;
}

export const DEFAULT_LINKS: HubLinks = { home: '/', account: '/account', admin: null };

export type View =
  | { kind: 'loading' }
  | { kind: 'failed' }
  | { kind: 'signed-out' }
  | { kind: 'ready'; apps: HubApp[]; links: HubLinks };

/**
 * A path on this origin, else `null`: one leading `/`, no `\` and no control
 * characters – the URL parser drops tabs and line breaks, so `/\n/host`
 * would become `//host`, another origin.
 */
function localPath(value: unknown): string | null {
  return typeof value === 'string' &&
    /^\/(?![/\\])/.test(value) &&
    !value.includes('\\') &&
    // eslint-disable-next-line no-control-regex -- control characters are what it looks for
    !/[\u0000-\u001f\u007f]/.test(value)
    ? value
    : null;
}

/** The apps of a `GET /api/v1/apps` body; entries without the needed fields are left out. */
export function parseApps(body: unknown): HubApp[] | null {
  const list = (body as { apps?: unknown } | null)?.apps;
  if (!Array.isArray(list)) return null;
  return list.flatMap((entry: Record<string, unknown> | null) => {
    const url = localPath(entry?.url);
    const iconUrl = localPath(entry?.icon);
    if (typeof entry?.slug !== 'string' || typeof entry.name !== 'string' || !url || !iconUrl) return [];
    return [
      {
        slug: entry.slug,
        name: entry.name,
        description: typeof entry.description === 'string' ? entry.description : '',
        url,
        icon: iconUrl,
        status: typeof entry.status === 'string' ? entry.status : 'online',
      },
    ];
  });
}

/** The hub's links of a `GET /api/v1/session` body, with the defaults for what is missing. */
export function parseLinks(body: unknown): HubLinks {
  const hub = (body as { hub?: Record<string, unknown> } | null)?.hub;
  return {
    home: localPath(hub?.home) ?? DEFAULT_LINKS.home,
    account: localPath(hub?.account) ?? DEFAULT_LINKS.account,
    admin: localPath(hub?.admin),
  };
}

async function getJson(url: string): Promise<{ status: number; body: unknown }> {
  const response = await fetch(url, {
    credentials: 'same-origin',
    cache: 'no-store',
    headers: { Accept: 'application/json' },
  });
  const body: unknown = response.ok ? await response.json() : null;
  return { status: response.status, body };
}

/** Loads what the menu shows; on a network or server error `failed`. */
export async function loadView(): Promise<View> {
  try {
    const [apps, session] = await Promise.all([getJson(APPS_URL), getJson(SESSION_URL).catch(() => null)]);
    if (apps.status === 401) return { kind: 'signed-out' };
    const parsed = parseApps(apps.body);
    if (!parsed) return { kind: 'failed' };
    return { kind: 'ready', apps: parsed, links: parseLinks(session?.body) };
  } catch {
    return { kind: 'failed' };
  }
}

let sharedSheet: CSSStyleSheet | undefined;

/** The style sheet: constructed and shared by all switchers, `<style>` only where the browser cannot adopt one. */
function adoptStyles(root: ShadowRoot) {
  if ('adoptedStyleSheets' in root && typeof CSSStyleSheet === 'function' && 'replaceSync' in CSSStyleSheet.prototype) {
    if (!sharedSheet) {
      sharedSheet = new CSSStyleSheet();
      sharedSheet.replaceSync(CSS);
    }
    root.adoptedStyleSheets = [sharedSheet];
  } else {
    const style = document.createElement('style');
    style.textContent = CSS;
    root.append(style);
  }
}

type Focus = 'first' | 'last' | 'keep';

export class RoedyAppSwitcher extends HTMLElement {
  static readonly observedAttributes = ['theme', 'lang', 'current', 'align'];

  readonly #shadow: ShadowRoot;
  readonly #root: HTMLDivElement;
  readonly #toggle: HTMLButtonElement;
  readonly #pop: HTMLDivElement;
  readonly #popover: boolean;
  #view: View = { kind: 'loading' };
  #loadedAt = 0;
  #loading: Promise<void> | null = null;
  #open = false;
  /** After "Try again": focus on the first item once the result is there. */
  #retried = false;
  #scheme: MediaQueryList | null = null;

  constructor() {
    super();
    this.#shadow = this.attachShadow({ mode: 'open' });
    adoptStyles(this.#shadow);
    const doc = this.ownerDocument;
    this.#root = doc.createElement('div');
    this.#root.className = 'root';
    this.#toggle = doc.createElement('button');
    this.#toggle.type = 'button';
    this.#toggle.className = 'toggle';
    this.#toggle.part.add('button');
    this.#toggle.setAttribute('aria-haspopup', 'menu');
    this.#toggle.setAttribute('aria-expanded', 'false');
    this.#toggle.setAttribute('aria-controls', 'menu');
    this.#toggle.append(icon(doc, 'grid'));
    this.#pop = doc.createElement('div');
    this.#pop.className = 'pop';
    this.#pop.part.add('menu');
    // Top layer where the browser has it: never cut off by the app's header.
    this.#popover = typeof this.#pop.showPopover === 'function';
    if (this.#popover) this.#pop.popover = 'manual';
    else this.#pop.hidden = true;
    this.#root.append(this.#toggle, this.#pop);
    this.#shadow.append(this.#root);

    this.#toggle.addEventListener('click', () => {
      if (this.#open) this.close();
      else this.#show('first');
    });
    this.#toggle.addEventListener('keydown', (event) => {
      if (event.key !== 'ArrowDown' && event.key !== 'ArrowUp') return;
      event.preventDefault();
      this.#show(event.key === 'ArrowDown' ? 'first' : 'last');
    });
    this.#pop.addEventListener('keydown', (event) => this.#onMenuKey(event));
    this.#pop.addEventListener('focusout', (event) => {
      // Only when focus moves to something outside (Tab); a click on the menu's background moves it nowhere.
      const to = event.relatedTarget;
      if (this.#open && to instanceof Node && !this.#shadow.contains(to) && !this.contains(to)) this.close();
    });
  }

  /** Whether the menu is open. */
  get open(): boolean {
    return this.#open;
  }

  connectedCallback() {
    this.#scheme = typeof matchMedia === 'function' ? matchMedia('(prefers-color-scheme: dark)') : null;
    this.#scheme?.addEventListener('change', this.#applyTheme);
    this.#applyTheme();
    this.#label();
    if (!this.#loadedAt) void this.refresh();
  }

  disconnectedCallback() {
    this.#scheme?.removeEventListener('change', this.#applyTheme);
    this.close();
  }

  attributeChangedCallback() {
    this.#applyTheme();
    this.#label();
    if (this.#open) {
      this.#render('keep');
      this.#place();
    }
  }

  /** Fetches the apps and the hub's links again. */
  refresh(): Promise<void> {
    this.#loading ??= loadView().then((view) => {
      // A failed refresh keeps the apps already shown.
      if (view.kind !== 'failed' || this.#view.kind !== 'ready') this.#view = view;
      this.#loadedAt = Date.now();
      this.#loading = null;
      if (this.#open) {
        this.#render(this.#retried ? 'first' : 'keep');
        this.#place();
      }
      this.#retried = false;
    });
    return this.#loading;
  }

  /** Closes the menu; focus stays where it is (Escape returns it to the button itself). */
  close() {
    if (!this.#open) return;
    this.#open = false;
    // Removed from the page, the browser has already hidden it.
    if (this.#popover) {
      if (this.#pop.matches(':popover-open')) this.#pop.hidePopover();
    } else this.#pop.hidden = true;
    this.#toggle.setAttribute('aria-expanded', 'false');
    removeEventListener('resize', this.#place);
    removeEventListener('scroll', this.#place, true);
    this.ownerDocument.removeEventListener('pointerdown', this.#onOutside, true);
    this.dispatchEvent(new Event('toggle'));
  }

  #show(focus: Focus) {
    if (this.#open) return;
    this.#open = true;
    this.#render('none');
    if (this.#popover) this.#pop.showPopover();
    else this.#pop.hidden = false;
    this.#toggle.setAttribute('aria-expanded', 'true');
    this.#place();
    this.#focus(focus);
    addEventListener('resize', this.#place, { passive: true });
    addEventListener('scroll', this.#place, { passive: true, capture: true });
    this.ownerDocument.addEventListener('pointerdown', this.#onOutside, true);
    this.dispatchEvent(new Event('toggle'));
    if (!this.#loading && Date.now() - this.#loadedAt > STALE_MS) void this.refresh();
  }

  readonly #onOutside = (event: Event) => {
    if (!event.composedPath().includes(this)) this.close();
  };

  readonly #applyTheme = () => {
    const theme = this.getAttribute('theme');
    const dark = theme === 'dark' || (theme !== 'light' && !!this.#scheme?.matches);
    this.#root.classList.toggle('dark', dark);
  };

  #locale(): Locale {
    const tag = this.closest('[lang]')?.getAttribute('lang') || this.ownerDocument.defaultView?.navigator.language;
    return localeOf(tag);
  }

  #messages(): Messages {
    return MESSAGES[this.#locale()];
  }

  #label() {
    const text = this.#messages().toggle;
    this.#toggle.setAttribute('aria-label', text);
    this.#toggle.title = text;
  }

  #current(): string {
    return this.getAttribute('current') ?? this.ownerDocument.location.pathname.split('/')[1] ?? '';
  }

  #items(): HTMLElement[] {
    return [...this.#pop.querySelectorAll<HTMLElement>('[role="menuitem"]')];
  }

  /** Builds the menu for the current view; `keep` puts focus back on the same item if it is still there. */
  #render(focus: Focus | 'none') {
    const doc = this.ownerDocument;
    const t = this.#messages();
    const view = this.#view;
    const active = this.#shadow.activeElement;
    const activeKey = active instanceof HTMLElement && this.#pop.contains(active) ? active.dataset.key : undefined;

    this.#pop.lang = this.#locale();
    const title = doc.createElement('div');
    title.className = 'title';
    title.id = 'title';
    title.textContent = t.toggle;
    const menu = doc.createElement('div');
    menu.setAttribute('role', 'menu');
    menu.id = 'menu';
    menu.setAttribute('aria-labelledby', 'title');
    const parts: Node[] = [title];

    const message =
      view.kind === 'loading'
        ? t.loading
        : view.kind === 'failed'
          ? t.failed
          : view.kind === 'signed-out'
            ? t.signedOut
            : view.apps.length === 0
              ? t.empty
              : null;
    if (message) {
      const text = doc.createElement('p');
      text.className = 'msg';
      text.id = 'msg';
      text.textContent = message;
      if (view.kind === 'loading') text.setAttribute('role', 'status');
      menu.setAttribute('aria-describedby', 'msg');
      parts.push(text);
    }

    if (view.kind === 'ready' && view.apps.length > 0) {
      const apps = doc.createElement('div');
      apps.className = 'apps';
      apps.setAttribute('role', 'none');
      const current = this.#current();
      view.apps.forEach((app, index) => apps.append(this.#appItem(app, index, app.slug === current, t)));
      const sep = doc.createElement('div');
      sep.className = 'sep';
      sep.setAttribute('role', 'separator');
      menu.append(apps, sep);
    }

    const links = view.kind === 'ready' ? view.links : DEFAULT_LINKS;
    if (view.kind === 'failed') menu.append(this.#row('retry', t.retry, 'retry', null));
    if (view.kind === 'signed-out') {
      const here = doc.location;
      const next = encodeURIComponent(here.pathname + here.search + here.hash);
      menu.append(this.#row('sign-in', t.signIn, 'signIn', `/login?next=${next}`));
    }
    menu.append(this.#row('home', t.home, 'home', links.home));
    if (view.kind === 'ready') {
      menu.append(this.#row('account', t.account, 'account', links.account));
      if (links.admin) menu.append(this.#row('admin', t.admin, 'admin', links.admin));
    }
    parts.push(menu);
    this.#pop.replaceChildren(...parts);

    if (focus === 'keep' && activeKey !== undefined) {
      const same = this.#items().find((item) => item.dataset.key === activeKey);
      if (same) same.focus();
      else this.#focus('first');
    } else if (focus === 'first' || focus === 'last') {
      this.#focus(focus);
    }
  }

  #appItem(app: HubApp, index: number, current: boolean, t: Messages): HTMLAnchorElement {
    const doc = this.ownerDocument;
    const item = doc.createElement('a');
    item.className = 'app';
    item.href = app.url;
    item.setAttribute('role', 'menuitem');
    item.tabIndex = -1;
    item.dataset.key = `app:${app.slug}`;
    item.dataset.label = app.name;
    item.setAttribute('aria-labelledby', `name-${index}`);
    const described: string[] = [];
    if (current) item.setAttribute('aria-current', 'page');
    // The grid has no room for the description: a tooltip, and the item's description for screen readers.
    if (app.description) item.title = app.description;

    const tile = doc.createElement('span');
    tile.className = 'tile';
    tile.setAttribute('aria-hidden', 'true');
    const img = doc.createElement('img');
    img.src = app.icon;
    img.alt = '';
    img.width = 40;
    img.height = 40;
    img.decoding = 'async';
    tile.append(img);

    const text = doc.createElement('span');
    text.className = 'text';
    const name = doc.createElement('span');
    name.className = 'name';
    name.id = `name-${index}`;
    name.textContent = app.name;
    text.append(name);
    if (app.status === 'offline') {
      item.classList.add('offline');
      const badge = doc.createElement('span');
      badge.className = 'badge';
      badge.id = `off-${index}`;
      badge.append(icon(doc, 'offline'), doc.createTextNode(t.offline));
      text.append(badge);
      described.push(badge.id);
    }
    item.append(tile, text);
    if (described.length) item.setAttribute('aria-describedby', described.join(' '));
    return item;
  }

  #row(key: string, label: string, glyph: IconName, href: string | null): HTMLElement {
    const doc = this.ownerDocument;
    const row = href === null ? doc.createElement('button') : doc.createElement('a');
    if (row instanceof HTMLAnchorElement && href !== null) row.href = href;
    if (row instanceof HTMLButtonElement) {
      row.type = 'button';
      row.addEventListener('click', () => {
        this.#view = { kind: 'loading' };
        this.#retried = true;
        this.#render('first');
        void this.refresh();
      });
    }
    row.className = 'row';
    row.setAttribute('role', 'menuitem');
    row.tabIndex = -1;
    row.dataset.key = key;
    row.dataset.label = label;
    row.append(icon(doc, glyph), doc.createTextNode(label));
    return row;
  }

  #focus(which: Focus) {
    const items = this.#items();
    const target = which === 'last' ? items.at(-1) : items[0];
    target?.focus();
  }

  #onMenuKey(event: KeyboardEvent) {
    if (event.altKey || event.ctrlKey || event.metaKey) return;
    const items = this.#items();
    const active = this.#shadow.activeElement;
    const index = items.findIndex((item) => item === active);
    let next: HTMLElement | undefined;
    switch (event.key) {
      case 'Escape':
        event.preventDefault();
        event.stopPropagation();
        this.close();
        this.#toggle.focus();
        return;
      case 'Tab':
        this.close();
        return;
      case 'ArrowRight':
        next = items[(index + 1) % items.length];
        break;
      case 'ArrowLeft':
        next = items[(index - 1 + items.length) % items.length];
        break;
      case 'ArrowDown':
      case 'ArrowUp':
        next = vertical(items, index, event.key === 'ArrowDown' ? 1 : -1);
        break;
      case 'Home':
        next = items[0];
        break;
      case 'End':
        next = items.at(-1);
        break;
      case ' ':
        // Space activates an item like Enter (APG), links included.
        if (index >= 0) {
          event.preventDefault();
          items[index]?.click();
        }
        return;
      default:
        if (event.key.length === 1 && event.key.trim()) next = typeahead(items, index, event.key);
        if (!next) return;
    }
    event.preventDefault();
    next?.focus();
  }

  /** Places the menu at the button: below it, or above when there is more room there. */
  readonly #place = () => {
    if (!this.#open) return;
    const view = this.ownerDocument.defaultView;
    if (!view) return;
    const button = this.#toggle.getBoundingClientRect();
    const width = this.#pop.offsetWidth;
    const viewWidth = this.ownerDocument.documentElement.clientWidth || view.innerWidth;
    const viewHeight = view.innerHeight;
    const start = this.getAttribute('align') === 'start';
    const left = Math.max(EDGE, Math.min(start ? button.left : button.right - width, viewWidth - width - EDGE));
    const below = viewHeight - button.bottom - GAP - EDGE;
    const above = button.top - GAP - EDGE;
    const up = this.#pop.scrollHeight > below && above > below;
    const style = this.#pop.style;
    style.setProperty('left', `${Math.round(left)}px`);
    style.setProperty('max-height', `${Math.max(0, Math.floor(up ? above : below))}px`);
    if (up) {
      style.setProperty('top', 'auto');
      style.setProperty('bottom', `${Math.round(viewHeight - button.top + GAP)}px`);
    } else {
      style.setProperty('top', `${Math.round(button.bottom + GAP)}px`);
      style.setProperty('bottom', 'auto');
    }
    style.setProperty('transform-origin', `${up ? 'bottom' : 'top'} ${start ? 'left' : 'right'}`);
  };
}

/**
 * The item in the next row up or down (by position on screen) closest to the
 * current one horizontally, wrapping around at the ends; in one column (or
 * without layout, as in tests) simply the next or previous item.
 */
export function vertical(items: HTMLElement[], index: number, step: 1 | -1): HTMLElement | undefined {
  const linear = () => items[(index + step + items.length) % items.length];
  const from = items[index];
  if (!from) return step === 1 ? items[0] : items.at(-1);
  const rect = from.getBoundingClientRect();
  if (rect.height === 0) return linear();
  const center = rect.left + rect.width / 2;
  const rows = items.map((item) => item.getBoundingClientRect());
  const candidates = rows
    .map((r, i) => ({ r, i }))
    .filter(({ r }) => (step === 1 ? r.top >= rect.bottom - 1 : r.bottom <= rect.top + 1));
  let pool = candidates;
  if (pool.length === 0) {
    // Wrap: the first row from the top (↓) or bottom (↑).
    pool = rows.map((r, i) => ({ r, i }));
  }
  const edge = step === 1 ? Math.min(...pool.map(({ r }) => r.top)) : Math.max(...pool.map(({ r }) => r.bottom));
  const row = pool.filter(({ r }) => (step === 1 ? Math.abs(r.top - edge) < 1 : Math.abs(r.bottom - edge) < 1));
  const best = row.reduce((a, b) =>
    Math.abs(a.r.left + a.r.width / 2 - center) <= Math.abs(b.r.left + b.r.width / 2 - center) ? a : b,
  );
  return items[best.i];
}

/** The next item after `index` whose label starts with `key`, wrapping around. */
export function typeahead(items: HTMLElement[], index: number, key: string): HTMLElement | undefined {
  const char = key.toLocaleLowerCase();
  for (let step = 1; step <= items.length; step++) {
    const item = items[(index + step) % items.length];
    if (item?.dataset.label?.toLocaleLowerCase().startsWith(char)) return item;
  }
  return undefined;
}

if (!customElements.get(TAG)) customElements.define(TAG, RoedyAppSwitcher);

declare global {
  interface HTMLElementTagNameMap {
    [TAG]: RoedyAppSwitcher;
  }
}
