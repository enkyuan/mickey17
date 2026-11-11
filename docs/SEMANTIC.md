# Semantic conversion: IR, Base UI patterns, emitters

`mochi semantic` turns **one captured page** into a report of every Base UI
pattern it can prove, and with `--emit` into typed TSX that uses direct Base UI
primitives only under `src/components/ui/`. It does not change `mochi export`.

```sh
mochi semantic .mochi/trials/<run>/natural/capture --out /new/dir --emit
#   report.json  report.md  project/src/{components,components/ui,data,types}/
```

`--page N` picks a manifest page, `--viewport 1440x900` the evidence file
(default: the widest), `--no-js` ignores the original bundles. The output
directory must not exist. Emitted files are a project **fragment**, not a
runnable app: merge them into an exported project (`@/` alias, React 19,
`@base-ui/react` 1.9.0).

## Pipeline (all deterministic, AST/IR based; no regex or string patching)

| Stage | Module | Evidence |
| --- | --- | --- |
| IR | `semantic::ir` | captured DOM + computed style; effective role (HTML-AAM table), accessible name (labelledby > label > native > content > title), ARIA state, inert/hidden. Node ids are capture node indices. |
| Script facts | `semantic::js` | oxc AST of the page's own bundles (`capture.resources`, same host): `useState` pairs, `jsx(..)` call sites with static props, class tokens (state-dependent ones separated), handlers resolved through symbol tables (toggle, set key, toggle key), `.map` loops with static item lists, `Escape` listeners, `matchMedia` listeners with their guard, scroll-lock assignments, `useEffect` dependency effects. React Compiler memo-cache reads are not treated as definitions. |
| Linking | `semantic::patterns::Links` | DOM element ↔ JSX site by shared class tokens, literal props, literal text; component call sites kept apart from the intrinsic element inside; wrappers without classes inherit the parent site of linked children. Ties are *ambiguous* and link nothing. |
| Recognition | `semantic::patterns` | `RULES` table: candidate selector + required / supporting / contra (veto) checks with weights. Confidence = 0.5 + 0.5·(supporting − contra)/Σsupporting once all required checks pass. First accepting rule claims an element. |
| Repeats | `semantic::components` | Merkle hash (tag, role, non-state classes, attribute names, child hashes; text/values ignored) → instances; lock-step walk gives text/attribute/class-variant slots; Zhang–Shasha tree edit distance reports near duplicates. Names come from `data-component`, BEM classes (utility classes are rejected), link/button labels, headings, an ancestor `data-component`; otherwise the group is reported, **not** extracted. |
| Emit | `semantic::emit` | `ui/<primitive>.tsx` wrappers, compositions, data modules, repeated-component files, `types/custom-elements.d.ts`. |

## Rule table (what maps to what)

| Rule | Base UI | Main evidence |
| --- | --- | --- |
| `dialog.role`, `alert-dialog.role` | `Dialog` / `AlertDialog` | role, `aria-modal`, fixed layering, inert while closed, bundle state. Trigger from `aria-controls`/`aria-owns` or from the handler that toggles the bundle's open state. Escape, scroll lock, focus trap are *provided*; media-query/route resets stay app code. |
| `tabs.tablist` | `Tabs` | tablist + tabs, `aria-controls` panels, one selected. Link tabs render as `<a>`. |
| `toggle-group.pressed`, `toggle.pressed` | `ToggleGroup` / `Toggle` | sibling `aria-pressed` buttons. |
| `accordion.disclosure-group`, `collapsible.disclosure` | `Accordion` / `Collapsible` | `aria-expanded` + panel (`aria-controls`, labelledby, adjacent sibling); single/multiple and "open item cannot be closed" from the handler (`SetKey` vs `ToggleKey`). |
| `navigation-menu.hover-state` | `NavigationMenu` | pointer-enter handlers that set one state to distinct keys. |
| `navigation-menu.popup-links` | `NavigationMenu` | `aria-haspopup` triggers inside a navigation landmark whose popups are mostly links (recognized; **not emitted yet**). |
| `popover.haspopup`, `menu.role`, `menubar.role`, `tooltip.role` | `Popover`, `Menu`, `Menubar`, `Tooltip` | recognized; no composition emitter yet (the `ui` wrapper is). |
| `switch.role`, `checkbox.role`, `radio-group.*` | `Switch`, `Checkbox`, `RadioGroup` | roles / native inputs. |
| `combobox.editable`, `select.combobox` | `Combobox`, `Select` | editable vs non-editable combobox, native `<select>`. |
| `field.native`, `button.native` | `Field`, `Button` | native form controls and buttons. |

Every interactive element no rule accepts is listed under *Unrecognized* with
the reason and the rules that nearly matched.

## Measured (page 0, 1440×900, initial state)

| Site | Controls | Bound to a primitive | Notes |
| --- | ---: | --- | --- |
| Natural | 102 (19 buttons, 83 links) | all 19 buttons: Dialog 1, Tabs 7 (2 lists), ToggleGroup 4, Accordion 3, NavigationMenu 3, Button 1 | the compat export used one primitive (Button) |
| Infinite Garden | 31 | Accordion 3, Button 18 | this page has 3 `aria-expanded` triggers; other pages were not analyzed |
| Microsoft AI | 251 | Tabs 5, Switch 2, Combobox 2, NavigationMenu 4, Accordion 2, Collapsible 1, Button 6 | no JSX bundle: DOM/ARIA evidence only |

These are counts of recognized controls, not fidelity claims. The emitted
fragments for the three pages pass `tsc --strict` against `@base-ui/react`
1.9.0; none has been rendered or compared with the original (parity belongs to
`mochi check-parity`).

## Checking the emitted TSX

The fragments are type-checked outside the test suite (needs an installed export for
`node_modules`): copy the emitted `src/` beside a `node_modules` containing
`typescript`, `react`, `@types/react` and `@base-ui/react@1.9.0`, add a `tsconfig.json`
(`strict`, `jsx: react-jsx`, `moduleResolution: Bundler`, `paths: {"@/*": ["./src/*"]}`)
and run `tsc --noEmit`. The three example pages pass. This proves the output type-checks
against the Base UI typings, not that it behaves like the original.

## Assembling a whole project

```sh
mochi semantic <capture> --out /new/dir --assemble [--viewport 1440x900] [--routes N] [--no-tokens | --captured-css]
cd /new/dir/project && bun install --frozen-lockfile && bun run check && bun run build
bun run dev                                  # http://127.0.0.1:3000 (PORT=... overrides)
```

`--assemble` writes a complete TanStack Start project (`mochi semantic` without it writes
the fragment described above). It reuses the compat exporter's `package.json`, `bun.lock`,
`vite.config.ts`, `tsconfig.json` and route-tree script read-only, so the two generators share
one stack and `--frozen-lockfile` stays valid. Nothing under `src/export*` is touched.

| Part | How it is made |
| --- | --- |
| Routes | one file route per captured path (`/` is `index`, `/blog/x` is `blog.x`); a captured path that is a prefix of another captured path is written `models_.x` so it does not become that route's layout and both pages render; query/fragment variants, other origins, paths that cannot be file routes and name collisions are skipped with the reason; a trailing slash is dropped and said so |
| Root document | `src/routes/__root.tsx`: `HeadContent`, `Outlet`, `Scripts`, `<html>`/`<body>` attributes from the leaf route's loader |
| Head | title, `meta`, canonical from the capture; icon links and JSON-LD from the mirrored HTML response with the same whitelist as `export::head` (mirrored, since that module is private to the exporter); a per-response `csp-nonce` meta is dropped |
| Markup rules | whitespace collapses only on ASCII whitespace (`pre` is verbatim), literal `&amp;` text is escaped, `srcset` follows the HTML tokenization, `src`/`srcset`/`poster`/`use href` are mapped to the copied files, captured form values become `defaultValue`/`defaultChecked`; not emitted at all (and counted per reason): scripts, styles, framework runtime elements (`next-route-announcer`), elements with a `javascript:` URL, iframe content documents; attributes React cannot type on the element go through a `Record<string, string>` spread; `style` attributes are dropped and counted; captured state-like classes stay on plain markup and are dropped only on parts of compositions |
| Page | `src/pages/<route>.tsx`: the captured `<body>` as TSX with Base UI compositions and repeated-structure components in place; compositions that need page state (controlled tabs, the dialog's `pathname`, hover-nav `dropdowns`) get their hooks and props there |
| Components | `src/components/<route>/…`, data in `src/data/<route>/…` (namespaced per route so identical names never collide), wrappers in `src/components/ui/` |
| Assets, fonts | files referenced by `src`/`srcset`/`poster` and by `url()` in the stylesheets (found with the CSS tokenizer) are copied byte for byte from the capture's `site/` to `public/<same path>`, so root-relative references need no rewriting; `.mochi/provenance/assets.json` lists source, size, SHA-256 and referrers, and what was not copied |
| Styles | default: the `tokens` workstream's translation of each route's computed author styles (`translate`, per captured page) into Tailwind utilities and shared recipes, applied through `emit::StyleBridge` (`StylePlan::bridge()`). The bridge is asked for **every** emitted element, class-less ones and `<html>`/`<body>` (via `element_props`) included; `js-*` hook classes stay beside the translated classes. One central `src/styles/tokens.css`, the carried `@keyframes`/`@font-face`/`@property` rules (`at-rules.css`, one copy of each) and every route's recipes (`recipes.css`) are imported by `src/styles/app.css` (Tailwind theme and utilities, no preflight), so one Tailwind compilation resolves the `@apply`s. Recipe names are unique across routes (`StyleOptions::reserved_names`); per-route reports and class maps are under `.mochi/provenance/styles/`. `--captured-css` keeps the captured class names and imports the tokenized captured stylesheets instead (also the fallback, with a gap, when the translation fails). Static equality checks only; `url()` in an arbitrary-value class is copied as an asset |
| Structure | the Base UI parts keep the captured element (`render={<ul />}`, `<li />`, `<section />` …) where it differs from the part's default tag; tabs that hold more than text are written out with their captured children; a bare link in a navigation list is a `NavigationMenu.Link` without an `Item` wrapper |

`scripts/semantic/verify-project.sh <project> / /pay ...` runs the three commands and then
server-renders each route.

Verified for the three example captures (this commit): `bun install --frozen-lockfile`,
`bun run check` and `bun run build` pass, and every route server-renders with status 200 and its
captured title, and no two routes render the same visible text (called through the built handler, no port opened; `scripts/semantic/ssr-check.mjs`). Hydration and rendering in a
browser were **not** checked here.

### Inventory the IR could not map (per route, widest viewport)

| Site / route | Body elements | In components | Plain markup | Controls (primitive / links / unrecognized) | Landmarks left plain | Graphics, embeds |
| --- | ---: | ---: | ---: | --- | --- | --- |
| Natural `/` | 995 | 354 | 641 | 102 (19 / 83 / 0) | main, 2 nav, 10 section, 6 article | canvas 1, iframe 1, svg 73 |
| Natural `/pay`, `/request` | 310 each | 169 | 141 | 79 (7 / 72 / 0) | main, 2 nav, 5 section | iframe 1, picture 1, svg 10 |
| Garden `/` | 1205 | 308 | 897 | 31 (21 / 10 / 0) | header, footer, main, 2 nav, 5 of 8 sections, 24 of 30 articles | canvas 5, picture 3, svg 51 |
| Garden `/articles` | 92 | 0 | 92 | 15 (3 / 12 / 0) | all | svg 5 |
| Garden `/articles/…` | 99 | 9 | 90 | 15 (0 / 15 / 0) | most | picture 1 |
| Microsoft `/` | 1379 | 156 | 1223 | 251 (22 / 229 / 0) | all 13 nav, 6 article, 2 header, 2 footer, 5 section | canvas 1, iframe 1, svg 36 |
| Microsoft `/models` | 897 | 156 | 741 | 246 (20 / 226 / 0) | all | iframe 1, svg 24 |
| Microsoft `/models/mai-cyber-1-1-flash` | 984 | 156 | 828 | 218 (21 / 197 / 0) | all | canvas 1, iframe 1, svg 28, video 1 |

"In components" means rendered by a Base UI composition or a repeated-structure component,
not that it is converted: the rest is a structural copy with the captured class names. Links
are counted separately because Base UI has no link primitive; they are between 32 % (Garden
home, 10 of 31) and 100 % (Garden article, 15 of 15) of the controls on these pages. No route was skipped. Assets not copied: Garden 17 (served from
`cdn.sanity.io`), Microsoft 3 (`uhf.microsoft.com`), Natural 0; they stay hotlinked.

### Handing the project to the parity pipeline

```sh
# offline, works now
bun benchmarks/src/parity/cli.ts inventory <capture> --out inv.json      # routes x viewports x states, gaps
mochi fingerprint-project /new/dir/project                                # prints the project hash the gate records
# needs local ports / a visible browser
mochi serve <capture> --port 8100 &
(cd /new/dir/project && PORT=3101 bun .output/server/index.mjs) &         # or bun run dev (port 3000)
bun benchmarks/src/parity/cli.ts run --inventory inv.json --reference http://127.0.0.1:8100 \
  --candidate http://127.0.0.1:3101 --out .mochi/parity-runs/<new> --profile core --project /new/dir/project
```

Expect the first comparison to differ broadly: styles are statically checked only, behavior and
motion are absent, and header/footer markup that is shared by the original is duplicated per route.

### Offline structure comparison

```sh
(cd project && bun run build && bun ../../scripts/semantic/ssr-check.mjs --save ../ssr / /pay)
mochi semantic <capture> --out /new/report --structure-diff /new/ssr    # structure.json, structure.md
```

Aligns the captured `<body>` with the server-rendered one (children by tag and own text,
recursively) and sorts every unmatched element: not emitted by design (script, style, link,
framework runtime elements), a dialog popup (Base UI mounts it in a portal on the client),
hidden in the capture, moved (same tag and text under another parent: a wrapper Base UI needs),
added by a Base UI part, or no counterpart. It reads no geometry, so it separates "the
element is not there" from "the parity matcher could not pair it"; it does not replace the
browser run. Server HTML only: portal content is absent from it by construction.

Snapshot at this commit (translated projects, server HTML against the evidence DOM at the widest
viewport; recompute with the commands above, no test pins these numbers): Garden is
element-for-element equal except the framework's announcer (2 captured elements) and the 6
scripts the server adds. Natural `/pay`: 310 captured, 182 rendered, 163 matched; 134 of the
unmatched are the mobile-nav dialog (a client portal, absent from server HTML by construction),
7 + 7 are elements under a Base UI wrapper. Microsoft AI: 1331 of 1379 matched on `/`, 894 of 897
on `/models`, 959 of 984 on the model page; the unmatched are mostly `<p>` elements that hold
`<div>`s (valid in the script-built DOM, restructured by the HTML parser; counted as a gap per
route) and hidden or svg elements. Pairing in the browser harness depends on geometry, which this
comparison does not read.

## Findings from the browser parity run of 7917885 (what was fixed, found, deferred)

Source: `docs/parity-reports/2026-11-10-semantic-final-outputs.md`. Causes below were found by reading the captures and the
original bundles, not by a browser run; the effect of each fix on the parity counts is not measured here.

| # | Finding | Status |
| --- | --- | --- |
| 1 | Natural desktop nav stacked: `NavigationMenu.Root` carried the captured element's classes (`display:flex`) and its `List` wrapper was a plain block between it and the items. | **Fixed.** When one captured element holds the items, `List` keeps it (tag, classes and the attributes the composition carries, box) and `Root` renders a `div` with `contents`, so the items' flex/grid parent is the captured one. Same for tabs whose root is the list (`TabsRoot` gets `contents`). |
| 2 | Menu panel: the original keeps the mobile-nav sheet in the DOM, `inert`, as a child of `<body>`; the build mounted a portal with `hidden`. | **Fixed as far as Base UI allows.** The dialog popup is `keepMounted` with `hidden={false}` and `inert={!open}`, so it is present, inert, and not display-hidden at rest. Unavoidable: `Dialog.Portal` wraps it in one `div` under `<body>` (`body>div#_R_…>popup` instead of `body>div.mobile-nav-sheet`) and the portal is absent from server HTML. Candidate named equivalence for the parity pipeline: *portal wrapper div*. The open look is not generated (closed-state styles are baked from the capture). |
| 3 | Garden diagram parts collapse onto one point. | **Fixed (cause: repeated-structure grouping).** The positions were already in the translated classes (`[left:246px]`, the bridge reads inline declarations into the cascade); the collapse came from `edge-network__slot`/`__card` instances being grouped into one component rendered from its first instance. A group whose corresponding elements differ in translated classes or inline style is no longer extracted (listed under `skipped_groups`), so each instance keeps its own. Under `--captured-css` (no translation) inline `style` attributes are emitted as a typed object instead (positions, sizes, custom properties); animation-state declarations (`transform`, `opacity`, `visibility`, `clip-path`, `filter`, `transition*`, `animation*`, stroke dash/opacity, vendor-prefixed forms) are left out there and counted per property in the route gap. The script that sets them is the `edge-network` code in chunk `e3cc149f…js` (`classList.add`, `dataset`). |
| 4 | Garden small-viewport typography (620 of 633 at 375). | **Cause found for part of it, deferred.** The bundle chooses the layout with `matchMedia("(max-width: 1023px)")` (the diagram code adds `home-engines--compact` or `--wide`; the categories component passes `layout: …?"compact":"wide"`), and `.home-engines--compact … { font-size: 10.384px }` and similar rules explain the 10.384, 11 and 10 px groups; about 120 items (12 → 16, 13 → 16) are not attributed. So the DOM differs per viewport (script-set classes), which the style translation cannot see from one snapshot. Needs the responsive milestone: per-viewport DOM variants for nodes whose classes or inline styles differ between the evidence viewports, selected by a media-query hook whose threshold is read from the bundle, with the tokens bridge asked once per viewport (coordinate with the tokens session). |
| 5 | Whitespace between inline spans lost. | **Fixed.** A whitespace-only text node between two inline neighbours (captured `display`, tag list otherwise; looking outward through an inline parent; not in flex/grid/table-structure/svg/list parents) is emitted as `{' '}`, and the leading/trailing space of a single-text element (`<span> is</span>`) is no longer collapsed away. |
| 6 | Microsoft AI React error #418 on `/` and the model page. | **Cause found and fixed at the generator.** Those two routes contain `<p>` elements holding `<div>`s (inside `split-text`); script-built DOM allows it, the HTML parser closes the `<p>`, so the server HTML differs from the client tree (`/models` has none). Such a `<p>` (when rendered as plain markup; not counting content inside a `button` or svg) is now emitted as `<div role="paragraph">` and counted as a gap; author selectors on `p` do not match it under `--captured-css`. Not verified in a browser. |
| 7 | Garden canvases blank. | **Plan only** (below). |
| 8 | Garden `Updated 11/8/2026`. | **Cause found, deferred.** Computed at runtime by the original: `Updated ${new Date().toLocaleDateString("en-US",{month:"numeric",day:"numeric",year:"numeric"})}`; the footer's year is a separate JSX child `new Date().getFullYear()`. The capture holds the capture-day text. Emitting a runtime date needs the JS-fact link from a text node to a call-site child expression that contains a `Date` call (client-only, `suppressHydrationWarning`); not built. |

### Canvas and WebGL milestone plan (not started)

What the original code needs (from the saved bundles): Natural draws its hero with a WebGL scene (`assets/dappled-scene-*.js`,
923 kB, with its shader strings in `assets/shaders-*.js`, 84 kB; five other chunks use only 2D contexts for small widgets);
Garden's five canvases are 2D (`getContext("2d")` ×3, `createElement("canvas")` ×2, 20 `requestAnimationFrame`, 11 `ResizeObserver`,
4 `IntersectionObserver` in the one app chunk that also builds the diagram). Proposed units, each its own new files:
`src/semantic/graphics/survey.rs` (per canvas: creator, context kind, size source, frame loop, inputs; reads `semantic::js` facts and
`surfaces` from the evidence), `src/semantic/graphics/shaders.rs` (extract shader source strings with provenance to
`src/graphics/<name>.glsl.ts`), `src/semantic/graphics/hooks.rs` (a named `use<Name>Canvas` hook per canvas: ref, resize, visibility
and frame loop, `mochi lift` output for closed pure geometry functions), `src/components/ui` untouched. Open questions before any
code: whether the scene libraries are vendored in the bundle (then they are imported as owned, provenance-recorded modules, not
reimplemented) and how the frame-time and scroll inputs are taken. Needs review before starting.

## Known gaps (kept explicit in the report)

- Initial state of one viewport only; states the capture never entered are
  known only through recovered scripts.
- Only React/JSX bundles yield script facts; hand-written DOM scripts do not.
- Tabs without `tabpanel` (Natural) keep panel content outside the component.
- Hover-menu dropdown bodies are rendered by script and absent from the capture.
- Trigger icons that swap with state, pointer-intent timing: not recovered. Links inside
  a nested accordion that close the dialog get an `onNavigate` prop; close handlers with no
  linked element, or inside repeated-structure components, are listed as gaps.
- Accordion without a heading emits no `Accordion.Header`; the runtime behavior
  of that arrangement is not exercised.
- Styles are baked per captured element: the selected tab or the open menu keeps the look the
  capture showed, no state variant is derived from a single captured state (a tabs gap says
  when the selected tab's styles differ). The translation takes about 2.5 minutes per page on
  a release build, and `recipes.css` is large (Natural `/` alone is ~4 MB: plain-CSS recipes
  repeat the full declaration set per width interval instead of only what changes); both
  belong to the tokens workstream.
- Base UI needs a `Root` element around a `List` (Tabs, NavigationMenu): where the capture
  has one element for both, the render has one extra wrapper `div` that generates no box (`contents`).
- A root-relative asset reference in an emitted file with no file under `public/` is counted as
  not copied and listed (`refs::dangling`, independent of what the resolver was asked).
- Utility-class detection for naming is a prefix/digit heuristic.
- `popup-links` navigation, Popover, Menu, Tooltip, Select, Combobox, Checkbox
  and Field are recognized but have no composition emitter yet.

## Later milestones and file boundaries (disjoint file sets)

| Milestone | New files only | Reads |
| --- | --- | --- |
| Motion translation (scroll, entrance, state animations → `motion`) | `src/semantic/motion/**` | `semantic::js` call sites and `Ex`; CSS animations from the IR style map |
| Canvas/WebGL → named modules with owned shader source, `mochi lift` geometry | `src/semantic/graphics/**` | `canvas` nodes, `surfaces`, `lift` manifests |
| Reconstructed assets | `src/semantic/assets/**` | capture `site/` and `resources` |
| Styles bridge | `impl StyleBridge` in `src/semantic/tokens/bridge.rs` | `semantic::tokens::TokenSet` |
| Remaining composition emitters | `src/semantic/emit/{popover,menu,select,...}.rs` | `semantic::patterns::Match` |
| Shared layout routes (header/footer duplicated per route today) | `src/semantic/layout/**` | the per-route component files `project` writes |
| Responsive variants (one evidence viewport per route today) | `src/semantic/responsive/**` | `Doc` per viewport from `evidence::load` |
