<!--
  Invitation page at `/invite/[token]`: one route, the screen comes from the
  flow's state (`#lib/invite.svelte.ts`). Each screen is its own component
  inside the shared frame (`#lib/components/invite/`): loading, load error,
  step 1 (the invitation), name and password, authenticator app, recovery
  codes, "Welcome, ‹first name›", invitation expired and request sent.
  Unknown links show the 404 error page. "No connection" covers every screen
  when the window goes offline or a request of a step gets no answer.

  Taken over from Roedy Mail (`apps/web/src/routes/invite/[token]/+page.svelte`).
-->
<script lang="ts">
  import { onDestroy, tick, untrack } from 'svelte';
  import { goto } from '$app/navigation';
  import { resolve } from '$app/paths';
  import { page } from '$app/state';
  import ErrorView from '#lib/components/ErrorView.svelte';
  import NoConnection from '#lib/components/auth/NoConnection.svelte';
  import InviteExpired from '#lib/components/invite/InviteExpired.svelte';
  import InviteLoadError from '#lib/components/invite/InviteLoadError.svelte';
  import InviteLoading from '#lib/components/invite/InviteLoading.svelte';
  import InviteRequested from '#lib/components/invite/InviteRequested.svelte';
  import InviteStep1 from '#lib/components/invite/InviteStep1.svelte';
  import InviteStep2 from '#lib/components/invite/InviteStep2.svelte';
  import InviteStep3 from '#lib/components/invite/InviteStep3.svelte';
  import InviteStep4 from '#lib/components/invite/InviteStep4.svelte';
  import InviteWelcome from '#lib/components/invite/InviteWelcome.svelte';
  import { InviteTargets } from '#lib/components/invite/targets.ts';
  import { NetworkError, api } from '#lib/api.ts';
  import { isMobileWidth } from '#lib/auth-layout.ts';
  import { InviteFlow, type InviteField } from '#lib/invite.svelte.ts';
  import { Reconnect } from '#lib/no-connection.svelte.ts';
  import { shake } from '#lib/shake.ts';
  import { forgetAccount, loadSignedIn } from '#lib/state/account.ts';
  import { session, type PublicConfig } from '#lib/state/session.svelte.ts';
  import { ui } from '#lib/state/ui.svelte.ts';

  const token = $derived(page.params.token ?? '');

  const targets = new InviteTargets();
  let netBox: HTMLDivElement | undefined = $state();
  /**
   * Accounts created on this page (`accept`) and how many of them the last finished load covers. A load brings the
   * session as it is when it starts, so it covers exactly the accounts created before that; whichever invitation is
   * shown by then does not matter.
   */
  let created = 0;
  let covered = 0;
  /** Settings and account are loading (after "Welcome", on leaving or on another invitation). */
  let loading = false;
  /** An account came while a load ran: another load follows once that one has ended, whatever its outcome. */
  let queued = false;
  /** The page has gone: the account still loads, but no navigation follows. */
  let leaving = false;

  /** Loads settings and account of the newest session. */
  async function loadAccount(): Promise<boolean> {
    const upTo = created;
    loading = true;
    try {
      const done = await loadSignedIn();
      if (done) covered = Math.max(covered, upTo);
      return done;
    } finally {
      loading = false;
      if (queued) {
        queued = false;
        adopt();
      }
    }
  }

  /** Loads the newest created account unless a finished load covers it; while a load runs, after that one. */
  function adopt() {
    if (created <= covered) return;
    if (loading) queued = true;
    else void loadAccount();
  }

  /** Fields of the screens on the shared shell: on mobile entering one focuses the heading (no on-screen keyboard). */
  const SHELL_FIELDS: readonly InviteField[] = ['name', 'pw', 'pw2', 'otp'];

  /**
   * Moves the focus to `field` once the screen is there. `entering` a shell screen: desktop the field without
   * scrolling, mobile the heading instead. A rejected or missing field of step 2 gets the focus on mobile too (as the
   * checkbox of step 4); the code field on mobile only on entering (the heading): after an answer the on-screen
   * keyboard stays as it is.
   */
  function focus(field: InviteField, entering: boolean) {
    void tick().then(() => {
      // "No connection" keeps the focus until it closes.
      if (net.open) return;
      const shell = SHELL_FIELDS.includes(field);
      const mobile = isMobileWidth(ui.width);
      if (mobile && field === 'otp' && !entering) return;
      const el = targets.get(shell && mobile && entering ? 'heading' : field);
      el?.focus(mobile ? undefined : { preventScroll: true });
    });
  }

  const flow = new InviteFlow({
    fail: (field) => {
      void tick().then(() => {
        shake(targets.get('card'), undefined);
        if (field) focus(field, false);
      });
    },
    focus: (field) => focus(field, true),
    missing: (field) => focus(field, false),
    unlocked: () => focus('otp', false),
    // A previous session of this browser has ended.
    accepted: (left) => {
      created++;
      if (session.authed) forgetAccount();
      // Page left in the meantime: the new session applies anyway.
      if (left) adopt();
    },
    signedIn: async () => {
      if (!(await loadAccount())) return false;
      if (!leaving) void goto(resolve('/'));
      return true;
    },
    toLogin: () => void goto(resolve('/login')),
    error: (text) => ui.toasts.error(text),
    offline: () => net.show(),
  });

  /** An attempt of "No connection": any answer of `public/config` counts, a server error too. */
  async function probe(signal: AbortSignal): Promise<boolean> {
    try {
      await api<PublicConfig>('GET', 'public/config', undefined, { signal });
      return true;
    } catch (error) {
      return !(error instanceof NetworkError);
    }
  }
  const net = new Reconnect({ probe, stayed: () => shake(netBox, undefined) });

  onDestroy(() => {
    leaving = true;
    flow.dispose();
    net.close();
    // Page left after the account was created: the new session applies.
    adopt();
  });

  // New link (visit, switch between invitations): load from the start.
  $effect(() => {
    const value = token;
    untrack(() => {
      // Another invitation in the same page: the account just created still applies before the flow starts anew.
      adopt();
      void flow.open(value);
    });
  });
</script>

<svelte:window onoffline={() => net.show()} ononline={() => void net.attempt()} />

{#if flow.status === 'missing'}
  <ErrorView code="404" />
{:else if flow.status === 'failed'}
  <InviteLoadError {flow} {targets} hidden={net.open} />
{:else if flow.status !== 'ready'}
  <InviteLoading {targets} hidden={net.open} />
{:else if flow.showDone}
  <InviteWelcome {flow} {targets} hidden={net.open} />
{:else if flow.showRequested}
  <InviteRequested {flow} {targets} hidden={net.open} />
{:else if flow.showExpired}
  <InviteExpired {flow} {targets} hidden={net.open} />
{:else if flow.step === 1}
  <InviteStep1 {flow} {targets} hidden={net.open} />
{:else if flow.step === 2}
  <InviteStep2 {flow} {targets} hidden={net.open} />
{:else if flow.step === 3}
  <InviteStep3 {flow} {targets} hidden={net.open} />
{:else}
  <InviteStep4 {flow} {targets} hidden={net.open} />
{/if}
{#if net.open && flow.status !== 'missing'}<NoConnection {net} bind:box={netBox} />{/if}
