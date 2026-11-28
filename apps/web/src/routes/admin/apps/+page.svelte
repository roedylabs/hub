<!--
  Administration › Apps: the apps in their order on the home page with icon,
  name, address, status and number of users; each row opens the app's page.
  "Move up" and "Move down" change the order (buttons: keyboard and screen
  reader; the new position is announced). "Add app" with slug and internal
  address; afterwards the first pairing code is shown once with "Copy".
  Flow in `#lib/admin-apps.svelte.ts`, status texts in `#lib/apps.ts`.

  Built after Administration › Users (title, bar, form, rows).
-->
<script lang="ts">
  import { onMount, tick } from 'svelte';
  import { goto } from '$app/navigation';
  import { resolve } from '$app/paths';
  import { focusback } from '#lib/actions/focusback.ts';
  import { FirstLoad, Icon, LoadCard, tooltip, ui } from '@roedy/ui';
  import { AppsPage, slugErrorText, urlErrorText } from '#lib/admin-apps.svelte.ts';
  import { appName, appStatus, eventsProblem } from '#lib/apps.ts';
  import AppIcon from '#lib/components/AppIcon.svelte';
  import PairingCode from '#lib/components/PairingCode.svelte';
  import { m } from '#lib/paraglide/messages.js';
  import { pageOf } from '#lib/settings-nav.ts';

  const p = new AppsPage();
  const isMobile = $derived(ui.isMobile);
  const info = $derived(pageOf('/admin/apps'));
  const uid = $props.id();
  /** Clock for the status details (pairing code validity, last contact). */
  let now = $state(Date.now());
  let slugInput: HTMLInputElement | undefined = $state();
  let list: HTMLElement | undefined = $state();

  const usersOf = (n: number) => (n === 1 ? m.apps_users_one() : m.apps_users_many({ n }));
  const rows = $derived(
    p.rows.map((a, i) => ({
      slug: a.slug,
      name: appName(a),
      icon: a.icon,
      status: appStatus(a, now),
      problem: a.state === 'paired' ? eventsProblem(a, now) : '',
      sub: a.state === 'paired' ? m.apps_row_sub({ slug: a.slug, users: usersOf(a.user_count) }) : `/${a.slug}/`,
      first: i === 0,
      last: i === p.rows.length - 1,
    })),
  );
  const form = $derived(p.add);

  onMount(() => {
    p.start();
    const clock = setInterval(() => (now = Date.now()), 30_000);
    return () => {
      clearInterval(clock);
      p.dispose();
    };
  });

  // Opening "Add app" focuses the slug.
  const addOpen = $derived(!!p.add && p.status === 'ready');
  $effect(() => {
    if (!addOpen) return;
    const timer = setTimeout(() => slugInput?.focus(), 40);
    return () => clearTimeout(timer);
  });

  function open(slug: string) {
    void goto(resolve('/admin/apps/[slug]', { slug }));
  }

  /** Moves the app and keeps the focus on the pressed button (or the other one at the end of the list). */
  async function move(slug: string, delta: -1 | 1) {
    p.move(slug, delta);
    await tick();
    const dir = delta < 0 ? 'up' : 'down';
    const other = delta < 0 ? 'down' : 'up';
    const button =
      list?.querySelector<HTMLButtonElement>(`[data-move="${slug}:${dir}"]:not([aria-disabled="true"])`) ??
      list?.querySelector<HTMLButtonElement>(`[data-move="${slug}:${other}"]`);
    button?.focus();
  }

  function onKey(event: KeyboardEvent) {
    if (event.key === 'Enter') {
      event.preventDefault();
      void p.submitAdd();
    } else if (event.key === 'Escape') {
      event.preventDefault();
      p.closeAdd();
    }
  }
</script>

<div class="page" class:mobile={isMobile}>
  {#if !isMobile}
    <h1 class="title">{info?.label}</h1>
    <p class="lead">{info?.desc}</p>
  {:else}
    <h1 class="m-title">{info?.label}</h1>
    <p class="m-lead">{info?.desc}</p>
  {/if}

  <FirstLoad
    status={p.status}
    slow={p.loading.slow}
    title={m.apps_load_failed()}
    text={p.failure}
    onRetry={() => void p.retry()}
    {isMobile}
  >
    {#if !form}
      <div class="bar">
        <button type="button" class="add-btn" onclick={() => p.openAdd()}>
          <Icon name="ti-plus" />{m.apps_add()}
        </button>
      </div>
    {:else}
      <!-- Closing it (Escape, "Cancel", added) removes the focused control: focus goes back to "Add app". -->
      <form
        class="add"
        use:focusback={'.add-btn'}
        aria-labelledby="{uid}-add"
        onsubmit={(e) => {
          e.preventDefault();
          void p.submitAdd();
        }}
        novalidate
      >
        <h2 class="add-title" id="{uid}-add">{m.apps_add_title()}</h2>
        <p class="add-hint">{m.apps_add_hint()}</p>

        <label class="field-label" for="{uid}-slug">{m.apps_slug()}</label>
        <div class="slug" class:bad={!!form.slugErr}>
          <span class="slug-pre" aria-hidden="true">/</span>
          <input
            id="{uid}-slug"
            bind:this={slugInput}
            value={form.slug}
            oninput={(e) => p.setSlug(e.currentTarget.value)}
            onkeydown={onKey}
            placeholder={m.apps_slug_placeholder()}
            aria-invalid={!!form.slugErr}
            aria-describedby={form.slugErr ? `${uid}-slug-err` : `${uid}-slug-hint`}
            autocomplete="off"
            autocapitalize="none"
            spellcheck="false"
          />
        </div>
        {#if form.slugErr}
          <div role="alert" class="field-error" id="{uid}-slug-err">
            <Icon name="ti-alert-circle" />{slugErrorText(form.slugErr)}
          </div>
        {:else}
          <div class="field-hint" id="{uid}-slug-hint">{m.apps_slug_hint()}</div>
        {/if}

        <label class="field-label" for="{uid}-url">{m.apps_internal()}</label>
        <input
          id="{uid}-url"
          class="text"
          class:bad={!!form.urlErr}
          value={form.internal}
          oninput={(e) => p.setInternal(e.currentTarget.value)}
          onkeydown={onKey}
          placeholder={m.apps_internal_placeholder()}
          aria-invalid={!!form.urlErr}
          aria-describedby={form.urlErr ? `${uid}-url-err` : `${uid}-url-hint`}
          inputmode="url"
          autocomplete="off"
          autocapitalize="none"
          spellcheck="false"
        />
        {#if form.urlErr}
          <div role="alert" class="field-error" id="{uid}-url-err">
            <Icon name="ti-alert-circle" />{urlErrorText(form.urlErr)}
          </div>
        {:else}
          <div class="field-hint" id="{uid}-url-hint">{m.apps_internal_hint()}</div>
        {/if}

        <div class="actions">
          <button type="button" class="cancel" onclick={() => p.closeAdd()}>{m.apps_add_cancel()}</button>
          <button type="submit" class="submit" aria-disabled={form.busy}>
            {#if form.busy}<Icon name="ti-loader-2" />{:else}<Icon name="ti-plus" />{/if}{m.apps_add_submit()}
          </button>
        </div>
      </form>
    {/if}

    {#if p.shown}
      <PairingCode
        name={p.shown.name}
        pairing={p.shown.pairing}
        onCopy={() => p.copyShown()}
        onClose={() => p.closeShown()}
        {isMobile}
      />
    {/if}

    {#if rows.length === 0}
      <div class="empty">
        <LoadCard icon="ti-apps" title={m.apps_empty_title()} text={m.apps_empty_text()} />
      </div>
    {:else}
      <p class="order-hint">{m.apps_order_hint()}</p>
      <ol class="list" bind:this={list} aria-label={m.apps_list_label()}>
        {#each rows as r (r.slug)}
          <li class="row">
            <button type="button" class="open" onclick={() => open(r.slug)}>
              <AppIcon src={r.icon} size={isMobile ? 40 : 44} />
              <span class="who">
                <span class="name">{r.name}</span>
                {#if isMobile}
                  <!-- On mobile the status takes the line below the name. -->
                  <span class="sub st st-{r.status.kind}"><span class="st-dot"></span>{r.status.label}</span>
                {:else}
                  <span class="sub">{r.sub}</span>
                {/if}
              </span>
              {#if !isMobile}
                <span class="st st-{r.status.kind}"><span class="st-dot"></span>{r.status.label}</span>
              {/if}
              {#if r.problem && !isMobile}
                <span class="warn" use:tooltip={r.problem}>
                  <Icon name="ti-alert-triangle" label={r.problem} />
                </span>
              {/if}
              {#if !isMobile}<span class="chev"><Icon name="ti-chevron-right" /></span>{/if}
            </button>
            <span class="moves">
              <button
                type="button"
                class="move"
                data-move="{r.slug}:up"
                aria-disabled={r.first}
                aria-label={m.apps_move_up({ name: r.name })}
                use:tooltip={isMobile ? undefined : m.apps_move_up_short()}
                onclick={() => !r.first && void move(r.slug, -1)}
              >
                <Icon name="ti-arrow-up" />
              </button>
              <button
                type="button"
                class="move"
                data-move="{r.slug}:down"
                aria-disabled={r.last}
                aria-label={m.apps_move_down({ name: r.name })}
                use:tooltip={isMobile ? undefined : m.apps_move_down_short()}
                onclick={() => !r.last && void move(r.slug, 1)}
              >
                <Icon name="ti-arrow-down" />
              </button>
            </span>
          </li>
        {/each}
      </ol>
    {/if}
    <div class="sr-only" role="status" aria-live="polite">{p.moved}</div>
  </FirstLoad>
</div>

<style>
  .title {
    margin: 0;
    font-family: var(--font-serif);
    font-weight: 400;
    font-size: 40px;
    line-height: 1.1;
  }

  .lead {
    margin: 10px 0 0;
    color: var(--mfg);
    font-size: 16px;
  }

  .m-title {
    margin: 0;
    font-family: var(--font-serif);
    font-weight: 400;
    font-size: 32px;
    line-height: 1.1;
  }

  .m-lead {
    margin: 8px 0 0;
    font-size: 15px;
    line-height: 1.5;
    color: var(--mfg);
  }

  .bar {
    margin-top: 26px;
    display: flex;
    justify-content: flex-end;
  }

  .mobile .bar {
    margin-top: 20px;
  }

  .add-btn {
    height: 44px;
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 0 20px 0 16px;
    border-radius: 9999px;
    background: var(--ink);
    color: var(--inkfg);
    font-size: 14.5px;
    font-weight: 800;
    white-space: nowrap;
    box-shadow: 0 10px 24px -10px rgba(43, 34, 28, 0.5);
    transition: transform var(--dur, 220ms) cubic-bezier(0.3, 1.5, 0.5, 1);
  }

  .mobile .add-btn {
    flex: 1;
    justify-content: center;
    height: 48px;
  }

  .add-btn :global(svg) {
    font-size: 18px;
  }

  .add-btn:hover {
    transform: translateY(-2px);
  }

  /* Form "Add app" */
  .add {
    margin-top: 26px;
    padding: 22px;
    border-radius: 26px;
    background: var(--accsoft);
    animation: snDlg 300ms cubic-bezier(0.2, 1.2, 0.3, 1);
  }

  .mobile .add {
    margin-top: 20px;
    padding: 16px 14px 14px;
    border-radius: 24px;
  }

  .add-title {
    margin: 0;
    font-family: var(--font-serif);
    font-weight: 400;
    font-size: 20px;
  }

  .add-hint {
    margin: 4px 0 0;
    font-size: 14px;
    line-height: 1.5;
  }

  .field-label {
    display: block;
    margin-top: 18px;
    font-size: 14px;
    font-weight: 800;
  }

  .slug,
  .text {
    margin-top: 8px;
    width: 100%;
    height: 50px;
    border-radius: 25px;
    background: var(--surface);
    box-shadow: var(--sh1);
    transition: box-shadow var(--dur, 220ms);
  }

  .slug {
    display: flex;
    align-items: center;
    padding-left: 20px;
  }

  .slug-pre {
    color: var(--mfg);
    font-weight: 700;
  }

  .slug input,
  .text {
    border: 0;
    outline: none;
    font-size: 14px;
    font-weight: 600;
  }

  .slug input {
    flex: 1;
    min-width: 0;
    height: 100%;
    padding: 0 20px 0 2px;
    background: none;
  }

  .text {
    padding: 0 20px;
  }

  .mobile .slug input,
  .mobile .text {
    font-size: 16px;
  }

  .slug:focus-within,
  .text:focus {
    box-shadow:
      0 0 0 3px var(--acc),
      var(--sh1);
  }

  .slug.bad,
  .text.bad {
    box-shadow: 0 0 0 2px var(--danger);
  }

  .field-hint {
    margin: 8px 4px 0;
    font-size: 13.5px;
    color: var(--fg);
  }

  .field-error {
    margin: 8px 4px 0;
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 14px;
    font-weight: 600;
    color: var(--danger);
  }

  .field-error :global(svg) {
    flex-shrink: 0;
    font-size: 18px;
  }

  .actions {
    margin-top: 18px;
    display: flex;
    justify-content: flex-end;
    gap: 8px;
  }

  .cancel,
  .submit {
    height: 44px;
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 8px;
    padding: 0 20px;
    border-radius: 9999px;
    font-size: 14px;
    font-weight: 800;
    white-space: nowrap;
  }

  .cancel {
    font-weight: 700;
    transition: background-color var(--dur, 220ms);
  }

  .cancel:hover {
    background: var(--hov);
  }

  .submit {
    background: var(--acc);
    color: var(--accink);
  }

  .submit[aria-disabled='true'] {
    cursor: progress;
  }

  .mobile .cancel,
  .mobile .submit {
    flex: 1;
    height: 48px;
    font-size: 15px;
  }

  .empty {
    margin-top: 24px;
  }

  .order-hint {
    margin: 26px 4px 0;
    font-size: 14px;
    color: var(--mfg);
  }

  .mobile .order-hint {
    margin-top: 20px;
  }

  /* Rows */
  .list {
    margin: 12px 0 0;
    padding: 6px;
    list-style: none;
    border-radius: 26px;
    background: var(--surface2);
  }

  .mobile .list {
    padding: 4px;
    border-radius: 24px;
  }

  .row {
    display: flex;
    align-items: center;
    gap: 4px;
    border-radius: 20px;
  }

  .row + .row {
    border-top: 1px solid var(--bd);
  }

  .open {
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: center;
    gap: 14px;
    padding: 12px 12px 12px 14px;
    border-radius: 20px;
    text-align: left;
    transition: background-color var(--dur, 220ms);
  }

  .mobile .open {
    gap: 12px;
    padding: 10px 6px 10px 10px;
  }

  .open:hover {
    background: var(--hov);
  }

  .who {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .name {
    font-size: 15.5px;
    font-weight: 800;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .sub {
    font-size: 13.5px;
    color: var(--mfg);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .st {
    flex-shrink: 0;
    display: inline-flex;
    align-items: center;
    gap: 7px;
    font-size: 13.5px;
    font-weight: 700;
    white-space: nowrap;
  }

  .mobile .st {
    font-size: 13px;
  }

  .st-dot {
    width: 8px;
    height: 8px;
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

  .warn {
    display: grid;
    color: var(--danger);
    font-size: 18px;
  }

  .chev {
    display: grid;
    color: var(--mfg);
    font-size: 18px;
  }

  .moves {
    display: flex;
    gap: 2px;
    padding-right: 6px;
  }

  .move {
    width: 36px;
    height: 36px;
    border-radius: 9999px;
    display: grid;
    place-items: center;
    color: var(--fg);
    font-size: 17px;
    transition: background-color var(--dur, 220ms);
  }

  .mobile .move {
    width: 40px;
    height: 44px;
  }

  .move:hover {
    background: var(--hov);
  }

  .move[aria-disabled='true'] {
    opacity: 0.3;
    cursor: default;
  }

  .move[aria-disabled='true']:hover {
    background: none;
  }
</style>
