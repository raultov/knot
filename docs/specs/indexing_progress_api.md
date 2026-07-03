# Indexing Progress API: File-Based Progress Tracking & Logging

**Status:** Planned — specification only, no implementation yet
**Approach:** TDD/BDD — tests written first, must fail before implementation begins
**Downstream consumer:** [knot-server](https://github.com/raultov/knot-server) (REST API)

---

## Problem Summary

The indexing pipeline (`src/pipeline/runner.rs`) provides no way to observe
progress while a run is in flight:

1. **Logs** — progress is only inferable from scattered `tracing::info!` lines
   (`"Embedding batch #N"`, `"Ingesting batch #N"`). There is no line that says
   *"we are X% done"* because no stage knows both the total work and the work
   completed so far.
2. **Library API** — `run_indexing_pipeline()` is a black box: it returns
   `RunMetrics` only **after** the run completes. A caller such as
   `knot-server` (which spawns indexing runs from REST endpoints) has no way
   to answer *"how far along is the indexing of repo X?"* while the run is
   executing.

### Goal

- Emit a progress log line **after each ingested batch**, e.g. with 5 000
  files to index and 1 000 files parsed:
  `[Progress] [my-repo] 1000/5000 files (20.0%) — batch #16 ingested (64 entities)`
- Expose a **thread-safe, poll-able progress API** in the `knot` library so
  `knot-server` can implement `GET /repos/{name}/progress` returning a JSON
  snapshot of the current run.

### Non-Goals

- No push/streaming notification mechanism (no channels, no SSE hooks). The
  API is **pull-based**: the caller polls a snapshot. knot-server can layer
  SSE/WebSocket on top by polling internally if it ever wants to.
- No persistence of progress across process restarts. Progress is in-memory
  and per-run. `.knot/index_state.json` is not touched.
- No progress for the MCP server (`knot-mcp`) or CLI query tool (`knot`) —
  they do not index.
- No entity-level progress. **The unit of progress is the file** (decision
  confirmed): the total is known up front, whereas entity counts per file are
  unknown until parsed.

---

## Design Overview

### Unit of progress: files

`percent = parsed_files / total_files * 100`

- `total_files` = `files_to_parse.len()` — known immediately after file
  classification (`runner.rs:91`), *before* parsing starts. This counts only
  added + modified files for the current run (incremental semantics), not the
  whole repo.
- `parsed_files` = number of files whose parsing has finished (successfully
  **or** with a parse error — an unparseable file is still "processed").

**Why not "files fully ingested"?** Entities from one file are interleaved
across embed batches (batching happens on the entity stream, not per file, see
`runner.rs:141-143`), so "all entities of file F are ingested" would require
tagging every entity with a per-file entity count and reference-counting at
the ingest sink. That complexity buys little: parsing dominates wall-clock
share per file and file-level parse completion is a monotonically increasing,
cheap, exact signal. The `stage` field (below) disambiguates the tail of the
run where parsing is at 100% but ingestion/resolution is still working.

### Stage state machine

```
Idle ──► Discovering ──► Classifying ──► CleaningStaleData ──► Parsing ──► Ingesting ──► ResolvingReferences ──► Completed
                              │                                                                                       ▲
                              └────────────── (no changes / empty repo) ──────────────────────────────────────────────┘
              any stage ──► Failed (error message captured in snapshot)
```

Notes:
- `Parsing` and `Ingesting` overlap in reality (streaming pipeline). The
  tracker reports `Parsing` from parse start until the parse thread pool
  completes, then `Ingesting` until the ingest `JoinSet` drains, then
  `ResolvingReferences`. This is a simplification for consumers; the
  file counters remain exact regardless of stage.
- Watch mode re-enters the state machine on every cycle: each
  `run_indexing_pipeline` call resets counters via `begin_run()` (see below).

---

## New Module: `src/pipeline/progress.rs`

Registered in `src/pipeline/mod.rs` as `pub mod progress;` (thus reachable as
`knot::pipeline::progress::*`).

### Public types

```rust
use serde::Serialize;
use std::sync::RwLock;
use std::sync::atomic::{AtomicU64, Ordering};

/// Pipeline stage for consumer display. Unit variants only so that it
/// serializes as a plain snake_case string (error text lives in
/// `IndexingProgress::error`, not in the enum).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum IndexingStage {
    Idle,
    Discovering,
    Classifying,
    CleaningStaleData,
    Parsing,
    Ingesting,
    ResolvingReferences,
    Completed,
    Failed,
}

/// Immutable snapshot of a run's progress. `Serialize` so knot-server can
/// return it directly as the JSON body of a progress endpoint.
#[derive(Debug, Clone, Serialize)]
pub struct IndexingProgress {
    /// Logical repository name (`Config::repo_name`).
    pub repo_name: String,
    /// Current pipeline stage.
    pub stage: IndexingStage,
    /// Files scheduled for parsing in this run (added + modified).
    pub total_files: u64,
    /// Files whose parsing has completed (success or parse error).
    pub parsed_files: u64,
    /// File-based completion percentage, one-decimal precision semantics.
    /// Invariants: 0.0 when total_files == 0 and stage is not terminal;
    /// 100.0 when stage == Completed (even for empty/no-change runs).
    pub percent_complete: f32,
    /// Entities ingested into Qdrant/Neo4j so far (informational).
    pub entities_ingested: u64,
    /// Ingest batches completed so far (informational).
    pub batches_ingested: u64,
    /// Error message when stage == Failed, otherwise None.
    pub error: Option<String>,
}

/// Thread-safe, cheaply cloneable (via `Arc`) progress tracker.
///
/// Counters are lock-free atomics; only the stage/error/repo_name fields sit
/// behind an `RwLock` (written rarely — on stage transitions).
pub struct ProgressTracker {
    repo_name: RwLock<String>,
    stage: RwLock<IndexingStage>,
    error: RwLock<Option<String>>,
    total_files: AtomicU64,
    parsed_files: AtomicU64,
    entities_ingested: AtomicU64,
    batches_ingested: AtomicU64,
}
```

### Method surface

```rust
impl ProgressTracker {
    /// Fresh tracker in stage Idle with all counters at zero.
    pub fn new() -> Self;

    /// Point-in-time snapshot. Never blocks writers for long: reads two
    /// RwLocks (read side) and four atomics.
    pub fn snapshot(&self) -> IndexingProgress;

    // ---- mutation API: pub(crate), called only from runner.rs ----

    /// Reset all counters and set stage = Discovering. Called at the top of
    /// every `run_indexing_pipeline` invocation (critical for watch mode,
    /// where one tracker observes many consecutive runs).
    pub(crate) fn begin_run(&self, repo_name: &str);

    pub(crate) fn set_stage(&self, stage: IndexingStage);

    /// Store total_files. Called once per run after classification.
    pub(crate) fn set_total_files(&self, total: u64);

    /// += 1 on parsed_files. Called from the parse-completion callback.
    pub(crate) fn incr_parsed_files(&self);

    /// Record one ingested batch: batches_ingested += 1,
    /// entities_ingested += entity_count.
    pub(crate) fn record_batch_ingested(&self, entity_count: u64);

    /// stage = Completed. Forces percent_complete to 100.0 in snapshots.
    pub(crate) fn complete(&self);

    /// stage = Failed, error = Some(msg).
    pub(crate) fn fail(&self, msg: &str);
}

impl Default for ProgressTracker {
    fn default() -> Self { Self::new() }
}
```

### Percent computation rules (in `snapshot()`)

| Condition | `percent_complete` |
|---|---|
| `stage == Completed` | `100.0` |
| `stage == Failed` | last computed value (frozen counters) |
| `total_files == 0` | `0.0` |
| otherwise | `(parsed_files as f32 / total_files as f32) * 100.0`, clamped to `[0.0, 100.0]` |

Atomic ordering: `Ordering::Relaxed` is sufficient for all counters — they are
independent monotonic counters used for display; no cross-counter invariant
requires acquire/release synchronization.

---

## Change 1 — `src/pipeline/parser/mod.rs`: per-file completion callback

`parse_files_stream` (`src/pipeline/parser/mod.rs:82`) gains an optional
callback so the parse stage can report per-file completion **without knowing
about `ProgressTracker`** (keeps the parser decoupled from progress):

```rust
/// Callback invoked exactly once per input file after the file has been
/// fully processed (all entities sent to the channel, or parse failed).
pub type FileParsedCallback = std::sync::Arc<dyn Fn() + Send + Sync>;

pub fn parse_files_stream(
    files: &[PathBuf],
    parse_cfg: &ParseConfig,
    sender: mpsc::Sender<ParsedEntity>,
    max_concurrent: usize,
    on_file_parsed: Option<FileParsedCallback>,   // NEW
)
```

Inside the per-file spawned thread (`parser/mod.rs:110-130`), invoke the
callback **after** the `match parse_single_file(...)` block — i.e. on both the
`Ok` path (after the entity send loop) and the `Err` path — and **before**
releasing the semaphore:

```rust
s.spawn(move || {
    match parse_single_file(&path, &parse_cfg) {
        Ok(entities) => { /* existing send loop */ }
        Err(e) => { warn!(...); }
    }
    if let Some(cb) = &on_file_parsed {
        cb();                                    // NEW — exactly once per file
    }
    /* existing semaphore release */
});
```

Also invoke the callback when `sender.blocking_send(entity)` fails and the
loop `break`s — the file counts as processed (the run is aborting anyway; the
counter must stay ≤ total either way, and calling it keeps "exactly once per
file" simple and testable).

**Ripple effects:**
- `parse_files()` wrapper (`parser/mod.rs:140`) passes `None`.
- `runner.rs:126` passes the runner's callback (see Change 2).
- Any unit tests / test utilities calling `parse_files_stream` directly get
  `None` appended (search for call sites during implementation:
  `parse_files_stream(` across `src/`).

---

## Change 2 — `src/pipeline/runner.rs`: wire the tracker + progress log

### 2.1 Non-breaking public API (additive, no semver break)

`run_indexing_pipeline` is public library API consumed by knot-server. To
avoid a breaking change on the 1.x line, add a `_with_progress` variant and
keep the existing signature delegating to it:

```rust
/// Existing entry point — unchanged signature. Creates a private throwaway
/// tracker internally so progress *logging* always happens (knot-indexer
/// gets the log lines for free without opting into the API).
pub async fn run_indexing_pipeline(
    cfg: &Config,
    vector_db: &Arc<VectorDb>,
    graph_db: &Arc<GraphDb>,
    index_state: &mut IndexState,
) -> Result<RunMetrics> {
    run_indexing_pipeline_with_progress(
        cfg, vector_db, graph_db, index_state,
        Arc::new(ProgressTracker::new()),
    ).await
}

/// New entry point for callers that want to observe progress (knot-server).
/// The caller keeps a clone of `progress` and polls `snapshot()` from
/// another task/thread while this future runs.
pub async fn run_indexing_pipeline_with_progress(
    cfg: &Config,
    vector_db: &Arc<VectorDb>,
    graph_db: &Arc<GraphDb>,
    index_state: &mut IndexState,
    progress: Arc<ProgressTracker>,
) -> Result<RunMetrics>
```

The tracker parameter is **mandatory** on the `_with_progress` variant (no
`Option`): the delegating wrapper already covers the "don't care" case, and a
non-optional parameter removes `if let Some(...)` noise at every update site
inside the runner.

### 2.2 Completion/failure wrapping

Rename the current body of `run_indexing_pipeline` to a private
`run_pipeline_inner(...)` and let `run_indexing_pipeline_with_progress` own
the tracker lifecycle so `Failed`/`Completed` are set on **every** exit path
without touching each `?`:

```rust
pub async fn run_indexing_pipeline_with_progress(..., progress: Arc<ProgressTracker>) -> Result<RunMetrics> {
    progress.begin_run(&cfg.repo_name);
    let result = run_pipeline_inner(cfg, vector_db, graph_db, index_state, &progress).await;
    match &result {
        Ok(_) => progress.complete(),
        Err(e) => progress.fail(&format!("{e:#}")),
    }
    result
}
```

### 2.3 Update points inside `run_pipeline_inner` (current line refs)

| Location (today) | Action |
|---|---|
| entry (before `discover_files`, runner.rs:48) | `begin_run` already set stage `Discovering` — nothing extra |
| runner.rs:49-52 (empty repo early return) | return `Ok` → wrapper sets `Completed` (total stays 0, percent forced to 100.0) |
| before `classify_files_for_indexing` (runner.rs:54) | `progress.set_stage(Classifying)` |
| runner.rs:60-63 (no-changes early return) | return `Ok` → wrapper sets `Completed` |
| before `clean_stale_data` (runner.rs:80) | `progress.set_stage(CleaningStaleData)` |
| after `calculate_files_to_parse` (runner.rs:91) | `progress.set_total_files(files_to_parse.len() as u64)` |
| before spawning the parse task (runner.rs:125) | `progress.set_stage(Parsing)`; build callback: `let tracker = Arc::clone(&progress); let cb: FileParsedCallback = Arc::new(move \|\| tracker.incr_parsed_files());` pass `Some(cb)` to `parse_files_stream` (runner.rs:126) |
| inside the spawn_blocking closure, after `parse_files_stream` returns (runner.rs:127) | `progress.set_stage(Ingesting)` (parsing is done; ingest continues draining) — requires cloning the `Arc` into the closure |
| ingest task, after `join_set.spawn(...)` per batch (runner.rs:228-251) | `progress.record_batch_ingested(embedded_batch.len() as u64)` + **emit the progress log line** (see 2.4) — requires cloning the `Arc` into the `ingest_handle` task |
| before `resolve_and_save_relationships` (runner.rs:283) | `progress.set_stage(ResolvingReferences)` |
| deletions-only branch (runner.rs:296-299) | nothing extra; wrapper sets `Completed` |

Note on the `Parsing → Ingesting` transition: the stage is flipped by the
parse task when it finishes, while the ingest task may already have been
ingesting for a while — this matches the state-machine simplification
declared above and needs a code comment at the transition site.

### 2.4 Progress log line (the user-visible deliverable)

Emitted in the ingest loop **once per batch**, immediately after
`record_batch_ingested`:

```rust
let snap = progress.snapshot();
info!(
    "[Progress] [{}] {}/{} files ({:.1}%) — batch #{} ingested ({} entities)",
    snap.repo_name, snap.parsed_files, snap.total_files,
    snap.percent_complete, snap.batches_ingested, /* this batch len */
);
```

Example with 5 000 files where 1 000 have been parsed:

```
[Progress] [my-repo] 1000/5000 files (20.0%) — batch #16 ingested (64 entities)
```

Additionally, one final log line right before `resolve_and_save_relationships`
so logs always show 100% parse completion even if the last batches raced:

```
[Progress] [my-repo] 5000/5000 files (100.0%) — parsing and ingestion complete, resolving references...
```

No throttling: the user explicitly asked for per-batch emission, and batch
frequency is already bounded by embedding throughput (~1 line/sec typical).

---

## Change 3 — `src/pipeline/watch.rs`: thread the tracker through watch mode

`setup_watch_mode` (`watch.rs:27`) gains the same treatment so knot-server
(or a future watch-capable server) can observe per-cycle progress:

```rust
pub async fn setup_watch_mode(
    cfg: &Config,
    vector_db: &Arc<VectorDb>,
    graph_db: &Arc<GraphDb>,
    index_state: &mut IndexState,
) -> Result<()> {
    setup_watch_mode_with_progress(
        cfg, vector_db, graph_db, index_state,
        Arc::new(ProgressTracker::new()),
    ).await
}

pub async fn setup_watch_mode_with_progress(
    ..., progress: Arc<ProgressTracker>,
) -> Result<()>
```

Inside the event loop (`watch.rs:108`), replace the `run_indexing_pipeline`
call with `run_indexing_pipeline_with_progress(..., Arc::clone(&progress))`.
Each cycle's `begin_run()` resets the counters, so a poller always sees the
progress of the **latest** cycle (previous cycles end at `Completed`, which
persists between cycles until the next `begin_run`).

---

## Change 4 — `src/pipeline/mod.rs` and `src/lib.rs`: exports

- `src/pipeline/mod.rs`: add `pub mod progress;`
- `src/lib.rs`: no structural change needed (`knot::pipeline::progress::...`
  is already reachable), but add a convenience re-export for downstream
  ergonomics:

```rust
// src/pipeline/mod.rs
pub use progress::{IndexingProgress, IndexingStage, ProgressTracker};
```

so knot-server writes `use knot::pipeline::{ProgressTracker, IndexingProgress};`.

## Change 5 — `src/bin/knot-indexer.rs`: no change required

`main` (knot-indexer.rs:51) keeps calling the unchanged
`run_indexing_pipeline` signature and gets the `[Progress]` log lines for free
via the internal throwaway tracker (see 2.1). Same for the watch-mode call at
knot-indexer.rs:65.

---

## knot-server Integration Sketch (out of scope for this repo, for reference)

```rust
// Server-side registry (knot-server code, NOT knot):
struct AppState {
    trackers: DashMap<String, Arc<ProgressTracker>>, // repo_name → tracker
}

// POST /repos/{name}/index — spawn a run:
let tracker = Arc::new(ProgressTracker::new());
state.trackers.insert(repo_name.clone(), Arc::clone(&tracker));
tokio::spawn(async move {
    let _ = run_indexing_pipeline_with_progress(&cfg, &vdb, &gdb, &mut index_state, tracker).await;
});

// GET /repos/{name}/progress:
let tracker = state.trackers.get(&repo_name).ok_or(404)?;
Json(tracker.snapshot())
// → { "repo_name": "my-repo", "stage": "ingesting", "total_files": 5000,
//     "parsed_files": 1000, "percent_complete": 20.0,
//     "entities_ingested": 1024, "batches_ingested": 16, "error": null }
```

The `Serialize` derive on `IndexingProgress`/`IndexingStage` is what makes
this one-liner possible; knot-server adds no mapping layer.

---

## Concurrency & Safety Analysis

- **Writers:** parse callback fires from N OS threads (`std::thread::scope`
  in `parse_files_stream`); `record_batch_ingested` fires from the tokio
  ingest task; stage transitions fire from the orchestrating async fn and the
  spawn_blocking parse task. All mutation goes through atomics or short
  `RwLock` write guards — no `await` is ever held across a lock guard.
- **Readers:** `snapshot()` may be called from any thread at any time
  (knot-server handler). It takes only read locks + relaxed atomic loads.
- **No `unsafe`**, per repo policy.
- **Snapshot consistency:** counters are read independently, so a snapshot
  may be *slightly* torn (e.g. `parsed_files` from instant T, `batches_ingested`
  from T+ε). Acceptable for a progress display; documented on `snapshot()`.
- **Monotonicity guarantee:** within one run, `parsed_files` never exceeds
  `total_files` because the callback fires exactly once per file in
  `files_to_parse`. A unit test enforces this.

---

## Testing Plan (TDD — write first, watch them fail)

### Unit tests — `src/pipeline/progress.rs` (inline `#[cfg(test)]`)

1. `test_new_tracker_is_idle_zeroed` — fresh tracker: stage `Idle`, all
   counters 0, percent 0.0, error `None`.
2. `test_percent_basic` — total 5000, incr parsed 1000× → snapshot
   `percent_complete == 20.0` (the exact scenario from the requirement).
3. `test_percent_zero_total` — total 0, stage `Parsing` → 0.0 (no NaN/inf).
4. `test_completed_forces_100` — total 0 + `complete()` → 100.0, stage
   `Completed` (empty-repo / no-change runs).
5. `test_fail_records_error` — `fail("boom")` → stage `Failed`,
   `error == Some("boom")`, counters frozen.
6. `test_begin_run_resets` — populate counters, `begin_run("repo2")` →
   all zeroed, stage `Discovering`, repo_name `"repo2"`, error cleared
   (watch-mode cycle semantics).
7. `test_concurrent_increments` — spawn 8 threads × 1000
   `incr_parsed_files()` → `parsed_files == 8000` exactly (atomicity).
8. `test_record_batch_accumulates` — 3 batches of 64/64/10 →
   `batches_ingested == 3`, `entities_ingested == 138`.
9. `test_serialize_snapshot_json` — `serde_json::to_value(snapshot)` →
   stage renders as `"ingesting"` (snake_case string), all fields present.
10. `test_percent_clamped` — artificially incr parsed beyond total → 100.0
    max (defensive clamp).

### Unit tests — `src/pipeline/parser/mod.rs`

11. `test_parse_files_stream_callback_once_per_file` — fixture dir with 3
    valid files → callback counter == 3 and equals number of input files.
12. `test_parse_files_stream_callback_counts_unparseable` — 2 valid + 1
    file with an unsupported/broken parse → callback counter == 3 (errors
    still count as processed).
13. `test_parse_files_stream_none_callback` — existing behavior unchanged
    when `None` is passed (regression guard; adapt one existing test).

### Unit tests — `src/pipeline/runner.rs`

14. `test_run_indexing_pipeline_delegates` — compile-level guarantee that the
    legacy signature still exists and delegates (call it against an empty
    temp repo pattern like the existing
    `test_run_indexing_pipeline_empty_repo` scaffold, if DB mocking permits;
    otherwise assert-by-compilation via the wrapper's existence).

### E2E — extend one existing suite (no new suite needed)

15. In `tests/run_rust_e2e.sh` (smallest fixture set), after the indexing
    step, grep the indexer log output for the progress line contract:
    - at least one line matching `\[Progress\] \[.*\] [0-9]+/[0-9]+ files \([0-9.]+%\)`
    - a final line containing `100.0%`
    This pins the log format as a contract for humans/scripts parsing logs.
16. Full regression gate: `./tests/run_all_e2e_fast.sh` — all suites pass.

### Quality gate (per repo policy)

```bash
cargo fmt -- --check
cargo clippy --all-targets -- -D warnings
cargo test
./tests/run_all_e2e_fast.sh
```

---

## Documentation Updates

- **README.md** — new subsection under the indexer docs: "Indexing progress"
  covering (a) the `[Progress]` log line format, (b) the library API
  (`run_indexing_pipeline_with_progress` + `ProgressTracker::snapshot`) with
  the knot-server usage sketch.
- **AGENTS.md** — add `progress.rs` to the pipeline module list if the
  architecture section is touched (optional).
- **`.prompt` / `.knot-agent.md`** — no changes: MCP tool behavior is
  unaffected.

---

## Versioning & Rollout

- **Additive API only** (new module, new `_with_progress` functions, one new
  optional parameter on `parse_files_stream`). `parse_files_stream` *is*
  technically public, so appending a parameter is a breaking change for
  direct external callers — accepted risk: it is an internal-ish streaming
  primitive and knot-server consumes `run_indexing_pipeline`, which keeps its
  exact signature. Judged as a **minor** bump (target: next 1.5.x release
  together with whatever ships next; no dedicated release required).
- knot-server adopts the new API in a follow-up PR on its own repo:
  tracker registry + `GET /repos/{name}/progress` endpoint.

## Implementation Order (for the future PR)

1. **Red:** add all unit tests above (progress module skeleton with
   `todo!()` bodies so tests compile but fail; parser tests against the new
   parameter).
2. **Green:** implement `ProgressTracker`, the parser callback, the runner
   wiring, the watch-mode passthrough.
3. **Refactor:** extract `run_pipeline_inner`, verify no clippy debt.
4. **E2E gate:** add the log-grep assertions to `run_rust_e2e.sh`, run
   `./tests/run_all_e2e_fast.sh`.
5. **Docs:** README section.

## Open Questions (non-blocking, defaults chosen)

| Question | Default chosen |
|---|---|
| Should `Failed` freeze `percent_complete` or zero it? | Freeze (shows how far it got before dying) |
| Should the tracker expose elapsed time (`started_at`)? | Not in v1 — knot-server can timestamp on its side; add later if needed |
| Log throttling for very fast batches? | None — per-batch emission is the explicit requirement |
| `Option<Arc<ProgressTracker>>` vs mandatory param? | Mandatory on `_with_progress`, legacy wrapper covers the None case |
