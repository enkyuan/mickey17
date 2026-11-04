# perfectclone

A Rust CLI that crawls public websites with Chromium, mirrors their original HTML and assets,
and edits the result with awareness of Next.js React flight payloads.

## Build and use

Install Rust with rustup and install Chrome or Chromium. The repository pins Rust 1.99.1
(edition 2024), including rustfmt and Clippy. No Python or Node runtime is required.

```sh
cargo build --release --locked
cargo install --path . --locked
perfectclone clone https://example.com -o example
perfectclone serve example
perfectclone replace example 'Example' 'Demo'
perfectclone rehost example https://example.com https://demo.example
perfectclone strip-trackers example
```

Set `PERFECTCLONE_CHROME` to the browser executable if automatic discovery fails.
The browser uses a temporary profile and certificate validation. It is closed when the crawl ends.

`clone` accepts `--max-pages 200`, `--extract`, `--keep-trackers`, and `--ignore-robots`.
It requires a new or empty output directory. It follows same-site links breadth-first, seeds
from sitemap locations, respects robots page rules, scrolls to trigger lazy assets, and records
same-site response URLs. DOM readiness is independent of network idleness, so persistent
background requests do not prevent a loaded page from being saved. HTTP fetches save server HTML
rather than the rendered DOM, and scan HTML/CSS/JS references recursively. Resources are fetched
sequentially with a 30-second timeout
and a 64 MiB response limit; discovery is capped at 100,000 URLs. Failed URLs are recorded in
`manifest.json`. Inspect that manifest before treating a snapshot as complete.

The output contains `site/`, `manifest.json`, and optionally `extract/` with text, metadata,
headings, links, images, forms, JSON-LD, and full-page PNGs. Query variants sharing an asset path
are checked for byte collisions and reported rather than silently overwriting each other.

`replace` protects HTTP URLs, root-relative paths, mailto and telephone targets, matches whole
words, and includes upper/lower/capitalized variants. Use `--exact-case` or `--substring` to change
those defaults. HTML flight data and standalone `.rsc` streams are edited together, including
UTF-8 byte-length updates for long text rows and rows split across chunks. Invalid flight lengths
return errors. Non-UTF-8 files are left unchanged. Edits stage a complete sibling file before atomic
replacement, preserving the original on write failure. The multi-file command is not transactional.

`serve` binds only to loopback, tries the requested `-p/--port` and the next 19 ports, and supports
clean URLs, RSC navigation, Next.js image originals, HEAD requests and streamed byte ranges.
Traversal and symlink paths are refused. Keep the output directory under your control while serving;
filesystem checks do not provide isolation from a concurrent local process changing files.

## Validation and profiling

```sh
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --locked
cargo test --test browser -- --ignored
cargo bench --bench editing
cargo run --release --example heap_profile
cargo run --release --example heap_profile -- --record
cargo install cargo-audit --locked
cargo audit
```

The browser test uses a local fixture with two pages, lazy-loaded assets, robots exclusion,
extraction, editing, and preview serving. It needs installed Chrome and loopback networking.
The benchmark measures replacement on ~160 KiB and flight processing over 4,096 rows.
The DHAT executable warms shared regex caches and asserts that five repeated editing workloads
retain no extra allocation bytes or blocks. The separate `--record` run writes
`target/dhat-heap.json` for the DHAT viewer.
These checks cover the measured workloads, not every input or Chromium's own heap.

For macOS process-level checks:

```sh
leaks --atExit -- target/release/examples/heap_profile
sample <running-perfectclone-pid> 5 -file target/cpu-sample.txt
```

Release builds retain symbols for CPU profiling. See [VALIDATION.md](VALIDATION.md) for the
observed results and environment limitations from the migration.

## Limits

This copies frontends, not backends. Forms, API calls and embeds may still contact their original
servers. Tracker removal uses signatures and does not guarantee zero outbound tracking.
Browser capture records response URLs, then fetches them again without browser authentication;
one-time, authenticated, POST-only and state-dependent responses may not replay.
Dynamically assembled URLs, interaction-only chunks and external CDN assets may remain remote.
robots rules apply to page discovery; dependencies of allowed pages may still be downloaded.
No bot-protection bypass or cross-site redirects are implemented. Visual parity across arbitrary
frameworks and sites has not been established for this rewrite.

Licensed under MIT; original copyright attribution is retained in LICENSE.
