use serde::Serialize;
use std::sync::RwLock;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

/// Lower bound of the ingest band, expressed as a percentage of the whole run.
/// `parse band`     → 0% .. PARSE_BAND_END
/// `ingest band`    → PARSE_BAND_END .. INGEST_BAND_END
const PARSE_BAND_END: f32 = 10.0;
/// Upper bound of the ingest band.
const INGEST_BAND_END: f32 = 90.0;
/// Percentage reported while the runner is in `ResolvingReferences`. No
/// sub-counters are available for resolution, so we hold the bar at a fixed
/// known-good value between the ingest ceiling (90%) and completion (100%).
const RESOLVING_PERCENT: f32 = 95.0;

/// Inputs to the banded percentage formula.
///
/// Grouped into a struct so the pure helper `compute_percent` stays under the
/// `too-many-arguments-threshold = 5` clippy lint and is trivially unit
/// testable without spinning up a `ProgressTracker`.
struct PercentInputs {
    stage: IndexingStage,
    total_files: u64,
    parsed_files: u64,
    /// `Some(n)` once the parser has published its total; `None` until then.
    /// `Some(0)` and `None` are distinct (the former is a legitimate empty
    /// repo; the latter is "still aggregating").
    total_entities: Option<u64>,
    entities_ingested: u64,
}

/// Banded percentage formula (v1.6.2).
///
/// Resolution order — see `docs/specs/indexing_progress_accuracy_plan.md` §3.2:
/// 1. `Completed`                          → `100.0`
/// 2. `ResolvingReferences`                → `RESOLVING_PERCENT`
/// 3. `total_entities` known               → `PARSE_BAND_END + ratio * (INGEST_BAND_END - PARSE_BAND_END)`
/// 4. parse band available (files > 0)     → `(parsed_files / total_files) * PARSE_BAND_END`
/// 5. otherwise                            → `0.0`
///
/// `Failed` deliberately falls through to steps 3-5 — the bar freezes at
/// whatever value the work-in-progress has produced, never resets to 0.
fn compute_percent(inputs: &PercentInputs) -> f32 {
    if inputs.stage == IndexingStage::Completed {
        return 100.0;
    }
    if inputs.stage == IndexingStage::ResolvingReferences {
        return RESOLVING_PERCENT;
    }
    if let Some(total_entities) = inputs.total_entities {
        // Ingest band: 10% .. 90% of the whole run.
        // If total_entities is 0 the band collapses instantly to its ceiling
        // (the parsing run produced nothing to ingest).
        let ratio = if total_entities == 0 {
            1.0
        } else {
            (inputs.entities_ingested as f32 / total_entities as f32).clamp(0.0, 1.0)
        };
        return PARSE_BAND_END + ratio * (INGEST_BAND_END - PARSE_BAND_END);
    }
    if inputs.total_files > 0 {
        let raw = (inputs.parsed_files as f32 / inputs.total_files as f32) * PARSE_BAND_END;
        return raw.clamp(0.0, PARSE_BAND_END);
    }
    0.0
}

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

#[derive(Debug, Clone, Serialize)]
pub struct IndexingProgress {
    pub repo_name: String,
    pub stage: IndexingStage,
    pub total_files: u64,
    pub parsed_files: u64,
    pub percent_complete: f32,
    pub entities_ingested: u64,
    pub batches_ingested: u64,
    pub total_entities: u64,
    pub error: Option<String>,
}

pub struct ProgressTracker {
    repo_name: RwLock<String>,
    stage: RwLock<IndexingStage>,
    error: RwLock<Option<String>>,
    total_files: AtomicU64,
    parsed_files: AtomicU64,
    entities_ingested: AtomicU64,
    batches_ingested: AtomicU64,
    total_entities: AtomicU64,
    total_entities_known: AtomicBool,
}

impl ProgressTracker {
    pub fn new() -> Self {
        Self {
            repo_name: RwLock::new(String::new()),
            stage: RwLock::new(IndexingStage::Idle),
            error: RwLock::new(None),
            total_files: AtomicU64::new(0),
            parsed_files: AtomicU64::new(0),
            entities_ingested: AtomicU64::new(0),
            batches_ingested: AtomicU64::new(0),
            total_entities: AtomicU64::new(0),
            total_entities_known: AtomicBool::new(false),
        }
    }

    pub fn snapshot(&self) -> IndexingProgress {
        let stage = *self.stage.read().unwrap();
        let total = self.total_files.load(Ordering::Relaxed);
        let parsed = self.parsed_files.load(Ordering::Relaxed);
        let entities = self.entities_ingested.load(Ordering::Relaxed);
        let batches = self.batches_ingested.load(Ordering::Relaxed);
        let total_entities_value = self.total_entities.load(Ordering::Acquire);
        let total_entities_known = self.total_entities_known.load(Ordering::Acquire);

        // The `known` flag is consumed here so its semantics are local to
        // the snapshot computation. Without it we could not distinguish
        // "parser has not finished aggregating yet" from "parser finished
        // and produced zero entities".
        let total_entities = if total_entities_known {
            Some(total_entities_value)
        } else {
            None
        };

        let pct = compute_percent(&PercentInputs {
            stage,
            total_files: total,
            parsed_files: parsed,
            total_entities,
            entities_ingested: entities,
        });

        IndexingProgress {
            repo_name: self.repo_name.read().unwrap().clone(),
            stage,
            total_files: total,
            parsed_files: parsed,
            percent_complete: pct,
            entities_ingested: entities,
            batches_ingested: batches,
            total_entities: total_entities_value,
            error: self.error.read().unwrap().clone(),
        }
    }

    pub(crate) fn begin_run(&self, repo_name: &str) {
        *self.repo_name.write().unwrap() = repo_name.to_string();
        *self.stage.write().unwrap() = IndexingStage::Discovering;
        *self.error.write().unwrap() = None;
        self.total_files.store(0, Ordering::Relaxed);
        self.parsed_files.store(0, Ordering::Relaxed);
        self.entities_ingested.store(0, Ordering::Relaxed);
        self.batches_ingested.store(0, Ordering::Relaxed);
        self.total_entities.store(0, Ordering::Release);
        self.total_entities_known.store(false, Ordering::Release);
    }

    pub(crate) fn set_stage(&self, stage: IndexingStage) {
        *self.stage.write().unwrap() = stage;
    }

    pub(crate) fn set_total_files(&self, total: u64) {
        self.total_files.store(total, Ordering::Relaxed);
    }

    pub(crate) fn incr_parsed_files(&self) {
        self.parsed_files.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn record_batch_ingested(&self, entity_count: u64) {
        self.batches_ingested.fetch_add(1, Ordering::Relaxed);
        self.entities_ingested
            .fetch_add(entity_count, Ordering::Relaxed);
    }

    /// Record the total number of entities the parse stage produced.
    ///
    /// Called exactly once, after the parser has finished collecting entities
    /// from every file and before they are pushed into the bounded channel.
    /// `0` is a legal value (empty repository), so the caller uses a separate
    /// `AtomicBool` (`total_entities_known`) to disambiguate "not yet seen"
    /// from "saw the value and it was zero".
    pub(crate) fn set_total_entities(&self, n: u64) {
        self.total_entities.store(n, Ordering::Release);
        self.total_entities_known.store(true, Ordering::Release);
    }

    pub(crate) fn complete(&self) {
        *self.stage.write().unwrap() = IndexingStage::Completed;
    }

    pub(crate) fn fail(&self, msg: &str) {
        *self.stage.write().unwrap() = IndexingStage::Failed;
        *self.error.write().unwrap() = Some(msg.to_string());
    }
}

impl Default for ProgressTracker {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_tracker_is_idle_zeroed() {
        let t = ProgressTracker::new();
        let snap = t.snapshot();
        assert_eq!(snap.stage, IndexingStage::Idle);
        assert_eq!(snap.total_files, 0);
        assert_eq!(snap.parsed_files, 0);
        assert_eq!(snap.percent_complete, 0.0);
        assert_eq!(snap.entities_ingested, 0);
        assert_eq!(snap.batches_ingested, 0);
        assert!(snap.error.is_none());
    }

    #[test]
    fn test_percent_basic() {
        let t = ProgressTracker::new();
        t.set_total_files(5000);
        t.set_stage(IndexingStage::Parsing);
        for _ in 0..1000 {
            t.incr_parsed_files();
        }
        let snap = t.snapshot();
        // 1000/5000 = 20% of the parse band (which is 0..10%) → 2.0
        assert_eq!(snap.percent_complete, 2.0);
    }

    #[test]
    fn test_percent_zero_total() {
        let t = ProgressTracker::new();
        t.set_stage(IndexingStage::Parsing);
        let snap = t.snapshot();
        assert_eq!(snap.percent_complete, 0.0);
    }

    #[test]
    fn test_completed_forces_100() {
        let t = ProgressTracker::new();
        t.complete();
        let snap = t.snapshot();
        assert_eq!(snap.stage, IndexingStage::Completed);
        assert_eq!(snap.percent_complete, 100.0);
    }

    #[test]
    fn test_fail_records_error() {
        let t = ProgressTracker::new();
        t.set_total_files(100);
        t.incr_parsed_files();
        t.fail("boom");
        let snap = t.snapshot();
        assert_eq!(snap.stage, IndexingStage::Failed);
        assert_eq!(snap.error, Some("boom".to_string()));
        assert_eq!(snap.parsed_files, 1);
    }

    #[test]
    fn test_begin_run_resets() {
        let t = ProgressTracker::new();
        t.begin_run("repo1");
        t.set_total_files(100);
        t.incr_parsed_files();
        t.record_batch_ingested(10);
        t.set_total_entities(500);
        t.fail("oops");
        t.begin_run("repo2");
        let snap = t.snapshot();
        assert_eq!(snap.repo_name, "repo2");
        assert_eq!(snap.stage, IndexingStage::Discovering);
        assert_eq!(snap.total_files, 0);
        assert_eq!(snap.parsed_files, 0);
        assert_eq!(snap.entities_ingested, 0);
        assert_eq!(snap.batches_ingested, 0);
        assert_eq!(snap.total_entities, 0);
        assert!(snap.error.is_none());
    }

    #[test]
    fn test_concurrent_increments() {
        let t = std::sync::Arc::new(ProgressTracker::new());
        let threads: Vec<_> = (0..8)
            .map(|_| {
                let t = std::sync::Arc::clone(&t);
                std::thread::spawn(move || {
                    for _ in 0..1000 {
                        t.incr_parsed_files();
                    }
                })
            })
            .collect();
        for th in threads {
            th.join().unwrap();
        }
        assert_eq!(t.snapshot().parsed_files, 8000);
    }

    #[test]
    fn test_record_batch_accumulates() {
        let t = ProgressTracker::new();
        t.record_batch_ingested(64);
        t.record_batch_ingested(64);
        t.record_batch_ingested(10);
        let snap = t.snapshot();
        assert_eq!(snap.batches_ingested, 3);
        assert_eq!(snap.entities_ingested, 138);
    }

    #[test]
    fn test_serialize_snapshot_json() {
        let t = ProgressTracker::new();
        t.begin_run("my-repo");
        t.set_stage(IndexingStage::Ingesting);
        let snap = t.snapshot();
        let val = serde_json::to_value(snap).unwrap();
        assert_eq!(val["stage"], "ingesting");
        assert!(val["repo_name"].is_string());
        assert!(val["total_files"].is_number());
        assert!(val["parsed_files"].is_number());
        assert!(val["percent_complete"].is_number());
        assert!(val["total_entities"].is_number());
        assert_eq!(val["error"], serde_json::Value::Null);
    }

    #[test]
    fn test_percent_clamped() {
        let t = ProgressTracker::new();
        t.set_total_files(100);
        t.set_stage(IndexingStage::Parsing);
        for _ in 0..200 {
            t.incr_parsed_files();
        }
        let snap = t.snapshot();
        // Over-counting parsing must NOT force 100% (that was the v1.6.1 bug);
        // the parse band saturates at 10% and ingestion is still pending.
        assert_eq!(snap.percent_complete, 10.0);
    }

    // ---- v1.6.2: tracker contract for `total_entities` ----

    #[test]
    fn given_new_tracker_when_snapshot_then_total_entities_is_zero_and_unknown() {
        let t = ProgressTracker::new();
        let snap = t.snapshot();
        assert_eq!(snap.total_entities, 0);
    }

    #[test]
    fn given_parse_finished_when_total_entities_recorded_then_snapshot_exposes_it() {
        let t = ProgressTracker::new();
        t.set_total_entities(83_456);
        let snap = t.snapshot();
        assert_eq!(snap.total_entities, 83_456);
    }

    #[test]
    fn given_a_populated_tracker_when_begin_run_then_total_entities_is_reset() {
        let t = ProgressTracker::new();
        t.begin_run("repo1");
        t.set_total_entities(500);
        t.begin_run("repo2");
        let snap = t.snapshot();
        assert_eq!(snap.total_entities, 0);
    }

    #[test]
    fn given_zero_value_when_set_total_entities_then_zero_is_recorded_as_a_real_total() {
        let t = ProgressTracker::new();
        t.set_total_entities(0);
        let snap = t.snapshot();
        assert_eq!(snap.total_entities, 0);
    }

    // ---- v1.6.2: banded percentage formula ----
    //
    // The percentage now spans the entire pipeline:
    //   parse band     → 0%  .. 10%   (parsed_files / total_files)
    //   ingest band    → 10% .. 90%   (entities_ingested / total_entities)
    //   resolving      → 95%         (fixed; no sub-counters available)
    //   completed      → 100%        (forced)
    //
    // Constants kept here mirror those in `compute_percent`; if they drift
    // these tests will fail and surface the divergence.

    #[test]
    fn given_half_the_files_parsed_when_snapshot_then_percent_is_half_the_parse_band() {
        let t = ProgressTracker::new();
        t.set_total_files(5000);
        t.set_stage(IndexingStage::Parsing);
        for _ in 0..2500 {
            t.incr_parsed_files();
        }
        let snap = t.snapshot();
        assert_eq!(snap.percent_complete, 5.0);
    }

    #[test]
    fn given_all_files_parsed_but_nothing_ingested_when_snapshot_then_percent_stays_in_parse_band()
    {
        // Regression test for the v1.6.1 bug: with 100% files read, the
        // percentage must NOT jump to 100% — we have not embedded/ingested
        // anything yet. It must sit at the parse band ceiling (10%).
        let t = ProgressTracker::new();
        t.set_total_files(3713);
        t.set_stage(IndexingStage::Parsing);
        for _ in 0..3713 {
            t.incr_parsed_files();
        }
        let snap = t.snapshot();
        assert_eq!(snap.percent_complete, 10.0);
    }

    #[test]
    fn given_half_the_entities_ingested_when_snapshot_then_percent_is_mid_ingest_band() {
        let t = ProgressTracker::new();
        t.set_total_files(3713);
        for _ in 0..3713 {
            t.incr_parsed_files();
        }
        t.set_total_entities(1000);
        t.set_stage(IndexingStage::Ingesting);
        t.record_batch_ingested(500);
        let snap = t.snapshot();
        // 10 + 0.5 * (90 - 10) = 50.0
        assert_eq!(snap.percent_complete, 50.0);
    }

    #[test]
    fn given_all_entities_ingested_when_snapshot_then_percent_is_ingest_band_ceiling() {
        let t = ProgressTracker::new();
        t.set_total_files(3713);
        for _ in 0..3713 {
            t.incr_parsed_files();
        }
        t.set_total_entities(1000);
        t.set_stage(IndexingStage::Ingesting);
        t.record_batch_ingested(1000);
        let snap = t.snapshot();
        assert_eq!(snap.percent_complete, 90.0);
    }

    #[test]
    fn given_zero_entities_parsed_when_snapshot_then_percent_jumps_to_ingest_band_ceiling() {
        // A repo that parses to zero entities (empty / unsupported-only)
        // is a legitimate state: `0` cannot mean "unknown", only "known zero".
        // The ingest band must collapse to its ceiling instantly.
        let t = ProgressTracker::new();
        t.set_total_files(10);
        for _ in 0..10 {
            t.incr_parsed_files();
        }
        t.set_total_entities(0);
        t.set_stage(IndexingStage::Ingesting);
        let snap = t.snapshot();
        assert_eq!(snap.percent_complete, 90.0);
    }

    #[test]
    fn given_more_entities_ingested_than_expected_when_snapshot_then_percent_is_clamped() {
        let t = ProgressTracker::new();
        t.set_total_files(100);
        for _ in 0..100 {
            t.incr_parsed_files();
        }
        t.set_total_entities(100);
        t.set_stage(IndexingStage::Ingesting);
        t.record_batch_ingested(150); // over-count
        let snap = t.snapshot();
        assert_eq!(snap.percent_complete, 90.0);
    }

    #[test]
    fn given_resolving_references_stage_when_snapshot_then_percent_is_resolving_constant() {
        let t = ProgressTracker::new();
        t.set_total_files(3713);
        for _ in 0..3713 {
            t.incr_parsed_files();
        }
        t.set_total_entities(83_456);
        t.set_stage(IndexingStage::ResolvingReferences);
        let snap = t.snapshot();
        assert_eq!(snap.percent_complete, 95.0);
    }

    #[test]
    fn given_failed_stage_when_snapshot_then_percent_reflects_work_done_so_far() {
        // On failure the bar must NOT snap to 0% or 100%; it freezes at
        // whatever value the work-in-progress has produced.
        let t = ProgressTracker::new();
        t.set_total_files(3713);
        for _ in 0..1856 {
            t.incr_parsed_files();
        }
        t.set_total_entities(83_456);
        t.set_stage(IndexingStage::Ingesting);
        t.record_batch_ingested(20_000);
        t.fail("boom");
        let snap = t.snapshot();
        assert_eq!(snap.stage, IndexingStage::Failed);
        // parsed band alone: 1856/3713 * 10 ≈ 5.0
        // ingest band: 10 + (20000/83456) * 80 ≈ 10 + 19.17 ≈ 29.17
        // Total ≈ 34.17 — well below 100 and well above 0.
        assert!(snap.percent_complete > 0.0);
        assert!(snap.percent_complete < 100.0);
    }

    // ---- v1.6.2: monotonicity property (the user-facing guarantee) ----

    #[test]
    fn given_a_full_pipeline_sequence_when_progressing_then_percent_never_decreases() {
        // Drive the tracker through the real sequence the runner follows,
        // snapshotting at every transition. The percentage must be
        // monotonically non-decreasing across the run and reach 100% only
        // once `complete()` is called.
        let t = ProgressTracker::new();
        let mut pct: Vec<f32> = Vec::new();

        // begin_run → Discovering
        t.begin_run("opencode");
        pct.push(t.snapshot().percent_complete);

        // set_total_files → still 0%
        t.set_total_files(3713);
        pct.push(t.snapshot().percent_complete);

        // Parsing — sample every 500 files so we don't inflate the test
        // runtime; monotonicity is a property of the formula, not of
        // density of samples.
        t.set_stage(IndexingStage::Parsing);
        let mut parsed = 0u64;
        while parsed < 3713 {
            for _ in 0..500 {
                if parsed < 3713 {
                    t.incr_parsed_files();
                    parsed += 1;
                }
            }
            pct.push(t.snapshot().percent_complete);
        }

        // Parse band must have saturated BELOW the ingest band ceiling
        // (this is the v1.6.1 bug regression).
        let parse_band_peak = *pct.last().unwrap();
        assert!(
            parse_band_peak < 90.0,
            "parse band must not exceed ingest ceiling, got {parse_band_peak}"
        );

        // Publish the entity total and switch to Ingesting
        let total_entities = 83_456u64;
        let batch_size = 128u64;
        let batches = total_entities / batch_size; // 652 batches
        t.set_total_entities(total_entities);
        t.set_stage(IndexingStage::Ingesting);

        let mut ingested = 0u64;
        for _ in 0..batches {
            let n = batch_size.min(total_entities - ingested);
            t.record_batch_ingested(n);
            ingested += n;
            pct.push(t.snapshot().percent_complete);
        }

        // ResolvingReferences — fixed jump to RESOLVING_PERCENT.
        t.set_stage(IndexingStage::ResolvingReferences);
        pct.push(t.snapshot().percent_complete);

        // Completion — final jump to 100.
        t.complete();
        pct.push(t.snapshot().percent_complete);

        // Property under test: pct[i] >= pct[i-1] for every consecutive pair.
        for w in pct.windows(2) {
            assert!(
                w[1] >= w[0],
                "progress decreased: {} -> {} (full series: {:?})",
                w[0],
                w[1],
                pct
            );
        }

        // And the bar lands at exactly 100% on completion.
        assert_eq!(pct.last().copied(), Some(100.0));
    }
}
