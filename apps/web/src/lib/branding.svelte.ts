/**
 * Administration › Branding against `GET/PUT admin/branding`: logo (kind,
 * initial, icon, image with upload, fit, plate, tilt), brand colour with
 * `userAccent`, preview (`brPrev`) and the bar "Unsaved changes" (`brDirty`,
 * `brSave`, `brDiscard`, `brReset`).
 *
 * One source counts as saved: `branding` in `session.config` (from
 * `public/config`); only the name and details of the logo file (`imageName`,
 * `imageMeta`) are known solely to `admin/branding`. The page holds the inputs
 * on top and reloads `admin/branding` when opened. "Save" sends only the
 * changed fields (a new logo as a data URL with name and details, a removed one
 * as `null`, an unchanged one not at all) and takes the response over into
 * `session.config` – logo, accent colour and app icon follow at once without
 * reloading `public/config`. The page shows logo errors in `brErr`, all others
 * in the error toast.
 *
 * Request sequencing: a load only applies if no save started meanwhile, the
 * session's branding is still the one it started from and the page is still
 * open; reading a file only applies if no later file, "Discard", "Remove",
 * "Reset" or other kind came in between; inputs made while a save runs stay
 * as inputs.
 *
 * Taken over from Roedy Mail (`apps/web/src/lib/branding.svelte.ts`) without
 * the app name (the hub's name is the organization's, which also gives the
 * initial), the sign-in page texts and the deferred sending with "Undo" (the
 * hub's toasts have none): "Save" sends at once and shows the busy bar.
 */
import { ACCENTS, DEFAULT_ACCENT, PLATES, type AccentName } from '@roedy/tokens';
import { lostAccess } from '#lib/access.ts';
import { ApiError, api, fieldsOf, isApiError } from '#lib/api.ts';
import { accentLabel, type SegOption, type Toggle } from '#lib/appearance.ts';
import { failText } from '#lib/errors.ts';
import type { IconName } from '#lib/icons.ts';
import { intlLocale } from '#lib/locale.ts';
import { mkLogo, type LogoValues } from '#lib/logo.ts';
import { m } from '#lib/paraglide/messages.js';
import { BRAND0, session, type Branding, type PlateName } from '#lib/state/session.svelte.ts';
import { ui } from '#lib/state/ui.svelte.ts';

export type BrandingSection = 'logo' | 'color';

/** Kinds of logo (`brKindOpts`); the values are those of the API (`instance::KINDS`). */
export const KINDS = ['standard', 'initial', 'icon', 'image'] as const;
export type KindName = (typeof KINDS)[number];
/** Display of an image (`brFitOpts`, `instance::FITS`). */
export const FITS = ['padded', 'fill'] as const;
export type FitName = (typeof FITS)[number];

/** `GET/PUT admin/branding`: the public branding plus name and details of the logo file. */
export interface AdminBranding extends Branding {
  imageName: string;
  imageMeta: string;
}

/** Inputs of the branding form: `image` is the address or a data URL. */
export type BrandingDraft = AdminBranding;

/** Fields of `public/config` › `branding`. */
const BRANDING_KEYS = [
  'kind',
  'letter',
  'icon',
  'image',
  'fit',
  'plate',
  'tilt',
  'color',
  'userAccent',
] as const satisfies readonly (keyof Branding)[];

/** Fields of the draft. */
export const DRAFT_KEYS = [
  ...BRANDING_KEYS,
  'imageName',
  'imageMeta',
] as const satisfies readonly (keyof BrandingDraft)[];

/** Maximum length of the initial (`api::branding::MAX_LETTER`). */
export const MAX_LETTER = 2;

/** Largest logo file in bytes (1 MiB, `logo::upload::MAX_BYTES`). */
export const MAX_FILE = 1_048_576;

/** Accepted logo formats. */
const FILE_TYPES = /^image\/(png|jpeg|webp|svg\+xml)$/;

/** Icons to choose from for the logo (`logo::ICONS` of the server). */
export const BRAND_ICONS: readonly [IconName, () => string][] = [
  ['ti-mountain', m.branding_icon_mountain],
  ['ti-leaf', m.branding_icon_leaf],
  ['ti-bolt', m.branding_icon_bolt],
  ['ti-flame', m.branding_icon_flame],
  ['ti-star', m.branding_icon_star],
  ['ti-heart', m.branding_icon_heart],
  ['ti-cloud', m.branding_icon_cloud],
  ['ti-compass', m.branding_icon_compass],
  ['ti-diamond', m.branding_icon_diamond],
  ['ti-rocket', m.branding_icon_rocket],
  ['ti-feather', m.branding_icon_feather],
  ['ti-planet', m.branding_icon_planet],
];

const KIND_LABELS: Record<KindName, () => string> = {
  standard: m.branding_kind_standard,
  initial: m.branding_kind_initial,
  icon: m.branding_kind_icon,
  image: m.branding_kind_image,
};

const FIT_LABELS: Record<FitName, () => string> = {
  padded: m.branding_fit_pad,
  fill: m.branding_fit_fill,
};

const PLATE_LABELS: Record<PlateName, () => string> = {
  accent: m.branding_plate_accent,
  light: m.branding_plate_light,
  dark: m.branding_plate_dark,
};

/** Section tabs (`brTabs`). */
const BR_TABS: readonly [BrandingSection, () => string][] = [
  ['logo', m.branding_tab_logo],
  ['color', m.branding_tab_color],
];

/**
 * Logo errors from `fields.image` (`logo::upload::Rejected`) or from the check in the browser (`readFile`) with
 * their texts.
 */
const IMAGE_ERRORS = {
  unsupported: m.branding_file_unsupported,
  too_large: m.branding_file_too_large,
  unreadable: m.branding_file_unreadable,
  svg_text: m.branding_file_svg_text,
  dimensions: m.branding_file_dimensions,
  required: m.branding_image_required,
} as const;
export type ImageError = keyof typeof IMAGE_ERRORS;

export function imageErrorText(code: ImageError): string {
  return IMAGE_ERRORS[code]();
}

/**
 * Logo error of a failed save: the code at `fields.image` of 422 `invalid`, or 413 if a proxy in front of the hub
 * refuses the size of the request.
 */
export function imageErrorOf(error: unknown): ImageError | null {
  if (isApiError(error, 413)) return 'too_large';
  if (!(error instanceof ApiError) || error.status !== 422) return null;
  const code = fieldsOf(error).image?.code;
  return code !== undefined && Object.hasOwn(IMAGE_ERRORS, code) ? (code as ImageError) : null;
}

/** File details: "PNG · 12 KB". */
export function fileMeta(file: Pick<File, 'type' | 'size'>): string {
  const type =
    file.type === 'image/svg+xml'
      ? 'SVG'
      : file.type === 'image/jpeg'
        ? 'JPG'
        : (file.type.split('/')[1] ?? '').toUpperCase();
  const kb = Math.max(1, Math.round(file.size / 1024));
  return m.branding_file_meta({ type, kb: kb.toLocaleString(intlLocale()) });
}

/** Checks format and size of a file before reading it. */
export function checkFile(file: Pick<File, 'type' | 'size'>): ImageError | null {
  if (!FILE_TYPES.test(file.type)) return 'unsupported';
  if (file.size > MAX_FILE) return 'too_large';
  return null;
}

/** Largest pixel size of a logo in width and height (`logo::upload::MAX_SIDE`). */
export const MAX_PIXELS = 4096;

const SVG_NS = 'http://www.w3.org/2000/svg';
/** Fallback without a parser: `<text` or `<svg:text`. */
const SVG_TEXT = /<(svg:)?text[\s/>]/i;

/**
 * Element `<text>` in an SVG; the server does not draw text and rejects it. Checked on the parsed document so
 * that `<text` in comments, CDATA or `<style>` does not count: in the SVG namespace (also as `<svg:text>`) and
 * without a namespace, in case the SVG declares none. If the SVG cannot be read as XML or `DOMParser` is missing,
 * the regex applies.
 */
export function svgHasText(svg: string): boolean {
  if (typeof DOMParser !== 'function') return SVG_TEXT.test(svg);
  const doc = new DOMParser().parseFromString(svg, 'image/svg+xml');
  if (doc.getElementsByTagName('parsererror').length > 0) return SVG_TEXT.test(svg);
  return doc.getElementsByTagNameNS(SVG_NS, 'text').length > 0 || doc.getElementsByTagNameNS(null, 'text').length > 0;
}

/**
 * Pixel size of an image via an `Image` from the data URL: `naturalWidth` and `naturalHeight` are set after
 * `onload` from the file header, without fully decoding the image. `createImageBitmap` only if there is no `Image`.
 */
export async function imageSize(file: Blob, url: string): Promise<{ width: number; height: number }> {
  if (typeof Image !== 'function') {
    const bitmap = await createImageBitmap(file);
    const size = { width: bitmap.width, height: bitmap.height };
    bitmap.close();
    return size;
  }
  return new Promise((resolve, reject) => {
    const img = new Image();
    img.onload = () => resolve({ width: img.naturalWidth, height: img.naturalHeight });
    img.onerror = () => reject(new Error('unreadable'));
    img.src = url;
  });
}

/** Default as a draft (`BRAND0`; "Restore defaults", `brNotDefault`). */
export function defaultDraft(): BrandingDraft {
  return { ...BRAND0, imageName: '', imageMeta: '' };
}

/** Do two states differ? The initial counts trimmed. */
export function differs(a: BrandingDraft, b: BrandingDraft): boolean {
  return DRAFT_KEYS.some((k) => (k === 'letter' ? a.letter.trim() !== b.letter.trim() : a[k] !== b[k]));
}

/** Saved shape: the initial trimmed (as the server stores it). */
export function normalized(d: BrandingDraft): BrandingDraft {
  return { ...d, letter: d.letter.trim() };
}

/** Fields of `public/config` › `branding` from a draft or a response. */
export function brandingOf(b: Branding): Branding {
  return Object.fromEntries(BRANDING_KEYS.map((k) => [k, b[k]])) as unknown as Branding;
}

/**
 * Request for `PUT admin/branding`: only changed fields. A new logo goes as a data URL with name and details, a
 * removed one as `null`; an unchanged one is missing (stays).
 */
export function bodyOf(next: BrandingDraft, prev: BrandingDraft): Partial<AdminBranding> {
  const body: Partial<Record<keyof AdminBranding, unknown>> = {};
  for (const k of BRANDING_KEYS) if (k !== 'image' && next[k] !== prev[k]) body[k] = next[k];
  if (next.image !== prev.image) {
    body.image = next.image;
    if (next.image) {
      body.imageName = next.imageName;
      body.imageMeta = next.imageMeta;
    }
  }
  return body as Partial<AdminBranding>;
}

/** Takes a branding over into `session.config` (public view: header, logo, accent colour of everyone's pages). */
function publish(b: Branding): void {
  if (session.config) session.config = { ...session.config, branding: brandingOf(b) };
}

/** Name and details of the saved file, only from `admin/branding`; they apply to the logo under `image`. */
interface FileInfo {
  image: string | null;
  name: string;
  meta: string;
}

export interface BrandingContext {
  scrollTo: (section: BrandingSection) => void;
}

export class BrandingPage {
  /** Name and details of the saved logo file (only known after `admin/branding`). */
  file = $state.raw<FileInfo>({ image: null, name: '', meta: '' });
  /** Saved state (until the response of `admin/branding` the one from `public/config`, without file details). */
  saved = $derived(this.#savedDraft());
  /** Inputs on top of the saved state; fields not named show it. */
  edits = $state.raw<Partial<BrandingDraft>>({});
  draft = $derived<BrandingDraft>({ ...this.saved, ...this.edits });
  dirty = $derived(differs(this.draft, this.saved));
  notDefault = $derived(differs(this.draft, defaultDraft()));
  /** Logo error (`brErr`). */
  error = $state<string | null>(null);
  /** File over the drop zone (`brDrag`). */
  drag = $state(false);
  /** "Save" is running (bar busy). */
  saving = $state(false);

  #ctx: BrandingContext;
  /** Counts loads and saves: a load only applies if it is the latest of them. */
  #seq = 0;
  /** Counts file reads and what cancels them. */
  #read = 0;
  #disposed = false;

  constructor(ctx: BrandingContext) {
    this.#ctx = ctx;
  }

  /** Saved state as a draft: `session` and the file details if they belong to its logo. */
  #savedDraft(): BrandingDraft {
    const b = session.branding;
    const known = this.file.image !== null && this.file.image === b.image;
    return {
      ...brandingOf(b),
      imageName: known ? this.file.name : '',
      imageMeta: known ? this.file.meta : '',
    };
  }

  /** Takes over a response of `admin/branding`. */
  #accept(b: AdminBranding): void {
    publish(b);
    this.file = { image: b.image, name: b.imageName, meta: b.imageMeta };
  }

  /** Loads `admin/branding`; without a response the state from `public/config` stays (without file details). */
  async load(): Promise<void> {
    const seq = ++this.#seq;
    const before = session.config?.branding;
    try {
      const b = await api<AdminBranding>('GET', 'admin/branding');
      // Saved meanwhile, page left, or the session's state changed: that one is newer.
      if (seq !== this.#seq || this.#disposed || session.config?.branding !== before) return;
      this.#accept(b);
    } catch (error) {
      if (seq !== this.#seq || this.#disposed) return;
      // Without a session or role: reload the account, the layout then shows sign-in or "Access denied".
      // Otherwise the state from `public/config` stays; the page has no error state.
      lostAccess(error);
    }
  }

  /** Input into a field. */
  set<K extends keyof BrandingDraft>(key: K, value: BrandingDraft[K]): void {
    this.edits = { ...this.edits, [key]: value };
  }

  /** Kind of logo (`brKindOpts`); clears the logo error and cancels a running read. */
  setKind(kind: KindName): void {
    this.#read++;
    this.set('kind', kind);
    this.error = null;
  }

  /** Initial (`onBrLetter`): at most two characters, without splitting surrogate pairs (emoji). */
  setLetter(value: string): void {
    this.set('letter', Array.from(value).slice(0, MAX_LETTER).join(''));
  }

  /**
   * Choose or drop a file: checks format and size, then text in the SVG or the pixel size, reads it as a data URL
   * and switches to "Image".
   */
  async readFile(f: File | null | undefined): Promise<void> {
    if (!f) return;
    const seq = ++this.#read;
    const bad = checkFile(f);
    if (bad) {
      this.error = imageErrorText(bad);
      return;
    }
    const svg = f.type === 'image/svg+xml';
    let url = '';
    let content: ImageError | null = null;
    try {
      // An SVG without text, an image at most 4096 × 4096 pixels.
      if (svg && svgHasText(await f.text())) content = 'svg_text';
      else url = await readAsDataUrl(f);
      if (!svg && !content) {
        const size = await imageSize(f, url);
        if (size.width > MAX_PIXELS || size.height > MAX_PIXELS) content = 'dimensions';
      }
    } catch {
      if (seq === this.#read && !this.#disposed) this.error = imageErrorText('unreadable');
      return;
    }
    // A later file, Discard, Remove, Reset or another kind win.
    if (seq !== this.#read || this.#disposed) return;
    if (content) {
      this.error = imageErrorText(content);
      return;
    }
    this.edits = { ...this.edits, kind: 'image', image: url, imageName: f.name, imageMeta: fileMeta(f) };
    this.error = null;
  }

  /** "Remove image" (`brImgRemove`). */
  removeImage(): void {
    this.#read++;
    this.edits = { ...this.edits, image: null, imageName: '', imageMeta: '' };
    this.error = null;
  }

  /** "Discard" (`brDiscard`). */
  discard(): void {
    if (this.saving) return;
    this.#read++;
    this.edits = {};
    this.error = null;
  }

  /** "Restore defaults" (`brReset`): default as a draft, saved only with "Save". */
  reset(): void {
    this.#read++;
    this.edits = defaultDraft();
    this.error = null;
  }

  /** "Save" (`brSave`): sends the changed fields and takes the response over. */
  async save(): Promise<void> {
    if (this.saving) return;
    const draft = this.draft;
    if (draft.kind === 'image' && !draft.image) {
      ui.toasts.show(imageErrorText('required'));
      this.#ctx.scrollTo('logo');
      return;
    }
    if (!this.dirty) return;
    const next = normalized(draft);
    const body = bodyOf(next, this.saved);
    const sent = this.edits;
    // Inputs made while the request runs stay as inputs; the ones sent go.
    const settle = (): void => {
      this.edits = Object.fromEntries(
        Object.entries(this.edits).filter(([k, v]) => sent[k as keyof BrandingDraft] !== v),
      );
    };
    if (Object.keys(body).length === 0) {
      // Only whitespace changed: nothing to send.
      settle();
      this.error = null;
      ui.toasts.show(m.branding_saved());
      return;
    }
    // A load still running would bring back the state before the save.
    this.#seq++;
    this.saving = true;
    try {
      const saved = await api<AdminBranding>('PUT', 'admin/branding', body);
      this.#accept(saved);
      settle();
      this.error = null;
      ui.toasts.show(m.branding_saved());
    } catch (error) {
      this.#failed(error);
    } finally {
      this.saving = false;
    }
  }

  /** A failed save: the inputs stay with the bar; logo errors at the logo, all others as the error toast. */
  #failed(error: unknown): void {
    if (lostAccess(error)) {
      ui.toasts.error(failText(error, m.error_act()));
      return;
    }
    const code = imageErrorOf(error);
    // Page already left: the logo error would not arrive there any more, so as a toast.
    if (code && !this.#disposed) {
      this.error = imageErrorText(code);
      this.#ctx.scrollTo('logo');
      return;
    }
    // Other 422s (lengths, values): the inputs limit them already, so there are no texts per field.
    ui.toasts.error(code ? imageErrorText(code) : failText(error, m.error_act()));
  }

  dispose(): void {
    this.#disposed = true;
  }
}

function readAsDataUrl(f: Blob): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => (typeof reader.result === 'string' ? resolve(reader.result) : reject(new Error('empty')));
    reader.onerror = () => reject(reader.error ?? new Error('unreadable'));
    reader.readAsDataURL(f);
  });
}

/** Preview (`brPrev`): logo on a plate or brand colour, the hub's name, tilt. */
export interface BrandingPreview {
  logo: LogoValues;
  name: string;
  bg: string;
  ink: string;
  /** `rotate(-4deg)` or `none`. */
  tilt: string;
}

export interface BrandingIcon {
  icon: IconName;
  label: string;
  pressed: boolean;
  onClick: () => void;
}

export interface BrandingColor {
  name: AccentName;
  label: string;
  base: string;
  ink: string;
  on: boolean;
  onClick: () => void;
}

/** Values of the branding page. */
export interface BrandingValues {
  brTabs: { label: string; onClick: () => void }[];
  brPrev: BrandingPreview;
  brKindOpts: SegOption<KindName>[];
  brKindExtra: boolean;
  brIsInit: boolean;
  brIsIcon: boolean;
  brIsImg: boolean;
  brLetter: string;
  brLetterPh: string;
  onBrLetter: (value: string) => void;
  brIcons: BrandingIcon[];
  /** File from the picker or dropped (`onBrFile`, `brDrop`). */
  onBrFile: (file: File | null | undefined) => void;
  /** File over the drop zone. */
  brDrag: boolean;
  onBrDrag: (on: boolean) => void;
  brHasImg: boolean;
  brNoImg: boolean;
  brImgName: string;
  brImgMeta: string;
  brImgRemove: () => void;
  brErr: string;
  brHasErr: boolean;
  brFitOpts: SegOption<FitName>[];
  brPlateOpts: SegOption<PlateName>[];
  tgBrTilt: Toggle;
  brColors: BrandingColor[];
  tgBrUserAcc: Toggle;
  brDirty: boolean;
  brSaving: boolean;
  brNotDefault: boolean;
  brSave: () => void;
  brDiscard: () => void;
  brReset: () => void;
}

export function brandingValues(p: BrandingPage, ctx: BrandingContext): BrandingValues {
  const d = p.draft;
  const name = session.appName;
  const plate = PLATES[d.plate] ?? null;
  const accent = ACCENTS[d.color] ?? ACCENTS[DEFAULT_ACCENT];
  const seg = <T extends string>(
    values: readonly T[],
    current: string,
    labels: Record<T, () => string>,
    pick: (v: T) => void,
  ) => values.map((value) => ({ value, label: labels[value](), on: current === value, onClick: () => pick(value) }));
  return {
    brTabs: BR_TABS.map(([section, label]) => ({ label: label(), onClick: () => ctx.scrollTo(section) })),
    brPrev: {
      logo: mkLogo(d, name),
      name,
      bg: plate ? plate[0] : accent.base,
      ink: plate ? plate[1] : accent.ink,
      tilt: d.tilt ? 'rotate(-4deg)' : 'none',
    },
    brKindOpts: seg(KINDS, d.kind, KIND_LABELS, (v) => p.setKind(v)),
    brKindExtra: d.kind !== 'standard',
    brIsInit: d.kind === 'initial',
    brIsIcon: d.kind === 'icon',
    brIsImg: d.kind === 'image',
    brLetter: d.letter,
    brLetterPh: (name.trim()[0] ?? 'A').toUpperCase(),
    onBrLetter: (value) => p.setLetter(value),
    brIcons: BRAND_ICONS.map(([icon, label]) => ({
      icon,
      label: label(),
      pressed: d.icon === icon,
      onClick: () => p.set('icon', icon),
    })),
    onBrFile: (f) => void p.readFile(f),
    brDrag: p.drag,
    onBrDrag: (on) => {
      p.drag = on;
    },
    brHasImg: !!d.image,
    brNoImg: !d.image,
    brImgName: d.imageName || m.branding_file_name(),
    brImgMeta: d.imageMeta,
    brImgRemove: () => p.removeImage(),
    brErr: p.error ?? '',
    brHasErr: p.error !== null,
    brFitOpts: seg(FITS, d.fit, FIT_LABELS, (v) => p.set('fit', v)),
    brPlateOpts: seg(Object.keys(PLATES) as PlateName[], d.plate, PLATE_LABELS, (v) => p.set('plate', v)),
    tgBrTilt: { on: d.tilt, onClick: () => p.set('tilt', !p.draft.tilt) },
    brColors: (Object.keys(ACCENTS) as AccentName[]).map((color) => ({
      name: color,
      label: accentLabel(color),
      base: ACCENTS[color].base,
      ink: ACCENTS[color].ink,
      on: d.color === color,
      onClick: () => p.set('color', color),
    })),
    tgBrUserAcc: { on: d.userAccent, onClick: () => p.set('userAccent', !p.draft.userAccent) },
    brDirty: p.dirty,
    brSaving: p.saving,
    brNotDefault: p.notDefault,
    brSave: () => void p.save(),
    brDiscard: () => p.discard(),
    brReset: () => p.reset(),
  };
}
