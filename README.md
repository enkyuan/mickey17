# Mochi

Clone a website's frontend with Chromium, then edit and serve the local copy. Written in Rust.

## Use

Requires Rust 1.99.1 and Chrome or Chromium.

```sh
cargo install mochi-web --locked
# Or install a checkout:
cargo install --path . --locked

mochi clone https://example.com -o example
mochi serve example
mochi replace example 'Example' 'Demo'
mochi rehost example https://example.com https://demo.example
mochi strip-trackers example

# Development checkout installed with --path . (not registry 0.2.0):
mochi clone https://example.com -o example-analyzed --analyze --jobs 4 --cache-dir ../mochi-cache
mochi analyze example-analyzed

# Richer evidence for the source-export pipeline (development checkout):
mochi clone https://example.com -o example-source --source-evidence --max-pages 20
mochi check-evidence example-source
mochi model-project example-source
```

Clones contain `site/` and `manifest.json`. Use a new or empty output directory and check the
manifest for failed resources. `--extract` adds text, metadata and screenshots; `--max-pages`
defaults to 200. Trackers and robots rules can be overridden with `--keep-trackers` and
`--ignore-robots`. Set `MOCHI_CHROME` if browser discovery fails.

The crates.io package is `mochi-web`; the executable and Rust library are named `mochi`.

## Output

The current clone is a mirror of deployed files, with paths based on captured URLs:

```text
example/
  manifest.json
  site/
    index.html
    about/index.html
    _next/static/...       # when present on a Next.js site
    assets/...            # original asset paths vary by site
  extract/                # only with --extract
    page-00000.json
    page-00000.txt
    page-00000.png
  analysis/               # only with --analyze
    pages/page-00000.json # rendered DOM and selected computed styles
    report.json           # templates and repeated-subtree candidates
  reconstruction/         # only with --source-evidence
    manifests/page-00000.json
    pages/page-00000-375x812.json
    screens/page-00000-375x812.png
    report.json           # offline validation and completeness summary
    project.json          # offline model, after model-project
```

It does not generate components, hooks, framework source routes, or a buildable frontend
project. CSS and same-site fonts are copied as assets; there is no component-library,
token, Tailwind, or StyleX source generator. External assets can still depend on their
original hosts.

[Editable source project design](docs/SOURCE_PROJECTS.md) describes the proposed reconstruction
pipeline, framework-specific layouts, component primitives, fonts, and styling adapters.
That pipeline is not implemented yet.

## Structural analysis

These options are available in the development checkout; the published 0.2.0 release
predates this analysis pipeline. Install the checkout with `cargo install --path . --locked`.

`clone --analyze` captures rendered DOM after JavaScript and lazy loading, then produces
`analysis/report.json`. `mochi analyze <clone>` rebuilds that report offline from the saved
snapshots. Ordinary clones retain their mirrored-file output and do not capture DOM snapshots.

The analyzer hashes normalized DOM subtrees bottom-up with SHA-256. It preserves namespaces, child and text-slot order,
selected semantic attributes, and twelve computed CSS properties. Text values, asset URLs,
IDs, and class names remain in the snapshots but do not define structural identity. Page
templates also distinguish viewport dimensions. Repeated subtrees are ranked as component
candidates with source page, snapshot, and node references; no components are generated or
automatically named. Similar structures with different child counts are separate templates.

This captures the light DOM, not shadow-root or iframe internals, stylesheet rules, or pseudo-elements.
It is evidence from one viewport and the state reached during the visit. It does not establish
responsive, interactive, or semantic equivalence. Captures are limited to 20,000 nodes, 4,096
distinct style records, 256 levels, and 8 MiB per page. Text snippets are capped at 512 UTF-16
code units. Snapshot URLs are limited to 8 KiB. Offline analysis accepts at most 1,000 snapshots and hashes up to 250,000 nodes;
the report labels truncated captures and pages excluded by that budget.

`--jobs` bounds concurrent asset downloads to 1–16 (default 4); page visits and output writes
remain sequential. Downloads still have the 64 MiB response limit, so buffering can approach
`jobs × 64 MiB` plus parsing overhead. The explicit `--cache-dir` stores asset bodies by
content hash and indexes exact URLs, including queries. Reuse always requires server
validation with ETag or Last-Modified; there is no offline/stale fallback. The cache skips
redirected responses, `private`, `no-store`, `Vary`, and `Set-Cookie` responses, and verifies
cached body hashes before use. `manifest.json` records successful cache revalidations.
Pages and RSC responses are fetched normally. The cache has no automatic disk eviction yet.

## Source evidence

`clone --source-evidence` saves a separate source-evidence schema. It preserves ordered
element/text nodes without shortening text, namespaced attributes, live control values,
metadata, interned computed styles/custom properties, readable CSSOM rules, font-face
descriptors, active `::before`/`::after` styles, resource observations, and viewport PNGs.
It captures the initial state at 375x812, 768x1024, and 1440x900; override with, for example,
`--source-viewports 375x812,1440x900`. Sizes are CSS pixels with device scale 1; this changes
viewport dimensions, not the browser's user agent or touch mode. The observed color scheme
is recorded; themes and controls are not automatically toggled. Fonts get a bounded readiness
wait. Screenshots are references, not proof that an application was reconstructed.

`check-evidence <clone>` runs offline, validates the expected captures against per-page
viewport manifests and the clone inventory, verifies screenshot hashes and dimensions,
and refreshes `reconstruction/report.json` atomically. Missing/changed files or invalid
graphs fail validation without replacing the previous report. Inspectable incomplete
captures are reported, and the command exits unsuccessfully until every expected capture
is complete within its declared initial light-DOM/CSSOM scope. Files are created without
overwriting existing captures. Old `--analyze` snapshots do not count as source evidence;
the library exposes an explicitly lossy projection from complete source captures to the
existing analysis schema.

Limits per capture: 16 MiB JSON, 20,000 nodes (including text), 4,096 computed style records,
256 levels, 128 stylesheets, 20,000 CSS rules, 10,000 resource observations, and 1 MiB per
text/attribute/CSS value. There are 1–8 distinct viewports, each dimension 200–4096 pixels;
`max-pages × viewports` must be at most 1,000. Offline inspection caps total nodes at 250,000.
Screenshots are bounded at 64 MiB each. A limit marks evidence incomplete or rejects the
capture rather than silently clipping content and claiming completeness.

Blocked/unloaded stylesheets, missing images/fonts, open shadow roots, embedded frame/canvas/video
content, and omitted sensitive control values are reported as issues. Closed shadow roots,
other interaction states, animation timelines, and arbitrary-site fidelity are outside this
capture scope. Font declarations and resource observations do not verify font binary metadata,
licensing, or availability of every remote asset. Query variants remain distinct in source
capture; the mirror retains its existing file-path collision reporting.

`model-project <clone>` derives `reconstruction/project.json` offline from complete evidence.
It preserves literal URLs/queries and full observations, pools styles by exact content hash,
finds structural component candidates, extracts varying text/attribute/control props, and
records viewport style variants. Shared shells require identical siblings around one `<main>`
across at least two routes. Component selections cannot overlap. Repeated viewports alone
cannot establish reuse. Unselected nodes remain in the model as evidence.

The asset registry records local candidate file hashes under `site/`, external/unresolved
resources, and original runtime observations. Query-bearing asset URLs remain ambiguous
because mirror paths omit queries. Candidate hashes verify the local bytes, not their origin.
The model has a 64 MiB aggregate source JSON budget, 128 MiB output budget, 256 MiB aggregate
local asset budget, and at most 256 selected component candidates. Failed validation or an
input changing during a rebuild preserves the previous model. See [the model contract](docs/PROJECT_MODEL.md).

These are capture, validation, and modeling features for the development checkout. The source
exporter, semantic token naming, Base UI generation, and Jev interpretation remain future
steps; capture, inspection, analysis, and model building make no AI calls.

## Capabilities

- **Browser capture.** Scrolls to discover lazy assets; saves server HTML for hydration.
  Persistent background requests do not block document readiness.
- **Safe editing.** Rewrites Next.js Flight JSON and UTF-8 text lengths alongside HTML,
  protects asset URLs, and atomically replaces each edited file.
- **Bounded downloads.** Deduplicates up to 100,000 asset URLs, limits responses to 64 MiB,
  and uses bounded parallel batches with a 30-second request timeout. Conflicting paths are reported.
- **Structural evidence.** Optional DOM fingerprints, exact page-template clustering, and
  repeated-subtree candidates, with offline replay and inspectable source references.
- **Streaming preview.** Loopback server with clean URLs, RSC navigation, original-image
  routing, HEAD requests and byte ranges for media.

## Performance

Local release-build measurements on Apple M1, Rust 1.99.1:

| Workload | Time |
| --- | ---: |
| Replace text while protecting URLs, 156 KiB | 1.68 ms |
| Rewrite 4,096 Flight text rows | 923 µs |

These measure in-memory editing, not website cloning speed. The heap workload retained no
additional bytes after warm-up. [Validation](VALIDATION.md) has methodology, memory results
and commands to reproduce the checks.

## Limits

Frontends only: forms, APIs, embeds and external CDN assets may still need their original
servers. Authenticated or interaction-only resources may be missed. Tracker removal uses
signatures; arbitrary-site visual parity is not established.

Keep served files under your control. Path checks reject traversal and symlinks but do not
isolate concurrent local filesystem changes. Edits are atomic per file, not across a whole clone.

## License

[MIT](LICENSE).
