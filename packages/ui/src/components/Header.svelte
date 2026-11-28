<!--
  Header of the shell: on the desktop the brand (logo and the hub's name, to
  the home page), the toolbar (administration for administrators, settings)
  and the avatar with the account menu below it; on mobile only the brand and
  the avatar, which opens the account menu as a sheet. The account menu comes
  as a snippet from the layout and follows the disclosure pattern: toggle with
  `aria-expanded`, Escape in the popup puts focus back on the toggle before
  the layout closes it.

  Taken over from Roedy Mail (`apps/web/src/lib/components/Header.svelte`)
  without the sidebar toggle, search, notifications and apps menu.
-->
<script lang="ts">
  import type { Snippet } from 'svelte';
  import type { Attachment } from 'svelte/attachments';
  import { tooltip } from '../actions/tooltip.ts';
  import Icon from './Icon.svelte';
  import Logo from './Logo.svelte';
  import { texts } from '../i18n.ts';
  import type { HeaderActions, HeaderValues } from '../shell.ts';

  const t = texts({
    en: {
      admin: 'Administration',
      settings: 'Settings',
    },
    de: {
      admin: 'Administration',
      settings: 'Einstellungen',
    },
  });

  interface Props extends HeaderValues, HeaderActions {
    /** Popover of the account menu while it is open, with its `id` for the avatar's `aria-controls`. */
    accountPop?: Snippet<[string]>;
  }

  let props: Props = $props();
  const uid = $props.id();
  const accountId = `${uid}-account`;

  /**
   * Escape in an open popup: focus first back to its toggle, then the
   * layout closes the popup – otherwise focus would fall to `body` with it.
   */
  const escapeToTrigger: Attachment<HTMLElement> = (anchor) => {
    function onkeydown(event: KeyboardEvent) {
      const trigger = anchor.querySelector<HTMLElement>(':scope > button');
      const from = event.target;
      if (event.key !== 'Escape' || !trigger || !(from instanceof Node) || !anchor.contains(from) || from === trigger)
        return;
      trigger.focus();
    }
    anchor.addEventListener('keydown', onkeydown);
    return () => anchor.removeEventListener('keydown', onkeydown);
  };
</script>

<!-- Initials as decoration, name only for screen readers: it then contains what is visible (WCAG 2.5.3). -->
{#snippet avatarText()}
  <span aria-hidden="true">{props.initials}</span><span class="sr-only">{props.userName}</span>
{/snippet}

{#snippet brand(size: 'desk' | 'mobile')}
  <button type="button" class="brand {size}" onclick={props.goHome}>
    <span class="logo"><Logo logo={props.logo} /></span>
    <span class="name">{props.appName}</span>
  </button>
{/snippet}

{#if !props.isMobile}
  <header class="desk" class:tablet={props.isTablet}>
    <div class="start">{@render brand('desk')}</div>

    <div class="end">
      <div class="tools">
        {#if props.showAdmin}
          <button
            type="button"
            class="tool"
            class:on={props.adminOn}
            onclick={props.openAdmin}
            aria-label={t.admin()}
            aria-current={props.adminOn ? 'page' : undefined}
            use:tooltip={t.admin()}
          >
            <Icon name="ti-shield" />
          </button>
        {/if}
        <button
          type="button"
          class="tool"
          class:on={props.settingsOn}
          onclick={props.openSettingsBtn}
          aria-label={t.settings()}
          aria-current={props.settingsOn ? 'page' : undefined}
          use:tooltip={t.settings()}
        >
          <Icon name="ti-settings" />
        </button>
      </div>
      <div class="anchor" {@attach escapeToTrigger}>
        <button
          type="button"
          class="avatar"
          class:ring={props.accountOpen}
          data-pop="1"
          onclick={props.toggleAccount}
          aria-expanded={props.accountOpen}
          aria-controls={props.accountOpen ? accountId : undefined}
          use:tooltip={props.userName}
        >
          {@render avatarText()}
        </button>
        {#if props.accountOpen}{@render props.accountPop?.(accountId)}{/if}
      </div>
    </div>
  </header>
{:else}
  <header class="mobile">
    {@render brand('mobile')}
    <button
      type="button"
      class="avatar mobile-avatar"
      data-pop="1"
      aria-haspopup="dialog"
      onclick={props.openAccountSheet}
      use:tooltip={props.userName}
    >
      {@render avatarText()}
    </button>
  </header>
{/if}

<style>
  header {
    flex-shrink: 0;
    display: flex;
    align-items: center;
  }

  .desk {
    position: relative;
    z-index: 40;
    height: 76px;
    gap: 16px;
    padding: 0 20px 0 16px;
  }

  .mobile {
    height: 60px;
    gap: 8px;
    padding: 0 14px 0 16px;
  }

  .start {
    flex: 1 1 0;
    display: flex;
    align-items: center;
    min-width: 0;
  }

  .brand {
    display: flex;
    align-items: center;
    min-width: 0;
    text-align: left;
  }

  .brand.desk {
    gap: 12px;
    padding: 4px 14px 4px 4px;
    border-radius: 9999px;
    transition: background-color var(--dur, 220ms);
  }

  .brand.desk:hover {
    background: var(--hov);
  }

  .brand.mobile {
    flex: 1;
    gap: 11px;
  }

  .logo {
    overflow: hidden;
    width: 40px;
    height: 40px;
    border-radius: 14px;
    background: var(--logo-bg, var(--acc));
    color: var(--logo-ink, var(--accink));
    display: grid;
    place-items: center;
    flex-shrink: 0;
    box-shadow:
      inset 0 -3px 0 rgba(0, 0, 0, 0.08),
      var(--sh1);
    transform: rotate(calc(var(--logo-tilt, 1) * -4deg));
  }

  .brand.mobile .logo {
    width: 38px;
    height: 38px;
    border-radius: 13px;
  }

  .name {
    font-family: var(--font-serif);
    font-size: 21px;
    line-height: 1.3;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .brand.mobile .name {
    font-size: 20px;
  }

  .end {
    flex-shrink: 0;
    display: flex;
    align-items: center;
    justify-content: flex-end;
    gap: 10px;
  }

  .tools {
    display: flex;
    align-items: center;
    gap: 2px;
    padding: 4px;
    border-radius: 9999px;
    background: var(--surface);
    border: 1px solid var(--bd);
    box-shadow: var(--sh1);
  }

  .anchor {
    position: relative;
  }

  .tool {
    width: 40px;
    height: 40px;
    border-radius: 9999px;
    display: grid;
    place-items: center;
    color: var(--mfg);
    font-size: 20px;
    transition:
      background-color var(--dur, 220ms),
      color var(--dur, 220ms);
  }

  /* Tablet: larger touch targets in the toolbar */
  .tablet .tool {
    width: 44px;
    height: 44px;
  }

  .tool.on {
    background: var(--accsoft);
    color: var(--deep);
  }

  /*
   * Hover after `.on`: an active button also shows `--hov` and `--fg`. Only with
   * a pointer, so that it does not stick after a tap and hide the active state.
   */
  @media (hover: hover) {
    .tool:hover {
      background: var(--hov);
      color: var(--fg);
    }
  }

  .avatar {
    position: relative;
    width: 46px;
    height: 46px;
    flex-shrink: 0;
    border-radius: 9999px;
    background: var(--accsoft);
    color: var(--deep);
    display: grid;
    place-items: center;
    font-size: 14px;
    font-weight: 800;
    transition:
      box-shadow var(--dur, 220ms),
      transform var(--dur, 220ms);
  }

  .avatar:hover {
    transform: scale(1.05);
  }

  .avatar.ring {
    box-shadow:
      0 0 0 4px var(--cnv),
      0 0 0 6px var(--acc);
  }

  .mobile-avatar {
    width: 44px;
    height: 44px;
  }

  .mobile-avatar:hover {
    transform: none;
  }
</style>
