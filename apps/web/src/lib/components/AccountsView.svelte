<!--
  Account › Accounts: the user's Stalwart accounts and "Add account". One
  component for `/account/accounts` and `/account/accounts/new` (`adding`:
  the form open, `next` where it leads afterwards); flow in
  `#lib/accounts.svelte.ts`. A row opens the account's page. The server
  picker is a popover at the button on the desktop (`ui.menu = 'pick'`), on
  mobile the layout's sheet (`ui.pick`).

  Taken over from Roedy Mail (`apps/web/src/lib/components/MailAccountsView.svelte`)
  without "All accounts", with the account's tile (`AccountTile`), storage in
  the row and the hub's methods: sign-in at the server or an access token.
-->
<script lang="ts">
  import { onMount } from 'svelte';
  import {
    AccountTile,
    accountStateText,
    clockText,
    FirstLoad,
    Icon,
    quotaText,
    radiogroup,
    scrollbar,
    type StalwartAccount,
    tooltip,
    ui,
  } from '@roedy/ui';
  import { AccountsPage, domainOf, serverForEmail, type AccountsNav } from '#lib/accounts.svelte.ts';
  import { m } from '#lib/paraglide/messages.js';
  import { pickPop, selIds, selKey, selectValues, type SelOption } from '#lib/select.ts';
  import { pageOf } from '#lib/settings-nav.ts';
  import { prefs } from '#lib/state/prefs.svelte.ts';
  import { session } from '#lib/state/session.svelte.ts';

  let { adding, next = null, nav }: { adding: boolean; next?: string | null; nav: AccountsNav } = $props();

  // The routes fix `adding`, `next` and `nav`; the page is created anew per route.
  // svelte-ignore state_referenced_locally
  const p = new AccountsPage(nav, adding, next);
  const uid = $props.id();
  const srvIds = selIds(uid, 'srv');
  const errId = `${uid}-err`;
  const isMobile = $derived(ui.isMobile);
  const info = $derived(pageOf('/account/accounts'));
  type FieldEvent = Event & { currentTarget: HTMLInputElement };

  /** Server picker open (`ui.menu === 'pick'`). */
  const open = $derived(ui.menu === 'pick');

  /** Server picked for the address's domain while "Automatic" is chosen. */
  const autoServer = $derived(p.form ? serverForEmail(p.form.email, p.servers) : null);
  const domain = $derived(p.form ? domainOf(p.form.email) : '');

  const opts = $derived.by((): SelOption[] => [
    {
      v: 'auto',
      l: m.acct_auto(),
      tl: autoServer ? m.acct_auto_server({ label: autoServer.label }) : m.acct_auto(),
      sub: m.acct_auto_desc(),
    },
    ...p.servers.map((s) => ({ v: s.id, l: s.label, sub: s.host })),
    ...(p.allowCustom ? [{ v: 'other', l: m.acct_other(), sub: m.acct_other_desc() }] : []),
  ]);

  const sel = $derived(
    selectValues(
      'srv',
      m.acct_server(),
      opts,
      p.form?.srv ?? 'auto',
      (v) => p.pickServer(v),
      !!p.form?.busy,
      { open: open ? 'srv' : null, hi: p.srvHi, isMobile },
      {
        show: (key) => {
          p.srvHi = null;
          if (key) ui.menu = 'pick';
          else if (ui.menu === 'pick') ui.closeMenu();
        },
        mark: (i) => (p.srvHi = i),
      },
    ),
  );

  /** No server to choose at all: the form says so instead of the picker. */
  const noServers = $derived(p.servers.length === 0 && !p.allowCustom);
  const methods = $derived(
    (
      [
        ['signin', m.acct_method_signin()],
        ['token', m.acct_method_token()],
      ] as const
    ).map(([value, label]) => ({
      value,
      label,
      on: p.method === value,
      onClick: () => p.update({ method: value }),
    })),
  );
  const form = $derived(p.form);
  const err = $derived(form?.error ?? null);
  const locked = $derived(p.lockLeft > 0);
  const btnLabel = $derived(
    locked
      ? m.acct_btn_locked({ clock: clockText(p.lockLeft) })
      : form?.busy === 'open'
        ? m.acct_btn_opening()
        : form?.busy === 'token'
          ? m.acct_btn_connecting()
          : p.method === 'signin'
            ? m.acct_btn_continue()
            : m.acct_btn_connect(),
  );
  const ready = $derived(!!form && !form.busy && !locked && !!form.email.trim());

  /** State and storage of a row. */
  function rowOf(a: StalwartAccount) {
    const host = a.server ? `${a.server.label} · ${hostOfUrl(a.url)}` : hostOfUrl(a.url);
    return {
      sub: host,
      st: accountStateText(a.state),
      bad: a.state !== 'ok',
      storage: a.quota ? (quotaText(a.quota, prefs.lang) ?? m.acct_no_limit()) : m.acct_not_checked(),
    };
  }

  function hostOfUrl(url: string): string {
    try {
      return new URL(url).host;
    } catch {
      return url;
    }
  }

  function key(event: KeyboardEvent) {
    if (event.key === 'Enter') {
      event.preventDefault();
      void p.submit();
    } else if (event.key === 'Escape' && !ui.menu) {
      event.preventDefault();
      p.cancel();
    }
  }

  function errAct() {
    if (!err) return;
    if (err.act === 'open' && err.account) nav.account(err.account);
    else if (err.act === 'servers') nav.servers();
  }

  const errActLabel = $derived(
    err?.act === 'open' && err.account
      ? m.acct_open_account()
      : err?.act === 'servers' && session.isAdmin
        ? m.acct_check_servers()
        : null,
  );

  onMount(() => {
    void p.load();
    ui.pick = () => sel;
    return () => {
      if (ui.menu === 'pick') ui.closeMenu();
      ui.pick = null;
      p.dispose();
    };
  });
</script>

{#snippet serverSelect()}
  {#if isMobile}
    <button
      type="button"
      class="sel-btn"
      class:dis={sel.disabled}
      onclick={sel.toggle}
      aria-haspopup="dialog"
      aria-disabled={sel.disabled}
      aria-describedby={err?.field === 'server' ? errId : undefined}
      aria-labelledby={srvIds.labelledby}
    >
      <Icon name="ti-server-2" /><span class="sel-label" id={srvIds.value}>{sel.label}</span><span class="chev">
        <Icon name="ti-chevron-down" />
      </span>
    </button>
  {:else}
    <div class="sel" class:dis={sel.disabled} data-pop>
      <button
        type="button"
        class="sel-btn"
        class:open={sel.open}
        class:bad={err?.field === 'server'}
        onclick={sel.toggle}
        onkeydown={selKey(sel.onKey)}
        onkeyup={sel.onKeyUp}
        role="combobox"
        aria-expanded={sel.open}
        aria-controls={sel.open ? srvIds.list : undefined}
        aria-activedescendant={sel.open && sel.opts[sel.active] ? srvIds.opt(sel.active) : undefined}
        aria-disabled={sel.disabled}
        aria-invalid={err?.field === 'server' ? 'true' : undefined}
        aria-describedby={err?.field === 'server' ? errId : undefined}
        aria-labelledby={srvIds.labelledby}
      >
        <Icon name="ti-server-2" /><span class="sel-label" id={srvIds.value}>{sel.label}</span><span class="chev">
          <Icon name="ti-chevron-down" />
        </span>
      </button>
      {#if sel.open}
        <div
          role="listbox"
          id={srvIds.list}
          aria-label={sel.title}
          class="pick-pop no-scrollbar"
          use:pickPop
          {@attach scrollbar}
        >
          {#each sel.opts as o, i (i)}
            <button
              type="button"
              role="option"
              id={srvIds.opt(i)}
              aria-selected={o.on}
              tabindex="-1"
              class="pick-opt"
              class:hi={o.hi}
              class:on={o.on}
              onclick={o.onClick}
              onmousemove={o.hover}
            >
              <span class="pick-text">
                <span class="pick-label" style:font-weight={o.fw}>{o.label}</span>
                {#if o.hasSub}<span class="pick-sub">{o.sub}</span>{/if}
              </span>
              <span class="pick-check"><Icon name="ti-check" /></span>
            </button>
          {/each}
        </div>
      {/if}
    </div>
  {/if}
{/snippet}

<div class="page" class:mobile={isMobile}>
  {#if !isMobile}
    <h1 class="title">{info?.label}</h1>
    <p class="lead">{m.acct_lead()}</p>
  {:else}
    <h1 class="m-title">{info?.label}</h1>
    <p class="m-lead">{m.acct_lead()}</p>
  {/if}

  <FirstLoad
    status={p.status}
    slow={p.loading.slow}
    title={m.acct_load_failed()}
    text={p.failure}
    onRetry={() => void p.retry()}
    {isMobile}
  >
    <section class="sec" aria-label={info?.label}>
      <div class="box">
        {#if p.accounts.length > 0}
          <ul class="list">
            {#each p.accounts as a (a.id)}
              {@const r = rowOf(a)}
              <li class="acct">
                <button type="button" class="acct-btn" onclick={() => nav.account(a.id)}>
                  <AccountTile email={a.email} color={a.color} size={isMobile ? 42 : 44} />
                  <span class="acct-text">
                    <span class="acct-head">
                      <span class="acct-email">{a.email}</span>{#if a.default}<span
                          class="badge"
                          use:tooltip={isMobile ? undefined : m.acct_default_tip()}
                        >
                          {m.acct_default()}
                        </span>{/if}
                    </span>
                    {#if isMobile}
                      <span class="st" class:bad={r.bad}>
                        <span class="dot"></span><span class="st-text">{r.st}</span>
                      </span>
                    {:else}
                      <span class="acct-sub">{r.sub}</span>
                    {/if}
                  </span>
                  {#if !isMobile}
                    <span class="acct-st">
                      <span class="st" class:bad={r.bad}>
                        <span class="dot"></span><span class="st-text">{r.st}</span>
                      </span>
                      <span class="last">{r.storage}</span>
                    </span>
                  {/if}
                  <span class="go-icon"><Icon name="ti-chevron-right" /></span>
                </button>
              </li>
            {/each}
          </ul>
        {:else if !form}
          <div class="empty">{m.acct_empty()}</div>
        {/if}

        {#if form}
          <div class="form" role="group" aria-labelledby="{uid}-ft">
            <h2 class="f-title" id="{uid}-ft">{m.acct_new()}</h2>
            <p class="f-intro">{m.acct_new_intro()}</p>
            <div class="fields">
              <label class="field">
                <span class="f-label">{m.acct_email()}</span><input
                  class="inp"
                  class:bad={err?.field === 'email'}
                  type="email"
                  value={form.email}
                  readonly={!!form.busy}
                  oninput={(e: FieldEvent) => p.update({ email: e.currentTarget.value })}
                  onkeydown={key}
                  placeholder={m.acct_email_placeholder()}
                  aria-invalid={err?.field === 'email' ? 'true' : undefined}
                  aria-describedby={err?.field === 'email' ? errId : undefined}
                  autocomplete="email"
                  autocapitalize="off"
                  spellcheck="false"
                />
              </label>
              <div class="field">
                <span class="f-label" id={srvIds.label}>{m.acct_server()}</span>
                {#if noServers}
                  <p class="none"><Icon name="ti-info-circle" /><span>{m.acct_no_servers()}</span></p>
                  {#if session.isAdmin}
                    <button type="button" class="err-act" onclick={() => nav.servers()}>
                      <Icon name="ti-arrow-right" />{m.acct_check_servers()}
                    </button>
                  {/if}
                {:else}
                  {@render serverSelect()}
                  {#if form.srv === 'auto' && autoServer}
                    <span class="hint-ok">
                      <Icon name="ti-circle-check" />{m.acct_server_host({ host: autoServer.host })}
                    </span>
                  {:else if form.srv === 'auto' && domain}
                    <span class="hint-warn"><Icon name="ti-info-circle" />{m.acct_auto_none({ domain })}</span>
                  {/if}
                {/if}
              </div>
              {#if form.srv === 'other'}
                <label class="field">
                  <span class="f-label">{m.acct_url()}</span><input
                    class="inp"
                    class:bad={err?.field === 'url'}
                    type="url"
                    value={form.url}
                    readonly={!!form.busy}
                    oninput={(e: FieldEvent) => p.update({ url: e.currentTarget.value })}
                    onkeydown={key}
                    placeholder={m.acct_url_placeholder()}
                    aria-invalid={err?.field === 'url' ? 'true' : undefined}
                    aria-describedby={err?.field === 'url' ? errId : undefined}
                    autocomplete="off"
                    autocapitalize="off"
                    spellcheck="false"
                  />
                </label>
              {/if}
              {#if p.canSignIn}
                <div class="field">
                  <span class="f-label" id="{uid}-method">{m.acct_method()}</span>
                  <div role="radiogroup" use:radiogroup aria-labelledby="{uid}-method" class="seg">
                    {#each methods as o (o.value)}
                      <button
                        type="button"
                        role="radio"
                        aria-checked={o.on}
                        class:on={o.on}
                        disabled={!!form.busy}
                        onclick={o.onClick}
                      >
                        {o.label}
                      </button>
                    {/each}
                  </div>
                </div>
              {/if}
              {#if p.method === 'token'}
                <label class="field">
                  <span class="f-label">{m.acct_token()}</span>
                  <span class="sec-wrap">
                    <input
                      class="inp sec-inp"
                      class:bad={err?.field === 'token'}
                      type={form.showToken ? 'text' : 'password'}
                      value={form.token}
                      readonly={!!form.busy}
                      oninput={(e: FieldEvent) => p.update({ token: e.currentTarget.value })}
                      onkeydown={key}
                      aria-invalid={err?.field === 'token' ? 'true' : undefined}
                      aria-describedby={err?.field === 'token' ? errId : undefined}
                      autocomplete="off"
                      autocapitalize="off"
                      spellcheck="false"
                    />
                    <button
                      type="button"
                      class="eye"
                      disabled={!!form.busy}
                      onclick={() => p.update({ showToken: !form.showToken })}
                      aria-label={form.showToken ? m.acct_hide() : m.acct_show()}
                      aria-pressed={form.showToken}
                      use:tooltip={isMobile ? undefined : form.showToken ? m.acct_hide() : m.acct_show()}
                    >
                      <Icon name={form.showToken ? 'ti-eye-off' : 'ti-eye'} />
                    </button>
                  </span>
                </label>
              {/if}
            </div>
            {#if !noServers}
              <div class="pw-hint">
                <Icon name="ti-info-circle" /><span>
                  {p.method === 'signin'
                    ? m.acct_signin_hint({ host: p.host || m.acct_your_server() })
                    : p.target?.server && !p.target.server.sign_in
                      ? m.acct_token_only({ host: p.host })
                      : m.acct_token_hint()}
                </span>
              </div>
            {/if}

            {#if form.busy}
              <div role="status" class="busy">
                <span class="busy-tile"><span class="spin"><Icon name="ti-loader-2" /></span></span>{form.busy ===
                'open'
                  ? m.acct_busy_open({ host: p.host })
                  : m.acct_busy_token()}
              </div>
            {/if}

            {#if err}
              <div role="alert" class="err" id={errId}>
                <span class="err-tile"><Icon name={err.icon} /></span>
                <div class="err-body">
                  <div class="err-title">{err.title}</div>
                  <div class="err-text">{err.text}</div>
                  {#if err.code}<div class="err-code">{m.acct_server_code({ code: err.code })}</div>{/if}
                  {#if errActLabel}
                    <button type="button" class="err-act" onclick={errAct}>
                      <Icon name="ti-arrow-right" />{errActLabel}
                    </button>
                  {/if}
                </div>
              </div>
            {/if}

            {#snippet submit()}
              <button
                type="button"
                class="submit"
                class:hot={ready || !!form?.busy}
                class:ready
                class:locked
                class:busy={!!form?.busy}
                aria-disabled={!ready}
                aria-busy={!!form?.busy}
                onclick={() => void p.submit()}
              >
                <span class="btn-icon" class:spin={!!form?.busy}>
                  <Icon
                    name={form?.busy ? 'ti-loader-2' : p.method === 'signin' ? 'ti-login-2' : 'ti-plug-connected'}
                  />
                </span>{btnLabel}
              </button>
            {/snippet}
            <div class="acts">
              {#if isMobile}
                {@render submit()}
                <button type="button" class="cancel" onclick={() => p.cancel()}>{m.acct_cancel()}</button>
              {:else}
                <span class="grow"></span>
                <button type="button" class="cancel" onclick={() => p.cancel()}>{m.acct_cancel()}</button>
                {@render submit()}
              {/if}
            </div>
          </div>
        {:else}
          <button type="button" class="add" onclick={() => p.add()}><Icon name="ti-plus" />{m.acct_add()}</button>
        {/if}
      </div>
    </section>
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
    text-wrap: pretty;
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

  .sec {
    margin-top: 28px;
  }

  .mobile .sec {
    margin-top: 20px;
  }

  .box {
    border-radius: 26px;
    background: var(--surface2);
    padding: 6px;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .mobile .box {
    border-radius: 24px;
    padding: 4px;
  }

  .list {
    margin: 0;
    padding: 0;
    list-style: none;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .acct-btn {
    width: 100%;
    display: flex;
    align-items: center;
    gap: 14px;
    padding: 10px 16px 10px 10px;
    border-radius: 20px;
    text-align: left;
    transition: background-color var(--dur, 220ms);
  }

  .acct-btn:hover {
    background: var(--hov);
  }

  .mobile .acct-btn {
    min-height: 64px;
    gap: 12px;
    padding: 8px 12px 8px 8px;
    transition: none;
  }

  .mobile .acct-btn:hover {
    background: none;
  }

  .mobile .acct-btn:active {
    background: var(--hov);
  }

  .acct-text {
    flex: 1 1 0;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 3px;
  }

  .acct-head {
    display: flex;
    align-items: center;
    gap: 8px;
    min-width: 0;
  }

  .mobile .acct-head {
    gap: 6px;
  }

  .acct-email {
    font-size: 15.5px;
    font-weight: 700;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .mobile .acct-email {
    font-size: 15px;
  }

  .badge {
    flex-shrink: 0;
    height: 22px;
    padding: 0 8px;
    border-radius: 9999px;
    background: var(--sunken);
    color: var(--mfg);
    display: grid;
    place-items: center;
    font-size: 11.5px;
    font-weight: 800;
    white-space: nowrap;
  }

  .acct-sub {
    font-size: 13.5px;
    color: var(--mfg);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .acct-st {
    flex: 0 1 200px;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 3px;
  }

  /* State: "Sign-in needed" and "Blocked" in the error colour, otherwise the dot fixed green */
  .st {
    display: flex;
    align-items: center;
    gap: 8px;
    min-width: 0;
    font-size: 14px;
    font-weight: 700;
    color: var(--fg);
    white-space: nowrap;
  }

  .mobile .st {
    gap: 7px;
    font-size: 13px;
  }

  .st.bad {
    color: var(--danger);
  }

  .dot {
    width: 9px;
    height: 9px;
    flex-shrink: 0;
    border-radius: 9999px;
    /* Fixed green */
    background: #5fbf73;
  }

  .mobile .dot {
    width: 8px;
    height: 8px;
  }

  .st.bad .dot {
    background: var(--danger);
  }

  .st-text {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .last {
    padding-left: 17px;
    font-size: 13px;
    color: var(--mfg);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .go-icon {
    width: 18px;
    flex-shrink: 0;
    display: grid;
    place-items: center;
    font-size: 18px;
    color: var(--mfg);
  }

  .empty {
    padding: 16px 18px;
    font-size: 14.5px;
    color: var(--mfg);
  }

  .mobile .empty {
    padding: 14px;
    font-size: 14px;
  }

  /* Add account */
  .form {
    margin-top: 4px;
    padding: 18px 18px 16px;
    border-radius: 20px;
    background: var(--surface);
    box-shadow: var(--sh1);
    animation: snFade 200ms ease-out;
  }

  .mobile .form {
    padding: 16px 12px 12px;
  }

  .f-title {
    margin: 0;
    font-size: 15.5px;
    font-weight: 800;
  }

  .mobile .f-title {
    padding: 0 4px;
    font-size: 15px;
  }

  .f-intro {
    margin: 3px 0 0;
    font-size: 14px;
    line-height: 1.45;
    color: var(--mfg);
    text-wrap: pretty;
  }

  .mobile .f-intro {
    margin-top: 2px;
    padding: 0 4px;
    font-size: 13.5px;
  }

  .fields {
    margin-top: 16px;
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(220px, 1fr));
    gap: 14px 12px;
  }

  .mobile .fields {
    margin-top: 14px;
    display: flex;
    flex-direction: column;
    gap: 12px;
  }

  .field {
    display: flex;
    flex-direction: column;
    gap: 7px;
    min-width: 0;
  }

  .mobile .field {
    padding: 0 2px;
  }

  .f-label {
    font-size: 13.5px;
    font-weight: 700;
    color: var(--mfg);
  }

  .mobile .f-label {
    padding: 0 2px;
  }

  .inp {
    display: block;
    width: 100%;
    height: 44px;
    padding: 0 18px;
    border: 0;
    border-radius: 9999px;
    background: var(--surface2);
    box-shadow: inset 0 0 0 1px var(--bd);
    font-size: 14px;
    font-weight: 600;
    outline: none;
    transition: box-shadow var(--dur, 220ms);
  }

  .inp.bad {
    box-shadow: 0 0 0 2px var(--danger);
  }

  .inp:focus {
    box-shadow: 0 0 0 3px var(--acc);
  }

  .mobile .inp {
    height: 46px;
    padding: 0 16px;
    font-size: 16px;
    transition: none;
  }

  .sec-wrap {
    position: relative;
    display: block;
  }

  .inp.sec-inp {
    padding: 0 50px 0 18px;
  }

  .mobile .inp.sec-inp {
    padding: 0 52px 0 16px;
  }

  .eye {
    position: absolute;
    top: 4px;
    right: 4px;
    width: 36px;
    height: 36px;
    border-radius: 9999px;
    display: grid;
    place-items: center;
    font-size: 18px;
    color: var(--mfg);
    transition:
      background-color var(--dur, 220ms),
      color var(--dur, 220ms);
  }

  .eye:disabled,
  .seg button:disabled {
    cursor: default;
    opacity: 0.6;
  }

  .eye:hover {
    background: var(--hov);
    color: var(--fg);
  }

  .mobile .eye {
    top: 1px;
    right: 1px;
    width: 44px;
    height: 44px;
    font-size: 19px;
    transition: none;
  }

  .mobile .eye:hover {
    background: none;
    color: var(--mfg);
  }

  .mobile .eye:active {
    background: var(--hov);
  }

  /* Server picker: ring and chevron while open; disabled half transparent */
  .sel {
    position: relative;
    min-width: 0;
  }

  .sel.dis,
  .sel-btn.dis {
    opacity: 0.5;
  }

  .sel-btn {
    width: 100%;
    height: 44px;
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 0 14px 0 18px;
    border-radius: 9999px;
    background: var(--surface2);
    box-shadow: inset 0 0 0 1px var(--bd);
    font-size: 14px;
    font-weight: 700;
    text-align: left;
    transition: box-shadow var(--dur, 220ms);
  }

  .sel-btn.bad {
    box-shadow: 0 0 0 2px var(--danger);
  }

  .sel-btn.open {
    box-shadow: 0 0 0 3px var(--acc);
  }

  .sel.dis .sel-btn,
  .sel-btn.dis {
    cursor: default;
  }

  .sel-btn > :global(svg) {
    font-size: 17px;
    color: var(--mfg);
  }

  .sel-label {
    flex: 1;
    min-width: 0;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .chev {
    display: grid;
    font-size: 16px;
    color: var(--mfg);
    transition: transform var(--dur, 220ms) cubic-bezier(0.3, 1.5, 0.5, 1);
  }

  .sel-btn.open .chev {
    transform: rotate(180deg);
  }

  .mobile .sel-btn {
    height: 46px;
    padding: 0 14px 0 16px;
    font-size: 15px;
    transition: none;
  }

  .mobile .chev {
    font-size: 17px;
  }

  .pick-pop {
    position: absolute;
    top: calc(100% + 8px);
    left: 0;
    z-index: 30;
    width: 100%;
    min-width: 260px;
    max-height: 300px;
    overflow-y: auto;
    padding: 6px;
    border-radius: 22px;
    background: var(--pop);
    color: var(--fg);
    box-shadow: var(--sh3);
    display: flex;
    flex-direction: column;
    gap: 2px;
    transform-origin: top center;
    animation: snPop 220ms cubic-bezier(0.2, 1.2, 0.3, 1);
  }

  .pick-opt {
    flex-shrink: 0;
    min-height: 48px;
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 7px 10px 7px 14px;
    border-radius: 15px;
    text-align: left;
    transition: background-color 120ms;
  }

  .pick-opt.hi {
    background: var(--hov);
  }

  .pick-text {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 1px;
  }

  .pick-label {
    font-size: 14.5px;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .pick-sub {
    font-size: 12.5px;
    color: var(--mfg);
  }

  .pick-check {
    width: 22px;
    height: 22px;
    flex-shrink: 0;
    border-radius: 9999px;
    display: grid;
    place-items: center;
    font-size: 13px;
    color: var(--inkfg);
    transition: background-color var(--dur, 220ms);
  }

  .pick-check :global(svg) {
    opacity: 0;
  }

  .pick-opt.on .pick-check {
    background: var(--ink);
  }

  .pick-opt.on .pick-check :global(svg) {
    opacity: 1;
  }

  /* Hints below the server picker */
  .hint-ok,
  .hint-warn,
  .none {
    display: flex;
    align-items: flex-start;
    gap: 6px;
    margin: 0;
    padding: 0 6px;
    font-size: 13px;
    line-height: 1.4;
  }

  .hint-ok {
    font-weight: 700;
    color: var(--fg);
  }

  .hint-ok :global(svg) {
    flex-shrink: 0;
    font-size: 15px;
    /* Fixed green */
    color: #4fae64;
  }

  .hint-warn,
  .none {
    font-weight: 600;
    color: var(--mfg);
  }

  .hint-warn :global(svg),
  .none :global(svg) {
    flex-shrink: 0;
    font-size: 15px;
    margin-top: 1px;
  }

  .none {
    padding: 4px 6px 0;
    font-size: 13.5px;
  }

  /* Segment "Sign-in" */
  .seg {
    display: flex;
    gap: 2px;
    padding: 4px;
    border-radius: 9999px;
    background: var(--sunken);
  }

  .seg button {
    flex: 1;
    min-width: 0;
    height: 36px;
    padding: 0 10px;
    border-radius: 9999px;
    font-size: 13.5px;
    font-weight: 700;
    white-space: nowrap;
    background: transparent;
    color: var(--mfg);
    transition:
      background-color var(--dur, 220ms),
      color var(--dur, 220ms);
  }

  .seg button:hover,
  .seg button.on {
    color: var(--fg);
  }

  .seg button.on {
    background: var(--surface);
    box-shadow: var(--sh1);
  }

  .mobile .seg button {
    height: 38px;
    padding: 0;
    transition: none;
  }

  .pw-hint {
    margin-top: 14px;
    display: flex;
    align-items: flex-start;
    gap: 8px;
    font-size: 13.5px;
    line-height: 1.45;
    color: var(--mfg);
    text-wrap: pretty;
  }

  .pw-hint :global(svg) {
    flex-shrink: 0;
    font-size: 16px;
    margin-top: 1px;
  }

  .pw-hint span {
    min-width: 0;
    overflow-wrap: anywhere;
  }

  .mobile .pw-hint {
    padding: 0 4px;
  }

  /* Spinner via `--spin` (off without animations) */
  .spin {
    display: grid;
    animation: var(--spin, none);
  }

  .busy {
    margin-top: 14px;
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 10px 14px 10px 10px;
    border-radius: 16px;
    background: var(--surface2);
    font-size: 14px;
    font-weight: 700;
    color: var(--mfg);
  }

  .busy-tile {
    width: 30px;
    height: 30px;
    flex-shrink: 0;
    border-radius: 10px;
    background: var(--surface);
    display: grid;
    place-items: center;
    font-size: 16px;
  }

  /* Error box */
  .err {
    margin-top: 14px;
    display: flex;
    align-items: flex-start;
    gap: 12px;
    padding: 12px 14px 12px 12px;
    border-radius: 18px;
    background: color-mix(in oklab, var(--danger) 10%, var(--surface2));
    animation: snFade 200ms ease-out;
  }

  .err-tile {
    width: 34px;
    height: 34px;
    flex-shrink: 0;
    border-radius: 12px;
    background: var(--surface);
    color: var(--danger);
    display: grid;
    place-items: center;
    font-size: 18px;
    transform: rotate(-6deg);
  }

  .err-body {
    flex: 1;
    min-width: 0;
    padding-top: 1px;
  }

  .err-title {
    font-size: 14.5px;
    font-weight: 800;
    color: var(--danger);
  }

  .err-text {
    margin-top: 2px;
    font-size: 14px;
    line-height: 1.5;
    text-wrap: pretty;
  }

  .err-code {
    margin-top: 6px;
    font-family: ui-monospace, 'SF Mono', Menlo, monospace;
    font-size: 12.5px;
    color: var(--mfg);
    overflow-wrap: anywhere;
  }

  .err-act {
    margin-top: 10px;
    height: 36px;
    display: flex;
    align-items: center;
    gap: 7px;
    padding: 0 14px 0 11px;
    border-radius: 9999px;
    background: var(--surface);
    box-shadow: var(--sh1);
    font-size: 13.5px;
    font-weight: 800;
    white-space: nowrap;
  }

  .err-act :global(svg) {
    font-size: 15px;
  }

  .mobile .err-act {
    height: 42px;
  }

  /* Buttons of the form */
  .acts {
    margin-top: 16px;
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .mobile .acts {
    margin-top: 14px;
    flex-direction: column;
    align-items: stretch;
    gap: 6px;
  }

  .grow {
    flex: 1;
  }

  .cancel {
    height: 44px;
    padding: 0 18px;
    border-radius: 9999px;
    font-size: 14px;
    font-weight: 700;
    transition: background-color var(--dur, 220ms);
  }

  .cancel:hover {
    background: var(--hov);
  }

  .mobile .cancel {
    width: 100%;
    font-size: 15px;
    transition: none;
  }

  .mobile .cancel:hover {
    background: none;
  }

  .mobile .cancel:active {
    background: var(--hov);
  }

  /* Submit: accent as soon as ready or while loading (`hot`); cursor by state */
  .submit {
    height: 44px;
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 0 20px 0 16px;
    border-radius: 9999px;
    background: var(--sunken);
    color: var(--dim);
    cursor: default;
    font-size: 14px;
    font-weight: 800;
    white-space: nowrap;
    transition:
      background-color var(--dur, 220ms),
      color var(--dur, 220ms);
  }

  .submit.hot {
    background: var(--acc);
    color: var(--accink);
  }

  .submit.ready {
    cursor: pointer;
  }

  .submit.locked {
    cursor: not-allowed;
  }

  .submit.busy {
    cursor: progress;
  }

  .btn-icon {
    display: grid;
    font-size: 17px;
  }

  .mobile .submit {
    width: 100%;
    height: 48px;
    justify-content: center;
    padding: 0 16px;
    font-size: 15px;
  }

  .add {
    width: 100%;
    margin-top: 4px;
    height: 52px;
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 10px;
    border-radius: 20px;
    border: 2px dashed var(--bdhi);
    font-size: 14.5px;
    font-weight: 700;
    color: var(--mfg);
    transition:
      background-color var(--dur, 220ms),
      color var(--dur, 220ms);
  }

  .add :global(svg) {
    font-size: 18px;
  }

  .add:hover {
    background: var(--hov);
    color: var(--fg);
  }

  .mobile .add {
    height: 54px;
    font-size: 15px;
    transition: none;
  }

  .mobile .add:hover {
    background: none;
    color: var(--mfg);
  }

  .mobile .add:active {
    background: var(--hov);
  }
</style>
