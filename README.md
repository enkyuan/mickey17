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
