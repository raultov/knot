//! Criterion benchmarks for token_tree nested macro extraction performance.
//!
//! These benchmarks verify the O(N) Substring Skipping optimization for deeply nested
//! token_tree nodes. Without the optimization, extraction time grows exponentially with
//! nesting depth due to redundant string allocations and `::` pattern matching.
//!
//! Run with: cargo bench --bench token_tree_bench

use criterion::{BenchmarkId, Criterion, black_box, criterion_group, criterion_main};
use tree_sitter::Parser;

fn generate_nested_macro_code(depth: usize) -> String {
    let mut code = String::from("fn test() {\n    ");
    let mut current = String::from("MyType::new()");
    for _ in 0..depth {
        current = format!("vec![{}]", current);
    }
    code.push_str(&current);
    code.push_str(";\n}\n");
    code
}

fn run_token_tree_extraction(code: &str) {
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_rust::LANGUAGE.into())
        .unwrap();
    let tree = parser.parse(code, None).unwrap();

    let mut entities: Vec<knot::models::ParsedEntity> = Vec::new();
    knot::pipeline::parser::languages::rust::collect_rust_type_references(
        tree.root_node(),
        code.as_bytes(),
        &mut entities,
        "benchmark.rs",
        "benchmark_repo",
    );
}

fn criterion_benchmark(c: &mut Criterion) {
    let mut group = c.benchmark_group("token_tree_nested");

    let depths: Vec<usize> = vec![1, 5, 10, 20, 50, 100];

    for depth in depths {
        let code = generate_nested_macro_code(depth);
        group.bench_with_input(BenchmarkId::from_parameter(depth), &depth, |b, _| {
            b.iter(|| run_token_tree_extraction(black_box(&code)));
        });
    }

    group.finish();
}

criterion_group! {
    name = benches;
    config = Criterion::default().sample_size(20);
    targets = criterion_benchmark
}
criterion_main!(benches);
