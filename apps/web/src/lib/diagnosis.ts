/**
 * Diagnosis of a Stalwart server: text per row, result and reason from the
 * API (`reasons` of `POST admin/stalwart/servers/:id/test`, `probe::Reasons`),
 * the action per row and the texts of a failed registration of the OAuth
 * client (`registration_failed`). Without a reason (the list after a reload
 * does not know it) the general text applies.
 *
 * Taken over from Roedy Mail (`apps/web/src/lib/diagnosis.ts`) with the hub's
 * reasons (`no_session`, `origin_not_allowed`, `no_client_id`,
 * `revocation_auth`, `session_url_mismatch`, `invalid_url`), without the
 * setup's throttled test box.
 */
import type { IconName } from '@roedy/ui';
import { waitText } from '@roedy/ui';
import { m } from '#lib/paraglide/messages.js';
import type { DiagReasons, DiagStatus } from '#lib/stalwart.ts';

export type { DiagReasons, DiagStatus } from '#lib/stalwart.ts';
export type DiagLine = 'reach' | 'cors' | 'oauth';

export interface DiagContext {
  /** Host of the hub for "The server allows browser requests from …". */
  origin: string;
  /** Redirect URI for the advice on the OAuth client. */
  redirect: string;
  /** Name of the hub (`unsupported_version`, `throttled`). */
  app: string;
  /** Remaining wait in seconds after a throttled test (`retry_after`); without it or expired "in a few minutes". */
  wait?: number;
}

type Texts = Partial<Record<string, () => string>>;

function texts(line: DiagLine, status: DiagStatus, ctx: DiagContext): Texts {
  const skip = () => m.srv_diag_skipped();
  const metadata = () => m.srv_diag_oauth_metadata();
  const browser = () => m.srv_diag_oauth_browser();
  const throttled = () =>
    ctx.wait && ctx.wait > 0
      ? m.srv_diag_throttled({ app: ctx.app, wait: waitText(ctx.wait) })
      : m.srv_diag_throttled_later({ app: ctx.app });
  if (line === 'reach') {
    if (status === 'ok')
      return { '': () => m.srv_diag_jmap_ok(), auth_required: () => m.srv_diag_reach_auth_required() };
    if (status === 'err') {
      return {
        '': () => m.srv_diag_jmap_err(),
        unreachable: () => m.srv_diag_reach_unreachable(),
        no_session: () => m.srv_diag_jmap_err(),
        no_mail: () => m.srv_diag_reach_no_mail(),
        api_url_mismatch: () => m.srv_diag_reach_api_url_mismatch(),
        session_url_mismatch: () => m.srv_diag_reach_api_url_mismatch(),
        redirect_other_origin: () => m.srv_diag_reach_redirect(),
        too_many_redirects: () => m.srv_diag_reach_redirects(),
      };
    }
    return { throttled };
  }
  if (line === 'cors') {
    if (status === 'ok') return { '': () => m.srv_diag_cors_ok({ host: ctx.origin }) };
    if (status === 'err') {
      return {
        '': () => m.srv_diag_cors_err(),
        origin_not_allowed: () => m.srv_diag_cors_err(),
        authorization_not_allowed: () => m.srv_diag_cors_authorization(),
        content_type_not_allowed: () => m.srv_diag_cors_content_type(),
        preflight_status: () => m.srv_diag_cors_preflight(),
        preflight_failed: () => m.srv_diag_cors_preflight(),
      };
    }
    return {
      not_reachable: skip,
      no_api_url: () => m.srv_diag_cors_no_api_url(),
      throttled,
    };
  }
  if (status === 'ok') return { '': () => m.srv_diag_oauth_ok() };
  if (status === 'err') {
    return {
      '': () => m.srv_diag_oauth_err({ redirect: ctx.redirect }),
      no_metadata: metadata,
      metadata_unreachable: metadata,
      token_endpoint_mismatch: () => m.srv_diag_oauth_token_endpoint(),
      grant_types: browser,
      pkce: browser,
      no_public_client: browser,
      unsupported_version: () => m.srv_diag_oauth_unsupported_version({ app: ctx.app }),
      issuer_mismatch: () => m.srv_diag_oauth_issuer(),
      authorization_endpoint_mismatch: () => m.srv_diag_oauth_authorization_endpoint(),
      revocation_endpoint_mismatch: () => m.srv_diag_oauth_revocation_endpoint(),
      revocation_auth: () => m.srv_diag_oauth_revocation_auth(),
      no_confidential_client: () => m.srv_diag_oauth_no_confidential(),
    };
  }
  return { not_reachable: skip, throttled };
}

/**
 * Text of a row: by the reason, otherwise the general one of the result; "Not checked because the server isn’t
 * reachable." if CORS or OAuth stayed unchecked without a reason and the server was unreachable.
 */
export function diagText(
  line: DiagLine,
  status: DiagStatus,
  reach: DiagStatus,
  reasons: DiagReasons | null | undefined,
  ctx: DiagContext,
): string {
  const reason = reasons?.[line] ?? (status === 'unk' && reach === 'err' ? 'not_reachable' : '');
  const table = texts(line, status, ctx);
  const text = table[reason] ?? table[''];
  return text ? text() : m.srv_diag_unknown();
}

/** Colour of the icon per result (green is fixed). */
export function diagColor(status: DiagStatus): string {
  return status === 'ok' ? '#4fae64' : status === 'err' ? 'var(--danger)' : 'var(--mfg)';
}

/** Icon per result: check mark, warning, dashed for "not checked". */
export function diagIcon(status: DiagStatus): IconName {
  return status === 'ok' ? 'ti-circle-check' : status === 'err' ? 'ti-alert-triangle' : 'ti-circle-dashed';
}

/** Action of a row: edit the address, test again or register the client. */
export type DiagAction = 'edit' | 'test' | 'register';

/**
 * Action of a row: JMAP session "Edit URL", CORS "Test again"; OAuth by reason – edit an address that does not
 * match Stalwart (`…_mismatch`), test a server without v1 or without clients with a secret again (a registration
 * would not help there), otherwise register the client.
 */
export function diagAction(line: DiagLine, reason: string | null | undefined): DiagAction {
  if (line === 'reach') return 'edit';
  if (line === 'cors') return 'test';
  if (reason?.endsWith('_mismatch')) return 'edit';
  if (reason === 'unsupported_version' || reason === 'no_confidential_client' || reason === 'revocation_auth')
    return 'test';
  return 'register';
}

/**
 * Rejected sign-in as administrator during the registration (`auth_failed`, `mfa_required`): the dialog clears the
 * app password; both count in the backend towards "Too many attempts.". `forbidden` has its own text and keeps the
 * fields.
 */
export function adminRejected(reason: string | null): boolean {
  return reason === 'auth_failed' || reason === 'mfa_required';
}

/**
 * Text of a failed registration of the OAuth client; `rejected` names `detail` in parentheses. A throttled server
 * with a wait time shows the box "Too many requests." instead (`throttleText`); without a wait time the fallback
 * `throttled` applies.
 */
export function registrationText(reason: string | null, detail: string | null, app: string): string {
  switch (reason) {
    case 'auth_failed':
      return m.srv_reg_auth_failed();
    case 'mfa_required':
      return m.srv_reg_mfa_required();
    case 'forbidden':
      return m.srv_reg_forbidden();
    case 'unreachable':
      return m.srv_reg_unreachable();
    case 'unsupported':
      return m.srv_reg_unsupported();
    case 'unsupported_version':
      return m.srv_reg_unsupported_version({ app });
    case 'throttled':
      return m.srv_reg_throttled();
    case 'rejected':
      return detail ? m.srv_reg_rejected_detail({ detail }) : m.srv_reg_rejected();
    default:
      return m.srv_reg_rejected();
  }
}

/** Text in the box "Too many requests.": Stalwart throttles the registration. */
export function throttleText(app: string, seconds: number): string {
  return m.srv_reg_throttled_text({ app, wait: waitText(seconds) });
}
