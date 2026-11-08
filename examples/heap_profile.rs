//! Run with: cargo run --release --example heap_profile
use mosha::{edit::Replacer, flight, trackers};
#[global_allocator]
static ALLOC: dhat::Alloc = dhat::Alloc;
fn workload() {
    let r = Replacer::new("Acme", "Globex", true, true).expect("valid replacement");
    let stream = "0:[\"$\",\"h1\",null,{\"children\":\"Acme\"}]\n1:T4,Acme".repeat(512);
    for _ in 0..100 {
        let replaced = flight::map_rows(&stream, &|s| r.replace(s)).expect("valid flight");
        std::hint::black_box(
            flight::map_rows(&replaced, &trackers::strip_row).expect("valid flight"),
        );
    }
}
fn main() {
    let record = std::env::args().any(|a| a == "--record");
    let builder = dhat::Profiler::builder().file_name("target/dhat-heap.json");
    let _profiler = if record {
        builder.build()
    } else {
        builder.testing().build()
    };
    workload(); // Initialize shared regex caches before taking the baseline.
    let before = dhat::HeapStats::get();
    for _ in 0..5 {
        workload()
    }
    let after = dhat::HeapStats::get();
    println!(
        "Retained baseline: {} bytes / {} blocks; final: {} bytes / {} blocks; peak: {} bytes; total allocations: {}",
        before.curr_bytes,
        before.curr_blocks,
        after.curr_bytes,
        after.curr_blocks,
        after.max_bytes,
        after.total_blocks
    );
    if !record {
        dhat::assert_eq!(after.curr_bytes, before.curr_bytes);
        dhat::assert_eq!(after.curr_blocks, before.curr_blocks);
    }
}
