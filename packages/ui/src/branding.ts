/**
 * The hub's branding as every Roedy app shows it: logo (standard mark,
 * letters, icon or image), its plate and tilt, and the accent colour
 * (`branding` of the hub's `GET /api/v1/public/config`, the server's
 * `instance::Branding` without the administration's fields).
 *
 * Moved here from the hub's web app (`state/session.svelte.ts`).
 */
import { DEFAULT_ACCENT, PLATES, type AccentName } from '@roedy/tokens';

export type PlateName = keyof typeof PLATES;

export interface Branding {
  kind: string;
  letter: string;
  icon: string;
  image: string | null;
  fit: string;
  plate: PlateName;
  tilt: boolean;
  color: AccentName;
  userAccent: boolean;
}

/** Default until the first response, like the server's. */
export const BRAND0: Branding = {
  kind: 'standard',
  letter: '',
  icon: 'ti-mountain',
  image: null,
  fit: 'padded',
  plate: 'accent',
  tilt: true,
  color: DEFAULT_ACCENT,
  userAccent: true,
};
