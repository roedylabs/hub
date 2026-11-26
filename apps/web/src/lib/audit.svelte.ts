/**
 * Administration › Audit log against `admin/audit`: search, filters with counts,
 * entries by day, export and retention.
 *
 * Texts, icons and counts come ready-made from the server in the
 * administrator's language; only the days (`aGroups`) and times are built here,
 * from `at` (UTC, RFC 3339) in the browser's time zone (the hub has no time
 * zone setting) and the UI language (`prefs.lang`). The search only goes out
 * after a typing pause: on the server it reads all rows of the retention
 * period and counts towards a per-administrator limit. The export is `POST`
 * and loads the file via `fetch`; it is itself logged, so the page reloads
 * afterwards, as it does after a changed retention period.
 *
 * Request sequencing: every list request (first page, further pages) belongs
 * to a generation (`#seq`); a newer `load()` supersedes the running one (its
 * response is dropped and the request aborted), a further page that comes back
 * after a change of filter or search is dropped too, and nothing writes state
 * after `dispose()`. The retention has its own generation.
 *
 * Loading: before the first response counts and retention stay empty, an empty
 * frame stands instead of the days (`aFirst`); when filter or search change, the
 * previous list with its counts stays until the response (`aBusyAttr`). If
 * loading fails, each part shows its card (`aLoad`, `aKeepLoad`) with "Try
 * again", the counts are empty; further pages are loaded only by the button
 * below the last day (`aMore`).
 *
 * Taken over from Roedy Mail (`apps/web/src/lib/audit.svelte.ts`) with the
 * category `apps`, times in the browser's time zone and the UI language
 * instead of Mail's time settings, the year on days of another year, and
 * the hub's errors (429 `rate_limited`, 422 `export_too_large`).
 */
import { lostAccess } from '#lib/access.ts';
import { api, apiBlob, isApiError, retryAfter } from '#lib/api.ts';
import { downloadBlob } from '#lib/download.ts';
import { failText, loadFailText } from '#lib/errors.ts';
import { clock, dayLabel, dayNumber, exportName, type AuditTime } from '#lib/audit-time.ts';
import { isIconName, type IconName } from '#lib/icons.ts';
import { m } from '#lib/paraglide/messages.js';
import { ui } from '#lib/state/ui.svelte.ts';
import { waitText } from '#lib/wait.ts';

export type AuditCat = 'login' | 'users' | 'apps' | 'settings' | 'system';
/** Filter (`aFilter`); `all` = "All". */
export type AuditFilter = 'all' | AuditCat;
export type KeepDays = 30 | 90 | 365;

/** Entry from `GET admin/audit`; `act` follows `who`, `at` in UTC. */
export interface AuditEntry {
  id: number;
  at: string;
  cat: AuditCat;
  icon: string;
  who: string;
  act: string;
  meta: string;
  warn: boolean;
}

export type AuditCounts = Record<AuditFilter, number>;

/** Response of `GET admin/audit`. */
export interface AuditData {
  entries: AuditEntry[];
  counts: AuditCounts;
  /** `before` of the next page; `null` on the last one. */
  next: number | null;
}

/** Filters in display order: "All", then the categories as the server counts them. */
const FILTERS: readonly [AuditFilter, () => string][] = [
  ['all', m.audit_all],
  ['login', m.audit_cat_login],
  ['users', m.audit_cat_users],
  ['apps', m.audit_cat_apps],
  ['settings', m.audit_cat_settings],
  ['system', m.audit_cat_system],
];

/** Selectable retention periods in days (the server's `KEEP_CHOICES`). */
const KEEP: readonly [KeepDays, () => string][] = [
  [30, m.audit_keep_30],
  [90, m.audit_keep_90],
  [365, m.audit_keep_365],
];

/** Retention period in a sentence (`aKeepSum`): "1 year" becomes "one year". */
const KEEP_PERIOD: Record<KeepDays, () => string> = {
  30: m.audit_keep_30,
  90: m.audit_keep_90,
  365: m.audit_keep_year,
};

/** Typing pause of the search in milliseconds; every search reads all rows on the server. */
export const AUDIT_SEARCH_DELAY = 400;

const LIST = 'admin/audit';
const EXPORT = 'admin/audit/export';
const RETENTION = 'admin/audit/retention';

/** Path of the request; empty values are left out. */
export function auditPath(base: string, filter: AuditFilter, q: string, before: number | null = null): string {
  const query: [string, string][] = [];
  if (filter !== 'all') query.push(['cat', filter]);
  if (q) query.push(['q', q]);
  if (before !== null) query.push(['before', String(before)]);
  return query.length ? `${base}?${query.map(([k, v]) => `${k}=${encodeURIComponent(v)}`).join('&')}` : base;
}

// --- State ------------------------------------------------------------------

/** Text of the error toast of the export: its own limits first. */
function exportFailText(error: unknown): string {
  if (isApiError(error, 422, 'export_too_large')) return m.audit_export_too_large();
  if (isApiError(error, 429)) return m.error_limit({ wait: waitText(retryAfter(error)) });
  return failText(error);
}

export class AuditPage {
  /** Search input (`aQ`); the search runs after the typing pause. */
  q = $state('');
  filter = $state<AuditFilter>('all');
  entries = $state.raw<AuditEntry[]>([]);
  /** Counts at the filters; `null` before the first response. */
  counts = $state.raw<AuditCounts | null>(null);
  /** `before` of the next page; `null` on the last one. */
  next = $state<number | null>(null);
  /** A request for the first page is running. */
  loading = $state(false);
  loadingMore = $state(false);
  /** Reason of the last failure; `null` = no error. */
  error = $state<string | null>(null);
  /** Retention period (`aKeep`); `null` while it is not loaded. */
  keep = $state<KeepDays | null>(null);
  keepError = $state<string | null>(null);

  /** Search text of the shown list (or the one on its way); the input may already be further. */
  #sent = '';
  /** Generation of the list: every `load()` counts up, older responses are dropped. */
  #seq = 0;
  /** Aborts the list requests of the current generation (on the next `load()` and on `dispose()`). */
  #abort: AbortController | null = null;
  /** The running `load()`, for the export that waits for it. */
  #current: Promise<void> = Promise.resolve();
  #keepSeq = 0;
  #timer: ReturnType<typeof setTimeout> | undefined;
  #exporting = false;
  #saving = false;
  #disposed = false;

  open(): void {
    void this.load();
    void this.loadKeep();
  }

  dispose(): void {
    this.#disposed = true;
    this.#stopTimer();
    this.#abort?.abort();
    this.#abort = null;
  }

  #stopTimer(): void {
    clearTimeout(this.#timer);
    this.#timer = undefined;
  }

  /** Loads the first page of the current selection; a later request supersedes a running one. */
  load(): Promise<void> {
    if (this.#disposed) return Promise.resolve();
    this.#stopTimer();
    const seq = ++this.#seq;
    this.#abort?.abort();
    const abort = (this.#abort = new AbortController());
    const q = (this.#sent = this.q.trim());
    const filter = this.filter;
    this.loading = true;
    this.loadingMore = false;
    const run = (async () => {
      try {
        const data = await api<AuditData>('GET', auditPath(LIST, filter, q), undefined, { signal: abort.signal });
        if (seq !== this.#seq || this.#disposed) return;
        this.entries = data.entries;
        this.counts = data.counts;
        this.next = data.next;
        this.error = null;
      } catch (error) {
        if (seq !== this.#seq || this.#disposed) return;
        lostAccess(error);
        this.error = loadFailText(error);
        // The previous list no longer applies: counts empty, "Try again" starts like the first load.
        this.entries = [];
        this.counts = null;
        this.next = null;
      } finally {
        if (seq === this.#seq && !this.#disposed) this.loading = false;
      }
    })();
    this.#current = run;
    return run;
  }

  /** "Try again": the card disappears while loading (empty frame as at the first load). */
  retry(): Promise<void> {
    this.error = null;
    return this.load();
  }

  /** Appends the next page; if a `load()` comes in between, the response is dropped. */
  async more(): Promise<void> {
    const before = this.next;
    if (before === null || this.loading || this.loadingMore || this.#disposed) return;
    const seq = this.#seq;
    const signal = this.#abort?.signal;
    this.loadingMore = true;
    try {
      const data = await api<AuditData>(
        'GET',
        auditPath(LIST, this.filter, this.#sent, before),
        undefined,
        signal ? { signal } : {},
      );
      if (seq !== this.#seq || this.#disposed) return;
      this.entries = [...this.entries, ...data.entries];
      this.counts = data.counts;
      this.next = data.next;
    } catch (error) {
      if (seq !== this.#seq || this.#disposed) return;
      lostAccess(error);
      ui.toasts.error(loadFailText(error));
    } finally {
      if (seq === this.#seq && !this.#disposed) this.loadingMore = false;
    }
  }

  /** Search input (`onAQ`); spaces at the edges do not change the search. */
  setQuery(value: string): void {
    this.q = value;
    this.#stopTimer();
    if (this.#disposed) return;
    if (value.trim() !== this.#sent) this.#timer = setTimeout(() => void this.load(), AUDIT_SEARCH_DELAY);
  }

  /** "Clear search" (`clearAQ`) takes effect at once. */
  clearQuery(): void {
    this.q = '';
    this.#stopTimer();
    if (this.#sent) void this.load();
  }

  setFilter(filter: AuditFilter): void {
    if (filter === this.filter) return;
    this.filter = filter;
    void this.load();
  }

  /**
   * Waits until the list shows the current filter and search: a pending search goes out at once, a running load
   * (and any that supersedes it) is awaited, a failed list is loaded again. `false` if it still fails.
   */
  async #settled(): Promise<boolean> {
    if (this.error !== null && !this.loading && this.#timer === undefined) await this.load();
    while (!this.#disposed && (this.#timer !== undefined || this.loading)) {
      await (this.#timer !== undefined ? this.load() : this.#current);
    }
    return !this.#disposed && this.error === null;
  }

  /**
   * "Export as CSV" (`aExport`). A pending or running search goes first: file and count in the toast apply to the
   * list that is shown.
   */
  async exportCsv(tz: string): Promise<void> {
    if (this.#exporting) return;
    this.#exporting = true;
    try {
      if (!(await this.#settled())) return;
      const filter = this.filter;
      const n = this.counts?.[filter] ?? 0;
      const blob = await apiBlob('POST', auditPath(EXPORT, filter, this.#sent));
      if (!downloadBlob(exportName(Date.now(), tz), blob)) ui.toasts.error(m.error_act());
      else ui.toasts.show(n === 1 ? m.audit_exported_one() : m.audit_exported({ n }));
      // The export is itself an entry.
      void this.load();
    } catch (error) {
      lostAccess(error);
      ui.toasts.error(exportFailText(error));
    } finally {
      this.#exporting = false;
    }
  }

  /** Loads the retention period; a later call supersedes a running one. */
  async loadKeep(): Promise<void> {
    if (this.#disposed) return;
    const seq = ++this.#keepSeq;
    this.keepError = null;
    try {
      const { days } = await api<{ days: KeepDays }>('GET', RETENTION);
      if (seq !== this.#keepSeq || this.#disposed) return;
      this.keep = days;
    } catch (error) {
      if (seq !== this.#keepSeq || this.#disposed) return;
      lostAccess(error);
      this.keepError = loadFailText(error);
    }
  }

  /**
   * Choose the retention period; it applies at once. Clicks while saving do not count, otherwise the server could
   * see the last choice before the second-to-last.
   */
  async setKeep(days: KeepDays): Promise<void> {
    const before = this.keep;
    if (before === null || days === before || this.#saving || this.#disposed) return;
    this.#saving = true;
    // A load still on its way must not undo the choice.
    this.#keepSeq++;
    this.keep = days;
    try {
      await api('PUT', RETENTION, { days });
      // The change is itself an entry.
      void this.load();
    } catch (error) {
      if (!this.#disposed) this.keep = before;
      lostAccess(error);
      ui.toasts.error(failText(error));
    } finally {
      this.#saving = false;
    }
  }
}

// --- Values of the page -------------------------------------------------------

export interface AuditPill {
  key: AuditFilter;
  label: string;
  /** `null` before the first response. */
  n: number | null;
  on: boolean;
  onClick: () => void;
}

export interface AuditRow {
  id: number;
  icon: IconName;
  who: string;
  act: string;
  /** Name of the category. */
  cat: string;
  meta: string;
  mMeta: string;
  sep: boolean;
  /** Tile in `--danger`. */
  warn: boolean;
}

export interface AuditGroup {
  /** Calendar day as a number, key of the list. */
  day: number;
  label: string;
  rows: AuditRow[];
}

export interface AuditKeepOption {
  days: KeepDays;
  label: string;
  on: boolean;
  /** `aria-disabled` until the first response: no selection, clicks have no effect. */
  dis: boolean;
  onClick: () => void;
}

/** Card "Loading failed" of the list (`aLoad`). */
export interface AuditFailure {
  fail: boolean;
  text: string;
  onRetry: () => void;
}

/** Box "Keep entries" or its card (`aKeepLoad`). */
export interface AuditKeepLoad extends AuditFailure {
  /** Box visible: loaded or until the first response. */
  box: boolean;
  /** `aria-busy` of the box until the first response. */
  busy: boolean;
}

/** Button "Load more entries" (`aMore`). */
export interface AuditMore {
  show: boolean;
  /** Loading more is running: ti-loader-2 and "Loading …", `aria-busy`. */
  busy: boolean;
  label: string;
  onClick: () => void;
}

export interface AuditValues {
  aQ: string;
  aHasQ: boolean;
  onAQ: (value: string) => void;
  clearAQ: () => void;
  aExport: () => void;
  aPills: AuditPill[];
  aGroups: AuditGroup[];
  aEmpty: boolean;
  aKeepOpts: AuditKeepOption[];
  aKeepSum: string;
  /** Until the first response (also after "Try again"): empty frame instead of the days. */
  aFirst: boolean;
  /** `aria-busy` on the container of the days while the previous list stays after a change. */
  aBusyAttr: boolean;
  aLoad: AuditFailure;
  aMore: AuditMore;
  aKeepLoad: AuditKeepLoad;
}

/** Name of a category; an unknown one (newer server) shows its key. */
function catLabel(cat: string): string {
  return FILTERS.find(([key]) => key === cat)?.[1]() ?? cat;
}

/**
 * The days of the list (`aGroups`). The server sorts by `id`; if the server clock jumps back, a day must still
 * appear only once.
 */
export function auditGroups(entries: readonly AuditEntry[], time: AuditTime): AuditGroup[] {
  const groups: AuditGroup[] = [];
  for (const e of entries) {
    const at = Date.parse(e.at);
    const day = dayNumber(at, time);
    let group = groups.find((g) => g.day === day);
    if (!group) {
      group = { day, label: dayLabel(at, time), rows: [] };
      groups.push(group);
    }
    const cat = catLabel(e.cat);
    const hour = clock(at, time);
    group.rows.push({
      id: e.id,
      icon: isIconName(e.icon) ? e.icon : 'ti-history',
      who: e.who,
      act: e.act,
      cat,
      meta: e.meta ? `${hour} · ${e.meta}` : hour,
      mMeta: e.meta ? `${hour} · ${cat} · ${e.meta}` : `${hour} · ${cat}`,
      sep: group.rows.length > 0,
      warn: e.warn,
    });
  }
  return groups;
}

export function auditValues(p: AuditPage, time: AuditTime): AuditValues {
  const keep = p.keep;
  const fail = p.error !== null;
  const first = p.counts === null && !fail;
  // Before the first response and after a failure: no counts, no days.
  const off = first || fail;
  const keepFail = p.keepError !== null;
  const more = p.loadingMore;
  return {
    aQ: p.q,
    aHasQ: !!p.q,
    onAQ: (value) => p.setQuery(value),
    clearAQ: () => p.clearQuery(),
    aExport: () => void p.exportCsv(time.tz),
    aPills: FILTERS.map(([key, label]) => ({
      key,
      label: label(),
      n: off ? null : (p.counts?.[key] ?? null),
      on: key === p.filter,
      onClick: () => p.setFilter(key),
    })),
    aGroups: off ? [] : auditGroups(p.entries, time),
    aEmpty: !off && p.entries.length === 0,
    aKeepOpts: KEEP.map(([days, label]) => ({
      days,
      label: label(),
      on: days === keep,
      dis: keep === null,
      onClick: () => void p.setKeep(days),
    })),
    aKeepSum: keep === null ? '' : m.audit_keep_sum({ period: KEEP_PERIOD[keep]() }),
    aFirst: first,
    aBusyAttr: !off && p.loading,
    aLoad: { fail, text: p.error ?? '', onRetry: () => void p.retry() },
    aMore: {
      show: !off && p.next !== null,
      busy: more,
      label: more ? m.audit_more_busy() : m.audit_more(),
      onClick: () => void p.more(),
    },
    aKeepLoad: {
      box: !keepFail,
      fail: keepFail,
      busy: keep === null && !keepFail,
      text: p.keepError ?? '',
      onRetry: () => void p.loadKeep(),
    },
  };
}
