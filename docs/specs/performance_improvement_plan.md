# knot-indexer Performance Optimization Plan — v1.1.0

> **Date:** 2026-05-02
> **Status:** Approved for Implementation
> **Version:** 1.1.0
> **Origin:** Merged from Opus and Gemini independent investigations
> **Owner:** knot development team

---

## Executive Summary

Two independent analyses of the knot-indexer pipeline converged on the same core findings: the post-parse pipeline is serialized through single-threaded tokio tasks, and Neo4j uses an N+1 query pattern that dominates latency. This unified plan extracts the best proposals from both investigations into a single, prioritized implementation roadmap for v1.1.0.

**Key bottlenecks identified:**

| Bottleneck | Root Cause | Measured Impact |
|------------|-----------|-----------------|
| Neo4j N+1 entity inserts | 64 individual `MERGE` queries per batch | ~250ms/batch → ~10ms with UNWIND |
| Neo4j N+1 relationship inserts | 1 Cypher query per edge | O(N) round-trips → O(8) |
| Sequential ingestion | Single ingester tokio task | Batches processed one-at-a-time |
| Unbounded channels | `parse_tx` and `res_tx` unbounded | Worst-case: 500MB uncontrolled |
| Sequential relationship resolution | Single-threaded HashMap traversal | O(N) → O(N/num_cpus) |

**Targets:**
- Memory: below 2 GB (nice-to-have), always below 5 GB (hard requirement)
- Throughput: maximize CPU and I/O utilization across all pipeline stages

---

## 1. Root Cause Analysis

### 1.1 The "2 Threads" Observation

The observed "2-thread" behavior in stages 3–4 logs is **not** caused by Rayon — Rayon correctly uses all logical CPUs via its global thread pool (`par_iter()` in `parse_files_stream`).

The pattern comes from the **pipeline topology**: exactly one tokio task for embedding (Stage 3) and one for ingestion (Stage 4), processing batches sequentially through bounded/unbounded channels. The log timestamps confirm this:

```
18:02:38.229 → Ingester starts batch #786
18:02:38.234 → Qdrant done (5ms)
18:02:38.375 → Embedder finishes batch #788 (overlapped with Neo4j wait)
18:02:38.473 → Neo4j done (244ms) → Ingester starts #787
18:02:38.475 → Qdrant done (2ms)
18:02:38.566 → Embedder finishes #789
18:02:38.737 → Neo4j done (262ms) → cycle repeats
```

**Pattern:** Qdrant takes ~2-5ms, Neo4j takes ~250ms (64 individual queries). The embed channel (capacity 16) fills up quickly, but the ingester drains it slowly due to Neo4j serialization.

### 1.2 Current Pipeline Architecture

```
Stage 2: parse_files_stream (Rayon par_iter, all CPU cores)
    │
    ▼ mpsc::unbounded_channel (⚠ no backpressure)
    │
Stage 3: [Single Embedder Task]
    │   batches of 64, fastembed ONNX (~190ms/batch)
    │
    ▼ mpsc::channel capacity=16
    │
Stage 4: [Single Ingester Task]
    │   tokio::try_join!(Qdrant, Neo4j) per batch
    │   ├─ Qdrant upsert: ~5ms (already batched, fast)
    │   └─ Neo4j upsert: ~250ms (64 individual queries, SLOW)
    │
    ▼ mpsc::unbounded_channel (⚠ no backpressure)
    │
Stage 5: resolve_and_save_relationships (single-threaded)
```

### 1.3 Neo4j N+1 Query Problem

In `src/db/graph/upsert.rs:104-151`, `upsert_entities` executes one Cypher `MERGE` per entity:

```rust
for e in entities {  // 64 iterations = 64 network round-trips
    let cypher = format!("MERGE (n:Entity {{uuid: $uuid}}) SET n:{label} SET n.name = $name, ...");
    self.graph.run(query(&cypher).param("uuid", ...).param("name", ...)).await?;
}
```

For 790 batches of 64 entities = **50,560 individual queries** just for entity insertion. At ~4ms each including network overhead, that's ~3.4 minutes of pure Neo4j serialization.

### 1.4 Unbounded Channel Memory Risk

In `src/pipeline/runner.rs:90,92`:

```rust
let (parse_tx, mut parse_rx) = mpsc::unbounded_channel::<ParsedEntity>();   // ⚠
let (res_tx, mut res_rx) = mpsc::unbounded_channel::<ResolutionEntity>();    // ⚠
```

If parsing outpaces embedding (very likely: Rayon parses on all cores while embedding is CPU-bound on one task), the parse channel grows without bound. For a large repo with 100K files producing ~5KB per `ParsedEntity`, this means ~500MB of uncontrolled memory growth.

---

## 2. Proposed Optimizations (Priority Order)

### Phase 1: Neo4j Batched UNWIND — `upsert_entities` [CRITICAL]

**File:** `src/db/graph/upsert.rs`
**Current:** N individual `MERGE` queries (one per entity)
**Target:** 1 `UNWIND` query per entity-kind group

neo4rs 0.9.0-rc.9 supports `Vec<HashMap<String, BoltType>>` as query parameters, mapping directly to Cypher's `UNWIND $entities AS e`. Since Cypher cannot parameterize node labels dynamically, we group entities by `EntityKind` and run one UNWIND per group.

**Implementation:**

```rust
async fn upsert_entities(&self, entities: &[EmbeddedEntity]) -> Result<()> {
    if entities.is_empty() { return Ok(()); }

    let mut groups: HashMap<String, Vec<&EmbeddedEntity>> = HashMap::new();
    for e in entities {
        let label = utils::kind_to_label(&e.entity.kind);
        groups.entry(label).or_default().push(e);
    }

    for (label, group) in &groups {
        let entity_params: Vec<HashMap<String, BoltType>> = group.iter().map(|e| {
            let mut map = HashMap::new();
            map.insert("uuid".into(), e.entity.uuid.to_string().into());
            map.insert("name".into(), e.entity.name.clone().into());
            map.insert("kind".into(), e.entity.kind.to_string().into());
            map.insert("language".into(), e.entity.language.clone().into());
            map.insert("repo_name".into(), e.entity.repo_name.clone().into());
            map.insert("file_path".into(), e.entity.file_path.clone().into());
            map.insert("start_line".into(), (e.entity.start_line as i64).into());
            map.insert("end_line".into(), (e.entity.end_line as i64).into());
            map.insert("signature".into(), e.entity.signature.clone().unwrap_or_default().into());
            map.insert("docstring".into(), e.entity.docstring.clone().unwrap_or_default().into());
            map.insert("inline_comments".into(), e.entity.inline_comments.clone().into());
            map.insert("decorators".into(), e.entity.decorators.clone().into());
            map.insert("embed_text".into(), e.entity.embed_text.clone().into());
            map.insert("fqn".into(), e.entity.fqn.clone().into());
            map.insert("enclosing_class".into(), e.entity.enclosing_class.clone().unwrap_or_default().into());
            map
        }).collect();

        let cypher = format!(
            "UNWIND $entities AS e
             MERGE (n:Entity {{uuid: e.uuid}})
             SET n:{label},
                 n.name = e.name, n.kind = e.kind, n.language = e.language,
                 n.repo_name = e.repo_name, n.file_path = e.file_path,
                 n.start_line = e.start_line, n.end_line = e.end_line,
                 n.signature = e.signature, n.docstring = e.docstring,
                 n.inline_comments = e.inline_comments, n.decorators = e.decorators,
                 n.embed_text = e.embed_text, n.fqn = e.fqn,
                 n.enclosing_class = e.enclosing_class"
        );

        self.graph.run(query(&cypher).param("entities", entity_params)).await?;
    }
    Ok(())
}
```

**Expected speedup:** 10–50x for entity insertion. 64 entities: ~250ms → ~10-30ms.

### Phase 2: Neo4j Batched UNWIND — `upsert_relationships` [CRITICAL]

**File:** `src/db/graph/upsert.rs`
**Current:** One `MATCH…MERGE` per relationship edge
**Target:** One `UNWIND` query per relationship type

```rust
async fn upsert_relationships(&self, entities: &[ResolutionEntity]) -> Result<()> {
    let mut by_type: HashMap<RelationshipType, Vec<(String, String)>> = HashMap::new();
    for e in entities {
        for (callee_uuid, rel_type) in &e.relationships {
            by_type.entry(*rel_type).or_default()
                .push((e.uuid.to_string(), callee_uuid.to_string()));
        }
    }

    for (rel_type, edges) in &by_type {
        let rel_label = rel_type.to_string();
        let edge_params: Vec<HashMap<String, BoltType>> = edges.iter().map(|(c, t)| {
            let mut map = HashMap::new();
            map.insert("caller_uuid".into(), c.clone().into());
            map.insert("callee_uuid".into(), t.clone().into());
            map
        }).collect();

        let cypher = format!(
            "UNWIND $edges AS e
             MATCH (caller:Entity {{uuid: e.caller_uuid}})
             MATCH (callee:Entity {{uuid: e.callee_uuid}})
             MERGE (caller)-[:{rel_label}]->(callee)"
        );
        self.graph.run(query(&cypher).param("edges", edge_params)).await?;
    }
    Ok(())
}
```

**Expected speedup:** From thousands of individual queries to ~8 queries (one per relationship type).

### Phase 3: Bounded Channels with Backpressure [HIGH — Memory Safety]

**Files:** `src/pipeline/runner.rs`, `src/pipeline/parser/mod.rs`

Replace unbounded channels with bounded ones:

```rust
// runner.rs
let (parse_tx, mut parse_rx) = mpsc::channel::<ParsedEntity>(cfg.batch_size * 4);
let (res_tx, mut res_rx) = mpsc::channel::<ResolutionEntity>(cfg.batch_size * 4);
```

In `parse_files_stream`, use `blocking_send` (Rayon threads cannot `.await`):

```rust
pub fn parse_files_stream(
    files: &[PathBuf],
    parse_cfg: &ParseConfig,
    sender: mpsc::Sender<ParsedEntity>,  // Changed from UnboundedSender
) {
    files.par_iter().for_each(|path| match parse_single_file(path, parse_cfg) {
        Ok(entities) => {
            for entity in entities {
                if let Err(e) = sender.blocking_send(entity) {
                    warn!("Failed to send entity to channel: {e}");
                    break;
                }
            }
        }
        Err(e) => warn!("Failed to parse {}: {e:#}", path.display()),
    });
}
```

**Memory impact:** Parse channel: 64 * 4 * 5KB = ~1.3MB (vs. potential 500MB unbounded).

### Phase 4: Concurrent Ingestion with JoinSet [HIGH]

**File:** `src/pipeline/runner.rs`
**Current:** 1 ingester task processes batches sequentially
**Target:** Multiple concurrent ingestion tasks

```rust
use tokio::task::JoinSet;

let ingest_handle = {
    let vdb = Arc::clone(vector_db);
    let gdb = Arc::clone(graph_db);
    let max_concurrent = cfg.ingest_concurrency;
    let semaphore = Arc::new(tokio::sync::Semaphore::new(max_concurrent));

    tokio::spawn(async move {
        let mut total_ingested = 0;
        let mut batch_count = 0;
        let mut join_set = JoinSet::new();

        while let Some(embedded_batch) = embed_rx.recv().await {
            batch_count += 1;
            total_ingested += embedded_batch.len();
            for ee in &embedded_batch {
                res_tx.send(ResolutionEntity::from(ee)).await?;
            }

            let permit = semaphore.clone().acquire_owned().await?;
            let vdb = Arc::clone(&vdb);
            let gdb = Arc::clone(&gdb);
            let bc = batch_count;
            let bl = embedded_batch.len();

            join_set.spawn(async move {
                info!("[Worker: Ingester] Ingesting batch #{bc} ({bl} entities)...");
                let result = ingest_batch(&embedded_batch, &vdb, &gdb).await;
                drop(permit);
                result
            });
        }

        while let Some(result) = join_set.join_next().await {
            result??;
        }

        Ok::<usize, anyhow::Error>(total_ingested)
    })
};
```

**Expected improvement:** ~2-3x ingestion throughput with UNWIND applied.

### Phase 5: Rayon Thread Pool Configuration [MEDIUM]

**Files:** `src/config.rs`, `src/bin/knot-indexer.rs`

New env var `KNOT_RAYON_THREADS` with default `N-1` (leave 1 core for tokio + OS):

```rust
fn configure_rayon(threads: Option<usize>) -> Result<usize> {
    let cpus = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4);

    let thread_count = threads.unwrap_or(cpus.saturating_sub(1).max(2));

    rayon::ThreadPoolBuilder::new()
        .num_threads(thread_count)
        .build_global()
        .context("Failed to initialize Rayon thread pool")?;

    Ok(thread_count)
}
```

**Startup logging:**

```rust
info!("Logical CPUs    : {}", cpus);
info!("Rayon threads   : {}", rayon_thread_count);
info!("Batch size      : {}", cfg.batch_size);
info!("Ingest workers  : {}", cfg.ingest_concurrency);
```

### Phase 6: Parallel Relationship Resolution [MEDIUM]

**File:** `src/pipeline/ingest/resolve.rs`

Replace sequential loop with `par_iter_mut()`:

```rust
use rayon::prelude::*;

entities.par_iter_mut().for_each(|entity| {
    let reference_intents = entity.reference_intents.clone();
    let mut seen: HashSet<(Uuid, RelationshipType)> = HashSet::new();

    for intent in reference_intents {
        let (resolved_uuid, rel_type) = /* ... existing match arms ... */;

        if let Some(uuid) = resolved_uuid && seen.insert((uuid, rel_type)) {
            entity.relationships.push((uuid, rel_type));
        }
    }
});
```

Context maps are read-only (built before the parallel loop). Linear speedup with core count.

---

## 3. Implementation Order

| Phase | Change | Complexity | Expected Impact |
|-------|--------|------------|-----------------|
| 1 | Neo4j UNWIND `upsert_entities` | Medium | **10–50x** entity write speedup |
| 2 | Neo4j UNWIND `upsert_relationships` | Medium | **10–50x** relationship write speedup |
| 3 | Bounded channels + backpressure | Low | Memory safety (500MB → 1.3MB worst-case) |
| 4 | Concurrent ingestion (JoinSet) | Medium | **2–3x** ingestion throughput |
| 5 | Rayon thread pool config | Low | User control, sensible default (N-1) |
| 6 | Parallel relationship resolution | Medium | **Nx** speedup on resolution |

**Rationale:** Phases 1-2 eliminate the dominant bottleneck (Neo4j serialization). Phase 3 is a safety net for memory. Phase 4 adds throughput. Phases 5-6 are polish.

---

## 4. New Configuration Options

| Environment Variable | CLI Flag | Default | Description |
|---------------------|----------|---------|-------------|
| `KNOT_RAYON_THREADS` | `--rayon-threads` | `num_cpus - 1` | Rayon parallel parsing thread count |
| `KNOT_INGEST_CONCURRENCY` | `--ingest-concurrency` | `4` | Max concurrent ingestion tasks |
| `KNOT_BATCH_SIZE` | `--batch-size` | `64` | Entities per embed/upsert batch (existing) |

---

## 5. Memory Budget

### Channel Memory (After Bounding)

| Channel | Capacity | Max Memory |
|---------|----------|------------|
| `parse_tx` (bounded) | `batch_size * 4` = 256 | 256 * 5KB = **1.3 MB** |
| `embed_tx` (bounded) | 16 batches | 16 * 64 * 8KB = **8 MB** |
| `res_tx` (bounded) | `batch_size * 4` = 256 | 256 * 2KB = **0.5 MB** |

### Estimated Peak Memory

| Component | Memory |
|-----------|--------|
| ONNX model (fastembed AllMiniLML6V2) | ~100 MB |
| Channel buffers (bounded) | ~10 MB |
| Resolution context maps (50K entries) | ~50 MB |
| In-flight batches (4 concurrent * 64 * 8KB) | ~2 MB |
| Tree-sitter parsers (Rayon pool) | ~50 MB |
| Base process + libraries | ~100 MB |
| **Estimated total** | **~300-400 MB** |

Well within 2 GB (nice-to-have) and far from 5 GB (hard limit).

---

## 6. Verification

### Quality Gates
```bash
cargo fmt -- --check
cargo clippy --all-targets -- -D warnings
cargo test
./tests/run_all_e2e.sh  # if Docker available
```

### Unit Tests to Add/Update
- `src/db/graph/upsert.rs`: Update tests for UNWIND batch queries
- `src/pipeline/runner.rs`: Test bounded channel backpressure
- `src/pipeline/parser/mod.rs`: Update `parse_files_stream` tests for bounded sender

### E2E Tests
- All existing suites must pass without modification
- No behavioral changes to output data (entities, relationships, graph structure)

---

## 7. Files to Modify

| File | Changes |
|------|---------|
| `src/db/graph/upsert.rs` | UNWIND batch for `upsert_entities` and `upsert_relationships` |
| `src/pipeline/runner.rs` | Bounded channels, concurrent ingestion with JoinSet, startup metrics |
| `src/pipeline/parser/mod.rs` | Change `parse_files_stream` to use bounded `Sender` with `blocking_send` |
| `src/pipeline/ingest/resolve.rs` | `par_iter_mut()` for relationship resolution |
| `src/config.rs` | Add `rayon_threads`, `ingest_concurrency` fields to `IndexerCli` and `Config` |
| `src/bin/knot-indexer.rs` | Rayon pool init, enhanced startup banner |
| `README.md` | Document new `KNOT_RAYON_THREADS`, `KNOT_INGEST_CONCURRENCY` env vars |

---

## 8. Decisions Log

| Decision | Rationale |
|----------|-----------|
| UNWIND grouping by EntityKind | Cypher cannot parameterize labels; grouping is portable and sufficient |
| JoinSet + Semaphore for ingestion | Better error propagation than bare spawns, concurrency is limited |
| Bounded channels over unbounded | Memory safety; backpressure prevents OOM on large repos |
| N-1 cores default for Rayon | Leave 1 core for tokio runtime + OS; user can override via env var |
| No multiple Embedders | ONNX Runtime already multi-threaded internally; avoids CPU oversubscription + ~200MB/instance RAM |
| `blocking_send` in Rayon threads | Rayon threads cannot `.await`; `blocking_send` blocks the worker until channel has space |

---

## 9. Performance Benchmarking & Validation Framework (Proposed)

### 9.1 Rationale

Once all 6 phases are implemented, we need a comprehensive validation system to:
1. **Verify** that each optimization achieves its predicted speedup (Phase 1: 10–50x, Phase 2: 10–50x, etc.)
2. **Detect regressions** in subsequent PRs or changes
3. **Track trends** over time across CI/CD pipelines
4. **Provide detailed breakdowns** of time spent in each pipeline stage

This section proposes a multi-level benchmarking strategy that extends the existing `token_tree_bench.rs` and integrates with the E2E test suite.

### 9.2 Current State

- **Unit-level benchmarks**: `benches/token_tree_bench.rs` uses Criterion to measure deeply nested macro extraction
- **E2E tests**: `tests/run_all_e2e.sh` validates correctness (no performance tracking)
- **No baseline tracking**: Previously measured performance is not saved or compared

### 9.3 Proposed Three-Level Benchmarking Strategy

#### Level 1: Unit-Level Benchmarks (Criterion)

**Scope**: Low-level, isolated component performance  
**Files to add**: `benches/pipeline_bench.rs`, `benches/graph_upsert_bench.rs`

**New benchmarks:**

1. **`pipeline_bench.rs`** – End-to-end pipeline throughput
   - Measures parsing + embedding + ingestion on a fixed test repository
   - Uses a subset of `tests/testing_files/` (e.g., 100 Rust files, 50 Java files)
   - Outputs per-stage timing:
     ```
     Stage 2 (Parsing):      245ms  (parsed 5,234 entities)
     Stage 3 (Embedding):    3,847ms (64-entity batches, ONNX)
     Stage 4 (Ingestion):    1,243ms (Neo4j UNWIND + Qdrant upsert)
     Stage 5 (Resolution):   156ms   (relationship resolution)
     ──────────────────────────────
     Total:                  5,491ms
     ```

2. **`graph_upsert_bench.rs`** – Neo4j UNWIND batching performance
   - Pre-populated Neo4j with 10K entities
   - Measures `upsert_entities` with different batch sizes (16, 64, 256)
   - Compares old N-query approach (simulated) vs. UNWIND batched
   - Expected output:
     ```
     64 entities, UNWIND:    12ms    (1 query)
     64 entities, old N-way: ~256ms  (64 queries)
     Speedup: 21.3x
     ```

3. **`channel_backpressure_bench.rs`** – Bounded channel overhead
   - Measures parse/embed channel throughput with various capacities
   - Validates memory ceiling under backpressure

**Run locally:**
```bash
cargo bench --bench pipeline_bench
cargo bench --bench graph_upsert_bench
cargo bench --bench channel_backpressure_bench
```

#### Level 2: E2E Integration Benchmarks

**Scope**: Full indexing pipeline on realistic test repositories  
**Integration**: Extend `tests/run_all_e2e.sh`

**Modifications to `tests/run_all_e2e.sh`:**

1. **Create `tests/benchmark_e2e.sh`** (new script, separate from correctness tests)
   - Runs the full indexing pipeline on a pre-defined test repository
   - Captures timing for each phase using `time -v` and custom logging
   - Saves results to `.perf_metrics/<date>/<run_id>/metrics.json`
   - Runs after all correctness E2E tests pass

2. **Metrics captured per language suite:**
   - Total indexing time
   - Per-stage breakdown (parse, embed, ingest, resolve)
   - Peak memory usage (via `/proc/self/status` on Linux)
   - Qdrant upsert latency (from logs)
   - Neo4j batch sizes and query counts
   - Entities parsed, embedded, ingested per minute

3. **Sample output structure:**
   ```json
   {
     "timestamp": "2026-05-02T14:32:15Z",
     "commit_hash": "abc1234",
     "test_suite": "rust_e2e",
     "total_time_ms": 5491,
     "stage_timings": {
       "parse": {"ms": 245, "entities_per_sec": 21384},
       "embed": {"ms": 3847, "batches": 82, "avg_batch_ms": 46.9},
       "ingest": {"ms": 1243, "neo4j_queries": 98, "qdrant_latency_ms": 412},
       "resolve": {"ms": 156, "relationships_resolved": 1823}
     },
     "memory_peak_mb": 387,
     "entities_total": 5234
   }
   ```

#### Level 3: Continuous CI/CD Baseline Tracking

**Scope**: Track performance across commits in GitHub Actions  
**Strategy**: Store baseline metrics in repository + detect regressions

**Implementation details:**

1. **Storage: `.perf_metrics/baseline.json`** (committed to repo)
   - Stores the most recent "good" baseline on each branch
   - Example structure:
     ```json
     {
       "commit": "abc1234",
       "date": "2026-05-02",
       "suite_results": {
         "rust_e2e": {
           "total_ms": 5491,
           "stage_timings": {...},
           "memory_peak_mb": 387
         },
         "java_e2e": {...}
       },
       "threshold_tolerances": {
         "total_time_regression_pct": 5,
         "memory_regression_pct": 10
       }
     }
     ```

2. **Comparison script: `scripts/compare_perf_metrics.sh`**
   - Compares current run results against baseline
   - Fails CI if:
     - Total time increases by >5% (configurable)
     - Peak memory increases by >10% (configurable)
     - Any single stage regresses by >10%
   - Generates a comparison report:
     ```
     Rust E2E Performance Report
     ──────────────────────────────────────────
     Total Time:    5,491ms (baseline: 5,234ms) ⚠ +4.9% (within tolerance)
     Parse:         245ms   (baseline: 248ms)   ✓ -1.2%
     Embed:         3,847ms (baseline: 3,892ms) ✓ -1.2%
     Ingest:        1,243ms (baseline: 1,089ms) ⚠ +14.1% (REGRESSION!)
     Resolve:       156ms   (baseline: 155ms)   ✓ +0.6%
     
     Peak Memory:   387MB   (baseline: 391MB)   ✓ -1.0%
     
     RESULT: ⚠ MARGINAL (Ingest stage regressed, investigate Phase 4 JoinSet overhead)
     ```

3. **GitHub Actions Integration**
   - Add step to `.github/workflows/ci.yml`:
     ```yaml
     - name: Run Performance Benchmarks
       run: |
         ./tests/benchmark_e2e.sh --output-dir /tmp/perf_results
         scripts/compare_perf_metrics.sh /tmp/perf_results .perf_metrics/baseline.json
       if: success()  # Only run if all unit + E2E tests pass
     
     - name: Update Baseline (on main/master merge)
       run: |
         cp /tmp/perf_results/aggregated.json .perf_metrics/baseline.json
       if: github.ref == 'refs/heads/main' || github.ref == 'refs/heads/master'
     ```

### 9.4 Metrics to Track

| Metric | Source | Purpose | Threshold |
|--------|--------|---------|-----------|
| **Total time (ms)** | Wall-clock | Overall speedup validation | ±5% tolerance |
| **Stage breakdown (parse, embed, ingest, resolve)** | Logging + timers | Identify bottleneck phase | ±10% per-stage |
| **Peak memory (MB)** | `/proc/self/status` (Linux) or `ps` | Validate memory optimizations | ±10% tolerance |
| **Entities/sec throughput** | Entities parsed / parse_time | Parsing efficiency | Trend tracking |
| **Neo4j batch query count** | Query logs | Validate UNWIND batching (Phase 1–2) | Target: <100 queries |
| **Qdrant latency (ms)** | Ingest logs | Already optimized, baseline only | Trend tracking |
| **Rayon thread utilization** | Startup logs | Validate Phase 5 config | N-1 cores + user override |
| **JoinSet concurrency (active tasks)** | Ingest logs | Validate Phase 4 throughput | 4 concurrent (configurable) |

### 9.5 Proposed File Structure

```
knot/
├── benches/
│   ├── token_tree_bench.rs        (existing)
│   ├── pipeline_bench.rs          (NEW: full pipeline on test repos)
│   ├── graph_upsert_bench.rs      (NEW: Neo4j UNWIND speedup)
│   └── channel_backpressure_bench.rs (NEW: bounded channel overhead)
├── scripts/
│   ├── compare_perf_metrics.sh    (NEW: baseline comparison)
│   └── sample_metrics_report.sh   (NEW: human-readable reporting)
├── tests/
│   ├── run_all_e2e.sh             (existing, unchanged)
│   ├── benchmark_e2e.sh           (NEW: full pipeline metrics capture)
│   └── fixtures/
│       └── perf_baseline_repo/    (NEW: fixed test data for benchmarks)
├── .perf_metrics/
│   ├── baseline.json              (NEW: last known good metrics)
│   ├── 2026-05-02/
│   │   └── run_001/metrics.json   (NEW: timestamped results)
│   └── threshold_tolerances.json  (NEW: per-metric tolerances)
├── .github/workflows/
│   └── ci.yml                     (MODIFIED: add perf benchmark step)
└── docs/specs/
    └── performance_improvement_plan.md (this file, section 9)
```

### 9.6 Example: Validating Phase 1 (Neo4j UNWIND)

**Before optimization (current behavior):**
```
Ingest stage: 1,243ms
└─ Neo4j queries: 1,456 individual MERGE/MATCH queries
   └─ Avg latency per query: ~0.8ms (including network)
```

**After Phase 1 UNWIND optimization:**
```
Ingest stage: 89ms (predicted: 10-30ms for entity inserts only)
└─ Neo4j queries: 12 UNWIND queries (one per EntityKind)
   └─ Avg latency per batch: ~7.4ms
```

**Benchmark verification:**
```bash
cargo bench --bench graph_upsert_bench
# Expected output: "Speedup: 14.7x" (within 10-50x range)
```

**E2E validation:**
```bash
./tests/benchmark_e2e.sh --focus rust_e2e
# Captured metrics: Ingest stage = 89ms, Neo4j queries = 12
scripts/compare_perf_metrics.sh /tmp/perf_results .perf_metrics/baseline.json
# Result: "Ingest: 89ms (baseline: 1,243ms) ✓ -92.8% (excellent!)"
```

### 9.7 Implementation Roadmap

| Step | Task | Owner | Priority |
|------|------|-------|----------|
| 1 | Create `benches/pipeline_bench.rs` with full pipeline on test repos | Team | Phase 4 (after Phase 3 complete) |
| 2 | Create `benches/graph_upsert_bench.rs` to measure UNWIND speedup | Team | Phase 1 completion |
| 3 | Implement `tests/benchmark_e2e.sh` with metrics capture | Team | Phase 3 completion |
| 4 | Add `.perf_metrics/baseline.json` to repo | Team | Phase 3 completion |
| 5 | Implement `scripts/compare_perf_metrics.sh` | Team | Phase 4 completion |
| 6 | Integrate into `.github/workflows/ci.yml` | CI/CD | After step 5 |
| 7 | Document baseline update process in CONTRIBUTING.md | Docs | After step 6 |

### 9.8 Known Constraints & Trade-offs

| Constraint | Impact | Mitigation |
|-----------|--------|-----------|
| **CI performance variation** | Hardware differences (GitHub runners vary) | Use relative % change, not absolute time; run 5x and take median |
| **Docker startup overhead** | Qdrant/Neo4j startup adds ~10-15s | Pre-warm containers in benchmark_e2e.sh; cache Docker images |
| **Criterion overhead** | Criterion re-runs benchmarks 20+ times | Use `sample_size(5)` for pre-release validation, `sample_size(20)` for final release |
| **Memory measurement accuracy** | `/proc/self/status` includes kernel pages | Cross-validate with `top` output; document method in README |
| **Tolerance too tight** | Flaky benchmarks fail CI | Start with ±10%, tighten after 2 weeks of baseline history |
| **Tolerance too loose** | Fail to detect real regressions | Review tolerance settings quarterly; adjust based on observed variance |

### 9.9 Success Criteria

After implementation, this benchmarking framework is considered successful when:

1. **Phase 1 (Neo4j UNWIND – `upsert_entities`)**: Measured speedup is 10–50x ✓
2. **Phase 2 (Neo4j UNWIND – `upsert_relationships`)**: Query count drops from >50K to <100 ✓
3. **Phase 3 (Bounded channels)**: Peak memory stays <400MB on large repos ✓
4. **Phase 4 (Concurrent ingestion)**: Ingest throughput improves 2–3x ✓
5. **Phase 5 (Rayon threads)**: Parse stage uses N-1 cores effectively ✓
6. **Phase 6 (Parallel resolution)**: Resolution stage shows linear speedup ✓
7. **Regression detection**: Any future PR that increases total time >5% is caught by CI ✓
8. **Baseline history**: At least 10 baseline measurements are stored and trended ✓

### 9.10 Integration with CONTRIBUTING.md

Update `CONTRIBUTING.md` with:
- Instructions to run benchmarks locally: `cargo bench && ./tests/benchmark_e2e.sh`
- Guidelines for interpreting comparison reports
- When to update baseline (only on main/master after merged PRs)
- How to investigate and fix performance regressions
