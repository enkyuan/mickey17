# Parity report: semantic assemble outputs of commit 7917885 (Natural, Infinite Garden, Microsoft AI)

Measured 2026-11-10 with the pipeline of commit `02c1bfa` (unchanged; the recorded `pipelineSha256` matches) against the built
projects `/tmp/claude-501/final-{natural,garden,msai}/project` (production builds, `bun .output/server/index.mjs` on 3000) and
the originals served by `mochi serve <capture>` on 8000 (Microsoft AI: `capture-modules`). Headless Chromium 158, SwiftShader GL.
**Evidence for the declared states of these builds only**; not a screenshot comparison, not a score. Raw captures, diffs and
reports are in `.mochi/parity-runs/` (git-ignored): `natural-sem-final-core-1`, `garden-sem-final-core-1`,
`msai-sem-final-core-1`, `natural-sem-final-rerun-1`.

Load average (1/5/15 min) was read by hand when the Natural run started (5.6 / 12.2 / 13.0; 13.3 one minute in), when the Microsoft
AI run started (2.9 / 19.8 / 23.3) and when it ended (5.3 / 6.7 / 12.4); none is stored in the run artifacts, and the Garden run
and the rerun were not read at their start. The 5- and 15-minute figures say the machine was heavily loaded around the Natural
and Garden runs; the rerun (started right after the Microsoft AI end reading) is described as "lower load", not a measured value.
Profile `core`: Natural 82 of 308 scheduled states, Garden 77 of 192, Microsoft AI 90 of 333; every skipped state is in each run's
`report.json` (`notRun`; `REPORT.md` only groups them by reason). The projects in `/tmp/claude-501/final-*/project` were generated
and built shortly before commit `7917885` was recorded (06:15–06:32 vs 06:34 local), carry no commit marker, and are called
"the outputs of 7917885" on the main session's word, not on evidence in the projects.

## Settling and repeatability

* **Natural.** 8 of 82 states were unsettled on the reference side (2 also on the candidate: the fixed-clock entrance states). They
  were rerun (`natural-sem-final-rerun-1`; 2 workers instead of 3, `project` not recorded in that run, original and candidate each
  loaded twice). The 6 non-entrance states settled. Major items: `home.mobile375.initial` 274 → 266 (the original differed from
  itself by 8 items in the first run's control: `bounds.y`, opacity, transform), `home.mobile375.scroll.down.50` 1,274 → 1,266 (the
  8 `assets` items of the candidate's own load noise), the other four identical. Their own second loads were not all clean:
  `home.mobile375.scroll.up.0` had an unsettled reference control, and the two entrance states are unsettled on both sides with
  unsettled controls (identical counts, but not passes). So the Natural class counts are stable to within a few items at the
  loads seen, except where the pages never settle.
* **Garden.** 50 of 77 states unsettled (23 reference only, 26 both, 1 candidate only; live animations). The original differs from
  itself in the four initial states, the candidate in the two home initial states. Garden numbers are indicative; not rerun.
* **Microsoft AI.** 21 of 90 states unsettled (reference only 15, both 6); the original differs from itself in the four initial states.

## Measurement caveats specific to Microsoft AI

The captured original's shared header/footer is injected by Microsoft's universal header script, which is blocked here (the
reference logs `ms_uhf_data is not defined`, `WcpConsent is not defined`, a failed dynamic import). On the reference, 128 text elements on every route and viewport (380 text elements on `/` at 1440; 326 at 375) therefore render in the browser's default `Times` at 10 px, while the
build renders its styled header/footer. About 85 % of the paired typography items and 73 % of the paired styling items fall on
those `Times`-rendered reference elements (the `uhf-header`/`uhf-footer` subtrees), so most of the Microsoft AI typography/styling
counts measure that reference artefact, not the build. A reference with those scripts allowed would be needed to separate the two.

## Natural: same 30 states as the previous assemble report, before vs now

Same state set (initial, reduced-motion initial, 12-press Tab, scroll 100 %, back to top; 3 routes × 2 viewports), summed:

| | Previous assemble (`natural-semantic-assemble-1`) | Now (`natural-sem-final-core-1`) |
|---|---:|---:|
| reference elements | 10,680 | 10,680 |
| paired | 3,715 | 6,242 |
| paired without any difference | 75 | 2,052 |
| missing in the build / extra in it | 6,965 / 6,657 | 4,438 / 4,566 |
| **layout** items | 5,275 | 3,357 |
| **styling** | 1,546 | 2,101 |
| **behavior** | 754 | 1,261 |
| semantics | 329 | 436 |
| **motion** | 316 | 3,787 |
| typography | 222 | 195 |
| graphics / content / interaction / routing / assets | 109 / 60 / 21 / 5 / 15 | 45 / 50 / 16 / 7 / 0 |
| image 404s only the build returns | 9 | **0** |
| largest geometry delta | 9,575 px | 2,334 px |

The image 404s are gone. Styling and behavior items rose roughly with the 1.7× increase in paired elements (a missing element contributes
no property differences). **Motion rose about 12×, which pairing does not explain**: 2,496 of the 3,787 items are in the six
reduced-motion states (the previous run had 50), 1,284 in scroll states and none in the initial states (previously 64); see point 3
below. Counts are items, not elements. Full property totals over these
30 states: `bounds.y` 2,191; `style.animationDuration` 1,487; `style.transitionDuration` 1,229; `inert` 692; `bounds.x` 496;
`effOpacity` 486; `bounds.h` 370; `style.opacity` 365; `style.transform` 359; `data-attributes` 339; `tabindex` 256.

Initial states across all routes, settled where noted:

| site | route | viewport | reference | candidate | paired | paired clean | missing | extra | major | settled |
|---|---|---|---:|---:|---:|---:|---:|---:|---:|---|
| Natural | / | 375 | 640 | 645 | 506 | 331 | 134 | 139 | 274 | no: reference |
| Natural | / | 1440 | 647 | 651 | 292 | 1 | 355 | 359 | 430 | yes |
| Natural | /pay | 375 | 212 | 215 | 143 | 111 | 69 | 72 | 40 | yes |
| Natural | /pay | 1440 | 215 | 218 | 99 | 2 | 116 | 119 | 114 | yes |
| Natural | /request | 375 | 212 | 215 | 143 | 111 | 69 | 72 | 40 | yes |
| Natural | /request | 1440 | 215 | 218 | 99 | 2 | 116 | 119 | 114 | yes |
| Garden | / | 375 | 480 | 523 | 208 | 32 | 272 | 312 | 935 | no: reference+candidate |
| Garden | / | 1440 | 525 | 525 | 500 | 377 | 25 | 25 | 281 | no: reference+candidate |
| Garden | /articles | 375 | 71 | 71 | 71 | 67 | 0 | 0 | 5 | no: reference |
| Garden | /articles | 1440 | 71 | 71 | 71 | 41 | 0 | 0 | 44 | no: reference |
| Garden | /articles/commercial-… | 375 | 92 | 92 | 92 | 81 | 0 | 0 | 11 | yes |
| Garden | /articles/commercial-… | 1440 | 100 | 100 | 100 | 82 | 0 | 0 | 18 | yes |
| Microsoft AI | / | 375 | 623 | 735 | 442 | 110 | 181 | 293 | 2331 | yes |
| Microsoft AI | / | 1440 | 688 | 729 | 371 | 64 | 317 | 358 | 2325 | no: reference |
| Microsoft AI | /models/ | 375 | 418 | 469 | 334 | 159 | 84 | 135 | 1760 | yes |
| Microsoft AI | /models/ | 1440 | 415 | 455 | 202 | 38 | 213 | 253 | 1684 | yes |
| Microsoft AI | /models/mai-cyber-1-1-flash/ | 375 | 441 | 548 | 282 | 73 | 159 | 266 | 1950 | yes |
| Microsoft AI | /models/mai-cyber-1-1-flash/ | 1440 | 428 | 534 | 195 | 39 | 233 | 339 | 1811 | yes |

## Top remaining classes and root-cause hypotheses

What is measured is stated as such; hypotheses are marked and are inferences, not verified causes.

**Natural**
1. **Desktop navigation stacks vertically.** *Measured:* at 1440 the build's `<nav>` is 184 px tall against 80 px; the three nav buttons are at
   x=348, y=20/56/92 where the original has a row at y=22 (x=348/435/520); the button container is 133×144 against 399×36 and is
   `display:flex; flex-direction:row` in both; the build's buttons (and its Pricing/Docs links, in a row at y=128) sit one wrapper level
   deeper; the page shifts down (document height 11,241 vs 11,137, `h1` y 656 vs 632). `bounds.y` is 2,191 items over the 30 states, 1,968
   of them at desktop. *Not measured:* the wrappers' own layout (the collector omits non-painting wrappers), so which wrapper stacks the
   buttons is unknown. *Hypothesis:* a generated wrapper around the nav items lacks the row layout.
2. **The menu-panel subtree differs in path and hiding.** *Measured:* 94 elements are `inert` in the original at initial (the panel under
   `body>div>div`) and 0 in the build, whose panel sits in a portal (`body>div#_R_…`); the build hides 75 elements by
   visibility/display (original 3, 72 of them in the portal subtree). 69 of the 355 missing and 72 of the 359 extra elements at desktop
   initial are this subtree (the same 69/72 at mobile 375, where the nav does not stack and 134 / 139 are still missing / extra). So the
   stacking nav explains the vertical shifts but **not** all of the missing/extra elements. `inert` is 692 items. *Hypothesis:* the Base UI
   popup content is mounted in a portal without the closed-state `inert`.
3. **Motion.** *Measured:* every `1e-05s` transition/animation duration item is in a `reduced-motion` state: the original sets a near-zero
   duration page-wide under `prefers-reduced-motion` (nav, main, sections) and the build's values are the normal ones (`0.15s` 193
   items, `0.1s` 102, `0.3s` 36, …). That accounts for all 1,229 `transitionDuration` items and 1,229 of the 1,487 `animationDuration`
   items; the other 258 are scroll states (`0.35s`/`0.4s` → `0s`). In scroll states the dominant missing animation is
   `benefit-bar-in` → `none` (238 items; opacity 1 → 0.4, `scaleY` 1 → 0.92; 329 of the 351 opacity items), and `agent-graphic-in`
   (20 items, including the 4 px translate). *Hypothesis:* the build does not carry the reduced-motion override or the scroll-triggered
   keyframes (the styles bridge copies resolved base styles, not those declarations).
4. **Added Base UI ARIA.** *Measured (first 40 items per state):* `aria-expanded` / `aria-disabled` / `aria-haspopup` appear on buttons
   that had none (32 / 24 / 8), plus 339 `data-attributes` items over the 30 states. Expected from Base UI; candidates for a named
   equivalence rather than defects.
5. **Whitespace between inline spans** is lost in accessible names and text: in the full diffs 30 `accessibleName` items (`1 Natural is…` →
   `1Natural is…`, h1 `payments. Wallets` → `payments.Wallets`) and 30 `text` items (`Column’s .` → `Column’s.`).

**Infinite Garden** (indicative: 50 of 77 states unsettled)
1. **Absolutely positioned diagram parts collapse onto one point.** *Measured at desktop initial:* document height and element count are
   equal (12,066; 525 / 525), with 25 missing and 25 extra. 12 are three `edge-network__slot` boxes (195×44) that the original has at
   (1181,4811), (784,4881), (1181,4881) and the build has all at (784,4811); 10 are five tiny scaled `edge-network__card` articles
   spread over x=380–1275 in the original and all at x=157 in the build; 3 are text differences (`Updated 11/10/2026` vs `Updated
   11/8/2026`: a date fixed in the build). *Hypothesis:* the per-item offsets/transforms that place these parts (probably computed by
   the original's script) are not generated.
2. **Reveal state not applied.** *Measured:* `effOpacity` / `style.visibility` / `effHidden` differ in 874 / 874 / 868 items, 845 of them
   opacity 1 → 0 (for example the `Articles` and `Explore` links), with `transform` identity matrix vs `none`. *Hypothesis:* the
   entrance/reveal behaviour is not wired, so elements stay in their pre-reveal style.
3. **Small-viewport typography.** *Measured:* `fontSize` and `lineHeight` differ in 633 items each, **620 of them at mobile 375**, 515 on
   `span` (45 on `p`, 60 on `dt`); the largest groups are 10.384 → 15 px (135), 11 → 16 (105), 12 → 16 (62), 13 → 16 (60), 10 → 13
   and 10 → 12 (55 each), and `lineHeight` 12.46 → 18 (135). *Hypothesis:* the project's semantic report records
   `viewport_policy: "widest captured viewport per route"`, so mobile sizes come out as desktop/base sizes.
4. **Canvases exist but are not drawn.** *Measured:* all five canvases have the original's sizes (1440×900, 592×648, 592×553, 592×729,
   1312×780) and every canvas in the build reads blank in every Garden capture (145 of 145 captures), while the original's non-hero
   canvases are drawn in 53 captures (at desktop initial four of the original's five are also blank; only the hero differs there).
5. **Accessible names and links.** *Measured:* 246 `accessibleName` items, about 203 of them whitespace-only differences between spans
   (`Infinite Gardenis an AI research lab…`), 12 the date difference and about 31 mispaired or different names; `href` differs in 50
   items over the 30 states, all `mailto:` vs the original's Cloudflare e-mail-protection link.

**Microsoft AI** (read with the reference caveat above)
1. **Hydration error.** *Measured:* `Minified React error #418` (HTML mismatch) is thrown by the build in 60 of 90 states (every state
   on `/` and on `/models/mai-cyber-1-1-flash/`, none on `/models/`) and is the only candidate-side runtime error. *Hypothesis:*
   server-rendered markup differs from the client render on those two routes (a date, a random id or a conditional branch).
2. **Structure of header links and headings.** *Measured:* in the full diffs `tag` / `role` link→none (a→div) is 25 items, h3→a /
   heading→link 50 each, tab↔link 30 / 30 and switch→combobox 27. `inert`: 730 items, all inside the original header's
   `#mobile-submenu-*` (690 at mobile, 40 at desktop); `h2 "Latest models"` contributes 15 of them, at mobile only.
3. **The reference's own failures** (blocked scripts) are in the caveat above; the reference also fails two dynamic imports
   (`scan-pipeline.js` in 45 states, `dot-globe.js` in 42).

## Class counts for all three sites (same 30-state set)

Same 30-state set (initial, reduced-motion initial, 12-press Tab, scroll 100 %, back to top; 3 routes × 2 viewports), summed:

| | Natural | Garden | Microsoft AI |
|---|---:|---:|---:|
| reference elements | 10,680 | 6,661 | 15,100 |
| paired / paired clean | 6,242 / 2,052 | 4,464 / 2,228 | 8,943 / 2,147 |
| missing / extra | 4,438 / 4,566 | 2,197 / 2,425 | 6,157 / 8,404 |
| layout | 3,357 | 4,543 | 13,946 |
| styling | 2,101 | 4,370 | 21,064 |
| typography | 195 | 1,903 | 16,238 |
| behavior | 1,261 | 284 | 2,176 |
| motion | 3,787 | 416 | 2,576 |
| semantics | 436 | 289 | 2,579 |
| graphics / content / interaction | 45 / 50 / 16 | 206 / 40 / 260 | 291 / 341 / 460 |
| routing / assets | 7 / 0 | 50 / 0 | 777 / 490 |
| image or file 404s only the build returns | 0 | 0 | 0 |
| largest geometry delta | 2,334 px | 9,975 px | 24,529 px |

Garden and Microsoft AI have no earlier assemble measurement in the pipeline's reports, so there is nothing to compare these with.

## Coverage gaps

The other 226 Natural, 115 Garden and 243 Microsoft AI scheduled states; Natural's 24 `not_exercised` states (menu clicks that time out on both sides); Microsoft AI's 39 (12 popup, 12 tab-click, 12 menu
and 3 hover states: timeouts, some of them 30 s `scrollIntoViewIfNeeded` timeouts on the candidate); Garden's 1
(`home.desktop1440.menu.cognitive-health13-brands.toggle-twice`); entrance and breakpoint
boundary states; touch/mobile emulation, light scheme, pointer-driven effects, forms, font bytes; WebGL beyond one frame per
state; the Rust gate (not run). See `docs/PARITY.md` for the pipeline's limits. Comparison caveats: the previous assemble numbers come from a separate 30-state run without control states, the current numbers are the
same 30 states taken from an 82-state run, at different machine loads; and three of the four largest geometry deltas (9,575 px old Natural,
9,975 px Garden, 24,529 px Microsoft AI) come from unsettled states.
