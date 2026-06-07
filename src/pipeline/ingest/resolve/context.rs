use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use uuid::Uuid;

pub struct RunMetrics {
    pub entities_indexed: AtomicU64,
    pub references_resolved: AtomicU64,
    pub references_ambiguous_skipped: AtomicU64,
    pub references_unresolved: AtomicU64,
}

impl RunMetrics {
    pub fn new(entities_indexed: u64) -> Self {
        Self {
            entities_indexed: AtomicU64::new(entities_indexed),
            references_resolved: AtomicU64::new(0),
            references_ambiguous_skipped: AtomicU64::new(0),
            references_unresolved: AtomicU64::new(0),
        }
    }

    pub fn accumulate(&self, other: &RunMetrics) {
        self.entities_indexed.fetch_add(
            other.entities_indexed.load(Ordering::Relaxed),
            Ordering::Relaxed,
        );
        self.references_resolved.fetch_add(
            other.references_resolved.load(Ordering::Relaxed),
            Ordering::Relaxed,
        );
        self.references_ambiguous_skipped.fetch_add(
            other.references_ambiguous_skipped.load(Ordering::Relaxed),
            Ordering::Relaxed,
        );
        self.references_unresolved.fetch_add(
            other.references_unresolved.load(Ordering::Relaxed),
            Ordering::Relaxed,
        );
    }
}

pub struct ResolutionContext<'a> {
    pub fqn_to_uuid: &'a HashMap<String, Uuid>,
    pub name_to_uuids: &'a HashMap<String, Vec<Uuid>>,
    pub uuid_to_file: &'a HashMap<Uuid, String>,
    pub extends_map: &'a HashMap<String, Vec<String>>,
    pub uuid_to_arg_count: Option<&'a HashMap<Uuid, usize>>,
    pub uuid_to_fqn: Option<&'a HashMap<Uuid, String>>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ambiguity_metric_increments() {
        let metrics = RunMetrics::new(3);
        metrics
            .references_ambiguous_skipped
            .fetch_add(1, Ordering::Relaxed);
        assert_eq!(
            metrics.references_ambiguous_skipped.load(Ordering::Relaxed),
            1
        );
    }
}
