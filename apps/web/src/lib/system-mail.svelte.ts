/**
 * Administration › Email: the sending account of the system mails against
 * `GET/PUT admin/system-mail` and `POST admin/system-mail/test`: status,
 * connection (server from the hub's Stalwart registry, sign-in by password or
 * token), sender, sending, test email and the bar "Unsaved changes". The
 * registry (`GET admin/stalwart/servers`) loads with the account and fills the
 * "Mail server" pick list; without a server in it the page shows a hint
 * instead, and saving or testing reports the server as missing.
 *
 * Status: "Checking" during a check, "Connected" as long as the draft matches
 * the last successfully checked connection (`good`), "Not set up" without a
 * sending account, "Not checked" after changed credentials (`tested`),
 * otherwise the result of the last check. A stored account without a check
 * (`unchecked`) is checked silently by the page after loading, meanwhile
 * "Checking …"; if that does not work (no response), "Not checked" stays with
 * the text for `unchecked`.
 *
 * The page knows secrets only as input: an empty field keeps the stored
 * secret of the same sign-in method on the same server (`has_secret`; another
 * server drops it on the server), it is only sent when entered. "Save" sends
 * the changed fields right away (the bar shows "Saving …"); the backend checks
 * a changed connection itself (status "Checking" during the request) and
 * answers with the stored account and the result. Inputs made during the
 * request stay; checking and sending wait for a running save.
 *
 * Field errors: the page checks live per the backend's rules (username,
 * secret, sender name, reply-to address) and shows the text at the field.
 * Saving with an error – or without the server, username or secret the
 * account needs – shows "Please check the highlighted fields",
 * jumps to the section and sends nothing. A 422 of the server shows the same
 * texts at the field until it is changed.
 *
 * Error texts per reason of `mail_failed`: in the status "No connection" and
 * in the error toast of "Send". "Test connection" only reports "Connection
 * failed"; if only the test email fails after a successful sign-in while
 * sending, the status stays, likewise for 429 `rate_limited`, server errors
 * and without a response (only an error toast).
 *
 * Taken over from Roedy Mail (`apps/web/src/lib/system-mail.svelte.ts`) with
 * the server from the hub's Stalwart registry instead of Mail's server list,
 * without "Manage mail servers", the template preview (the hub's API has none)
 * and the deferred save with undo (the hub's toasts only inform).
 */
import { lostAccess } from '#lib/access.ts';
import {
  ago,
  api,
  ApiError,
  failText,
  fieldsOf as apiFieldsOf,
  type IconName,
  isApiError,
  loadFailText,
  Loading,
  retryAfter,
  ui,
  waitText,
} from '@roedy/ui';
import { segOptions, selectValues, type SelectContext, type SelectState, type SelectValues } from '#lib/select.ts';
import type { SegOption } from '#lib/appearance.ts';
import { isEmail } from '#lib/email.ts';
import { m } from '#lib/paraglide/messages.js';
import { session } from '#lib/state/session.svelte.ts';
import { trimWs } from '#lib/trim.ts';

export type MailAuth = 'password' | 'token';

/** Identity of the sending account. */
export interface MailIdentity {
  id: string;
  name: string;
  email: string;
}

/** Response of `GET`/`PUT admin/system-mail`. */
export interface SystemMail {
  /** Server of the Stalwart registry; `null` without an account or after it was removed from the registry. */
  server_id: string | null;
  auth: MailAuth;
  user: string;
  has_secret: boolean;
  identity_id: string | null;
  sender_name: string | null;
  reply_to: string | null;
  keep_sent: boolean;
  status: 'none' | 'unchecked' | 'ok' | 'err';
  reason: string | null;
  /** Missing mailbox for `reason` `no_mailbox`; absent for other reasons. */
  role?: 'drafts' | 'sent';
  checked_at: string | null;
  identities: MailIdentity[];
}

/** Server of the Stalwart registry for the pick list (`admin/stalwart/servers`). */
export interface MailServer {
  id: string;
  label: string;
  /** Address as the registry stores it, e.g. `https://mail.example.com/`. */
  url: string;
  /** Host with port for the texts, e.g. `mail.example.com:8443`. */
  host: string;
}

/** Draft of the form; secrets only as input (empty = keep the stored one). */
export interface MailDraft {
  /** ID of the chosen server; empty = none. */
  srv: string;
  auth: MailAuth;
  user: string;
  pass: string;
  token: string;
  name: string;
  replyTo: string;
  keepSent: boolean;
}

export type MailDraftKey = keyof MailDraft;

const DRAFT_KEYS: readonly MailDraftKey[] = ['srv', 'auth', 'user', 'pass', 'token', 'name', 'replyTo', 'keepSent'];

/** Fields that can carry an error. */
export type MailField = 'srv' | 'user' | 'pass' | 'token' | 'name' | 'replyTo';
/** `not_found`: the chosen server is no longer in the registry. */
export type MailFieldError = 'required' | 'invalid' | 'too_long' | 'not_found';
export type MailErrors = Partial<Record<MailField, MailFieldError>>;

/** Request fields of the backend per field of the draft. */
const API_FIELD: Record<MailField, string> = {
  srv: 'server_id',
  user: 'user',
  pass: 'app_password',
  token: 'token',
  name: 'sender_name',
  replyTo: 'reply_to',
};

/** Fields in page order for the jump to the first error. */
const FIELD_ORDER: readonly MailField[] = ['srv', 'user', 'pass', 'token', 'name', 'replyTo'];

/** Status of the card; `unchecked` only while the silent check cannot run. */
export type MailStatusKey = 'busy' | 'new' | 'unchecked' | 'none' | 'ok' | 'err';

export type MailSection = 'conn' | 'sender' | 'delivery' | 'test';

/** Pick lists of the page: the mail server. */
export type MailPick = 'mailsrv';

/** Sections of the tabs. */
const MAIL_TABS: readonly [MailSection, () => string][] = [
  ['conn', () => m.email_tab_conn()],
  ['sender', () => m.email_tab_sender()],
  ['delivery', () => m.email_tab_delivery()],
  ['test', () => m.email_tab_test()],
];

/**
 * Reasons that concern the connection: if the test email also fails because
 * of them, the check counts as failed. The others concern only the one mail.
 */
const CONNECTION_REASONS: ReadonlySet<string> = new Set([
  'unreachable',
  'auth_failed',
  'auth_locked',
  'forbidden',
  'throttled',
  'foreign_origin',
  'unsupported',
  'no_identity',
  'no_mailbox',
]);

/** Without a sending account: the defaults. */
export const NO_MAIL: SystemMail = {
  server_id: null,
  auth: 'password',
  user: '',
  has_secret: false,
  identity_id: null,
  sender_name: null,
  reply_to: null,
  keep_sent: true,
  status: 'none',
  reason: null,
  checked_at: null,
  identities: [],
};

/** Maximum lengths like the backend: username in bytes, secret in bytes, sender name and reply-to in characters. */
export const MAIL_MAX = { user: 254, secret: 1024, name: 100, replyTo: 254 } as const;

/** Control characters like Rust `char::is_control` (category Cc). */
const CONTROL = /\p{Cc}/u;
/** Whitespace like Rust `char::is_whitespace`. */
const SPACE = /\p{White_Space}/u;
/** Invisible characters like the backend's `me::is_invisible`. */
const INVISIBLE = /[\u00AD\u061C\u180E\u200B-\u200F\u2028-\u202E\u2060-\u206F\uFEFF\uFFF9-\uFFFB]/u;
const bytes = (value: string): number => new TextEncoder().encode(value).length;

export function draftOf(mail: SystemMail): MailDraft {
  return {
    srv: mail.server_id ?? '',
    auth: mail.auth,
    user: mail.user,
    pass: '',
    token: '',
    name: mail.sender_name ?? '',
    replyTo: mail.reply_to ?? '',
    keepSent: mail.keep_sent,
  };
}

/** Entered secret of the chosen sign-in method (token trimmed like Rust `str::trim`, app password not). */
function secretOf(d: MailDraft): string {
  return d.auth === 'token' ? trimWs(d.token) : d.pass;
}

/**
 * Connection of the draft: server, sign-in method, user only with password,
 * entered secret (empty = the stored one).
 */
export function connOf(d: MailDraft): string {
  return JSON.stringify([d.srv, d.auth, d.auth === 'password' ? trimWs(d.user) : '', secretOf(d)]);
}

/** Fields of the connection for `test` and `PUT`. */
function connBody(d: MailDraft): Record<string, unknown> {
  const secret = secretOf(d);
  return {
    server_id: d.srv,
    auth: d.auth,
    // With a token the field is hidden; the backend keeps the stored user.
    ...(d.auth === 'password' && { user: trimWs(d.user) }),
    ...(secret !== '' && (d.auth === 'token' ? { token: secret } : { app_password: secret })),
  };
}

/** The whole draft for the test email (sender and sending already apply to it). */
function draftBody(d: MailDraft): Record<string, unknown> {
  return {
    ...connBody(d),
    sender_name: trimWs(d.name) || null,
    reply_to: trimWs(d.replyTo) || null,
    keep_sent: d.keepSent,
  };
}

/** Only the changed fields for `PUT`; the secret only if entered. */
export function changedBody(d: MailDraft, base: MailDraft): Record<string, unknown> {
  const conn = connBody(d);
  const body: Record<string, unknown> = {};
  if (d.srv !== base.srv) body.server_id = conn.server_id;
  if (d.auth !== base.auth) body.auth = d.auth;
  if ('user' in conn && conn.user !== base.user) body.user = conn.user;
  if ('app_password' in conn) body.app_password = conn.app_password;
  if ('token' in conn) body.token = conn.token;
  const name = trimWs(d.name);
  const reply = trimWs(d.replyTo);
  if (name !== base.name) body.sender_name = name || null;
  if (reply !== base.replyTo) body.reply_to = reply || null;
  if (d.keepSent !== base.keepSent) body.keep_sent = d.keepSent;
  return body;
}

/** Fields of the connection; if a `PUT` changes one of them, the backend checks right away. */
const CONNECTION_FIELDS: readonly string[] = ['server_id', 'auth', 'user', 'app_password', 'token'];

function touchesConnection(body: Record<string, unknown>): boolean {
  return CONNECTION_FIELDS.some((k) => k in body);
}

/** Does a stored secret apply to the draft (same sign-in method, same server)? */
function storedSecret(d: MailDraft, saved: SystemMail): boolean {
  return saved.has_secret && saved.auth === d.auth && saved.server_id !== null && d.srv === saved.server_id;
}

/**
 * Errors of the fields with a value, like the backend checks them one by one (`Body::checked`): username (password
 * only), the secret of the chosen method, sender name and reply-to address.
 */
export function liveErrors(d: MailDraft): MailErrors {
  const out: MailErrors = {};
  const user = trimWs(d.user);
  if (d.auth === 'password' && user && (bytes(user) > MAIL_MAX.user || SPACE.test(user) || CONTROL.test(user)))
    out.user = 'invalid';
  const secret = secretOf(d);
  if (secret && (bytes(secret) > MAIL_MAX.secret || CONTROL.test(secret)))
    out[d.auth === 'token' ? 'token' : 'pass'] = 'invalid';
  const name = trimWs(d.name);
  if (name)
    if (Array.from(name).length > MAIL_MAX.name) out.name = 'too_long';
    else if (CONTROL.test(name) || INVISIBLE.test(name)) out.name = 'invalid';
  const reply = trimWs(d.replyTo);
  if (reply)
    if (Array.from(reply).length > MAIL_MAX.replyTo) out.replyTo = 'too_long';
    else if (!isEmail(reply) || INVISIBLE.test(reply)) out.replyTo = 'invalid';
  return out;
}

/**
 * What the account needs, like the backend's `apply`: a server of the registry, with a password the username, and a
 * secret – entered or stored for the same sign-in method on the same server.
 */
export function requiredErrors(d: MailDraft, saved: SystemMail, servers: readonly MailServer[]): MailErrors {
  const out: MailErrors = {};
  if (!d.srv) out.srv = 'required';
  else if (!servers.some((s) => s.id === d.srv)) out.srv = 'not_found';
  if (d.auth === 'password' && !trimWs(d.user)) out.user = 'required';
  if (!secretOf(d) && !storedSecret(d, saved)) out[d.auth === 'token' ? 'token' : 'pass'] = 'required';
  return out;
}

/** Text of a field error. */
export function mailErrorText(field: MailField, code: MailFieldError): string {
  switch (field) {
    case 'srv':
      return code === 'not_found' ? m.email_err_srv_gone() : m.email_err_srv_required();
    case 'user':
      return code === 'required' ? m.email_err_user_required() : m.email_err_user();
    case 'pass':
      return code === 'required' ? m.email_err_pass_required() : m.email_err_pass();
    case 'token':
      return code === 'required' ? m.email_err_token_required() : m.email_err_token();
    case 'name':
      return code === 'too_long' ? m.email_err_name_long() : m.email_err_name();
    case 'replyTo':
      return code === 'too_long' ? m.email_err_reply_long() : m.email_err_reply();
  }
}

/** Field errors of a 422 `invalid` per field of the draft; other fields (`identity_id`, `to`) do not count. */
export function mailErrorsOf(error: unknown): MailErrors {
  if (!isApiError(error, 422)) return {};
  const fields = apiFieldsOf(error);
  const out: MailErrors = {};
  for (const field of FIELD_ORDER) {
    const code = fields[API_FIELD[field]]?.code;
    if (code === 'required' || code === 'invalid' || code === 'too_long' || code === 'not_found') out[field] = code;
  }
  return out;
}

/** Fields of the chosen sign-in method: with a token the username and the app password are hidden. */
function shownField(field: MailField, auth: MailAuth): boolean {
  return auth === 'token' ? field !== 'user' && field !== 'pass' : field !== 'token';
}

/** Section of the first field with an error in page order. */
export function firstBadSection(errors: MailErrors): MailSection | null {
  const first = FIELD_ORDER.find((k) => errors[k]);
  if (!first) return null;
  return first === 'name' || first === 'replyTo' ? 'sender' : 'conn';
}

/**
 * Check before the request: a server of the registry, a token or user and password – a stored secret of the same
 * sign-in method on the same server counts. Returns the text for the status.
 */
export function precheck(d: MailDraft, saved: SystemMail, servers: readonly MailServer[]): string | null {
  const errors = { ...requiredErrors(d, saved, servers), ...liveErrors(d) };
  if (errors.srv) return mailErrorText('srv', errors.srv);
  if (d.auth === 'token')
    return errors.token ? (errors.token === 'required' ? m.email_err_no_token() : m.email_err_token()) : null;
  if (errors.user === 'required' || errors.pass === 'required') return m.email_err_no_login();
  if (errors.user) return mailErrorText('user', errors.user);
  if (errors.pass) return mailErrorText('pass', errors.pass);
  return null;
}

/** Details of the reason. */
export interface ReasonContext {
  /** Sign-in by token. */
  tok?: boolean;
  /** `retry_after` in seconds (429 `auth_locked`, `throttled`); without: "later" or "in a few minutes". */
  wait?: number;
  /** Missing mailbox for `no_mailbox` (`drafts`, `sent`). */
  role?: string | null;
  /** Name of the app (`throttled`); without it the one from `public/config`. */
  app?: string;
}

/** Text for the reason of a failed check (the status text). */
export function reasonText(reason: string | null, o: ReasonContext = {}): string {
  const wait = o.wait !== undefined && o.wait > 0 ? waitText(o.wait) : null;
  switch (reason) {
    case 'unreachable':
      return m.email_err_reach();
    case 'foreign_origin':
      return m.email_err_foreign();
    case 'auth_failed':
      return o.tok ? m.email_err_auth_token() : m.email_err_auth_password();
    case 'forbidden':
      return m.email_err_forbidden();
    case 'auth_locked':
      return wait ? m.email_err_locked({ wait }) : m.email_err_locked_later();
    case 'throttled': {
      const app = o.app ?? session.appName;
      return wait ? m.email_err_throttled({ app, wait }) : m.email_err_throttled_later({ app });
    }
    case 'no_identity':
      return m.email_err_no_identity();
    case 'no_mailbox':
      return o.role === 'sent' ? m.email_err_no_sent() : m.email_err_no_drafts();
    case 'unsupported':
      return m.email_err_unsupported();
    default:
      return m.error_act();
  }
}

/** Field names of a 422 `invalid`. */
function fieldNames(error: ApiError): string[] {
  return error.code === 'invalid' ? Object.keys(apiFieldsOf(error)) : [];
}

/** Text for a rejected check of the draft `d` (`mail_failed` with `reason`, 422 per field). */
function errorText(error: ApiError, d: MailDraft): string {
  if (error.code === 'mail_failed') {
    const reason = typeof error.body.reason === 'string' ? error.body.reason : null;
    const role = typeof error.body.role === 'string' ? error.body.role : null;
    return reasonText(reason, { tok: d.auth === 'token', wait: retryAfter(error, 0), role });
  }
  const errors = mailErrorsOf(error);
  const first = FIELD_ORDER.find((k) => errors[k]);
  const code = first && errors[first];
  return first && code ? mailErrorText(first, code) : m.error_act();
}

/**
 * Does the rejection say something about the connection (`mail_failed`, 422
 * per field)? Otherwise (429 `rate_limited`, server error, without a response)
 * the status stays, and only an error toast reports it.
 */
const isCheckError = (error: unknown): error is ApiError =>
  error instanceof ApiError && (error.code === 'mail_failed' || fieldNames(error).length > 0);

/** Error toast when the status stays: 429 with wait time, otherwise server error or no connection. */
function toastText(error: unknown): string {
  return isApiError(error, 429) ? m.error_limit({ wait: waitText(retryAfter(error, 60)) }) : failText(error);
}

/** Does the rejection concern the connection (and not only sender, recipient or the one mail)? */
const isConnectionError = (error: unknown): error is ApiError =>
  error instanceof ApiError &&
  (fieldNames(error).some((k) => CONNECTION_FIELDS.includes(k)) ||
    (error.code === 'mail_failed' &&
      typeof error.body.reason === 'string' &&
      CONNECTION_REASONS.has(error.body.reason)));

export class SystemMailPage {
  status = $state<'loading' | 'ready' | 'failed'>('loading');
  /** Reason the first load failed (text of the card). */
  failure = $state('');
  /** Skeleton only after the skeleton delay. */
  readonly loading = new Loading();
  /** Stored state (after `PUT` the response). */
  saved = $state.raw<SystemMail>(NO_MAIL);
  /** Servers of the Stalwart registry for the pick list. */
  servers = $state.raw<MailServer[]>([]);
  /** Inputs on top of the stored state. */
  edits = $state.raw<Partial<MailDraft>>({});
  base = $derived(draftOf(this.saved));
  draft = $derived<MailDraft>({ ...this.base, ...this.edits });
  dirty = $derived(DRAFT_KEYS.some((k) => this.draft[k] !== this.base[k]));
  conn = $derived(connOf(this.draft));
  /** `PUT` is running ("Saving …" in the bar). */
  saving = $state(false);
  /** Errors from a 422 of the server; a field loses its error with the next input. */
  serverErrors = $state.raw<MailErrors>({});
  /**
   * "Save" found a required field missing: from then on the fields the account needs show their error, derived
   * from the current draft and the stored account (the stored server chosen again brings the stored secret back).
   */
  showRequired = $state(false);
  /**
   * Error per field of the chosen sign-in method (a hidden field never blocks "Save"): the check in the browser,
   * then a missing required field, then the server's.
   */
  errors = $derived.by((): MailErrors => {
    const d = this.draft;
    const live = liveErrors(d);
    const required = this.showRequired ? requiredErrors(d, this.saved, this.servers) : {};
    const out: MailErrors = {};
    for (const k of FIELD_ORDER) {
      if (!shownField(k, d.auth)) continue;
      const code = live[k] ?? required[k] ?? this.serverErrors[k];
      if (code) out[k] = code;
    }
    return out;
  });

  /** Connection of the last check and of the last successful one. */
  tested = $state<string | null>(null);
  good = $state<string | null>(null);
  /**
   * Result of the last check with its text; `new` = none yet, `unchecked` =
   * stored but never checked.
   */
  result = $state<'ok' | 'err' | 'new' | 'unchecked'>('new');
  errText = $state('');
  /** "Last checked" in ms. */
  checkedAt = $state<number | null>(null);
  identities = $state.raw<MailIdentity[]>([]);
  /** Running checks: "Test connection" and `PUT` with a new connection. */
  checks = $state(0);
  /** Check running. */
  busy = $derived(this.checks > 0);
  /** Rotation of the icon in degrees. */
  rot = $state(0);
  sending = $state(false);
  /** Recipient of the test email; `null` = the own address. */
  to = $state<string | null>(null);
  showSecret = $state(false);

  key = $derived.by((): MailStatusKey => {
    const connNew = this.conn !== this.tested;
    if (this.busy) return 'busy';
    if (this.good !== null && this.conn === this.good) return 'ok';
    // Without an account "Not set up", but not over the result of a check of the draft.
    if (!connNew && this.saved.status === 'none' && this.result === 'new') return 'none';
    return connNew ? 'new' : this.result;
  });

  #scrollTo: (section: MailSection) => void;
  #seq = 0;
  #disposed = false;
  /** The running `PUT`: checking and sending wait for it, otherwise they would use the old secret. */
  #inflight: Promise<unknown> | null = null;
  /** Counts saves: only the save that is still the latest clears `#inflight`. */
  #saveId = 0;
  /**
   * Counts saves that change the connection: a check that started before one says nothing about the stored
   * credentials any more (the draft's key can be the same, an empty secret field means "the stored one").
   */
  #credSeq = 0;

  constructor(scrollTo: (section: MailSection) => void = () => {}) {
    this.#scrollTo = scrollTo;
  }

  /** Loads the account and the registry's servers; with `unchecked` the page then checks silently. */
  async load(): Promise<void> {
    const seq = ++this.#seq;
    try {
      const [mail, list] = await this.loading.run(
        Promise.all([
          api<SystemMail>('GET', 'admin/system-mail'),
          api<{ servers: MailServer[] }>('GET', 'admin/stalwart/servers'),
        ]),
      );
      if (seq !== this.#seq || this.#disposed || this.saving) return;
      this.servers = list.servers.map(({ id, label, url, host }) => ({ id, label, url, host }));
      this.#adopt(mail);
      this.status = 'ready';
      if (mail.status === 'unchecked') void this.test(true);
    } catch (error) {
      if (seq !== this.#seq || this.#disposed) return;
      // Without a session or role: reload the account, the layout then shows the sign-in or "Access denied".
      lostAccess(error);
      this.failure = loadFailText(error);
      this.status = 'failed';
    }
  }

  /** "Try again" of the card: skeleton again until the response. */
  retry(): Promise<void> {
    this.status = 'loading';
    return this.load();
  }

  /** Take over the stored state with its status and identities (also none: they describe the stored connection). */
  #adopt(view: SystemMail): void {
    this.saved = view;
    const k = connOf(draftOf(view));
    this.tested = k;
    this.result = view.status === 'none' ? 'new' : view.status;
    // The stored status knows no wait time; without `role` "Drafts" applies.
    this.errText =
      view.status === 'err' ? reasonText(view.reason, { tok: view.auth === 'token', role: view.role ?? null }) : '';
    if (view.status === 'ok') this.good = k;
    else if (this.good === k) this.good = null;
    this.checkedAt = view.checked_at ? Date.parse(view.checked_at) : this.checkedAt;
    this.identities = view.identities;
  }

  set<K extends MailDraftKey>(key: K, value: MailDraft[K]): void {
    this.edits = { ...this.edits, [key]: value };
    const field = key as MailField;
    if (this.serverErrors[field]) {
      const rest = { ...this.serverErrors };
      delete rest[field];
      this.serverErrors = rest;
    }
  }

  /** "Discard" (not while saving: the bar blocks it). */
  discard(): void {
    if (this.saving) return;
    this.edits = {};
    this.serverErrors = {};
    this.showRequired = false;
  }

  /** Waits for a running save. */
  async #settle(): Promise<void> {
    if (this.#inflight) await this.#inflight.catch(() => {});
  }

  /**
   * Check failed: status "No connection" with reason and error toast
   * ("Connection failed", or the reason when sending a test email).
   */
  #fail(conn: string, text: string, toast: string = m.email_test_failed()): void {
    this.result = 'err';
    this.errText = text;
    this.tested = conn;
    // If a previously good connection fails, it is no longer "Connected" (`good` becomes null).
    if (this.good === conn) this.good = null;
    this.checkedAt = Date.now();
    ui.toasts.error(toast);
  }

  /** "Test connection"; `quiet` reports only errors. */
  async test(quiet = false): Promise<void> {
    if (this.busy) return;
    // A stored new secret is only in the backend after the save: wait for it (it checks itself).
    await this.#settle();
    if (this.busy || this.#disposed) return;
    const d = this.draft;
    const conn = connOf(d);
    const cred = this.#credSeq;
    const local = precheck(d, this.saved, this.servers);
    if (local) {
      this.#fail(conn, local);
      return;
    }
    this.checks++;
    this.rot += 720;
    try {
      const res = await api<{ identities: MailIdentity[] }>('POST', 'admin/system-mail/test', {
        ...connBody(d),
        ...this.#identityOf(d),
      });
      // A save changed the connection meanwhile: its own check applies, this result is older.
      if (this.#disposed || cred !== this.#credSeq) return;
      this.identities = res.identities;
      this.result = 'ok';
      this.errText = '';
      this.tested = conn;
      this.good = conn;
      this.checkedAt = Date.now();
      if (!quiet) ui.toasts.show(m.email_test_ok());
    } catch (error) {
      if (this.#disposed) return;
      if (lostAccess(error)) return;
      if (cred !== this.#credSeq) return;
      // Without a response, a server error or 429 nothing is known about the connection: the status stays. The
      // silent check without a connection reports nothing, "Not checked" stays.
      if (!isCheckError(error)) {
        if (!quiet || error instanceof ApiError) ui.toasts.error(toastText(error));
        return;
      }
      this.#fail(conn, errorText(error, d));
    } finally {
      this.checks--;
    }
  }

  /**
   * `identity_id` for checking and saving: another account (server, sign-in
   * method, user or new token) might not know the stored identity, and the
   * page has no selection. It then only applies if the last successful check
   * of this connection returned it, otherwise the first one (`null`).
   */
  #identityOf(d: MailDraft): { identity_id?: null } {
    const id = this.saved.identity_id;
    const b = this.base;
    const same =
      d.srv === b.srv && d.auth === b.auth && (d.auth === 'token' ? secretOf(d) === '' : trimWs(d.user) === b.user);
    if (id === null || same) return {};
    return this.good === connOf(d) && this.identities.some((i) => i.id === id) ? {} : { identity_id: null };
  }

  /**
   * "Send" in the Test section: test email with the draft as long as it is
   * checked. The backend signs in first for it; if that fails (reasons of the
   * connection), the check counts as failed and the toast names the reason. If
   * only the mail fails afterwards, the status stays ("… wasn’t sent").
   */
  async send(): Promise<void> {
    if (this.key !== 'ok' || this.sending) return;
    const to = trimWs(this.to ?? session.user?.email ?? '');
    if (!isEmail(to)) {
      ui.toasts.show(m.email_to_invalid());
      return;
    }
    this.sending = true;
    let d = this.draft;
    let cred = this.#credSeq;
    try {
      await this.#settle();
      d = this.draft;
      cred = this.#credSeq;
      if (this.key !== 'ok' || this.#disposed) return;
      await api('POST', 'admin/system-mail/test', { ...draftBody(d), ...this.#identityOf(d), to });
      if (!this.#disposed) ui.toasts.show(m.email_sent({ to }));
    } catch (error) {
      if (this.#disposed) return;
      if (lostAccess(error)) return;
      if (isConnectionError(error)) {
        const text = errorText(error, d);
        // A save changed the connection meanwhile: its own check sets the status, only the toast stays.
        if (cred !== this.#credSeq) ui.toasts.error(text);
        else this.#fail(connOf(d), text, text);
      } else if (error instanceof ApiError && error.code === 'mail_failed') {
        ui.toasts.error(m.email_send_failed({ to }));
      } else if (error instanceof ApiError && fieldNames(error).includes('to')) {
        ui.toasts.error(m.email_to_invalid());
      } else {
        const errors = mailErrorsOf(error);
        if (Object.keys(errors).length > 0) this.#showErrors(errors, d);
        else ui.toasts.error(toastText(error));
      }
    } finally {
      this.sending = false;
    }
  }

  /**
   * Field errors at the fields that are shown (sign-in method of the current draft) and still show the sent value,
   * and the jump to the first one; otherwise a toast.
   */
  #showErrors(errors: MailErrors, sent: MailDraft): void {
    const fields: MailErrors = {};
    const auth = this.draft.auth;
    for (const k of FIELD_ORDER) {
      if (errors[k] && shownField(k, auth) && this.draft[k] === sent[k]) fields[k] = errors[k];
    }
    const section = firstBadSection(fields);
    if (!section) {
      ui.toasts.error(m.error_act());
      return;
    }
    this.serverErrors = { ...this.serverErrors, ...fields };
    this.#scrollTo(section);
  }

  /** "Save"; with a field error or a missing required field only toast and jump. */
  async save(): Promise<void> {
    if (!this.dirty || this.saving) return;
    const d = this.draft;
    const required = requiredErrors(d, this.saved, this.servers);
    const bad = firstBadSection({ ...required, ...this.errors });
    if (bad) {
      if (Object.keys(required).length > 0) this.showRequired = true;
      ui.toasts.show(m.email_check());
      this.#scrollTo(bad);
      return;
    }
    const body: Record<string, unknown> = { ...changedBody(d, this.base), ...this.#identityOf(d) };
    // Only whitespace changed: nothing to send.
    if (Object.keys(body).length === 0) {
      this.edits = {};
      ui.toasts.show(m.email_saved());
      return;
    }
    // A load still running started before this save: its response is older.
    this.#seq++;
    this.saving = true;
    const id = ++this.#saveId;
    const run = this.#put(body, d);
    this.#inflight = run;
    try {
      await run;
    } finally {
      if (this.#saveId === id) this.#inflight = null;
      this.saving = false;
    }
  }

  /** `PUT`; the backend checks a new connection along the way ("Checking …"). */
  async #put(body: Record<string, unknown>, d: MailDraft): Promise<void> {
    const check = touchesConnection(body);
    if (check) {
      this.#credSeq++;
      this.checks++;
      this.rot += 720;
    }
    try {
      const view = await api<SystemMail>('PUT', 'admin/system-mail', body);
      if (this.#disposed) return;
      // Inputs made during the request stay; the sent ones are saved now.
      const rest: Partial<MailDraft> = {};
      for (const k of DRAFT_KEYS) {
        const value = this.edits[k];
        if (value !== undefined && value !== d[k]) Object.assign(rest, { [k]: value });
      }
      this.#adopt(view);
      this.edits = rest;
      this.serverErrors = {};
      this.showRequired = false;
      if (check && view.status === 'err') ui.toasts.error(m.email_saved_conn_failed());
      else ui.toasts.show(m.email_saved());
    } catch (error) {
      if (this.#disposed) return;
      if (lostAccess(error)) return;
      const errors = mailErrorsOf(error);
      if (Object.keys(errors).length > 0) this.#showErrors(errors, d);
      else ui.toasts.error(isApiError(error, 429) ? toastText(error) : failText(error, m.error_act()));
    } finally {
      if (check) this.checks--;
    }
  }

  dispose(): void {
    this.#disposed = true;
  }
}

/** Card "Status"; colours per `key` in the page. */
export interface MailStatusCard {
  key: MailStatusKey;
  icon: IconName;
  title: string;
  sub: string;
}

/** Error state of a field for the page. */
export interface MailFieldState {
  /** Red ring and text below. */
  bad: boolean;
  badA: 'true' | 'false';
  err: string;
}

export interface MailSecretValues extends MailFieldState {
  field: 'pass' | 'token';
  label: string;
  desc: string;
  value: string;
  /** Placeholder while a stored secret of the same sign-in method applies. */
  ph: string;
  type: 'text' | 'password';
  ac: 'off' | 'new-password';
  eyeIcon: IconName;
  eyeTip: string;
  onChange: (value: string) => void;
  onEye: () => void;
}

/** Values of the page. */
export interface SystemMailValues {
  mailTabs: { label: string; onClick: () => void }[];
  mSt: MailStatusCard;
  mailBusy: boolean;
  mailRot: number;
  mailBtn: string;
  mailTest: () => void;
  mailOk: boolean;
  mailNotOk: boolean;
  mailRows: { k: string; v: string; sep: boolean }[];
  mailF: MailDraft;
  mailOn: Record<'user' | 'name' | 'replyTo', (value: string) => void>;
  mailErr: Record<'srv' | 'user' | 'name' | 'replyTo', MailFieldState>;
  /** "Mail server" pick list; only with servers in the registry (`mailSrvHas`). */
  selMailSrv: SelectValues;
  mailSrvHas: boolean;
  mailSrvSub: string;
  mailAuthOpts: SegOption<MailAuth>[];
  mailIsPw: boolean;
  mailSecret: MailSecretValues;
  mailNamePh: string;
  mailFrom: string;
  tgMailSent: { on: boolean; onClick: () => void };
  mailToVal: string;
  onMailTo: (value: string) => void;
  mailSendDis: boolean;
  mailSendLabel: string;
  mailSend: () => void;
  mailToKey: (event: { key: string; preventDefault: () => void }) => void;
  mailDirty: boolean;
  mailSaving: boolean;
  mailSave: () => void;
  mailDiscard: () => void;
}

export interface SystemMailInput extends SelectState<MailPick> {
  /** Clock for "Last checked". */
  now?: number | undefined;
}

export interface SystemMailContext extends SelectContext<MailPick> {
  scrollTo: (section: MailSection) => void;
}

/** Sender of the preview: sender name or name of the identity with its address. */
function fromOf(name: string, id: MailIdentity | undefined, user: string): string {
  const shown = trimWs(name) || id?.name || '';
  const email = id?.email ?? (isEmail(trimWs(user)) ? trimWs(user) : '');
  if (!email) return shown;
  return shown ? `${shown} <${email}>` : email;
}

export function systemMailValues(p: SystemMailPage, input: SystemMailInput, ctx: SystemMailContext): SystemMailValues {
  const d = p.draft;
  const token = d.auth === 'token';
  const server = p.servers.find((s) => s.id === d.srv);
  const host = server?.host ?? '';
  const origin = server ? server.url.replace(/\/+$/, '') : '';
  const key = p.key;
  const ok = key === 'ok';
  const shownHost = host || m.email_host_fallback();
  const card: Record<MailStatusKey, Omit<MailStatusCard, 'key'>> = {
    busy: { icon: 'ti-search', title: m.email_st_busy(), sub: m.email_st_busy_sub({ host: shownHost }) },
    new: { icon: 'ti-plug', title: m.email_st_new(), sub: m.email_st_new_sub() },
    unchecked: { icon: 'ti-plug', title: m.email_st_new(), sub: m.email_st_unchecked_sub() },
    none: { icon: 'ti-mail-off', title: m.email_st_none(), sub: m.email_st_none_sub() },
    ok: { icon: 'ti-circle-check', title: m.email_st_ok(), sub: m.email_st_ok_sub({ host: shownHost }) },
    err: { icon: 'ti-alert-triangle', title: m.email_st_err(), sub: p.errText },
  };
  const errorOf = (k: MailField): MailFieldState => {
    const code = p.errors[k];
    return { bad: !!code, badA: code ? 'true' : 'false', err: code ? mailErrorText(k, code) : '' };
  };
  const id = p.identities.find((i) => i.id === p.saved.identity_id) ?? p.identities[0];
  const stored = storedSecret(d, p.saved);
  const toVal = p.to ?? session.user?.email ?? '';
  const canSend = ok && !p.sending;
  const send = () => void p.send();
  const secretField = token ? 'token' : 'pass';
  const has = p.servers.length > 0;
  const select = selectValues(
    'mailsrv',
    m.email_srv_label(),
    p.servers.map((s) => ({ v: s.id, l: s.label, sub: s.host })),
    d.srv,
    (v) => p.set('srv', v),
    !has,
    input,
    ctx,
  );
  // Without a server of the registry (none chosen, or removed meanwhile) the button asks for one.
  if (!server) select.label = m.email_srv_choose();
  return {
    mailTabs: MAIL_TABS.map(([section, label]) => ({ label: label(), onClick: () => ctx.scrollTo(section) })),
    mSt: { key, ...card[key] },
    mailBusy: p.busy,
    mailRot: p.rot,
    mailBtn: p.busy ? m.email_test_busy() : m.email_test(),
    mailTest: () => void p.test(),
    mailOk: ok,
    mailNotOk: !ok,
    mailRows: [
      [m.email_row_session(), origin ? `${origin}/.well-known/jmap` : ''],
      [m.email_row_account(), token ? m.email_row_token() : trimWs(d.user)],
      [m.email_row_checked(), p.checkedAt === null ? '' : ago(p.checkedAt, input.now)],
    ].map(([k = '', v = ''], i) => ({ k, v, sep: i > 0 })),
    mailF: d,
    mailOn: {
      user: (v) => p.set('user', v),
      name: (v) => p.set('name', v),
      replyTo: (v) => p.set('replyTo', v),
    },
    mailErr: { srv: errorOf('srv'), user: errorOf('user'), name: errorOf('name'), replyTo: errorOf('replyTo') },
    selMailSrv: select,
    mailSrvHas: has,
    mailSrvSub: host ? m.email_srv_sub({ host }) : m.email_srv_desc(),
    mailAuthOpts: segOptions<MailAuth>(
      ['password', 'token'],
      d.auth,
      (a) => (a === 'token' ? m.email_auth_token() : m.email_auth_password()),
      (a) => p.set('auth', a),
    ),
    mailIsPw: !token,
    mailSecret: {
      field: secretField,
      label: token ? m.email_secret_token() : m.email_secret_password(),
      desc: token ? m.email_secret_token_desc() : m.email_secret_password_desc(),
      value: token ? d.token : d.pass,
      ph: stored ? m.email_secret_stored_ph() : '',
      type: p.showSecret ? 'text' : 'password',
      ac: token ? 'off' : 'new-password',
      eyeIcon: p.showSecret ? 'ti-eye-off' : 'ti-eye',
      eyeTip: p.showSecret ? m.email_secret_hide() : m.email_secret_show(),
      onChange: (v) => p.set(secretField, v),
      onEye: () => {
        p.showSecret = !p.showSecret;
      },
      ...errorOf(secretField),
    },
    mailNamePh: id?.name ?? '',
    mailFrom: fromOf(d.name, id, d.user),
    tgMailSent: { on: d.keepSent, onClick: () => p.set('keepSent', !p.draft.keepSent) },
    mailToVal: toVal,
    onMailTo: (v) => {
      p.to = v;
    },
    mailSendDis: !canSend,
    mailSendLabel: p.sending ? m.email_sending() : m.email_send(),
    mailSend: send,
    mailToKey: (event) => {
      if (event.key !== 'Enter') return;
      event.preventDefault();
      send();
    },
    mailDirty: p.dirty,
    mailSaving: p.saving,
    mailSave: () => void p.save(),
    mailDiscard: () => p.discard(),
  };
}
