# Authored source export

`mochi export --authored-spec` emits a new TanStack Start project from a reviewed,
evidence-bound semantic specification. It generates application code, rather than
mounting a captured DOM tree or executing recovered bundles. The specification
requires interpretation: `plan-source` collects evidence but does **not** infer a
complete application. The Jev classifier is not a code author and is not invoked.

```sh
mochi plan-source .mochi/captures/site -o .mochi/site-plan.json
# Interpret the evidence and write a Specification (schema: src/authored/schema.rs).
mochi export .mochi/captures/site \
  --authored-spec .mochi/site-authored.json -o .mochi/projects/site
cd .mochi/projects/site
bun install --frozen-lockfile
bun run build
bun run check
bun start
```

The plan records the current input fingerprint, heading/control references, routes,
and structural candidates. A specification must use that fingerprint and valid
observation/node references. The plan is deliberately rejected as an export
specification. Existing output directories and plan files are never overwritten.

## What the specification describes

- Meaningful named components, feature ownership, purpose, and source references.
- Typed properties (`text`, `number`, `boolean`, `href`, `asset`, lists and objects).
  Typed route content is separate from component rendering.
- Native semantic elements and direct `@base-ui/react` parts. There is no blanket
  primitive wrapper package. A reusable composition such as `SiteNavigation` lives
  in `src/components/compositions/site-navigation.tsx` and imports the transparent
  Base UI `NavigationMenu` export from `components/ui/navigation-menu.tsx`.
  Only used primitives are emitted; owned source modules cannot occupy `components/ui/`.
- Repeated typed content with explicit stable keys, references, and component reuse.
- Explicit scalar React state and supported state transitions. For example, a
  controlled dialog connects `open` and `onOpenChange` to a named boolean state.
  Local form submission must prevent default; an external form action must be explicit.
- Tailwind v4 through its Vite plugin, semantic `@theme inline` aliases, scoped
  CSS Modules with Tailwind `@apply` for common layout utilities, root/theme/media token scopes, and
  named local font assets with weight, style, display and optional Unicode range.
- Known reconstruction gaps, retained in the generated report and editing guide.

The initial primitive set covers NavigationMenu, Menu, Dialog, Field, Form, Input,
Button, Switch and Tabs. Owned modules also support Accordion. It is an explicit subset of Base UI's APIs. Add supported
parts and semantic expressions as observed behavior needs them; do not silently
fall back to executing an old controller. The semantic tree has no arbitrary
JavaScript expressions, raw HTML, or inline style bags. Custom algorithms use
explicit reviewed source modules described below.

See [the owned UI fixture](../tests/fixtures/authored/ui.json) for navigation, action
menu, controlled dialog and form compositions. Its fingerprint and evidence IDs
are placeholders filled by the test fixture; it is not an exportable real-site plan.

## Generated ownership

```text
src/
  routes/{__root,index}.tsx
  routes/models/index.tsx
  components/layout/{site-header,site-footer}.tsx
  components/ui/{navigation-menu,dialog,switch,tabs}.tsx
  components/compositions/{desktop-navigation,mobile-navigation,model-navigation}.tsx
  components/providers/motion-preference.tsx
  features/models/models-page.tsx
  features/models/models-content.ts
  features/models/sections/{models-introduction,models-catalog}.tsx
  features/models/components/model-card.tsx
  features/hero/components/tree-background.tsx
  features/hero/hooks/use-tree-scene.ts
  features/hero/graphics/{tree-scene,branch-geometry,wind}.ts
  assets/fonts/<font-name>.woff2
  assets/images/<asset-name>.webp
  assets/shaders/{branch-wind,leaf-curvature}.glsl
  hooks/{use-scroll-header,use-scroll-reveal}.ts
  styles/{tokens,fonts,globals}.css
  lib/assets.ts
bun.lock
EDITING.md
.mochi/{authored-specification,authored-report}.json
```

Only files used by the specification are emitted. Components use adjacent CSS
Modules. `feature: "layout"` selects `components/layout`; `feature: "ui"` selects
`components/compositions` (also selected by `feature: "compositions"`); other features go under `features/<name>`. An optional `group`
selects a subdirectory such as `sections` or `components`. Route content defaults
to a readable page name (`models-content.ts`); `data_name` disambiguates multiple
routes sharing a page component. Route files remain TanStack file routes and Vite
performs route splitting. Imported passive assets use semantic filenames in source;
Vite fingerprints production URLs during the build.

## Owned graphics and algorithms

`source_modules` maps paths relative to `src` to `{ file, purpose, evidence }`.
`file` is a UTF-8 source file relative to the specification directory. This is
reviewed application code, not a sandbox and not a way to relabel deployed bundles.
Each external component is a named export with an explicit typed prop contract:

```json
{
  "source_modules": {
    "features/hero/components/tree-background.tsx": {
      "file": "source/tree-background.tsx",
      "purpose": "Owns the canvas and scene lifecycle",
      "evidence": [{ "observation": "<captured observation>", "node": 94 }]
    }
  },
  "external_components": {
    "TreeBackground": {
      "module": "features/hero/components/tree-background.tsx",
      "props": { "leafTexture": { "type": "asset" } }
    }
  },
  "libraries": ["three"]
}
```

Use a node with `kind: "external"`, the named export, and typed `props` to mount it.
Declare every imported local module; the complete graph must be reachable from
used entries. Ordinary relative and `@/` imports, literal lazy imports, CSS Modules,
named `assets/vectors/*.ts` geometry, global design definitions under `styles/`,
and `assets/shaders/*.glsl?raw` imports are supported. React, TanStack Router and
direct Base UI are available; opting into `three` pins Three and its types to
0.183.0 with a separate frozen Bun lock. Bundled framework runtimes, explicit
`any`, unresolved local imports, undeclared packages and unused modules are refused.
These checks are structural; source review and build/browser validation are still
required. Maximums: 128 modules, 64 external entries, 256 KiB per module, 4 MiB total.

Header scroll and intersection reveal behaviors emit owned hooks. A shared motion
provider honors system reduced motion and the persisted accessibility choice.
Custom scenes can import it to pause animation. Scene code must own resize,
visibility, requestAnimationFrame, context and GPU resource cleanup.


Edit generated TSX, typed content and CSS directly with `bun run dev`. The generated
specification is provenance, not a runtime interpreter; editing TSX does not require
changing that specification. Regeneration creates a separate project for comparison.
Source, imported assets and the lockfile belong in version control. Build output,
capture evidence and reports remain in ignored dot directories.

## Validation and limits

Validation checks evidence hashes, literal route names, asset availability, prop
shapes, references, component cycles/reachability, duplicate repeat keys, bounded
render expansion, supported handlers, basic Base UI anatomy, CSS class definitions,
and token references/cycles per declared scope. Generated TypeScript is parsed
before publication. Build/typecheck and browser qualification remain separate;
validation is not a complete TypeScript, CSS, accessibility or behavioral proof.

Bounds: 8 MiB specification, 128 components/routes, 8,192 declared nodes, 100,000
expanded nodes, 512 items per list, 256 assets/tokens, 64 fonts, 32 token scopes,
32 MiB emitted source, 16 MiB per passive asset and 256 MiB aggregate assets.
Copied asset hashes and source evidence are rechecked before atomic publication.
Only captured literal paths without query state are supported by this initial
adapter. Original destinations outside authored routes remain explicit links.

The emitter does not automatically infer custom algorithms, backend APIs, dynamic
route parameters or source-specific consent behavior. Reviewed typed hooks and
shader modules provide the explicit implementation path for custom graphics. Tailwind v4 is the default styling backend, with CSS Modules for measured selectors and graphics. StyleX is not implemented. Base UI styles must be supplied by the
specification. Scoped token sets are reviewed semantic choices, not automatic
inferences from equal computed pixels.

The report always starts with `verification: not_run` and
`source_quality: authored_candidate_requires_behavior_and_visual_qualification`.
No compatibility controller is emitted, even when declared gaps remain. Keep the
compatibility baseline available until each converted feature passes its visual,
interaction and edit checks.

## Tests

`cargo test --test authored` checks successful emission and rejects stale evidence,
wrong props, cycles, duplicate keys, incomplete UI anatomy, unsupported HTML,
unknown/cyclic tokens and unsafe local form submission. It also verifies local
asset imports, readable content names, owned shader import graphs, missing exports,
unused modules, explicit-any rejection, and that evidence plans are not treated as
application programs.

`cargo test --test authored_browser -- --ignored` installs the frozen lockfile with
Bun, edits emitted content and a token, builds, typechecks, runs production SSR and
checks real Base UI keyboard menu activation, controlled dialog naming/Escape/focus
restoration, router links, field labeling, native validation and local submission.
These are owned behavior fixtures, not arbitrary-site visual qualification.

## Starter and animation contract

Every new project uses Bun, Base UI, Tailwind v4 and Motion. The shared Vite template
installs Tailwind's plugin; authored projects include Preflight, while compatibility
projects load theme/utilities without overriding captured resets. `styles/tailwind.css`
defines semantic aliases. Local font files and scoped token values remain separate.

`components/ui` is reserved for transparent exports from `@base-ui/react`. These
retain Base UI's multipart API and types. Compositions own styling, content and state.
Motion implements reveal effects and frame scheduling; `MotionConfig` follows the
shared system/user reduced-motion preference. Custom WebGL scenes retain explicit
resource ownership and cleanup. Motion is an approved owned-source import.

The Natural, Infinite Garden, and Microsoft AI source packs use owned modules for
their nine captured routes, including their graphics and interactions. The CLI
exports these reviewed inputs without recovered controllers. These local source
packs are qualification inputs, not bundled site templates or proof of automatic
semantic interpretation. Follow the [deterministic qualification method](DETERMINISTIC_RECONSTRUCTION.md)
before treating an authored candidate as a faithful reconstruction.

References: [Base UI setup](https://base-ui.com/react/overview/quick-start),
[Tailwind Vite integration](https://tailwindcss.com/docs/installation/using-vite),
[Motion animation controls](https://motion.dev/docs/animate).

## Reviewed local artwork

`source_assets` declares passive assets used directly by owned modules or styles.
Each key is a descriptive path under `assets/images`, `assets/vectors`,
`assets/fonts`, or `assets/videos`. Each entry supplies `file`, `sha256`, `purpose`,
and `evidence`. File paths are relative to the specification. PNG, JPEG, WebP,
AVIF, GIF, SVG, ICO, WOFF/WOFF2, TTF/OTF, MP4 and WebM are accepted. Assets are copied
byte-for-byte, checked against their declared SHA-256, and rechecked before atomic
publication. Files must be reachable through a declared source import or local CSS
`url()`; unused assets, remote CSS URLs, traversal, and hash mismatches are refused.
The budgets are 256 local assets, 16 MiB per asset and 256 MiB in total.

This supports reviewed artwork recovered from an SVG or shader, and media retrieved
during interpretation that was absent from the original capture. Evidence references
record the observed source context; they do not prove the author's reconstruction is
visually equivalent. The source report records asset hashes alongside module hashes.

Component names, source paths, asset paths, feature groups and content filenames
reject compiler identities such as `Client72b310ddae8ef28e`, `Module123`,
`Component19` and hash-suffixed names. Descriptive route slugs and names such as
`ClientLogos` and `WebGL2Scene` remain valid. Production bundlers still fingerprint
built filenames inside `.output`; that is separate from editable source naming.

## Explicit compatibility migration

An export without `--authored-spec` now stops with instructions to prepare semantic
source. To intentionally obtain the older structural replay output, pass
`--compatibility`, including with `--interpretation` or `--recover-client-route`.
Compatibility output remains a diagnostic baseline and does not claim semantic
conversion. Existing projects are never silently rewritten.
