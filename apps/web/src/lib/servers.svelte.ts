/**
 * Administration › Stalwart servers against the hub's registry API: list from
 * `GET admin/stalwart/servers` with redirect URI, server of the system mail
 * and linked accounts, policy "Allow other servers"
 * (`GET`/`PUT admin/stalwart/policy`), "Test connection"
 * (`POST admin/stalwart/servers/:id/test`) with a diagnosis per row, removal
 * after confirmation and the dialog for adding and editing.
 *
 * The OAuth client: "Automatic" registers it by DCR with an administrator's
 * credentials at the server (`POST admin/stalwart/servers/:id/register-client`)
 * – for a saved server, so the dialog saves first (`POST`/`PATCH`) and
 * registers afterwards; the secret goes straight into the saved server. If the
 * registration fails, the server stays saved without a new client, the dialog
 * now edits it and shows the error at the credentials. Without credentials
 * "Automatic" keeps the stored client. "Manual" sends client ID and secret.
 * After saving (and registering) the dialog closes and the server is tested.
 *
 * The registration names the address the dialog saved (`url`): the hub sends
 * the credentials only there and answers `server_changed` if another
 * administrator moved the server meanwhile.
 *
 * Errors show the error toast; diagnosis and registration texts depend on the
 * reason (`#lib/diagnosis.ts`). 429 of the hub ("Too many attempts.") locks the
 * registration for every server, a throttling Stalwart (`throttled` with
 * `retry_after`) only for its own, each with a countdown – also when the
 * dialog that registered has closed meanwhile.
 *
 * Taken over from Roedy Mail (`apps/web/src/lib/servers.svelte.ts`) with the
 * hub's registration of a saved server instead of a pending one before
 * saving, without the list kept for the session, the removal with undo (the
 * hub's toasts only inform), the CSP reload (the hub's pages do not talk to
 * Stalwart) and the command palette.
 */
import { SvelteMap, SvelteSet } from 'svelte/reactivity';
import {
  ago,
  api,
  ApiError,
  clockText,
  failText,
  fieldsOf,
  type IconName,
  isApiError,
  type KeyInput,
  loadFailText,
  Loading,
  overlayClose,
  type OverlayEvent,
  retryAfter,
  ui,
  waitText,
} from '@roedy/ui';
import { copyText } from '#lib/clipboard.svelte.ts';
import { Countdown } from '#lib/countdown.svelte.ts';
import { credBad } from '#lib/credentials.ts';
import {
  adminRejected,
  diagAction,
  diagColor,
  diagIcon,
  diagText,
  registrationText,
  throttleText,
  type DiagAction,
  type DiagLine,
} from '#lib/diagnosis.ts';
import { m } from '#lib/paraglide/messages.js';
import { session } from '#lib/state/session.svelte.ts';
import type { AdminServer, AdminServers, DiagReasons, DiagStatus, TestReply } from '#lib/stalwart.ts';
import { MAX_DOMAINS, domainsOf, hostOf, httpOther, isDomain, localHttp } from '#lib/url.ts';

export type { AdminServer, DiagStatus } from '#lib/stalwart.ts';
export type { KeyInput } from '@roedy/ui';
export type ListStatus = 'loading' | 'ready' | 'failed';
export type OaMode = 'auto' | 'manual';
/** Lock of the registration: by the hub for every server, or by one throttling server. */
export type RegLock = 'hub' | `srv:${string}`;
/** Sections of the page. */
export type Section = 'list' | 'policy';

/** Errors in the dialog per field; `adm` below the credentials for the registration. */
interface FormErrors {
  label?: string | undefined;
  url?: string | undefined;
  dom?: string | undefined;
  adm?: string | undefined;
  /** Client ID or secret with control characters or over 1024 bytes, or a secret without a client ID. */
  cred?: string | undefined;
}

/** Dialog "Add server" or "Edit server". */
export interface ServerForm {
  /** Counts up per opened dialog: responses only apply to their dialog. */
  seq: number;
  /** Edited (or, after a failed registration, just saved) server; `null` when adding. */
  id: string | null;
  label: string;
  url: string;
  domains: string[];
  /** Text in the field, not yet a chip. */
  domIn: string;
  oa: OaMode;
  cid: string;
  csec: string;
  showSec: boolean;
  /** Stored client ID and address; a stored secret belongs only to them. */
  savedCid: string;
  savedUrl: string;
  /** Linked accounts of the edited server; a new client requires them to sign in again. */
  links: number;
  hasSec: boolean;
  /** "Register again" for a server with a client: the credentials show instead of the client. */
  again: boolean;
  au: string;
  ap: string;
  /** Text of a failed registration below the credentials. */
  aErr: string | null;
  /** The last save kept the server, but its registration failed. */
  regFailed: boolean;
  err: FormErrors;
  /** Saving or registering running. */
  saving: boolean;
  /** Registration running (after saving). */
  registering: boolean;
}

/** Address: `https://` with a dot in the host; the scheme in any case like the backend. */
const HTTPS = /^https:\/\/[^\s/]+\.[^\s/]+/i;
const SEP = /[\s,;]+/;

/** Maximum length of the label like in the backend (`MAX_LABEL`). */
export const MAX_LABEL = 100;

/** Fixed green for "OK". */
const OK_COLOR = diagColor('ok');

/** Address from the field: without whitespace and trailing `/`. */
const trimUrl = (url: string) => url.trim().replace(/\/+$/, '');

/** Append domains: lower case, without a leading `@`, duplicates only once. */
function addDomains(list: readonly string[], raw: string): string[] {
  const out = [...list];
  for (const d of domainsOf(raw)) if (!out.includes(d)) out.push(d);
  return out;
}

/** Error of the address when saving. */
function urlError(url: string): string | undefined {
  if (httpOther(url)) return m.srv_http_other();
  if (!HTTPS.test(url) && !localHttp(url)) return m.srv_err_url();
  return undefined;
}

/** Credentials in the fields: both, none, or only one (an error). */
function credentials(d: ServerForm): 'both' | 'none' | 'half' {
  const user = !!d.au.trim();
  const pass = !!d.ap;
  return user && pass ? 'both' : user || pass ? 'half' : 'none';
}

/** "Automatic" shows the credentials: no client yet, "Register again", or a new address. */
function showsCredentials(d: ServerForm): boolean {
  return !d.savedCid || d.again || trimUrl(d.url) !== d.savedUrl;
}

/** The dialog registers on saving: "Automatic" with the credentials shown and entered. */
function registers(d: ServerForm): boolean {
  return d.oa === 'auto' && showsCredentials(d) && credentials(d) === 'both';
}

export class ServersPage {
  rows = $state<AdminServer[]>([]);
  status = $state<ListStatus>('loading');
  /** Reason the first load failed (text of the card). */
  failure = $state('');
  /** Skeleton only after the skeleton delay. */
  readonly loading = new Loading();
  redirectUri = $state<string | null>(null);
  systemServer = $state<string | null>(null);
  /** "Users can connect accounts on other servers". */
  allowOwn = $state(false);
  /** Server whose test is running; only one at a time. */
  testing = $state<string | null>(null);
  /** Expanded diagnoses. */
  readonly diag = new SvelteSet<string>();
  /** Reasons of the last test per server (only from the response of "Test connection"). */
  readonly reasons = new SvelteMap<string, DiagReasons>();
  /**
   * Locks of the registration: `hub` after 429 of the hub ("Too many attempts.", for every server), `srv:<id>` after
   * `throttled` of that Stalwart server ("Too many requests.", only for it).
   */
  readonly regLock = new Countdown<RegLock>();
  /** Wait time after a throttled test per server (`retry_after`); only for the diagnosis text. */
  readonly testWait = new Countdown<string>();
  /** Seconds tick for it: the countdown clock ticks every 250 ms, the diagnosis needs only whole seconds. */
  #testSec = $derived(Math.floor(this.testWait.now / 1000));
  dialog = $state<ServerForm | null>(null);

  /** Redirect URI for Stalwart; until the list is loaded that of this browser. */
  redirect = $derived(this.redirectUri ?? `${location.origin}/oauth/callback`);
  /** Host of the hub for which the diagnosis checks CORS. */
  appHost = $derived(hostOf(this.redirect));

  #seq = 0;
  #formSeq = 0;
  /** Local changes per server; a test result only applies without newer ones. */
  #edits: Record<string, number> = {};
  /** Stored servers that are tested after the running test. */
  #queue: string[] = [];
  #policySeq = 0;
  /** Policy as the server last confirmed it (read or written); a failed toggle returns to it. */
  #policyConfirmed = false;
  /** Opened or closed diagnoses by the user per server: a failed test then leaves the diagnosis as it is. */
  #diagToggles: Record<string, number> = {};
  /** Servers removed on this page: a list loaded before the removal does not bring them back. */
  readonly #removed = new SvelteSet<string>();
  /** Toggles of the policy, one after the other, so that the last chosen state arrives last. */
  #policyWrites: Promise<unknown> = Promise.resolve();
  #disposed = false;

  /** Loads list and policy; after the first load the list stays as it is on an error. */
  async load(): Promise<void> {
    const seq = ++this.#seq;
    const policySeq = this.#policySeq;
    try {
      const [list, policy] = await this.loading.run(
        Promise.all([
          api<AdminServers>('GET', 'admin/stalwart/servers'),
          // After toggles still on their way: otherwise the response could be older than a confirmed one.
          this.#policyWrites
            .catch(() => {})
            .then(() => api<{ allow_custom_servers: boolean }>('GET', 'admin/stalwart/policy')),
        ]),
      );
      if (seq !== this.#seq || this.#disposed) return;
      // Reasons only apply to the test they come from (another one may have retested in the meantime).
      for (const id of [...this.reasons.keys()]) {
        const now = list.servers.find((r) => r.id === id);
        if (!now || now.tested_at !== this.rows.find((r) => r.id === id)?.tested_at) this.reasons.delete(id);
      }
      this.rows = list.servers.filter((s) => !this.#removed.has(s.id));
      this.redirectUri = list.redirect_uri;
      this.systemServer = list.system_mail_server;
      this.#policyConfirmed = policy.allow_custom_servers;
      // A toggle during loading applies, not the older state.
      if (policySeq === this.#policySeq) this.allowOwn = policy.allow_custom_servers;
      this.status = 'ready';
      const ids = list.servers.map((s) => s.id);
      for (const id of this.diag) if (!ids.includes(id) || this.#removed.has(id)) this.diag.delete(id);
    } catch (error) {
      if (seq !== this.#seq || this.#disposed) return;
      // Without a session or role: reload the account, the layout then shows the sign-in or "Access denied".
      if (error instanceof ApiError && (error.status === 401 || error.status === 403)) {
        void session.refreshUser().catch(() => {});
      }
      if (this.status !== 'ready') {
        this.failure = loadFailText(error);
        this.status = 'failed';
      } else {
        ui.toasts.error(failText(error, m.srv_err_list()));
      }
    }
  }

  /** "Try again" of the card: skeleton again until the response. */
  async retry(): Promise<void> {
    this.status = 'loading';
    await this.load();
  }

  /** Leave the page: late responses change nothing any more. */
  dispose(): void {
    this.#disposed = true;
    this.regLock.stop();
    this.testWait.stop();
  }

  /** Remaining time after a throttled test in seconds; changes at most once per second. */
  testLeft(id: string): number {
    const end = this.testWait.until(id);
    return end === null ? 0 : Math.max(0, Math.ceil((end - this.#testSec * 1000) / 1000));
  }

  /** Whether the registration is locked for this server: lock by the hub or throttling by exactly this server. */
  regBlocked(id: string | null): boolean {
    return this.regLock.active('hub') || (id !== null && this.regLock.active(`srv:${id}`));
  }

  /** Remaining lock of the hub in seconds (every server). */
  get regHubLeft(): number {
    return this.regLock.left('hub');
  }

  /** Remaining throttling of this server in seconds. */
  regServerLeft(id: string | null): number {
    return id === null ? 0 : this.regLock.left(`srv:${id}`);
  }

  /** Error toast, reload the list; without a session or role reload the account. Nothing after leaving the page. */
  #failed(error: unknown, text?: string): void {
    if (this.#disposed) return;
    ui.toasts.error(failText(error, text));
    if (error instanceof ApiError && (error.status === 401 || error.status === 403)) {
      void session.refreshUser().catch(() => {});
      return;
    }
    void this.load();
  }

  /** Take over a server from a response (add, edit, test, registration). */
  #put(server: AdminServer): void {
    const known = this.rows.some((s) => s.id === server.id);
    this.rows = known ? this.rows.map((s) => (s.id === server.id ? server : s)) : [...this.rows, server];
  }

  #edited(id: string): number {
    // The reasons of the last test no longer apply to the changed details.
    this.reasons.delete(id);
    const n = (this.#edits[id] ?? 0) + 1;
    this.#edits[id] = n;
    return n;
  }

  // List

  toggleDiag(id: string): void {
    this.#diagToggles[id] = (this.#diagToggles[id] ?? 0) + 1;
    if (this.diag.has(id)) this.diag.delete(id);
    else this.diag.add(id);
  }

  /**
   * "Test connection": expands the diagnosis, result as a toast. Only one test runs at a time; after saving
   * (`queue`) the test follows after a running one.
   */
  async test(id: string, queue = false): Promise<void> {
    if (!this.rows.some((s) => s.id === id)) return;
    if (this.testing) {
      if (queue && !this.#queue.includes(id)) this.#queue.push(id);
      return;
    }
    const wasOpen = this.diag.has(id);
    const toggles = this.#diagToggles[id] ?? 0;
    this.testing = id;
    this.diag.add(id);
    const stamp = this.#edits[id] ?? 0;
    try {
      const { server, reasons, retry_after } = await api<TestReply>('POST', `admin/stalwart/servers/${id}/test`);
      if (this.#disposed) return;
      // Saved or removed in the meantime: the response may be older than the state of the list.
      if ((this.#edits[id] ?? 0) !== stamp || !this.rows.some((s) => s.id === id)) return;
      this.#put(server);
      this.reasons.set(id, reasons);
      if (retry_after) this.testWait.start(id, retry_after);
      else this.testWait.clear(id);
      ui.toasts.show(testText(server, reasons));
    } catch (error) {
      // Without a result back to the state before the test: open diagnoses stay open, closed ones close again –
      // unless the user opened or closed it meanwhile.
      if (!wasOpen && (this.#diagToggles[id] ?? 0) === toggles) this.diag.delete(id);
      this.#failed(error);
    } finally {
      if (this.testing === id) this.testing = null;
      // The next queued server that still exists (one removed meanwhile is skipped).
      let next = this.#queue.shift();
      while (next !== undefined && !this.rows.some((s) => s.id === next)) next = this.#queue.shift();
      if (next !== undefined && !this.#disposed) void this.test(next);
    }
  }

  /** "Remove" with confirmation; the row goes when the server confirms. */
  remove(server: AdminServer): void {
    const n = server.accounts;
    const sys = server.id === this.systemServer;
    const linked = n === 0 ? m.srv_remove_none() : n === 1 ? m.srv_remove_one() : m.srv_remove_many({ n });
    ui.ask({
      title: m.srv_remove_title({ label: server.label }),
      text: sys ? `${linked} ${m.srv_remove_system()}` : linked,
      icon: 'ti-server-off',
      ok: m.srv_remove(),
      danger: true,
      run: () => {
        this.#edited(server.id);
        void api('DELETE', `admin/stalwart/servers/${server.id}`)
          .catch((error: unknown) => {
            // Already gone (e.g. removed by another administrator): treat as removed.
            if (!isApiError(error, 404)) throw error;
          })
          .then(
            () => {
              ui.toasts.show(m.srv_removed({ label: server.label }));
              if (this.#disposed) return;
              this.#removed.add(server.id);
              this.rows = this.rows.filter((s) => s.id !== server.id);
              this.diag.delete(server.id);
              if (this.systemServer === server.id) this.systemServer = null;
            },
            (error: unknown) => this.#failed(error, m.srv_remove_failed()),
          );
      },
    });
  }

  /** "Users can connect accounts on other servers": immediately, without undo. */
  toggleOwn(): void {
    const on = !this.allowOwn;
    this.allowOwn = on;
    const seq = ++this.#policySeq;
    ui.toasts.show(on ? m.srv_own_allowed() : m.srv_own_blocked());
    this.#policyWrites = this.#policyWrites
      .catch(() => {})
      .then(() => api('PUT', 'admin/stalwart/policy', { allow_custom_servers: on }))
      .then(
        () => {
          this.#policyConfirmed = on;
        },
        (error: unknown) => {
          if (seq !== this.#policySeq) return;
          // Back to what the server last confirmed, not to the opposite of the failed toggle.
          this.allowOwn = this.#policyConfirmed;
          this.#failed(error);
        },
      );
  }

  async copyRedirect(): Promise<void> {
    if (await copyText(this.redirect)) ui.toasts.show(m.srv_redirect_copied());
    else ui.toasts.error(m.srv_copy_failed());
  }

  // Dialog

  /** Open the dialog: new or for a server; `register` shows the credentials of "Automatic" at once. */
  openDialog(server: AdminServer | null = null, register = false): void {
    const cid = server?.client_id ?? '';
    this.dialog = {
      seq: ++this.#formSeq,
      id: server?.id ?? null,
      label: server?.label ?? '',
      url: server?.url ?? 'https://',
      domains: [...(server?.domains ?? [])],
      domIn: '',
      // A client entered by hand stays "Manual"; registering always "Automatic".
      oa: register ? 'auto' : cid && !server?.has_secret ? 'manual' : 'auto',
      cid,
      csec: '',
      showSec: false,
      savedCid: cid,
      savedUrl: server?.url ?? '',
      links: server?.accounts ?? 0,
      hasSec: !!server?.has_secret,
      again: register,
      au: '',
      ap: '',
      aErr: null,
      regFailed: false,
      err: {},
      saving: false,
      registering: false,
    };
    ui.closeMenu();
    ui.toasts.dismiss();
  }

  closeDialog(): void {
    this.dialog = null;
  }

  #set(patch: Partial<ServerForm>): void {
    if (this.dialog) this.dialog = { ...this.dialog, ...patch };
  }

  /** The dialog takes input: not while saving (the request already carries the fields). */
  get #editable(): boolean {
    return !!this.dialog && !this.dialog.saving;
  }

  #setErr(err: FormErrors): void {
    if (this.dialog) this.#set({ err: { ...this.dialog.err, ...err } });
  }

  setLabel(value: string): void {
    if (!this.#editable) return;
    this.#set({ label: value });
    this.#setErr({ label: undefined });
  }

  setUrl(value: string): void {
    if (!this.#editable) return;
    this.#set({ url: value });
    this.#setErr({ url: undefined });
  }

  /** Input in the domains field: separators make chips. */
  domainInput(value: string): void {
    const d = this.dialog;
    if (!d || d.saving) return;
    if (!SEP.test(value)) {
      this.#set({ domIn: value });
    } else {
      const parts = value.split(SEP);
      const last = /[,;\s]$/.test(value) ? '' : (parts.pop() ?? '');
      this.#set({ domains: addDomains(d.domains, parts.join(' ')), domIn: last });
    }
    this.#setErr({ dom: undefined });
  }

  /** Enter takes over, backspace in the empty field removes the last domain. */
  domainKey(event: KeyInput): void {
    const d = this.dialog;
    if (!d) return;
    if (d.saving) {
      if (event.key === 'Enter') event.preventDefault();
      return;
    }
    if (event.key === 'Enter') {
      event.preventDefault();
      this.#set({ domains: addDomains(d.domains, d.domIn), domIn: '' });
    } else if (event.key === 'Backspace' && !d.domIn && d.domains.length) {
      this.#set({ domains: d.domains.slice(0, -1) });
    }
  }

  domainBlur(): void {
    const d = this.dialog;
    if (d?.domIn && !d.saving) this.#set({ domains: addDomains(d.domains, d.domIn), domIn: '' });
  }

  /** Remove a domain; a message about the domains no longer applies afterwards. */
  removeDomain(domain: string): void {
    const d = this.dialog;
    if (!d || d.saving) return;
    this.#set({ domains: d.domains.filter((x) => x !== domain) });
    this.#setErr({ dom: undefined });
  }

  setOa(oa: OaMode): void {
    if (!this.#editable) return;
    this.#set({ oa });
    this.#setErr({ cred: undefined, adm: undefined });
  }

  setAdminUser(value: string): void {
    if (!this.#editable) return;
    this.#set({ au: value, aErr: null });
    this.#setErr({ adm: undefined });
  }

  setAdminPassword(value: string): void {
    if (!this.#editable) return;
    this.#set({ ap: value, aErr: null });
    this.#setErr({ adm: undefined });
  }

  setCid(value: string): void {
    if (!this.#editable) return;
    this.#set({ cid: value });
    this.#setErr({ cred: undefined });
  }

  setSecret(value: string): void {
    if (!this.#editable) return;
    this.#set({ csec: value });
    this.#setErr({ cred: undefined });
  }

  toggleSecret(): void {
    if (this.dialog) this.#set({ showSec: !this.dialog.showSec });
  }

  /** "Register again" for a server with a client, or back to keeping it. */
  setAgain(again: boolean): void {
    if (!this.#editable) return;
    this.#set({ again, au: '', ap: '', aErr: null });
    this.#setErr({ adm: undefined });
  }

  /** Enter in label or address saves. */
  dialogKey(event: KeyInput): void {
    if (event.key !== 'Enter') return;
    event.preventDefault();
    void this.save();
  }

  /**
   * "Add and test" / "Save and test", with credentials "… and register": checks the fields, saves, registers the
   * client if asked and tests afterwards. "Manual" sends client ID and secret (an empty secret keeps the stored one
   * with the same client ID); "Automatic" leaves the stored client to the registration.
   */
  async save(): Promise<void> {
    const d = this.dialog;
    if (!d || d.saving) return;
    const domains = addDomains(d.domains, d.domIn);
    const url = trimUrl(d.url);
    const others = this.rows.filter((s) => s.id !== d.id);
    const err: FormErrors = {};
    if (!d.label.trim()) err.label = m.srv_err_label();
    const badUrl = urlError(url);
    if (badUrl) err.url = badUrl;
    const bad = domains.find((x) => !isDomain(x));
    const taken = domains.map((x) => [x, others.find((s) => s.domains.includes(x))] as const).find(([, s]) => s);
    // One message at a time: an invalid domain first, then one already assigned, last the number.
    if (bad) err.dom = m.srv_err_domain({ domain: bad });
    else if (taken?.[1]) err.dom = m.srv_err_domain_taken({ domain: taken[0], server: taken[1].label });
    else if (domains.length > MAX_DOMAINS) err.dom = m.srv_doms_max({ n: MAX_DOMAINS });
    const manual = d.oa === 'manual';
    if (manual && credBad(d.cid.trim())) err.cred = m.srv_cid_bad();
    else if (manual && credBad(d.csec.trim())) err.cred = m.srv_secret_bad();
    else if (manual && d.csec.trim() && !d.cid.trim()) err.cred = m.srv_cid_needed();
    if (d.oa === 'auto' && showsCredentials(d) && credentials(d) === 'half') err.adm = m.srv_err_admin();
    const register = registers(d);
    if (err.label || err.url || err.dom || err.cred || err.adm) {
      this.#set({ err, domains, domIn: '' });
      return;
    }
    if (register && this.regBlocked(d.id)) return;
    const cid = d.cid.trim();
    const secret = manual && cid ? d.csec.trim() : '';
    const client = manual ? (d.id ? { client_id: cid || null } : cid ? { client_id: cid } : {}) : {};
    const body = {
      label: d.label.trim(),
      url,
      domains,
      ...client,
      ...(secret ? { client_secret: secret } : {}),
    };
    this.#set({ saving: true, domains, domIn: '', err: {}, aErr: null, regFailed: false });
    const current = () => this.dialog?.seq === d.seq;
    let server: AdminServer;
    try {
      server = d.id
        ? await api<AdminServer>('PATCH', `admin/stalwart/servers/${d.id}`, body)
        : await api<AdminServer>('POST', 'admin/stalwart/servers', body);
    } catch (error) {
      if (this.#disposed) return;
      if (!current()) {
        this.#failed(error);
        return;
      }
      this.#set({ saving: false });
      this.#saveFailed(error, url);
      return;
    }
    if (this.#disposed) return;
    this.#edited(server.id);
    this.#put(server);
    if (!current()) {
      void this.test(server.id, true);
      return;
    }
    if (register) {
      // From here on the dialog edits the saved server: a retry saves it again instead of adding it twice.
      this.#set({
        id: server.id,
        savedCid: server.client_id ?? '',
        savedUrl: server.url,
        hasSec: server.has_secret,
        links: server.accounts,
        // The credentials stay in view for a retry, also when the server already had a client.
        again: true,
        registering: true,
      });
      if (!(await this.#register(server, d.au.trim(), d.ap, d.seq))) return;
    } else {
      ui.toasts.show(m.srv_saved({ label: server.label }));
    }
    if (this.dialog?.seq === d.seq) this.dialog = null;
    // Test right afterwards (if a test is running, after it); the diagnosis no longer applies after a new address or client.
    void this.test(server.id, true);
  }

  /**
   * Registers the OAuth client of a saved server; `true` if it worked. On failure the dialog stays open on the saved
   * server with the error at the credentials.
   */
  async #register(server: AdminServer, username: string, password: string, seq: number): Promise<boolean> {
    const current = () => this.dialog?.seq === seq;
    try {
      // The address the dialog saved: the hub sends the credentials only there (`server_changed` otherwise).
      const registered = await api<AdminServer>('POST', `admin/stalwart/servers/${server.id}/register-client`, {
        url: server.url,
        username,
        password,
      });
      if (this.#disposed) return false;
      this.regLock.clear(`srv:${server.id}`);
      this.#edited(registered.id);
      this.#put(registered);
      ui.toasts.show(m.srv_registered({ label: registered.label }));
      return true;
    } catch (error) {
      if (this.#disposed) return false;
      // Locks apply to the next registration, whichever dialog it comes from.
      this.#lockRegistration(error, server.id);
      if (!current()) {
        this.#failed(error);
        return false;
      }
      this.#set({ saving: false, registering: false });
      this.#registerFailed(error);
      return false;
    }
  }

  /** 429 of the hub locks the registration for every server, `throttled` of Stalwart with a wait time for its own. */
  #lockRegistration(error: unknown, id: string): void {
    if (error instanceof ApiError && error.code === 'registration_failed') {
      const wait = error.body.retry_after;
      if (error.body.reason === 'throttled' && typeof wait === 'number' && wait > 0)
        this.regLock.start(`srv:${id}`, wait);
    } else if (isApiError(error, 429)) {
      this.regLock.start('hub', retryAfter(error));
    }
  }

  /**
   * Errors of the registration in the dialog per reason; the locks (box "Too many attempts." or "Too many
   * requests." with the wait time) come from `#lockRegistration`.
   */
  #registerFailed(error: unknown): void {
    if (error instanceof ApiError && error.code === 'registration_failed') {
      const reason = typeof error.body.reason === 'string' ? error.body.reason : null;
      const detail = typeof error.body.detail === 'string' ? error.body.detail : null;
      const wait = error.body.retry_after;
      if (reason === 'throttled' && typeof wait === 'number' && wait > 0) {
        this.#set({ regFailed: true });
        return;
      }
      this.#set({
        regFailed: true,
        aErr: registrationText(reason, detail, session.appName),
        ...(adminRejected(reason) ? { ap: '' } : {}),
      });
    } else if (isApiError(error, 429)) {
      this.#set({ regFailed: true });
    } else if (isApiError(error, 422, 'invalid')) {
      this.#set({ regFailed: true });
      this.#setErr({ adm: m.srv_err_admin() });
    } else if (isApiError(error, 409, 'server_changed')) {
      this.#set({ regFailed: true, aErr: m.srv_reg_changed() });
      void this.load();
    } else if (isApiError(error, 404)) {
      // Removed in the meantime.
      this.closeDialog();
      this.#failed(error);
    } else {
      this.#set({ regFailed: true });
      this.#failed(error);
    }
  }

  /** Field errors of the server; what the dialog has no text for as a toast. */
  #saveFailed(error: unknown, url: string): void {
    if (isApiError(error, 404)) {
      // Removed in the meantime.
      this.closeDialog();
      this.#failed(error);
      return;
    }
    if (!isApiError(error, 422, 'invalid')) {
      this.#failed(error);
      return;
    }
    const fields = fieldsOf(error);
    const err: FormErrors = {};
    const label = fields.label?.code;
    if (label === 'required') err.label = m.srv_err_label();
    else if (label === 'too_long') err.label = m.srv_err_too_long();
    const u = fields.url;
    if (u) {
      const other = typeof u.server === 'string' ? u.server : '';
      err.url =
        u.code === 'taken'
          ? other
            ? m.srv_err_taken_by({ server: other })
            : m.srv_err_taken()
          : (urlError(url) ?? m.srv_err_url());
    }
    const dom = fields.domains;
    const domain = typeof dom?.domain === 'string' ? dom.domain : '';
    if (dom?.code === 'taken' && domain) {
      err.dom = m.srv_err_domain_taken({ domain, server: typeof dom.server === 'string' ? dom.server : '' });
    } else if (dom?.code === 'invalid' && domain) {
      err.dom = m.srv_err_domain({ domain });
    } else if (dom?.code === 'too_many') {
      err.dom = m.srv_doms_max({ n: MAX_DOMAINS });
    }
    if (fields.client_secret?.code === 'needs_client_id') err.cred = m.srv_cid_needed();
    else if (fields.client_id) err.cred = m.srv_cid_bad();
    else if (fields.client_secret) err.cred = m.srv_secret_bad();
    const shown = ['url', 'client_id', 'client_secret'];
    if (err.label) shown.push('label');
    if (err.dom) shown.push('domains');
    // Otherwise a field for which the dialog has no text.
    if (Object.keys(fields).some((f) => !shown.includes(f))) ui.toasts.error(m.error_act());
    if (Object.keys(err).length) this.#set({ err });
  }
}

/**
 * Toast of a finished test: "connection OK" only when every check passed; a failed or throttled check gives
 * "warnings", a check that did not run (`unk`, e.g. OAuth without a client) "not everything checked".
 */
export function testText(server: AdminServer, reasons: DiagReasons): string {
  const statuses = [server.reach, server.cors, server.oauth];
  const label = server.label;
  if (statuses.includes('err') || Object.values(reasons).includes('throttled')) return m.srv_test_warn({ label });
  if (statuses.every((st) => st === 'ok')) return m.srv_test_ok({ label });
  return m.srv_test_unchecked({ label });
}

// Values of the page

export interface ServerChip {
  label: string;
  status: DiagStatus;
  icon: IconName;
  /** Colour of the icon and the text. */
  ic: string;
  ink: string;
  aria: string;
  tip: string;
}

export interface DiagRowValues {
  label: string;
  sub: string;
  sep: boolean;
  icon: IconName;
  ic: string;
  text: string;
  isErr: boolean;
  actLabel: string;
  /** The action is "Test again" while another server's test runs. */
  actDis: boolean;
  act: () => void;
}

export interface ServerRow {
  id: string;
  label: string;
  host: string;
  url: string;
  sep: boolean;
  doms: { d: string }[];
  noDoms: boolean;
  accts: string;
  isSys: boolean;
  plain: boolean;
  chips: ServerChip[];
  tested: string;
  /** With notices: tile red. */
  warn: boolean;
  /** Test of this server running. */
  busy: boolean;
  /** Another server's test is running: "Test connection" and "Test again" wait (only one test at a time). */
  testDis: boolean;
  testLabel: string;
  diagOpen: boolean;
  diagBusy: boolean;
  diagDone: boolean;
  diagRows: DiagRowValues[];
  diagLabel: string;
  canDiag: boolean;
  test: () => void;
  edit: () => void;
  remove: () => void;
  diagToggle: () => void;
}

export interface ServersValues {
  srvTabs: { label: string; onClick: () => void }[];
  srvHas: boolean;
  srvEmpty: boolean;
  srvAdd: () => void;
  srvSum: string;
  srvRows: ServerRow[];
  tgSrvOwn: { on: boolean; onClick: () => void };
  srvOwnSum: string;
  srvRedirect: string;
  srvCopyRedirect: () => void;
}

export interface ServersContext {
  /** Scroll to the section. */
  scrollTo: (section: Section) => void;
}

const STATUS: Record<DiagStatus, { icon: IconName; ic: string; ink: string; text: () => string }> = {
  ok: { icon: 'ti-check', ic: OK_COLOR, ink: 'var(--fg)', text: m.srv_status_ok },
  err: { icon: 'ti-x', ic: 'var(--danger)', ink: 'var(--danger)', text: m.srv_status_err },
  unk: { icon: 'ti-question-mark', ic: 'var(--mfg)', ink: 'var(--mfg)', text: m.srv_status_unk },
};

function chip(label: string, status: DiagStatus): ServerChip {
  const s = STATUS[status];
  const text = s.text();
  return {
    label,
    status,
    icon: s.icon,
    ic: s.ic,
    ink: s.ink,
    aria: m.srv_chip_aria({ label, status: text }),
    tip: text.charAt(0).toUpperCase() + text.slice(1),
  };
}

/** Rows of the diagnosis, texts per reason. */
function diagRows(p: ServersPage, s: AdminServer): DiagRowValues[] {
  const reasons = p.reasons.get(s.id);
  const ctx = { origin: p.appHost, redirect: p.redirect, app: session.appName, wait: p.testLeft(s.id) };
  const actions: Record<DiagAction, [string, () => void]> = {
    edit: [m.srv_diag_edit(), () => p.openDialog(s)],
    test: [m.srv_diag_retest(), () => void p.test(s.id)],
    register: [m.srv_diag_register(), () => p.openDialog(s, true)],
  };
  const action = (line: DiagLine) => actions[diagAction(line, reasons?.[line])];
  const otherTest = p.testing !== null && p.testing !== s.id;
  const rows: [DiagLine, string, string, DiagStatus, string, () => void][] = [
    ['reach', m.srv_diag_jmap(), m.srv_diag_jmap_sub({ url: s.url.replace(/\/+$/, '') }), s.reach, ...action('reach')],
    ['cors', m.srv_diag_cors(), m.srv_diag_cors_sub({ host: p.appHost }), s.cors, ...action('cors')],
    [
      'oauth',
      m.srv_diag_oauth(),
      s.client_id ? m.srv_diag_cid({ cid: s.client_id }) : m.srv_diag_no_cid(),
      s.oauth,
      ...action('oauth'),
    ],
  ];
  return rows.map(([line, label, sub, st, actLabel, act], i) => ({
    actDis: otherTest && diagAction(line, reasons?.[line]) === 'test',
    label,
    sub,
    sep: i > 0,
    icon: diagIcon(st),
    ic: diagColor(st),
    text: diagText(line, st, s.reach, reasons, ctx),
    isErr: st === 'err',
    actLabel,
    act,
  }));
}

const hasWarning = (s: AdminServer) => [s.reach, s.cors, s.oauth].includes('err');

function rowValues(p: ServersPage, s: AdminServer, i: number, now: number): ServerRow {
  const n = s.accounts;
  const busy = p.testing === s.id;
  const open = p.diag.has(s.id);
  const testedAt = s.tested_at ? Date.parse(s.tested_at) : NaN;
  return {
    id: s.id,
    label: s.label,
    host: s.host,
    url: s.url,
    sep: i > 0,
    doms: s.domains.map((d) => ({ d })),
    noDoms: s.domains.length === 0,
    accts: n === 0 ? m.srv_accounts_none() : n === 1 ? m.srv_accounts_one() : m.srv_accounts_many({ n }),
    isSys: s.id === p.systemServer,
    plain: localHttp(s.url),
    chips: [chip(m.srv_chip_reach(), s.reach), chip(m.srv_chip_cors(), s.cors), chip(m.srv_chip_oauth(), s.oauth)],
    tested: busy
      ? m.srv_testing()
      : Number.isNaN(testedAt)
        ? m.srv_untested()
        : m.srv_tested({ ago: ago(testedAt, now) }),
    warn: hasWarning(s),
    busy,
    testDis: p.testing !== null && !busy,
    testLabel: busy ? m.srv_test_busy() : m.srv_test(),
    diagOpen: open,
    diagBusy: open && busy,
    diagDone: open && !busy,
    diagRows: open && !busy ? diagRows(p, s) : [],
    diagLabel: open ? m.srv_diag_hide() : m.srv_diag_show(),
    canDiag: !!s.tested_at,
    test: () => void p.test(s.id),
    edit: () => p.openDialog(s),
    remove: () => p.remove(s),
    diagToggle: () => p.toggleDiag(s.id),
  };
}

/** Values of the page from the state. */
export function serversValues(p: ServersPage, ctx: ServersContext, now = Date.now()): ServersValues {
  const n = p.rows.length;
  const warn = p.rows.filter(hasWarning).length;
  const sum = n === 1 ? m.srv_sum_one() : m.srv_sum_many({ n });
  return {
    srvTabs: [
      { label: m.srv_tab_list(), onClick: () => ctx.scrollTo('list') },
      { label: m.srv_tab_policy(), onClick: () => ctx.scrollTo('policy') },
    ],
    srvHas: n > 0,
    srvEmpty: n === 0,
    srvAdd: () => p.openDialog(),
    srvSum: warn ? m.srv_sum_warn({ sum, n: warn }) : sum,
    srvRows: p.rows.map((s, i) => rowValues(p, s, i, now)),
    tgSrvOwn: { on: p.allowOwn, onClick: () => p.toggleOwn() },
    srvOwnSum: p.allowOwn ? m.srv_own_on() : m.srv_own_off(),
    srvRedirect: p.redirect,
    srvCopyRedirect: () => void p.copyRedirect(),
  };
}

// Values of the dialog

export interface OaOption {
  label: string;
  on: boolean;
  onClick: () => void;
}

export interface ServerDialogValues {
  title: string;
  sub: string;
  btn: string;
  btnIcon: IconName;
  icon: IconName;
  label: string;
  url: string;
  domIn: string;
  domPh: string;
  doms: { d: string; rmLabel: string; rm: () => void }[];
  eLabel: string;
  hasELabel: boolean;
  eUrl: string;
  hasEUrl: boolean;
  plain: boolean;
  eDom: string;
  hasEDom: boolean;
  eAdm: string;
  hasEAdm: boolean;
  hasECred: boolean;
  eCred: string;
  /** 429 of the hub during registration: box "Too many attempts.". */
  aLocked: boolean;
  aLockText: string;
  /** `throttled` of Stalwart: box "Too many requests.". */
  aThr: boolean;
  aThrText: string;
  /** The server is saved, but its registration failed. */
  regFailed: boolean;
  /** Credentials of "Automatic" shown (no client, "Register again" or a new address). */
  aFields: boolean;
  /** "Automatic" with a stored client: the box "OAuth client set up". */
  aDone: boolean;
  aDoneSub: string;
  /** "Register again" can go back to keeping the stored client. */
  canKeep: boolean;
  /** A new address while a client is stored: register again. */
  moved: boolean;
  /** Notice about the linked accounts before registering a new client. */
  hasLinkHint: boolean;
  linkHint: string;
  /** The same under "Manual" as soon as the client ID differs from the stored one. */
  hasCidHint: boolean;
  cidHint: string;
  oaOpts: OaOption[];
  isAuto: boolean;
  isManual: boolean;
  au: string;
  ap: string;
  cid: string;
  csec: string;
  secPh: string;
  showSec: boolean;
  secIcon: IconName;
  secTip: string;
  redirect: string;
  /** Saving or registering running. */
  saving: boolean;
  /** The save button waits for a lock of the registration. */
  saveLocked: boolean;
  onLabel: (value: string) => void;
  onUrl: (value: string) => void;
  onDomIn: (value: string) => void;
  domKey: (event: KeyInput) => void;
  domBlur: () => void;
  onAu: (value: string) => void;
  onAp: (value: string) => void;
  regAgain: () => void;
  keep: () => void;
  onCid: (value: string) => void;
  onCsec: (value: string) => void;
  onSecEye: () => void;
  copyRedirect: () => void;
  save: () => void;
  close: () => void;
  /** Click on the backdrop, not on the dialog. */
  overlay: (event: OverlayEvent) => void;
  key: (event: KeyInput) => void;
}

/** Values of the dialog; `null` as long as it is closed. */
export function dialogValues(p: ServersPage): ServerDialogValues | null {
  const d = p.dialog;
  if (!d) return null;
  const err = (k: keyof FormErrors) => d.err[k] ?? '';
  const eAdm = err('adm') || d.aErr || '';
  // Stalwart's throttling applies only to its server, the hub's lock to all.
  const hubLeft = p.regHubLeft;
  const srvLeft = p.regServerLeft(d.id);
  const thr = srvLeft > 0 && hubLeft === 0;
  const left = Math.max(hubLeft, srvLeft);
  const oa = (mode: OaMode, label: string): OaOption => ({
    label,
    on: d.oa === mode,
    onClick: () => p.setOa(mode),
  });
  const cid = d.cid.trim();
  // The stored secret belongs to the client ID and the address: a new address drops it on the server.
  const sameUrl = trimUrl(d.url) === d.savedUrl;
  const secText =
    cid && d.hasSec && cid === d.savedCid
      ? sameUrl
        ? m.srv_secret_kept()
        : m.srv_secret_moved()
      : m.srv_secret_optional();
  const links = d.links;
  const linkText = (one: () => string, many: (n: { n: number }) => string) =>
    links === 1 ? one() : links > 1 ? many({ n: links }) : '';
  const fields = showsCredentials(d);
  const register = registers(d);
  const moved = !!d.savedCid && trimUrl(d.url) !== d.savedUrl;
  const locked = register && left > 0;
  return {
    title: d.id ? m.srv_dlg_edit() : m.srv_add(),
    sub: d.id ? hostOf(trimUrl(d.url)) || d.url.trim() : m.srv_dlg_add_sub(),
    btn: locked
      ? m.srv_reg_locked_button({ clock: clockText(left) })
      : register
        ? d.id
          ? m.srv_dlg_save_reg()
          : m.srv_dlg_save_new_reg()
        : d.id
          ? m.srv_dlg_save()
          : m.srv_dlg_save_new(),
    btnIcon: locked ? 'ti-lock' : register ? 'ti-wand' : 'ti-stethoscope',
    icon: d.id ? 'ti-server-cog' : 'ti-server-2',
    label: d.label,
    url: d.url,
    domIn: d.domIn,
    domPh: d.domains.length ? '' : m.srv_domain_placeholder(),
    doms: d.domains.map((x) => ({
      d: x,
      rmLabel: m.srv_domain_remove({ domain: x }),
      rm: () => p.removeDomain(x),
    })),
    eLabel: err('label'),
    hasELabel: !!err('label'),
    eUrl: err('url'),
    hasEUrl: !!err('url'),
    plain: localHttp(d.url.trim()) && !err('url'),
    eDom: err('dom'),
    hasEDom: !!err('dom'),
    eAdm,
    hasEAdm: !!eAdm,
    eCred: err('cred'),
    hasECred: !!err('cred'),
    aLocked: hubLeft > 0,
    aLockText: m.limit_text({ wait: waitText(hubLeft) }),
    aThr: thr,
    aThrText: thr ? throttleText(session.appName, srvLeft) : '',
    regFailed: d.regFailed,
    aFields: fields,
    aDone: !fields,
    aDoneSub: d.hasSec ? m.srv_oa_done_secret({ cid: d.savedCid }) : m.srv_oa_cid({ cid: d.savedCid }),
    canKeep: d.again && !!d.savedCid && !moved,
    moved,
    hasLinkHint: links > 0,
    linkHint: linkText(m.srv_link_hint_one, m.srv_link_hint),
    hasCidHint: links > 0 && cid !== d.savedCid,
    cidHint: linkText(m.srv_cid_hint_one, m.srv_cid_hint),
    oaOpts: [oa('auto', m.srv_oa_auto()), oa('manual', m.srv_oa_manual())],
    isAuto: d.oa === 'auto',
    isManual: d.oa === 'manual',
    au: d.au,
    ap: d.ap,
    cid: d.cid,
    csec: d.csec,
    secPh: secText,
    showSec: d.showSec,
    secIcon: d.showSec ? 'ti-eye-off' : 'ti-eye',
    secTip: d.showSec ? m.srv_secret_hide() : m.srv_secret_show(),
    redirect: p.redirect,
    saving: d.saving,
    saveLocked: locked,
    onLabel: (value) => p.setLabel(value),
    onUrl: (value) => p.setUrl(value),
    onDomIn: (value) => p.domainInput(value),
    domKey: (event) => p.domainKey(event),
    domBlur: () => p.domainBlur(),
    onAu: (value) => p.setAdminUser(value),
    onAp: (value) => p.setAdminPassword(value),
    regAgain: () => p.setAgain(true),
    keep: () => p.setAgain(false),
    onCid: (value) => p.setCid(value),
    onCsec: (value) => p.setSecret(value),
    onSecEye: () => p.toggleSecret(),
    copyRedirect: () => void p.copyRedirect(),
    save: () => void p.save(),
    close: () => p.closeDialog(),
    overlay: overlayClose(() => p.closeDialog()),
    key: (event) => p.dialogKey(event),
  };
}
