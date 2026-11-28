<!--
  Account › Accounts › one account (`/account/accounts/[id]`): head with the
  account's tile and address, the state with "Sign in again" (at the server,
  or a new access token) and "Check", storage, colour and default account,
  the connection's facts and "Remove account". Flow in
  `#lib/stalwart-account.svelte.ts`. An unknown account shows a card with the
  way back to the list.

  Taken over from Roedy Mail (`apps/web/src/lib/components/MailAccountOverview.svelte`
  and `MailConnectionView.svelte`) on one page, without Mail's subpages.
-->
<script lang="ts">
  import { onMount } from 'svelte';
  import { MA_COLORS } from '@roedy/tokens';
  import {
    accountColor,
    accountColorName,
    AccountTile,
    accountStateText,
    ago,
    byteText,
    clockText,
    FirstLoad,
    Icon,
    LoadCard,
    quotaShare,
    radiogroup,
    tooltip,
    ui,
  } from '@roedy/ui';
  import type { AccountError } from '#lib/account-errors.ts';
  import { m } from '#lib/paraglide/messages.js';
  import { prefs } from '#lib/state/prefs.svelte.ts';
  import { session } from '#lib/state/session.svelte.ts';
  import { AccountDetailPage, type AccountNav, type CapabilityKind } from '#lib/stalwart-account.svelte.ts';

  let { id, nav }: { id: string; nav: AccountNav } = $props();

  // The route creates the page anew per account (`{#key}`).
  // svelte-ignore state_referenced_locally
  const p = new AccountDetailPage(id, nav);
  const uid = $props.id();
  const isMobile = $derived(ui.isMobile);
  const a = $derived(p.account);
  /** "Last checked" follows the clock. */
  let now = $state(Date.now());
  type FieldEvent = Event & { currentTarget: HTMLInputElement };

  const CAP_LABEL: Record<CapabilityKind, () => string> = {
    mail: m.acct_cap_mail,
    calendars: m.acct_cap_calendars,
    contacts: m.acct_cap_contacts,
  };

  const statusSub = $derived.by(() => {
    if (!a) return '';
    if (a.state === 'needs_sign_in') return m.acct_needs_text({ host: p.host });
    if (a.state === 'blocked') return m.acct_blocked_text({ host: p.host });
    if (p.checked === 'ok') return m.acct_check_ok({ host: p.host });
    const at = a.quota?.checked_at;
    return at ? m.acct_checked({ ago: ago(Date.parse(at), now) }) : m.acct_not_checked();
  });

  const storage = $derived.by(() => {
    const q = a?.quota;
    if (!q || q.used === null) return null;
    const share = quotaShare(q);
    return {
      used: byteText(q.used, prefs.lang),
      total: q.total === null ? null : byteText(q.total, prefs.lang),
      free: q.total === null ? null : byteText(Math.max(0, q.total - q.used), prefs.lang),
      pct: share === null ? null : Math.round(share * 100),
      full: share !== null && share >= 0.9,
    };
  });

  const facts = $derived.by((): [string, string][] => {
    if (!a) return [];
    const caps = p.capabilities.map((c) => CAP_LABEL[c]()).join(', ');
    return [
      [m.acct_fact_server(), a.server ? `${a.server.label} · ${p.host}` : m.acct_fact_custom({ host: p.host })],
      [m.acct_fact_user(), a.username],
      [m.acct_fact_auth(), a.auth === 'oauth' ? m.acct_auth_oauth() : m.acct_auth_token()],
      [m.acct_fact_caps(), caps || m.acct_fact_caps_none()],
      [
        m.acct_fact_added(),
        new Intl.DateTimeFormat(prefs.lang, { dateStyle: 'medium' }).format(Date.parse(a.created_at)),
      ],
    ];
  });

  const againLabel = m.acct_sign_in_again();
  const errId = `${uid}-err`;
  const lockLeft = $derived(p.clock.left('rate'));

  function act(err: AccountError) {
    if (err.act === 'servers') nav.servers();
  }

  onMount(() => {
    void p.load();
    const clock = setInterval(() => (now = Date.now()), 30_000);
    return () => {
      clearInterval(clock);
      p.dispose();
    };
  });
</script>

{#snippet errorBox(err: AccountError, boxId?: string)}
  <div role="alert" class="err" id={boxId}>
    <span class="err-tile"><Icon name={err.icon} /></span>
    <div class="err-body">
      <div class="err-title">{err.title}</div>
      <div class="err-text">{err.text}</div>
      {#if err.code}<div class="err-code">{m.acct_server_code({ code: err.code })}</div>{/if}
      {#if err.act === 'servers' && session.isAdmin}
        <button type="button" class="err-act" onclick={() => act(err)}>
          <Icon name="ti-arrow-right" />{m.acct_check_servers()}
        </button>
      {/if}
    </div>
  </div>
{/snippet}

<div class="page" class:mobile={isMobile}>
  <button type="button" class="back" onclick={() => nav.list()}><Icon name="ti-arrow-left" />{m.nav_accounts()}</button>
  <FirstLoad
    status={p.status === 'missing' ? 'ready' : p.status}
    slow={p.loading.slow}
    title={m.acct_load_failed()}
    text={p.failure}
    onRetry={() => void p.retry()}
    {isMobile}
  >
    {#if p.status === 'missing' || !a}
      <h1 class="sr-only">{m.acct_missing_title()}</h1>
      <div class="missing">
        <LoadCard icon="ti-mail-off" title={m.acct_missing_title()} text={m.acct_missing_text()} />
      </div>
    {:else}
      <div class="head">
        <AccountTile email={a.email} color={a.color} size={isMobile ? 48 : 56} tilt />
        <div class="head-text">
          <h1 class={isMobile ? 'm-title' : 'title'}>{a.email}</h1>
          <p class="lead">{a.server?.label ?? m.acct_custom_server()} · {p.host}</p>
        </div>
      </div>

      <section class="sec first" aria-labelledby="{uid}-st">
        <h2 class="h2" id="{uid}-st">{m.acct_status()}</h2>
        <div class="status">
          <div class="st-row">
            <div class="st-text">
              <div class="st" class:bad={a.state !== 'ok'}>
                <span class="dot"></span>{accountStateText(a.state)}{#if a.default}<span
                    class="badge small"
                    use:tooltip={isMobile ? undefined : m.acct_default_tip()}
                  >
                    {m.acct_default()}
                  </span>{/if}
              </div>
              <div class="last" aria-live="polite">{statusSub}</div>
            </div>
            <div class="st-acts">
              {#if a.state !== 'blocked' && (p.again === 'signin' || p.again === 'token')}
                <button
                  type="button"
                  class={a.state === 'needs_sign_in' ? 'reauth' : 'soft'}
                  class:busy={!!p.signIn?.busy}
                  aria-busy={!!p.signIn?.busy}
                  aria-disabled={!!p.signIn?.busy || lockLeft > 0}
                  aria-expanded={p.again === 'token' ? !!p.reauth : undefined}
                  onclick={() => void p.startAgain()}
                >
                  <span class:spin={!!p.signIn?.busy}>
                    <Icon name={p.signIn?.busy ? 'ti-loader-2' : 'ti-login-2'} />
                  </span>{lockLeft > 0 && p.again === 'signin'
                    ? m.acct_btn_locked({ clock: clockText(lockLeft) })
                    : againLabel}
                </button>
              {/if}
              {#if a.state === 'ok'}
                <button
                  type="button"
                  class="soft"
                  class:busy={p.checking}
                  aria-busy={p.checking}
                  aria-disabled={p.checking}
                  onclick={() => void p.check()}
                >
                  <span class="rot" style:transform="rotate({p.turns * 360}deg)">
                    <Icon name="ti-refresh" />
                  </span>{p.checking ? m.acct_checking() : m.acct_check()}
                </button>
              {/if}
            </div>
          </div>
          {#if p.again === 'noclient' && a.state === 'needs_sign_in'}
            <p class="note"><Icon name="ti-info-circle" /><span>{m.acct_again_noclient({ host: p.host })}</span></p>
          {/if}
          {#if p.checked && p.checked !== 'ok'}{@render errorBox(p.checked)}{/if}
          {#if p.signIn?.error}{@render errorBox(p.signIn.error)}{/if}
          {#if p.reauth}
            {@const r = p.reauth}
            <div class="reauth-form">
              <label class="field">
                <span class="f-label">{m.acct_token_new()}</span>
                <span class="sec-wrap">
                  <input
                    class="inp"
                    class:bad={r.error?.field === 'token'}
                    type={r.showToken ? 'text' : 'password'}
                    value={r.token}
                    oninput={(e: FieldEvent) => p.updateReauth({ token: e.currentTarget.value })}
                    onkeydown={(e) => {
                      if (e.key === 'Enter') {
                        e.preventDefault();
                        void p.submitReauth();
                      } else if (e.key === 'Escape') {
                        e.preventDefault();
                        p.cancelReauth();
                      }
                    }}
                    aria-invalid={r.error ? 'true' : undefined}
                    aria-describedby={r.error ? errId : undefined}
                    autocomplete="off"
                    autocapitalize="off"
                    spellcheck="false"
                  />
                  <button
                    type="button"
                    class="eye"
                    onclick={() => p.updateReauth({ showToken: !r.showToken })}
                    aria-label={r.showToken ? m.acct_hide() : m.acct_show()}
                    aria-pressed={r.showToken}
                    use:tooltip={isMobile ? undefined : r.showToken ? m.acct_hide() : m.acct_show()}
                  >
                    <Icon name={r.showToken ? 'ti-eye-off' : 'ti-eye'} />
                  </button>
                </span>
              </label>
              {#if r.error}{@render errorBox(r.error, errId)}{/if}
              <div class="acts">
                <button type="button" class="cancel" onclick={() => p.cancelReauth()}>{m.acct_cancel()}</button>
                <button
                  type="button"
                  class="submit"
                  class:busy={r.busy}
                  aria-busy={r.busy}
                  aria-disabled={r.busy || lockLeft > 0}
                  onclick={() => void p.submitReauth()}
                >
                  <span class="btn-icon" class:spin={r.busy}>
                    <Icon name={r.busy ? 'ti-loader-2' : 'ti-key'} />
                  </span>{lockLeft > 0
                    ? m.acct_btn_locked({ clock: clockText(lockLeft) })
                    : r.busy
                      ? m.acct_btn_connecting()
                      : m.acct_btn_save_token()}
                </button>
              </div>
            </div>
          {/if}
        </div>
      </section>

      <section class="sec" aria-labelledby="{uid}-store">
        <h2 class="h2" id="{uid}-store">{m.acct_storage()}</h2>
        <div class="card store">
          {#if storage}
            <div class="store-head">
              <span class="used">{storage.used}</span>
              {#if storage.free}<span class="free">{m.acct_free({ size: storage.free })}</span>{/if}
            </div>
            {#if storage.pct !== null && storage.total}
              <div
                class="bar"
                role="progressbar"
                aria-label={m.acct_storage()}
                aria-valuenow={storage.pct}
                aria-valuemin={0}
                aria-valuemax={100}
                aria-valuetext={m.acct_quota({ used: storage.used, total: storage.total })}
              >
                <div class="fill" class:full={storage.full} style:width="{storage.pct}%"></div>
              </div>
              <div class="quota">{m.acct_quota({ used: storage.used, total: storage.total })} · {storage.pct} %</div>
            {:else}
              <div class="quota">{m.acct_no_limit_text({ host: p.host })}</div>
            {/if}
          {:else if a.quota}
            <div class="q-title">{m.acct_no_limit()}</div>
            <div class="quota">{m.acct_no_limit_text({ host: p.host })}</div>
          {:else}
            <div class="q-title">{m.acct_not_checked()}</div>
            <div class="quota">{m.acct_storage_idle({ host: p.host })}</div>
          {/if}
        </div>
      </section>

      <section class="sec" aria-labelledby="{uid}-look">
        <h2 class="h2" id="{uid}-look">{m.acct_appearance()}</h2>
        <div class="card rows">
          <div class="row">
            <div class="row-text">
              <div class="row-title" id="{uid}-color">{m.acct_color()}</div>
              <div class="row-desc">{m.acct_color_desc()}</div>
            </div>
            <div class="swatches" role="radiogroup" use:radiogroup aria-labelledby="{uid}-color">
              {#each MA_COLORS as color, i (color[0])}
                {@const c = accountColor(i)}
                <button
                  type="button"
                  class="sw"
                  class:on={a.color === i}
                  role="radio"
                  aria-checked={a.color === i}
                  aria-label={accountColorName(i)}
                  style:--sw-base={c.bg}
                  style:--sw-ink={c.ink}
                  onclick={() => void p.setColor(i)}
                  use:tooltip={isMobile ? undefined : accountColorName(i)}
                >
                  <Icon name="ti-check" />
                </button>
              {/each}
            </div>
          </div>
          <div class="sep"></div>
          <div class="row def">
            <div class="row-text">
              <div class="row-title">{m.acct_default_title()}</div>
              <div class="row-desc">{m.acct_default_desc()}</div>
            </div>
            {#if a.default}
              <span class="badge"><Icon name="ti-star-filled" />{m.acct_default()}</span>
            {:else}
              <button type="button" class="soft" onclick={() => void p.makeDefault()}>
                <Icon name="ti-star" />{m.acct_make_default()}
              </button>
            {/if}
          </div>
        </div>
      </section>

      <section class="sec" aria-labelledby="{uid}-con">
        <h2 class="h2" id="{uid}-con">{m.acct_connection()}</h2>
        <dl class="card facts">
          {#each facts as [k, val] (k)}
            <div class="fact">
              <dt class="fact-k">{k}</dt>
              <dd class="fact-v">{val}</dd>
            </div>
          {/each}
        </dl>
      </section>

      <section class="sec" aria-labelledby="{uid}-rm">
        <h2 class="h2" id="{uid}-rm">{m.acct_remove_head()}</h2>
        <div class="remove-box">
          <div class="remove-lead">{m.acct_remove_lead({ app: session.appName })}</div>
          <button type="button" class="remove-btn" onclick={() => p.remove()}>
            <Icon name="ti-mail-off" />{m.acct_remove()}
          </button>
        </div>
      </section>
    {/if}
  </FirstLoad>
</div>

<style>
  .back {
    height: 36px;
    display: inline-flex;
    align-items: center;
    gap: 7px;
    margin: -4px 0 18px -10px;
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
    display: none;
  }

  .missing {
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
    font-size: 34px;
    line-height: 1.15;
    overflow-wrap: anywhere;
  }

  .m-title {
    margin: 0;
    font-family: var(--font-serif);
    font-weight: 400;
    font-size: 26px;
    line-height: 1.15;
    overflow-wrap: anywhere;
  }

  .lead {
    margin: 6px 0 0;
    color: var(--mfg);
    font-size: 15px;
    overflow-wrap: anywhere;
  }

  .mobile .lead {
    font-size: 14px;
  }

  .sec {
    margin-top: 36px;
  }

  .sec.first {
    margin-top: 32px;
  }

  .mobile .sec {
    margin-top: 28px;
  }

  .h2 {
    margin: 0;
    font-family: var(--font-serif);
    font-weight: 400;
    font-size: 24px;
  }

  .mobile .h2 {
    font-size: 22px;
  }

  /* State */
  .status {
    margin-top: 14px;
    padding: 20px 22px;
    border-radius: 26px;
    background: var(--surface2);
  }

  .mobile .status {
    padding: 16px;
    border-radius: 24px;
  }

  .st-row {
    display: flex;
    align-items: center;
    gap: 16px 20px;
    flex-wrap: wrap;
  }

  .st-text {
    flex: 1 1 220px;
    min-width: 0;
  }

  .st {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 15.5px;
    font-weight: 800;
    color: var(--fg);
  }

  .st.bad {
    color: var(--danger);
  }

  .dot {
    width: 10px;
    height: 10px;
    flex-shrink: 0;
    border-radius: 9999px;
    /* Fixed green */
    background: #5fbf73;
  }

  .st.bad .dot {
    background: var(--danger);
  }

  .last {
    margin-top: 4px;
    font-size: 14px;
    color: var(--mfg);
    text-wrap: pretty;
    overflow-wrap: anywhere;
  }

  .st-acts {
    display: flex;
    gap: 8px;
    flex-wrap: wrap;
  }

  .mobile .st-acts {
    width: 100%;
    flex-direction: column;
  }

  .note {
    margin: 14px 0 0;
    display: flex;
    align-items: flex-start;
    gap: 8px;
    font-size: 14px;
    line-height: 1.5;
    color: var(--mfg);
  }

  .note :global(svg) {
    flex-shrink: 0;
    font-size: 16px;
    margin-top: 2px;
  }

  /* "Sign in again" as the main action while needed */
  .reauth {
    height: 44px;
    flex-shrink: 0;
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 8px;
    padding: 0 20px 0 16px;
    border-radius: 9999px;
    background: var(--ink);
    color: var(--inkfg);
    font-size: 14px;
    font-weight: 800;
    white-space: nowrap;
    box-shadow: var(--sh1);
  }

  .soft {
    height: 44px;
    flex-shrink: 0;
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 8px;
    padding: 0 18px 0 14px;
    border-radius: 9999px;
    background: var(--surface);
    font-size: 14px;
    font-weight: 700;
    white-space: nowrap;
    box-shadow: var(--sh1);
    transition: background-color var(--dur, 220ms);
  }

  .soft:hover {
    background: var(--sunken);
  }

  .soft.busy,
  .reauth.busy {
    cursor: progress;
  }

  .reauth :global(svg),
  .soft :global(svg) {
    font-size: 17px;
  }

  .rot,
  .spin {
    display: grid;
    place-items: center;
  }

  /* Refresh icon: two turns per check */
  .rot {
    transition: transform calc(var(--dur, 220ms) * 6.4) cubic-bezier(0.45, 0, 0.2, 1);
  }

  /* Spinner via `--spin` (off without animations) */
  .spin {
    animation: var(--spin, none);
  }

  /* Access token form */
  .reauth-form {
    margin-top: 16px;
    padding-top: 16px;
    border-top: 1px solid var(--bd);
  }

  .field {
    display: flex;
    flex-direction: column;
    gap: 7px;
    max-width: 420px;
  }

  .f-label {
    font-size: 13.5px;
    font-weight: 700;
    color: var(--mfg);
  }

  .sec-wrap {
    position: relative;
    display: block;
  }

  .inp {
    display: block;
    width: 100%;
    height: 44px;
    padding: 0 50px 0 18px;
    border: 0;
    border-radius: 9999px;
    background: var(--surface);
    box-shadow: var(--sh1);
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
    height: 48px;
    padding: 0 52px 0 16px;
    font-size: 16px;
    transition: none;
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
  }

  .eye:hover {
    background: var(--hov);
    color: var(--fg);
  }

  .mobile .eye {
    top: 2px;
    right: 2px;
    width: 44px;
    height: 44px;
    font-size: 19px;
  }

  .acts {
    margin-top: 14px;
    display: flex;
    justify-content: flex-end;
    gap: 8px;
  }

  .mobile .acts {
    flex-direction: column-reverse;
    gap: 6px;
  }

  .cancel {
    height: 44px;
    padding: 0 18px;
    border-radius: 9999px;
    font-size: 14px;
    font-weight: 700;
  }

  .cancel:hover {
    background: var(--hov);
  }

  .submit {
    height: 44px;
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 8px;
    padding: 0 20px 0 16px;
    border-radius: 9999px;
    background: var(--acc);
    color: var(--accink);
    font-size: 14px;
    font-weight: 800;
    white-space: nowrap;
  }

  .submit.busy {
    cursor: progress;
  }

  .btn-icon {
    display: grid;
    font-size: 17px;
  }

  /* Error box */
  .err {
    margin-top: 14px;
    display: flex;
    align-items: flex-start;
    gap: 12px;
    padding: 12px 14px 12px 12px;
    border-radius: 18px;
    background: color-mix(in oklab, var(--danger) 10%, var(--surface));
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

  /* Storage */
  .card {
    margin: 14px 0 0;
    border-radius: 26px;
    background: var(--surface2);
  }

  .mobile .card {
    border-radius: 24px;
  }

  .store {
    padding: 20px 22px;
  }

  .mobile .store {
    padding: 16px;
  }

  .store-head {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: 12px;
    flex-wrap: wrap;
  }

  .used,
  .q-title {
    font-family: var(--font-serif);
    font-size: 26px;
    line-height: 1.25;
  }

  .q-title {
    color: var(--mfg);
  }

  .free {
    font-size: 14px;
    font-weight: 700;
    color: var(--mfg);
  }

  .bar {
    margin-top: 12px;
    height: 10px;
    border-radius: 9999px;
    background: var(--sunken);
    overflow: hidden;
  }

  /* Bar colour: red from 90 % */
  .fill {
    height: 100%;
    border-radius: 9999px;
    background: var(--acc);
    transition: width 400ms cubic-bezier(0.2, 0.9, 0.3, 1);
  }

  .fill.full {
    background: var(--danger);
  }

  .quota {
    margin-top: 8px;
    font-size: 13.5px;
    color: var(--mfg);
    text-wrap: pretty;
  }

  /* Appearance */
  .rows {
    padding: 6px;
  }

  .row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px 24px;
    padding: 14px 12px 14px 18px;
    flex-wrap: wrap;
  }

  .mobile .row {
    padding: 12px 10px 12px 12px;
  }

  .row.def {
    min-height: 72px;
  }

  .row-text {
    flex: 1 1 200px;
    min-width: 0;
  }

  .row-title {
    font-size: 15.5px;
    font-weight: 700;
  }

  .row-desc {
    font-size: 14px;
    color: var(--mfg);
    margin-top: 3px;
  }

  .sep {
    height: 1px;
    background: var(--bd);
    margin: 0 18px;
  }

  .swatches {
    display: flex;
    gap: 10px;
    flex-wrap: wrap;
    padding: 5px;
  }

  .sw {
    width: 32px;
    height: 32px;
    border-radius: 9999px;
    background: var(--sw-base);
    display: grid;
    place-items: center;
    color: var(--sw-ink);
    font-size: 15px;
    transition:
      box-shadow var(--dur, 220ms),
      transform var(--dur, 220ms) cubic-bezier(0.3, 1.5, 0.5, 1);
  }

  .sw:hover {
    transform: scale(1.1);
  }

  .sw.on {
    box-shadow:
      0 0 0 3px var(--surface2),
      0 0 0 5px var(--sw-base);
  }

  .sw :global(svg) {
    opacity: 0;
  }

  .sw.on :global(svg) {
    opacity: 1;
  }

  .badge {
    height: 32px;
    padding: 0 14px;
    border-radius: 9999px;
    background: var(--accsoft);
    color: var(--deep);
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: 13.5px;
    font-weight: 800;
  }

  .badge :global(svg) {
    font-size: 15px;
  }

  .badge.small {
    height: 22px;
    margin-left: 4px;
    padding: 0 8px;
    background: var(--sunken);
    color: var(--mfg);
    font-size: 11.5px;
  }

  /* Facts of the connection */
  .facts {
    padding: 6px 0;
  }

  .fact {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: 6px 24px;
    padding: 12px 22px;
    flex-wrap: wrap;
  }

  .mobile .fact {
    display: block;
    padding: 10px 16px;
  }

  .fact + .fact {
    border-top: 1px solid var(--bd);
  }

  .fact-k {
    font-size: 14.5px;
    font-weight: 700;
  }

  .fact-v {
    margin: 0;
    min-width: 0;
    font-size: 14.5px;
    font-weight: 600;
    color: var(--mfg);
    text-align: right;
    overflow-wrap: anywhere;
  }

  .mobile .fact-v {
    margin-top: 2px;
    text-align: left;
    font-size: 14px;
  }

  /* Remove */
  .remove-box {
    margin-top: 14px;
    display: flex;
    align-items: center;
    gap: 14px 20px;
    flex-wrap: wrap;
    padding: 18px 18px 18px 22px;
    border-radius: 26px;
    background: color-mix(in oklab, var(--danger) 8%, var(--surface2));
  }

  .mobile .remove-box {
    padding: 16px;
    border-radius: 24px;
  }

  .remove-lead {
    flex: 1 1 240px;
    min-width: 0;
    font-size: 14.5px;
    line-height: 1.5;
    text-wrap: pretty;
  }

  .remove-btn {
    height: 44px;
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 8px;
    padding: 0 20px 0 16px;
    border-radius: 9999px;
    background: var(--danger);
    color: var(--surface);
    font-size: 14px;
    font-weight: 800;
    white-space: nowrap;
  }

  .mobile .remove-btn {
    width: 100%;
  }

  .remove-btn :global(svg) {
    font-size: 17px;
  }
</style>
