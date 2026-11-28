# @roedy/ui

Svelte 5 components, actions and helpers that every Roedy app (the hub, Mail,
Calendar, Contacts …) shares, so that they look and behave the same. The
package brings its own English and German texts: apps need no message keys
for it. The hub's web app uses it from the workspace; the apps get it from npm,
published with every hub release under the hub's version (`docs/plan.md` § 9.4).

```sh
npm install @roedy/ui @roedy/tokens
```

The package is published as Svelte source with type declarations
(`svelte-package`): the app's Svelte compiles the components. It needs
`svelte` 5 as a peer dependency; `@roedy/ui/testing` needs `axe-core` for
`violations()`. Tests in jsdom need `matchMedia` and `ResizeObserver` stubs
(see `vitest-setup.ts`).

What an app does not get here: the hub's frame (the app switcher and later the
notification bell and notice bar) comes at run time from the hub
(`GET /api/v1/frame.js`, `docs/app-contract.md` § 5.5); the design tokens are
`@roedy/tokens`; JMAP is `@roedy/jmap`.

## Use

```ts
// Once at start: the package follows the user's shared settings in the hub.
import { connectHub } from '@roedy/ui';
connectHub();
```

`connectHub()` reads the hub's shared settings (`docs/app-contract.md` ›
`GET /api/v1/session`, `prefs`): first the copy in
`localStorage["roedy.hub.prefs"]`, so that nothing flashes, then
`GET /api/v1/session`, and changes in another tab (`storage` events). From
then on the package's texts follow the language, `ui.dark` the colour scheme,
`<html>` gets `data-font-size`, `data-density` and `data-animations`
(`@roedy/tokens`), `ui.motion`, `ui.undoMs`, `ui.confirmDelete`,
`ui.shortcuts` and `ui.developer` follow the settings, and `formatDate`,
`formatShortDate`, `formatTime`, `formatNumber` and `firstDayOfWeek` use the
user's formats and time zone (with "automatic" the device's). `hubPrefs()`
returns the values, e.g. the accent for `applyTheme()` or the region. The app
still decides what the settings mean for its own features: it skips `ask()`
while `ui.confirmDelete` is off, ignores its shortcuts while `ui.shortcuts`
is off and shows its developer page only with `ui.developer`.

The hub's own web app connects its state instead:
`connectSettings({ lang: () => prefs.lang, mode: () => prefs.values.mode, … })`.

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
| Components | `AccountMenu`, `ColorEditor`, `CommandPalette`, `ConfirmDialog`, `DebugBar`, `DirtyBar`, `ErrorView`, `FirstLoad`, `Header`, `Icon`, `LicensesDialog`, `LoadCard`, `Logo`, `PickerList`, `SettingsNav`, `Sheet`, `ShortcutsDialog`, `Skeleton`, `TimePicker`, `Toast` (with "Undo" and swiping), `Tooltip` |
| Actions | `focustrap`, `radiogroup`, `scrollbar`, `tablist`, `tooltip`, `dialogDrag` and the swipe gestures (`swipe.ts`) |
| Shell state | `ui` (layout, dark theme and motion, menus, the page's dialog, confirmation, command palette and shortcuts help, toasts, tooltip), `Loading`, `network` (connection state, `appFetch`, `pollVisible`), `devLog` (developer log) |
| Shared settings | `connectHub`, `hubPrefs`, `prefs` (choices, `REGION_FORMATS` and `resolveFormats`, `HubPrefs`, `readHubPrefs`, `applyDisplay`, `effectiveZone`) |
| Helpers | `api` (the hub's browser API, `/api/v1`), `ago`, `waitText`/`clockText`, `theme` (`applyTheme`), `color` and the colour editor's values, `errors` (error pages, the gateway's codes), `keys`, `landmark`, `logo`, `branding`, `shell` (values of the shell components, command palette, shortcuts), `Reconnect` ("No connection" with retries), `icons`, `undo` (`Toasts`: deferred actions with "Undo"), `date`, `time-picker`, `licenses` (the app's licence list), `developer` (debug bar), `AppWorker` (the app's service worker and new-build detection) |
| Texts and settings | `texts()` (the package's `en`/`de` tables), `connectSettings()` |
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

Imports inside the package are relative, with the file extension (`.ts`; the
published build rewrites it to `.js`). Checks: `pnpm --filter @roedy/ui check`
and `pnpm --filter @roedy/ui test` (Vitest with jsdom and Testing Library;
component tests include axe). `pnpm pack` builds `dist/` and the published
exports; how the packages are versioned, checked and released is in
CONTRIBUTING.md › Packages and crates.

## Origin

The modules come from the hub's web app (`apps/web/src/lib`), which took most
of them over from Roedy Mail standalone, and from Roedy Mail standalone
directly: the command palette, shortcuts, licences and debug dialogs, the time
picker, undo, network state, developer log, app worker and date. Roedy Mail
standalone is `roedylabs/mail` (archived as `mail-standalone`, same project
and licence) at commit `d5bdf14` (v0.5.0); each module names its origin in its
comment.
