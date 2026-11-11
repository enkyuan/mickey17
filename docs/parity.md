# Reconstruction parity contracts

A successful export/build is not proof that a reconstruction matches its source.
`mochi check-parity` compares saved browser measurements against a reviewed state
contract. It returns a failure for changed runtime inputs, omitted routes, missing
states or anchors, browser errors, unsettled measurements, or differing facts.
It never upgrades an export report automatically to claim whole-site parity.
A contract that declares coverage `gaps` still produces a report (status
`unqualified_coverage_gaps`, or `failed` when facts also differ), but can never pass.

Keep contracts, captures, reports and reference material in `.mochi/parity/`.
Keep actual components, styles, fonts, SVG geometry and shaders in `src/`.

## Capture procedure

1. Inventory every captured route, section, graphic, control and state transition.
   Record unknown behavior as a coverage gap. Do not remove failing probes to pass.
2. Set the viewport and reduced-motion preference **before navigation**. Wait for
   fonts and assets. Use identical browser versions, zoom and device pixel ratio.
3. Record initial, intermediate scroll, bottom, reduced-motion, and every supported
   interaction state. Include scroll entry/exit, reverse scrolling, resize and
   keyboard behavior. Test desktop and mobile for every route. The minimum matrix
   enforced by the CLI does not substitute for this behavior inventory.
4. Measure source and candidate using reviewed semantic selectors. The browser
   expression in `scripts/parity/measure.js` captures geometry, typography, colors,
   content, ARIA state, links, image loading and SVG paths. A selector must match
   exactly one element. Collect console/network failures separately and append them
   to `errors`; a DOM expression cannot recover console errors from before it ran.
5. Use source equations and DOM measurements to fix discrepancies. Do not tune CSS
   against screenshots. Test graphics at an explicit identical time/progress; a
   matching canvas rectangle says nothing about shader output or drawn geometry.
6. Rebuild and recapture after every runtime-input change. Re-run the full matrix
   after repairs. Preserve failing reports and the original immutable reference.

## Offline gate

```sh
mochi fingerprint-project .mochi/projects/example
mochi check-parity .mochi/projects/example \
  --contract .mochi/parity/contract.json \
  --reference .mochi/parity/reference.json \
  --candidate .mochi/parity/candidate.json \
  --out .mochi/parity/report.json
```

The contract is schema version 1 with `reference_sha256` (SHA-256 of exact reference
JSON bytes), `project_sha256` (from fingerprint-project), `routes`, `states`, and
`gaps`. State names are stable descriptions, such as `home.desktop.menu.open`.
Each state declares `route`, `[width,height]` in `viewport`, `reduced_motion`,
`phase` (`initial`, `scroll`, `interaction`, or `bottom`), an `inventory` of semantic
anchor names, and numeric `tolerances` for `.bounds.*` geometry only. Tolerances
cannot exceed one CSS pixel;
unlisted facts, strings and booleans compare exactly. Do not use pixel tolerance
for unitless animation progress, matrices, colors or path equations.

Reference and candidate captures have `schema_version`, `project_sha256` (null for
reference), and `states`. Each state contains its browser context, `inventory`,
flattened scalar `facts`, `errors`, and `settled`. Every anchor needs measurements.
Both captures must contain exactly the contract's states and facts. The current
literal TanStack route declarations are parsed from TypeScript/JavaScript ASTs
and must match the route inventory. A TanStack index-route ID’s trailing slash
is normalized for this identity check only; recorded browser URLs remain exact.
Comments cannot stand in for route declarations. Source edits
and newly added files invalidate the candidate fingerprint.

The output path must be new for every invocation. Malformed inputs return nonzero
and produce no report; never reuse an earlier successful artifact. A declared gap is not
malformed input: it is reported (see below). Capture files may be up to 256 MiB.

The result is `matched_declared_contract`, `unqualified_coverage_gaps` or `failed`. It is evidence for exactly
the recorded inputs and states, not proof about unobserved behavior, browser/GPU
combinations, capture authenticity, or other websites. Coverage must be independently
reviewed. Every new site remains unqualified until its own contract passes; known
gaps and missing evidence cannot be replaced by a successful build or a screenshot.

## Whole-page element pipeline

`check-parity` compares a handful of reviewed anchors. The element pipeline in
`benchmarks/src/parity/` (collector in `scripts/parity/collect.js`) compares the
**original capture, served by `mochi serve`, with a generated app** over every element
that paints, carries text, media or a control, at identical scripted states. It
feeds the same strict gate. It is evidence for the declared states only.

```
capture ──inventory.ts──▶ inventory.json (routes × viewports × scroll × interactions × clock × breakpoints, plus gaps)
original (mochi serve) ┐
                       ├─ driver (headless Playwright, or Browser pane + sink) ─▶ collect.js ─▶ ref/cand captures
generated app          ┘
captures ──diff.ts (bidirectional) ─▶ per-state summary, REPORT.md ──gate.ts──▶ mochi check-parity
```

### What `collect.js` records (schema 2)

A superset of the hardened Natural collector. Per element (including `<html>` and `<body>`, which carry the page background): tag, role (and
whether it is an explicit attribute), accessible name,
Base UI part (`data-base-ui-*`, `data-slot`, `data-state`, …), ARIA attributes, `inert`,
tabindex, page-coordinate bounds (2 decimals), own text, resolved computed style in five
groups (layout, typography, styling, motion, interaction; deduplicated through a style
table), effective opacity / clip / filter / mask / blend through **all ancestors**
(wrapper opacity was a past blind spot), link `href` and target, images (`currentSrc`,
natural size, alt, **content SHA-256 of the bytes**), CSS background and mask image
bytes, SVG (`viewBox`, structure hash, every `<path d>` hash and length, paint hash,
animation), canvas (size, `toDataURL` hash, blank detection), form state, `<dialog>`
modal state, pseudo-elements `::before/::after` of html, body and every element (content,
the host's position plus the pseudo's computed width/height, style, background and mask bytes).
Page facts: title, lang, meta, landmarks, headings, text, counts, document size, scroll
lock (`body`/`html` overflow), `[inert]` / open dialog / `aria-hidden` counts, declared
font faces, running animations (which font faces happened to load is recorded as
informational `info.fonts.loaded` only). Focus is read with `activeElement()` (`:focus-visible`,
outline). Anything a DOM read cannot see or had to truncate (cross-origin bytes, blank WebGL
read-back, iframes, shadow roots, text over 600 chars (a full-text hash is kept), more than
400 paths or 120 shapes in one SVG, `data-*` values over 60 and input values over 80 chars,
`::placeholder` styles) is added to `limits`; the remaining unlisted limits are the ones
named here and in the inventory gaps. Font
bytes and a handful of rarely used CSS properties are not compared.

### Matching and what counts as a difference

`diff.ts` pairs text elements by text then geometry, other elements by kind and geometry,
and runs a **reverse pass** over leftover candidate elements so a displaced element is
reported as moved rather than missing + extra. Unpaired elements on either side fail.
Geometry tolerance is 1 CSS px (the gate's cap); everything else is exact. Differences
are classified by dimension (layout, typography, styling, motion, content, assets,
graphics, behavior, routing, semantics, interaction). Only these named equivalences are
`minor` (counted in every report, applied identically in the gate): `gradient-direction`
(`to top, A, B` ≡ `B, A`), `zero-position-unit` (`0%` ≡ `0px` in position properties),
`href-trailing-slash` (after removing the site origin), `redundant-tabindex-0`
(`tabindex="0"` added to a natively focusable element; not to an `<a>` without `href`),
`container-retag` (the **tag** of a `div`/`section`/`article`/`span` changes to another of
them and neither side has a `role` attribute; `aside`, headings, lists, controls and any
explicit role are never covered), `added-component-attributes` (the candidate only
*adds* `data-base-ui-*`, `data-slot`, `data-state`-style attributes) and `base-ui-added-aria`
(the candidate only *adds* `aria-expanded="false"` / `aria-disabled="false"` / `aria-haspopup` to an element whose role is
`button` (a plain button or summary) or an `<a>` link, or its implicit `aria-live` / `aria-atomic` values to a
`status` / `alert` / `log` region that had neither; tabs, comboboxes, menu items and switches never qualify; see
`docs/parity-named-equivalences.md`). The Rust gate sees the
same equivalences: style/href values are canonicalised on both sides, and tag / role / data /
tabindex / aria facts of a pair are set to the reference value (aria: only the equivalent
additions are dropped from the candidate) only when the pair carries that minor item. `tests/parity-diff.test.ts` asserts, over a catalogue of mutations, that the gate's facts
differ exactly when the TS report has a major item.
State-level checks add focus order (tab sequence incl. `:focus-visible` outline and
box-shadow), failed interaction steps, candidate-only console/page errors, reached scroll
position, and two non-passing statuses: `unsettled` (the page never stopped moving) and
`not_exercised` (a step failed on the reference, for example a trigger that is not
interactable, so the state proves nothing, even if the candidate fails the same way).
Steps that fail only on the candidate are differences. A state that crashed is recorded in
`failures.json`, listed in the report and passed to the gate as a gap.

### Inventory, derived from the capture

```sh
bun benchmarks/src/parity/cli.ts inventory <capture> --out .mochi/generated/parity-runs/site-inventory.json
```

Routes and site origins come from `manifest.json`; viewports, controls (tabs, disclosures,
popup triggers, links, buttons), graphics counts and `role=dialog` come from
`reconstruction/pages/*.json`; media-query breakpoints from the captured CSS and inline
`<style>` (boundary states at `N−1, N, N+1`, so both `min-width:N` and the inclusive `max-width:N` are straddled). Per route and captured viewport it emits:
initial, reduced-motion initial, scroll down at 25/50/75/100 %, reverse scroll up at
75/50/25/0 % (the whole journey is replayed, with wheel events), reduced-motion bottom,
12-step Tab sequence, click / Escape / toggle-twice per disclosure or popup trigger,
click per tab, hover on a bounded set of controls, fixed-clock entrance states
(t = 0/400/1200/3000 ms) and breakpoint boundaries. Detected-but-unscripted things
(canvas/WebGL pixels, native or role dialogs without a derivable trigger, controls with roles
such as switch/checkbox/combobox, forms, iframes, blocked third-party hosts, pointer-driven
effects, height / container / orientation / touch / light-scheme media queries, CSS that was
not scanned, mobile touch emulation, font bytes) become explicit `gaps`; native `<details>` /
`<summary>` are scripted as disclosures; they flow into the
contract and the report. Nothing is hand-listed.

### Headless driver

```sh
mochi serve <capture> --port 8100 &                       # original
(cd <export> && PORT=3101 bun .output/server/index.mjs) & # generated app (or bun run dev with its own port)
bun benchmarks/src/parity/cli.ts run --inventory inv.json \
  --reference http://127.0.0.1:8100 --candidate http://127.0.0.1:3101 \
  --out .mochi/generated/parity-runs/new-run --profile standard --controls --workers 2 \
  --project <export>      # optional; recorded for the gate
bun benchmarks/src/parity/cli.ts gate .mochi/generated/parity-runs/new-run --project <export> --out .mochi/generated/parity-runs/new-run-gate
```

Needs `playwright-core` (resolved from `MOCHI_PLAYWRIGHT`, `benchmarks/node_modules`, or
gstack's `node_modules`) and a Chromium binary (`MOCHI_CHROME`, or the Playwright cache's
`chrome-headless-shell`). The driver blocks every origin except each side's own and lists
what it blocked. It uses a fresh context per state, UTC / en-US / sRGB / DPR 1, wheel
scrolling followed by an exact `scrollTo` correction (smooth scrollers overshoot, unequally
on the two sides), and a Playwright virtual clock for `entrance` states (virtual time is paused before any page script runs,
so load time cannot advance Date, timers or rAF, and moves only through `runFor(t)`; the driver checks
`Date.now()` afterwards and marks the state unsettled on drift or on images still loading; every CSS/WAAPI animation that exists then is paused at *t* of its
own local time). Animations created after the freeze (scroll- or observer-triggered) are not
frozen: such a state (any animation still running after the freeze) is marked unsettled and says so
in its limits. "Settled" means the visual
signature (every element's box, opacity / transform / clip / filter, colors of transitioning
elements, text length, scroll position, running finite animations) is unchanged for about
1.2 s; an animation that changes none of these and starts later than that looks settled. `--controls` loads the
original twice for initial states to measure nondeterminism; `--force-webgl-readback`
forces `preserveDrawingBuffer` identically on both sides so canvas pixels are readable.
Profiles: `quick` (initial + reduced-motion initial), `core` (initial, reduced-motion,
scroll down 50/100 % and up 50/0 %, Tab sequence, the first menu / tab / hover states per
route and viewport, entrance at 0 and 1200 ms, custom viewports; no breakpoint boundaries),
`standard` (everything; breakpoint boundaries for the first route only), `full` (everything). Skipped states are listed with reasons. Output
directories must be new. Every raw capture is retained gzipped next to each summary.

### Browser pane driver (no local Chrome needed)

The pane is the app's own browser, so it reaches loopback servers that the sandbox blocks.
Constraints, all observed:

1. **The pane must be visible** (a hidden pane reports a 0×0 viewport and pauses
   animations and timers). If it cannot be shown, `resize_window` with an explicit size
   gives a correct layout viewport, but animation timing is then not trustworthy.
   Reset with `preset: "desktop"` afterwards.
2. It cannot set `prefers-reduced-motion`, has no virtual clock, and runs a different
   browser than the headless path (real GPU vs SwiftShader). **Never mix the two in one
   comparison.** Both sides of a pane comparison must be captured in the pane.
3. A JavaScript result is too large to read back, and the page cannot write files. A
   **sink server** receives POSTs instead:

```sh
bun scripts/parity/sink.ts --port 8765 --dir .mochi/generated/parity-runs/pane-1        # or a launch.json entry
```
```
preview_start {url: "http://127.0.0.1:8100/"}              # original
resize_window {width: 1440, height: 900}
javascript_tool: (0, eval)(await (await fetch('http://127.0.0.1:8765/pane.js')).text())
javascript_tool: JSON.stringify(await __mochiPane.capture('home.desktop1440.initial.ref'))
navigate {url: "http://127.0.0.1:3101/"}                   # candidate; globals reset, so inject again
javascript_tool: (same two calls) …capture('home.desktop1440.initial.cand')
```
```sh
bun benchmarks/src/parity/cli.ts pair --ref .mochi/generated/parity-runs/pane-1/home.desktop1440.initial.ref.json \
  --cand .mochi/generated/parity-runs/pane-1/home.desktop1440.initial.cand.json --origins https://www.natural.com --out pair.json
```

The sink serves `collect.js` / `pane.js`, grants CORS to loopback origins only (the pages under test) and answers 403 to any POST or GET that carries a non-loopback `Origin`, refuses non-JSON bodies,
unsafe names and overwrites, and listens on loopback only. Interactions in the pane are
done with `computer` clicks / `find` before `capture`. The pane sends no error list, so
console errors are not compared on that path. A `launch.json` entry for the sink:
`{"name":"parity-sink","runtimeExecutable":"bun","runtimeArgs":["scripts/parity/sink.ts","--port","8765","--dir",".mochi/generated/parity-runs/pane"],"port":8765}`.

### Calibration

```sh
bun benchmarks/src/parity/cli.ts calibrate --inventory inv.json --reference http://127.0.0.1:8100 --out .mochi/generated/parity-runs/calibration-1
```

The original is loaded twice (an identical control, which must produce zero differences; it
is the nondeterminism floor, and is why which font faces *happened to load* is recorded as
informational `info.fonts.loaded`, while the declared faces are compared), a known
regression is injected into the second load, and the diff must contain a kind of difference
the control did not: opacity, wrapper
opacity, body / html background, explicit role on a div, font-variation, letter-spacing, text-align, image bytes, backgrounds, borders,
radius, shadows, injected text, edited text, removed section, hidden menu, SVG hide / path /
viewBox / fill, canvas size, ARIA label / hidden / expanded, link target, heading tag,
pseudo content, transition, a 2 px move of the whole page (an empty unpainted block prepended to body), and a **shifted media-query breakpoint** for every
breakpoint cluster of the first route (rules within 1.5 px of the cluster's first value are
shifted, in `@media`, `@import`-ed sheets and `media=` attributes), tested at N−1, N and N+1 (strict `width < N` and
inclusive conditions switch on different sides of N; lower bounds are shifted up and upper bounds
down, in every syntax). A "detected" mutation must surface a kind of difference its own dimension can
produce and that the control lacked, not any new difference. Documented blind spots are asserted too: hover rules, focus rings and removed click handlers, which are invisible in
an initial state and **must be detected** in `hover`, `keyboard` and `menu` states.
A miss fails calibration, and so does a mutation the Rust-gate facts do not see. A breakpoint
shift whose rules were rewritten but which no fact detected is reported **unproven** (it may be a no-op on
that route or a blind spot; it neither passes nor fails calibration) unless the pixel oracle is switched on
(`MOCHI_PIXEL_PROOF=1`; slow, and the headless browser crashed in screenshots on a loaded machine), in which case
it passes only if detected or if the **rendered page (viewport-sized slices down the first 40 screens, canvas and video hidden, after a warm-up pass, measured on a separate page so the screenshots cannot disturb the diffed run) is pixel-identical before and after** and rules were actually rewritten
(a proven no-op: the screenshot is the independent oracle, so style changes confined to uncollected wrappers do not count as blind spots); a breakpoint that appears in no stylesheet the route loads is reported as not applicable; a shift that changed pixels without a
detected difference is a blind spot and fails. Every mutated capture and diff is kept under
`evidence/`, next to `CALIBRATION.md` / `calibration.json`. This measures the
instrument, not the candidate.

### The Rust gate

`gate` writes `contract.json`, `reference.json`, `candidate.json` for `mochi check-parity`
(anchors are reference element indices; candidate-only elements get `x…` anchors, so they
fail the inventory check; tolerances are 1 px on bounds only) and runs it. Page-level facts
carry the focus order and the reached scroll offset; failed interaction steps and **every**
console/page error of either side are the state's `errors`, which the gate fails on (so the
fallout of blocked third-party hosts on the original makes gate states fail; the TS report
separates reference-only, candidate-only and shared errors). Declared gaps produce a retained
report with status `unqualified_coverage_gaps` instead of a bare error, and the capture size cap is
256 MiB (it also applies to each project file hashed by `fingerprint-project`). The
inventory always declares generic gaps (pointer-driven effects, smooth-scroll curves, mobile
emulation, …), so this pipeline **cannot reach `matched_declared_contract`**; the gate's
role is the strict per-fact comparison, the fingerprint and route binding. It additionally
requires a TanStack project whose literal `createFileRoute` routes equal the contract's.

### What this can and cannot detect

Detects: any change of the recorded geometry (>1 px), resolved style (including the page
background on html/body), text, links, image bytes, SVG geometry/paint, ARIA, tab order and
focus ring (outline or box-shadow), dialog/inert/scroll-lock page facts, candidate-only
console errors, and the effect of media-query changes, at the scripted states.
Cannot detect: pixel-level rendering (fonts hinting, filters, blend results), WebGL /
canvas content unless readback is forced and the state is frozen at a source-supported time,
shader uniforms, pointer-driven or per-frame scroll-linked values between positions,
form validation or submission, third-party scripts (blocked on both sides), video frames,
iframe content, shadow roots, server behavior, other browsers or GPUs, and anything not in
the inventory. An unreachable control or unscripted state is a gap, not a pass.

### Known limitations of the pipeline itself

Found by three adversarial reviews and deliberately not fixed (each is also a reason to read a report
as evidence for the scripted states only):

- Calibration judges detection by kinds of difference plausible for a mutation's *dimension* (a coarse
  table), not by a per-mutation expected property; a coincidental new kind in a plausible dimension
  still counts.
- Breakpoint shifts do not reach stylesheets inside shadow roots, nor `adoptedStyleSheets` of shadow
  roots; the pixel proof only covers the document's scroll area, not inner scrollers.
- The gate-detection check in calibration has no plausibility filter and does not subtract noise in
  `page.state.*` facts.
- Menu calibration decides whether a click changes the original by comparing two separate page loads,
  so noise can make an inert menu look active (that can only produce a false calibration failure).
- Descriptor resolution (inventory → runtime) matches name and role heuristically; shadow-root and iframe
  text, `<button role="">`, and unusual accessible-name sources can still make a trigger unfindable. That
  yields `not_exercised`, never a pass.
- `settled` is "unchanged for about 1.2 s with no finite animation running"; JS timers that fire later than
  that are not seen. Real-time states are therefore weaker evidence than fixed-clock states, and fixed-clock
  states only control what the Playwright clock controls.
- Pane captures carry no console errors, focus order or step failures.
- Scroll-linked state is not reproducible across runs on a loaded machine (the same Natural state gave 270 and
  1,132 major items in two runs), although each side agrees with itself within a run; only the classes of
  difference are evidence, not their totals.
