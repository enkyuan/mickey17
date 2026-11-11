# First scoped parity report (whole-page element pipeline)

Produced 2026-11-09/10 with `benchmarks/src/parity` on a loaded development machine (Chromium 158 headless
shell, Playwright 1.65, SwiftShader GL, DPR 1, UTC, en-US). **This is evidence about the declared states of
the declared candidates only.** It is not a screenshot comparison, not a score, and says nothing about
unobserved states, routes, browsers, GPUs, or any other site. Raw captures, per-state diffs, reports and
the failed/superseded runs are retained under `.mochi/generated/parity-runs/` (git-ignored).

Pipeline revision: the working tree this report ships with (`run.json` of each run records the hashes of
the collector and of every `benchmarks/src/parity/*.ts`; the `-rediff` directories recompute summaries from
the retained raw captures with the final comparison code, and the gate ran on those).

## What was compared

| Run | Original (served by `mochi serve`) | Candidate | Profile | States executed |
|---|---|---|---|---|
| `natural-compat-core-1` | Natural capture, 3 routes `/ /pay /request` | `.mochi/generated/exports/natural-compat` (compat export, production build) | core | 82 of 308 scheduled |
| `garden-recover-core-1` | Infinite Garden capture, 3 routes | `.mochi/generated/exports/infinitegarden-recover` | core | 77 of 192 |
| `garden-hand-core-1` | same | hand-edited `.mochi/projects/infinitegarden` (built from a scratch copy; the workspace has no build) | core | 77 of 192 |
| `garden-hand-bp-1` | same | same | breakpoint boundary 1022/1023/1024 | 9 |

`core` = initial, reduced-motion initial, scroll down 50/100 % and up 50/0 %, a 12-step Tab sequence, the first
menu / tab / hover states per route and viewport, fixed-clock entrance at 0 and 1200 ms, the custom 1000×800
viewport (Garden). Every skipped scheduled state is listed with its reason in each run's `REPORT.md`.
Viewports are the captured ones (1440×900 and 375×812; the earlier Natural harness used 390). Microsoft AI
was not run in this report (no run was scheduled for it); it is a coverage gap, not a pass.

## Reproducing the earlier Natural measurements

The collector's inclusion rule for the old harness is built in (`legacyComparable`): it yields exactly the
earlier element counts, **585 (/ at 1440), 575 (/ at 390→375 here), 192 (/pay)**, so the two pipelines look at
the same elements; the new one additionally records html/body, wrappers with effects, tab order, fonts, etc.

| State | Elements (all / legacy rule) | Major items | Minor items (named equivalences) |
|---|---|---|---|
| `/pay` 1440 and 375, initial, reduced, 4 scroll states, entrance 0/1200 | 215 / 192 | **0** | 13: 3 `href-trailing-slash`, 7 `redundant-tabindex-0`, 3 `zero-position-unit` |
| `/request` (same) | 215 / 192 | **0** | 13, same breakdown |
| `/` 1440 initial | 647 / 585 | 40 | 70: 36 `gradient-direction`, 19 `redundant-tabindex-0`, 11 `zero-position-unit`, 4 `href-trailing-slash` |
| `/` 375 initial | 640 / 575 | 40 | 70 (same) |

So `/pay` and `/request` reproduce the earlier result (only trailing-slash differences, plus two equivalences the
old harness either normalised silently or did not look at: `0%`↔`0px` background positions and an added
`tabindex="0"` on native controls; both are now counted by name instead). `/` does **not** reproduce "~13 minor
differences" as a clean match: 40 major items remain, in three clusters, each a real difference between the
original and the compat export at that state:

1. **Scroll-reveal timing.** `api-terminal-enter` (a 0.11 s CSS animation, 9 elements around y=5333): the original
   leaves it unrevealed (`opacity:0`, translated 8 px) until it is scrolled to; the export has already played it
   (`opacity:1`, `animation-fill-mode: forwards`) at load. Collected at the same state, after the same settle rule.
2. **The hero canvas.** The export draws a static snapshot (`data-mochi-surface="static-snapshot"`, pixel hash
   differs, original reads back blank because WebGL is not readable without forcing it); reported, never judged.
3. `fact.animations.running` (the list of animations differs consequently).

Nondeterminism control: the original loaded twice in the six initial states differs in **0** items, so these are not
timing noise on the original side.

## What the other states show (Natural, `/`)

26 states differ, 24 are `not_exercised`, 32 are minor-only (all `/pay` and `/request` non-interactive states).

* **Hover menus differ substantially.** Hovering `Products` / `Features` opens a mega-menu (63 / 17 more elements in
  the original). In the export the panel's contents are laid out elsewhere (`Wallets` at x=1584 instead of x=120,
  15 px instead of 16 px, `white-space: normal` instead of `nowrap`, `transition-property: all` instead of `color`);
  the largest geometry delta is >1000 px. This repeats on `/pay` and `/request` (the shared nav).
* **Tabs** (`Claude Code`, `Codex` on `/`): the panel switch changes layout differently (a 3,638 px displacement; 57
  elements missing and 58 extra): the export's tab panels are not laid out like the original's.
* **Scroll states** (`/`, 50 % / 100 % / back up): 270–1,256 major items; dominated by scroll-linked
  opacity/transform values (`effOpacity` 0.5 vs 0.2, `translateY` 4 px, bar-chart `scaleY(0.92)`) and
  `benefit-bar-in` animations that run in the original but not the export at that scroll offset. **These counts are not reproducible
  across runs**: the same state with the same code gave 270 major items in `natural-compat-core-1` and 1,132 in
  `natural-compat-scrollnoise-2`, although within a run the original loaded twice and the candidate loaded twice
  each agree with themselves (0 differing items, desktop and mobile 50 % scroll). Scroll-linked state depends on
  something the harness does not pin (wheel-event pacing under machine load is the suspect); report the classes of
  difference, not the totals, and do not average them.
* **Keyboard** (Tab ×12): on `/pay` and `/request`, tab order and focus rings match except the redundant
  `tabindex` (now an equivalence) — those states are minor-only after the final comparison rules; on `/` the Tab
  sequence scrolls the page and inherits the scroll-linked differences above.
* **Click-driven menu states are `not_exercised`**: `click` on the nav buttons times out on **both** sides (those
  menus open on hover and the buttons are not click-stable). These states prove nothing; they are listed and counted as
  not exercised, never as passes.
* Elements in the executed states: 30,638 reference elements; 26,266 paired without any difference; 3,918 paired with
  differences; 454 missing in the export; 227 extra. Largest geometry delta 3,638 px (tolerance 1 px).

## Infinite Garden, recovered export

All 77 executed states differ, but narrowly: 16,361 of 18,438 elements are paired without any difference, only
**6 of 77 states have a geometry delta above 1 px** (largest 441 px), 45 missing and 39 extra elements in total.
What differs:

* `href` of the contact link in all 77 states (the original's Cloudflare e-mail-protection URL vs a plain `mailto:`):
  a real difference (and the original requests a script the capture does not contain).
* **SVG paint/structure hashes and canvas pixels** (graphics, 1,295 items): these also differ between two loads of
  the *original* (`svgPaint`, `svgHash`: animated SVG), so most are animation noise; 4 of the 6 control states are
  noisy. The report tags each item that also differs in the control.
* `transition-duration` 0.48 s vs 0.24 s and a different `transition-timing-function` on headings (18 states);
  list numerals rendered `rgb(105,105,105)` in the original and `rgb(0,0,0)` in the export (9 states); popup state
  (`aria-expanded` true vs false) and the sort menu content (2 states); 5 + 5 missing/extra elements in the
  cognitive-health card menu.

## Infinite Garden, hand-built project (the known-broken case)

Every state differs by a wide margin: 24 of 18,438 elements paired cleanly, 9,978 missing and 5,741 extra, largest
delta 11,017 px. At the 1000×800 viewport the original has 481 elements and the project 371 (325 missing / 215 extra,
max delta 6,940 px). The engines diagram region shows the failure concretely: the original's six 22.5×22.5 icon SVGs
(`viewBox 0 0 20 20`) at y≈5462–5639 are rendered 0.9×1.58 px in the project; the engine labels sit 210–300 px lower; the
canvas (896×1003.5) and the diagram SVG (540×1003.5, `viewBox 0 0 360 669`) are displaced by +209 px. The 1022 / 1023 / 1024
boundary states show the same (1022/1023 identical to each other; 1024 changes the element count 481→525). The pipeline
therefore **does detect** the known break at ~1000 px; it was not given the answer (the inventory and the states are
derived from the capture).

## Rust gate (`mochi check-parity`)

`gate` built contract/reference/candidate files for each run and ran `check-parity`: Natural 651,999 facts over 82 states,
276 declared gaps; the gate reports `failed` (6,569 differences) — and, as designed, could never pass because the
contract declares gaps. In the minor-only `/pay` and `/request` states the **only** failure is `reference unsettled or
browser errors` (the original logs a failed third-party SDK script because that host is blocked); no fact differs.
Reports (`*-gate/check-parity-report.json`) are retained.

## Calibration (sensitivity of the instrument)

See `calibration-natural-9/CALIBRATION.md` (final, passed) and the kept failing/aborted runs `calibration-natural-1`,
`-2-FAILED-noisy-baseline`, `-3-FAILED-instrument-gaps`, `-4`, `-5-ABORTED-superseded`, `-6-CRASHED-fullpage-screenshot`,
`-7-ABORTED-slow-pixel-proof`, and run 8 (`calibration-natural-8`, FAILED: the identical control of the Natural keyboard state
was noisy that time). Their history is the
list of defects found in the instrument while calibrating it (a sub-pixel control that also changed a style, a handler-removal
that a delegated listener survived, a baseline judged by item counts, breakpoint shifts that were vacuous or whose "no-op" proof
was unsound, a font-loading nondeterminism, an under-powered settle rule, a wheel scroll that overshot differently on the two
sides). 

**Final run (`calibration-natural-9`, Natural `/`)**: PASSED, 87 mutation rows.
* The original loaded twice is identical (0 items) in every state used (initial, hover, keyboard, the 48 breakpoint boundary states used, plus three more).
* 36 of 36 planted regressions that must be detected were detected, **and** seen by the Rust-gate facts: opacity, wrapper opacity,
  font-variation, letter-spacing, text-align, image bytes, background colour / image, body and html background, border, radius,
  box- and text-shadow, injected text, edited text, removed section, hidden menu, SVG hide / path / viewBox / fill, canvas size,
  ARIA label / hidden / expanded, link href, heading tag, explicit role on a div, pseudo content and pseudo mask, transition, a 2 px
  page shift, hover rule (in a hover state), focus ring as outline and as box-shadow (in a keyboard state).
* The 3 documented blind spots stay undetected where expected (hover rule and focus ring in an initial state; a handler removal).
* **Not calibrated**: removed click handlers. Every click-driven menu state on Natural is inert (the click times out on both
  sides), so no menu state could host the control; the mutation is reported not applicable. The same fact limits what the
  menu states mean in the runs above.
* **Changed breakpoint**: 3 of 16 clusters (480, 767 and 1023) produced a detected difference; one (899) does not exist
  in any stylesheet this route loads; **12 are unproven** (rules were rewritten, nothing recorded changed). They may be no-ops on
  this route or blind spots; the independent pixel oracle that would decide is implemented but was not usable here (it cost over a
  minute per row and the headless browser crashed during screenshots on this loaded machine), so it is opt-in. Do not read
  "3 of 16" as 19 % sensitivity or as 100 %: it is unknown.
* The identical control of the Natural keyboard state (12 Tab presses) was **not** clean in run 8 (138 items between two loads
  of the original, scroll-triggered reveals caught mid-animation) and was clean in run 9: that state is intermittently noisy.
  Calibration therefore uses the first state whose control is clean and lists the noisy ones.

## Coverage gaps (not covered by any state)

Listed per run in `REPORT.md` and in each contract. In short: WebGL/canvas pixels (the Natural hero is a canvas) beyond
the static comparison; click-driven menus on Natural (not exercisable); pointer-driven effects; per-frame scroll-linked values
between the scripted offsets; forms; third-party scripts (blocked on both sides); touch/mobile emulation and hover/pointer media
queries; the other color scheme; font bytes; shadow-root/iframe content; 226 Natural and 115 Garden scheduled states that the
`core` profile deliberately skipped (including every breakpoint boundary on Natural and routes 2–3 of Garden); and Microsoft AI.

## Adversarial review

Three Opus 5.5 high reviews of the code, tests and docs. Confirmed findings fixed: page background (html/body) never compared; crashed
states dropped from the gate/rediff; identically failing interaction steps reported as identical; gate dropping facts the report counts
(image/SVG/video attributes, tabindex on links, focus order, step failures, reached scroll); breakpoint states not straddling
`max-width`; pseudo mask bytes; locator mismatch/clamping; inventory blind spots (details/summary, dialog, switch roles, height /
container / touch queries); weak settle rule; calibration detection by item count / vacuous breakpoint shifts / unsound no-op proof /
undisturbed-run proof; over-broad `container-retag`; collector coverage holes (+ truncation recorded); stale docs; an unpaused
virtual clock; sink writes from any origin; gate/TS invariant gaps; html/body pseudo-elements; error-origin matching;
rediff robustness. Deferred on purpose and documented in `docs/PARITY.md` ("Known limitations of the pipeline itself").
