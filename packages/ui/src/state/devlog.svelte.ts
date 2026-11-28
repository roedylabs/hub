/**
 * Session log (`devLog`) for an app's developer page: start, notices,
 * navigation, browser errors, connection. Lives only in this tab's memory,
 * newest first, and goes nowhere unless someone copies it. The app sets the
 * chosen log level (`devLog.level`, e.g. from its settings); `visible` holds
 * the entries up to it.
 *
 * Taken over from Roedy Mail (`apps/web/src/lib/state/devlog.svelte.ts`)
 * with `LOG_LEVELS` and `visibleLog` (from Mail's `settings-choices.ts` and
 * `developer.ts`) and with the level as a settable field instead of Mail's
 * `devLevel` setting.
 */
import { untrack } from 'svelte';
import { texts } from '../i18n.ts';

const t = texts({
  en: { unknown: 'Unknown error' },
  de: { unknown: 'Unbekannter Fehler' },
});

/** Log levels, most severe first: a level shows itself and everything above it. */
export const LOG_LEVELS = ['error', 'warning', 'info', 'debug'] as const;

export type LogLevel = (typeof LOG_LEVELS)[number];

/** At most this many entries; older ones drop out (`slice(0, 120)`). */
export const LOG_MAX = 120;

export interface LogEntry {
  id: number;
  /** Timestamp in milliseconds. */
  t: number;
  level: LogLevel;
  msg: string;
}

/** Entries up to the level `level` (with `info`: errors, warnings and info, no debug). */
export function visibleLog(entries: readonly LogEntry[], level: LogLevel): LogEntry[] {
  const limit = LOG_LEVELS.indexOf(level);
  return entries.filter((e) => LOG_LEVELS.indexOf(e.level) <= limit);
}

/** Text for a rejected promise that nobody handled. */
function reasonText(reason: unknown): string {
  if (reason instanceof Error && reason.message) return reason.message;
  return typeof reason === 'string' && reason ? reason : t.unknown();
}

export class DevLog {
  entries = $state.raw<LogEntry[]>([]);
  /** Chosen log level of the developer page; the app sets it from its settings. */
  level = $state<LogLevel>('info');
  /** Entries up to the chosen level, newest first. */
  readonly visible = $derived(visibleLog(this.entries, this.level));
  #id = 0;

  /** Writes an entry; also from effects, which do not depend on the log because of it. */
  add(level: LogLevel, msg: string): void {
    const entry = { id: ++this.#id, t: Date.now(), level, msg };
    this.entries = [entry, ...untrack(() => this.entries)].slice(0, LOG_MAX);
  }

  clear(): void {
    this.entries = [];
  }

  /** Records browser errors; plus rejected promises. */
  attach(target: Window): () => void {
    const onError = (event: ErrorEvent) => this.add('error', event.message || t.unknown());
    const onRejection = (event: PromiseRejectionEvent) => this.add('error', reasonText(event.reason));
    target.addEventListener('error', onError);
    target.addEventListener('unhandledrejection', onRejection);
    return () => {
      target.removeEventListener('error', onError);
      target.removeEventListener('unhandledrejection', onRejection);
    };
  }
}

export const devLog = new DevLog();
