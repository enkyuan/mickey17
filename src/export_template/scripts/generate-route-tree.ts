// Writes src/routeTree.gen.ts without building. TanStack Start's router plugin
// generates the file in Vite's `configResolved` hook, so resolving the config
// runs the exact generator (config, footer, route manifest) that `vite build`
// and `vite dev` use. `bun run check` calls this first so a fresh checkout
// type-checks; the generated file is committed like any TanStack Start project.
import { resolveConfig } from 'vite'

await resolveConfig({ logLevel: 'warn' }, 'build')
