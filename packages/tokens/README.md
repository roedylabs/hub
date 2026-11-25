# @roedy/tokens

Design tokens (colours, accents, fonts, sizes, motion) as TypeScript constants
and CSS custom properties.

**Origin:** a copy of `packages/tokens` from
[roedylabs/mail](https://github.com/roedylabs/mail) at commit `d5bdf14`
(v0.5.0). The only change: `src/tokens.test.ts` no longer requires the web
app to use a shadow token, since the hub's pages do not use one yet. Change the tokens in Roedy Mail first and copy them over,
so that both apps keep the same look. Moving them into a package that both
repositories share is an open point (`docs/plan.md` › Open points).

`tokens.css` is generated from `src/tokens.ts`: `pnpm --filter @roedy/tokens build`.
