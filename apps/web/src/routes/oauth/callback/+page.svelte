<!--
  Return from a Stalwart server's sign-in page (`/oauth/callback`, redirect
  URI of the hub's OAuth client), in the design of the hub's sign-in pages:
  "Finishing sign-in …" while the hub completes it, then on to where the
  sign-in started; an error stays here with its text, "Try again" where it
  can help (or "Open account" for an account connected already) and "Back".
  Flow in `#lib/callback.svelte.ts`.

  Roedy Mail's page (`apps/web/src/routes/oauth/callback/+page.svelte`) had no
  UI of its own; there the sign-in page showed the return.
-->
<script lang="ts">
  import { onMount } from 'svelte';
  import { afterNavigate, replaceState } from '$app/navigation';
  import AuthResult from '#lib/components/auth/AuthResult.svelte';
  import type { ResultPrimary, ResultQuiet } from '#lib/components/auth/types.ts';
  import { CallbackFlow } from '#lib/callback.svelte.ts';
  import { leaveTo } from '#lib/navigate.ts';
  import { m } from '#lib/paraglide/messages.js';
  import { session } from '#lib/state/session.svelte.ts';
  import { CALLBACK_PATH } from '#lib/stalwart.ts';

  /** Code and `state` still in the address: the router was not ready yet (the tab's first page). */
  let dirty = false;
  function clean(): void {
    try {
      // eslint-disable-next-line svelte/no-navigation-without-resolve -- the page's own path without the query
      void replaceState(CALLBACK_PATH, {});
      dirty = false;
    } catch {
      dirty = true;
    }
  }
  // The first navigation of the tab ends right after the page mounted: then the router replaces the entry.
  afterNavigate(() => {
    if (dirty) clean();
  });

  const flow = new CallbackFlow({
    // Code and `state` leave the address and the history entry at once.
    clean,
    leave: (to) => leaveTo(to),
  });
  /** No element of the result screen is reached from outside. */
  const reg = () => () => {};

  const err = $derived(flow.error);
  const primary = $derived.by((): ResultPrimary | undefined => {
    if (flow.phase !== 'error') return undefined;
    if (err?.act === 'open' && err.account)
      return { label: m.acct_open_account(), icon: 'ti-arrow-right', onclick: () => flow.openAccount() };
    if (flow.canRetry)
      return {
        label: m.cb_retry(),
        icon: 'ti-refresh',
        iconLeft: true,
        busy: flow.retrying,
        busyLabel: m.acct_btn_opening(),
        onclick: () => void flow.retry(),
      };
    return { label: m.cb_back(), icon: 'ti-arrow-left', iconLeft: true, onclick: () => flow.back() };
  });
  const quiet = $derived.by((): ResultQuiet | undefined =>
    flow.phase === 'error' && primary?.label !== m.cb_back()
      ? { label: m.cb_back(), icon: 'ti-arrow-left', onclick: () => flow.back() }
      : undefined,
  );

  onMount(() => {
    void flow.run(new URL(location.href));
    return () => flow.dispose();
  });
</script>

{#snippet text()}
  {#if flow.phase === 'error' && err}
    {err.text}
    {#if err.code}<span class="code">{m.acct_server_code({ code: err.code })}</span>{/if}
  {:else}
    {flow.host ? m.cb_busy_text({ host: flow.host }) : m.cb_busy_text_none()}
  {/if}
{/snippet}

<AuthResult
  docTitle={m.cb_doc_title({ app: session.appName })}
  title={flow.phase === 'error' && err ? err.title : m.cb_busy_title()}
  themeToggle
  {reg}
  contentKey={flow.phase + (err?.title ?? '')}
  tone={flow.phase === 'error' ? 'muted' : 'good'}
  tileShadow={flow.phase === 'error' ? 'edge' : 'lift'}
  icon={flow.phase === 'error' && err ? err.icon : 'ti-login-2'}
  role={flow.phase === 'error' ? 'alert' : 'status'}
  {text}
  progress={flow.phase === 'busy' ? { text: m.cb_busy_progress(), ms: 2600 } : undefined}
  {primary}
  {quiet}
  endGap={flow.phase === 'busy' ? 18 : 0}
/>

<style>
  .code {
    display: block;
    margin-top: 8px;
    font-family: ui-monospace, 'SF Mono', Menlo, monospace;
    font-size: 0.85em;
    color: var(--mfg);
    overflow-wrap: anywhere;
  }
</style>
