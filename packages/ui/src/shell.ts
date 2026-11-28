/**
 * Values of the shell components: prop types and the derivations for the
 * header, settings navigation, account menu, confirmation, settings skeleton
 * and select lists.
 * Not in the components so that tests and layout can import them
 * type-checked.
 *
 * Taken over from Roedy Mail (`apps/web/src/lib/shell.ts`), trimmed to the
 * hub's shell: no search, notifications, apps menu, sidebar, folders, tags,
 * banners, command palette or keyboard shortcuts.
 */
import { texts } from './i18n.ts';
import type { IconName } from './icons.ts';
import type { KeyInput } from './keys.ts';
import type { LogoValues } from './logo.ts';

const t = texts({
  en: { ok: 'Confirm' },
  de: { ok: 'Bestätigen' },
});

// --- Header ---

export interface HeaderValues {
  isMobile: boolean;
  /** Tablet (desktop layout on touch): 44 instead of 40 px buttons in the toolbar. */
  isTablet?: boolean;
  appName: string;
  logo: LogoValues;
  accountOpen: boolean;
  userName: string;
  initials: string;
  /** Offer the administration (only for administrators) and whether it is open. */
  showAdmin: boolean;
  adminOn: boolean;
  /** Settings (`/account/…`) currently open. */
  settingsOn: boolean;
}

export interface HeaderActions {
  /** The brand: to the home page. */
  goHome?: () => void;
  openAdmin?: () => void;
  openSettingsBtn?: () => void;
  /** Desktop: the account menu below the avatar. */
  toggleAccount?: () => void;
  /** Mobile: the account menu as a sheet. */
  openAccountSheet?: () => void;
}

// --- Settings navigation ---

/** Entry of the navigation of the account and administration area. */
export interface NavEntry {
  id: string;
  label: string;
  icon: IconName;
  /** Current page: card, tile in the accent colour. */
  on: boolean;
  fw: number;
  onClick: () => void;
}

export interface NavGroupValues {
  label: string;
  hasLabel: boolean;
  items: NavEntry[];
}

/** Props of the navigation component (`SettingsNav.svelte`). */
export interface SettingsNavValues {
  isMobile: boolean;
  appName: string;
  logo: LogoValues;
  backLabel: string;
  /** "Back to home" of the desktop bar. */
  panelBack: () => void;
  /** Name of the area: label of the navigation, heading of the list on mobile. */
  mPanelH: string;
  settingsGroups: NavGroupValues[];
}

// --- Account menu ---

export interface ModeSegment {
  label: string;
  icon: IconName;
  on: boolean;
  onClick: () => void;
}

/** Language in the account menu's segment: `system` shows `ti-world` instead of a code. */
export interface LangSegment {
  id: string;
  /** Code on the segment ("DE"); empty for "System". */
  code: string;
  /** Name in the language itself ("Deutsch"); the tooltip of "System". */
  name: string;
  /** Name for screen readers ("EN English", "System – Browser: English"). */
  label: string;
  on: boolean;
  onClick: () => void;
}

export interface AccountMenuValues {
  isMobile: boolean;
  initials: string;
  userName: string;
  userEmail: string;
  modeSeg: ModeSegment[];
  /** Languages like on the page Settings › Language. */
  langSeg: LangSegment[];
  openAccountSettings: () => void;
  /** "Administration" (only for administrators, on mobile where the header has no shield). */
  openAdmin?: (() => void) | undefined;
  logout: () => void;
}

// --- Skeleton ---

/** Settings skeleton: an overview (card and tiles) or a list. */
export type SkeletonSet = 'ueb' | 'list';

interface Row {
  a: string;
  b: string;
  d: string;
  sep: boolean;
}

export interface SkeletonValues {
  skSetUeb: boolean;
  skSetList: boolean;
  skRows3: Row[];
}

/** Widths of the rows in percent. */
const SKW = [
  [38, 62],
  [30, 74],
  [44, 56],
] as const;

export function skeletonValues(set: SkeletonSet = 'list'): SkeletonValues {
  return {
    skSetUeb: set === 'ueb',
    skSetList: set === 'list',
    skRows3: SKW.map(([a, b], i) => ({ a: `${a + 10}%`, b: `${b}%`, d: `${i * 140}ms`, sep: i > 0 })),
  };
}

// --- Confirmation ---

export interface ConfirmDialogValues {
  isMobile: boolean;
  cf: {
    title: string;
    text: string;
    icon: IconName;
    ok: string;
    hasCancel: boolean;
    hasWord: boolean;
    word: string;
    /** Red instead of dark (`tileBg`, `okBg`). */
    danger: boolean;
    /** Word still missing (`okDis`). */
    okDis: boolean;
  };
  confirmIn: string;
  onConfirmIn: (event: Event & { currentTarget: HTMLInputElement }) => void;
  confirmKey: (event: KeyboardEvent) => void;
  confirmOk: () => void;
  confirmCancel: () => void;
  onConfirmOverlay: (event: MouseEvent) => void;
}

/** `cf` from a request (`ask()`); `ready` if the required word matches. Without `ok` the button says "Confirm". */
export function confirmValues(
  request: {
    title: string;
    text: string;
    icon?: IconName;
    ok?: string;
    danger?: boolean;
    word?: string;
    noCancel?: boolean;
  },
  ready: boolean,
): ConfirmDialogValues['cf'] {
  return {
    title: request.title,
    text: request.text,
    icon: request.icon ?? 'ti-help-circle',
    ok: request.ok ?? t.ok(),
    hasCancel: !request.noCancel,
    hasWord: !!request.word,
    word: request.word ?? '',
    danger: !!request.danger,
    okDis: !ready,
  };
}

// --- Select list ---

/** Entry of a select list (`pick.opts[]`, `mkSel` of the shell). */
export interface PickerOption {
  label: string;
  sub: string;
  hasSub: boolean;
  /** Selected value: check mark and bold font (`fw`). */
  on: boolean;
  fw: number;
  onClick: () => void;
}

/** Entry of the select list on the desktop: plus the mark. */
export interface PickOption extends PickerOption {
  /** Marked by arrow key or mouse (`hi`). */
  hi: boolean;
  hover: () => void;
}

/** What the keyboard of a select list needs (`mkSel` of the shell). */
export interface PickKeysInput {
  count: number;
  open: boolean;
  /** Disabled (nothing to choose, step running, mobile): keys do nothing. */
  disabled: boolean;
  /** Selected row with which the list opens. */
  idx: number;
  /** Marked row while the list is open. */
  hi: number;
  /** Set the mark (`null` = none, `selHi`). */
  mark: (i: number | null) => void;
  /** Open or close the list (`ui.menu`). */
  show: () => void;
  hide: () => void;
  choose: (i: number) => void;
}

/**
 * Keyboard of a select list like `mkSel` of the shell: arrow keys open and
 * mark, Enter selects, Tab closes (`keydown`); space selects only on release,
 * otherwise its click would open the list again (`keyup`).
 */
export function pickKeys(input: PickKeysInput): {
  onKey: (event: KeyInput) => void;
  onKeyUp: (event: KeyInput) => void;
} {
  const { count, open, disabled, idx, hi, mark, show, hide, choose } = input;
  return {
    onKey: (event) => {
      if (disabled) return;
      if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
        event.preventDefault();
        if (!open) {
          show();
          mark(idx);
        } else {
          mark((hi + (event.key === 'ArrowDown' ? 1 : -1) + count) % count);
        }
      } else if (event.key === 'Enter' && open) {
        event.preventDefault();
        choose(hi);
      } else if (event.key === ' ' && open) {
        event.preventDefault();
      } else if (event.key === 'Tab' && open) {
        hide();
        mark(null);
      }
    },
    onKeyUp: (event) => {
      if (disabled || event.key !== ' ' || !open) return;
      event.preventDefault();
      choose(hi);
    },
  };
}

/** Values of the select list (on mobile in the sheet). */
export interface PickerValues {
  title: string;
  opts: PickerOption[];
}

export interface PickerListValues {
  pick: PickerValues;
  closeSheet: () => void;
}

/** What `overlayClose` needs from the event (in tests without a real `MouseEvent`). */
export type OverlayEvent = Pick<MouseEvent, 'target' | 'currentTarget' | 'preventDefault'>;

/**
 * A click on the backdrop of a dialog, not on the dialog, closes it
 * (`overlayClose`). Without `preventDefault()`, `mousedown` would take the
 * focus away again from the element that the dialog returns it to on closing.
 */
export function overlayClose(close: () => void): (event: OverlayEvent) => void {
  return (event) => {
    if (event.target !== event.currentTarget) return;
    event.preventDefault();
    close();
  };
}
