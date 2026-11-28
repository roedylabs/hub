/**
 * Texts of the app switcher in English and German, from the web app's
 * message files (`switcher_*` keys), so that they are translated in one place.
 * Named imports: the build keeps only these keys, not the whole files.
 */
import {
  switcher_account as enAccount,
  switcher_admin as enAdmin,
  switcher_empty as enEmpty,
  switcher_failed as enFailed,
  switcher_home as enHome,
  switcher_loading as enLoading,
  switcher_offline as enOffline,
  switcher_retry as enRetry,
  switcher_sign_in as enSignIn,
  switcher_signed_out as enSignedOut,
  switcher_toggle as enToggle,
} from '../../messages/en.json';
import {
  switcher_account as deAccount,
  switcher_admin as deAdmin,
  switcher_empty as deEmpty,
  switcher_failed as deFailed,
  switcher_home as deHome,
  switcher_loading as deLoading,
  switcher_offline as deOffline,
  switcher_retry as deRetry,
  switcher_sign_in as deSignIn,
  switcher_signed_out as deSignedOut,
  switcher_toggle as deToggle,
} from '../../messages/de.json';

export type Locale = 'en' | 'de';

export interface Messages {
  account: string;
  admin: string;
  empty: string;
  failed: string;
  home: string;
  loading: string;
  offline: string;
  retry: string;
  signIn: string;
  signedOut: string;
  toggle: string;
}

export const MESSAGES: Record<Locale, Messages> = {
  en: {
    account: enAccount,
    admin: enAdmin,
    empty: enEmpty,
    failed: enFailed,
    home: enHome,
    loading: enLoading,
    offline: enOffline,
    retry: enRetry,
    signIn: enSignIn,
    signedOut: enSignedOut,
    toggle: enToggle,
  },
  de: {
    account: deAccount,
    admin: deAdmin,
    empty: deEmpty,
    failed: deFailed,
    home: deHome,
    loading: deLoading,
    offline: deOffline,
    retry: deRetry,
    signIn: deSignIn,
    signedOut: deSignedOut,
    toggle: deToggle,
  },
};

/** German for every `de…` tag, otherwise English (the hub's base language). */
export function localeOf(tag: string | null | undefined): Locale {
  return tag?.trim().toLowerCase().split(/[-_]/)[0] === 'de' ? 'de' : 'en';
}
