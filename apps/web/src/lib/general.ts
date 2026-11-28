/**
 * Values of the Account › General page: behaviour ("Ask before deleting",
 * the undo delay, keyboard shortcuts) and developer mode, with the tabs
 * `genTabs` jumping to the sections. These settings apply in the hub and,
 * through the shared settings, in every app (`ui.confirmDelete`,
 * `ui.undoMs`, `ui.shortcuts`, `ui.developer` of `@roedy/ui`). Changes go
 * through `prefs.set()` at once.
 *
 * Taken over from Roedy Mail (`apps/web/src/lib/general.svelte.ts`) with the
 * behaviour section only – start page, swipe gestures, external content,
 * export and local data stay with Mail – and developer mode (Mail's
 * "Advanced settings") as a setting of its own.
 */
import { UNDO_DELAYS, type UndoDelay } from '@roedy/ui';
import type { SegOption, Toggle } from '#lib/appearance.ts';
import { m } from '#lib/paraglide/messages.js';
import { segOptions } from '#lib/select.ts';
import type { Settings } from '#lib/state/prefs.svelte.ts';

/** Sections of the page (`data-sec`, `genTabs`). */
export type GeneralSection = 'behavior' | 'developer';

export interface GeneralValues {
  genTabs: { label: string; onClick: () => void }[];
  tgConfirm: Toggle;
  undoOpts: SegOption<`${UndoDelay}`>[];
  tgKeys: Toggle;
  tgDeveloper: Toggle;
}

export interface GeneralInput {
  values: Pick<Settings, 'confirm_delete' | 'undo_delay' | 'shortcuts' | 'developer'>;
}

export interface GeneralContext {
  set: (patch: Partial<Settings>) => void;
  scrollTo: (section: GeneralSection) => void;
}

export function generalValues(input: GeneralInput, ctx: GeneralContext): GeneralValues {
  const { values } = input;
  const toggle = (key: 'confirm_delete' | 'shortcuts' | 'developer'): Toggle => ({
    on: values[key],
    onClick: () => ctx.set({ [key]: !values[key] }),
  });
  return {
    genTabs: [
      { label: m.general_tab_behavior(), onClick: () => ctx.scrollTo('behavior') },
      { label: m.general_tab_developer(), onClick: () => ctx.scrollTo('developer') },
    ],
    tgConfirm: toggle('confirm_delete'),
    undoOpts: segOptions(
      UNDO_DELAYS.map((s) => `${s}` as const),
      `${values.undo_delay}`,
      (s) => m.general_undo_seconds({ n: s }),
      (s) => ctx.set({ undo_delay: Number(s) as UndoDelay }),
    ),
    tgKeys: toggle('shortcuts'),
    tgDeveloper: toggle('developer'),
  };
}
