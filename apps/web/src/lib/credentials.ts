/**
 * Client ID and secret of an OAuth client like the backend
 * (`MAX_CLIENT_FIELD`, `char::is_control`): no control characters, at most
 * 1024 bytes.
 *
 * Taken over from Roedy Mail (`apps/web/src/lib/credentials.ts`).
 */
export function credBad(value: string): boolean {
  return /\p{Cc}/u.test(value) || new TextEncoder().encode(value).length > 1024;
}
