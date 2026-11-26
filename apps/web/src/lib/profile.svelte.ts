/**
 * Account › Profile against `GET/PATCH/DELETE auth/me` (`api::me`) and the
 * email change of `api::email`: display name and username apply
 * immediately, a new email address in two steps – first the server checks
 * format and availability (`status: reauth_required`), then with the current
 * password and, with multi-factor, a code; afterwards notice and
 * confirmation link go out (`email_sent`). "Confirmation pending" comes from
 * `auth/me` › `email_change`, with "Resend link" (`POST auth/me/email/resend`)
 * and "Discard change" (`DELETE auth/me/email`). "Delete account" asks for
 * the confirmation word and is locked for the only active administrator
 * (`sole_admin`, 409 `last_admin`); afterwards the session is gone and the
 * sign-in opens.
 *
 * The page loads `auth/me` itself (the session's account from `GET session`
 * has no `mfa_enabled`, `email_change` or `sole_admin`); only the latest read
 * writes the state, a change saved in the meantime wins over an older read,
 * and nothing is written once the page is left. After a change of name,
 * username or address the session's account reloads (`session.refreshUser`)
 * so that header and account menu follow.
 *
 * Taken over from Roedy Mail (`apps/web/src/lib/profile.svelte.ts`, delete
 * account from `apps/web/src/lib/general.svelte.ts`; check responses in
 * `#lib/reauth.ts`) without mail accounts and without undo:
 * "Discard change" runs at once and then shows its toast.
 */
import { goto } from '$app/navigation';
import { resolve } from '$app/paths';
import { hideTooltip } from '#lib/actions/tooltip.ts';
import { api, fieldsOf, isApiError, retryAfter } from '#lib/api.ts';
import { isEmail } from '#lib/email.ts';
import { failText, isOffline, loadFailText } from '#lib/errors.ts';
import type { KeyInput } from '#lib/keys.ts';
import { formatOtp } from '#lib/login.svelte.ts';
import { nameError, nameErrorText, normalizeName } from '#lib/name.ts';
import { loginHref } from '#lib/next.ts';
import { m } from '#lib/paraglide/messages.js';
import { CODE_LENGTH, needsReauth, reauthError } from '#lib/reauth.ts';
import { forgetAccount } from '#lib/state/account.ts';
import { initialsOf, session } from '#lib/state/session.svelte.ts';
import { Loading, ui } from '#lib/state/ui.svelte.ts';
import { waitText } from '#lib/wait.ts';

/** Address of the page (`next` of a fresh sign-in). */
export const PROFILE_PATH = '/account/profile';

/** Editable rows. */
export type ProfileField = 'login' | 'email';

/** Pending change of the address (`api::email::open_json`). */
export interface EmailChange {
  email: string;
  expires_at: string;
}

/** User like `user_json` of the backend. */
export interface ProfileUser {
  id: string;
  username: string;
  email: string;
  display_name: string;
  role: 'admin' | 'user';
}

/** Response of `GET auth/me` (`api::me::me`). */
export interface ProfileMe extends ProfileUser {
  mfa_enabled: boolean;
  password_changed_at: string;
  email_change: EmailChange | null;
  /** The only active administrator: cannot delete the account (409 `last_admin`). */
  sole_admin: boolean;
  /** Administrators only: a sending account is stored. */
  system_mail_configured?: boolean;
}

/** Response of `PATCH auth/me`. */
interface ProfileResponse {
  status: 'ok' | 'reauth_required' | 'email_sent';
  user: ProfileUser;
  email_change?: EmailChange;
}

/** "Resend link" sends two mails; after this long the page shows the server's state. */
export const RESEND_TIMEOUT_MS = 60_000;
/** Reading `auth/me` again before "Discard change" and after "Resend link". */
export const READ_TIMEOUT_MS = 15_000;

/** Confirmation value of the API for `DELETE auth/me`, independent of the UI language. */
export const DELETE_CONFIRM = 'delete';

/** User name like the backend (`auth::username::is_valid`). */
const USERNAME = /^[a-z0-9][a-z0-9._-]{1,31}$/;

/** Result of reading `auth/me` again (`#reread`). */
type Read = { me: ProfileMe } | { error: unknown } | null;

export type LoadStatus = 'loading' | 'ready' | 'failed';

export class ProfilePage {
  /** The account from `auth/me`; `null` until the first answer. */
  me = $state<ProfileMe | null>(null);
  status = $state<LoadStatus>('loading');
  /** Reason on the "Couldn’t load" card. */
  loadError = $state('');
  readonly loading = new Loading();

  /** Display name being edited; `null` = reading. */
  nameDraft = $state<string | null>(null);
  /** User name or address being edited. */
  drafts = $state<Partial<Record<ProfileField, string>>>({});
  /** Message below the row. */
  errors = $state<Partial<Record<ProfileField, string>>>({});
  /** New address waits for password and code. */
  ea = $state<{ email: string } | null>(null);
  eaPw = $state('');
  eaOtp = $state('');
  eaShowPw = $state(false);
  eaErr = $state<string | null>(null);
  /** Message after "Resend link". */
  pendErr = $state<string | null>(null);
  saving = $state(false);
  sending = $state(false);
  resending = $state(false);
  /** "Discard change" reloads `auth/me` first, then discards. */
  checking = $state(false);
  deleting = $state(false);

  /** The page is gone: late answers change nothing anymore. */
  #disposed = false;
  /** Counts the writes of `me` and the starts of `load`: a read applies only if nothing happened since it started. */
  #version = 0;

  /** Leaving the page (`onDestroy`). */
  dispose(): void {
    this.#disposed = true;
  }

  /**
   * First load and "Try again": reads `auth/me`. Only the most recent read writes; a change saved while it runs
   * (newer than the read) stays.
   */
  async load(): Promise<void> {
    if (this.#disposed) return;
    const version = ++this.#version;
    if (!this.me) this.status = 'loading';
    try {
      const me = await this.loading.run(api<ProfileMe>('GET', 'auth/me'));
      if (this.#disposed || version !== this.#version) return;
      this.#write(me);
      this.status = 'ready';
    } catch (error) {
      if (this.#disposed || version !== this.#version) return;
      // Without a session the account reloads, the layout then shows the sign-in.
      if (isApiError(error, 401)) this.#syncSession();
      // A reload in the background (after a change): the page keeps what it shows.
      if (this.me) return;
      this.loadError = loadFailText(error);
      this.status = 'failed';
    }
  }

  /**
   * The session's account changed (another tab signed in someone else): the page shows the new account's profile.
   */
  follow(id: string | undefined): void {
    const me = this.me;
    if (!me || !id || id === me.id) return;
    this.discardDrafts();
    this.cancelAuth();
    this.pendErr = null;
    this.me = null;
    void this.load();
  }

  get mfa(): boolean {
    return this.me?.mfa_enabled ?? false;
  }

  /** "Send confirmation link" ready: password and, with multi-factor, a code. */
  get eaReady(): boolean {
    return !!this.eaPw && (!this.mfa || !!this.eaOtp);
  }

  /** Unsaved changes: an input differs from the account. */
  get dirty(): boolean {
    const user = this.me;
    if (!user) return false;
    const name = this.nameDraft;
    if (name !== null && normalizeName(name) !== user.display_name) return true;
    const current: Record<ProfileField, string> = { login: user.username, email: user.email };
    return (Object.entries(this.drafts) as [ProfileField, string][]).some(([k, v]) => v.trim() !== current[k]);
  }

  // ---- Display name and rows ----

  editName(): void {
    this.nameDraft = this.me?.display_name ?? '';
  }

  setName(value: string): void {
    this.nameDraft = value;
  }

  /** Leaving the field: empty or unchanged goes back to reading. */
  blurName(): void {
    if (this.nameDraft === null) return;
    const name = normalizeName(this.nameDraft);
    if (!name || name === this.me?.display_name) this.nameDraft = null;
  }

  nameKey(event: KeyInput): void {
    if (event.key === 'Enter') {
      event.preventDefault();
      void this.save();
    } else if (event.key === 'Escape') {
      event.preventDefault();
      this.nameDraft = null;
    }
  }

  edit(field: ProfileField): void {
    this.drafts = { ...this.drafts, [field]: this.#current(field) };
  }

  setField(field: ProfileField, value: string): void {
    this.drafts = { ...this.drafts, [field]: value };
    this.#clearError(field);
  }

  blurField(field: ProfileField): void {
    const value = this.drafts[field]?.trim();
    if (value !== undefined && (!value || value === this.#current(field))) this.#drop(field);
  }

  fieldKey(field: ProfileField, event: KeyInput): void {
    if (event.key === 'Enter') {
      event.preventDefault();
      void this.save();
    } else if (event.key === 'Escape') {
      event.preventDefault();
      this.#drop(field);
    }
  }

  /** "Discard" of the "Unsaved changes" bar. */
  discardDrafts(): void {
    this.nameDraft = null;
    this.drafts = {};
    this.errors = {};
  }

  // ---- Saving ----

  /**
   * Checks like the backend and saves display name and user name; the server checks a new address, then the
   * password prompt follows.
   */
  async save(): Promise<void> {
    const user = this.me;
    if (!user || this.saving) return;
    const body: Record<string, string> = {};
    if (this.nameDraft !== null) {
      const name = normalizeName(this.nameDraft);
      if (!name) {
        ui.toasts.show(m.profile_name_required());
        return;
      }
      const bad = nameError(name);
      if (bad) {
        ui.toasts.error(nameErrorText(bad));
        return;
      }
      if (name !== user.display_name) body.display_name = name;
    }
    const errors: Partial<Record<ProfileField, string>> = {};
    const login = this.drafts.login?.trim();
    if (login !== undefined) {
      if (!USERNAME.test(login)) errors.login = m.profile_username_invalid();
      else if (login !== user.username) body.username = login;
    }
    const email = this.drafts.email?.trim();
    if (email !== undefined) {
      if (!isEmail(email)) errors.email = m.profile_email_invalid();
      // A different case is also a new address (like the backend).
      else if (email !== user.email) body.email = email;
    }
    if (errors.login || errors.email) {
      this.errors = errors;
      return;
    }
    if (Object.keys(body).length === 0) {
      this.discardDrafts();
      return;
    }
    this.saving = true;
    try {
      const res = await api<ProfileResponse>('PATCH', 'auth/me', body);
      this.#applyUser(res.user);
      this.discardDrafts();
      if (body.display_name !== undefined || body.username !== undefined) {
        ui.toasts.show(m.profile_saved());
        this.#syncSession();
      }
      if (res.status === 'reauth_required' && body.email) this.#openAuth(body.email);
    } catch (error) {
      this.#saveFailed(error);
    } finally {
      this.saving = false;
    }
  }

  #saveFailed(error: unknown): void {
    if (isApiError(error, 422, 'email_taken')) {
      this.errors = { ...this.errors, email: m.profile_email_taken() };
      return;
    }
    if (isApiError(error, 422, 'invalid')) {
      const fields = fieldsOf(error);
      const login = fields.username?.code;
      // A taken user name below the row like a taken address; the input stays.
      if (login) {
        this.errors = {
          ...this.errors,
          login: login === 'taken' ? m.profile_username_taken() : m.profile_username_invalid(),
        };
      }
      if (fields.email) this.errors = { ...this.errors, email: m.profile_email_invalid() };
      const name = fields.display_name?.code;
      if (name === 'too_long' || name === 'invalid') ui.toasts.error(nameErrorText(name));
      if (login || fields.email || name === 'too_long' || name === 'invalid') return;
    }
    this.#unexpected(error);
  }

  // ---- Password prompt ----

  setEaPw(value: string): void {
    this.eaPw = value;
    this.eaErr = null;
  }

  setEaOtp(value: string): void {
    this.eaOtp = formatOtp(value);
    this.eaErr = null;
  }

  toggleEaEye(): void {
    this.eaShowPw = !this.eaShowPw;
  }

  eaKey(event: KeyInput): void {
    if (event.key === 'Enter') {
      event.preventDefault();
      void this.sendLink();
    } else if (event.key === 'Escape') {
      event.preventDefault();
      this.cancelAuth();
    }
  }

  cancelAuth(): void {
    this.ea = null;
    this.eaPw = '';
    this.eaOtp = '';
    this.eaShowPw = false;
    this.eaErr = null;
  }

  /** "Send confirmation link": the server checks password and code, then both mails go out. */
  async sendLink(): Promise<void> {
    const ea = this.ea;
    // During "Resend link" both would replace the link: one after the other.
    if (!ea || this.sending || this.resending || !this.eaReady) return;
    const mfa = this.mfa;
    if (mfa && this.eaOtp.length !== CODE_LENGTH) {
      this.eaErr = m.reauth_code_short();
      return;
    }
    this.sending = true;
    try {
      const body = { email: ea.email, password: this.eaPw, ...(mfa ? { code: this.eaOtp } : {}) };
      const res = await api<ProfileResponse>('PATCH', 'auth/me', body);
      this.cancelAuth();
      this.pendErr = null;
      if (res.status === 'email_sent') {
        this.#applyUser(res.user, res.email_change ?? null);
        ui.toasts.show(m.profile_link_sent());
      } else {
        // The address already applies in the meantime (confirmed elsewhere): no link, reload the account.
        this.#applyUser(res.user);
        void this.load();
        this.#syncSession();
      }
    } catch (error) {
      // Cancelled in the meantime: the message would apply to fields that no longer exist.
      if (this.ea === ea) this.#sendFailed(error, ea.email, mfa);
    } finally {
      this.sending = false;
    }
  }

  #sendFailed(error: unknown, email: string, mfa: boolean): void {
    const reauth = reauthError(error);
    if (reauth) {
      this.eaErr = reauth.text;
      if (reauth.clear === 'password') this.eaPw = '';
      if (reauth.clear === 'code') this.eaOtp = '';
      // Multi-factor turned on in the meantime (another tab): reload the account, then the code field appears.
      if (!mfa && isApiError(error, 422, 'invalid') && fieldsOf(error).code) void this.load();
      return;
    }
    if (isApiError(error, 502, 'mail_failed')) {
      // The code is used up; the password stays.
      this.eaErr = mfa ? m.profile_mail_failed_code() : m.profile_mail_failed();
      this.eaOtp = '';
      return;
    }
    if (isApiError(error, 422, 'email_taken')) {
      // As in the first step, in the row: back to entering the address.
      this.cancelAuth();
      this.drafts = { ...this.drafts, email };
      this.errors = { ...this.errors, email: m.profile_email_taken() };
      return;
    }
    if (isApiError(error, 409, 'email_change_discarded')) this.eaOtp = '';
    this.#unexpected(error);
  }

  // ---- Confirmation pending ----

  /** "Resend link": new link with a new 24 hours; if it fails, everything stays. */
  async resend(): Promise<void> {
    // The account and change the request is for; the answer applies only to them.
    const before = this.me;
    if (!before?.email_change || this.resending || this.checking || this.sending) return;
    this.resending = true;
    this.pendErr = null;
    try {
      const open = await api<EmailChange>('POST', 'auth/me/email/resend', undefined, {
        signal: AbortSignal.timeout(RESEND_TIMEOUT_MS),
      });
      // A newer link (or another account) in the meantime: its state and notice stay.
      const me = this.me;
      if (!me || !this.#unchanged(before)) return;
      this.#write({ ...me, email_change: open });
      ui.toasts.show(m.profile_link_resent());
    } catch (error) {
      // A newer link (or another account) in the meantime: the message would not apply to it.
      if (!this.#unchanged(before)) return;
      if (isApiError(error, 502, 'mail_failed')) {
        this.pendErr =
          error.body.previous_link_valid === true ? m.profile_resend_failed_valid() : m.profile_resend_failed();
      } else if (isApiError(error, 422, 'email_taken')) {
        this.pendErr = m.profile_email_taken();
      } else if (isApiError(error, 429)) {
        // Limit `auth.email.send`.
        this.pendErr = m.error_limit({ wait: waitText(retryAfter(error)) });
      } else if (isApiError(error, 404) || isApiError(error, 409, 'email_change_discarded')) {
        // No pending change anymore (confirmed or discarded elsewhere, also while sending).
        await this.#gone(before);
      } else {
        // No answer in time: the server may still have sent the link, so show its state.
        if (isOffline(error)) await this.#reread(before);
        this.#unexpected(error);
      }
    } finally {
      this.resending = false;
    }
  }

  /**
   * "Discard change": reads `auth/me` first, so that only the change the user sees is discarded – one discarded
   * or confirmed elsewhere is reported instead, one that replaced it in the meantime stays. Without an answer the
   * change stays and the error toast appears.
   */
  async discardChange(): Promise<void> {
    const before = this.me;
    // During "Resend link" its response would set the change right back.
    if (!before?.email_change || this.resending || this.checking) return;
    const clicked = before.email_change;
    this.checking = true;
    let read: Read;
    try {
      read = await this.#reread(before);
      if (!read) return;
      if ('error' in read) {
        if (!isApiError(read.error, 401)) ui.toasts.error(failText(read.error));
        return;
      }
      // Gone or replaced by a different change: show it, discard nothing; the clicked one is gone.
      if (read.me.email_change?.email !== clicked.email) {
        this.#reportGone(clicked, read.me);
        return;
      }
      // A new link is being sent: discarding now would remove the new change.
      if (this.sending) return;
      const shown = this.me;
      if (!shown || shown.email_change?.email !== clicked.email) return;
      try {
        await api('DELETE', 'auth/me/email');
      } catch (error) {
        // Already discarded or confirmed: nothing pending anymore.
        if (!isApiError(error, 404)) {
          if (!this.#disposed) void this.load();
          this.#unexpected(error);
          return;
        }
      }
      this.pendErr = null;
      const now = this.me;
      if (now && now.id === shown.id && now.email_change?.email === clicked.email) {
        this.#write({ ...now, email_change: null });
      }
      ui.toasts.show(m.profile_discarded());
    } finally {
      this.checking = false;
    }
  }

  // ---- Delete account ----

  /** "Delete account": asks with the confirmation word; locked for the only administrator. */
  deleteAccount(): void {
    const me = this.me;
    if (!me || me.sole_admin || this.deleting) return;
    ui.ask({
      title: m.profile_delete_title(),
      text: m.profile_delete_text({ email: me.email }),
      icon: 'ti-trash',
      ok: m.profile_delete(),
      danger: true,
      word: m.profile_delete_word(),
      run: () => void this.#delete(),
    });
  }

  async #delete(): Promise<void> {
    if (this.deleting) return;
    this.deleting = true;
    try {
      await api('DELETE', 'auth/me', { confirm: DELETE_CONFIRM });
    } catch (error) {
      this.deleting = false;
      if (isApiError(error, 409, 'last_admin')) {
        // Meanwhile no other administrator is active any more: from now on the notice instead of the button.
        const me = this.me;
        if (me) this.#write({ ...me, sole_admin: true });
        ui.toasts.error(m.profile_delete_last());
        return;
      }
      this.#unexpected(error);
      return;
    }
    this.deleting = false;
    // Like signing out: the server deleted the account and the cookie.
    hideTooltip();
    forgetAccount();
    await goto(resolve('/login'), { replace: true });
  }

  // ---- internal ----

  /** Writes `me` unless the page is gone; older reads that are still running no longer apply. */
  #write(me: ProfileMe): void {
    if (this.#disposed) return;
    this.#version++;
    this.me = me;
  }

  /** Account, address and pending change are still those of `before`. */
  #unchanged(before: ProfileMe): boolean {
    const now = this.me;
    return (
      !!now &&
      now.id === before.id &&
      now.email === before.email &&
      now.email_change?.email === before.email_change?.email &&
      now.email_change?.expires_at === before.email_change?.expires_at
    );
  }

  /**
   * Reads `auth/me` for "Confirmation pending" and applies address, pending change and `sole_admin` (other fields
   * may have been saved in the meantime). `null`: the page changed in the meantime (another account, a new link,
   * left), the newer state stays and decides; `error`: no answer, nothing changed.
   */
  async #reread(before: ProfileMe): Promise<Read> {
    let read: ProfileMe;
    try {
      read = await api<ProfileMe>('GET', 'auth/me', undefined, { signal: AbortSignal.timeout(READ_TIMEOUT_MS) });
    } catch (error) {
      if (isApiError(error, 401)) this.#syncSession();
      return !this.#disposed && this.#unchanged(before) ? { error } : null;
    }
    const now = this.me;
    if (this.#disposed || !now || !this.#unchanged(before)) return null;
    if (read.id !== before.id) {
      this.#syncSession();
      return null;
    }
    const merged = { ...now, email: read.email, email_change: read.email_change, mfa_enabled: read.mfa_enabled };
    this.#write(merged);
    // Confirmed elsewhere: header and account menu show the new address.
    if (read.email !== before.email) this.#syncSession();
    return { me: merged };
  }

  /** "Resend link" found no pending change anymore: reload the account, then report it. */
  async #gone(before: ProfileMe): Promise<void> {
    const read = await this.#reread(before);
    if (!read) return;
    if ('error' in read) {
      if (isApiError(read.error, 401)) return;
      // Without an answer the 404 still counts: nothing is pending anymore.
      const now = this.me;
      if (!now) return;
      const gone = { ...now, email_change: null };
      this.#write(gone);
      this.#reportGone(before.email_change, gone);
    } else if (read.me.email_change?.email !== before.email_change?.email) {
      // Also with a new change from elsewhere: it shows, the resent one is gone.
      this.#reportGone(before.email_change, read.me);
    }
  }

  /**
   * "Confirmation pending" is gone: discarded elsewhere gives the regular toast "… has since been discarded";
   * confirmed elsewhere (the address applies) no toast.
   */
  #reportGone(open: EmailChange | null | undefined, user: ProfileMe): void {
    this.pendErr = null;
    if (open && user.email === open.email) return;
    ui.toasts.show(m.profile_email_gone());
  }

  #current(field: ProfileField): string {
    const user = this.me;
    return field === 'login' ? (user?.username ?? '') : (user?.email ?? '');
  }

  #drop(field: ProfileField): void {
    const drafts = { ...this.drafts };
    delete drafts[field];
    this.drafts = drafts;
    this.#clearError(field);
  }

  #clearError(field: ProfileField): void {
    if (this.errors[field] === undefined) return;
    const errors = { ...this.errors };
    delete errors[field];
    this.errors = errors;
  }

  #openAuth(email: string): void {
    this.cancelAuth();
    this.ea = { email };
  }

  /** Takes over the response with the user (`user_json`); the other fields of `auth/me` stay. */
  #applyUser(user: ProfileUser, emailChange?: EmailChange | null): void {
    const now = this.me;
    if (!now || now.id !== user.id) return;
    this.#write({ ...now, ...user, ...(emailChange === undefined ? {} : { email_change: emailChange }) });
  }

  /** Reloads the session's account (header, account menu); without a session the layout shows the sign-in. */
  #syncSession(): void {
    void session.refreshUser().catch(() => {});
  }

  /**
   * Without a text of its own: error toast; a change discarded in the meantime (`email_change_discarded`) with its
   * own text as a regular toast and the account reloaded; a fresh sign-in asked for leads to it. Without a session
   * the account reloads, the layout then shows the sign-in.
   */
  #unexpected(error: unknown): void {
    if (needsReauth(error)) {
      // eslint-disable-next-line svelte/no-navigation-without-resolve -- `/login` with the fixed `next` of this page
      void goto(loginHref(PROFILE_PATH, true));
      return;
    }
    const gone = isApiError(error, 409, 'email_change_discarded');
    if (gone && !this.#disposed) void this.load();
    if (isApiError(error, 401)) this.#syncSession();
    if (gone) ui.toasts.show(m.profile_email_gone());
    else if (!isApiError(error, 401)) ui.toasts.error(failText(error));
  }
}

// Values of the page

export interface AcctValues {
  name: string;
  initials: string;
  sub: string;
}

export interface AcctRowValues {
  k: string;
  v: string;
  type: 'text' | 'email';
  canEdit: boolean;
  noEdit: boolean;
  read: boolean;
  editing: boolean;
  val: string;
  editLabel: string;
  err: string;
  hasErr: boolean;
  /** New address waits for password and code ("Email" row only). */
  auth: boolean;
  /** Confirmation pending ("Email" row only). */
  pend: boolean;
  pendEmail: string;
  pendErr: string;
  hasPendErr: boolean;
  /** Response pending ("Resend link", "Discard change", or a new link). */
  busy: boolean;
  edit: () => void;
  onChange: (value: string) => void;
  onBlur: () => void;
  onKey: (event: KeyInput) => void;
  resend: () => void;
  discard: () => void;
}

export interface EaFieldValues {
  label: string;
  desc: string;
  /** `autocomplete`. */
  ac: 'current-password' | 'one-time-code';
  /** `inputmode`. */
  im: 'text' | 'numeric';
  type: 'text' | 'password';
  value: string;
  focus: boolean;
  hasEye: boolean;
  eyeIcon: 'ti-eye' | 'ti-eye-off';
  eyeTip: string;
  onChange: (value: string) => void;
  onEye?: () => void;
}

export interface DeleteValues {
  email: string;
  sub: string;
  /** Only active administrator: notice with "Manage users", button locked. */
  soleAdm: boolean;
  tip: string | undefined;
  label: string;
  /** `aria-disabled` (locked or deleting). */
  dis: boolean;
  busy: boolean;
  onClick: () => void;
}

export interface ProfileValues {
  acct: AcctValues;
  acctNameRead: boolean;
  acctNameEditing: boolean;
  acctNameVal: string;
  acctNameEdit: () => void;
  onNameDraft: (value: string) => void;
  onNameBlur: () => void;
  onAcctNameKey: (event: KeyInput) => void;
  acctRows: AcctRowValues[];
  eaIntro: string;
  eaFields: EaFieldValues[];
  eaKey: (event: KeyInput) => void;
  eaMsg: string;
  hasEaMsg: boolean;
  /** "Send confirmation link" still disabled (`aria-disabled`). */
  eaDis: boolean;
  /** Response pending. */
  eaBusy: boolean;
  eaGo: () => void;
  eaCancel: () => void;
  nameDirty: boolean;
  acctSave: () => void;
  acctDiscard: () => void;
  del: DeleteValues;
}

/** Values of the page; `isMobile` decides the tooltip of the locked "Delete account". */
export function profileValues(p: ProfilePage, isMobile = false): ProfileValues {
  const user = p.me;
  const name = user?.display_name ?? '';
  const username = user?.username ?? '';
  const pending = user?.email_change ?? null;
  const rows: [ProfileField | null, string, string][] = [
    ['login', m.profile_username(), username],
    ['email', m.profile_email(), user?.email ?? ''],
    [null, m.profile_login(), m.profile_login_password()],
  ];
  const mfa = p.mfa;
  const pwField: EaFieldValues = {
    label: m.reauth_password_label(),
    desc: m.reauth_password_desc(),
    ac: 'current-password',
    im: 'text',
    type: p.eaShowPw ? 'text' : 'password',
    value: p.eaPw,
    focus: true,
    hasEye: true,
    eyeIcon: p.eaShowPw ? 'ti-eye-off' : 'ti-eye',
    eyeTip: p.eaShowPw ? m.password_hide() : m.password_show(),
    onChange: (value) => p.setEaPw(value),
    onEye: () => p.toggleEaEye(),
  };
  const codeField: EaFieldValues = {
    label: m.reauth_code_label(),
    desc: m.reauth_code_desc(),
    ac: 'one-time-code',
    im: 'numeric',
    type: 'text',
    value: p.eaOtp,
    focus: false,
    hasEye: false,
    eyeIcon: 'ti-eye',
    eyeTip: '',
    onChange: (value) => p.setEaOtp(value),
  };
  const sole = user?.sole_admin ?? false;
  return {
    acct: { name, initials: initialsOf(name), sub: username },
    acctNameRead: p.nameDraft === null,
    acctNameEditing: p.nameDraft !== null,
    acctNameVal: p.nameDraft ?? name,
    acctNameEdit: () => p.editName(),
    onNameDraft: (value) => p.setName(value),
    onNameBlur: () => p.blurName(),
    onAcctNameKey: (event) => p.nameKey(event),
    acctRows: rows.map(([id, k, v]) => {
      const draft = id ? p.drafts[id] : undefined;
      const auth = id === 'email' && p.ea !== null;
      const pend = id === 'email' && pending !== null && !auth;
      const err = (id && p.errors[id]) || '';
      const pendErr = pend ? (p.pendErr ?? '') : '';
      return {
        k,
        v: auth && p.ea ? p.ea.email : v,
        type: id === 'email' ? 'email' : 'text',
        canEdit: id !== null && !auth,
        noEdit: id === null || auth,
        read: draft === undefined,
        editing: draft !== undefined && !auth,
        val: draft ?? v,
        editLabel: m.profile_edit_row({ label: k }),
        err,
        hasErr: !!err,
        auth,
        pend,
        pendEmail: pend && pending ? pending.email : '',
        pendErr,
        hasPendErr: !!pendErr,
        busy: pend && (p.resending || p.checking || p.sending),
        edit: () => id && p.edit(id),
        onChange: (value: string) => id && p.setField(id, value),
        onBlur: () => id && p.blurField(id),
        onKey: (event: KeyInput) => id && p.fieldKey(id, event),
        resend: () => void p.resend(),
        discard: () => void p.discardChange(),
      };
    }),
    eaIntro: mfa ? m.profile_ea_intro_code() : m.profile_ea_intro(),
    eaFields: mfa ? [pwField, codeField] : [pwField],
    eaKey: (event) => p.eaKey(event),
    eaMsg: p.eaErr ?? '',
    hasEaMsg: !!p.eaErr,
    eaDis: !p.eaReady,
    eaBusy: p.sending || p.resending,
    eaGo: () => void p.sendLink(),
    eaCancel: () => p.cancelAuth(),
    nameDirty: p.dirty,
    acctSave: () => void p.save(),
    acctDiscard: () => p.discardDrafts(),
    del: {
      email: user?.email ?? '',
      sub: sole ? m.profile_delete_sole() : m.profile_delete_sub(),
      soleAdm: sole,
      tip: sole && !isMobile ? m.profile_delete_tip() : undefined,
      label: p.deleting ? m.profile_delete_busy() : m.profile_delete(),
      dis: sole || p.deleting,
      busy: p.deleting,
      onClick: () => p.deleteAccount(),
    },
  };
}
