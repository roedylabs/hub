/**
 * Administration › Organization against `GET/PUT admin/org`: name, support,
 * legal and source code, with the "Unsaved changes" bar with save and discard.
 *
 * The saved state has one source: `org` in `session.config` (from
 * `public/config`, the same fields). The page only keeps the inputs on top
 * and loads `admin/org` on opening. "Save" sends the changed fields right
 * away (the bar shows "Saving …"); the response becomes `org` and the hub's
 * name in `session.config` – header, logo initial and the sign-in footers
 * follow – and the toast "Changes saved" confirms it. Inputs made while the
 * request runs stay. If it fails, the draft stays with the bar.
 *
 * Field errors: the page checks live per the backend's rules (`orgError`) and
 * shows the text at the field. Saving with an error shows the toast "Please
 * check the highlighted fields", jumps to the section of the first field and
 * sends nothing. A 422 from the server (`fields.<field>` with
 * `invalid`/`too_long`) shows the same texts at the field instead of the error
 * toast until the field is changed; the page does not add `https://`.
 *
 * Requests are sequenced: a response of `admin/org` that started before a
 * save (or an earlier `load`) never overwrites the newer state, also across
 * page instances (a save that answers after the page was left and reopened
 * does not replace a newer save's state), and nothing writes page state after
 * `dispose`.
 *
 * Taken over from Roedy Mail (`apps/web/src/lib/org.svelte.ts`) without the
 * deferred save with undo (the hub's toasts only inform) and the link to
 * Settings › About, which the hub does not have.
 */
import { lostAccess } from '#lib/access.ts';
import { api, fieldsOf, isApiError } from '#lib/api.ts';
import { failText } from '#lib/errors.ts';
import { m } from '#lib/paraglide/messages.js';
import { session, type Org } from '#lib/state/session.svelte.ts';
import { ui } from '#lib/state/ui.svelte.ts';
import { trimWs } from '#lib/trim.ts';

export type OrgKey = keyof Org;
export type OrgSection = 'firm' | 'support' | 'legal' | 'source';
/** Inputs per field; `''` means "not given". */
export type OrgDraft = Record<OrgKey, string>;

export const ORG_KEYS = [
  'name',
  'website',
  'help',
  'email',
  'imprint',
  'privacy',
  'terms',
  'source',
] as const satisfies readonly OrgKey[];

export const NO_ORG: Org = {
  name: null,
  website: null,
  help: null,
  email: null,
  imprint: null,
  privacy: null,
  terms: null,
  source: null,
};

/** Reasons of a field error like `fields.<field>.code` of `PUT admin/org`. */
export type OrgError = 'invalid' | 'too_long';
export type OrgErrors = Partial<Record<OrgKey, OrgError>>;

/** Maximum lengths in characters like the backend (`MAX_NAME`, `MAX_EMAIL`, `MAX_LINK`). */
export const ORG_MAX = { name: 120, email: 254, link: 2048 } as const;

/** Control characters like Rust `char::is_control` (category Cc). */
const CONTROL = /\p{Cc}/u;
/** Whitespace like Rust `char::is_whitespace`. */
const SPACE = /\p{White_Space}/u;

function hasHost(value: string): boolean {
  try {
    return new URL(value).hostname !== '';
  } catch {
    return false;
  }
}

/**
 * Error of a field, like `org_error` in the backend (`api::organization`): the value is trimmed like Rust
 * `str::trim` (a U+0085 at the edges goes, one inside is a control character), empty is never an error; characters
 * count like Rust `chars()`. Name: over 120 characters `too_long`, control characters `invalid`. Email: over 254
 * `too_long`, `invalid` without `local@domain` at the first `@` or with whitespace, `?` or control characters.
 * Links: over 2048 `too_long`, `invalid` without `http://` or `https://`, with whitespace or control characters or
 * without a host.
 */
export function orgError(key: OrgKey, value: string): OrgError | null {
  const t = trimWs(value);
  if (!t) return null;
  const n = Array.from(t).length;
  const ctl = CONTROL.test(t);
  if (key === 'name') return n > ORG_MAX.name ? 'too_long' : ctl ? 'invalid' : null;
  if (key === 'email') {
    if (n > ORG_MAX.email) return 'too_long';
    const at = t.indexOf('@');
    return at < 1 || at === t.length - 1 || SPACE.test(t) || t.includes('?') || ctl ? 'invalid' : null;
  }
  if (n > ORG_MAX.link) return 'too_long';
  return /^https?:\/\//i.test(t) && !SPACE.test(t) && !ctl && hasHost(t) ? null : 'invalid';
}

/** Text of a field error. */
export function orgErrorText(key: OrgKey, code: OrgError): string {
  if (key === 'name') return code === 'too_long' ? m.org_err_name_long() : m.org_err_name_invalid();
  if (key === 'email') return code === 'too_long' ? m.org_err_email_long() : m.org_err_email();
  return code === 'too_long' ? m.org_err_link_long() : m.org_err_link();
}

/** Field errors from a 422 of `PUT admin/org`; unknown fields and reasons do not count. */
export function orgErrorsOf(error: unknown): OrgErrors {
  if (!isApiError(error, 422)) return {};
  const out: OrgErrors = {};
  for (const [key, detail] of Object.entries(fieldsOf(error))) {
    if ((ORG_KEYS as readonly string[]).includes(key) && (detail.code === 'invalid' || detail.code === 'too_long'))
      out[key as OrgKey] = detail.code;
  }
  return out;
}

/** Placeholder of the links; not text, hence not in `messages`. */
const URL_PH = (): string => 'https://';

interface FieldDef {
  k: OrgKey;
  label: () => string;
  desc: () => string;
  type: 'text' | 'url' | 'email';
  ph: () => string;
}

/** Fields per section. */
export const ORG_FIELDS: Record<OrgSection, readonly FieldDef[]> = {
  firm: [
    { k: 'name', label: m.org_name, desc: m.org_name_desc, type: 'text', ph: m.org_name_ph },
    { k: 'website', label: m.org_website, desc: m.org_website_desc, type: 'url', ph: URL_PH },
  ],
  support: [
    { k: 'help', label: m.org_help, desc: m.org_help_desc, type: 'url', ph: URL_PH },
    { k: 'email', label: m.org_email, desc: m.org_email_desc, type: 'email', ph: m.org_email_ph },
  ],
  legal: [
    { k: 'imprint', label: m.org_imprint, desc: m.org_imprint_desc, type: 'url', ph: URL_PH },
    { k: 'privacy', label: m.org_privacy, desc: m.org_privacy_desc, type: 'url', ph: URL_PH },
    { k: 'terms', label: m.org_terms, desc: m.org_terms_desc, type: 'url', ph: URL_PH },
  ],
  source: [{ k: 'source', label: m.org_source, desc: m.org_source_desc_field, type: 'url', ph: URL_PH }],
};

/** Section tabs. */
const ORG_TABS: readonly [OrgSection, () => string][] = [
  ['firm', m.org_tab_firm],
  ['support', m.org_tab_support],
  ['legal', m.org_tab_legal],
  ['source', m.org_tab_source],
];

/** Inputs from a saved organization (`null` → `''`). */
export function draftOf(org: Org): OrgDraft {
  return Object.fromEntries(ORG_KEYS.map((k) => [k, org[k] ?? ''])) as OrgDraft;
}

/** Saved form: trimmed like Rust `str::trim`, empty means `null` (like the backend). */
export function orgOf(draft: OrgDraft): Org {
  const org = { ...NO_ORG };
  for (const k of ORG_KEYS) org[k] = trimWs(draft[k]) || null;
  return org;
}

/** Section of the first field with an error in page order. */
export function firstBadSection(errors: OrgErrors): OrgSection | null {
  for (const [section] of ORG_TABS) if (ORG_FIELDS[section].some((f) => errors[f.k])) return section;
  return null;
}

/**
 * Takes a saved organization into `session.config`: `org` and the hub's name (`public/config` › `name`, the
 * server's `Org::hub_name`: the organization's name, otherwise "Roedy Hub"), which the header, the logo's initial
 * and the document titles read.
 */
export function applyOrg(org: Org): void {
  const config = session.config;
  if (!config) return;
  session.config = { ...config, org, name: org.name ?? m.app_name() };
}

/**
 * Saves across page instances (each opening creates a new `OrgPage`): numbered when they start; a response only
 * goes into the session if no later save has been taken over. A save that answers after the page was left still
 * applies if it is the latest, so the session follows the server.
 */
const saves = { started: 0, applied: 0 };

export class OrgPage {
  /** Saved state: `org` of `session.config` (until the response of `admin/org` the one from `public/config`). */
  saved = $derived(session.config?.org ?? NO_ORG);
  /** Inputs on top of the saved state; fields not named show it. */
  edits = $state.raw<Partial<OrgDraft>>({});
  draft = $derived({ ...draftOf(this.saved), ...this.edits });
  dirty = $derived(ORG_KEYS.some((k) => this.draft[k] !== (this.saved[k] ?? '')));
  /** `PUT admin/org` is running ("Saving …" in the bar). */
  saving = $state(false);
  /** Errors from a 422 of the server; a field loses its error with the next input. */
  serverErrors = $state.raw<OrgErrors>({});
  /** Error per field: the check in the browser, otherwise the server's. */
  errors = $derived.by((): OrgErrors => {
    const out: OrgErrors = {};
    for (const k of ORG_KEYS) {
      const code = orgError(k, this.draft[k]) ?? this.serverErrors[k];
      if (code) out[k] = code;
    }
    return out;
  });

  #ctx: Pick<OrgContext, 'scrollTo'>;
  /** Counts loads and saves: a response of `admin/org` only applies if nothing started after it. */
  #seq = 0;
  #disposed = false;

  constructor(ctx: Pick<OrgContext, 'scrollTo'>) {
    this.#ctx = ctx;
  }

  /** Loads `admin/org`; without a response the state from `public/config` stays. */
  async load(): Promise<void> {
    const seq = ++this.#seq;
    const started = saves.started;
    const before = session.config?.org;
    try {
      const org = await api<Org>('GET', 'admin/org');
      // Left, loaded again or saved in the meantime (also on another page instance): the newer state applies.
      if (seq !== this.#seq || this.#disposed || this.saving) return;
      if (saves.started !== started || session.config?.org !== before) return;
      applyOrg(org);
    } catch (error) {
      if (seq !== this.#seq || this.#disposed) return;
      // Without a session or role: reload the account, the layout then shows the sign-in or "Access denied".
      // Otherwise the state from `public/config` stays – the same details, the page has no error state.
      lostAccess(error);
    }
  }

  /** Input into a field. */
  set(key: OrgKey, value: string): void {
    this.edits = { ...this.edits, [key]: value };
    if (this.serverErrors[key]) {
      const rest = { ...this.serverErrors };
      delete rest[key];
      this.serverErrors = rest;
    }
  }

  /** "Discard" (not while saving: the bar blocks it). */
  discard(): void {
    if (this.saving) return;
    this.edits = {};
    this.serverErrors = {};
  }

  /** "Save"; with a field error only toast and jump. */
  async save(): Promise<void> {
    if (!this.dirty || this.saving) return;
    const bad = firstBadSection(this.errors);
    if (bad) {
      ui.toasts.show(m.org_check());
      this.#ctx.scrollTo(bad);
      return;
    }
    this.serverErrors = {};
    const draft = this.draft;
    const prev = this.saved;
    const next = orgOf(draft);
    const body: Partial<Org> = {};
    for (const k of ORG_KEYS) if (next[k] !== prev[k]) body[k] = next[k];
    // Only whitespace changed: nothing to send.
    if (Object.keys(body).length === 0) {
      this.edits = {};
      ui.toasts.show(m.org_saved());
      return;
    }
    // A load still running started before this save: its response is older.
    this.#seq++;
    const ticket = ++saves.started;
    this.saving = true;
    try {
      const org = await api<Org>('PUT', 'admin/org', body);
      // The session state is global: it follows the server even if the page was left, unless a save started later
      // (on a page opened since) has already been taken over.
      if (ticket > saves.applied) {
        saves.applied = ticket;
        applyOrg(org);
      }
      // Inputs made during the request stay; the sent ones are saved now.
      const rest: Partial<OrgDraft> = {};
      for (const k of ORG_KEYS) {
        const value = this.edits[k];
        if (value !== undefined && value !== draft[k]) rest[k] = value;
      }
      this.edits = rest;
      ui.toasts.show(m.org_saved());
    } catch (error) {
      if (lostAccess(error)) return;
      // 422 per field: the text at the field while it shows the sent value.
      const found = orgErrorsOf(error);
      const fields: OrgErrors = {};
      for (const k of ORG_KEYS) if (found[k] && this.draft[k] === draft[k]) fields[k] = found[k];
      const first = ORG_KEYS.find((k) => fields[k]);
      if (first && !this.#disposed) {
        this.serverErrors = { ...this.serverErrors, ...fields };
        const section = firstBadSection(fields);
        if (section) this.#ctx.scrollTo(section);
        return;
      }
      // Page already left: the text would no longer reach the field, so as a toast.
      const code = first && fields[first];
      ui.toasts.error(first && code ? orgErrorText(first, code) : failText(error, m.error_act()));
    } finally {
      this.saving = false;
    }
  }

  dispose(): void {
    this.#disposed = true;
  }
}

export interface OrgField {
  k: OrgKey;
  label: string;
  desc: string;
  type: 'text' | 'url' | 'email';
  ph: string;
  value: string;
  /** Divider before the field (from the second one). */
  sep: boolean;
  /** Field not empty and invalid, or a 422 of the server: red ring, text below. */
  bad: boolean;
  /** `aria-invalid` of the field. */
  badA: 'true' | 'false';
  /** Error text below the field, empty without an error. */
  err: string;
  onChange: (value: string) => void;
}

/** Values of the page. */
export interface OrgValues {
  orgTabs: { label: string; onClick: () => void }[];
  orgFirm: OrgField[];
  orgSupport: OrgField[];
  orgLegal: OrgField[];
  orgSource: OrgField[];
  orgDirty: boolean;
  orgSaving: boolean;
  orgSave: () => void;
  orgDiscard: () => void;
}

export interface OrgContext {
  scrollTo: (section: OrgSection) => void;
}

export function orgValues(p: OrgPage, ctx: OrgContext): OrgValues {
  const errorOf = (k: OrgKey): Pick<OrgField, 'bad' | 'badA' | 'err'> => {
    const code = p.errors[k];
    return { bad: !!code, badA: code ? 'true' : 'false', err: code ? orgErrorText(k, code) : '' };
  };
  const fields = (section: OrgSection): OrgField[] =>
    ORG_FIELDS[section].map((f, i) => ({
      k: f.k,
      label: f.label(),
      desc: f.desc(),
      type: f.type,
      ph: f.ph(),
      value: p.draft[f.k],
      sep: i > 0,
      ...errorOf(f.k),
      onChange: (value: string) => p.set(f.k, value),
    }));
  return {
    orgTabs: ORG_TABS.map(([section, label]) => ({ label: label(), onClick: () => ctx.scrollTo(section) })),
    orgFirm: fields('firm'),
    orgSupport: fields('support'),
    orgLegal: fields('legal'),
    orgSource: fields('source'),
    orgDirty: p.dirty,
    orgSaving: p.saving,
    orgSave: () => void p.save(),
    orgDiscard: () => p.discard(),
  };
}
