/**
 * The package's own texts in English and German, so that apps need no message
 * keys for them. Each module declares its texts next to its code:
 *
 * ```ts
 * const t = texts({
 *   en: { close: 'Close', minutes: ({ n }: { n: number }) => `${n} minutes` },
 *   de: { close: 'Schließen', minutes: ({ n }) => `${n} Minuten` },
 * });
 * t.close(); // in the language of `connectSettings`
 * ```
 *
 * The German table must have exactly the English keys (checked by the type).
 * A text reads the language on every call, so templates follow a change.
 */
import { settings } from './settings.svelte.ts';

/** A text: fixed, or a function of its parameters. */
// eslint-disable-next-line @typescript-eslint/no-explicit-any -- the parameters differ per text
type Text = string | ((params: any) => string);

/** Accessors like Paraglide's `m`: `t.key()` or `t.key({ … })`. */
export type Texts<T extends Record<string, Text>> = {
  readonly [K in keyof T]: T[K] extends (params: infer P) => string ? (params: P) => string : () => string;
};

/** Texts per language: German with the same keys and parameters as English. */
export interface TextTables<T extends Record<string, Text>> {
  en: T;
  de: { [K in keyof T]: T[K] };
}

export function texts<T extends Record<string, Text>>(tables: TextTables<T>): Texts<T> {
  const out: Record<string, (params?: unknown) => string> = {};
  for (const key of Object.keys(tables.en)) {
    out[key] = (params) => {
      const text: Text | undefined = tables[settings().lang()][key];
      return typeof text === 'function' ? text(params) : (text ?? '');
    };
  }
  return out as Texts<T>;
}
