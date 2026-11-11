# Design tokens

`mochi tokens` (module `src/semantic/tokens/`) turns the styles of one capture into a
single central styles file and rewrites the component stylesheets to reference it. It does
not change `mochi export`; nothing calls it by default.

```sh
mochi-tokens <capture-dir> --out <new-dir>      # stand-alone binary today
# mochi tokens <capture-dir> --out <new-dir>    # same code, to be registered in src/main.rs
```

`<capture-dir>` is a capture (it holds `reconstruction/pages/*.json`). `--out` must not exist.
`--min-uses N` (default 2) sets how often a value must repeat before it gets a new token;
`--no-merge-rem-px` keeps px and rem lengths apart. If the static equality check finds any
difference the command prints the first ones, exits non-zero and writes nothing;
`--allow-differences` writes the output anyway (report status `static_differences_found`).

## What it writes

| File | Content |
| --- | --- |
| `src/styles/tokens.css` | The central file: one `@theme static { … }` block, sectioned by category. |
| `src/styles/rewritten/<name>.css` | Every captured stylesheet, pretty-printed, with literals replaced by `var()` references and hoisted site variables removed. Names are the URL stem without the bundler hash. |
| `tokens.manifest.json` | Machine-readable tokens: name, category, CSS properties, merged literals with their normalized key, Tailwind utility per property, origin, naming basis. |
| `tokens.report.{json,md}` | What was merged, named, left literal and why; resolver and equality results; explicit gaps; the environment assumptions used. |
| `tokens.equality-harness.js` | Browser harness (below). |

### Why Tailwind v4 `@theme static` + CSS variables

The generated projects already run Tailwind v4; `@theme` is where its namespaces
(`--color-*`, `--spacing`, `--radius-*`, `--text-*`, `--font-*`, `--shadow-*`, `--ease-*`,
`--tracking-*`, `--leading-*`) generate utilities, while plain `var()` works in any CSS file,
including raw `?inline` imports that skip Tailwind. `static` stops Tailwind pruning variables
no utility mentions, which would break the `var()` references in rewritten component CSS.
StyleX was not chosen: it compiles per component and cannot express the captured cascade
(selectors, layers, media, nesting) without hand conversion, so it would be less readable
here, not more.

## Pipeline

1. **Parse** (`css.rs`, `selector.rs`): `cssparser` (already in the dependency tree through
   `scraper`, MPL-2.0; no new crate; `lightningcss` is not vendored and would add a large
   tree) builds a real AST: rules, nesting, at-rules, keyframes, selectors with specificity.
   Unparsable text is kept as `Raw` with the reason, never dropped. All three example
   captures parse with zero raw nodes. Unmodified declarations and selectors are written back
   from their source text (token trees are only re-serialized for values that were replaced), so
   numbers, `U+` ranges, selector case and attribute flags cannot drift.
2. **Cascade** (`cascade.rs`, `media.rs`): importance, `style=""`, `@layer` order (normal and
   reversed for `!important`), specificity, source order, CSS nesting, `@media`/`@supports`
   evaluated against each snapshot's viewport and color scheme. Anything undecidable
   (`@container`, `@scope`, vendor `@supports`, unknown pseudo-classes) is `Unknown`: the
   rule never selects, and the properties it declares are marked *uncertain* and excluded
   from equality counts.
   `<link media>` conditions become an explicit `@media` wrapper in the rewritten file and make
   the sheet's variable definitions conditional (a media value that is not a plain media list,
   or a sheet with both a media condition and an `@import`, is refused); disabled sheets are
   ignored. Sheets captured as raw text instead of CSSOM rules keep their values (only hoisted
   definitions move), because invalid declarations cannot be told from valid ones there.
   `:host` rules never count as the root (Tailwind's `:root, :host` does).
3. **Resolver check** (`verify::validate_resolver`): the cascade is compared with the computed
   style the browser reported for every element and `::before`/`::after`, after expanding the
   box shorthands, substituting `var()` from the captured computed custom properties and
   evaluating `calc()`.
4. **Inventory** (`recognize.rs`, `inventory.rs`): one walker finds tokenizable values
   (colors, spacing, radius, font family/size/weight, tracking, leading, shadows, border
   widths, easing, duration, z-index) with their property, the component (`subject`) of the
   selector, and how many captured elements use them. Utility classes (`px-4`, `hover:…`)
   never become subjects.
5. **Plan** (`plan.rs`): merges by unit-normalized key (see below), reuses the site's own
   custom properties, derives names, builds the spacing scale, decides what stays literal.
6. **Rewrite** (`rewrite.rs`): rebuilds the AST with replacements; rule order, layers, nesting
   and declaration order are preserved. Hoisted site variables leave their original rules.
7. **Check** (`verify::verify_rewrite`): before/after cascades compared per element and
   pseudo-element (see Equality).

### Merging

Keys are equal only when the browser computes the same value:

* sRGB colors by 8-bit channels and alpha (`#fff` = `white` = `rgb(255 255 255 / 100%)` =
  `hsl(0 0% 100%)`); other color spaces (`oklch`, `color-mix`, …) by their own numbers and
  never with sRGB; `transparent` is never tokenized.
* absolute lengths in CSS px (`12pt` = `16px`); `rem` joins only when the root font size is
  the same in every snapshot and no rule changes the root element's font size under a
  media/supports/container condition (reported; `--no-merge-rem-px` disables). `em`, `%`,
  viewport units keep their unit. Merging px with rem is computed-equal at the captured root
  font size but changes behavior if a user changes the browser font size; tokens that merged
  both are marked `merged_across_px_rem` and scale conversions are counted in the report.
* quoted generic family names (`"serif"`) never merge with the generic keyword.
* times in ms, numbers to four decimals, font families case-insensitively, shadows layer by
  layer, easing functions by their numbers.

### Names

Priority, first free name wins, most used value first:

1. the site's own custom property (kept verbatim; only when its value is the same in every
   definition and every definition sits unconditionally on `:root`/`html`/`:host`, so a
   themed variable never absorbs literals);
2. page role (`--color-foreground`/`--color-background` from `html`/`body`);
3. usage subject: the component that dominates (≥60 %, ≥2 declarations) the declarations
   using the value, e.g. `--color-nav-surface`, `--radius-card`, `--z-header`;
4. a standard scale (`--text-sm`, `--radius-lg`, `--leading-tight`, `--font-weight-medium`,
   `--tracking-wide`, `--ease-in-out`), matched by value;
5. a palette name from hue family and lightness (`--color-blue-500`, `--color-white-a50`);
6. the value itself where that is the convention (`--duration-200`, `--font-weight-360`,
   `--text-13px`, `--radius-20px`) or a description of the geometry (`--shadow-y4-blur12`).

No rank ladders: `--text-2xl` or `--radius-lg` would contradict Tailwind's meaning of those
names, so sizes outside Tailwind's defaults are named by value. A derived name never reuses
any custom property the site defines, hoisted or not.

Derived tokens in Tailwind namespaces (`--color-*`, `--spacing`, `--radius-*`, `--text-*`)
override Tailwind's defaults of the same name inside `@theme`: the generated project's theme
is the site's theme.

Names are never hashes or sequence numbers (an inline stylesheet's output file is named after
the component of its first rule; the numeric suffix only appears when two files would
collide). Where no defensible name exists (z-indexes without a dominant component,
fractional sizes, non-standard curves) the value stays literal and the
report lists it under "Eligible but unnamed" with the reason.

### Spacing

One base unit and a scale, as Tailwind v4 does: `--spacing: 0.25rem` (or the site's own),
steps written `calc(var(--spacing) * 4)`. A value joins the scale when it is a multiple of
the base (halves allowed, |n| ≤ 96); everything else stays literal and is counted. If the
site defines a named spacing variable with the same value, that name wins. If the site owns
`--spacing` in a form that cannot be reused (scoped, themed, rem with a varying root), no
scale is derived. Declarations converted between px and rem by the scale are counted in the
report (`converted_between_px_and_rem`).

### Site variables

Constant, root-defined custom properties move into the central file (they are cascade-neutral
to move). Variables that are redefined in scoped/conditional contexts (themes, media,
components), `!important`, registered with `@property`, framework-internal (`--tw-*`, `--_*`,
`--radix-*`), dependent on `env()`/`attr()`, or referencing such a variable stay in place;
`tokens.report.md` lists each group with its reason.

### Breakpoints

Collected from `@media` width conditions and named (`--breakpoint-md` for Tailwind's
defaults, otherwise `--breakpoint-<px>`). Media conditions cannot contain `var()`, so queries
stay literal; the tokens are documentation and Tailwind inputs. This is an explicit gap.

## Equality checks

* **Static** (always run, in the report). For every element and pseudo-element of every
  snapshot, the winning declarations before and after are compared by exact normalized
  signature (colors by channel, absolute lengths in px, `calc()` evaluated). Before, `var()`
  is substituted from the captured computed custom properties; after, from the *rewritten
  cascade itself with inheritance*, so a local override that shadows a token is seen, and a
  `var()` that resolves on one side only is a difference. The "after" cascade is built by
  re-parsing the files exactly as written. Declarations that were not touched and reference no
  token count as equal without substitution. Mutation tests prove the check catches a wrong
  token value, a wrong scale step, a dropped declaration, a shadowing override, a `var()`
  fallback that starts resolving, and a variable defined on only one page.
* **How much it covers.** Most compared declarations are untouched ones. The report states how
  many comparisons involve a rewritten declaration or a token definition
  (`rewritten_comparisons`), and lists derived tokens that no compared element references
  (`unexercised_tokens`): their replacement rests on the value merge alone. `:hover`,
  `:focus`, `:active` and similar are treated as not matching, so rules for them and rules for
  elements absent from every snapshot are never compared. Properties that depend on an
  undecidable rule are skipped and counted (`skipped_uncertain`).
* **Browser** (`tokens.equality-harness.js`): records every computed property of every element
  and of generated `::before`/`::after` (shadow roots keyed by host path); `compare(before,
  after, { introduced })` reports missing/extra elements and per-property differences (exact
  unless `tolerancePx` is given). Custom properties listed in the manifest's
  `defined_properties` are accepted as additions because the central file is site-wide. Run it
  on the original and tokenized project with identical viewport, color scheme, reduced-motion
  emulation and fonts:

  ```js
  const a = await mochiTokenEquality.snapshot();   // save as JSON from each page load
  const b = await mochiTokenEquality.snapshot();
  mochiTokenEquality.compare(a, b, { introduced: manifest.defined_properties });
  ```

  or `bun tokens.equality-harness.js a.json b.json tokens.manifest.json` (exit 1 on any
  difference). `snapshot()` has not been executed in a real browser by the authors of this
  module (no scriptable browser was available); `compare()` and the command line are covered by
  `bun test scripts/tokens/computed-equality.test.js`.

Report status is `static_checks_only_not_browser_verified` until the browser harness has run;
the tool never upgrades it by itself. `static_differences_found` and `static_check_vacuous` (a
snapshot where nothing was compared) make the command exit non-zero without writing, unless
`--allow-differences` is given. Resolver mismatches against the capture are reported
(`resolver_mismatches`) but do not change the status: they measure the cascade model, not the rewrite.

## Programmatic use

```rust
let output = mochi::semantic::tokens::pipeline::run(capture_dir, &Options::default())?;
let token = token_set.token_for("background-color", "#0a0a0a"); // Option<&Token>
```

`pipeline::run` returns the files and the typed `Report`; the emitters can use
`TokenSet::token_for(property, value)` or the manifest.

## Known gaps

Reported by every run (`gaps` in `tokens.report.json`): only the captured color scheme is
verified; resting states only (no hover/focus/open); lengths inside functions, percentages and
viewport units are untouched; sRGB colors compare by 8-bit channels and alpha, lengths/line-height/opacity to four decimals, oklab/oklch to three, other numbers to six (merging and the equality check share this precision); inline `style` attributes, script-set styles, `@font-face`
descriptors and `@property` registrations are not rewritten; shadow DOM and iframe surfaces are
not covered; `@container`, `@scope` and vendor `@supports` conditions are undecided; hover/
pointer media features assume a desktop capture (listed in the report).

# Style translation: computed author styles to Tailwind utilities

`mochi-tokens <capture> --out <new dir> --styles reconstruction/pages/page-00000-1440x900.json`
(also `tokens::styles::run::translate`) translates the **author styles of one captured page**
into the classes its emitted elements carry, on top of the token pipeline above. It runs the
token pipeline first and translates the tokenized stylesheets, so utilities reference the
central `@theme` tokens. `--tailwind-dir` points at a Tailwind v4 package (default: found in
`MOCHI_TAILWIND_DIR`, else below `.mochi/mochi-exports` relative to the working directory; pass `--tailwind-dir` when running from elsewhere. Without a Tailwind package or `bun` the check falls back to this crate's own model of the utilities and the report says so).

## What it produces

| File | Content |
| --- | --- |
| `src/styles/tokens.css` | the central tokens (above) |
| `src/styles/recipes.css` | named recipes: `.name { @apply … }` for utility sets shared by ≥ 2 elements, plain CSS for what utilities cannot order (shorthand + longhand in one variant, unsafe values) and for `::before`/`::after` |
| `src/styles/at-rules.css` | `@keyframes`, `@font-face`, `@property` … carried verbatim |
| `styles.classes.json` | capture node index -> class string (what `emit::StyleBridge` returns) |
| `styles.report.{json,md}` | counts, equality result, what was not translated, explicit gaps |

`StylePlan::bridge()` implements `semantic::emit::StyleBridge` (`tokens::bridge`), keyed by
capture node index; elements it did not cover keep their original classes and are counted.

Project setup: one Tailwind compilation must see everything, because `@apply` in `recipes.css`
only resolves inside it. Use a single entry stylesheet and import Tailwind's theme and
utilities only, **not** preflight (the captured site's own reset is part of the author styles
that were translated; a second reset would disagree with it):

```css
@import "tailwindcss/theme.css" layer(theme);
@import "tailwindcss/utilities.css" layer(utilities);
@import "./tokens.css";
@import "./at-rules.css";
@import "./recipes.css";
```

A file that is compiled separately must start with `@reference "<entry>.css";` to use `@apply`.
The proof in the report compiles utilities with the real Tailwind package; recipes written as
plain CSS need nothing.

## How

1. **Layers.** For every element the cascade engine resolves the author declarations at a
   representative width of every interval between the media thresholds in the sheets, and with
   one of hover / focus / focus-visible / active forced on that element. Box shorthands are
   expanded so layers compare property by property.
2. **Placement.** Mobile-first deltas: the narrowest layer is the base; each wider interval adds
   `min-[Wpx]:` utilities for what changed (a declaration that disappears becomes `revert`);
   state layers add `hover:` … utilities for what the state changes, carrying overrides across
   intervals until a delta cancels them. `focus-visible` builds on `focus`.
3. **Classes.** Each declaration becomes a named utility only when its Tailwind expansion is
   exactly that declaration (display, position, flex/align/justify keywords, overflow, text
   alignment/transform, cursor, spacing and sizing on the `--spacing` scale, token colors,
   radii, families, opacity, z-index); everything else is an arbitrary property
   (`[prop:value]`, which Tailwind defines as exactly that declaration) and is counted in the
   report by property. Utilities that add more than the declaration (`font-*`, `leading-*`,
   `shadow-*`, `border`, `transition`, `text-<size>`, `select-*`, `px`/`py`, `-z-*`) are never
   used. A theme that defines `--spacing-0`, `--container-4` … changes what `p-0`/`max-w-4`
   mean in Tailwind, so those keys fall back to arbitrary properties.
4. **Recipes.** Elements with identical utility sets and a component class (`nav__link`, not
   `px-4`, not `is-open`) share a recipe named after it; elements with pseudo-elements or
   unorderable declarations get a plain-CSS recipe on the same hook. When no component class
   exists the hook is `styled-<tag>`; name clashes are separated by the parent's component class
   and, last, a numeric suffix (both counted in the report).

## Equality

With a Tailwind package present the proof is by the cascade engine, not by a model of Tailwind: the CSS the project would
produce is **compiled by a real Tailwind** (`tailwindcss` 4.3.5 in the runs below), parsed, and
cascade-resolved for every element at every sampled width (each interval's representative, each
threshold and the pixel before it, the captured width) and each forced state (hover, focus, focus-visible, focus-within, active; only elements whose
styles change in some state are re-checked per state, pseudo-elements only at rest), with the elements
carrying their new class names and no inline `style`. The result must equal the declarations the
original tokenized stylesheets resolve to in the same environment, value by normalized signature.
Without a Tailwind package the same check runs against this crate's model of the utilities, and
the report says so. The model itself is tested against a real Tailwind build for every utility
it renders (`utilities_expand_exactly_like_a_real_tailwind`).

Measured (page 0, 1440×900 primary capture with the page's other captured viewports as
overlays, Tailwind 4.3.5; static, not browser-verified). Widths with the same active rules are
compared once, so counts are lower than in the first version of this table; the unverified column
is comparisons the check does not count (values recovered or settled from the captured computed
style, and variables the site never defines):

| Site | Elements | Widths | Comparisons | Different | Not counted | Against another viewport's capture |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Natural | 997 | 48 | 2,147,074 | 0 | 17,993 + 126 | 6,951 |
| Infinite Garden | 1,207 | 21 | 161,255 | 0 | 7 | 2,287 |
| Microsoft AI | 1,378 | 56 | 5,007,075 | 0 | 56,027 + 59,559 | 6,832 |

The 375×812 capture follows for widths 374–418 (Natural), 361–760 (Garden) and 0–479 (Microsoft AI):
the run of width intervals that activate the same rules as the captured width; other widths use the
primary capture's cascade.

Translation time on the same machine under heavy load (load average 20–50 on 8 cores, 4
threads): Natural 83 s, Infinite Garden 3.5 s, Microsoft AI 84 s (was about 2, 0.1 and 4
minutes). `MOCHI_TOKENS_THREADS=n` sets the worker count, `MOCHI_TOKENS_TIMING=1` prints the
phases.

The check found real problems while it was built (Tailwind's `-z-1` is `calc(1 * -1)`,
`select-none` adds a `-webkit-` declaration, `p-0` is `0px`, a theme with `--spacing-0` changes
`p-0`, `!important` winners overriding an inline style): each was fixed or excluded and is covered by a test.

## Explicit gaps (also in every report)

- Pseudo-elements other than `::before`/`::after`, and state selectors on an ancestor
  (`.card:hover .title`) or on a pseudo-element, are not translated (counted, with examples).
- Only one state at a time is forced, on the element itself; combinations and states set by
  script (open, selected, `aria-*`/`data-state` toggled at runtime) exist only as the capture
  shows them. Tailwind orders state variants in its own fixed order, not the author's source
  order, so when two states are on at once (pressed + hovered, focused + hovered) a property
  they both set can resolve differently from the original; the check cannot see it.
  `:has(<state>)` selectors are counted with the ancestor-state rules and not translated.
- Attribute selectors (`[data-state=open]`, `[aria-expanded=true]`, `[data-pressed]`) are resolved
  against the attributes as captured and baked into each element's classes; styles that depend on
  an attribute a script toggles later (open/pressed menus, Base UI's `data-open`/`data-popup-open`
  in place of the original's `data-state`) are only as captured, never re-evaluated in the build.
- `hover:` utilities and hover recipe states are wrapped in `@media (hover: hover)`; the
  original's sticky `:hover` on touch devices is not reproduced.
- Viewport height, orientation, resolution, color scheme other than the captured one and
  container queries are not varied; properties behind undecidable conditions are skipped.
- **Motion.** `@media (prefers-reduced-motion: reduce)` is translated as a state: every element is
  resolved under reduced motion at each width interval and the difference becomes `motion-reduce:`
  utilities (or an `@media` block in a plain-CSS recipe), checked like the hover states. Tailwind
  sorts `motion-reduce:` before `min-[W]:`, so the placement simulates what applies under reduced
  motion and restates it (`min-[W]:motion-reduce:`) where a wider plain utility would beat it.
  `animation`/`transition` declarations and `@keyframes` (central `at-rules.css`) are carried as
  written; scroll-driven `animation-timeline`, view transitions and script-started animations are
  not reproduced. Reduced motion combined with hover/focus is not translated.
- **Script-driven end states.** Two sources say an element ended somewhere other than the
  stylesheet's pre-reveal declaration, and the end state is written wherever the declaration
  still has the author value (`settled_from_computed`; the check does not count these):
  1. The captured computed `opacity`/`visibility`/`transform`/`translate`/`scale`/`rotate` differs
     (a GSAP/WAAPI reveal that ran before capture). Elements listed in the capture's
     `animation_states` may be mid-flight (infinite or unfinished animations are sampled too), so
     for them only a clean end value is taken (opacity exactly 1, identity transform). A transform
     that is identity only because a custom property is 0 at rest (`translateY(var(--y, 0px))`)
     is not a reveal.
  2. The element is still at `opacity: 0` and the sheets say it is a once-only reveal
     (`reveal_pre_state_elements`): it is visible and displayed, does not swallow pointer events,
     **no rule shows it through a hover/focus/active/open/selected toggle** (also on an ancestor or
     sibling: `.cta:hover .arrow`, `.slide.is-active`), and either a reveal rule exists
     (`.x.is-revealed`, `[data-revealed]`, `.in-view`), or its opacity is 1 under
     `prefers-reduced-motion`, or it carries a `reveal`/`in-view`/`fade-in` attribute or class
     token. The end values come from the reveal rule (`opacity: 1; translate: none`) or the
     reduced-motion layer; an offset that neither clears is kept. Reduced motion alone counts as
     evidence only when it also switches the motion off (`transition: none`). Rules are matched
     with their toggling words removed in every compound, so `.logos.is-revealed .logo` is about
     the logos under `.logos` only. Measured on the original Infinite Garden page (scrolling it in
     a browser): the story kickers and copy, index rows, logos and footer items reach opacity 1
     and stay there; the hover arrows, panels with `visibility: hidden` and pointer-less popovers
     are left hidden. Reveals driven only by a script (no reveal rule, no motion-skipping
     reduced-motion rule, no reveal-ish name), `.is-visible`-style popovers and class-toggled
     slides stay hidden. Toggles are read at the captured width and applied to every width layer
     that still carries the author value; the overlay layers of other captured viewports follow
     the same rule but are not counted separately. On the Garden home page the footer items
     and index rows inside `visibility: hidden` roots (`.home-footer.is-finale`, pinned cards) are
     still hidden: only their opacity is fixed.
  The reveal itself is not reproduced. Elements hidden by `visibility: hidden` or `display: none`
  (pinned story steps, carousels, responsive duplicates) stay as captured, because showing them
  all at once would stack them; the report counts text elements visible / hidden by opacity /
  visibility / display (`text_visibility`, author declarations only: browser defaults such as the
  `hidden` attribute are not read).
- **Other captured viewports.** Snapshots of the same page at narrower widths
  (`page-00000-375x812.json` next to `…-1440x900.json`) are used as overlays: elements are paired
  across the two DOMs (tag, identity attributes and classes without `--modifier`/`is-`/`has-`/`js-`
  suffixes, in matching order); for each pair the narrower capture's own author layer replaces the
  primary's in the run of width intervals that activate the same rules as that capture's width, and
  the check compares the output there with the capture's layer, not the primary's. Hover/focus/
  active and reduced-motion layers at those widths are resolved on the other capture's own DOM and
  cascade. Siblings with equal keys that one capture has more of than the other are left unpaired;
  elements present in only one capture, pseudo-elements, widths between captured viewports (or
  intervals shared with the primary) and captures whose stylesheets add width thresholds the primary
  lacks are not overlaid. Classes a script toggles over time (an active carousel slide) are taken
  as captured, and script-toggled classes outside the captured intervals are unknown.
  `StyleOptions::viewport_overlays` turns this off.
- `!important` is flattened. A declaration the capture serialized with an empty value (a `var()`
  shorthand artifact of the CSSOM, e.g. `animation-duration: ;`) is never written. Where the
  element's captured computed style has that longhand, its resting value is used instead,
  frozen at every width and state: an approximation, listed per property
  (`recovered_from_computed`, `recovered_in_states`) and **not counted as verified** (the output
  carries the same text, so equality would be a self-comparison; `recovered_unverified`).
  Without a computed value the declaration is listed and omitted.
- Properties behind conditions a capture cannot decide (container queries, unknown
  pseudo-classes) are removed from the output, including their unconditional value; the report
  status is then `static_check_incomplete`. A custom property the site reads but never defines
  is listed (`undefined_in_original`) and carried unchanged.
- Properties that share a longhand with a shorthand, or are spelled `<shorthand>-…`
  (`background`/`background-position`, `font`/`font-variant-numeric`, `grid-row`/`grid-row-start`)
  stay in plain CSS in the author's order, because Tailwind sorts arbitrary properties by its own
  table and would put a shorthand after the longhand it resets.
- The emitter integration asks the bridge only for elements that already carry a class attribute;
  classless elements and `html`/`body` styled by tag or descendant selectors are not asked, so
  their translated classes must be applied by the emitter, and JS hook classes (`js-*`,
  `is-open`) are replaced along with the rest unless the emitter keeps them.
- `@keyframes` and friends are carried verbatim; `animation`/`transition`/`mask`/
  `animation-timeline` declarations are translated as exact arbitrary properties but their
  behavior is not verified. SVG presentation attributes and canvas/WebGL drawing are not CSS.
- Flattening the cascade puts the whole reset on every element, so classes per element are
  high (see `average_per_element`); recipes absorb repeats only where a component class names them.
- The browser harness has not been run against a translated project in a real browser.
  `tokens.equality-harness.js` evaluates like the parity collector (Playwright `evaluate`,
  Browser pane `javascript_tool`) and is idempotent; compare an original page and the generated
  app with identical viewport, color scheme and reduced-motion emulation, passing the manifest's
  `defined_properties` as `introduced`.
