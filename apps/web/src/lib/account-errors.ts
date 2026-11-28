/**
 * Texts for what can go wrong with a Stalwart account: adding it (sign-in at
 * the server or an access token), signing it in again, checking it, and the
 * return from the server's sign-in page. One error per answer of the API
 * (`crates/server/src/api/stalwart/accounts.rs`): a title, a text naming the
 * server, the field it belongs to, the server's own code where it helps
 * ("Server message: …"), an action ("Open account", "Check Stalwart
 * servers") and the wait of a lock.
 *
 * Texts taken over from Roedy Mail (`mail_login_*`, `mail_account_check_*`)
 * where the case is the same.
 */
import { ApiError, fieldsOf, type IconName, isOffline, retryAfter, waitText } from '@roedy/ui';
import { m } from '#lib/paraglide/messages.js';

/** Field an error belongs to. */
export type AccountField = 'email' | 'server' | 'url' | 'token';

export interface AccountError {
  icon: IconName;
  title: string;
  text: string;
  field?: AccountField;
  /** The server's code or reason, shown as "Server message: …". */
  code?: string;
  /** "Open account" (`dup`, with `account`) or Administration › Stalwart servers (`servers`, administrators). */
  act?: 'open' | 'servers';
  /** Account of `open`. */
  account?: string;
  /** Seconds of a lock (429). */
  wait?: number;
}

/** What the texts name. */
export interface ErrorContext {
  /** Host of the server (`mail.example.com`); empty: "the server". */
  host: string;
  /** The address entered or the account's. */
  email: string;
}

const str = (value: unknown): string => (typeof value === 'string' ? value : '');

/** "mail.example.com", or "The server" without a host. */
function hostName(c: ErrorContext): string {
  return c.host || m.acct_the_server();
}

/** The server rejected or could not complete the check of the JMAP session (`check_failed` with `reason`). */
export function checkFailed(reason: string, c: ErrorContext, token: boolean): AccountError {
  const host = hostName(c);
  switch (reason) {
    case 'auth_failed':
    case 'forbidden':
      return token
        ? { icon: 'ti-key', title: m.acct_err_tok_title(), text: m.acct_err_tok_text({ host }), field: 'token' }
        : { icon: 'ti-lock', title: m.acct_err_denied_title(), text: m.acct_err_denied_text({ host }) };
    case 'unreachable':
    case 'http_status':
      return { icon: 'ti-plug-connected-x', title: m.acct_err_reach_title(), text: m.acct_err_reach_text({ host }) };
    case 'throttled':
      return { icon: 'ti-clock', title: m.acct_err_rate_title(), text: m.acct_err_rate_text({ host }) };
    case 'foreign_origin':
    case 'session_url_mismatch':
      return { icon: 'ti-server-off', title: m.acct_err_session_title(), text: m.acct_err_session_text({ host }) };
    case 'unsupported':
      return { icon: 'ti-server-off', title: m.acct_err_jmap_title(), text: m.acct_err_jmap_text({ host }) };
    case 'no_mail':
      return { icon: 'ti-mail-off', title: m.acct_err_no_mail_title(), text: m.acct_err_no_mail_text({ host }) };
    case 'no_address':
      return { icon: 'ti-at', title: m.acct_err_no_address_title(), text: m.acct_err_no_address_text({ host }) };
    default:
      return {
        icon: 'ti-alert-triangle',
        title: m.acct_err_other_title(),
        text: m.acct_err_other_text({ host }),
        ...(reason ? { code: reason } : {}),
      };
  }
}

/** Field errors of 422 `invalid`: the first that the page knows. */
function fieldError(error: ApiError, c: ErrorContext): AccountError | null {
  const fields = fieldsOf(error);
  const email = fields.email?.code;
  if (email) return { icon: 'ti-at', title: m.acct_err_email_title(), text: m.acct_err_email_text(), field: 'email' };
  const server = fields.server_id?.code;
  if (server === 'no_client')
    return {
      icon: 'ti-key-off',
      title: m.acct_err_no_client_title(),
      text: m.acct_err_no_client_text({ host: hostName(c) }),
      field: 'server',
      act: 'servers',
    };
  if (server === 'not_found')
    return { icon: 'ti-server-off', title: m.acct_err_gone_title(), text: m.acct_err_gone_text(), field: 'server' };
  if (server)
    return { icon: 'ti-server', title: m.acct_err_nosrv_title(), text: m.acct_err_choose_text(), field: 'server' };
  const url = fields.url?.code;
  if (url === 'private')
    return { icon: 'ti-shield-x', title: m.acct_err_private_title(), text: m.acct_err_private_text(), field: 'url' };
  if (url === 'not_allowed')
    return { icon: 'ti-server-off', title: m.acct_err_custom_title(), text: m.acct_err_custom_text(), field: 'url' };
  if (url) return { icon: 'ti-world', title: m.acct_err_url_title(), text: m.acct_err_url_text(), field: 'url' };
  const token = fields.token?.code;
  if (token === 'required')
    return { icon: 'ti-key', title: m.acct_err_sec_title(), text: m.acct_err_sec_text(), field: 'token' };
  if (token)
    return { icon: 'ti-key', title: m.acct_err_tok_bad_title(), text: m.acct_err_tok_bad_text(), field: 'token' };
  if (fields.code)
    return { icon: 'ti-alert-triangle', title: m.acct_err_failed_title(), text: m.acct_err_failed_text() };
  return null;
}

/** A text that starts with "the server" (without a host) starts with a capital. */
function capital(e: AccountError): AccountError {
  const lead = m.acct_the_server();
  if (!e.text.startsWith(lead)) return e;
  return { ...e, text: lead.charAt(0).toLocaleUpperCase() + e.text.slice(1) };
}

/** Text of an error of the API (or of no answer) for the account's forms and the return page. */
export function accountError(error: unknown, c: ErrorContext, options: { token?: boolean } = {}): AccountError {
  const e = errorOf(error, c, options);
  return c.host ? e : capital(e);
}

function errorOf(error: unknown, c: ErrorContext, options: { token?: boolean }): AccountError {
  const host = hostName(c);
  if (isOffline(error))
    return { icon: 'ti-wifi-off', title: m.acct_err_offline_title(), text: m.acct_err_offline_text() };
  if (!(error instanceof ApiError))
    return { icon: 'ti-alert-triangle', title: m.acct_err_other_title(), text: m.acct_err_other_text({ host }) };
  const reason = str(error.body.reason);
  switch (error.code) {
    case 'invalid': {
      const field = fieldError(error, c);
      if (field) return field;
      break;
    }
    case 'rate_limited': {
      const wait = retryAfter(error);
      return {
        icon: 'ti-clock',
        title: m.acct_err_rate_title(),
        text: m.acct_err_rate_wait({ wait: waitText(wait) }),
        wait,
      };
    }
    case 'dup': {
      const account = error.body.account as { id?: unknown; email?: unknown } | undefined;
      return {
        icon: 'ti-copy',
        title: m.acct_err_dup_title(),
        text: m.acct_err_dup_text({ email: str(account?.email) || c.email }),
        act: 'open',
        account: str(account?.id),
        field: 'email',
      };
    }
    case 'other_account': {
      const account = error.body.account as { email?: unknown } | undefined;
      return {
        icon: 'ti-user-question',
        title: m.acct_err_other_account_title(),
        text: m.acct_err_other_account_text({ host, as: str(account?.email), email: c.email }),
      };
    }
    case 'check_failed':
      return checkFailed(reason, c, options.token ?? false);
    case 'no_client':
      return {
        icon: 'ti-key-off',
        title: m.acct_err_no_client_title(),
        text: m.acct_err_no_client_again_text({ host }),
        act: 'servers',
      };
    case 'blocked':
      return { icon: 'ti-server-off', title: m.acct_err_blocked_title(), text: m.acct_err_blocked_text({ host }) };
    case 'needs_sign_in':
      return { icon: 'ti-login-2', title: m.acct_err_needs_title(), text: m.acct_err_needs_text({ host }) };
    case 'refresh_failed':
      return {
        icon: 'ti-plug-connected-x',
        title: m.acct_err_reach_title(),
        text: m.acct_err_reach_text({ host }),
        ...(reason ? { code: reason } : {}),
      };
    case 'sign_in_unavailable':
      return {
        icon: 'ti-plug-connected-x',
        title: m.acct_err_unavailable_title(),
        text: m.acct_err_unavailable_text({ host }),
        ...(reason ? { code: reason } : {}),
        act: 'servers',
      };
    case 'not_found':
      return { icon: 'ti-mail-off', title: m.acct_err_removed_title(), text: m.acct_err_removed_text() };
    // The return from the sign-in page.
    case 'invalid_state':
      return { icon: 'ti-link-off', title: m.acct_err_state_title(), text: m.acct_err_state_text() };
    case 'session_changed':
      return {
        icon: 'ti-user-question',
        title: m.acct_err_session_changed_title(),
        text: m.acct_err_session_changed_text(),
      };
    case 'sign_in_expired':
      return { icon: 'ti-clock-x', title: m.acct_err_expired_title(), text: m.acct_err_expired_text() };
    case 'sign_in_failed':
      return reason === 'access_denied'
        ? { icon: 'ti-arrow-back-up', title: m.acct_err_cancel_title(), text: m.acct_err_cancel_text({ host }) }
        : {
            icon: 'ti-alert-triangle',
            title: m.acct_err_incomplete_title(),
            text: m.acct_err_incomplete_text({ host }),
            ...(reason ? { code: reason } : {}),
          };
    case 'issuer_mismatch':
      return { icon: 'ti-shield-x', title: m.acct_err_iss_title(), text: m.acct_err_iss_text({ host }) };
    case 'server_changed':
      return { icon: 'ti-server-off', title: m.acct_err_changed_title(), text: m.acct_err_changed_text() };
    case 'exchange_failed':
      return {
        icon: 'ti-key-off',
        title: m.acct_err_incomplete_title(),
        text: m.acct_err_exchange_text({ host }),
        ...(reason ? { code: str(error.body.detail) || reason } : {}),
      };
  }
  return {
    icon: 'ti-alert-triangle',
    title: m.acct_err_failed_title(),
    text: m.acct_err_failed_text(),
    ...(error.code !== 'unknown' ? { code: error.code } : {}),
  };
}
