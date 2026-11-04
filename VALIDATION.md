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
