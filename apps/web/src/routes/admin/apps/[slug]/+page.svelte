<!--
  Administration › Apps › an app: status (health, addresses, version, event
  delivery), "New pairing code" (shown once), and for a paired app the access
  default for new users, the roles of its manifest and who has access; at the
  end "Remove app" with confirmation. "Apps" leads back to the list. Flow in
  `#lib/admin-apps.svelte.ts` (`AppPage`).

  Built after the sections of Administration › Organization.
-->
<script lang="ts">
  import { untrack } from 'svelte';
  import { goto } from '$app/navigation';
  import { resolve } from '$app/paths';
  import { page } from '$app/state';
  import { radiogroup } from '#lib/actions/radiogroup.ts';
  import { AppPage } from '#lib/admin-apps.svelte.ts';
  import {
    appName,
    appStatus,
    defaultRole,
    eventsProblem,
    roleName,
    textIn,
    type AccessDefault,
    type AdminApp,
  } from '#lib/apps.ts';
  import AppIcon from '#lib/components/AppIcon.svelte';
  import Icon from '#lib/components/Icon.svelte';
  import LoadCard from '#lib/components/LoadCard.svelte';
  import PairingCode from '#lib/components/PairingCode.svelte';
  import Skeleton from '#lib/components/Skeleton.svelte';
  import type { IconName } from '#lib/icons.ts';
  import { intlLocale } from '#lib/locale.ts';
  import { m } from '#lib/paraglide/messages.js';
  import { ui } from '#lib/state/ui.svelte.ts';

  const LIST = resolve('/admin/apps');
  const ctx = { leave: () => void goto(LIST) };
  const slug = $derived(page.params.slug ?? '');
  let p = $state.raw<AppPage | null>(null);
  const isMobile = $derived(ui.isMobile);
  let now = $state(Date.now());

  // One state per app: another slug (history) starts afresh.
  $effect(() => {
    const ap = new AppPage(slug, ctx);
    p = ap;
    untrack(() => ap.start());
    return () => ap.dispose();
  });

  $effect(() => {
    const clock = setInterval(() => (now = Date.now()), 30_000);
    return () => clearInterval(clock);
  });

  const day = (iso: string | null) =>
    iso ? new Date(iso).toLocaleDateString(intlLocale(), { day: 'numeric', month: 'long', year: 'numeric' }) : '';

  /** Rows of "Status". */
  function facts(app: AdminApp): { label: string; value: string; href?: string; mono?: boolean }[] {
    const out: { label: string; value: string; href?: string; mono?: boolean }[] = [];
    out.push(
      app.state === 'paired' && app.url
        ? { label: m.apps_fact_address(), value: app.url, href: app.url }
        : { label: m.apps_fact_address(), value: `/${app.slug}/` },
    );
    out.push({ label: m.apps_fact_internal(), value: app.internal_url, mono: true });
    if (app.state === 'paired') {
      out.push({ label: m.apps_fact_version(), value: app.version ?? '' });
      out.push({ label: m.apps_fact_id(), value: app.manifest_id ?? '', mono: true });
      out.push({ label: m.apps_fact_paired(), value: day(app.paired_at) });
    } else out.push({ label: m.apps_fact_added(), value: day(app.created_at) });
    return out;
  }

  const defaults: { v: AccessDefault; icon: IconName; label: () => string }[] = [
    { v: 'all', icon: 'ti-users', label: m.apps_default_all },
    { v: 'selected', icon: 'ti-user-check', label: m.apps_default_selected },
  ];
  function defaultDesc(app: AdminApp, v: AccessDefault): string {
    if (v === 'selected') return m.apps_default_selected_desc();
    const role = defaultRole(app);
    return m.apps_default_all_desc({ role: role ? textIn(role.name, role.id) : '' });
  }

  const uid = $props.id();
  /** Users and invitations with access, and those without. */
  const withAccess = $derived(p?.users.filter((u) => u.app_role !== null) ?? []);
  const without = $derived(p?.users.filter((u) => u.app_role === null) ?? []);

  const statusLabel = (s: string) =>
    s === 'invited' ? m.users_status_invited() : s === 'locked' ? m.users_status_locked() : '';
</script>

<div class="page" class:mobile={isMobile}>
  <button type="button" class="back" onclick={() => void goto(LIST)}>
    <Icon name="ti-arrow-left" />{m.apps_back()}
  </button>

  {#if !p || p.status === 'loading'}
    {#if p?.loading.slow}<Skeleton set="ueb" {isMobile} />{/if}
  {:else if p.status === 'gone'}
    <h1 class="sr-only">{m.apps_gone_title()}</h1>
    <div class="space"><LoadCard icon="ti-map-question" title={m.apps_gone_title()} text={m.apps_gone_text()} /></div>
  {:else if p.status === 'failed' || !p.app}
    <h1 class="sr-only">{m.apps_app_failed()}</h1>
    <div class="space">
      <LoadCard title={m.apps_app_failed()} text={p.failure} onRetry={() => void p?.retry()} />
    </div>
  {:else}
    {@const app = p.app}
    {@const st = appStatus(app, now)}
    {@const problem = app.state === 'paired' ? eventsProblem(app, now) : ''}
    {@const paired = app.state === 'paired'}
    <div class="head">
      <AppIcon src={app.icon} size={isMobile ? 52 : 60} />
      <div class="head-text">
        <h1 class={isMobile ? 'm-title' : 'title'}>{appName(app)}</h1>
        <p class="lead">{paired ? textIn(app.description) : m.apps_pending_lead()}</p>
      </div>
    </div>

    {#if p.pairing}
      <PairingCode
        name={appName(app)}
        pairing={p.pairing}
        onCopy={() => p?.copyCode()}
        onClose={() => p?.closeCode()}
        {isMobile}
      />
    {/if}

    <section class="sec">
      <h2>{m.apps_sec_status()}</h2>
      <div class="box">
        <div class="row">
          <div class="row-text">
            <div class="label">{m.apps_fact_status()}</div>
            {#if st.detail}<div class="desc">{st.detail}</div>{/if}
          </div>
          <span class="st st-{st.kind}"><span class="st-dot"></span>{st.label}</span>
        </div>
        {#if problem}
          <div class="sep"></div>
          <div class="row">
            <div role="note" class="problem"><Icon name="ti-alert-triangle" />{problem}</div>
          </div>
        {/if}
        {#each facts(app) as f (f.label)}
          <div class="sep"></div>
          <div class="row">
            <div class="label">{f.label}</div>
            {#if f.href}
              <!-- eslint-disable-next-line svelte/no-navigation-without-resolve -- the app's own path -->
              <a class="value link" href={f.href}>{f.value}<Icon name="ti-external-link" /></a>
            {:else}
              <span class="value" class:mono={f.mono}>{f.value}</span>
            {/if}
          </div>
        {/each}
        <div class="sep"></div>
        <div class="row">
          <div class="row-text">
            <div class="label">{m.apps_pairing()}</div>
            <div class="desc">{paired ? m.apps_pairing_paired_desc() : m.apps_pairing_pending_desc()}</div>
          </div>
          <button type="button" class="act" aria-disabled={p.busy} onclick={() => void p?.newCode()}>
            <Icon name="ti-key" />{m.apps_new_code()}
          </button>
        </div>
      </div>
    </section>

    {#if paired}
      <section class="sec">
        <h2>{m.apps_sec_access()}</h2>
        <p class="sec-desc">{m.apps_sec_access_desc()}</p>
        <div class="box pad">
          <div class="field-label" id="{uid}-default">{m.apps_default_label()}</div>
          <div role="radiogroup" use:radiogroup aria-labelledby="{uid}-default" class="cards">
            {#each defaults as d (d.v)}
              <button
                type="button"
                role="radio"
                class="card"
                class:on={app.access_default === d.v}
                aria-checked={app.access_default === d.v}
                onclick={() => p?.setDefault(d.v)}
              >
                <span class="card-tile"><Icon name={d.icon} /></span>
                <span class="card-text">
                  <span class="card-label">{d.label()}</span>
                  <span class="card-desc">{defaultDesc(app, d.v)}</span>
                </span>
              </button>
            {/each}
          </div>
          <p class="note"><Icon name="ti-info-circle" />{m.apps_default_note()}</p>
        </div>

        <h3>{m.apps_roles()}</h3>
        <ul class="box roles">
          {#each app.roles ?? [] as r, i (r.id)}
            <li>
              {#if i > 0}<div class="sep"></div>{/if}
              <div class="row">
                <div class="row-text">
                  <div class="label">
                    {textIn(r.name, r.id)}
                    {#if r.default}<span class="badge">{m.apps_role_default()}</span>{/if}
                    {#if r.admin}<span class="badge">{m.apps_role_admin()}</span>{/if}
                  </div>
                  <div class="desc">{textIn(r.description)}</div>
                </div>
              </div>
            </li>
          {/each}
        </ul>
      </section>

      <section class="sec">
        <h2>{m.apps_sec_users()}</h2>
        <p class="sec-desc">
          {p.usersStatus === 'ready'
            ? m.apps_users_count({ n: withAccess.length, total: p.users.length })
            : m.apps_sec_users_desc()}
        </p>
        {#if p.usersStatus === 'failed'}
          <div class="box pad">
            <p class="note">
              <Icon name="ti-alert-circle" />{m.apps_users_failed()}
              <button type="button" class="inline" onclick={() => void p?.loadUsers()}>
                {m.users_details_retry()}
              </button>
            </p>
          </div>
        {:else if p.usersStatus === 'ready'}
          {#if withAccess.length}
            <ul class="box people">
              {#each withAccess as u, i (u.id)}
                <li>
                  {#if i > 0}<div class="sep"></div>{/if}
                  <div class="row">
                    <div class="row-text">
                      <div class="label">
                        {u.name || u.email}
                        {#if statusLabel(u.status)}<span class="badge">{statusLabel(u.status)}</span>{/if}
                      </div>
                      {#if u.name}<div class="desc">{u.email}</div>{/if}
                    </div>
                    <span class="value">{roleName(app, u.app_role ?? '')}</span>
                  </div>
                </li>
              {/each}
            </ul>
          {/if}
          {#if without.length}
            <details class="without">
              <summary>{m.apps_without({ n: without.length })}</summary>
              <ul class="plain">
                {#each without as u (u.id)}
                  <li>
                    {u.name || u.email}{#if statusLabel(u.status)}<span class="badge">
                        {statusLabel(u.status)}
                      </span>{/if}
                  </li>
                {/each}
              </ul>
            </details>
          {/if}
        {/if}
        <button type="button" class="act manage" onclick={() => void goto(resolve('/admin/users'))}>
          <Icon name="ti-users" />{m.apps_manage_access()}
        </button>
      </section>
    {/if}

    <section class="sec">
      <h2>{m.apps_sec_remove()}</h2>
      <div class="box">
        <div class="row">
          <div class="row-text">
            <div class="label">{m.apps_remove()}</div>
            <div class="desc">{m.apps_remove_desc()}</div>
          </div>
          <button type="button" class="act danger" onclick={() => p?.remove()}>
            <Icon name="ti-trash" />{m.apps_remove()}
          </button>
        </div>
      </div>
    </section>
  {/if}
</div>

<style>
  .back {
    height: 36px;
    display: inline-flex;
    align-items: center;
    gap: 8px;
    margin: -8px 0 18px -12px;
    padding: 0 14px 0 10px;
    border-radius: 9999px;
    font-size: 14px;
    font-weight: 700;
    color: var(--mfg);
    transition:
      background-color var(--dur, 220ms),
      color var(--dur, 220ms);
  }

  .back:hover {
    background: var(--hov);
    color: var(--fg);
  }

  .mobile .back {
    height: 44px;
    margin: -10px 0 10px -10px;
  }

  .space {
    margin-top: 12px;
  }

  .head {
    display: flex;
    align-items: center;
    gap: 18px;
  }

  .mobile .head {
    gap: 14px;
  }

  .head-text {
    min-width: 0;
  }

  .title {
    margin: 0;
    font-family: var(--font-serif);
    font-weight: 400;
    font-size: 40px;
    line-height: 1.1;
  }

  .m-title {
    margin: 0;
    font-family: var(--font-serif);
    font-weight: 400;
    font-size: 30px;
    line-height: 1.1;
  }

  .lead {
    margin: 8px 0 0;
    color: var(--mfg);
    font-size: 16px;
  }

  .mobile .lead {
    font-size: 15px;
  }

  /* Sections as on Administration › Organization */
  .sec {
    margin-top: 44px;
  }

  .mobile .sec {
    margin-top: 34px;
  }

  h2 {
    margin: 0;
    font-family: var(--font-serif);
    font-weight: 400;
    font-size: 24px;
  }

  .mobile h2 {
    font-size: 22px;
  }

  h3 {
    margin: 28px 0 0;
    font-size: 16px;
    font-weight: 800;
  }

  .sec-desc {
    margin: 6px 0 0;
    font-size: 14.5px;
    color: var(--mfg);
  }

  .box {
    margin: 18px 0 0;
    padding: 6px;
    list-style: none;
    border-radius: 26px;
    background: var(--surface2);
  }

  .box.pad {
    padding: 18px;
  }

  .mobile .box {
    margin-top: 16px;
    padding: 4px;
    border-radius: 24px;
  }

  .mobile .box.pad {
    padding: 14px;
  }

  .sep {
    height: 1px;
    background: var(--bd);
    margin: 0 18px;
  }

  .row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 10px 24px;
    padding: 14px 18px;
    flex-wrap: wrap;
  }

  .mobile .row {
    padding: 12px 14px;
  }

  .row-text {
    flex: 1 1 220px;
    min-width: 0;
  }

  .label {
    font-size: 15px;
    font-weight: 700;
  }

  .desc {
    margin-top: 3px;
    font-size: 14px;
    color: var(--mfg);
  }

  .value {
    min-width: 0;
    font-size: 14px;
    font-weight: 600;
    overflow-wrap: anywhere;
  }

  .mono {
    font-family: ui-monospace, 'SFMono-Regular', Menlo, Consolas, monospace;
    font-size: 13.5px;
  }

  .link {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    color: var(--deep);
  }

  .st {
    display: inline-flex;
    align-items: center;
    gap: 7px;
    font-size: 14px;
    font-weight: 700;
  }

  .st-dot {
    width: 9px;
    height: 9px;
    border-radius: 9999px;
    background: var(--mfg);
  }

  /* fixed value as for "Active" users */
  .st-online .st-dot {
    background: #94c48d;
  }

  .st-offline {
    color: var(--danger);
  }

  .st-offline .st-dot {
    background: var(--danger);
  }

  /* waiting: a ring, as for invitations */
  .st-pending .st-dot,
  .st-unknown .st-dot {
    background: transparent;
    box-shadow: inset 0 0 0 2px var(--acc);
  }

  .st-unknown .st-dot {
    box-shadow: inset 0 0 0 2px var(--mfg);
  }

  .problem {
    display: flex;
    align-items: flex-start;
    gap: 10px;
    font-size: 14px;
    font-weight: 600;
    line-height: 1.45;
    color: var(--danger);
  }

  .problem :global(svg) {
    flex-shrink: 0;
    margin-top: 1px;
    font-size: 18px;
  }

  .act {
    height: 40px;
    display: inline-flex;
    align-items: center;
    gap: 8px;
    padding: 0 18px 0 14px;
    border-radius: 9999px;
    background: var(--surface);
    box-shadow: var(--sh1);
    font-size: 14px;
    font-weight: 700;
    white-space: nowrap;
    transition: background-color var(--dur, 220ms);
  }

  .act:hover {
    background: var(--hov);
  }

  .act[aria-disabled='true'] {
    cursor: progress;
    opacity: 0.6;
  }

  .act.danger {
    color: var(--danger);
  }

  .act.manage {
    margin-top: 16px;
  }

  .mobile .act {
    height: 44px;
  }

  .field-label {
    font-size: 14px;
    font-weight: 800;
  }

  .cards {
    margin-top: 10px;
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: 8px;
  }

  .mobile .cards {
    grid-template-columns: 1fr;
  }

  .card {
    display: flex;
    align-items: flex-start;
    gap: 12px;
    padding: 14px;
    border-radius: 20px;
    background: var(--surface);
    box-shadow: var(--sh1);
    text-align: left;
    transition: box-shadow var(--dur, 220ms);
  }

  .card.on {
    box-shadow:
      0 0 0 2px var(--acc),
      var(--sh1);
  }

  .card-tile {
    width: 36px;
    height: 36px;
    flex-shrink: 0;
    display: grid;
    place-items: center;
    border-radius: 12px;
    background: var(--surface2);
    font-size: 18px;
  }

  .card.on .card-tile {
    background: var(--acc);
    color: var(--accink);
  }

  .card-text {
    display: flex;
    flex-direction: column;
    gap: 3px;
    min-width: 0;
  }

  .card-label {
    font-size: 14.5px;
    font-weight: 800;
  }

  .card-desc {
    font-size: 13.5px;
    line-height: 1.45;
    color: var(--mfg);
  }

  .note {
    margin: 14px 2px 0;
    display: flex;
    align-items: flex-start;
    gap: 8px;
    font-size: 13.5px;
    line-height: 1.45;
    color: var(--fg);
  }

  .box.pad > .note:first-child {
    margin-top: 0;
  }

  .note :global(svg) {
    flex-shrink: 0;
    margin-top: 1px;
    font-size: 17px;
  }

  .inline {
    margin-left: 4px;
    font-weight: 800;
    text-decoration: underline;
  }

  .badge {
    display: inline-block;
    margin-left: 8px;
    padding: 2px 9px;
    border-radius: 9999px;
    background: var(--surface);
    font-size: 12px;
    font-weight: 700;
    color: var(--mfg);
    vertical-align: 1px;
  }

  .without {
    margin-top: 14px;
    padding: 4px 18px;
  }

  .without summary {
    min-height: 40px;
    display: flex;
    align-items: center;
    cursor: pointer;
    font-size: 14.5px;
    font-weight: 700;
  }

  .plain {
    margin: 6px 0 0;
    padding: 0 0 0 4px;
    list-style: none;
    display: flex;
    flex-direction: column;
    gap: 8px;
    font-size: 14px;
  }
</style>
