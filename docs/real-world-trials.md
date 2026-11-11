# Real-site reconstruction trials

## Current Bun previews and source ownership, 2026-11-08

The latest CLI-regenerated projects are under `/Users/enkyuan/Desktop/Projects/mochi/.mochi/projects`:

| Site | Project | Preview |
| --- | --- | --- |
| Natural | `natural/` | http://localhost:4184 |
| Infinite Garden | `infinitegarden/` | http://localhost:4185 |
| Microsoft AI | `microsoft-ai/` | http://localhost:4186 |

All use Bun 1.4.4 and physical TanStack routes. Route files delegate to `src/features/pages`; `src/compat/recovered` explicitly holds retained deployed programs. `EDITING.md` maps the files, and `.mochi/style-audit.json` records observed style values. Frozen installs, builds and typechecks pass. Compatibility source remains semantically unreconstructed: these are not fully authored Base UI applications or inferred design systems. Current checks are in `.mochi/runs/logs/examples-results.json` and `.mochi/runs/verification/`.

Older trial/source directories below are historical, preserved under `.mochi/evidence/trials`. Their previous Node servers were stopped when the Bun previews replaced them.


## Microsoft AI native-element trial, 2026-11-08

The development Mochi CLI recreates three captured Microsoft AI routes: `/`, `/models/`
and `/models/mai-cyber-1-1-flash/`. The final source project is
`/Users/enkyuan/Desktop/Projects/mochi/.mochi/evidence/trials/20261108-microsoft-ai/microsoft-ai`, served
locally on port 4186. Its TanStack routes host pristine server JSX plus 25 formatted native
ESM modules. Original custom-element controllers run the hero shader, menus, chart drawing
and accessibility behavior. This native path retains whole module initialization; it does
not claim React binding slices or restoration of the original repository's types/names.

The consumed capture has six complete observations and 166 assets with no download failures.
Cross-origin stylesheet text is obtained from the loaded Chrome CSS response, with provenance
recorded, rather than bypassing an incomplete capture. Source-evidence downloads preserve
bounded exact full-URL module receipts before mirror rewriting. Parsed import discovery also
downloads lazy native modules; markup-declared versioned chart URLs lower to finite import
branches using exact receipts. Already-localized images stay local, and recorded redirects
map native anchors to the captured canonical directory routes. Unregistered external custom
elements are omitted only when nonvisual in every selected capture; tag names remain in
the recovery notes. Registered hidden UI is retained.

Installation, production build, typecheck and all three HTTP route checks pass. Eighteen
local scroll states across 375x812@2 and 1440x900@1 have no initialization errors or broken
visible images. Visibility checks exclude hidden lazy menu images using `checkVisibility`.
An additional 1920x1080@2 check confirms live canvas rendering and different successive frame
hashes. The Models menu, accessibility toggle and local navigation to both captured model
routes work. No snapshot surface markers are used by these recovered pages.

The final serial suite passes 77 regular tests and one doctest; all seven explicit Chrome
tests pass. The updated native fixture separately checks anchors/SVG types, embedded fonts,
local images, canonical redirects, single initialization, interaction, scrolling, resizing
and HiDPI canvas dimensions. Clippy, formatting, actionlint and diff checks pass. An existing
interpretation-lock test failed in one parallel run and passed in the complete serial rerun;
its implementation was not changed.

**Visual parity remains unqualified.** The reference site began returning 429 responses,
interrupting paired live comparison. Failed candidates and raw diagnostics are retained;
they are not substituted for final validation. Final screenshots and states are in
`verification-local/`, source references in `capture-modules/reconstruction/screens/`, and
exact executable/source identities and check scope in `results.json`. Other public routes
remain linked to the source site. No backend restoration, paid model call, commit, push,
deployment or crate publication is claimed.

## Client-source recovery follow-up, 2026-11-08

The earlier static previews below were rejected. The development Mochi CLI now has explicit
client-source recovery adapters. Infinite Garden's homepage uses recovered Turbopack/Flight
React source; Natural's `/`, `/pay` and `/request` use recovered Vite binding slices, captured
TanStack loader data and the original pathless marketing layout. These components execute
their drawing algorithms and event code. They do not stretch saved canvas PNGs on those
selected routes. Infinite Garden's two article routes still use the initial-state emitter.

Artifacts are under `/Users/enkyuan/Desktop/Projects/mochi/.mochi/evidence/trials/20261108-client-recovery`.
The original normalized captures were reused offline without changes. Both source projects
were produced by `mochi export --recover-client-route ...`, then installed, built and
typechecked. The export command itself leaves `verification: not_run`; independent browser
checks and build logs provide runtime evidence. No paid model calls were used. Source
receipts identify consumed build files; original repository naming, TypeScript types and
private/backend behavior are not claimed to be restored.

The Infinite Garden comparison covers four viewport profiles, including 2x pixel density,
and eight scroll states per profile, including return scrolling. All 32 pairs have matching
recorded SVG connection geometry/dash offsets/opacity, scroll positions, document heights,
canvas dimensions and SVG counts, with no broken visible images. Unmasked full-viewport
pixel differences range up to 2.2742% on mobile because continuously moving effects were
sampled at different times. These results do **not** mean all frames pass the existing 0.25%
pixel threshold. The comparison manifest and original/generated PNGs are in `verification/`.

Natural's recovered WebGL component uses React 19.2.7, matching its custom reconciler's
producer build. Its generated lockfile pins that React version. Recovered routes preserve
the saved raw stylesheet text: CSSOM serialization lost variable animation shorthands and
otherwise left entrance elements invisible. Authoritative passive assets retain their
public paths for dynamic icon URL construction, within the existing asset budget.

The final projects are `natural-final/` (port 4184) and `infinitegarden-final/` (port 4185).
Both install, build and typecheck successfully; all six captured production routes return
HTTP 200. Their runtime sources and pinned package files match the separately checked
candidates byte for byte; equivalence receipts are retained alongside the projects.

Natural has 36 paired observations: three routes, four viewport profiles and three sampled
scroll states each. All recorded headings, section geometry, scroll positions, document
heights and canvas dimensions match, with no broken visible images. Its own hero controller
adjusts large programmatic scroll jumps, so the manifests record actual positions rather
than calling each requested midpoint an observed midpoint. Desktop product menus, the
mobile navigation dialog and the Codex tab respond in the final local app. No application
backend actions were tested or recreated.

Natural's raw unmasked mismatch reaches 9.5797% in a homepage dashboard frame; the recorded
frames include an animated shader background and different sticky-navigation paint states.
A subsequent settled recheck renders the navigation correctly. All raw comparisons remain
in `verification-natural/`, including the failed pixel-threshold comparisons. Matching
geometry and recovered programs do not establish synchronized frame-by-frame pixel parity.
The existing 0.25% threshold and channel tolerance 2 were not relaxed, cropped or masked.

The final local suite passes 72 regular Rust tests, one doctest and all six explicitly run
Chrome integration tests, plus formatting, Clippy with warnings denied, actionlint and diff
checks. The new owned fixture checks scroll-driven vector paths, live export namespaces,
React interactions, live resize and HiDPI canvas dimensions. Offline Vite tests cover binding
selection, shadowing, prototype writes, literal SSR data, undefined values, ambiguous data
refusal and bounded dependency/data expansion. These are local results, not hosted CI or a
published crate update. `results.json` records the executable identity and artifact paths.

The prior checkpoint and owner rejection remain below as historical evidence.

## Acceptance correction: functional reconstruction failed

The owner reviewed the Infinite Garden preview and rejected it on 2026-11-08. The generated
scene visibly scales a low-resolution captured image, and the original site's scroll-driven
SVG/scene animation is absent. These projects are initial-state exports, not accepted
functional reconstructions. The earlier build/typecheck and initial-viewport pixel results
remain measured facts; they do not qualify full-page motion, responsive geometry, device
pixel density or interaction behavior. The complete-capture count refers only to the initial
state schema. It must not be presented as completeness of website reconstruction.

The exporter excludes original script/event logic. Native SVG nodes retain sampled path,
visibility, transform and dash values without their scroll controllers, while canvas surfaces
are rendered from static PNGs. The source has both SVG and canvas layers. Raising screenshot
resolution alone would leave the missing scroll behavior unresolved. The next reconstruction
work must preserve native vector geometry and implement the observed scroll timelines,
reveals, transitions and responsive geometry as editable code, with whole-page behavioral
checks at intermediate scroll positions and viewport sizes.

## Initial-state export follow-up, 2026-11-08

The development exporter now generates and runs editable TanStack Start projects for both
sites. The original failed checkpoint is retained below; it describes the earlier build.
Historical artifacts: `/Users/enkyuan/Desktop/Projects/mochi/.mochi/evidence/trials/20261108-normalized`.
`results.json` records the tested executable hash, dirty Git head, renderer profile, stage
results, all six route HTTP checks, download diagnostics and four screenshot comparisons.
This is a local development result; the published crate has not been updated by this step.
No credentials or paid/live Jev calls were used.

| Site | Captured routes | Complete observations | Build / typecheck / production HTTP | Homepage mismatch, desktop / mobile |
| --- | --- | --- | --- | --- |
| Natural | `/`, `/pay`, `/request` | 6 / 6 | Passed | 0.027% / 0.063% |
| Infinite Garden | `/`, `/articles`, `/articles/commercial-intelligence-with-infinite-memory` | 6 / 6 | Passed | 0% / 0% |

Each observation has a saved 1440x900 or 375x812 reference screenshot. The reported differences
compare the entire homepage viewport against the generated production app, with a tolerance
of 2 per RGB channel and the unchanged fixture threshold of 0.25%. No cropping, scene masks
or relaxed thresholds were used. These four homepage comparisons do not qualify every route,
scroll position, browser, intermediate width or interaction. All six captured routes returned
HTTP 200 from production SSR; visual qualification of the remaining routes is separate work.

The preview ports were subsequently reassigned to the recovered builds. Historical editable sources
are in each site's `project/src/`; `capture/reconstruction/screens/` contains references and
`verification/actual-*.png` contains generated-app screenshots. `verification/stages.json`
and `comparison-*.json` hold the per-site results. The exporter leaves its own verification
field at `not_run`; these external checks provide the actual verification evidence.

The successful pipeline used:

```sh
mochi clone <url> -o <trial>/<name>/capture --source-evidence \
  --source-viewports 375x812,1440x900 --max-pages 3
mochi check-evidence <trial>/<name>/capture
mochi model-project <trial>/<name>/capture
mochi export <trial>/<name>/capture -o <trial>/<name>/project
cd <trial>/<name>/project
npm ci --ignore-scripts --no-audit --no-fund
npm run build
npm run check
HOST=127.0.0.1 PORT=<port> npm start
```

### What changed

- Lossless string interning removes repeated style/property text without raising the
  16 MiB capture budget. Natural's complete homepage now includes over 1,260 nodes per width.
- Shadow trees and readable frames are captured and emitted as actual source content.
  Hidden runtime frames are classified explicitly. Opaque visible frames have isolated images.
- Canvas and WebGL pixels are captured directly and painted into generated native canvas
  components. Pixel files are hashed and checked before modeling and export.
- Each viewport loads a fresh document. WebGL requests select the SwiftShader backend for
  Natural; Infinite Garden uses the default renderer. Both capture and verification use
  Chrome 157.0.8103.39 at scale 1 with native scrollbars hidden, recorded in the manifest.
- Finite entrance effects settle before sampling. Script-created animation properties retain
  their observed values, keeping Infinite Garden's navigation and Explore link visible.
  Captured CSS animation declarations preserve Natural's text entrances.
- Responsive DOM variants, scoped shadow CSS, SVG elements and inline style priorities
  survive export. JavaScript expressions are no longer mistaken for CSS resource URLs.

Natural saved 651 assets with one reported resource failure; Infinite Garden saved 37 with
18 failures. The exact failed URLs remain in their manifests/results. These diagnostics
were not suppressed or reclassified as successful downloads. Completeness applies to the
recorded source graph and surface pixels, not universal availability of every external URL.

Canvas scenes currently preserve a captured initial frame in an editable component with a
replaceable image. This fixes missing rendered content; it does not recover the original
scene graph, animation loop or input program. Original scripts, backend actions and form
submission destinations are not replayed. Further reconstruction should implement those
behaviors explicitly and verify them against observed interactions.

The expanded owned fixture also passes all six reference comparisons (worst 0.093%), native
2D/WebGL pixel checks, shadow/slot/frame checks, CSS and script-created entrance states,
production builds, typechecks, route/query navigation, controls and a generated-prop edit.
The complete five-test Chrome suite, 63 regular Rust tests plus one doctest, Clippy with
warnings denied, formatting and diff checks passed. Evidence is in
`/private/tmp/mochi-normalized-qualified/fixture-verification.json`,
`/private/tmp/mochi-normalized-qualified.log`, `/private/tmp/mochi-final-tests.log` and
`/private/tmp/mochi-final-clippy.log`.

---

# Initial failed checkpoint

Checked locally on 2026-11-08 against the development checkout on
`feat/tanstack-exporter`. These are observed failure cases, not fidelity qualifications.
The existing local-only model constraint was retained: no real credentials or paid Jev calls.

Trial artifacts: `/Users/enkyuan/Desktop/Projects/mochi/.mochi/evidence/trials/20261108-030806`.
`results.json` includes the branch/head, dirty-state flag, tested executable hash, exact
stage commands, exit codes and errors. Each site's `capture/` contains mirrored files,
source observations, screenshots, manifests and completeness reports.

## Results

| Site | Captured pages | Complete source captures | Source project |
| --- | --- | --- | --- |
| [Natural](https://www.natural.com/) | `/`, `/pay`, `/request` | 0 of 6 | Refused: incomplete source evidence |
| [Infinite Garden](https://infinitegarden.com/) | `/articles` and two article details | 0 of 6 | Refused: incomplete source evidence |

Both commands saved useful mirror/evidence files. Neither produced an editable source
project. No npm install/build, local source-app runtime or pixel-parity result is claimed.
Incomplete evidence prevented `model-project`; request planning then lacked a saved model,
and `export` refused with `run model-project before export`. No failed stage was bypassed.

Commands were run independently for each site with a new output directory:

```sh
mochi clone <url> -o <trial>/<name>/capture --source-evidence \
  --source-viewports 375x812,1440x900 --max-pages 3
mochi check-evidence <trial>/<name>/capture
mochi model-project <trial>/<name>/capture
mochi plan-interpretation <trial>/<name>/capture
mochi export <trial>/<name>/capture -o <trial>/<name>/project
```

The three-page bound counts successful page observations, so a failed homepage attempt
can be followed by other discovered same-site pages. This run respected robots directives
and did not click external forms or sign in. These commands used the newly built local
binary, not the previously published crate version.

## Natural

The mirror saved three pages and 651 assets, with 83 reported download failures (404).
Both homepage widths recorded uncaptured canvas content and reached the 16 MiB source
evidence budget. The desktop JSON contains 514 recorded nodes and 359 computed-style maps;
its styles dominate the saved payload. Those are partial observations, not a complete
homepage tree. `/pay` and `/request` instead record uncaptured iframe content at both widths.

The independent gstack browser inspection also logged Three.js failures creating a WebGL
context in this headless environment. Its viewport screenshot has an empty hero region.
That is a limitation of the tested rendering environment; it does not establish a defect
in Natural's normal browser experience. Preserving this screenshot cannot qualify faithful
reconstruction of the missing rendered scene.

Artifacts: `natural/capture/reconstruction/report.json`,
`natural/capture/reconstruction/screens/`, `natural/stages.json`, `natural-live.png`,
`natural-console.txt` and `natural-clone.log`. The live browser screenshot was taken at
1440x900; the source captures include 375x812 and 1440x900.

## Infinite Garden

The mirror saved three article pages and 37 assets, with 46 reported failures (45 HTTP 404,
one HTTP 403). Its homepage attempt was skipped with
`Method call error -32000: Inspected target navigated or closed`; the pipeline then captured
the articles listing and two article details. The root cause of that navigation interruption
has not been isolated. The 403 was an analytics configuration resource, not evidence of a
site-wide access restriction. Several 404 URLs contain minified JavaScript expressions,
which also exposes false positives in the mirror's resource discovery.

All six article captures record `shadow_document`. A separate gstack inspection identified
an open `next-route-announcer` shadow root containing a visually hidden live-region element.
The current contract refuses any uncaptured open shadow content, including framework runtime
infrastructure. Ignoring this particular host would need an explicit capture/runtime policy;
it was not silently marked complete for this trial.

The homepage loads in gstack. A separate invocation of the production source-capture script
in that browser produced 1,448 nodes and reported five canvas elements plus the shadow root.
That diagnostic is saved as `infinitegarden-home-probe.json`. It is not a successful CLI
capture or a validated screenshot/manifest set. The live homepage screenshot visibly includes
the canvas composition; its behavior cannot be reconstructed from light DOM/CSSOM alone.

Artifacts: `infinitegarden/capture/reconstruction/report.json`,
`infinitegarden/capture/reconstruction/screens/`, `infinitegarden/stages.json`,
`infinitegarden-live.png`, `infinitegarden-dom-audit.json`, `infinitegarden-home-probe.json`
and `infinitegarden-clone.log`. `browser-console-aggregate.txt` is the fallback driver's
combined tab stream; it must not be treated as Infinite Garden's page-specific error log.

## What these trials prioritize

1. Distinguish captured content from framework-owned runtime infrastructure, including
   a deliberate accessibility replacement policy for Next's route announcer. Qualify article
   exports before claiming support for these full sites.
2. Add an explicit strategy for canvas/WebGL and embedded frames: supported reconstruction,
   separately labeled static raster fallback, or a precise refusal. Raster fallback would
   lose original interactivity and must not be presented as recovered editable drawing code.
3. Reduce repeated computed-style/custom-property storage within the evidence schema while
   preserving values and completeness checks. Investigate homepage navigation stability and
   invalid JavaScript-derived resource URLs. Raising limits alone would leave the missing
   canvas/frame content unresolved.

Semantic interpretation comes after valid evidence and a project model. Jev labeling cannot
repair these omissions or establish fidelity. The controlled fixture's passing renders
remain valid for that fixture; these real-site failures prevent generalizing them.
