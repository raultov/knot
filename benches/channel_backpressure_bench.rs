//! Criterion benchmarks for bounded channel backpressure overhead (Phase 3).
//!
//! Measures parse-to-embed channel throughput with various capacities to
//! validate memory safety and detect buffer bloat at different sizes.
//!
//! Run with: cargo bench --bench channel_backpressure_bench

use criterion::{BenchmarkId, Criterion, black_box, criterion_group, criterion_main};
use std::path::PathBuf;
use tokio::sync::mpsc;

use knot::models::ParsedEntity;
use knot::pipeline::parser::{ParseConfig, parse_files_stream};

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn rust_fixtures() -> Vec<PathBuf> {
    repo_root()
        .join("tests")
        .join("testing_files")
        .read_dir()
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().map(|e| e == "rs").unwrap_or(false))
        .collect()
}

/// Benchmark parsing throughput with different channel capacities.
/// Validates Phase 3: bounded channels don't cause significant overhead
/// over unbounded channels while providing memory safety.
fn bench_channel_capacity(c: &mut Criterion) {
    let files = rust_fixtures();
    if files.is_empty() {
        eprintln!("No Rust test fixtures found — skipping channel benchmarks");
        return;
    }

    let mut group = c.benchmark_group("channel_capacity");

    let capacities = [4, 16, 64, 256, 1024];

    for capacity in capacities {
        group.bench_with_input(
            BenchmarkId::new("capacity", capacity),
            &capacity,
            |b, &capacity| {
                b.iter(|| {
                    let parse_cfg = ParseConfig {
                        custom_queries_path: None,
                        repo_name: "bench_channel".to_string(),
                        include_config_files: true,
                        repo_path: None,
                    };

                    let cpus = std::thread::available_parallelism()
                        .map(|n| n.get())
                        .unwrap_or(4);
                    let max_concurrent = cpus;

                    let (tx, mut rx) = mpsc::channel::<ParsedEntity>(capacity);
                    parse_files_stream(black_box(&files), &parse_cfg, tx, max_concurrent, None);

                    // Drain channel to avoid backpressure skewing results
                    let mut count = 0u64;
                    while let Ok(_entity) = rx.try_recv() {
                        count += 1;
                    }
                    black_box(count);
                });
            },
        );
    }

    group.finish();
}

/// Benchmark parsing throughput with different concurrency levels.
/// Validates Phase 5: optimal thread utilization for parse stage.
fn bench_concurrency_levels(c: &mut Criterion) {
    let files = rust_fixtures();
    if files.is_empty() {
        return;
    }

    let cpus = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4);

    let mut group = c.benchmark_group("parse_concurrency");

    for concurrency in [1, 2, 4] {
        let max_c = concurrency.min(cpus).max(1);

        group.bench_with_input(
            BenchmarkId::new("concurrency", max_c),
            &max_c,
            |b, &max_c| {
                b.iter(|| {
                    let parse_cfg = ParseConfig {
                        custom_queries_path: None,
                        repo_name: "bench_concurrency".to_string(),
                        include_config_files: true,
                        repo_path: None,
                    };

                    let (tx, mut rx) = mpsc::channel::<ParsedEntity>(1024);
                    parse_files_stream(black_box(&files), &parse_cfg, tx, max_c, None);

                    let mut count = 0u64;
                    while let Ok(_entity) = rx.try_recv() {
                        count += 1;
                    }
                    black_box(count);
                });
            },
        );
    }

    group.finish();
}

criterion_group! {
    name = benches;
    config = Criterion::default().sample_size(10);
    targets = bench_channel_capacity, bench_concurrency_levels
}
criterion_main!(benches);
