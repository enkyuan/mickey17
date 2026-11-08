# Editable source projects

Status: design proposal, not implemented. Research checked on 2026-11-07.

The intended output is an editable frontend project with reusable components and native
framework routing. The existing Rust CLI captures deployed HTML and assets. Its Next.js
Flight editing and preview helpers do not reconstruct React source or run a Next.js server.

## Current behavior

- `src/crawl.rs` discovers pages using Chromium, links, and sitemaps. `--extract` saves
  metadata, text, and screenshots. It does not extract a component tree or design system.
- `src/mirror.rs` downloads server HTML and same-site referenced assets, including CSS
  and fonts. It preserves URL paths and can save Next.js RSC responses as `index.rsc`.
- `src/edit.rs` rewrites captured text and URLs, including Flight payloads.
- `src/serve.rs` serves the captured files with clean URLs and Next.js image/RSC handling.

There is no generated `package.json`, `ui/`, `hooks/`, router source, Base UI/shadcn
dependency, Tailwind configuration, StyleX compiler, or token extraction. Styles survive
as compiled CSS, inline styles, or original runtime behavior when their assets are captured.
External font/CDN dependencies may remain remote. This is not verified compatibility with
every framework or CSS engine.

## Architecture for reconstruction

Keep capture reusable and add a separate source export pipeline:

```text
browser capture
    -> document + assets + stylesheet rules + observed states
    -> route graph + component model + token model
    -> framework adapter + primitive adapter + styling adapter
    -> editable project
    -> install, typecheck, build, routing, interaction, visual checks
```

The intermediate model should include:

- Routes: literal paths, confirmed parameter patterns, nested layouts, redirects, query
  state, metadata, error/404 views, and navigation relationships.
- Components: semantic elements, shared shells, repeated sections, props, local state,
  interactive behavior, accessibility requirements, and asset references.
- Styles: matched rules, computed values at several widths and states, CSS variables,
  media/container queries, pseudo elements, typography, and animations.
- Assets: source URLs, content hashes, local destinations, MIME types, and font metadata.
- Evidence: what was observed, what was inferred, confidence, and unresolved behavior.

A rendered website cannot reliably reveal its original hooks, source filenames, route
groups, backend code, or component boundaries. Infer a maintainable new structure; do not
claim to recover the original repository. Preserve literal routes unless there is evidence
or an explicit mapping for dynamic parameters. Two similar URLs alone do not prove a
shared dynamic route. Mark unavailable server behavior as an integration requirement.

Framework adapters own project configuration, syntax, entry points, routing, metadata,
layout composition, asset URLs, hydration boundaries, and build/check commands. Styling
and primitive adapters should be independent, with an explicit supported-combinations
matrix. Unsupported combinations should fail with an explanation.

An eventual CLI could expose `--framework`, `--styling`, and `--primitives`. These flags
do not exist today. Detecting the source framework can aid capture; the selected output
framework decides the generated source structure.

## Framework routing and directories

| Target | Native route output | Shared code |
| --- | --- | --- |
| TanStack Start / React | `src/routes/__root.tsx`, `index.tsx`, `posts/$postId.tsx`; `src/router.tsx`; tool-generated `routeTree.gen.ts` | `src/components`, `src/features`, `src/hooks`, `src/lib` |
| Astro | `src/pages/index.astro`, `posts/[postId].astro`; `src/layouts`; interactive islands only where needed | `.astro` components, optional framework islands, `src/styles`, `public` |
| SvelteKit | `src/routes/+layout.svelte`, `+page.svelte`, `posts/[postId]/+page.svelte`; data loading only where implemented | `src/lib/components`, feature modules/stores, `static` |
| Next.js App Router | `src/app/layout.tsx`, `page.tsx`, `posts/[postId]/page.tsx`; route groups where justified | shared React components, feature modules, hooks, `public` |

These are examples, not a universal fixed folder template. TanStack owns route tree
generation; the exporter should not maintain that generated file by hand. Astro content
routes may need a content collection or `getStaticPaths`; SSR needs the appropriate runtime
adapter. SvelteKit client/server hooks are framework lifecycle files, not React hooks.
Pin framework versions and validate the scaffold against those versions.

Official references: [TanStack Start routing](https://tanstack.com/start/latest/docs/framework/react/guide/routing),
[Astro project structure](https://docs.astro.build/en/basics/project-structure/),
[SvelteKit project structure](https://svelte.dev/docs/kit/project-structure).

For a single React application, a proposed output could look like:

```text
project/
  package.json
  <lockfile>
  tsconfig.json
  <framework/build configuration>
  public/
    fonts/
    images/
  src/
    <native framework routes>
    components/
      ui/                    # shared accessible controls
      layout/                # header, footer, navigation, shells
    features/
      booking/
        components/
        hooks/               # only for reusable React state/behavior
        data/
        types.ts
    hooks/                   # cross-feature React hooks, when justified
    lib/                     # general utilities and integration clients
    styles/
      tokens.css
      fonts.css
      globals.css
  tests/
    navigation/
    interactions/
  .mochi/
    manifest.json            # route/asset maps and reconstruction limitations
```

Only create directories that contain useful code. Keep routes thin, colocate feature
logic, and extract shared controls when repetition or behavior justifies it. Use
`apps/web` and shared `packages/ui`/`packages/tokens` only when multiple consumers warrant
a workspace. For an existing target repo, discover its configuration and follow its
established conventions.

## Open-source comparisons

The requested `calcom/cal.com` URL currently redirects to `calcom/cal.diy`. These findings
describe that public repository, not a claim about Cal.com's private production source.

- [Cal apps/web](https://github.com/calcom/cal.diy/tree/main/apps/web) contains both `app/`
  and `pages/`, plus `components/`, `modules/`, `lib/`, `styles/`, and `fonts/`.
  [App routes](https://github.com/calcom/cal.diy/tree/main/apps/web/app) include route groups,
  a root layout, error boundaries, and `reschedule/[uid]`.
- [Cal packages](https://github.com/calcom/cal.diy/tree/main/packages) separate `features`,
  `ui`, `coss-ui`, `lib`, and other concerns. [Coss UI's manifest](https://github.com/calcom/cal.diy/blob/main/packages/coss-ui/package.json)
  exports components, hooks, shared code, and styles and depends on `@base-ui/react`.
  [The older UI manifest](https://github.com/calcom/cal.diy/blob/main/packages/ui/package.json)
  still lists Radix primitives. A large repo can contain several generations of a design system.
- [Immich web/src](https://github.com/immich-app/immich/tree/main/web/src) uses SvelteKit
  `routes/`, lifecycle hook files, and `app.css`. Its
  [lib directory](https://github.com/immich-app/immich/tree/main/web/src/lib) contains
  components, elements, stores, queries, services, actions, and utilities.
- [Starlight's source](https://github.com/withastro/starlight/tree/main/packages/starlight/src)
  separates components, internal components, integrations, routes, style, translations,
  and utilities. Its library routes are an integration concern; an ordinary generated
  Astro app should follow Astro's `src/pages` convention.

The lesson is to separate route entry points, reusable controls, feature behavior, and
design tokens while respecting the target framework. Folder names alone do not make an
output production-ready.

## Primitives

[Base UI](https://base-ui.com/react/overview/about) supplies unstyled accessible React
behavior. [shadcn/ui](https://ui.shadcn.com/docs) distributes editable component source.
They serve different layers and should not be treated as mutually exclusive choices.

For React output, prefer Base UI behavior for dialogs, menus, popovers, selects, tabs,
and similar controls, wrapped in project-owned `components/ui` code. A compatible shadcn
source distribution can provide those wrappers, but its default appearance must be
adapted to the captured site's tokens. Use semantic HTML for straightforward content.
Do not replace a site's design with the default shadcn theme.

Base UI React components cannot be imported directly into Svelte components. SvelteKit
needs native accessible controls or a separately validated Svelte primitive adapter.
Astro can use static native markup and hydrate React islands for controls that need them.
Each interactive primitive needs keyboard, focus, labeling, and visual-state checks.

## Fonts, tokens, and CSS

Capture styles before generating abstractions. Preserve source CSS variables when their
meaning is observable; otherwise propose tokens from repeated values and label them as
inferred. Keep colors, typography, spacing, radii, shadows, motion, breakpoints, and theme
variants in one token model. Do not flatten responsive rules into a single screenshot's
computed styles.

Font records should retain family, weight/range, style, stretch, unicode range, variable
axes, fallback stack, and `font-display`. Localize available font files with appropriate
reuse rights; record unresolved remote/licensed fonts rather than silently substituting
them. Cal's [fonts directory](https://github.com/calcom/cal.diy/tree/main/apps/web/fonts)
contains a local `CalSans-SemiBold.woff2`, illustrating why asset capture matters as much
as copying a `font-family` string. Wait for fonts before visual comparisons.

Recommended initial source-export styling choices:

- CSS variables for shared tokens plus plain/scoped CSS or CSS Modules where the
  framework supports them. This is the broadest common foundation across the targets.
- An optional Tailwind adapter that emits utilities backed by the same token model.
  Preserve arbitrary values where needed for fidelity; Tailwind is not currently used
  as a generation engine by this repository.
- A StyleX adapter for validated combinations, initially React/TanStack Start. Emit
  `stylex.create`, `stylex.props`, named `defineVars` exports in `*.stylex.ts`, theme
  overrides, and the compiler/bundler configuration. Validate CSS extraction and SSR.
  Do not claim Svelte/Astro parity without integration fixtures.

[StyleX](https://stylexjs.com/docs/learn/) compiles to static atomic CSS and can require
extra configuration for custom component formats such as Svelte. Its
[variable APIs](https://stylexjs.com/docs/learn/theming/defining-variables/) provide a token
backend. Existing compiled StyleX CSS can be mirrored today; reconstructing original
StyleX source or generating a working StyleX project is not implemented. Shadcn wrappers
that contain Tailwind classes would require restyling for a StyleX-only output.

## Delivery order and acceptance

Start with one complete source exporter, preferably TanStack Start with React and Base UI,
then add Astro and SvelteKit through the same model. Support a narrow, explicit component
and interaction set before expanding it. Add StyleX after the source/style interfaces are
stable, or make it the first React styling adapter if that is the selected product scope.

For every supported combination, keep a pinned fixture covering nested and dynamic routes,
shared layouts, fonts, responsive/theme states, and interactive controls. A generated
project must install from its lockfile, typecheck, build, and run. Check direct route loads,
client navigation, asset resolution, hydration, focus/keyboard behavior, and viewport
screenshots. Report missing integrations and visual differences. Renaming captured HTML
to TSX or creating empty folder scaffolds does not meet this bar.
