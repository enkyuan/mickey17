# Parity report: default-recovery compat exports and a semantic assemble output

Measured 2026-11-10 with the pipeline of commit `02c1bfa` (no pipeline code changed for this report; the recorded
`pipelineSha256` and collector hash match that commit) on a heavily loaded machine (load average observed at ~12–15),
headless Chromium 158, Playwright 1.65, SwiftShader GL. Originals are served by `mochi serve <capture>` on 8000;
candidates are production builds (`bun .output/server/index.mjs`) on 3000.
**Evidence for the declared states of these builds only**: not a screenshot comparison, not a score, nothing about other
states, routes, browsers or sites. Raw captures, diffs and reports are in `.mochi/parity-runs/` (git-ignored; run names
below); stopped or superseded runs are kept and labelled.

Candidates (built 03:30–03:31 by the default-recovery commit `37a4dd3`, not rebuilt here):
`.mochi/mochi-exports/natural-compat` (3 routes recovered; 3 client programs), `.mochi/mochi-exports/infinitegarden-compat`
(home recovered; `/articles` and the article route are static replicas), and a fresh `mochi semantic --assemble` output
(commit `60b99aa`; the generator reported 3 routes, 36 assets, 0 not copied; `bun install --frozen-lockfile`, `bun run check`
and `bun run build` all exited 0; the logs were not retained and the project was deleted after measuring).

## Reading the numbers

* **Settling.** A state is `unsettled` when the page did not come to rest under the settle rule on at least one side
  (this machine is slow enough that it happens to the *original* as often as to the export). An unsettled state is not a
  pass even with 0 major items, and several observations below rest on unsettled states; each says so.
* **Counts of "items"** in property tables come from the first 40 items kept per state (`top`) unless a total is stated.
* **The "previous export" column** (`natural-compat-core-1`) was captured with an earlier, uncommitted revision of the
  capture/settle code (the diff logic is the same: a rediff of those captures with the current code reproduces every number
  quoted), so it is a comparison of two builds measured under slightly different harness revisions and different state sets.

## Natural (default-recovery compat export)

Runs `natural-dr-static-1` (34 states) and `natural-dr-scroll-1` (12 states): 46 of 308 scheduled states. A first 82-state
run (`natural-default-recovery-core-1-ABORTED-slow`, 14 states done and reported, 68 not run) was stopped because the states
took several minutes each; the two targeted passes replaced it. In `natural-dr-scroll-1` the **original** is the slow side
(desktop 100 % scroll 7.96 min on the original vs 1.27 min on the export; mobile back-to-top 19.6 vs 0.58 min), so the slowness
is the original page plus the loaded machine, not the recovered scripts.

| | Previous export (`natural-compat-core-1`) | Default-recovery export |
|---|---|---|
| `/` initial, desktop 1440 and mobile 375 | 40 major items | **0 major**, 47 minor (`gradient-direction` 36, `zero-position-unit` 11) |
| `/` reduced motion | 50 / 48 major | mobile 0 major / 39 minor; desktop 0 major but `unsettled` |
| `/pay`, `/request` initial / reduced | 0 major, 13 minor | 0 major, 3 minor (`zero-position-unit` only); **no trailing-slash `href` and no `tabindex` difference remains in any of the 46 states** |
| `/` scroll 100 % desktop | 1,123 major | 1 major: the hero canvas pixel hash |
| `/` scroll 50 % mobile | 544 major | 0 major (from the stopped first run, whose per-state summaries are valid) |
| `/` scroll 100 % mobile, back to top mobile | 1,039 major | unsettled on the original side (not a result) |
| `/` tab `Codex` (desktop) | 396 major, 57 missing, 58 extra | 138 major, 0 missing, 0 extra, largest delta 4 px |

Counts over the two passes: 13,513 + 4,281 reference elements; 12,784 + 4,068 paired without any difference; **0 missing,
0 extra** in every state; largest geometry delta 4 px (the `Codex` tab state only); 139 major items (138 + 1).
In the `Codex` state the original plays the `agent-graphic-in` animation (opacity 1) where the export shows the element
mid-reveal (opacity 0.4, translateY 4 px, animation `none`), the same class of difference as before but much smaller.
Controls: the original loaded twice and the candidate loaded twice are each identical in the 6 initial states of the static
pass. Counter-evidence kept: in the stopped first run `home.mobile375.initial` was `different` (5 major: opacity/transform
of animated elements, original side unsettled and its control noisy), so the "0 major at `/` initial" result is not
perfectly repeatable on this machine.

Does **not** establish:
* **11 of 46 states are `unsettled`** (reference side involved in 7), including both fixed-clock entrance states and desktop
  reduced motion; they show 0 major items but are not passes.
* Not run: click-driven menu states (they time out on both sides, as before), the other 262 of 308 scheduled states (most
  scroll steps, every breakpoint boundary), and anything about the hero canvas beyond one pixel hash (WebGL readback was not
  forced on Natural).

## Infinite Garden (default-recovery compat export; home recovered)

Runs `garden-dr-static-1` (47 states), `garden-dr-scroll-1` (24) and `garden-dr-bp-1` (3: widths 1022, 1023, 1024 at height
900), WebGL readback forced on both sides; 74 of 192 scheduled states. Statuses: 73 `different`, 1 `not_exercised`. **46 of the
74 states are `unsettled` on at least one side** (static: 15 on both, 13 reference only; scroll: 8 + 7; all 3 breakpoint
states on both), which includes nearly every home state behind the canvas and engines observations below, except the
reduced-motion states. Every state differs, mostly in the same few classes as for the previous recovered export:

* `href`: the original's Cloudflare e-mail-protection links vs plain `mailto:hello@infinitegarden.ai` in every state; on the
  article route also six table-of-contents anchors that became absolute (`/articles/commercial-intelligence-with-infinite-memory#section-…`),
  i.e. 8 `href` items per article state.
* SVG paint/structure hashes: the dominant item class (scroll pass: 288 `svgPaint` + 126 `svgHash` major items in total;
  the 3 breakpoint states: 78 + 30). They are animation noise in the initial states (the original differs from itself by 56
  items, the candidate from itself by 52–56) and are probably noise elsewhere, but the settled reduced-motion home states
  still show 21 (desktop) and 26 (mobile) such items and have no control, so "noise" is an inference there.
* Transition timing: `transition-duration` / `-timing-function` differ on `li`, `p`, `h2`, `h3` and others (static pass 180 + 180
  items over 10 states; scroll pass 125 + 125 over 6 home states); values include 0.48 s→0.24 s, 1.2 s→0.24 s, 0.48 s→0.4 s
  and 1 s→0.4 s.
* Article table of contents: the highlighted entry differs in both directions (original greys "01 / Introduction", export
  greys "06"), with 36 `style.color` items (6 article states × 6 elements, and the same for the other colour properties) and 12 `aria-current` items in the scroll pass.
* Geometry: 4,501 major items in total (static 3,055, scroll 1,323, breakpoint 123). Elements missing / extra: 71 / 68 in the
  static pass, mostly the `cognitive-health` card menu states (deltas of 398–804 px) and `entrance.t0` (35 missing, 38 extra,
  62.8 px, 1,212 major items); 2 / 2 in the scroll pass (deltas ≤ 2 px; `home.desktop1440.scroll.down.50` 2 px and
  `home.vp1000x800.scroll.down.100` 1.1 px from animated spans). The 3 breakpoint states and `home.vp1000x800.initial`
  have 0 missing, 0 extra and a largest delta ≤ 0.01 px.
* A popup state (`aria-expanded` true vs false) and the sort menu content on `/articles`. In the sampled `/articles` initial and
  scroll states the 4 major items are two `href` and the two animated-SVG hashes.

The three observations from the main session, on the home route:
1. **Hero canvas.** With readback forced on both sides, canvas 1 has pixel hash `116400` at the initial and back-to-top states and
   `c54e57` at 50 % and 100 % scroll, **identical in the export and in the original in every home capture of the three runs**.
   That is consistent with live drawing, but the hash is not a function of scroll alone (`c54e57` also appears at scroll 0 in the
   reduced-motion state, the only settled home state), and the four normal-motion states sampled are all unsettled on both
   sides. One frame per state; no time series.
2. **Five canvases; no `data-mochi-surface`.** Original and export each have 5 canvases in every state, and no export canvas
   carries a `data-mochi-surface` attribute (`data-mochi*` is on the collector's recorded list; other `data-*` names are
   not recorded, so nothing is claimed about them; 0 of 148 Garden captures show `data-mochi*` on any canvas). Canvases 2–5 are
   blank and equal at the initial state; at 50 % scroll canvases 2–4 and at 100 % and back at the top canvases 2–5 have different
   pixel hashes from the original's, in the same size: live content at a different frame, not shown to be wrong.
3. **Engines diagram.** At 1022, 1023 and 1024 × 900 and at 1000×800 the export has exactly the original's element count (481 /
   481 / 525 / 481), 0 missing, 0 extra and x/y/width deltas of 0 for the 8 engine labels, and neither side has overlapping
   labels, **but** in those initial states the label spans are captured before their reveal animation (1.5–5 px wide), so this
   shows equal *positions*, not an equal rendered diagram. The meaningful evidence is `home.vp1000x800.scroll.down.50` and
   `.100`, where the labels are at full size: deltas 0 and no overlaps on either side, with 1 / 1.1 px deltas elsewhere on the
   page (animated spans). There is no revealed-diagram evidence at 1022–1024. (Height 768 was not used; boundary states are 900
   high.) The remaining differences in those states are `href`, SVG hashes and `opacity`/`scale` of animated spans.

## Semantic assemble output (`mochi semantic <capture> --out DIR --assemble`)

Run `natural-semantic-assemble-1`: 30 states (initial, reduced-motion initial, a 12-press Tab sequence, scroll 100 % and back
to top; 3 routes × 2 viewports), all `different`; no control states; 3 states unsettled on the original side. Expected to differ
a lot (no styles bridge yet); counts by class over the 30 states:

* **Structure:** of 10,680 reference elements, 6,965 are missing in the output and 6,657 extra in it; 3,715 are paired, of which
  75 without any difference. Desktop 1440 initial: `/` renders 619 elements against the original's 647 and pairs 216;
  `/pay` and `/request` 218 against 215, pairing 74. Mobile 375 initial: `/` 607 against 640, pairing 228; `/pay`, `/request`
  209 against 212, pairing 82.
* **By dimension (items):** layout 5,275, styling 1,546, behavior 754, semantics 329, motion 316, typography 222, graphics 109,
  content 60, interaction 21, assets 15, routing 5. Largest geometry delta 9,575 px.
* **By property (first 40 per state):** `bounds.y` 571, `effOpacity` 252, `listStyleType` 148, `bounds.x` 63, animation
  properties 16–20 each.
* **Assets:** the assembled project requests 9 files the original serves but the project returns 404 for (the original loads
  all of them): three blog hero JPEGs (`sarah`, `david`, `jerry`) and `pay-graphic-1..3` / `request-graphic-1..3` under
  `/assets/images/`, although the generator reported "0 not copied": a real defect to look at.
* Named minor equivalences seen: `added-component-attributes` 112, `redundant-tabindex-0` 52, `zero-position-unit` 40.

## Coverage gaps

Every run's `REPORT.md` and gap list apply. For these runs: the other 262 Natural and 118 Garden scheduled states (Natural
click-driven menus, which time out; most scroll and breakpoint states; most entrance times); Garden `/articles` interactions
beyond hover/popup; touch/mobile emulation, light scheme, pointer-driven effects, forms, font bytes; WebGL beyond one frame per
state; Microsoft AI; the Rust gate (`check-parity`) was not run on these split runs because each pass alone does not meet its
minimum state matrix. The pipeline's own limitations are in `docs/PARITY.md` ("Known limitations of the pipeline itself");
notably scroll-linked totals are not reproducible between runs on a loaded machine and `unsettled` states are not passes. Re-run
on a quiet machine before relying on any single-state number here.
