# @roedy/ui

Svelte 5 components, actions and helpers that every Roedy app (the hub, Mail,
Calendar, Contacts …) shares, so that they look and behave the same. The
package brings its own English and German texts: apps need no message keys
for it. The hub's web app uses it from the workspace; the apps will get it as a
published package (`docs/plan.md` § 9.4, publishing is milestone 7.3).

What an app does not get here: the hub's frame (the app switcher and later the
notification bell and notice bar) comes at run time from the hub
(`GET /api/v1/frame.js`, `docs/app-contract.md` § 5.5); the design tokens are
`@roedy/tokens`; JMAP is `@roedy/jmap`.

## Use

```ts
// Once at start: the package's texts follow the app's language, `ui.dark` its colour scheme.
import { connectSettings } from '@roedy/ui';
connectSettings({ lang: () => prefs.lang, mode: () => prefs.values.mode });
```

```svelte
<script lang="ts">
  import { ConfirmDialog, Icon, Sheet, Toast, ui } from '@roedy/ui';
</script>
```

The app's page loads the tokens' style sheet (`@roedy/tokens/tokens.css`) and
the package's utility classes (`@roedy/ui/base.css`: `sr-only`,
`no-scrollbar`); the components use the tokens' custom properties for colours,
fonts and motion, in light and dark.

## Contents

| Area | Modules |
|---|---|
| Components | `AccountMenu`, `ColorEditor`, `ConfirmDialog`, `DirtyBar`, `ErrorView`, `FirstLoad`, `Header`, `Icon`, `LoadCard`, `Logo`, `PickerList`, `SettingsNav`, `Sheet`, `Skeleton`, `Toast`, `Tooltip` |
| Actions | `focustrap`, `radiogroup`, `scrollbar`, `tablist`, `tooltip`, `dialogDrag` and the swipe gestures (`swipe.ts`) |
| Shell state | `ui` (layout, dark theme and motion, menus, the page's dialog, confirmation, toasts, tooltip), `Loading` |
| Helpers | `api` (the hub's browser API, `/api/v1`), `ago`, `waitText`/`clockText`, `theme` (`applyTheme`), `color` and the colour editor's values, `errors` (error pages, the gateway's codes), `keys`, `landmark`, `logo`, `branding`, `shell` (values of the shell components), `Reconnect` ("No connection" with retries), `icons` |
| Texts | `texts()` (the package's `en`/`de` tables), `connectSettings()` |
| Testing (`@roedy/ui/testing`) | `pointer`/`swipe` gestures in jsdom, WCAG contrast, axe-core (`violations()`, needs `axe-core`) |

`Icon` takes the name of a registered icon (`ti-…`, `icons.ts`: names in data –
branding, the apps' manifests, the audit log – mean the same icon everywhere) or
a Tabler component for an app's own icons.

## Texts

Each module keeps its texts next to its code:

```ts
const t = texts({
  en: { close: 'Close', minutes: ({ n }: { n: number }) => `${n} minutes` },
  de: { close: 'Schließen', minutes: ({ n }) => `${n} Minuten` },
});
```

The German table must have the English keys and parameters (checked by the
type). A text reads the language on every call, so templates follow a change.

## Development

Imports inside the package are relative, with the file extension. Checks:
`pnpm --filter @roedy/ui check` and `pnpm --filter @roedy/ui test` (Vitest
with jsdom and Testing Library; component tests include axe).

## Origin

The modules come from the hub's web app (`apps/web/src/lib`), which took most of
them over from Roedy Mail standalone (`roedylabs/mail`, archived as
`mail-standalone`, same project and licence; commit `d5bdf14`, v0.5.0); each
module names its origin in its comment.
