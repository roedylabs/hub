<!--
  Access to one app as a radio group of pills: "No access" and the app's
  roles (the role's description as a tooltip and as the option's accessible
  description). Arrow keys switch, one tab stop (`radiogroup`). Used in the
  expanded user, the invitation form and the bulk action of Administration ›
  Users. With `unchanged` a first option "Unchanged" (bulk action) that
  stands for `undefined`.
-->
<script lang="ts">
  import { radiogroup } from '#lib/actions/radiogroup.ts';
  import { tooltip } from '#lib/actions/tooltip.ts';
  import type { AccessChoice, AccessOption } from '#lib/apps.ts';
  import { m } from '#lib/paraglide/messages.js';

  interface Props {
    /** Name of the group, e.g. "Access to Mail". */
    label: string;
    options: AccessOption[];
    onPick: (value: AccessChoice) => void;
    /** Offer "Unchanged" first; `unchangedOn` checks it. */
    unchanged?: { on: boolean; pick: () => void } | null;
    disabled?: boolean;
    /** Id of an error text below the group. */
    describedby?: string | undefined;
    invalid?: boolean;
    isMobile?: boolean;
  }

  let {
    label,
    options,
    onPick,
    unchanged = null,
    disabled = false,
    describedby,
    invalid = false,
    isMobile = false,
  }: Props = $props();
  const uid = $props.id();
</script>

<div
  role="radiogroup"
  use:radiogroup
  aria-label={label}
  aria-describedby={describedby}
  aria-invalid={invalid ? 'true' : undefined}
  class="access"
  class:invalid
  class:mobile={isMobile}
>
  {#if unchanged}
    <button
      type="button"
      role="radio"
      class="opt keep"
      class:on={unchanged.on}
      aria-checked={unchanged.on}
      aria-disabled={disabled}
      onclick={() => !disabled && unchanged.pick()}
    >
      {m.access_unchanged()}
    </button>
  {/if}
  {#each options as o, i (o.value ?? '')}
    <button
      type="button"
      role="radio"
      class="opt"
      class:none={o.value === null}
      class:on={o.on && !unchanged?.on}
      aria-checked={o.on && !unchanged?.on}
      aria-disabled={disabled}
      aria-describedby={o.desc ? `${uid}-d${i}` : undefined}
      use:tooltip={isMobile || !o.desc ? undefined : o.desc}
      onclick={() => !disabled && onPick(o.value)}
    >
      {o.label}{#if o.isDefault}<span class="def">{m.access_default_mark()}</span>{/if}
    </button>
    {#if o.desc}<span class="sr-only" id="{uid}-d{i}">{o.desc}</span>{/if}
  {/each}
</div>

<style>
  .access {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
  }

  .opt {
    height: 36px;
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 0 14px;
    border-radius: 9999px;
    background: var(--surface);
    box-shadow: var(--sh1);
    font-size: 13.5px;
    font-weight: 700;
    white-space: nowrap;
    transition:
      background-color var(--dur, 220ms),
      color var(--dur, 220ms),
      box-shadow var(--dur, 220ms);
  }

  .mobile .opt {
    height: 40px;
    font-size: 14px;
  }

  .opt:hover {
    background: var(--hov);
  }

  .opt.on {
    background: var(--ink);
    color: var(--inkfg);
  }

  .opt.none.on {
    background: var(--sunken);
    color: var(--fg);
  }

  .opt[aria-disabled='true'] {
    opacity: 0.55;
    cursor: default;
  }

  .invalid .opt {
    box-shadow: 0 0 0 2px var(--danger);
  }

  .def {
    font-size: 12px;
    font-weight: 600;
    opacity: 0.75;
  }
</style>
