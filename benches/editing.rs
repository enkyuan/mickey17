use criterion::{Criterion, criterion_group, criterion_main};
use mosha::{edit::Replacer, flight};
use std::hint::black_box;
fn editing(c: &mut Criterion) {
    let r = Replacer::new("Acme", "Globex", true, true).expect("replacer");
    let text = "<h1>Acme</h1><img src=\"/acme/logo.png\">".repeat(4096);
    c.bench_function("replace_160KiB", |b| b.iter(|| r.replace(black_box(&text))));
    let stream = "1:T4,Acme".repeat(4096);
    c.bench_function("flight_4096_rows", |b| {
        b.iter(|| flight::map_rows(black_box(&stream), &|s| s.replace("Acme", "Globex")))
    });
}
criterion_group!(benches, editing);
criterion_main!(benches);
