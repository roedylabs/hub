# @roedy/tokens

Design tokens of the Roedy apps (colours, accents, fonts, sizes, motion) as
TypeScript constants and CSS custom properties. This package is their source:
the hub's web app and `@roedy/ui` use it from the workspace, the apps (Mail,
Calendar, Contacts …) as a published package (`docs/plan.md` § 9.4). Change the
tokens here, never in an app.

`tokens.css` is generated from `src/tokens.ts`: `pnpm --filter @roedy/tokens build`.

## Use

```sh
npm install @roedy/tokens
```

```css
@import '@roedy/tokens/tokens.css';
```

```ts
import { ACCENTS, DEFAULT_ACCENT, MODES } from '@roedy/tokens';
```

Published with every hub release under the hub's version (`0.x` until the
first release), as JavaScript with type declarations and the style sheet
(CONTRIBUTING.md › Packages and crates).

**Origin:** taken over from `packages/tokens` of Roedy Mail standalone
(`roedylabs/mail`, archived as `mail-standalone`) at commit `d5bdf14` (v0.5.0),
same project and licence; that copy is archived with it.
