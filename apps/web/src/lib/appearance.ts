/**
 * Values of the Settings › Appearance page: colour scheme (`modeCards`) and
 * accent colour (`accentOpts` and the sixth tile `accentMore` for a custom
 * colour from the colour editor; locked without `userAccent` of the
 * branding), plus the segment and switch types the other settings pages use.
 * Changes go through `prefs.set()`; the layout applies the theme at once
 * (`applyTheme()` in `#lib/theme.ts`).
 *
 * Taken over from Roedy Mail (`apps/web/src/lib/appearance.ts`), trimmed to
 * colour scheme and accent: the hub has no font size, animations setting,
 * density, mailbox design or sidebar. `Toggle` comes from Mail's
 * `banner.svelte.ts`.
 */
import { ACCENTS, MODE_PREV, type AccentName } from '@roedy/tokens';
import {
  accentInk,
  accentOf,
  type Branding,
  type HexColor,
  type IconName,
  isHexColor,
  modeName,
  type ModeName,
  MODES,
} from '@roedy/ui';
import { m } from '#lib/paraglide/messages.js';
import type { Settings } from '#lib/state/prefs.svelte.ts';

/** Sections of the page (`data-sec`, `tabs`). */
export type AppearanceSection = 'mode' | 'accent';

/** Option of a segmented control. */
export interface SegOption<T extends string> {
  value: T;
  label: string;
  on: boolean;
  onClick: () => void;
}

/** A switch (`role="switch"`). */
export interface Toggle {
  on: boolean;
  onClick: () => void;
}

/** Colour scheme card with a preview in the fixed colours of its mode (`MODE_PREV`). */
export interface ModeCard {
  name: ModeName;
  label: string;
  on: boolean;
  icon: IconName;
  bg: string;
  panel: string;
  line: string;
  inkc: string;
  onClick: () => void;
}

export interface AccentOption {
  name: AccentName;
  label: string;
  base: string;
  ink: string;
  on: boolean;
  onClick: () => void;
}

/**
 * Sixth accent tile: "More" opens the colour editor; while a custom accent is active it shows that colour
 * like a named tile. Not remembered apart from `accent`: a named accent turns it back into "More". Name and
 * start colour of the editor come from the page (`customName()`, `editorStart()` of
 * `#lib/color-editor.svelte.ts`).
 */
export interface AccentMore {
  /** Active custom accent; `null`: "More" (`ti-plus`). */
  hex: HexColor | null;
  /** Ink of the check on it (`accentInk()`). */
  ink: HexColor | null;
  on: boolean;
  /** Active accent, named or custom (the brand colour while locked): the editor starts with it. */
  accent: AccentName | HexColor;
  /** "Apply" of the editor: saves at once, like a click on a named tile. */
  apply: (hex: HexColor) => void;
}

export interface AppearanceValues {
  tabs: { label: string; onClick: () => void }[];
  modeCards: ModeCard[];
  accentOpts: AccentOption[];
  accentMore: AccentMore;
  /** Without `userAccent` the branding sets the accent colour: selection locked (`accOp`, `accPe`). */
  accLocked: boolean;
}

export interface AppearanceInput {
  values: Pick<Settings, 'mode' | 'accent'>;
  brand: Pick<Branding, 'userAccent' | 'color'>;
}

export interface AppearanceContext {
  set: (patch: Partial<Settings>) => void;
  scrollTo: (section: AppearanceSection) => void;
}

export function modeLabel(mode: ModeName): string {
  return { light: m.appearance_mode_light, dark: m.appearance_mode_dark, system: m.appearance_mode_system }[mode]();
}

export function accentLabel(name: AccentName): string {
  return {
    apricot: m.appearance_accent_apricot,
    sage: m.appearance_accent_sage,
    lavender: m.appearance_accent_lavender,
    sky: m.appearance_accent_sky,
    rose: m.appearance_accent_rose,
  }[name]();
}

export function appearanceValues(input: AppearanceInput, ctx: AppearanceContext): AppearanceValues {
  const { values, brand } = input;
  const mode = modeName(values.mode);
  const accent = accentOf(values, brand);
  const locked = !brand.userAccent;
  return {
    tabs: [
      { label: m.appearance_tab_mode(), onClick: () => ctx.scrollTo('mode') },
      { label: m.appearance_tab_accent(), onClick: () => ctx.scrollTo('accent') },
    ],
    modeCards: MODES.map((name) => ({
      name,
      label: modeLabel(name),
      on: name === mode,
      ...MODE_PREV[name],
      onClick: () => ctx.set({ mode: name }),
    })),
    accentOpts: (Object.keys(ACCENTS) as AccentName[]).map((name) => ({
      name,
      label: accentLabel(name),
      ...ACCENTS[name],
      on: name === accent,
      // When locked a click has no effect.
      onClick: () => {
        if (!locked) ctx.set({ accent: name });
      },
    })),
    accentMore: {
      hex: isHexColor(accent) ? accent : null,
      ink: isHexColor(accent) ? accentInk(accent) : null,
      on: isHexColor(accent),
      accent,
      apply: (hex) => {
        if (!locked) ctx.set({ accent: hex });
      },
    },
    accLocked: locked,
  };
}
