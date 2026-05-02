//! Criterion benchmarks for end-to-end pipeline throughput on test fixtures.
//!
//! Measures parsing + preparation throughput across all supported languages
//! using real test fixtures from `tests/testing_files/`. These benchmarks
//! validate:
//! - Phase 1-2: Entity extraction speed (parsing correctness baseline)
//! - Phase 3:  Preparation stage overhead (UUID assignment + embed text)
//! - Phase 5:  Rayon thread utilization for the parse stage
//!
//! Run with: cargo bench --bench pipeline_bench

use criterion::{BenchmarkId, Criterion, black_box, criterion_group, criterion_main};
use std::path::PathBuf;
use tokio::sync::mpsc;

use knot::models::ParsedEntity;
use knot::pipeline::{
    parser::{ParseConfig, parse_files_stream},
    prepare::prepare_entities,
};

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn test_files_dir() -> PathBuf {
    repo_root().join("tests").join("testing_files")
}

fn fixtures_for(ext: &str) -> Vec<PathBuf> {
    test_files_dir()
        .read_dir()
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().map(|e| e == ext).unwrap_or(false))
        .collect()
}

fn parse_benchmark(files: &[PathBuf], label: &str) {
    let parse_cfg = ParseConfig {
        custom_queries_path: None,
        repo_name: format!("benchmark_{label}"),
    };

    let cpus = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4);
    let max_concurrent = cpus;

    let (tx, mut rx) = mpsc::channel::<ParsedEntity>(1024);
    parse_files_stream(files, &parse_cfg, tx, max_concurrent);

    black_box(&parse_cfg);

    let mut entities = Vec::with_capacity(1024);
    while let Ok(entity) = rx.try_recv() {
        entities.push(entity);
    }

    prepare_entities(&mut entities);

    black_box(&entities);
}

/// Parsing throughput — measures raw entity extraction speed per-language.
fn bench_parse_throughput(c: &mut Criterion) {
    let mut group = c.benchmark_group("pipeline_parse_throughput");

    // Rust
    let rust_files: Vec<PathBuf> = fixtures_for("rs");
    if !rust_files.is_empty() {
        group.bench_with_input(
            BenchmarkId::new("rust", rust_files.len()),
            &rust_files,
            |b, files| b.iter(|| parse_benchmark(black_box(files), "rust")),
        );
    }

    // Java
    let java_files: Vec<PathBuf> = fixtures_for("java");
    if !java_files.is_empty() {
        group.bench_with_input(
            BenchmarkId::new("java", java_files.len()),
            &java_files,
            |b, files| b.iter(|| parse_benchmark(black_box(files), "java")),
        );
    }

    // TypeScript
    let ts_files: Vec<PathBuf> = fixtures_for("ts");
    if !ts_files.is_empty() {
        group.bench_with_input(
            BenchmarkId::new("typescript", ts_files.len()),
            &ts_files,
            |b, files| b.iter(|| parse_benchmark(black_box(files), "typescript")),
        );
    }

    // Kotlin
    let kt_files: Vec<PathBuf> = fixtures_for("kt");
    if !kt_files.is_empty() {
        group.bench_with_input(
            BenchmarkId::new("kotlin", kt_files.len()),
            &kt_files,
            |b, files| b.iter(|| parse_benchmark(black_box(files), "kotlin")),
        );
    }

    // Python
    let py_files: Vec<PathBuf> = fixtures_for("py");
    if !py_files.is_empty() {
        group.bench_with_input(
            BenchmarkId::new("python", py_files.len()),
            &py_files,
            |b, files| b.iter(|| parse_benchmark(black_box(files), "python")),
        );
    }

    group.finish();
}

/// Preparation stage — measures UUID assignment + embed text construction overhead.
fn bench_preparation_stage(c: &mut Criterion) {
    let mut group = c.benchmark_group("pipeline_preparation");

    // Pre-parse all Rust files to have a realistic batch of ParsedEntity
    let rust_files: Vec<PathBuf> = fixtures_for("rs");
    if rust_files.is_empty() {
        return;
    }

    for lang in ["rust", "java", "typescript", "kotlin", "python"] {
        let lang_files: Vec<PathBuf> = fixtures_for(match lang {
            "rust" => "rs",
            "java" => "java",
            "typescript" => "ts",
            "kotlin" => "kt",
            "python" => "py",
            _ => continue,
        });

        if lang_files.is_empty() {
            continue;
        }

        let parse_cfg = ParseConfig {
            custom_queries_path: None,
            repo_name: format!("bench_prep_{lang}"),
        };

        let cpus = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(4);
        let max_concurrent = cpus;
        let (tx, mut rx) = mpsc::channel::<ParsedEntity>(1024);
        parse_files_stream(&lang_files, &parse_cfg, tx, max_concurrent);

        let mut entities = Vec::with_capacity(1024);
        while let Ok(entity) = rx.try_recv() {
            entities.push(entity);
        }

        if entities.is_empty() {
            continue;
        }

        group.bench_with_input(
            BenchmarkId::new("prepare", format!("{}_{}_entities", lang, entities.len())),
            &entities,
            |b, entities| {
                b.iter(|| {
                    let mut cloned: Vec<_> = entities.to_vec();
                    prepare_entities(black_box(&mut cloned));
                })
            },
        );
    }

    group.finish();
}

/// Full parse + prepare pipeline — end-to-end Stage 2-3 measurement.
fn bench_full_parse_prepare(c: &mut Criterion) {
    let mut group = c.benchmark_group("pipeline_full");

    for (lang, ext) in [
        ("rust", "rs"),
        ("java", "java"),
        ("typescript", "ts"),
        ("kotlin", "kt"),
        ("python", "py"),
    ] {
        let files: Vec<PathBuf> = fixtures_for(ext);
        if files.is_empty() {
            continue;
        }

        group.bench_with_input(
            BenchmarkId::new(
                "parse_and_prepare",
                format!("{}_{}_files", lang, files.len()),
            ),
            &files,
            |b, files| {
                b.iter(|| parse_benchmark(black_box(files), lang));
            },
        );
    }

    group.finish();
}

criterion_group! {
    name = benches;
    config = Criterion::default().sample_size(10);
    targets = bench_parse_throughput, bench_preparation_stage, bench_full_parse_prepare
}
criterion_main!(benches);
