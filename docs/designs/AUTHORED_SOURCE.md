# From reconstruction to maintainable application source

Status: bounded semantic specification, validation and authored emission implemented; automated semantic inference and full-site controller replacement remain unfinished. See [authored export](../AUTHORED_EXPORT.md). A passing screenshot is not a source-quality qualification.

## What went wrong

The previous trials optimized fidelity by retaining deployed programs. Pretty-printing those programs preserves motion but does not restore names, domain types, component boundaries, or a design system. The exporter used virtual routes with opaque filenames and called captured global styles CSS Modules. Its Base UI integration only wrapped observed buttons. Those are compatibility exports, not production-style authored code.

## Reference repositories

Inspected 2026-11-08:

- [Midday website](https://github.com/midday-ai/midday/tree/main/apps/website/src): the landing app separates components, data, hooks, library code, and styles. Reconsulted for the section/graphics split.
- [Midday dashboard](https://github.com/midday-ai/midday/tree/main/apps/dashboard/src): routes in `app`, components, hooks, styles, types, and library code have separate ownership. [Shared UI](https://github.com/midday-ai/midday/tree/main/packages/ui/src) lives in a package; its [global stylesheet](https://github.com/midday-ai/midday/blob/main/packages/ui/src/globals.css) defines semantic variables and theme values.
- [Cal.diy web](https://github.com/calcom/cal.diy/tree/main/apps/web): framework routes, components, modules, fonts, and styles are separate. [Packages](https://github.com/calcom/cal.diy/tree/main/packages) include features, UI, and coss-ui.
- [TanStack physical routes](https://tanstack.com/router/latest/docs/framework/react/routing/file-based-routing): use the target framework's conventions, not Next.js `app` directories copied from those references.

Use these ownership boundaries, not their entire monorepo topology or backend. A standalone recreation has one app. Extract `packages/ui` only when multiple apps actually share it.

## Target structure

```text
site/
  src/
    routes/
      __root.tsx
      index.tsx
      models/
        index.tsx
        mai-cyber-1-1-flash/index.tsx
    features/
      home/
        home-page.tsx
        hero.tsx
        hero-scene.tsx
        use-hero-scroll.ts
        home-content.ts
        home.module.css
      models/
        models-page.tsx
        model-card.tsx
        model-data.ts
        models.module.css
    components/
      site-header.tsx
      site-footer.tsx
      ui/                 # Base UI primitive exports only
      compositions/       # Styled navigation, forms, dialogs
        site-navigation.tsx
        contact-form.tsx
        details-dialog.tsx
    styles/
      tokens.css
      fonts.css
      globals.css
    lib/
    types/
    assets/
      fonts/
      images/
      shaders/
  .mochi/                 # captures, receipts, reports, comparisons, intermediates
  .output/               # Bun production bundle
  .tanstack/             # router/build cache
  bun.lock
  package.json
```

Create only modules needed by observed features. Do not scaffold empty hooks, database, auth, server actions, or fake APIs.

The implemented transitional exporter uses `features/pages/<literal path>/{page.tsx,data.ts}`, `styles/compat`, and `compat/recovered`. These remain visible, versioned runtime inputs. Moving minified code into a hidden ignored directory would break a clean checkout and conceal the remaining work.

## Decisions

| Area | Decision | Current status |
| --- | --- | --- |
| Runtime | Bun 1.4.4, frozen `bun.lock`, Bun-powered Vite/checks and Nitro Bun server | Implemented; runtime tests required |
| Routes | Physical TanStack files, thin adapters to feature pages; literal segments escaped; do not infer `$id` from two similar URLs | Implemented |
| Layouts | Shared layout only with verified content/behavior ownership; path nesting does not prove a layout | Existing structural candidates only |
| Components | Semantic names, explicit typed props, readable JSX, data separated from rendering | Reviewed semantic specs emit named typed components; automatic extraction pending |
| UI | Base UI for matching behavior; plain native HTML for semantic content | Authored path imports direct Base UI parts in reusable compositions; navigation/menu/dialog/form fixture tested |
| Styles | Semantic custom properties, font declarations, genuinely local CSS Modules; global CSS for reset/document defaults | Authored path emits local CSS Modules, scoped semantic tokens and local fonts; compatibility CSS retained only on compatibility exports |
| Motion | Preserve SVG/WebGL algorithms as named feature modules/hooks with lifecycle cleanup | Motion reveal effects, reduced-motion provider and frame scheduling implemented. Owned module import graphs and GLSL emission implemented. Natural tree renderer and Garden geometry migrated; remaining controllers still need translation |
| Uncertainty | Emit explicit reconstruction debt and evidence references; never advertise semantic quality from formatting | Export quality status and editing guide implemented |
| Artifacts | Ignored `.mochi/`; versioned application source/assets/lockfile; hidden framework bundles/caches | Implemented |
| Generated edits | Never overwrite an existing destination. Reconcile later regenerations through reviewed diffs | Existing refusal retained |

## Reconstruction pipeline

1. **Evidence:** keep DOM, CSSOM, font files, rendered geometry, module source, route URLs, and sampled interaction/scroll states in `.mochi/`. A screenshot alone is insufficient.
2. **Deterministic analysis:** parse source/DOM/CSS; build import, binding, state/effect, selector-consumer and route graphs. Deduplicate exact subtrees/styles. Establish responsive/theme variants before grouping. Preserve provenance for every decision.
3. **Semantic proposal:** propose feature boundaries, meaningful names, typed data, reusable component props, state machines, and token roles from evidence. This is synthesis, not recovering the original private repository. Unknown behavior stays explicitly unknown.
4. **Validated intermediate representation:** routes, layouts, components, data, interactions, motion, style scopes, fonts, and token references must be a bounded schema. Validate references, ownership, route collisions, and generated-source dependencies before emission.
5. **Authored emission:** framework adapters generate native route files; React emission uses explicit JSX/props and Base UI assemblies. Styling adapters consume the same token/style representation. No deployed bundle belongs on an authored route's dependency graph.
6. **Replacement and comparison:** replace one feature at a time, then exercise the old and new behavior across viewport, keyboard, theme, menu, scroll, and animation states. Keep the compatibility baseline for comparisons until replacement passes.
7. **Edit qualification:** edit text/data, change a semantic token, add a route, and alter an interaction. Build, typecheck, and verify actual SSR/hydrated output. A developer must be able to complete these without decoding a bundled module.

AI can help at step 3 and generate candidate code at step 5, but cannot certify its own output. Jev is not selected as the code author. The existing Jev adapter produces review-required interpretation annotations; it does not currently generate maintainable application source. A future model choice needs qualification against the schema and the edit/behavior suite. No paid calls are implied or performed by this change.

## Base UI assembly contract

- Navigation: `NavigationMenu.Root`, `List`, `Item`, `Trigger`, `Portal`, `Positioner`, `Popup`, `Content`, and `Link`. Compose captured local destinations with TanStack `Link` through `render`. Distinguish navigation from an action menu.
- Actions: `Menu.Root`, `Trigger`, `Portal`, `Positioner`, `Popup`, `Item` (plus groups/separators when observed).
- Modal: `Dialog.Root`, `Trigger`, `Portal`, `Backdrop`, `Popup`, `Title`, `Description`, and `Close`.
- Forms: `Form`, `Field.Root`, `Field.Label`, `Field.Control`, `Field.Error`; use the appropriate Input, Select, Checkbox, RadioGroup, or Switch primitive.
- Retain observed controlled/open/selected state explicitly. Remove the old controller when its replacement becomes authoritative; two controllers must not own the same element.
- Preserve accessible names, focus entry/return, keyboard navigation, Escape/outside dismissal, and disabled behavior. Style focus-visible, highlighted, checked, open, invalid and disabled states.
- Use `render` for composition; function renderers must use `mergeProps`. Portals must be styled in their actual document context. Base UI supplies behavior, not a visual theme.

## Styling reconstruction

Build a symbol table for custom properties and a selector-consumer graph. Separate literal design values from layout results and animation state. Keep scoped definitions and media/theme overrides attached to their context. Equal computed pixels do not prove a common token: `16px` can mean unrelated body type, spacing, or an icon.

Recover font family, exact local file, weight/style range, variable axes, unicode range, display policy, fallback metrics and usage. Preserve SVG/chart/shader palettes as feature values unless they demonstrably belong to a shared theme.

Produce reference values (color/space/type/radius/motion), semantic roles (surface/text/action/border/focus), then component aliases where needed. Migrate consumers to named tokens before removing captured definitions. Validate responsive geometry and theme state after each consolidation. Do not flatten dark mode or responsive values into one root table.

Tailwind v4 is the default authored styling backend. The Vite plugin compiles semantic theme aliases, literal utility classes and scoped CSS Module `@apply` rules. Measured graphics/layout selectors remain CSS Modules. StyleX support is not implemented. A captured stylesheet does not reveal which library originally produced it.

On compatibility exports, `tokens.css` provides an explicit owner-editable override surface, not an automatically inferred token system. Authored exports populate it from reviewed semantic token definitions and retain declared theme/media scopes. `.mochi/style-audit.json` is evidence, not token recommendations. Identical captured stylesheets are deduplicated without changing selector order or cascade semantics.

## Completion gates

An authored-source claim requires: no compatibility bundles reachable from the converted route; no generic `prop_N`/opaque state records at public component boundaries; framework-native routability; real Base UI state/focus behavior; a consistent scoped token/font system; build and typecheck; observed behavior/visual checks; and successful edit probes. Compatibility exports do not meet these gates. Authored exports are labeled unqualified candidates until independent behavior, visual and edit checks pass. The authored UI fixture covers behavior and edits; the first real Microsoft models candidate does not establish full-site parity.
