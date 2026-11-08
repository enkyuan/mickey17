# Validation

## Benchmarks

Measured on 2026-11-04 at `2d0f526`, Apple M1, macOS 27.0.1, Rust 1.99.1. Criterion 0.8.2,
release build, 30 samples, 2-second warm-up and 4-second measurement per case.

| Benchmark | Input | Estimate | Reported interval |
| --- | --- | ---: | ---: |
| `replace_160KiB` | 159,744 bytes of HTML, protected asset paths | 1.6788 ms | 1.6119–1.7652 ms |
| `flight_4096_rows` | 4,096 length-prefixed text rows | 923.39 µs | 903.10–950.30 µs |

The first benchmark's historical name rounds its 156 KiB input size. Both measure in-memory
helpers; neither includes filesystem writes, Chromium or network traffic. These are local
observations, not speedup claims or crawl-throughput guarantees.

```sh
cargo bench --locked --bench editing -- --sample-size 30 --warm-up-time 2 --measurement-time 4
```

Criterion writes reports to `target/criterion/`. Release builds retain symbols for CPU profiling.

## Memory

The DHAT workload edits 512 pairs of JSON/text Flight rows and strips tracker rows per
iteration. It warms up for 100 iterations, then repeats five batches of 100.

| Check | Observed result |
| --- | --- |
| Retained heap before / after | 8,752 / 8,752 bytes; 49 / 49 blocks |
| Peak live heap | 368,073 bytes |
| Allocations across the run | 11,070,941 |
| macOS `leaks`, Chromium integration test process | 0 leaked bytes |

These results cover the Rust editing workload and test process, not Chromium's child-process
heaps or long-running crawls. Reachable regex caches remain allocated after warm-up.

```sh
cargo run --locked --release --example heap_profile           # assert no retained growth
cargo run --locked --release --example heap_profile -- --record
```

The recording run writes `target/dhat-heap.json` for the DHAT viewer. On macOS:

```sh
leaks --atExit -- target/release/examples/heap_profile
sample <pid> 5 -file target/cpu-sample.txt
```

## Checks

```sh
cargo fmt --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked
cargo test --locked --test browser -- --ignored
cargo audit
```

The local suite passed 20 non-browser tests and one explicitly executed Chrome integration
test. Coverage includes Flight escaping and byte lengths, protected paths, collision reporting,
queue bounds, interrupted writes, server ranges, and crawling with persistent requests.
The dependency audit reported no advisories for the locked dependencies.

[CI](.github/workflows/ci.yml) runs checks on Linux and macOS. The
[release workflow](.github/workflows/release.yml) builds Linux, macOS and Windows binaries;
this document records local measurements, not a cross-platform performance claim.

## Structural analysis development checks

The reconstruction foundation was verified locally on Apple M1 with Rust 1.99.1:

- The initial full run passed 32 regular tests and two explicitly executed Chromium tests.
  After three additional tests were added, the scoped analysis, cache, and CLI run passed
  all 15 tests. The final full-suite rerun passed all 35 regular tests with no failures;
  its two browser tests were ignored and had been explicitly executed in the initial run.
  Coverage includes
  ETag/Last-Modified revalidation, query identities, content deduplication, corrupt-body
  recovery, cache-control/redirect changes, concurrency limits, structural distinctions,
  mixed-text ordering, namespaces, snapshot/report path checks, and the total-node budget.
- The browser fixture produced one structural template from two product pages, captured
  JavaScript-rendered text, reused validated assets on the second clone, and preserved
  cache objects when the first clone was edited.
- Formatting, Clippy with warnings denied, and the locked dependency audit passed.
- An actual CLI fixture cloned two pages twice with zero resource failures, reused three
  validated asset bodies, and successfully regenerated its analysis after the origin was stopped.

A single synthetic offline run processed 200 snapshots containing 50,400 nodes and 10,000
repeated card subtrees in **1.077 seconds**. This was a debug build and includes process
startup, JSON parsing, hashing, grouping, and report I/O. The fixture has a body/main wrapper
and 50 identical-structure, five-node cards per page; page content varies independently of
structure. It yielded one template and a card candidate with 10,000 occurrences. This is
an initial local observation, not a statistical benchmark or a crawl/source-generation speed claim.

Build and replay captured snapshots with:

```sh
cargo build --locked
time target/debug/mochi analyze <clone-captured-with-analyze>
```

These checks cover the development checkout. The published 0.2.0 package predates this feature;
no new registry release or remote CI result is represented by these local observations.

## Source evidence development checks

The initial-state source-evidence contract was verified locally on Apple M1 with Rust
1.99.1 and installed Google Chrome:

- `cargo test --locked`: 40 non-browser tests and one doctest passed. Four browser tests
  are ignored by that command and were executed separately.
- `cargo test --locked --test browser --test evidence --test source_browser -- --include-ignored`:
  all four real Chrome tests and five source-evidence boundary tests passed. The existing
  clone/edit/serve and shared-cache browser fixtures still pass.
- The rich fixture preserves text beyond the analysis snippet limit, mixed text order,
  Unicode, SVG attribute namespaces, live input/textarea/checkbox states, custom properties,
  media/supports rules, pseudo-elements, and font-face descriptors. Two query URLs were
  captured at 375x812 and 1440x900; computed font sizes changed at the CSS breakpoint and
  all four PNG hashes/dimensions were checked. An actual CLI invocation captured and
  validated a separate one-viewport clone.
- Small configured node/depth/style/rule/value/byte budgets produce explicit incomplete
  captures. Cross-origin CSSOM access, HTTP 404 CSS, shadow content, and nonempty password
  values are not silently treated as complete evidence. Password values were omitted.
- Invalid graphs/metadata, missing snapshots, unlisted partial snapshots, mismatched
  inventory, changed screenshot bytes, oversized inputs, and symlinked files are rejected.
  Failed inspection preserves the previous complete report; offline validation fails for
  recorded incomplete captures. Repeating capture cannot overwrite prior files.
- Formatting, Clippy with warnings denied, workflow linting, and documentation link checks
  passed. CI is configured to execute both browser test files on its existing platforms;
  these notes record local execution, not a new hosted-CI result.

The source schema describes observed initial light-DOM/CSSOM states. It does not establish
unobserved interactions, complete theme coverage, font binary metadata, arbitrary-site
fidelity, a working exported application, or live Jev behavior. No source exporter/model
integration was exercised, and no registry release was published by these checks.

## Project model development checks

The deterministic project-model stage was verified locally on 2026-11-08 with the same
Rust/Chrome environment. `cargo test --locked` passed 45 non-browser tests and one doctest;
the separate ignored-browser run passed all four Chrome tests.

- Five model tests cover literal query routes, exact shared shells, shared style lookup,
  local font/image references, ambiguity of query-bearing mirror paths, external assets,
  original runtime observations, and exact content reconstruction from prototype plus props.
  Checks enforce non-overlapping selections. Prop text includes
  Unicode strings longer than the old analysis snippet limit.
- Rebuilding frozen evidence produces identical JSON bytes, including via the actual
  `model-project` CLI. Local candidate asset changes alter the input fingerprint. A component
  moving between viewport positions cannot establish reuse by itself; incompatible semantic
  attribute values prevent alignment.
- Invalid source identities/style/component/layout/navigation references and changed embedded
  content fail model validation. Incomplete captures, corrupt PNGs, oversized local assets,
  and symlinked asset/output paths fail rebuilding without replacing the prior model.
- The real two-route/two-viewport Chromium fixture also builds and validates the model,
  preserves the 12px/20px style variants and query states, resolves `site/style.css` as a local
  candidate, and produces identical output on rerun. Existing clone/edit/serve/cache fixtures
  remain green. This establishes fixture behavior, not arbitrary-site export fidelity.

Formatting, Clippy with warnings denied, workflow linting, local documentation links and
`git diff --check` also passed. Final asset-reference validation changes were rechecked
with all five model tests, Clippy and formatting after the full regression run.

Source emission, framework builds, semantic token naming, Base UI control reconstruction,
and Jev calls are still unimplemented. These checks are local; no new package publication,
GitHub release, or hosted CI result is implied.
