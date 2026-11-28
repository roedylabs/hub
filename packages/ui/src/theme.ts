/**
 * Theme switching: mode and accent as attributes on `<html>`, the logo plate
 * of the branding as custom properties.
 *
 * Taken over from Roedy Mail (`apps/web/src/lib/theme.ts`) without density,
 * font size and the animations setting; without a choice the color scheme
 * follows the system (Mail: dark).
 */
import { ACCENTS, DEFAULT_ACCENT, PLATES, type AccentName } from '@roedy/tokens';
import type { Branding } from './branding.ts';
import { accentInk, accentText, isHexColor, type HexColor } from './color.ts';
import type { ModeName } from './settings.svelte.ts';

/** Named accent or a custom colour as `#rrggbb` (`color.ts`). */
export type AccentChoice = AccentName | HexColor;

/** The account's choice: colour scheme and accent, `null` without one (the system's scheme, the brand colour). */
export interface ThemeChoice {
  mode: ModeName | null;
  accent: AccentChoice | null;
}

export interface Theme {
  mode: ModeName;
  /** Whether the dark colours apply (`isDarkMode()`, with "System" like the operating system). */
  dark: boolean;
  /** Named accent or a custom colour (`#rrggbb`). */
  accent: AccentChoice;
  /** Plate of the logo; `null` = accent colour (default in CSS). */
  logo: readonly [bg: string, ink: string] | null;
  logoTilt: boolean;
}

/**
 * Accent like `accentName()`: own choice (named or custom) only if the branding
 * allows it, otherwise the brand colour.
 */
export function accentOf(
  values: Pick<ThemeChoice, 'accent'>,
  brand: Pick<Branding, 'userAccent' | 'color'>,
): AccentChoice {
  const name = (brand.userAccent ? values.accent : null) ?? brand.color;
  return Object.hasOwn(ACCENTS, name) || isHexColor(name) ? name : DEFAULT_ACCENT;
}

/** Mode like `modeName()`: `system` without a choice. */
export function modeName(mode: ThemeChoice['mode']): ModeName {
  return mode ?? 'system';
}

/** Dark like `isDark()`: with "System" or without a choice like the operating system. */
export function isDarkMode(mode: ThemeChoice['mode'], systemDark: boolean): boolean {
  return mode === null || mode === 'system' ? systemDark : mode === 'dark';
}

/** Theme of the settings and branding; `systemDark` is the operating system's choice for "System". */
export function themeOf(values: ThemeChoice, brand: Branding, systemDark: boolean): Theme {
  return {
    mode: modeName(values.mode),
    dark: isDarkMode(values.mode, systemDark),
    accent: accentOf(values, brand),
    logo: PLATES[brand.plate] ?? null,
    logoTilt: brand.tilt,
  };
}

export function applyTheme(root: HTMLElement, theme: Theme): void {
  root.dataset.mode = theme.mode;
  // A custom colour overrides the stylesheet's accent on the element itself, in light and dark alike.
  if (isHexColor(theme.accent)) {
    root.dataset.accent = 'custom';
    root.style.setProperty('--acc', theme.accent);
    root.style.setProperty('--accink', accentInk(theme.accent));
    // Computed per active mode: on a theme change (also by the system) this runs again.
    for (const [name, value] of Object.entries(accentText(theme.accent, theme.dark ? 'dark' : 'light'))) {
      root.style.setProperty(name, value);
    }
  } else {
    root.dataset.accent = theme.accent;
    for (const name of ['--acc', '--accink', '--accsoft', '--deep', '--toastacc']) root.style.removeProperty(name);
  }
  if (theme.logo) {
    root.style.setProperty('--logo-bg', theme.logo[0]);
    root.style.setProperty('--logo-ink', theme.logo[1]);
  } else {
    root.style.removeProperty('--logo-bg');
    root.style.removeProperty('--logo-ink');
  }
  root.style.setProperty('--logo-tilt', theme.logoTilt ? '1' : '0');
}

/**
 * Error colours of the guest pages (setup and sign-in): fixed values per
 * light/dark, as `style` for the root element.
 */
export function errorTones(dark: boolean): string {
  const soft = `color-mix(in oklab, #f097ad ${dark ? 18 : 24}%, var(--surface))`;
  return `--err: ${dark ? '#e08a9c' : '#d9677e'}; --errfg: ${dark ? '#f5abb9' : '#a3344a'}; --errsoft: ${soft}`;
}
