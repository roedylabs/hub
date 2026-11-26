<!--
  First load of a settings page below its head (Administration › Users and
  Email): until the first response the list skeleton with "Loading data …"
  (only after the skeleton delay), if the first load fails a "Couldn’t load …" card (`ti-cloud-off` in
  `--danger`, reason, "Try again") instead of the content, otherwise the
  content. Reopening with known data shows the content at once; a later
  failure is an error toast of the page.

  Taken over from Roedy Mail (`apps/web/src/lib/components/FirstLoad.svelte`).
-->
<script lang="ts">
  import type { Snippet } from 'svelte';
  import LoadCard from '#lib/components/LoadCard.svelte';
  import Skeleton from '#lib/components/Skeleton.svelte';
  import { m } from '#lib/paraglide/messages.js';

  interface Props {
    status: 'loading' | 'ready' | 'failed';
    /** The request takes longer than the skeleton delay. */
    slow: boolean;
    /** Heading of the card, e.g. "Couldn’t load users". */
    title: string;
    /** Reason on the card (`loadFailText`). */
    text: string;
    onRetry: () => void;
    isMobile: boolean;
    children: Snippet;
  }

  let { status, slow, title, text, onRetry, isMobile, children }: Props = $props();
</script>

{#if status === 'loading' && slow}<Skeleton set="list" {isMobile} note={m.loading_data()} />{/if}

{#if status === 'failed'}<LoadCard icon="ti-cloud-off" {title} {text} {onRetry} />{/if}

{#if status === 'ready'}{@render children()}{/if}
