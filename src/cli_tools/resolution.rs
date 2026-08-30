//! Shared rendering helpers for the `resolution` block returned by
//! `find_references`.
//!
//! The graph query resolves an ambiguous entity name to one or more concrete
//! targets and reports how it did so (the match tier), whether the match was
//! fuzzy, and whether the target list was truncated. Both the Markdown
//! formatter (`cli_tools::find_callers`) and the table formatter
//! (`cli_tools::formatters::table`) render that same block, so the parsing and
//! the wording live here to keep them in sync.

use serde_json::Value;

/// Borrowed view over the `resolution` object of a `find_references` result.
pub(crate) struct ResolutionView<'a> {
    query: &'a str,
    tier: &'a str,
    targets: &'a [Value],
    fuzzy: bool,
    truncated: bool,
    total_targets: i64,
}

impl<'a> ResolutionView<'a> {
    /// Parse the `resolution` block out of a `find_references` result.
    ///
    /// Returns `None` when the key is absent or does not carry both a `tier`
    /// and a `targets` array — callers then fall back to their plain header.
    pub(crate) fn from_references(references: &'a Value) -> Option<Self> {
        let resolution = references.get("resolution")?;
        let tier = resolution.get("tier").and_then(Value::as_str)?;
        let targets = resolution.get("targets").and_then(Value::as_array)?;
        let count = targets.len() as i64;

        Some(Self {
            query: resolution
                .get("query")
                .and_then(Value::as_str)
                .unwrap_or_default(),
            tier,
            targets,
            fuzzy: resolution
                .get("fuzzy")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            truncated: resolution
                .get("truncated")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            total_targets: resolution
                .get("total_targets")
                .and_then(Value::as_i64)
                .unwrap_or(count),
        })
    }

    pub(crate) fn query(&self) -> &'a str {
        self.query
    }

    pub(crate) fn count(&self) -> usize {
        self.targets.len()
    }

    pub(crate) fn is_fuzzy(&self) -> bool {
        self.fuzzy
    }

    pub(crate) fn is_truncated(&self) -> bool {
        self.truncated
    }

    pub(crate) fn total_targets(&self) -> i64 {
        self.total_targets
    }

    /// Human-readable label for the match tier.
    pub(crate) fn tier_label(&self) -> &'a str {
        match self.tier {
            "exact_fqn" => "exact FQN match",
            "fqn_suffix" => "FQN suffix match",
            "exact_name" => "exact name match",
            "signature_prefix" => "signature prefix match",
            "fuzzy" => "fuzzy match",
            other => other,
        }
    }

    /// `"Resolved to 1 target by exact name match"` (no trailing punctuation).
    pub(crate) fn summary(&self) -> String {
        let count = self.count();
        let target_word = if count == 1 { "target" } else { "targets" };
        format!(
            "Resolved to {} {} by {}",
            count,
            target_word,
            self.tier_label()
        )
    }

    /// One `- \`fqn\` (kind) at \`file:line\`` bullet per resolved target.
    pub(crate) fn target_bullets(&self) -> String {
        let mut out = String::new();
        for target in self.targets {
            let fqn = target.get("fqn").and_then(Value::as_str).unwrap_or("");
            let kind = target.get("kind").and_then(Value::as_str).unwrap_or("");
            let file_path = target
                .get("file_path")
                .and_then(Value::as_str)
                .unwrap_or("");
            let start_line = target
                .get("start_line")
                .and_then(Value::as_i64)
                .unwrap_or(0);
            out.push_str(&format!(
                "- `{}` ({}) at `{}:{}`\n",
                fqn, kind, file_path, start_line
            ));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sample() -> Value {
        json!({
            "resolution": {
                "query": "Off",
                "tier": "exact_name",
                "fuzzy": false,
                "truncated": false,
                "targets": [
                    {
                        "fqn": "Ns.GestureOwner.Off",
                        "kind": "csharp_record",
                        "file_path": "src/GestureOwner.cs",
                        "start_line": 15
                    }
                ]
            }
        })
    }

    #[test]
    fn test_missing_resolution_returns_none() {
        assert!(ResolutionView::from_references(&json!({"calls": []})).is_none());
    }

    #[test]
    fn test_incomplete_resolution_returns_none() {
        let refs = json!({"resolution": {"tier": "exact_name"}});
        assert!(ResolutionView::from_references(&refs).is_none());
    }

    #[test]
    fn test_summary_singular() {
        let refs = sample();
        let view = ResolutionView::from_references(&refs).expect("resolution");
        assert_eq!(view.summary(), "Resolved to 1 target by exact name match");
    }

    #[test]
    fn test_summary_plural() {
        let refs = json!({
            "resolution": {
                "tier": "fqn_suffix",
                "targets": [{"fqn": "A"}, {"fqn": "B"}]
            }
        });
        let view = ResolutionView::from_references(&refs).expect("resolution");
        assert_eq!(view.summary(), "Resolved to 2 targets by FQN suffix match");
    }

    #[test]
    fn test_unknown_tier_passes_through() {
        let refs = json!({"resolution": {"tier": "brand_new", "targets": []}});
        let view = ResolutionView::from_references(&refs).expect("resolution");
        assert_eq!(view.tier_label(), "brand_new");
    }

    #[test]
    fn test_target_bullets() {
        let refs = sample();
        let view = ResolutionView::from_references(&refs).expect("resolution");
        assert_eq!(
            view.target_bullets(),
            "- `Ns.GestureOwner.Off` (csharp_record) at `src/GestureOwner.cs:15`\n"
        );
    }

    #[test]
    fn test_total_targets_defaults_to_target_count() {
        let refs = sample();
        let view = ResolutionView::from_references(&refs).expect("resolution");
        assert_eq!(view.total_targets(), 1);
        assert!(!view.is_truncated());
        assert!(!view.is_fuzzy());
        assert_eq!(view.query(), "Off");
    }

    #[test]
    fn test_total_targets_uses_explicit_value() {
        let refs = json!({
            "resolution": {
                "tier": "exact_name",
                "truncated": true,
                "total_targets": 112,
                "targets": [{"fqn": "A"}]
            }
        });
        let view = ResolutionView::from_references(&refs).expect("resolution");
        assert!(view.is_truncated());
        assert_eq!(view.total_targets(), 112);
    }
}
