/**
 * Trimming like Rust's `str::trim`, which the backend applies to inputs: it removes Unicode `White_Space` at both
 * ends. JavaScript's `String.prototype.trim` differs: it keeps U+0085 (next line), which Rust removes, and removes
 * U+FEFF (byte order mark), which Rust keeps.
 */

const EDGES = /^\p{White_Space}+|\p{White_Space}+$/gu;

/** The value without Unicode `White_Space` at the start and the end (Rust `str::trim`). */
export function trimWs(value: string): string {
  return value.replace(EDGES, '');
}
