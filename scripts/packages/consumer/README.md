# Package consumer

A throwaway SvelteKit app that `scripts/packages.sh verify` copies outside the
workspace, installs with npm from the packed tarballs of `@roedy/tokens`,
`@roedy/jmap` and `@roedy/ui` (as a Roedy app in its own repository would get
them from npm), then type checks with the published declarations, builds,
renders a component and imports the plain packages in Node. It is not part of
the pnpm workspace.
