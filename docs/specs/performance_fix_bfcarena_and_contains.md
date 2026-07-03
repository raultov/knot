# Performance Fix: ONNX BFCArena Memory Growth & CONTAINS Query O(n²)

**Issue:** [#18 — Performance issue](https://github.com/raultov/knot/issues/18)  
**Status:** Planned  
**Approach:** TDD/BDD — tests written first, must fail before implementation begins  

---

## Problem Summary

Two independent bugs compound to produce severe indexing slowdowns on large repos:

### Bug 1 — ONNX BFCArena memory growth (primary bottleneck)

`src/pipeline/embed.rs` calls `self.model.embed(texts, Some(batch_size))` in a
long-lived ONNX session. The ONNX Runtime's BFCArena allocator accumulates
heap blocks for each unique tensor shape it encounters (determined by the
maximum tokenized sequence length within each batch). Blocks are **never
returned to the OS**. Over time the arena grows until available RAM is
exhausted, triggering OS-level swapping and causing embedding time to jump from
~1s/batch to 9s/batch.

Evidence from logs (`knot-logs.txt`):
```
Batch 1 → BFCArena: 43 MB → 714 MB
Batch 2 → BFCArena: 714 MB → 2.8 GB
Batch 3 → BFCArena: 2.8 GB → 5.0 GB
```

The threshold varies by language: Java entities have longer `embed_text`
(verbose FQNs, JavaDoc, full generic signatures) → faster arena growth →
degradation at ~batch 100. TypeScript entities are shorter → degradation at
~batch 4300.

### Bug 2 — CONTAINS auto-link query is O(n²) total (secondary bottleneck)

In `src/db/graph/upsert.rs:221-236`, after each batch of 64 entities is
upserted into Neo4j, a Cypher query creates `CONTAINS` edges (class→method):

```cypher
MATCH (m:Entity {repo_name: $repo_name})   -- scans ALL repo entities
WHERE m.enclosing_class IS NOT NULL ...
MERGE (c)-[:CONTAINS]->(m)
```

With `n` batches total, this scan runs `n` times over `n × 64` entities →
**O(n²)** total Neo4j work. The fix scopes the query to the 64 entities in the
current batch, reducing it to **O(n × 64) = O(n)** total.

---

## Methodology: TDD/BDD

Each fix follows three phases:

1. **Red** — write all tests; run them and confirm they fail.
2. **Green** — implement the minimum code to make tests pass.
3. **Refactor** — clean up without breaking tests.
4. **E2E gate** — run `./tests/run_all_e2e_fast.sh` and confirm all 17 suites pass.

Tests at every level:
- **Unit tests** (inline in `src/`) — pure logic, no databases, no ONNX model.
- **Integration tests** (`#[ignore]`) — require live Neo4j; run manually.
- **E2E tests** (`tests/run_*_e2e.sh`) — full pipeline, shared Docker DB.

---

## Fix 1 — Periodic Embedder Reset

### Design

Add a configurable `embedder_reset_interval: usize` to `Config`. Every N
batches, the embed_handle in `runner.rs` destroys the current `Embedder` and
creates a new one from the same cache directory. This resets the ONNX session
and its BFCArena to zero. Model weights are loaded from disk cache (~400ms
overhead per reset, amortised over N batches).

A value of `0` disables automatic resets entirely (useful for small repos or
debugging).

### Files affected

| File | Change |
|------|--------|
| `src/config.rs` | Add `embedder_reset_interval: usize` field |
| `src/bin/knot-indexer.rs` | Wire CLI arg `--embedder-reset-interval` + env var |
| `src/pipeline/embed.rs` | Add pure helper `needs_reset(batch_count, interval) -> bool` |
| `src/pipeline/runner.rs` | Capture `cache_dir` in embed_handle; check and reset at interval |

### Phase 1 — Red: Unit tests to write first

All tests below live in the `#[cfg(test)]` block of `src/pipeline/embed.rs`
unless noted. They must **fail** before any implementation is touched.

---

#### Test group A — `needs_reset` helper (pure function, no ONNX)

```rust
// A helper pure function that decides whether to reset the embedder.
// Signature: fn needs_reset(batch_count: usize, interval: usize) -> bool

#[test]
fn test_needs_reset_disabled_when_interval_zero() {
    // interval=0 means "never reset"
    assert!(!needs_reset(500, 0));
    assert!(!needs_reset(1000, 0));
}

#[test]
fn test_needs_reset_true_exactly_at_interval() {
    assert!(needs_reset(500, 500));
    assert!(needs_reset(1000, 500));
    assert!(needs_reset(250, 250));
}

#[test]
fn test_needs_reset_false_before_interval() {
    assert!(!needs_reset(499, 500));
    assert!(!needs_reset(1, 500));
}

#[test]
fn test_needs_reset_false_between_intervals() {
    // Must reset at 500, 1000, … not at 501, 999, …
    assert!(!needs_reset(501, 500));
    assert!(!needs_reset(999, 500));
}

#[test]
fn test_needs_reset_multiples_of_interval() {
    for multiplier in 1..=10 {
        assert!(needs_reset(500 * multiplier, 500));
    }
}

#[test]
fn test_needs_reset_batch_count_zero_never_resets() {
    // batch_count=0 means no batch has been processed yet; must never reset
    assert!(!needs_reset(0, 500));
    assert!(!needs_reset(0, 1));
}
```

---

#### Test group B — Embedder struct changes (no ONNX model, struct-level logic)

These tests verify the `Embedder` type exposes the cache_dir so runner.rs can
recreate it.

```rust
#[test]
fn test_embedder_exposes_cache_dir() {
    // Embedder must store cache_dir so it can be recreated without runner.rs
    // having to keep a separate copy.
    // After this test the Embedder struct must have a cache_dir field.
    let temp = tempfile::tempdir().unwrap();
    // This just tests that the field is accessible; actual init is #[ignore]d.
    let cache_dir = temp.path().to_path_buf();
    // We can test the field exists by checking struct layout at compile time
    // via a helper that returns cache_dir without initialising the model:
    let stored = Embedder::cache_dir_path(&cache_dir);
    assert_eq!(stored, cache_dir);
}
```

Note: `Embedder::cache_dir_path` is a trivial static helper added as part of
the green phase — it just returns its argument. The test is intentionally
minimal so it does not require model download.

---

#### Test group C — Config field

Tests live in whatever module defines `Config` (typically `src/config.rs`).

```rust
#[test]
fn test_config_embedder_reset_interval_default() {
    // Default must be 500 so existing users get the fix without any config change.
    let cfg = Config::default(); // or however default is constructed
    assert_eq!(cfg.embedder_reset_interval, 500);
}

#[test]
fn test_config_embedder_reset_interval_zero_valid() {
    // 0 is a valid value meaning "never reset"
    let mut cfg = Config::default();
    cfg.embedder_reset_interval = 0;
    assert_eq!(cfg.embedder_reset_interval, 0);
}
```

---

#### Test group D — Integration (marked `#[ignore]`, requires ONNX model)

These confirm that resetting the embedder produces semantically stable results
(same entity → same vector cosine similarity before and after reset).

```rust
#[ignore = "Downloads ONNX model (~23MB) and requires significant memory/CPU"]
#[test]
fn test_embedder_reset_produces_stable_vectors() {
    // Given the same embed_text embedded before and after an Embedder reset,
    // the resulting vectors must be identical (deterministic model).
    let temp = tempfile::tempdir().unwrap();
    let cache_dir = temp.path().to_path_buf();

    let entity_text = "[class] PaymentService\nFile: PaymentService.java:1";

    let vec_before = {
        let mut e = Embedder::init(cache_dir.clone()).unwrap();
        let v = e.embed_query(entity_text).unwrap();
        v
    };

    // Simulate reset: drop old Embedder, create new one from same cache.
    let vec_after = {
        let mut e = Embedder::init(cache_dir.clone()).unwrap();
        let v = e.embed_query(entity_text).unwrap();
        v
    };

    // Vectors must be identical (deterministic ONNX inference)
    assert_eq!(vec_before.len(), vec_after.len());
    let dot: f32 = vec_before.iter().zip(&vec_after).map(|(a, b)| a * b).sum();
    let norm_a: f32 = vec_before.iter().map(|x| x * x).sum::<f32>().sqrt();
    let norm_b: f32 = vec_after.iter().map(|x| x * x).sum::<f32>().sqrt();
    let cosine = dot / (norm_a * norm_b);
    assert!(
        cosine > 0.9999,
        "Vectors before and after reset must be near-identical, got cosine={cosine}"
    );
}

#[ignore = "Downloads ONNX model (~23MB) and requires significant memory/CPU"]
#[test]
fn test_embedder_reinit_from_cache_does_not_download() {
    // Given the model is already in cache, a second Embedder::init must not
    // attempt a download (observable via absence of download-progress log).
    // This is a smoke test; exact log validation is out of scope.
    let temp = tempfile::tempdir().unwrap();
    let cache_dir = temp.path().to_path_buf();
    let _first = Embedder::init(cache_dir.clone()).unwrap(); // downloads model
    let _second = Embedder::init(cache_dir.clone()).unwrap(); // must use cache
    // If this doesn't panic, the cache path is respected.
}
```

---

### Phase 2 — Green: Implementation steps

Implement in this exact order; run `cargo test --lib pipeline::embed` after
each step to confirm the corresponding tests turn green.

1. **Add `needs_reset` function** to `src/pipeline/embed.rs`:
   ```rust
   pub(crate) fn needs_reset(batch_count: usize, interval: usize) -> bool {
       interval > 0 && batch_count > 0 && batch_count % interval == 0
   }
   ```
   → Tests A pass.

2. **Add `cache_dir` field to `Embedder`** and `cache_dir_path` static helper:
   ```rust
   pub struct Embedder {
       model: TextEmbedding,
       cache_dir: std::path::PathBuf,
   }
   impl Embedder {
       pub fn cache_dir_path(p: &std::path::Path) -> std::path::PathBuf { p.to_path_buf() }
   }
   ```
   Store `cache_dir.clone()` in `init`. Add a `reinit(&mut self) -> Result<()>` method
   that calls `Embedder::init(self.cache_dir.clone())` and replaces `self.model`.
   → Tests B pass.

3. **Add `embedder_reset_interval: usize` to `Config`** with default `500`.
   Add CLI flag `--embedder-reset-interval` and env var `KNOT_EMBEDDER_RESET_INTERVAL`.
   → Tests C pass.

4. **Modify `run_indexing_pipeline` in `src/pipeline/runner.rs`**:
   - Clone `cache_dir` before moving into the embed_handle closure.
   - Pass `reset_interval = cfg.embedder_reset_interval` into the closure.
   - Inside the embed_handle loop, after `batch_count += 1`:
     ```rust
     if needs_reset(batch_count, reset_interval) {
         info!("[Worker: Embedder] Resetting ONNX session at batch #{batch_count} \
                to release BFCArena memory");
         let cache = cache_dir.clone();
         let fresh = tokio::task::spawn_blocking(move || Embedder::init(cache)).await??;
         let mut lock = embedder.lock().await;
         *lock = fresh;
     }
     ```
   - Repeat the same check before the final (remainder) batch.
   → Integration test D passes (manual run).

---

### Phase 3 — Refactor

- Move `needs_reset` to a private module if it grows beyond a single function.
- Confirm all clippy warnings resolved: `cargo clippy --all-targets -- -D warnings`.
- Confirm fmt: `cargo fmt -- --check`.

---

### BDD scenario (E2E — no new script needed)

The 17 existing E2E suites already verify end-to-end correctness. Because the
reset is transparent (same vectors, same entities), **no new E2E script is
needed**. The gate is:

```
./tests/run_all_e2e_fast.sh   →  all 17 suites PASS
```

To validate the fix specifically, one can set `KNOT_EMBEDDER_RESET_INTERVAL=1`
when running the TypeScript or Java E2E suite and confirm the index is complete
and searchable (interval=1 resets every batch — maximum stress test).

---

## Fix 2 — Scope CONTAINS Query to Current Batch

### Design

Replace the global `MATCH (m:Entity {repo_name: $repo_name})` in the
auto-link Cypher with an `UNWIND` over the UUIDs of the 64 entities just
upserted. This limits the MATCH to the current batch while **still searching
all Neo4j for the parent class** (`c1`, `c2` OPTIONAL MATCHes are unchanged —
they continue to scan the full graph). No correctness is lost.

Correctness argument:
- Entities in batch `k` need their `enclosing_class` linked.
- Their parent class was inserted in batch `j ≤ k` and already has its own
  `CONTAINS` links from when batch `j` ran.
- Searching all Neo4j for the parent (`c1`/`c2`) is correct and unchanged.
- Re-running MERGE for already-existing edges (old behaviour) is idempotent but
  wasteful; eliminating it is safe.

### File affected

`src/db/graph/upsert.rs` — `upsert_entities` method, lines 221–237.

### Phase 1 — Red: Unit tests to write first

All new tests below live in the `#[cfg(test)]` block of
`src/db/graph/upsert.rs`.

---

#### Test group E — CONTAINS Cypher structure

```rust
#[test]
fn test_contains_auto_link_uses_unwind_with_uuids() {
    // The new CONTAINS Cypher must start with UNWIND $entity_uuids
    let cypher = build_contains_auto_link_cypher(); // helper to extract the string
    assert!(
        cypher.contains("UNWIND $entity_uuids AS entity_uuid"),
        "CONTAINS query must use UNWIND over entity UUIDs, got:\n{cypher}"
    );
}

#[test]
fn test_contains_auto_link_matches_by_uuid() {
    let cypher = build_contains_auto_link_cypher();
    assert!(
        cypher.contains("MATCH (m:Entity {uuid: entity_uuid})"),
        "CONTAINS query must match by UUID not by repo_name scan, got:\n{cypher}"
    );
}

#[test]
fn test_contains_auto_link_does_not_scan_full_repo() {
    let cypher = build_contains_auto_link_cypher();
    // The old pattern was a full-repo scan; it must be gone.
    assert!(
        !cypher.contains("MATCH (m:Entity {repo_name:"),
        "CONTAINS query must NOT scan all repo entities, got:\n{cypher}"
    );
}

#[test]
fn test_contains_auto_link_preserves_optional_match_for_parent() {
    // Parent lookup must still search the full graph (no UUID restriction on c1/c2).
    let cypher = build_contains_auto_link_cypher();
    assert!(
        cypher.contains("OPTIONAL MATCH (c1:Entity {fqn: m.enclosing_class_fqn"),
        "CONTAINS query must still OPTIONAL MATCH parent by FQN across all entities"
    );
    assert!(
        cypher.contains("OPTIONAL MATCH (c2:Entity {name: m.enclosing_class"),
        "CONTAINS query must still OPTIONAL MATCH parent by name/file_path across all entities"
    );
}

#[test]
fn test_contains_auto_link_preserves_coalesce_fallback() {
    let cypher = build_contains_auto_link_cypher();
    assert!(
        cypher.contains("COALESCE(c1, c2)"),
        "CONTAINS query must still prefer FQN match (c1) over name fallback (c2)"
    );
}
```

The `build_contains_auto_link_cypher()` helper is a new **private** function
extracted from `upsert_entities` that returns the Cypher string as a
`&'static str` or `String`. Extracting it enables unit-testing the query
without hitting Neo4j.

---

#### Test group F — UUID list construction

```rust
#[test]
fn test_contains_uuid_list_contains_all_batch_uuids() {
    // The UUID list passed to CONTAINS must exactly match the UUIDs of the
    // entities in the current batch.
    let entities = vec![
        create_embedded_test_entity("ClassA", EntityKind::Class),
        create_embedded_test_entity("methodFoo", EntityKind::Method),
        create_embedded_test_entity("methodBar", EntityKind::Method),
    ];

    let uuids = extract_entity_uuids(&entities); // new helper

    assert_eq!(uuids.len(), entities.len());
    for (uuid, entity) in uuids.iter().zip(entities.iter()) {
        assert_eq!(uuid, &entity.entity.uuid.to_string());
    }
}

#[test]
fn test_contains_uuid_list_empty_for_empty_batch() {
    let entities: Vec<EmbeddedEntity> = vec![];
    let uuids = extract_entity_uuids(&entities);
    assert!(uuids.is_empty());
}
```

The `extract_entity_uuids` helper is a new **private** function that maps
`&[EmbeddedEntity]` → `Vec<String>`.

---

#### Test group G — Regression: cross-batch CONTAINS (existing test updated)

The existing test `test_contains_auto_link_uses_enclosing_class_fqn` must
continue to pass unchanged (it validates the parent-lookup logic which is not
being modified).

Add one new regression test:

```rust
#[test]
fn test_contains_cypher_repo_name_still_used_for_parent_lookup() {
    // The parent class lookup (c1/c2) must still be scoped to repo_name to
    // avoid false matches across repositories.
    let cypher = build_contains_auto_link_cypher();
    // c1 is scoped by repo_name
    assert!(cypher.contains("repo_name: $repo_name"),
        "Parent lookup must still filter by repo_name, got:\n{cypher}");
}
```

---

### Phase 2 — Green: Implementation steps

1. **Extract `build_contains_auto_link_cypher()`** as a private `fn` in
   `src/db/graph/upsert.rs` returning the Cypher `String`. Initial content
   is the existing query (to make existing tests still pass).
   → Run `cargo test --lib db::graph::upsert` — existing tests green.

2. **Extract `extract_entity_uuids(entities: &[EmbeddedEntity]) -> Vec<String>`**
   as a private `fn`.
   → Tests F green.

3. **Rewrite `build_contains_auto_link_cypher()`** to use UNWIND:
   ```rust
   fn build_contains_auto_link_cypher() -> &'static str {
       "UNWIND $entity_uuids AS entity_uuid
        MATCH (m:Entity {uuid: entity_uuid})
        WHERE m.enclosing_class IS NOT NULL AND m.enclosing_class <> ''
        OPTIONAL MATCH (c1:Entity {fqn: m.enclosing_class_fqn, repo_name: $repo_name})
        WITH m, c1
        OPTIONAL MATCH (c2:Entity {name: m.enclosing_class, repo_name: $repo_name, file_path: m.file_path})
        WITH m, COALESCE(c1, c2) AS c
        WHERE c IS NOT NULL
        MERGE (c)-[:CONTAINS]->(m)"
   }
   ```
   → Tests E green, tests G green.

4. **Update `upsert_entities`** to call both helpers:
   ```rust
   let entity_uuids = extract_entity_uuids(entities);
   let cypher = build_contains_auto_link_cypher();
   self.graph
       .run(
           query(cypher)
               .param("entity_uuids", entity_uuids)
               .param("repo_name", repo_name.clone()),
       )
       .await
       .context("Failed to auto-link CONTAINS relationships")?;
   ```
   → All unit tests green.

---

### Phase 3 — Refactor

- Confirm `build_contains_auto_link_cypher` is `pub(crate)` if needed by tests,
  otherwise `fn` (module-private).
- Ensure `extract_entity_uuids` is also module-private.
- Run `cargo clippy --all-targets -- -D warnings` and `cargo fmt -- --check`.

---

### BDD scenario (E2E regression)

The critical correctness property to validate is: **methods indexed in a later
batch than their enclosing class still receive correct CONTAINS edges**.

This is implicitly tested by any E2E suite that indexes a Java, Kotlin, or
TypeScript class with methods, because the batch split is non-deterministic
(depends on file order and entity count). The Java E2E suite
(`tests/run_java_e2e.sh`) and TypeScript suite (`tests/run_typescript_e2e.sh`)
both query for entities and their relationships, providing sufficient coverage.

Additionally, add one explicit BDD-style assertion to `run_java_e2e.sh` (or a
new `run_contains_regression_e2e.sh` if preferred):

```
SCENARIO: Class and methods indexed in different batches retain CONTAINS edges

  GIVEN  a Java fixture file with class PaymentService (1 entity)
         and methods processPayment, validateCard, refund (3 entities)
  WHEN   batch_size=2 so the class and methods land in separate batches
  THEN   Neo4j contains:
           (PaymentService)-[:CONTAINS]->(processPayment)
           (PaymentService)-[:CONTAINS]->(validateCard)
           (PaymentService)-[:CONTAINS]->(refund)
  AND    no CONTAINS edges are missing
```

If a dedicated E2E fixture is added for this scenario, it must:
- Live under `tests/testing_files/java/contains_regression/`
- Be indexed with `--batch-size 2` to force cross-batch entity splits
- Assert CONTAINS relationships via the `knot` CLI's `explore` or `callers` command

---

## Quality Gates (must all pass before merging)

```bash
# 1. Unit tests (fast, no ONNX, no DB)
cargo test --lib

# 2. Lint
cargo clippy --all-targets -- -D warnings

# 3. Format
cargo fmt -- --check

# 4. E2E (requires Docker + databases)
./tests/run_all_e2e_fast.sh
```

All 17 E2E suites must pass. No new `#[ignore]` integration tests may be left
failing without explicit justification in the PR description.

---

## Sequencing recommendation

Implement Fix 2 first. It is a pure Cypher change with no config surface, is
easier to test in isolation, and reduces Neo4j load during any subsequent
validation of Fix 1 on real repos.

```
Fix 2 (CONTAINS query)
  → all tests green
  → E2E green
  → merge

Fix 1 (Embedder reset)
  → all tests green
  → E2E green
  → manual smoke test on large repo with KNOT_EMBEDDER_RESET_INTERVAL=100
  → merge
```

---

## Non-goals

- Changing the embedding model (separate issue).
- Modifying the `embed_text` truncation logic (model already truncates at 256
  tokens; no content is gained by pre-truncating the raw string).
- Tuning ONNX session options directly (the periodic reset achieves the same
  memory release with zero dependency on internal ONNX API surface).
