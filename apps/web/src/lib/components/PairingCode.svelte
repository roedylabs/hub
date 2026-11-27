<!--
  A pairing code, shown once after adding an app or "New pairing code": the
  code in groups of four, how to use it, its validity, "Copy" and "Close".
  The heading takes the focus when it appears, so that a screen reader reads
  it next.
-->
<script lang="ts">
  import { onMount } from 'svelte';
  import { tooltip } from '#lib/actions/tooltip.ts';
  import type { Pairing } from '#lib/apps.ts';
  import Icon from '#lib/components/Icon.svelte';
  import { intlLocale } from '#lib/locale.ts';
  import { m } from '#lib/paraglide/messages.js';

  interface Props {
    name: string;
    pairing: Pairing;
    onCopy: () => void;
    onClose: () => void;
    isMobile: boolean;
  }

  let { name, pairing, onCopy, onClose, isMobile }: Props = $props();
  const uid = $props.id();
  let heading: HTMLElement | undefined = $state();

  const until = $derived(
    new Date(pairing.expires_at).toLocaleTimeString(intlLocale(), { hour: '2-digit', minute: '2-digit' }),
  );

  onMount(() => heading?.focus());
</script>

<section class="code-box" class:mobile={isMobile} aria-labelledby="{uid}-h">
  <div class="head">
    <span class="tile"><Icon name="ti-key" /></span>
    <div class="text">
      <h2 id="{uid}-h" tabindex="-1" bind:this={heading}>{m.apps_code_title({ name })}</h2>
      <p>{m.apps_code_text()}</p>
    </div>
    <button
      type="button"
      class="close"
      onclick={onClose}
      aria-label={m.apps_code_close()}
      use:tooltip={isMobile ? undefined : m.shell_close()}
    >
      <Icon name="ti-x" />
    </button>
  </div>
  <div class="code-row">
    <code aria-label={m.apps_code_label()}>{pairing.display}</code>
    <button type="button" class="copy" onclick={onCopy}><Icon name="ti-copy" />{m.apps_code_copy()}</button>
  </div>
  <p class="valid"><Icon name="ti-clock" />{m.apps_code_until({ time: until })}</p>
</section>

<style>
  .code-box {
    margin-top: 14px;
    padding: 18px;
    border-radius: 26px;
    background: var(--accsoft);
    animation: snDlg 300ms cubic-bezier(0.2, 1.2, 0.3, 1);
  }

  .code-box.mobile {
    margin-top: 12px;
    padding: 14px;
    border-radius: 24px;
  }

  .head {
    display: flex;
    align-items: flex-start;
    gap: 14px;
  }

  .tile {
    width: 40px;
    height: 40px;
    flex-shrink: 0;
    display: grid;
    place-items: center;
    border-radius: 14px;
    background: var(--surface);
    color: var(--deep);
    font-size: 20px;
  }

  .text {
    flex: 1;
    min-width: 0;
  }

  h2 {
    margin: 0;
    font-family: var(--font-serif);
    font-weight: 400;
    font-size: 20px;
    outline: none;
  }

  .text p {
    margin: 4px 0 0;
    font-size: 14px;
    line-height: 1.5;
  }

  .close {
    width: 36px;
    height: 36px;
    flex-shrink: 0;
    margin: -6px -6px 0 0;
    border-radius: 9999px;
    display: grid;
    place-items: center;
    color: var(--mfg);
    font-size: 17px;
    transition:
      background-color var(--dur, 220ms),
      color var(--dur, 220ms);
  }

  .mobile .close {
    width: 44px;
    height: 44px;
    margin: -10px -10px 0 0;
  }

  .close:hover {
    background: var(--hov);
    color: var(--fg);
  }

  .code-row {
    margin-top: 14px;
    display: flex;
    align-items: center;
    gap: 10px;
    flex-wrap: wrap;
  }

  code {
    flex: 1 1 320px;
    min-width: 0;
    padding: 12px 18px;
    border-radius: 18px;
    background: var(--surface);
    box-shadow: var(--sh1);
    font-family: ui-monospace, 'SFMono-Regular', Menlo, Consolas, monospace;
    font-size: 16px;
    font-weight: 600;
    letter-spacing: 0.04em;
    overflow-wrap: anywhere;
    user-select: all;
  }

  .mobile code {
    font-size: 15px;
  }

  .copy {
    height: 44px;
    display: inline-flex;
    align-items: center;
    gap: 8px;
    padding: 0 20px 0 16px;
    border-radius: 9999px;
    background: var(--ink);
    color: var(--inkfg);
    font-size: 14px;
    font-weight: 800;
    white-space: nowrap;
  }

  .mobile .copy {
    flex: 1;
    justify-content: center;
    height: 48px;
  }

  .valid {
    margin: 12px 0 0;
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 13.5px;
    font-weight: 600;
    color: var(--fg);
  }
</style>
