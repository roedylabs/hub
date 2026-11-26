/**
 * Files to download: the recovery codes of the setup, the invitation and
 * Account › Security.
 *
 * Taken over from Roedy Mail (`apps/web/src/lib/download.ts`) without the
 * developer log.
 */
import { m } from '#lib/paraglide/messages.js';

/** Offers `blob` as file `name`; `false` if the browser cannot. */
export function downloadBlob(name: string, blob: Blob): boolean {
  try {
    const url = URL.createObjectURL(blob);
    const link = document.createElement('a');
    link.href = url;
    link.download = name;
    document.body.append(link);
    link.click();
    link.remove();
    setTimeout(() => URL.revokeObjectURL(url), 1000);
    return true;
  } catch {
    return false;
  }
}

/** Offers `text` as file `name`; `false` if the browser cannot. */
export function downloadText(name: string, text: string): boolean {
  return downloadBlob(name, new Blob([text], { type: 'text/plain' }));
}

/** "‹App› – recovery codes", the address and one code per line (`recovery-codes.txt`). */
export function downloadCodes(app: string, email: string, codes: readonly string[]): boolean {
  return downloadText(m.codes_file_name(), `${m.codes_file({ app })}\n${email}\n\n${codes.join('\n')}\n`);
}
