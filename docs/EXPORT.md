# TanStack Start source export

The development checkout implements an offline source emitter, separate from the deployed-file
mirror. It generates actual editable JSX and native TanStack route files from a saved project
model; it does not embed a DOM-JSON interpreter or replay original JavaScript bundles.

```sh
mochi clone https://example.com -o captured --source-evidence --max-pages 20
mochi check-evidence captured
mochi model-project captured
mochi export captured --compatibility -o reconstructed --framework tanstack-start
cd reconstructed
bun install --frozen-lockfile
bun run build
bun run check
bun start
```

The output parent must exist, and the project directory must be new and separate from the
source clone. Build once before typechecking so TanStack generates its route tree. Generated
README/report files describe the commands and captured paths. Ordinary `clone`, `analyze`,
and mirror editing retain their existing behavior. Registry version 0.2.0 predates this work.

## Recovering client behavior as editable source

```sh
mochi export captured --compatibility -o reconstructed --recover-client-route /
```

The opt-in adapters recover public React client components from supported saved
Next/Turbopack or split Vite/TanStack build output. The Next adapter reads captured Flight
component records and the route's observed chunk inventory, including module preloads.
They do not fetch source, call a model, execute the
captured code during export, inject original script tags, or replay a bundler runtime.
The generated application **does execute the recovered client code** when built and opened.

The Rust parser resolves module-parameter bindings, follows static imports, preserves their
order, lowers live export getters into ordinary source modules, and formats the result.
Known React/JSX/server-renderer modules bridge to the project's pinned React packages.
Actual drawing algorithms, scroll listeners, observers, animation code and cleanup functions
stay in `src/compat/recovered/Module<hash>.js`. A typed component wrapper and captured data props
live in `src/components/Client<hash>.tsx`; existing route files, compatibility CSS and asset folders
remain in place. Declaration files describe module boundaries without claiming recovered
TypeScript types. Names from minified source may remain short; this is recovery from public
build output, not restoration of the original repository's names or file boundaries.

For split Vite routes, the adapter reads the captured TanStack route manifest and literal
SSR loader data without evaluating its scripts. Binding references determine the required
declarations and their imports; prototype/property writes retain source order. Unrelated
bootstrap, authentication and telemetry startup are excluded from the selected slice.
Recovered files use `Esm<hash>.js` names. Original pathless layouts compose through a local
outlet context; public router hooks bridge to the generated project's router. The supported
producer factories require React 19.2.7, so that adapter selects a separately pinned lockfile
with matching React and ReactDOM. Unsupported namespace imports, missing exports, ambiguous
loader data or route components fail explicitly. This is a narrow adapter for that observed
build format, not arbitrary Vite or framework support. Select multiple paths by repeating
`--recover-client-route /pay --recover-client-route /request`.

For native custom-element sites, a separate adapter emits JSX from pristine mirrored
server markup, before client text splitting or canvas insertion. It parses and formats
the native ESM graph, rewrites literal imports to generated source modules and retains
module initialization. This adapter preserves whole modules rather than the React
adapter's binding slices; vendor code and short producer names can remain. The component
waits for document load and hydration before importing its controllers. Native anchors
reload documents so global registrations and listeners receive a fresh document lifecycle.
It does not inject the original script tags or attach captured shadow roots over elements
whose own code creates them. Inline scripts, native handler attributes and embedded frames
are omitted from the server-markup path.

Nonliteral module loads require a finite set of same-origin `.js` targets declared in the
selected server markup's data attributes. Those targets become explicit native import
branches; other values reject rather than fetching arbitrary code. Query-bearing modules
require an exact full-URL response saved during capture, never an ambiguous mirror path.
Source-evidence clones preserve up to 512 module responses, 16 MiB each / 128 MiB total,
under `site/.mochi-source/<url-hash>.blob` before mirror URL edits. These are source receipts,
not additional scripts in the generated public runtime. Ordinary mirror clones do not
write these receipts. Literal ESM imports are discovered with the JavaScript parser during
download, including lazy modules that were not requested in the initial viewport.

Recorded redirects in the clone manifest map native anchors to captured canonical routes,
including directory URLs with trailing slashes. Already-localized asset paths remain local
when passed through the native attribute adapter. External custom elements without recovered
controllers are omitted only if consistently nonvisual in every selected observation;
registered hidden UI remains available. Omitted tag names are recorded in recovery notes.

Capture can read a loaded cross-origin stylesheet through Chrome's CSS domain when page
CSSOM access is denied. The text is reparsed without attaching another stylesheet or
refetching it; `text_source: devtools_loaded_stylesheet` records that path. Existing
byte/rule/depth limits still apply, and unsupported imports remain incomplete evidence.
CSS export also preserves validated base64 WOFF/WOFF2/TTF/OTF data URLs.

This adapter handles captured same-origin entry graphs. External platform scripts and
private backend services are not inferred. Generated module receipts, production builds,
interaction checks and scroll/viewport comparisons remain necessary for each actual site.

Select each literal path explicitly. A selected path must have one captured route state,
one unambiguous default client root with data-only props, and a completely lowerable static
dependency graph. Unknown ABI calls, deferred/conditional imports, cyclic dependencies,
missing chunks, conflicting registrations, unsupported CommonJS modules and encoded server
references are refused. Length-framed Flight text is never interpreted as component records.
No screenshot fallback is substituted for a requested recovery. Unselected routes retain
the initial-state emitter, and `verification` remains `not_run` until external checks run.

Every consumed chunk and Flight file has a SHA-256 receipt in `client_recovery.sources` and
is read again before atomic publication. Source inputs remain beneath the clone root.
Recovery reads at most 16 MiB per file / 128 MiB total and 4 MiB per factory, accepts at most 8,192 registrations
and 128 active dependency levels, and emits at most 512 recovered files / 128 MiB recovered source. Aliases share one parsed factory while retaining separate live export namespaces. Formatting rejects excessively deep/large syntax trees. Existing export
text/asset budgets and refusal to overwrite destinations still apply. Static image/font URL
literals with authoritative local mappings are localized. Original backend services,
private source maps, arbitrary framework runtimes and unsupported module formats are not
recreated by this adapter.

Recovered routes prefer the hashed, saved original stylesheet text over CSSOM serialization.
CSSOM can discard variable-based animation shorthands, leaving entrance elements invisible.
Supported URL localization still applies to those stylesheets. Passive image/font files also
retain their original public paths when unambiguous, within the unchanged asset budget, for
URLs assembled dynamically by recovered code. Scripts and API payloads are not published
through that path alias mechanism.

The decision is to preserve available executable algorithms before attempting behavioral
inference from screenshots. Inference still needs a separate evidence/validation design for
sites whose client code cannot be recovered through a supported adapter. Neither strategy
can be qualified by comparing only the initial viewport.

## Generated source

```text
reconstructed/
  package.json, bun.lock, tsconfig.json, vite.config.ts, .bun-version
  EDITING.md
  .mochi/export.json, .mochi/style-audit.json
  public/_mochi/assets/<content-hash>.<extension>
  src/
    router.tsx
    routes/__root.tsx
    routes/index.tsx
    routes/products/item/index.tsx
    features/pages/page.tsx, data.ts
    features/pages/products/item/page.tsx, data.ts
    components/ComponentN.tsx       # structural candidates, not semantic recovery
    components/ui/button.tsx       # Base UI normal React props
    compat/recovered/              # versioned compatibility runtime programs
    layouts/LayoutN.tsx            # evidenced shared shells
    styles/compat/<capture-id>.css # captured global cascades, exact deduplication
    styles/tokens.css              # owner-authored overrides, not inferred tokens
    lib/runtime.tsx
    lib/references.ts
```

Physical TanStack route files delegate to feature pages and data. Adding a physical route
regenerates the router tree on build. Index files preserve literal paths without inventing
parent layouts; reserved filename syntax is escaped. Reusable candidates expose mechanical
typed props, but semantic names and component ownership remain unreconstructed. Captured
client programs live in the explicit compatibility boundary. No empty hooks or backend
scaffolding is created. See [authored-source decisions](designs/AUTHORED_SOURCE.md).

Captured query order, repeated keys
and encoding spellings are preserved through an opaque query adapter. Because Start also
normalizes its SSR history URL, source-state selection reads the untouched Request on the
server and the opaque router location on the client. Canonical cache keys align hydration;
every navigation reloads its exact literal state. Unknown paths/query states return 404.
Captured internal anchors support client navigation while keeping the literal URL and fragment.

Native buttons use project-owned Base UI wrappers. Other supported native inputs retain
their observed uncontrolled defaults, including live text, checkbox and single-select values.
Original handlers, executable/embedded elements and submission destinations are omitted;
form submission is prevented. Application actions and backend behavior require implementation.
These are initial-state reconstructions, not recovered original hooks or business logic.

## CSS and assets

The first CSS backend preserves readable CSSOM rules in their recorded order, including
media/supports groups, variables, font-face descriptors and pseudo rules. Per-document global
compatibility CSS attaches through the route head, so another
document's cascade is removed on navigation. This deliberately preserves source selectors;
component-local selector rewriting and semantic token naming are not implemented. The
computed-style snapshots remain evidence, not invented responsive breakpoints. Existing
inline styles use a shared attribute adapter and are flagged for review.

Passive local candidate image/font files are copied by content hash after checking their
recorded bytes/hash. URLs in supported CSS and JSX attributes are resolved against the
recorded base URL and localized where a candidate is available. SVG namespaces and common
namespaced attributes are preserved. Original scripts and API responses are not copied into
the new application's runtime. Ambiguous query-based mirror assets and external/unavailable
resources remain explicit diagnostics and may still require the original host.

This backend emits ordinary CSS, even if the deployed website originally used StyleX,
Tailwind or CSS-in-JS. It does not recover those source APIs. Font availability and pixel
parity must be checked in the generated app; font declarations alone do not establish them.

## Shadow DOM, frames and rendered surfaces

Source capture now records open shadow trees and instruments newly created closed roots.
The exporter emits their actual JSX and scoped stylesheet text, attaches the shadow boundary,
and preserves light children/slot distribution. Next.js route-announcer content is captured;
it no longer blocks export merely because it uses a shadow root.

Readable same-origin frames become sandboxed `srcDoc` documents built from validated nodes,
styles and recorded initial values. Scripts, handler attributes, form destinations and
executable URLs are excluded. Nonvisual runtime frames are reported and omitted. Opaque
frames, canvas and video surfaces have an explicit image-based initial-state representation.
Canvas/WebGL scenes live in editable `CapturedSurface` components with replaceable assets;
they are not recovered drawing programs or original animation/input handlers.

Canvas snapshots read the drawing surface's PNG pixels directly, with WebGL drawing buffers
preserved during capture. This avoids baking overlapping DOM text into the canvas asset.
A browser-side isolated raster path handles surfaces whose pixels cannot be read directly;
it hides unrelated content temporarily and restores the original styles/background.
The generated canvas component paints the frame into its native 2D bitmap after loading;
CSS background images are used only as an initial placeholder. Captured pixel files have hashes, are checked again during modeling/export, and are localized
under `public/_mochi/assets`. Per-surface capture is bounded to 16,777,216 CSS pixels/64 MiB PNG.

Computed maps remain lossless but use interned strings on disk; existing uncompressed maps
remain readable. No evidence-byte limit was raised. Each viewport loads a fresh document
at its target dimensions, eagerly loads observed images, and retains the capture browser
for longer native processing intervals. The entry page is probed for WebGL requests: software WebGL is enabled when needed,
while ordinary 2D pages retain the default backend. The selected backend is recorded in
the clone manifest. Source capture hides native scrollbars, matching the browser verifier;
`hide_scrollbars` records that profile choice. These choices are recorded behavior, not claims about every GPU or website.

Inline style priorities are preserved on mount. Computed animation declarations are emitted
when a named animation is observed, compensating for CSSOM serialization that can lose
variable-based animation shorthands. Observed script-created animation effects retain their sampled styles, so a completed
fade does not revert to an invisible stylesheet default. Finite top-document entrance
effects are allowed up to eight seconds to settle; renderer frame callbacks are then held
while surface pixels and reference screenshots are collected. Page-wide original JavaScript
is still excluded.

## Authored source mode

For named typed components and direct Base UI compositions without recovered controllers, use [`--authored-spec`](AUTHORED_EXPORT.md). This separate path consumes a reviewed semantic specification; it does not automatically infer one or use the Jev classifier as a code generator. The compatibility behavior and limits below apply to the default export path.

## Refusals and bounds

- Evidence must be complete, and the saved model must match the current capture inventory,
  screenshots and local candidate asset bytes. Export never refreshes or overwrites the source.
- One source origin is supported. Paths use explicit ASCII literal segments with letters,
  digits, `_`, `.`, `-` and `/`; parameter-like/encoded path syntax, doubled slashes and bare
  empty queries need an explicit mapping adapter. No dynamic parameters are inferred.
- Viewport-specific DOM/CSSOM is emitted as explicit variants. The runtime selects the
  nearest recorded width/height after hydration; SSR starts with the largest observation.
  This preserves sampled states without inventing intermediate breakpoint behavior.
- CSS `@import`, escaped/nested CSS URL syntax, unsupported tag/namespace
  syntax and multi-select state are refused. Supported CSS URL references use plain `url(...)`
  with unescaped quoted/unquoted contents. This is a conservative adapter, not a general CSS
  parser. Source-set attributes are omitted and reported.
- Existing model limits apply. Generation caps source text at 128 MiB, candidate asset copies
  at 256 MiB total/64 MiB per read, and model input at 128 MiB. Source/asset hashes are rechecked
  before publication. Limits fail the operation and discard staging output.
- On macOS/Linux, atomic no-replace directory rename refuses even a concurrently created
  empty destination. Existing files and symlinks are not overwritten. Other platforms are
  currently refused by the final publication step. This is not filesystem isolation against
  a process concurrently swapping local source entries.

`--interpretation` explicitly attaches a validated saved [semantic report](INTERPRETATION.md)
as review-required comments on component source and metadata in the export report. It makes
no model calls and does not change generated names, props, routes, CSS or control mappings.
Exports without this flag ignore interpretation artifacts, including invalid ones.

`.mochi/export.json` records the model fingerprint, captured paths, component source references,
diagnostics and current limits. Its verification field starts at `not_run`: writing files is
not a build or fidelity claim. `source_quality` explicitly reports compatibility reconstruction
with semantic refactoring required; formatter output does not establish maintainability. No network/package/model call occurs during `mochi export`.

## Pinned toolchain and qualification

The generated tuple uses Bun 1.4.4, React/React DOM 19.3.1, TanStack Start 1.169.5,
Router 1.170.46, Vite 8.3.6, React Vite plugin 6.2.1, TypeScript
7.0.2, Base UI 1.9.0 and Nitro **3.0.261103-rc**. The Nitro dependency is a pinned release
candidate. The emitted lockfile fixes the full dependency graph. The current production
target is Nitro's Bun server; no deployment or Cloudflare qualification is claimed.

The fixture test installs the emitted lockfile, builds, typechecks and starts production SSR.
It checks three literal states (two query states plus a nested path), source base-URL handling,
client navigation, fragments, repeated query keys, `%20`/`+`, unknown-state 404s, control values,
local image/font loading, SVG, keyboard activation/focus, injection literals and hydration errors.
It also edits a generated prop and a CSS token, adds an escaped physical route, rebuilds,
and verifies changed SSR output and desktop/mobile computed styles. Pixel comparison
uses the source captures at 375 and 1440 pixels: the preregistered threshold is 0.25% differing
pixels with a per-channel tolerance of 2. Validation results are recorded separately in
[VALIDATION.md](../VALIDATION.md); they do not establish arbitrary-site fidelity.

The scaffold follows the official [from-scratch setup](https://tanstack.com/start/latest/docs/framework/react/build-from-scratch),
[Nitro hosting guide](https://tanstack.com/start/latest/docs/framework/react/guide/hosting),
[physical file routing](https://tanstack.com/router/latest/docs/framework/react/routing/file-based-routing),
[environment functions](https://tanstack.com/start/latest/docs/framework/react/guide/environment-functions),
and [Base UI button API](https://base-ui.com/react/components/button), checked on 2026-11-08.
