<script lang="ts">
  import { ErrorView, Icon, Skeleton, isIconName, ui } from '@roedy/ui';
  import IconPlanet from '@tabler/icons-svelte-runes/icons/planet';
  import { DEFAULT_ACCENT } from '@roedy/tokens';
  import { MAIL } from '#lib/mail.ts';

  const name = 'ti-bug';
</script>

<p data-accent={DEFAULT_ACCENT} data-mail={MAIL} data-dark={ui.dark}>
  <Icon name={isIconName(name) ? name : 'ti-x'} label="Bug" />
  <Icon name={IconPlanet} />
</p>
<Skeleton set="list" note="Loading" />
<ErrorView code="404" appName="Consumer" />
