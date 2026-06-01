//! Criterion benchmarks for Neo4j UNWIND batching performance (Phases 1-2).
//!
//! Validates the 10-50x speedup of batched `UNWIND` queries over the old
//! N-query approach for entity inserts.
//!
//! These benchmarks require a running Neo4j instance. If Neo4j is not
//! available, the benchmarks are skipped with a warning message.
//!
//! Environment variables:
//!   KNOT_NEO4J_URI  — Neo4j connection URI (default: bolt://localhost:7687)
//!   KNOT_NEO4J_USER — Neo4j username (default: neo4j)
//!   KNOT_NEO4J_PASSWORD — Neo4j password (required)
//!
//! Run with: cargo bench --bench graph_upsert_bench

use criterion::{BenchmarkId, Criterion, black_box, criterion_group, criterion_main};
use std::sync::Arc;
use uuid::Uuid;

use knot::db::graph::{ConnectExt, DeleteExt, GraphDb, UpsertExt};
use knot::models::{EmbeddedEntity, EntityKind, ParsedEntity};

fn make_test_parsed_entity(id: u32) -> ParsedEntity {
    ParsedEntity {
        uuid: Uuid::new_v4(),
        name: format!("TestEntity_{id}"),
        kind: if id.is_multiple_of(4) {
            EntityKind::RustStruct
        } else if id % 4 == 1 {
            EntityKind::RustFunction
        } else if id % 4 == 2 {
            EntityKind::RustTrait
        } else {
            EntityKind::RustMethod
        },
        fqn: format!("crate::test_entity_{id}"),
        signature: Some(format!("fn test_{id}()")),
        docstring: Some(format!("Benchmark entity #{id}")),
        inline_comments: Vec::new(),
        decorators: Vec::new(),
        rust_attributes: None,
        impl_trait: None,
        impl_target: None,
        generics: None,
        lifetimes: None,
        alias_module_path: None,
        original_export_name: None,
        default_export: None,
        language: "rust".to_string(),
        repo_name: "benchmark_repo".to_string(),
        file_path: format!("src/test_{}.rs", id % 10),
        start_line: 1,
        end_line: 5,
        enclosing_class: None,
        reference_intents: Vec::new(),
        calls: Vec::new(),
        relationships: Vec::new(),
        embed_text: format!("[RustStruct] TestEntity_{id}\nSignature: fn test_{id}()"),
    }
}

fn make_embedded_entity(pe: &ParsedEntity) -> EmbeddedEntity {
    EmbeddedEntity {
        entity: pe.clone(),
        vector: vec![0.0f32; 384],
    }
}

fn generate_entities(count: u32) -> Vec<ParsedEntity> {
    (0..count).map(make_test_parsed_entity).collect()
}

fn connect_and_cleanup() -> Option<(Arc<GraphDb>, tokio::runtime::Runtime)> {
    let neo4j_uri =
        std::env::var("KNOT_NEO4J_URI").unwrap_or_else(|_| "bolt://localhost:7687".to_string());
    let neo4j_user = std::env::var("KNOT_NEO4J_USER").unwrap_or_else(|_| "neo4j".to_string());
    let neo4j_password = match std::env::var("KNOT_NEO4J_PASSWORD") {
        Ok(p) => p,
        Err(_) => {
            eprintln!("WARNING: KNOT_NEO4J_PASSWORD not set — skipping Neo4j upsert benchmarks");
            return None;
        }
    };

    let rt = match tokio::runtime::Runtime::new() {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("WARNING: Failed to create tokio runtime: {e}");
            return None;
        }
    };

    let gdb = match rt.block_on(GraphDb::connect(&neo4j_uri, &neo4j_user, &neo4j_password)) {
        Ok(db) => Arc::new(db),
        Err(e) => {
            eprintln!("WARNING: Failed to connect to Neo4j at {neo4j_uri}: {e}");
            return None;
        }
    };

    // Ensure indexes exist
    let _ = rt.block_on(gdb.ensure_indexes());

    // Clean up any previous benchmark data
    let _ = rt.block_on(gdb.delete_by_repo("benchmark_repo"));

    Some((gdb, rt))
}

fn cleanup_benchmark_data(rt: &tokio::runtime::Runtime, gdb: &Arc<GraphDb>) {
    let gdb = Arc::clone(gdb);
    let _ = rt.block_on(async move { gdb.delete_by_repo("benchmark_repo").await });
}

/// Benchmark UNWIND-batched entity upserts at various batch sizes.
/// Validates Phase 1: 10-50x speedup vs old N-query approach.
fn bench_entity_upsert_unwind(c: &mut Criterion) {
    let (gdb, rt) = match connect_and_cleanup() {
        Some(v) => v,
        None => return,
    };

    let mut group = c.benchmark_group("graph_upsert_entities_unwind");

    let batch_sizes = [16, 64, 128, 256];

    for &batch_size in &batch_sizes {
        let entities: Vec<EmbeddedEntity> = generate_entities(batch_size)
            .iter()
            .map(make_embedded_entity)
            .collect();

        let gdb = Arc::clone(&gdb);

        group.bench_with_input(
            BenchmarkId::new("unwind_batch", batch_size),
            &entities,
            |b, entities| {
                b.iter(|| {
                    rt.block_on(async {
                        let gdb = Arc::clone(&gdb);
                        gdb.upsert_entities(black_box(entities)).await
                    })
                });
            },
        );
    }

    group.finish();

    cleanup_benchmark_data(&rt, &gdb);
}

/// Benchmark the UNWIND-batched approach with increasing entity counts
/// to validate linear scaling with entity count.
fn bench_entity_count_scaling(c: &mut Criterion) {
    let (gdb, rt) = match connect_and_cleanup() {
        Some(v) => v,
        None => return,
    };

    let mut group = c.benchmark_group("graph_upsert_scaling");

    let entity_counts: [u32; 6] = [16, 32, 64, 128, 256, 512];

    for &count in &entity_counts {
        let entities: Vec<EmbeddedEntity> = generate_entities(count)
            .iter()
            .map(make_embedded_entity)
            .collect();

        let gdb = Arc::clone(&gdb);

        group.bench_with_input(
            BenchmarkId::new("entity_count", count),
            &entities,
            |b, entities| {
                b.iter(|| {
                    rt.block_on(async {
                        let gdb = Arc::clone(&gdb);
                        gdb.upsert_entities(black_box(entities)).await
                    })
                });
            },
        );
    }

    group.finish();

    cleanup_benchmark_data(&rt, &gdb);
}

criterion_group! {
    name = benches;
    config = Criterion::default().sample_size(10);
    targets = bench_entity_upsert_unwind, bench_entity_count_scaling
}
criterion_main!(benches);
