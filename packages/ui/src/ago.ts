/**
 * Time span until now ("2 hours ago"), e.g. for users and invitations, signed-in devices and authenticator apps,
 * and the last check of the system mail server.
 *
 * Taken over from Roedy Mail (`apps/web/src/lib/ago.ts`).
 */
import { texts } from './i18n.ts';

const t = texts({
  en: {
    days: ({ n }: { n: number }) => `${n} days ago`,
    hour: '1 hour ago',
    hours: ({ n }: { n: number }) => `${n} hours ago`,
    minute: '1 min ago',
    minutes: ({ n }: { n: number }) => `${n} min ago`,
    now: 'just now',
  },
  de: {
    days: ({ n }) => `vor ${n} Tagen`,
    hour: 'vor 1 Stunde',
    hours: ({ n }) => `vor ${n} Stunden`,
    minute: 'vor 1 Minute',
    minutes: ({ n }) => `vor ${n} Minuten`,
    now: 'gerade eben',
  },
});

/** "just now", "5 min ago", "47 hours ago", from two days on in days. */
export function ago(time: number, now = Date.now()): string {
  const minutes = Math.floor((now - time) / 60000);
  const hours = Math.floor(minutes / 60);
  if (minutes < 1) return t.now();
  if (minutes < 60) return minutes === 1 ? t.minute() : t.minutes({ n: minutes });
  if (hours < 48) return hours === 1 ? t.hour() : t.hours({ n: hours });
  return t.days({ n: Math.floor(hours / 24) });
}
