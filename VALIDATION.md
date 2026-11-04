# Rust migration validation

Observed on 2026-11-04, Apple Silicon macOS 27.0.1. These are local results; the new CI
workflows have not been run on GitHub, Linux, or Windows.

## Toolchain and implementation

Rust/Cargo 1.99.1, edition 2024. `rustup update stable` confirmed the installed release;
the [official stable release notes](https://doc.rust-lang.org/stable/releases.html) listed
1.99.1. The exact toolchain and dependency resolution are pinned in rust-toolchain.toml
and Cargo.lock. Key resolved dependencies: headless_chrome 1.0.22, reqwest 0.13.5,
axum 0.8.9, tower-http 0.6.11, Criterion 0.8.2 and DHAT 0.3.3.
Application code forbids unsafe Rust; dependencies and Chromium have their own native code.

Python implementation, packaging, scripts, tests, publishing workflow, old images and stale
documentation were deleted. MIT copyright attribution was retained. Browser-side evaluation
expressions remain embedded in Rust because the crawler operates on JavaScript documents.
Chrome/Chromium remains a separate required executable; it is not rewritten in Rust.

## Correctness

- `cargo fmt --check`: passed.
- `cargo clippy --all-targets --all-features -- -D warnings`: passed.
- Initial migration run of `cargo test --locked`: ten integration tests passed, browser test
  explicitly ignored here. The post-review suite counts are recorded below.
- `cargo test --test browser -- --ignored --nocapture`: passed against installed Chrome.
  The fixture verifies two-page crawling, robots exclusion, lazy-loaded SVG capture, full-page
  screenshot and metadata extraction, replacement, and preview serving.
- Core regression coverage includes split flight pushes, UTF-8 lengths, malformed/truncated rows,
  script-closing escapes, nonce preservation, replacement URL protection and literal dollar signs,
  rehosting boundaries, asset localization, tracker payloads, encoded traversal and symlinks.
- Mirror integration verifies original HTML and RSC bytes and asset query-collision reporting.
- Server integration verifies clean routes, RSC header/query routing, image originals and external
  redirects, byte ranges, HEAD, missing files and symlink refusal.
- `cargo audit` 0.22.2: passed; scanned 288 locked dependencies against 1,403 loaded advisories.

## Native build environment

This session inherited an SDKROOT pointing to a missing MacOSX27.0.sdk. Validation commands
used the installed SDK instead:

```sh
export SDKROOT=/Applications/Xcode.app/Contents/Developer/Platforms/MacOSX.platform/Developer/SDKs/MacOSX27.2.sdk
```

The initial build failures were SDK discovery errors. No global SDK configuration was changed.

## Memory measurements

`leaks --atExit -- target/debug/deps/core-daeee3cd04eeec13` ran all eight core tests:
zero leaks, zero leaked bytes; peak physical footprint 7,904 KiB.

`leaks --atExit -- target/debug/deps/browser-12f2a4aa2d0f6c12 --ignored` ran the complete
Chromium fixture successfully: zero leaks, zero leaked bytes in the Rust test process;
peak physical footprint 8,417 KiB. Chromium child-process heaps were not analyzed by this check.

The release DHAT assertion run warmed caches with 100 iterations, then ran five additional
100-iteration workloads. Each iteration edits 512 pairs of JSON/text flight rows and strips
tracker JSON rows. Results:

| Measurement | Result |
| --- | ---: |
| Retained bytes before / after repeated workloads | 8,752 / 8,752 |
| Retained blocks before / after | 49 / 49 |
| Maximum live allocation bytes | 363,180 |
| Total allocations across the run | 11,070,017 |

Both retained-byte and retained-block assertions passed. Shared regex caches remain live and
reachable. This demonstrates no retained growth in this workload; it is not proof of no leaks
for arbitrary sites, concurrent processes, dependencies or Chromium itself.

Raw native leak logs, heap checks and CPU samples are in `target/validation/` (ignored build
artifacts). A two-second `sample` capture of the DHAT workload found flight editing, JSON parsing
and allocator/backtrace instrumentation on the sampled stacks; timing under DHAT is not an
estimate of normal application throughput.

The separate recording run wrote a valid 522 KiB DHAT JSON trace with 1,207 allocation sites
to `target/dhat-heap.json`. Its final snapshot, after statistics output, contains 9,840 live
bytes in 51 blocks. The repeated-workload assertion above compares snapshots taken before
that output. The release CLI `--version` also passed `leaks --atExit` with zero leaked bytes.

## Performance and CPU profile

The final isolated benchmark used the release executable, 30 samples per case, two seconds
of warmup and four seconds of measurement per case. No DHAT process or build was running.
A three-second macOS `sample` capture overlapped the beginning of this run, so these are local
observations rather than a controlled performance comparison or a speedup claim.

| Workload | Criterion estimate | Reported interval |
| --- | ---: | ---: |
| URL-protecting text replacement, roughly 160 KiB | 1.3530 ms | 1.3071–1.4277 ms |
| Recompute lengths and rewrite 4,096 flight rows | 922.55 µs | 867.06–1,012.2 µs |

Sampled stacks during replacement show regex searching and replacement allocation as the
main work. The CPU sample is `target/validation/cpu-sample.txt`; benchmark output is
`target/validation/benchmark-final.txt`, with Criterion's HTML results in `target/criterion/`.
The DHAT viewer can open `target/dhat-heap.json`. Profiles cover synthetic editing workloads;
network throughput, large real sites, long-running preview serving and Chromium's memory use
remain uncharacterized. No arbitrary-site pixel-parity claim is made.

## Adversarial Rust review before PR

The first pass identified and fixed seven defect classes:

- URL paths were decoded twice during final symlink checks. URL decoding is now separate from
  validation of the exact filesystem path; a percent-named directory cannot redirect the check.
- Replacement text containing quotes/backslashes/newlines corrupted serialized Flight JSON.
  JSON string values are now decoded and serialized while protecting leading root-relative asset
  paths; long text rows remain plain text with recomputed UTF-8 lengths, including JSON-looking
  text bodies.
- Mirrored page aliases could overwrite each other. Snapshot writes now create files exclusively
  and permit existing files only when their bytes match, covering pages and RSC as well as assets.
- Pending assets were not deduplicated or bounded at admission. A discovered set now caps both
  scheduled and processed URLs; per-resource reference collection is bounded too.
- Default-port HTTP-to-HTTPS upgrades were rejected as off-site. Default HTTP(S) origins can
  upgrade on the same host; unrelated hosts and nondefault ports remain distinct.
- Rehosting an explicit nondefault port could also rewrite the default-port origin. The explicit
  old port is now required.
- Direct editor writes could truncate the original on write failure. Both editor entry points
  now write/sync a sibling temporary file and atomically persist it, preserving permissions.

Review also found navigation waiting for networkAlmostIdle in the locked browser library.
The crawler now matches DOMContentLoaded to the main frame and navigation loader; the browser
fixture leaves three requests permanently pending and still completes. The image optimizer route
also handles query strings on local original-image URLs.

Five adversarial regression tests were observed failing before fixes and passing afterward.
Final local suite: 20 non-browser tests and one explicitly executed Chrome integration test pass.
The native adversarial source review read tests/fixtures in summary mode; a separate specialist
read the tests fully. The optional outside Claude Code pass failed authentication and supplies no
outside review coverage. Linux, Windows and GitHub Actions results are pending publication.

The earlier leak/DHAT/benchmark figures above describe the initial migration snapshot, before
these review fixes. Final-candidate checks are recorded separately during PR preparation.

Documentation consulted: installed official Rust 1.99.1 std::path::Component and std::fs::write
references, plus locked headless_chrome navigation implementation. See the equivalent official
[Component](https://doc.rust-lang.org/std/path/enum.Component.html) and
[fs::write](https://doc.rust-lang.org/std/fs/fn.write.html) reference pages.
