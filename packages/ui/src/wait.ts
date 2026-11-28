/**
 * Wait time of a lockout (`retry_after` in seconds) as text, plus the clock
 * of the locked buttons.
 *
 * Taken over from Roedy Mail (`apps/web/src/lib/wait.ts`).
 */
import { texts } from './i18n.ts';

const t = texts({
  en: {
    minute: '1 minute',
    minutes: ({ n }: { n: number }) => `${n} minutes`,
    second: '1 second',
    seconds: ({ n }: { n: number }) => `${n} seconds`,
  },
  de: {
    minute: '1 Minute',
    minutes: ({ n }) => `${n} Minuten`,
    second: '1 Sekunde',
    seconds: ({ n }) => `${n} Sekunden`,
  },
});

/** Seconds below one minute, otherwise minutes rounded up ("1 second", "15 minutes"). */
export function waitText(seconds: number): string {
  const s = Math.max(0, Math.ceil(seconds));
  if (s < 60) return s === 1 ? t.second() : t.seconds({ n: s });
  const minutes = Math.ceil(s / 60);
  return minutes === 1 ? t.minute() : t.minutes({ n: minutes });
}

/** Remaining time as a clock "m:ss" ("Anmelden in 4:59", "Erneut senden in 0:30"). */
export function clockText(seconds: number): string {
  const s = Math.max(0, Math.ceil(seconds));
  return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, '0')}`;
}
