/**
 * Account › Security against the API: change password (`PUT auth/password`),
 * add and remove authenticator apps (`auth/totp/devices`), new recovery codes
 * (`POST auth/recovery-codes`) and signed-in devices (`auth/sessions`).
 *
 * Whatever changes the sign-in asks for the current password (and a code
 * with an authenticator app) in the page itself; the server checks the
 * password before the code (`api::me::Reauth`). The last app may be removed:
 * multi-factor is then off and the recovery codes are dropped. Changing the
 * password ends all other sessions. "Sign out" (a device) goes immediately,
 * "Sign out all others" after a confirmation – the hub's toasts have no undo.
 * Errors without a text of their own show the generic error toast.
 *
 * Request sequencing: only the latest load writes state; a load that started
 * before a change confirmed by the server (app added or removed, devices
 * signed out, password changed) is discarded and loads again; late answers to
 * a closed prompt or a left page change nothing.
 *
 * Taken over from Roedy Mail (`apps/web/src/lib/security.svelte.ts`; check
 * responses in `#lib/reauth.ts`, time spans in `#lib/ago.ts`) without the
 * kept data of the session (`Kept`), the undo of the toasts, place and
 * installed app of a session and the date format setting (the date follows
 * the language).
 */
import { SvelteSet } from 'svelte/reactivity';
import { ago } from '#lib/ago.ts';
import { api, fieldsOf, isApiError, retryAfter } from '#lib/api.ts';
import { copyText } from '#lib/clipboard.svelte.ts';
import { downloadCodes } from '#lib/download.ts';
import { failText, loadFailText } from '#lib/errors.ts';
import type { IconName } from '#lib/icons.ts';
import type { KeyInput } from '#lib/keys.ts';
import { intlLocale } from '#lib/locale.ts';
import { formatOtp } from '#lib/login.svelte.ts';
import { m } from '#lib/paraglide/messages.js';
import { PW_RULES, tooLong, type PasswordRuleId } from '#lib/password.ts';
import { qrSymbol, type QrSymbol } from '#lib/qr.ts';
import { CODE_LENGTH, needsReauth, reauthError, type ReauthError } from '#lib/reauth.ts';
import { session } from '#lib/state/session.svelte.ts';
import { Loading, ui } from '#lib/state/ui.svelte.ts';
import { waitText } from '#lib/wait.ts';

export type ListStatus = 'loading' | 'ready' | 'failed';
/** Sections of the page. */
export type Section = 'password' | '2fa' | 'sessions';

/** Authenticator app from `GET auth/totp/devices`. */
export interface TotpDevice {
  id: string;
  name: string;
  created_at: string;
  last_used_at: string | null;
}

/** State of the recovery codes. */
export interface RecoverySummary {
  total: number;
  unused: number;
  created_at: string;
}

/** Response with devices (`api::mfa::overview`); new codes only with the first app and when generating. */
export interface MfaOverview {
  devices: TotpDevice[];
  recovery: RecoverySummary | null;
  recovery_codes?: string[] | null;
}

/** Session from `GET auth/sessions` (`api::sessions::session_json`). */
export interface ListedSession {
  id: string;
  current: boolean;
  /** Browser and system in the account's language, e.g. "Firefox on Linux". */
  device: string;
  device_type: 'desktop' | 'mobile' | 'tablet';
  /** Shortened IP address ("198.51.x.x"). */
  ip: string | null;
  created_at: string;
  last_seen_at: string;
}

/** The part of `GET auth/me` the page reads. */
interface OwnAccount {
  password_changed_at: string | null;
}

/** Started enrolment (`POST auth/totp/devices/enroll`). */
interface Enrollment {
  id: string;
  /** Base32 without spaces. */
  secret: string;
  uri: string;
}

/** Add or remove an app. */
export type TotpMode = 'setup' | 'off';

/** Maximum length of an app's name as in the backend (`MAX_NAME` in `api::mfa`). */
export const MAX_APP_NAME = 64;

/**
 * Characters an app name must not contain, checked before anything is sent:
 * control characters, line and paragraph separators and invisible format
 * characters (soft hyphen, direction marks and overrides, zero width, BOM).
 * The name is checked as typed, so a tab or line break at either end counts
 * too. The server (`api::mfa::check_name`) stays the fallback.
 */
const APP_NAME_BAD = /[\p{Cc}\u00AD\u061C\u180E\u200B-\u200F\u2028-\u202E\u2060-\u206F\uFEFF]/u;

/** Time from the API in ms; now without a valid value. */
function ms(value: string | null | undefined, now: number): number {
  const t = value ? Date.parse(value) : NaN;
  return Number.isNaN(t) ? now : t;
}

/** Date in the language of the interface ("1 Oct 2026", "01.10.2026"). */
export function securityDate(t: number, timeZone?: string): string {
  return new Intl.DateTimeFormat(intlLocale(), { dateStyle: 'medium', ...(timeZone ? { timeZone } : {}) }).format(t);
}

export interface SecurityHooks {
  /**
   * The server asks for a fresh sign-in (`reauth_required`): the page sends the user to
   * `/login?reauth=1&next=/account/security`.
   */
  reauth?: () => void;
}

export class SecurityPage {
  status = $state<ListStatus>('loading');
  /** Reason the first load failed (text of the card). */
  failure = $state('');
  /** Skeleton only after 150 ms. */
  loading = new Loading();
  devices = $state<TotpDevice[]>([]);
  recovery = $state<RecoverySummary | null>(null);
  sessions = $state<ListedSession[]>([]);
  /** Last password change (`GET auth/me`). */
  changedAt = $state<string | null>(null);
  /** Sessions whose sign-out is on its way (hidden). */
  readonly hidden = new SvelteSet<string>();

  // Change password
  pwCur = $state('');
  pwOtp = $state('');
  pwNew = $state('');
  pwConf = $state('');
  pwShowCur = $state(false);
  pwShowNew = $state(false);
  pwErr = $state<string | null>(null);
  pwBusy = $state(false);

  // Add or remove an authenticator app
  totpMode = $state<TotpMode | null>(null);
  totpTarget = $state<string | null>(null);
  enrollment = $state<Enrollment | null>(null);
  tName = $state('');
  tPw = $state('');
  tCode = $state('');
  tErr = $state<string | null>(null);
  /** Name of the app with invisible characters or too long (`fields.name`). */
  tNameErr = $state<string | null>(null);
  /** Enrolment restarted in another window or expired: "Start over". */
  tStale = $state(false);
  tBusy = $state(false);
  qr = $derived<QrSymbol | null>(this.enrollment ? qrSymbol(this.enrollment.uri) : null);

  // Recovery codes
  /** Codes just generated, shown only once. */
  codes = $state<string[] | null>(null);
  rcForm = $state(false);
  rcPw = $state('');
  rcCode = $state('');
  rcErr = $state<string | null>(null);
  rcBusy = $state(false);

  #seq = 0;
  #enrollSeq = 0;
  #rcSeq = 0;
  /** Counts changes confirmed by the server: a load that started before one does not apply. */
  #changes = 0;
  #disposed = false;
  readonly #hooks: SecurityHooks;

  constructor(hooks: SecurityHooks = {}) {
    this.#hooks = hooks;
  }

  get mfa(): boolean {
    return this.devices.length > 0;
  }

  get pwReady(): boolean {
    return !!(this.pwCur && this.pwNew && this.pwConf && (!this.mfa || this.pwOtp));
  }

  /** Loads apps, codes, devices and the last password change; after the first load the page stays as it is on an error. */
  async load(): Promise<void> {
    const seq = ++this.#seq;
    const changes = this.#changes;
    try {
      const [mfa, list, me] = await this.loading.run(
        Promise.all([
          api<MfaOverview>('GET', 'auth/totp/devices'),
          api<{ sessions: ListedSession[] }>('GET', 'auth/sessions'),
          api<OwnAccount>('GET', 'auth/me'),
        ]),
      );
      if (seq !== this.#seq || this.#disposed) return;
      // Changed in the meantime: the response is older, so load again.
      if (changes !== this.#changes) {
        void this.load();
        return;
      }
      this.devices = mfa.devices;
      this.recovery = mfa.recovery;
      this.sessions = list.sessions;
      this.changedAt = me.password_changed_at ?? null;
      this.status = 'ready';
    } catch (error) {
      if (seq !== this.#seq || this.#disposed) return;
      if (isApiError(error, 401)) void session.refreshUser().catch(() => {});
      if (this.status !== 'ready') {
        this.failure = loadFailText(error);
        this.status = 'failed';
      } else if (!isApiError(error, 401)) {
        ui.toasts.error(failText(error, m.security_list_failed()));
      }
    }
  }

  /** "Try again" of the card. */
  retry(): void {
    this.status = 'loading';
    void this.load();
  }

  /** Leave the page: late responses change nothing any more. */
  dispose(): void {
    this.#disposed = true;
    this.#seq++;
    this.#enrollSeq++;
    this.#rcSeq++;
  }

  // ---- Change password ----

  setPw(field: 'pwCur' | 'pwNew' | 'pwConf', value: string): void {
    this[field] = value;
    this.pwErr = null;
  }

  setPwOtp(value: string): void {
    this.pwOtp = formatOtp(value);
    this.pwErr = null;
  }

  async changePassword(): Promise<void> {
    if (!this.pwReady || this.pwBusy) return;
    const broken = PW_RULES.find((rule) => !rule.test(this.pwNew));
    if (broken) {
      this.pwErr = broken.error();
      return;
    }
    if (tooLong(this.pwNew)) {
      this.pwErr = m.password_too_long();
      return;
    }
    if (this.pwNew !== this.pwConf) {
      this.pwErr = m.security_pw_mismatch();
      return;
    }
    const mfa = this.mfa;
    if (mfa && this.pwOtp.length !== CODE_LENGTH) {
      this.pwErr = m.reauth_code_short();
      return;
    }
    this.pwBusy = true;
    try {
      const body = { current_password: this.pwCur, new_password: this.pwNew, ...(mfa ? { code: this.pwOtp } : {}) };
      const res = await api<{ password_changed_at: string; sessions_ended: number }>('PUT', 'auth/password', body);
      if (this.#disposed) return;
      this.pwCur = this.pwNew = this.pwConf = this.pwOtp = '';
      this.pwShowCur = this.pwShowNew = false;
      this.pwErr = null;
      // All other sessions are ended (`sessions_ended`), open links from mails discarded.
      this.#changes++;
      this.sessions = this.sessions.filter((s) => s.current);
      this.changedAt = res.password_changed_at;
      ui.toasts.show(res.sessions_ended > 0 ? m.security_pw_done_others() : m.security_pw_done());
    } catch (error) {
      if (!this.#disposed) this.#passwordFailed(error);
    } finally {
      this.pwBusy = false;
    }
  }

  #passwordFailed(error: unknown): void {
    const reauth = reauthError(error, false, 'current_password');
    if (reauth) {
      this.pwErr = reauth.text;
      this.#clear(reauth, 'pwCur', 'pwOtp');
      // Multi-factor switched on in the meantime (another tab): reload, then the code field appears.
      if (!this.mfa && isApiError(error, 422, 'invalid') && fieldsOf(error).code) void this.load();
      return;
    }
    if (isApiError(error, 422, 'invalid')) {
      const field = fieldsOf(error).new_password;
      if (field?.code === 'too_long') {
        this.pwErr = m.password_too_long();
        return;
      }
      const rules = Array.isArray(field?.rules) ? (field.rules as PasswordRuleId[]) : [];
      const broken = PW_RULES.find((rule) => rules.includes(rule.id));
      if (broken) {
        this.pwErr = broken.error();
        return;
      }
    }
    this.#unexpected(error);
  }

  // ---- Authenticator apps ----

  /** "Add authenticator app": new secret from the server, then QR code and key. */
  async addApp(): Promise<void> {
    this.cancelTotp();
    this.totpMode = 'setup';
    const seq = this.#enrollSeq;
    try {
      const res = await api<Enrollment>('POST', 'auth/totp/devices/enroll');
      if (seq === this.#enrollSeq) this.enrollment = res;
    } catch (error) {
      if (seq !== this.#enrollSeq) return;
      this.cancelTotp();
      this.#unexpected(error);
    }
  }

  /** Trash can of an app: ask for password and code to remove it. */
  askRemove(id: string): void {
    this.cancelTotp();
    this.totpMode = 'off';
    this.totpTarget = id;
  }

  /** "Start over" after a replaced enrolment: new secret, name and password stay. */
  restartApp(): void {
    const { tName, tPw } = this;
    void this.addApp();
    this.tName = tName;
    this.tPw = tPw;
  }

  cancelTotp(): void {
    this.#enrollSeq++;
    this.tBusy = false;
    this.totpMode = null;
    this.totpTarget = null;
    this.enrollment = null;
    this.tName = '';
    this.tPw = '';
    this.tCode = '';
    this.tErr = null;
    this.tNameErr = null;
    this.tStale = false;
  }

  setTName(value: string): void {
    this.tName = value;
    this.tNameErr = null;
  }

  setTPw(value: string): void {
    this.tPw = value;
    this.tErr = null;
  }

  setTCode(value: string): void {
    this.tCode = formatOtp(value);
    this.tErr = null;
  }

  totpKey(event: KeyInput): void {
    if (event.key === 'Enter') {
      event.preventDefault();
      void this.confirmTotp();
    } else if (event.key === 'Escape') {
      event.preventDefault();
      this.cancelTotp();
    }
  }

  /** "Add" or "Remove". */
  async confirmTotp(): Promise<void> {
    const mode = this.totpMode;
    if (!mode || this.tBusy) return;
    // The name comes first, before password and code; nothing is sent.
    if (mode === 'setup' && APP_NAME_BAD.test(this.tName)) {
      this.tNameErr = m.error_name_invalid();
      return;
    }
    if (!this.tPw) {
      this.tErr = m.security_password_required();
      return;
    }
    if (this.tCode.length !== CODE_LENGTH) {
      this.tErr = m.security_code_required();
      return;
    }
    if (mode === 'setup') await this.#confirmApp();
    else await this.#removeApp();
  }

  async #confirmApp(): Promise<void> {
    const enrollment = this.enrollment;
    if (!enrollment) return;
    const name = this.tName.trim();
    // Responses only apply to this prompt; closed in the meantime or another one open: fields stay.
    const seq = this.#enrollSeq;
    this.tBusy = true;
    try {
      const res = await api<MfaOverview>('POST', 'auth/totp/devices/confirm', {
        id: enrollment.id,
        ...(name ? { name } : {}),
        password: this.tPw,
        code: this.tCode,
      });
      if (this.#disposed) return;
      this.#applyMfa(res);
      const added = res.devices.find((d) => d.id === enrollment.id)?.name ?? name;
      if (seq === this.#enrollSeq) this.cancelTotp();
      // The codes exist only now, with the first device – even if the prompt is closed in the meantime.
      const first = !!res.recovery_codes?.length;
      if (res.recovery_codes?.length) this.#showCodes(res.recovery_codes);
      ui.toasts.show(first ? m.security_mfa_on() : m.security_app_added({ name: added }));
    } catch (error) {
      if (seq !== this.#enrollSeq) return;
      const reauth = reauthError(error, true);
      if (reauth) {
        this.tErr = reauth.text;
        this.#clear(reauth, 'tPw', 'tCode');
        return;
      }
      // Enrolment replaced in the meantime (second tab) or expired: "Start over"; the code is moot.
      if (isApiError(error, 404)) {
        this.tStale = true;
        this.tCode = '';
        return;
      }
      const field = isApiError(error, 422, 'invalid') ? fieldsOf(error).name : undefined;
      if (field) {
        this.tNameErr = field.code === 'too_long' ? m.error_name_long() : m.error_name_invalid();
        return;
      }
      this.#unexpected(error);
    } finally {
      if (seq === this.#enrollSeq) this.tBusy = false;
    }
  }

  async #removeApp(): Promise<void> {
    const id = this.totpTarget;
    const target = this.devices.find((d) => d.id === id);
    if (!id || !target) return;
    const seq = this.#enrollSeq;
    this.tBusy = true;
    try {
      const res = await api<MfaOverview>('DELETE', `auth/totp/devices/${encodeURIComponent(id)}`, {
        password: this.tPw,
        code: this.tCode,
      });
      if (this.#disposed) return;
      this.#applyMfa(res);
      if (seq === this.#enrollSeq) this.cancelTotp();
      const last = res.devices.length === 0;
      if (last) {
        // With the last app the codes are dropped; changing the password no longer needs a code.
        this.codes = null;
        this.cancelRegen();
        this.pwOtp = '';
      }
      ui.toasts.show(last ? m.security_mfa_off() : m.security_app_removed({ name: target.name }));
    } catch (error) {
      if (seq !== this.#enrollSeq) return;
      const reauth = reauthError(error);
      if (reauth) {
        this.tErr = reauth.text;
        this.#clear(reauth, 'tPw', 'tCode');
        return;
      }
      if (isApiError(error, 404)) {
        // Already removed (another tab): reload the list.
        this.cancelTotp();
        void this.load();
        return;
      }
      this.#unexpected(error);
    } finally {
      if (seq === this.#enrollSeq) this.tBusy = false;
    }
  }

  async copySecret(): Promise<void> {
    if (this.enrollment && (await copyText(this.enrollment.secret))) ui.toasts.show(m.security_key_copied());
  }

  // ---- Recovery codes ----

  openRegen(): void {
    this.cancelRegen();
    this.rcForm = true;
    this.codes = null;
  }

  cancelRegen(): void {
    this.#rcSeq++;
    this.rcBusy = false;
    this.rcForm = false;
    this.rcPw = '';
    this.rcCode = '';
    this.rcErr = null;
  }

  setRcPw(value: string): void {
    this.rcPw = value;
    this.rcErr = null;
  }

  setRcCode(value: string): void {
    this.rcCode = formatOtp(value);
    this.rcErr = null;
  }

  rcKey(event: KeyInput): void {
    if (event.key === 'Enter') {
      event.preventDefault();
      void this.regenerate();
    } else if (event.key === 'Escape') {
      event.preventDefault();
      this.cancelRegen();
    }
  }

  async regenerate(): Promise<void> {
    if (!this.rcForm || this.rcBusy) return;
    if (!this.rcPw) {
      this.rcErr = m.security_password_required();
      return;
    }
    if (this.rcCode.length !== CODE_LENGTH) {
      this.rcErr = m.security_code_required();
      return;
    }
    const seq = this.#rcSeq;
    this.rcBusy = true;
    try {
      const res = await api<MfaOverview>('POST', 'auth/recovery-codes', { password: this.rcPw, code: this.rcCode });
      if (this.#disposed) return;
      this.#applyMfa(res);
      if (seq === this.#rcSeq) this.cancelRegen();
      // The old codes no longer work: show the new ones even if the form was closed in the meantime.
      if (res.recovery_codes?.length) this.#showCodes(res.recovery_codes);
      ui.toasts.show(m.security_rc_new());
    } catch (error) {
      if (seq !== this.#rcSeq) return;
      const reauth = reauthError(error);
      if (reauth) {
        this.rcErr = reauth.text;
        this.#clear(reauth, 'rcPw', 'rcCode');
        return;
      }
      if (isApiError(error, 409, 'mfa_disabled')) {
        // The last app was removed in the meantime (another tab).
        this.cancelRegen();
        void this.load();
        return;
      }
      this.#unexpected(error);
    } finally {
      if (seq === this.#rcSeq) this.rcBusy = false;
    }
  }

  async copyCodes(): Promise<void> {
    if (this.codes && (await copyText(this.codes.join('\n')))) ui.toasts.show(m.security_rc_copied());
  }

  downloadCodes(): void {
    if (this.codes && downloadCodes(session.appName, session.user?.email ?? '', this.codes)) {
      ui.toasts.show(m.security_rc_downloaded());
    }
  }

  doneCodes(): void {
    this.codes = null;
  }

  // ---- Signed-in devices ----

  /** "Sign out" of another device: hidden at once, back if the server rejects it. */
  async revoke(id: string): Promise<void> {
    if (this.hidden.has(id)) return;
    const target = this.sessions.find((s) => s.id === id);
    // The own session is not signed out here (that is "Sign out" in the account menu).
    if (!target || target.current) return;
    this.hidden.add(id);
    try {
      try {
        await api('DELETE', `auth/sessions/${encodeURIComponent(id)}`);
      } catch (error) {
        // Already ended or expired.
        if (!isApiError(error, 404)) throw error;
      }
      if (this.#disposed) return;
      this.#changes++;
      this.sessions = this.sessions.filter((s) => s.id !== id);
      this.hidden.delete(id);
      ui.toasts.show(m.security_session_ended());
    } catch (error) {
      if (this.#disposed) return;
      this.hidden.delete(id);
      this.#sessionsFailed(error, m.security_session_failed());
    }
  }

  /** "Sign out all other devices": asks first, as the toasts have no undo. */
  revokeOthers(): void {
    if (!this.sessions.some((s) => !s.current && !this.hidden.has(s.id))) return;
    ui.ask({
      title: m.security_others_ask_title(),
      text: m.security_others_ask_text(),
      icon: 'ti-logout',
      ok: m.security_sign_out(),
      danger: true,
      run: () => void this.#revokeOthers(),
    });
  }

  async #revokeOthers(): Promise<void> {
    const ids = this.sessions.filter((s) => !s.current && !this.hidden.has(s.id)).map((s) => s.id);
    if (ids.length === 0) return;
    for (const id of ids) this.hidden.add(id);
    try {
      await api('POST', 'auth/sessions/revoke-others');
      if (this.#disposed) return;
      this.#changes++;
      this.sessions = this.sessions.filter((s) => s.current);
      for (const id of ids) this.hidden.delete(id);
      ui.toasts.show(m.security_others_ended());
    } catch (error) {
      if (this.#disposed) return;
      for (const id of ids) this.hidden.delete(id);
      this.#sessionsFailed(error, m.security_others_failed());
    }
  }

  #sessionsFailed(error: unknown, text: string): void {
    this.#unexpected(error, text);
    if (!isApiError(error, 401)) void this.load();
  }

  // ---- internal ----

  /** Apps and codes after a change confirmed by the server. */
  #applyMfa(res: MfaOverview): void {
    this.#changes++;
    this.devices = res.devices;
    this.recovery = res.recovery;
  }

  #showCodes(codes: string[]): void {
    this.codes = codes;
    this.rcForm = false;
  }

  #clear<P extends 'pwCur' | 'tPw' | 'rcPw', C extends 'pwOtp' | 'tCode' | 'rcCode'>(
    reauth: ReauthError,
    password: P,
    code: C,
  ): void {
    if (reauth.clear === 'password') this[password] = '';
    if (reauth.clear === 'code') this[code] = '';
  }

  /**
   * Error toast; without a session the account reloads (the layout then shows the sign-in), a fresh sign-in asked
   * for by the server leads to it, too many requests name the wait.
   */
  #unexpected(error: unknown, text: string = m.error_act()): void {
    if (isApiError(error, 401)) void session.refreshUser().catch(() => {});
    else if (needsReauth(error)) this.#hooks.reauth?.();
    else if (isApiError(error, 429)) ui.toasts.error(m.error_limit({ wait: waitText(retryAfter(error)) }));
    else ui.toasts.error(failText(error, text));
  }
}

// Values of the page

export interface PwFieldValues {
  label: string;
  desc: string;
  /** `autocomplete` attribute. */
  ac: 'current-password' | 'one-time-code' | 'new-password';
  im: 'text' | 'numeric';
  type: 'text' | 'password';
  value: string;
  sep: boolean;
  hasEye: boolean;
  eyeIcon: 'ti-eye' | 'ti-eye-off';
  eyeTip: string;
  onChange: (value: string) => void;
  onEye?: () => void;
}

export interface TfaEntryValues {
  id: string;
  name: string;
  sub: string;
  sep: boolean;
  delLabel: string;
  /** This app is being removed (highlighted background and shadow). */
  selected: boolean;
  onRemove: () => void;
}

export interface SessionValues {
  id: string;
  icon: IconName;
  name: string;
  sub: string;
  current: boolean;
  notCurrent: boolean;
  onRevoke: () => void;
}

export interface SecurityValues {
  /** Skeleton while the first load takes longer than 150 ms. */
  skSet: boolean;
  secTabs: { label: string; onClick: () => void }[];
  pwDesc: string;
  pwFields: PwFieldValues[];
  pwKey: (event: KeyInput) => void;
  pwMsg: string;
  hasPwMsg: boolean;
  /** "Change password" still disabled (`aria-disabled`). */
  pwDis: boolean;
  pwBusy: boolean;
  pwSubmit: () => void;
  tfaEntries: TfaEntryValues[];
  tfaEmpty: boolean;
  tfaAdd: () => void;
  totpPanel: boolean;
  totpNoPanel: boolean;
  totpSetup: boolean;
  tNameVal: string;
  onTName: (value: string) => void;
  totpIntro: string;
  /** Key in groups of four; empty while it is loading. */
  totpSecret: string;
  totpQr: QrSymbol | null;
  copyTotp: () => void;
  tPwVal: string;
  onTPw: (value: string) => void;
  tCodeVal: string;
  onTCode: (value: string) => void;
  tKey: (event: KeyInput) => void;
  tErr: string;
  hasTErr: boolean;
  tNameErr: string;
  hasTNameErr: boolean;
  tStale: boolean;
  tRestart: () => void;
  totpBusy: boolean;
  totpCancel: () => void;
  totpGo: () => void;
  totpGoLabel: string;
  rcOn: boolean;
  rcOff: boolean;
  rcShow: boolean;
  rcForm: boolean;
  rcNoForm: boolean;
  rcSum: string;
  rcSub: string;
  rcCodes: { code: string; n: string }[];
  rcCopy: () => void;
  rcDownload: () => void;
  rcDone: () => void;
  rcRegen: () => void;
  rcPwVal: string;
  onRcPw: (value: string) => void;
  rcCodeVal: string;
  onRcCode: (value: string) => void;
  rcKey: (event: KeyInput) => void;
  rcErr: string;
  hasRcErr: boolean;
  rcBusy: boolean;
  rcCancel: () => void;
  rcGo: () => void;
  sessList: SessionValues[];
  hasOtherSess: boolean;
  revokeOthers: () => void;
}

export interface SecurityContext {
  /** Scroll to the section. */
  scrollTo: (section: Section) => void;
}

const DEVICE_ICON: Record<ListedSession['device_type'], IconName> = {
  desktop: 'ti-device-desktop',
  mobile: 'ti-device-mobile',
  tablet: 'ti-device-tablet',
};

function eye(shown: boolean): Pick<PwFieldValues, 'eyeIcon' | 'eyeTip'> {
  return { eyeIcon: shown ? 'ti-eye-off' : 'ti-eye', eyeTip: shown ? m.password_hide() : m.password_show() };
}

function pwFields(p: SecurityPage): PwFieldValues[] {
  const fields: Omit<PwFieldValues, 'sep'>[] = [
    {
      label: m.reauth_password_label(),
      desc: m.reauth_password_desc(),
      ac: 'current-password',
      im: 'text',
      type: p.pwShowCur ? 'text' : 'password',
      value: p.pwCur,
      hasEye: true,
      ...eye(p.pwShowCur),
      onChange: (value) => p.setPw('pwCur', value),
      onEye: () => (p.pwShowCur = !p.pwShowCur),
    },
  ];
  if (p.mfa) {
    fields.push({
      label: m.reauth_code_label(),
      desc: m.reauth_code_desc(),
      ac: 'one-time-code',
      im: 'numeric',
      type: 'text',
      value: p.pwOtp,
      hasEye: false,
      ...eye(false),
      onChange: (value) => p.setPwOtp(value),
    });
  }
  fields.push(
    {
      label: m.security_new_password(),
      desc: m.security_pw_rule(),
      ac: 'new-password',
      im: 'text',
      type: p.pwShowNew ? 'text' : 'password',
      value: p.pwNew,
      hasEye: true,
      ...eye(p.pwShowNew),
      onChange: (value) => p.setPw('pwNew', value),
      onEye: () => (p.pwShowNew = !p.pwShowNew),
    },
    {
      label: m.security_confirm_password(),
      desc: m.security_confirm_desc(),
      ac: 'new-password',
      im: 'text',
      type: 'password',
      value: p.pwConf,
      hasEye: false,
      ...eye(false),
      onChange: (value) => p.setPw('pwConf', value),
    },
  );
  return fields.map((f, i) => ({ ...f, sep: i > 0 }));
}

function totpIntro(p: SecurityPage): string {
  if (p.totpMode === 'setup') return m.security_totp_intro_add();
  const name = p.devices.find((d) => d.id === p.totpTarget)?.name ?? '';
  return p.devices.length === 1 ? m.security_totp_intro_remove_last({ name }) : m.security_totp_intro_remove({ name });
}

function sessionSub(s: ListedSession, now: number): string {
  const where = s.ip ? m.security_session_ip({ ip: s.ip }) : '';
  const when = s.current
    ? m.security_session_now()
    : m.security_session_last({ ago: ago(ms(s.last_seen_at, now), now) });
  return [where, when].filter(Boolean).join(' · ');
}

/** Values of the page. `timeZone` only for tests (dates otherwise in the browser's zone). */
export function securityValues(
  p: SecurityPage,
  ctx: SecurityContext,
  now = Date.now(),
  timeZone?: string,
): SecurityValues {
  const sessions = p.sessions.filter((s) => !p.hidden.has(s.id));
  const setup = p.totpMode === 'setup';
  const rc = p.recovery;
  return {
    skSet: p.status === 'loading' && p.loading.slow,
    secTabs: [
      { label: m.security_tab_password(), onClick: () => ctx.scrollTo('password') },
      { label: m.security_tab_mfa(), onClick: () => ctx.scrollTo('2fa') },
      { label: m.security_tab_sessions(), onClick: () => ctx.scrollTo('sessions') },
    ],
    pwDesc: p.changedAt ? m.security_pw_changed({ ago: ago(ms(p.changedAt, now), now) }) : '',
    pwFields: pwFields(p),
    pwKey: (event) => {
      if (event.key === 'Enter') {
        event.preventDefault();
        void p.changePassword();
      }
    },
    pwMsg: p.pwErr ?? '',
    hasPwMsg: !!p.pwErr,
    pwDis: !p.pwReady,
    pwBusy: p.pwBusy,
    pwSubmit: () => void p.changePassword(),
    tfaEntries: p.devices.map((d, i) => {
      const added = m.security_app_added_at({ date: securityDate(ms(d.created_at, now), timeZone) });
      const used = d.last_used_at ? m.security_app_used({ ago: ago(ms(d.last_used_at, now), now) }) : '';
      return {
        id: d.id,
        name: d.name,
        sub: [added, used].filter(Boolean).join(' · '),
        sep: i > 0,
        delLabel: m.security_app_remove_label({ name: d.name }),
        selected: p.totpMode === 'off' && p.totpTarget === d.id,
        onRemove: () => p.askRemove(d.id),
      };
    }),
    tfaEmpty: !p.mfa,
    tfaAdd: () => void p.addApp(),
    totpPanel: p.totpMode !== null,
    totpNoPanel: p.totpMode === null,
    totpSetup: setup,
    tNameVal: p.tName,
    onTName: (value) => p.setTName(value),
    totpIntro: p.totpMode ? totpIntro(p) : '',
    totpSecret: (p.enrollment?.secret.match(/.{1,4}/g) ?? []).join(' '),
    totpQr: p.qr,
    copyTotp: () => void p.copySecret(),
    tPwVal: p.tPw,
    onTPw: (value) => p.setTPw(value),
    tCodeVal: p.tCode,
    onTCode: (value) => p.setTCode(value),
    tKey: (event) => p.totpKey(event),
    tErr: p.tErr ?? '',
    hasTErr: !!p.tErr,
    tNameErr: p.tNameErr ?? '',
    hasTNameErr: !!p.tNameErr,
    tStale: p.tStale,
    tRestart: () => p.restartApp(),
    totpBusy: p.tBusy || (setup && !p.enrollment),
    totpCancel: () => p.cancelTotp(),
    totpGo: () => void p.confirmTotp(),
    totpGoLabel: setup ? m.security_add() : m.security_remove(),
    rcOn: p.mfa,
    rcOff: !p.mfa,
    rcShow: p.codes !== null,
    rcForm: p.mfa && p.rcForm,
    rcNoForm: p.mfa && !p.rcForm,
    rcSum: m.security_rc_sum({ unused: rc?.unused ?? 0, total: rc?.total ?? 10 }),
    rcSub: rc ? m.security_rc_sub({ ago: ago(ms(rc.created_at, now), now) }) : '',
    rcCodes: (p.codes ?? []).map((code, i) => ({ code, n: String(i + 1) })),
    rcCopy: () => void p.copyCodes(),
    rcDownload: () => p.downloadCodes(),
    rcDone: () => p.doneCodes(),
    rcRegen: () => p.openRegen(),
    rcPwVal: p.rcPw,
    onRcPw: (value) => p.setRcPw(value),
    rcCodeVal: p.rcCode,
    onRcCode: (value) => p.setRcCode(value),
    rcKey: (event) => p.rcKey(event),
    rcErr: p.rcErr ?? '',
    hasRcErr: !!p.rcErr,
    rcBusy: p.rcBusy,
    rcCancel: () => p.cancelRegen(),
    rcGo: () => void p.regenerate(),
    sessList: sessions.map((s) => ({
      id: s.id,
      icon: DEVICE_ICON[s.device_type] ?? 'ti-device-desktop',
      name: s.device,
      sub: sessionSub(s, now),
      current: s.current,
      notCurrent: !s.current,
      onRevoke: () => void p.revoke(s.id),
    })),
    hasOtherSess: sessions.some((s) => !s.current),
    revokeOthers: () => p.revokeOthers(),
  };
}
