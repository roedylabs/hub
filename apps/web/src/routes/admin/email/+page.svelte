<!--
  Administration › Email: the sending account of the system mails with status
  and "Test connection", connection (address of the JMAP server, sign-in with
  password or token), sender, sending, test email and the "Unsaved changes"
  bar. Flow and values in `#lib/system-mail.svelte.ts`. The status, tile,
  ring and switch styles are classes derived from `v.mSt.key`, `busy` and
  `on`. Field errors: red ring and text with `role="alert"` below the field,
  on desktop right-aligned below the row, on mobile directly below the field;
  the field names the text via `aria-describedby`.

  Taken over from Roedy Mail (`apps/web/src/routes/admin/email/+page.svelte`)
  with an address field instead of the server registry's pick list and without
  the template preview (the hub's API has none).
-->
<script lang="ts">
  import { onMount } from 'svelte';
  import { DirtyBar, FirstLoad, Icon, tooltip, ui } from '@roedy/ui';
  import { m } from '#lib/paraglide/messages.js';
  import { scrollToSection } from '#lib/sections.ts';
  import { pageOf } from '#lib/settings-nav.ts';
  import {
    SystemMailPage,
    systemMailValues,
    type MailFieldState,
    type SystemMailContext,
  } from '#lib/system-mail.svelte.ts';

  let root: HTMLElement | undefined = $state();
  const ctx: SystemMailContext = {
    scrollTo: (section) => scrollToSection(root, section, ui.motion),
  };
  const p = new SystemMailPage(ctx.scrollTo);
  const uid = $props.id();
  const errId = (field: string): string => `${uid}-${field}-err`;
  /** "Last checked" follows the clock. */
  let now = $state(Date.now());
  const v = $derived(systemMailValues(p, now, ctx));
  const isMobile = $derived(ui.isMobile);
  const info = $derived(pageOf('/admin/email'));

  onMount(() => {
    void p.load();
    const clock = setInterval(() => (now = Date.now()), 30_000);
    return () => {
      clearInterval(clock);
      p.dispose();
    };
  });
</script>

{#snippet rowText(label: string, desc: string)}
  <div class="row-text">
    <div class="row-label">{label}</div>
    <div class="row-desc">{desc}</div>
  </div>
{/snippet}

{#snippet fieldError(field: string, f: MailFieldState)}
  <div role="alert" id={errId(field)} class="field-error"><Icon name="ti-alert-circle" />{f.err}</div>
{/snippet}

<div bind:this={root} class="page" class:mobile={isMobile}>
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
    title={m.email_load_failed()}
    text={p.failure}
    onRetry={() => void p.retry()}
    {isMobile}
  >
    <div class="tabs no-scrollbar">
      {#each v.mailTabs as t (t.label)}<button type="button" class="tab" onclick={t.onClick}>{t.label}</button>{/each}
    </div>

    <section data-sec="status" class="sec first">
      <h2 class="sec-title">{m.email_status_title()}</h2>
      <div class="status st-{v.mSt.key}">
        <span class="st-tile"><Icon name={v.mSt.icon} /></span>
        <div class="st-text">
          <div class="st-title">{v.mSt.title}</div>
          <div class="st-sub">{v.mSt.sub}</div>
        </div>
        <button
          type="button"
          class="test"
          class:busy={v.mailBusy}
          aria-disabled={v.mailBusy}
          aria-busy={v.mailBusy}
          onclick={v.mailTest}
        >
          <span class="test-icon" style:--rot="{v.mailRot}deg"><Icon name="ti-refresh" /></span>{v.mailBtn}
        </button>
      </div>
      {#if v.mailOk}
        <div class="box rows">
          {#each v.mailRows as r (r.k)}
            <div>
              {#if r.sep}<div class="sep"></div>{/if}
              <div class="kv">
                <span class="kv-k">{r.k}</span>
                <span class="kv-v">{r.v}</span>
              </div>
            </div>
          {/each}
        </div>
      {/if}
    </section>

    <section data-sec="conn" class="sec">
      <h2 class="sec-title">{m.email_tab_conn()}</h2>
      <p class="sec-desc">{m.email_conn_desc()}</p>
      <div class="box">
        <div class="row">
          {@render rowText(m.email_url_label(), v.mailUrlSub)}
          <input
            type="url"
            value={v.mailF.url}
            placeholder={m.email_url_ph()}
            aria-label={m.email_url_label()}
            aria-invalid={v.mailErr.url.badA}
            aria-describedby={v.mailErr.url.bad ? errId('url') : undefined}
            class:bad={v.mailErr.url.bad}
            autocomplete="url"
            autocapitalize="off"
            spellcheck="false"
            oninput={(e) => v.mailOn.url(e.currentTarget.value)}
          />
          {#if v.mailErr.url.bad && isMobile}{@render fieldError('url', v.mailErr.url)}{/if}
        </div>
        {#if v.mailErr.url.bad && !isMobile}{@render fieldError('url', v.mailErr.url)}{/if}
        <div class="sep"></div>
        <div class="row">
          {@render rowText(m.email_auth_label(), m.email_auth_desc())}
          <div role="group" aria-label={m.email_auth_label()} class="seg">
            {#each v.mailAuthOpts as o (o.value)}
              <button type="button" class:on={o.on} aria-pressed={o.on} onclick={o.onClick}>{o.label}</button>
            {/each}
          </div>
        </div>
        {#if v.mailIsPw}
          <div class="sep"></div>
          <div class="row">
            {@render rowText(m.email_user_label(), m.email_user_desc())}
            <input
              type="text"
              value={v.mailF.user}
              placeholder={m.email_user_ph()}
              aria-label={m.email_user_label()}
              aria-invalid={v.mailErr.user.badA}
              aria-describedby={v.mailErr.user.bad ? errId('user') : undefined}
              class:bad={v.mailErr.user.bad}
              autocomplete="username"
              autocapitalize="off"
              spellcheck="false"
              oninput={(e) => v.mailOn.user(e.currentTarget.value)}
            />
            {#if v.mailErr.user.bad && isMobile}{@render fieldError('user', v.mailErr.user)}{/if}
          </div>
          {#if v.mailErr.user.bad && !isMobile}{@render fieldError('user', v.mailErr.user)}{/if}
        {/if}
        <div class="sep"></div>
        <div class="row">
          {@render rowText(v.mailSecret.label, v.mailSecret.desc)}
          <div class="secret">
            <input
              type={v.mailSecret.type}
              value={v.mailSecret.value}
              placeholder={v.mailSecret.ph}
              autocomplete={v.mailSecret.ac}
              aria-label={v.mailSecret.label}
              aria-invalid={v.mailSecret.badA}
              aria-describedby={v.mailSecret.bad ? errId(v.mailSecret.field) : undefined}
              class:bad={v.mailSecret.bad}
              autocapitalize="off"
              spellcheck="false"
              oninput={(e) => v.mailSecret.onChange(e.currentTarget.value)}
            />
            <button
              type="button"
              class="eye"
              aria-label={v.mailSecret.eyeTip}
              use:tooltip={isMobile ? undefined : v.mailSecret.eyeTip}
              onclick={v.mailSecret.onEye}
            >
              <Icon name={v.mailSecret.eyeIcon} />
            </button>
          </div>
          {#if v.mailSecret.bad && isMobile}{@render fieldError(v.mailSecret.field, v.mailSecret)}{/if}
        </div>
        {#if v.mailSecret.bad && !isMobile}{@render fieldError(v.mailSecret.field, v.mailSecret)}{/if}
      </div>
    </section>

    <section data-sec="sender" class="sec">
      <h2 class="sec-title">{m.email_tab_sender()}</h2>
      <p class="sec-desc">{m.email_sender_desc()}</p>
      <div class="box">
        <div class="row">
          {@render rowText(m.email_name_label(), m.email_name_desc())}
          <input
            type="text"
            value={v.mailF.name}
            placeholder={v.mailNamePh}
            aria-label={m.email_name_label()}
            aria-invalid={v.mailErr.name.badA}
            aria-describedby={v.mailErr.name.bad ? errId('name') : undefined}
            class:bad={v.mailErr.name.bad}
            oninput={(e) => v.mailOn.name(e.currentTarget.value)}
          />
          {#if v.mailErr.name.bad && isMobile}{@render fieldError('name', v.mailErr.name)}{/if}
        </div>
        {#if v.mailErr.name.bad && !isMobile}{@render fieldError('name', v.mailErr.name)}{/if}
        <div class="sep"></div>
        <div class="row">
          {@render rowText(m.email_reply_label(), m.email_reply_desc())}
          <input
            type="email"
            value={v.mailF.replyTo}
            placeholder={m.email_reply_ph()}
            aria-label={m.email_reply_label()}
            aria-invalid={v.mailErr.replyTo.badA}
            aria-describedby={v.mailErr.replyTo.bad ? errId('replyTo') : undefined}
            class:bad={v.mailErr.replyTo.bad}
            autocapitalize="off"
            spellcheck="false"
            oninput={(e) => v.mailOn.replyTo(e.currentTarget.value)}
          />
          {#if v.mailErr.replyTo.bad && isMobile}{@render fieldError('replyTo', v.mailErr.replyTo)}{/if}
        </div>
        {#if v.mailErr.replyTo.bad && !isMobile}{@render fieldError('replyTo', v.mailErr.replyTo)}{/if}
        <div class="sep"></div>
        <div class="preview">
          <span class="row-label">{m.email_preview()}</span>
          <span class="preview-from">{v.mailFrom}</span>
        </div>
      </div>
    </section>

    <section data-sec="delivery" class="sec">
      <h2 class="sec-title">{m.email_tab_delivery()}</h2>
      <p class="sec-desc">{m.email_delivery_desc()}</p>
      <div class="box">
        <div class="row toggle">
          {@render rowText(m.email_keep_sent(), m.email_keep_sent_desc())}
          <button
            type="button"
            role="switch"
            class="switch"
            class:on={v.tgMailSent.on}
            aria-checked={v.tgMailSent.on}
            aria-label={m.email_keep_sent()}
            onclick={v.tgMailSent.onClick}
          >
            <span class="knob"></span>
          </button>
        </div>
      </div>
    </section>

    <section data-sec="test" class="sec">
      <h2 class="sec-title">{m.email_test_title()}</h2>
      <p class="sec-desc">{m.email_test_desc()}</p>
      <div class="box">
        <div class="row">
          {@render rowText(m.email_to_label(), m.email_to_desc())}
          <div class="to">
            <input
              type="email"
              value={v.mailToVal}
              aria-label={m.email_to_label()}
              autocapitalize="off"
              spellcheck="false"
              oninput={(e) => v.onMailTo(e.currentTarget.value)}
              onkeydown={v.mailToKey}
            />
            {#if !isMobile}
              <button
                type="button"
                class="send"
                class:on={!v.mailSendDis}
                aria-disabled={v.mailSendDis}
                onclick={v.mailSend}
              >
                <Icon name="ti-send" />{v.mailSendLabel}
              </button>
            {/if}
          </div>
        </div>
        {#if !isMobile}
          {#if v.mailNotOk}
            <div class="sep"></div>
            <div class="hint"><Icon name="ti-info-circle" />{m.email_send_hint()}</div>
          {/if}
        {:else}
          <div class="m-send">
            {#if v.mailNotOk}<div class="hint"><Icon name="ti-info-circle" />{m.email_send_hint()}</div>{/if}
            <button
              type="button"
              class="send"
              class:on={!v.mailSendDis}
              aria-disabled={v.mailSendDis}
              onclick={v.mailSend}
            >
              <Icon name="ti-send" />{v.mailSendLabel}
            </button>
          </div>
        {/if}
      </div>
    </section>

    {#if v.mailDirty}
      <DirtyBar onDiscard={v.mailDiscard} onSave={v.mailSave} {isMobile} saving={v.mailSaving} />
    {/if}
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

  /* Section tabs */
  .tabs {
    display: flex;
    gap: 8px;
    flex-wrap: wrap;
    margin-top: 22px;
  }

  .mobile .tabs {
    flex-wrap: nowrap;
    margin: 20px -18px 0;
    padding: 0 18px;
    overflow-x: auto;
  }

  .tab {
    height: 40px;
    padding: 0 18px;
    border-radius: 9999px;
    background: var(--surface2);
    font-size: 14px;
    font-weight: 700;
    white-space: nowrap;
    transition: background-color var(--dur, 220ms);
  }

  .tab:hover {
    background: var(--sunken);
  }

  .mobile .tab {
    height: 38px;
    flex-shrink: 0;
    padding: 0 16px;
    font-size: 13.5px;
    transition: none;
  }

  .mobile .tab:hover {
    background: var(--surface2);
  }

  .mobile .tab:active {
    background: var(--sunken);
  }

  /* Sections */
  .sec {
    margin-top: 44px;
  }

  .sec.first {
    margin-top: 36px;
  }

  .mobile .sec {
    margin-top: 34px;
  }

  .mobile .sec.first {
    margin-top: 28px;
  }

  .sec-title {
    margin: 0;
    font-family: var(--font-serif);
    font-weight: 400;
    font-size: 24px;
  }

  .mobile .sec-title {
    font-size: 22px;
  }

  .sec-desc {
    margin: 6px 0 0;
    font-size: 14.5px;
    color: var(--mfg);
  }

  .mobile .sec-desc {
    margin-top: 5px;
    font-size: 14px;
  }

  .box {
    margin-top: 18px;
    border-radius: 26px;
    background: var(--surface2);
    padding: 6px;
  }

  .mobile .box {
    margin-top: 16px;
    border-radius: 24px;
    padding: 4px;
  }

  .box.rows {
    margin-top: 12px;
  }

  .mobile .box.rows {
    margin-top: 10px;
  }

  .sep {
    height: 1px;
    background: var(--bd);
    margin: 0 18px;
  }

  .mobile .sep {
    margin: 0 14px;
  }

  /* Status: colours per state (`mSt.bg`, `mSt.tile`, `mSt.ink`) */
  .status {
    --st-bg: var(--surface2);
    --st-tile: var(--surface);
    --st-ink: var(--fg);
    margin-top: 18px;
    display: flex;
    align-items: center;
    gap: 16px 18px;
    flex-wrap: wrap;
    padding: 20px 20px 20px 22px;
    border-radius: 26px;
    background: var(--st-bg);
    transition: background-color var(--dur, 220ms);
  }

  .status.st-busy,
  .status.st-ok {
    --st-bg: var(--accsoft);
    --st-tile: var(--acc);
    --st-ink: var(--accink);
  }

  .status.st-err {
    --st-bg: color-mix(in oklab, var(--danger) 13%, var(--surface));
    --st-tile: var(--danger);
    --st-ink: var(--surface);
  }

  .mobile .status {
    margin-top: 16px;
    flex-direction: column;
    flex-wrap: nowrap;
    gap: 0;
    text-align: center;
    padding: 24px 16px 16px;
    border-radius: 24px;
  }

  .st-tile {
    width: 56px;
    height: 56px;
    flex-shrink: 0;
    border-radius: 19px;
    background: var(--st-tile);
    color: var(--st-ink);
    display: grid;
    place-items: center;
    font-size: 28px;
    transform: rotate(-6deg);
    box-shadow: var(--sh1);
    transition: background-color var(--dur, 220ms);
  }

  .st-text {
    flex: 1 1 220px;
    min-width: 0;
  }

  .mobile .st-text {
    flex: none;
  }

  .st-title {
    font-family: var(--font-serif);
    font-size: 24px;
    line-height: 1.2;
  }

  .mobile .st-title {
    margin-top: 14px;
    font-size: 22px;
  }

  .st-sub {
    margin-top: 4px;
    font-size: 14.5px;
    color: var(--fg);
    text-wrap: pretty;
  }

  .mobile .st-sub {
    font-size: 14px;
  }

  .test {
    height: 44px;
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 9px;
    padding: 0 20px 0 16px;
    border-radius: 9999px;
    background: var(--ink);
    color: var(--inkfg);
    font-size: 14.5px;
    font-weight: 800;
    white-space: nowrap;
    /* Fixed shadow */
    box-shadow: 0 10px 24px -10px rgba(43, 34, 28, 0.5);
  }

  .test.busy {
    cursor: progress;
  }

  .mobile .test {
    margin-top: 18px;
    align-self: stretch;
  }

  /* Rotates by the tile's tilt; without animations `--dur` is 0. */
  .test-icon {
    display: grid;
    font-size: 18px;
    transform: rotate(var(--rot));
    transition: transform calc(var(--dur, 220ms) * 6.4) cubic-bezier(0.45, 0, 0.2, 1);
  }

  /* Rows session, account, last checked */
  .kv {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 24px;
    padding: 16px 18px;
  }

  .mobile .kv {
    gap: 16px;
    padding: 14px;
  }

  .kv-k {
    flex-shrink: 0;
    font-size: 15.5px;
    font-weight: 700;
  }

  .mobile .kv-k {
    font-size: 15px;
  }

  .kv-v {
    min-width: 0;
    font-size: 15px;
    font-weight: 600;
    color: var(--mfg);
    font-variant-numeric: tabular-nums;
    text-align: right;
    overflow-wrap: anywhere;
  }

  .mobile .kv-v {
    font-size: 14.5px;
  }

  /* Fields: on desktop label left, field right; stacked on mobile */
  .row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px 24px;
    padding: 16px 18px;
    flex-wrap: wrap;
  }

  .mobile .row {
    display: block;
    padding: 14px 14px 12px;
  }

  .row.toggle {
    gap: 24px;
    flex-wrap: nowrap;
  }

  .mobile .row.toggle {
    display: flex;
    gap: 16px;
    padding: 14px;
  }

  .row-text {
    flex: 1 1 200px;
    min-width: 0;
  }

  .row.toggle .row-text {
    flex: 1;
  }

  .row-label {
    font-size: 15.5px;
    font-weight: 700;
  }

  .mobile .row-label {
    font-size: 15px;
  }

  .row-desc {
    font-size: 14px;
    color: var(--mfg);
    margin-top: 3px;
  }

  .mobile .row-desc {
    margin-top: 2px;
    font-size: 13.5px;
  }

  input {
    flex: 0 1 320px;
    min-width: 0;
    width: 100%;
    height: 44px;
    padding: 0 18px;
    border: 0;
    border-radius: 9999px;
    background: var(--surface);
    box-shadow: var(--sh1);
    font-size: 14px;
    font-weight: 600;
    outline: none;
    transition: box-shadow var(--dur, 220ms);
  }

  .mobile input {
    display: block;
    height: 46px;
    margin-top: 12px;
    padding: 0 16px;
    font-size: 16px;
    transition: none;
  }

  input:focus-visible {
    box-shadow: 0 0 0 3px var(--acc);
  }

  /* "Sign-in" segment */
  .seg {
    display: flex;
    gap: 2px;
    padding: 4px;
    border-radius: 9999px;
    background: var(--sunken);
  }

  .mobile .seg {
    margin-top: 12px;
  }

  .seg button {
    height: 36px;
    padding: 0 16px;
    border-radius: 9999px;
    background: transparent;
    color: var(--mfg);
    font-size: 14px;
    font-weight: 700;
    white-space: nowrap;
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
    flex: 1;
    min-width: 0;
    height: 38px;
    padding: 0;
  }

  .mobile .seg button:hover:not(.on) {
    color: var(--mfg);
  }

  /* Password or access token with "Show" */
  .secret {
    position: relative;
    flex: 0 1 320px;
    min-width: 0;
    width: 100%;
  }

  .mobile .secret {
    margin-top: 12px;
  }

  .secret input {
    display: block;
    padding-right: 50px;
  }

  .mobile .secret input {
    margin-top: 0;
    padding-right: 52px;
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

  /* Sender preview */
  .preview {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px 24px;
    padding: 16px 18px;
    flex-wrap: wrap;
  }

  .mobile .preview {
    display: block;
    padding: 14px;
  }

  .preview-from {
    min-width: 0;
    font-size: 15px;
    font-weight: 600;
    color: var(--mfg);
    overflow-wrap: anywhere;
  }

  .mobile .preview-from {
    display: block;
    margin-top: 4px;
    font-size: 14.5px;
  }

  /* Switch */
  .switch {
    width: 52px;
    height: 32px;
    border-radius: 9999px;
    background: var(--sunken);
    position: relative;
    flex-shrink: 0;
    transition: background-color var(--dur, 220ms);
  }

  .switch.on {
    background: var(--acc);
  }

  .knob {
    position: absolute;
    top: 4px;
    left: 4px;
    width: 24px;
    height: 24px;
    border-radius: 9999px;
    /* fixed values (switch) */
    background: #fff;
    box-shadow: 0 2px 4px rgba(60, 35, 10, 0.25);
    transition: transform var(--dur, 220ms) cubic-bezier(0.3, 1.5, 0.5, 1);
  }

  .switch.on .knob {
    transform: translateX(20px);
  }

  /* Test email */
  .to {
    flex: 0 1 420px;
    min-width: 0;
    width: 100%;
    display: flex;
    gap: 8px;
  }

  .to input {
    flex: 1;
  }

  .mobile .to {
    display: block;
  }

  .send {
    height: 44px;
    flex-shrink: 0;
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

  .send :global(svg) {
    font-size: 17px;
  }

  .send.on {
    background: var(--acc);
    color: var(--accink);
    cursor: pointer;
  }

  .mobile .send {
    width: 100%;
    height: 48px;
    justify-content: center;
    padding: 0;
    font-size: 15px;
  }

  .mobile .send :global(svg) {
    font-size: 18px;
  }

  .m-send {
    padding: 0 10px 10px;
  }

  .hint {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 14px 18px;
    font-size: 14px;
    color: var(--mfg);
  }

  .hint :global(svg) {
    font-size: 18px;
    flex-shrink: 0;
  }

  .mobile .hint {
    gap: 8px;
    padding: 2px 4px 12px;
  }

  .mobile .hint :global(svg) {
    font-size: 17px;
  }

  /* Field error: red ring, text below the row (desktop) or the field (mobile) */
  input.bad {
    box-shadow: 0 0 0 2px var(--danger);
  }

  input.bad:focus-visible {
    box-shadow: 0 0 0 3px var(--acc);
  }

  .field-error {
    display: flex;
    align-items: center;
    justify-content: flex-end;
    gap: 8px;
    padding: 0 18px 14px;
    font-size: 14px;
    font-weight: 600;
    line-height: 1.4;
    text-align: right;
    color: var(--danger);
  }

  .mobile .field-error {
    margin-top: 10px;
    align-items: flex-start;
    justify-content: flex-start;
    padding: 0;
    font-size: 13.5px;
    text-align: left;
  }

  .field-error :global(svg) {
    flex-shrink: 0;
    font-size: 18px;
  }
</style>
