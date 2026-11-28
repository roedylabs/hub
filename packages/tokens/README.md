# @roedy/tokens

Design tokens of the Roedy apps (colours, accents, fonts, sizes, motion) as
TypeScript constants and CSS custom properties. This package is their source:
the hub's web app and `@roedy/ui` use it from the workspace, the apps (Mail,
Calendar, Contacts …) as a published package (`docs/plan.md` § 9.4). Change the
tokens here, never in an app.

`tokens.css` is generated from `src/tokens.ts`: `pnpm --filter @roedy/tokens build`.

**Origin:** taken over from `packages/tokens` of Roedy Mail standalone
(`roedylabs/mail`, archived as `mail-standalone`) at commit `d5bdf14` (v0.5.0),
same project and licence; that copy is archived with it.
