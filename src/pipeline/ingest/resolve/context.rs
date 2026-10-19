use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use uuid::Uuid;

use crate::models::EntityKind;

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
    pub uuid_to_kind: Option<&'a HashMap<Uuid, EntityKind>>,
    pub uuid_to_name: Option<&'a HashMap<Uuid, String>>,
    /// `"Class.member"` (last two FQN segments) → every `(fqn, uuid)` whose
    /// FQN ends that way. Turns the "method of a type named `Class`" lookup
    /// into a hash probe instead of a scan over every FQN.
    pub class_member_index: &'a HashMap<String, Vec<(String, Uuid)>>,
}

/// Index key for an FQN: its last two segments joined by `.`, whatever the
/// separator (`pkg.Class.member`, `crate::Type::member`). `None` for FQNs
/// with fewer than two segments.
pub(crate) fn class_member_key(fqn: &str) -> Option<String> {
    let mut segments = fqn.rsplit(['.', ':']).filter(|s| !s.is_empty());
    let member = segments.next()?;
    let class = segments.next()?;
    Some(format!("{class}.{member}"))
}

/// Build the `"Class.member"` index over a FQN map.
pub(crate) fn build_class_member_index(
    fqn_to_uuid: &HashMap<String, Uuid>,
) -> HashMap<String, Vec<(String, Uuid)>> {
    let mut index: HashMap<String, Vec<(String, Uuid)>> = HashMap::new();
    for (fqn, uuid) in fqn_to_uuid {
        if let Some(key) = class_member_key(fqn) {
            index.entry(key).or_default().push((fqn.clone(), *uuid));
        }
    }
    index
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_class_member_key_handles_both_separators() {
        assert_eq!(
            class_member_key("com.x.CatalogProvider.fetchItems").as_deref(),
            Some("CatalogProvider.fetchItems")
        );
        assert_eq!(
            class_member_key("knot::db::Graph::connect").as_deref(),
            Some("Graph.connect")
        );
        assert_eq!(class_member_key("main"), None);
    }

    #[test]
    fn test_build_class_member_index_groups_homonyms() {
        let a = Uuid::new_v4();
        let b = Uuid::new_v4();
        let c = Uuid::new_v4();
        let fqns: HashMap<String, Uuid> = [
            ("pkg.alpha.Widget.render".to_string(), a),
            ("pkg.beta.Widget.render".to_string(), b),
            ("pkg.HttpWidget.render".to_string(), c),
        ]
        .into_iter()
        .collect();
        let index = build_class_member_index(&fqns);
        let mut widget: Vec<Uuid> = index["Widget.render"].iter().map(|(_, u)| *u).collect();
        widget.sort();
        let mut expected = vec![a, b];
        expected.sort();
        assert_eq!(widget, expected);
        assert_eq!(index["HttpWidget.render"].len(), 1);
    }

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
