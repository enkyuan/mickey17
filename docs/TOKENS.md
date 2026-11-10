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
