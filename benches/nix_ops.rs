//! Criterion microbenchmarks for the warm Nix operation path and the pure
//! request-hot-path helpers.
//!
//! The Nix benchmarks build a [`NixWorker`] once (Nix init is process-global and
//! one-shot) and then time individual handlers, so they measure the warm,
//! in-process latency the server is built around rather than startup cost. They
//! need a live Nix store, so run them from the dev shell: `cargo bench`.

use std::hint::black_box;
use std::time::Duration;

use criterion::{Criterion, criterion_group, criterion_main};
use nix_mcp::tools::{ClosureType, Page};
use nix_mcp::worker::NixWorker;

/// A moderately-sized pure evaluation: sum 1..=1000 via `foldl'`/`genList`.
const EVAL_FOLD: &str = "builtins.foldl' (a: b: a + b) 0 (builtins.genList (x: x + 1) 1000)";

/// Pure, allocation-free helpers that run on every request.
fn bench_pure(c: &mut Criterion) {
    let mut group = c.benchmark_group("pure");

    group.bench_function("page_new", |b| {
        b.iter(|| Page::new(black_box(5_000), black_box(1_000), black_box(100)));
    });
    group.bench_function("closure_type_flags", |b| {
        b.iter(|| black_box(ClosureType::Runtime).flags());
    });

    group.finish();
}

/// Warm, in-process Nix operations on a single shared worker.
fn bench_nix(c: &mut Criterion) {
    let worker = NixWorker::new().expect("initialize Nix worker");

    let mut group = c.benchmark_group("nix_warm");
    group
        .sample_size(30)
        .measurement_time(Duration::from_secs(3));

    group.bench_function("store_info", |b| {
        b.iter(|| worker.handle_store_info().expect("store_info"));
    });
    group.bench_function("eval_small", |b| {
        b.iter(|| {
            worker
                .handle_eval(black_box("1 + 1"), None, 10_000)
                .expect("eval")
        });
    });
    group.bench_function("eval_fold_sum_1000", |b| {
        b.iter(|| {
            worker
                .handle_eval(black_box(EVAL_FOLD), None, 10_000)
                .expect("eval")
        });
    });

    group.finish();
}

criterion_group!(benches, bench_pure, bench_nix);
criterion_main!(benches);
