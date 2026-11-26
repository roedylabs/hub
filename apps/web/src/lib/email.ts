/** Email address, as checked by `auth::email` in the backend.
 *
 * Taken over from Roedy Mail (`apps/web/src/lib/email.ts`).
 */
const EM = /^[^\s@]+@[^\s@]+\.[^\s@]{2,}$/;

export function isEmail(value: string): boolean {
  return EM.test(value);
}
