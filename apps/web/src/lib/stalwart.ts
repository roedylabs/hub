/**
 * The hub's Stalwart API for its own pages (`crates/server/src/api/stalwart`):
 * the registry as every signed-in user sees it (`GET stalwart/servers`), the
 * registry for administrators (`admin/stalwart/…`) and the sign-in at a
 * server (`POST stalwart/sign-in`, `…/complete`). The accounts themselves
 * are `StalwartAccount` of `@roedy/ui` (contract § 6.1).
 */
import type { StalwartAccount } from '@roedy/ui';

export type { StalwartAccount } from '@roedy/ui';

/** Result of one check of the diagnosis: OK, failed, not checked. */
export type DiagStatus = 'ok' | 'err' | 'unk';

/** Server of `GET stalwart/servers` (`api::stalwart::servers::PublicServer`). */
export interface PublicServer {
  id: string;
  label: string;
  /** Origin, e.g. `https://mail.example.com`. */
  url: string;
  /** Host with port if given (`mail.example.com:8443`). */
  host: string;
  domains: string[];
  /** The hub's OAuth client is set up: users sign in at the server; otherwise only with an access token. */
  sign_in: boolean;
  reach: DiagStatus;
  cors: DiagStatus;
  oauth: DiagStatus;
}

/** Response of `GET stalwart/servers`. */
export interface PublicServers {
  servers: PublicServer[];
  /** Users may connect accounts on servers outside the registry (with an access token). */
  allow_custom_servers: boolean;
}

/** Server of `GET admin/stalwart/servers` (`AdminServer`); the secret only as `has_secret`. */
export interface AdminServer {
  id: string;
  label: string;
  url: string;
  host: string;
  domains: string[];
  client_id: string | null;
  has_secret: boolean;
  reach: DiagStatus;
  cors: DiagStatus;
  oauth: DiagStatus;
  tested_at: string | null;
  /** Linked accounts of all users. */
  accounts: number;
  created_at: string;
}

/** Response of `GET admin/stalwart/servers`. */
export interface AdminServers {
  servers: AdminServer[];
  /** `<public address>/oauth/callback`, for a client created by hand. */
  redirect_uri: string;
  /** Server of the system mail (Administration › Email). */
  system_mail_server: string | null;
}

/** Reasons of a failed check (`probe::Reasons`), only in the answer of "Test connection". */
export interface DiagReasons {
  reach?: string;
  cors?: string;
  oauth?: string;
}

/** Response of `POST admin/stalwart/servers/:id/test`. */
export interface TestReply {
  server: AdminServer;
  reasons: DiagReasons;
  /** Seconds while the server throttles the hub. */
  retry_after?: number;
}

/** Response of `POST stalwart/sign-in`: the server's sign-in page. */
export interface SignInStart {
  authorize_url: string;
}

/** Response of `POST stalwart/sign-in/complete`: the account (201 new, 200 signed in again) and where to go. */
export interface SignInComplete {
  account: StalwartAccount;
  return_to: string;
}

/** Address of the return from the server's sign-in page (redirect URI of the hub's OAuth client). */
export const CALLBACK_PATH = '/oauth/callback';
