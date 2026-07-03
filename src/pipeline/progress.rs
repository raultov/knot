use serde::Serialize;
use std::sync::RwLock;
use std::sync::atomic::{AtomicU64, Ordering};

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
        }
    }

    pub fn snapshot(&self) -> IndexingProgress {
        let stage = *self.stage.read().unwrap();
        let total = self.total_files.load(Ordering::Relaxed);
        let parsed = self.parsed_files.load(Ordering::Relaxed);
        let entities = self.entities_ingested.load(Ordering::Relaxed);
        let batches = self.batches_ingested.load(Ordering::Relaxed);

        let pct = if stage == IndexingStage::Completed {
            100.0
        } else if total == 0 {
            0.0
        } else {
            let raw = (parsed as f32 / total as f32) * 100.0;
            raw.clamp(0.0, 100.0)
        };

        IndexingProgress {
            repo_name: self.repo_name.read().unwrap().clone(),
            stage,
            total_files: total,
            parsed_files: parsed,
            percent_complete: pct,
            entities_ingested: entities,
            batches_ingested: batches,
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
        assert_eq!(snap.percent_complete, 20.0);
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
        t.fail("oops");
        t.begin_run("repo2");
        let snap = t.snapshot();
        assert_eq!(snap.repo_name, "repo2");
        assert_eq!(snap.stage, IndexingStage::Discovering);
        assert_eq!(snap.total_files, 0);
        assert_eq!(snap.parsed_files, 0);
        assert_eq!(snap.entities_ingested, 0);
        assert_eq!(snap.batches_ingested, 0);
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
        assert_eq!(snap.percent_complete, 100.0);
    }
}
