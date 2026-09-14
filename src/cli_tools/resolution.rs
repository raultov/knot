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

use crate::cli_tools::format_file_line;

/// Borrowed view over the `resolution` object of a `find_references` result.
pub(crate) struct ResolutionView<'a> {
    query: &'a str,
    tier: &'a str,
    targets: &'a [Value],
    fuzzy: bool,
    truncated: bool,
    total_targets: i64,
    kind_filter: &'a str,
    hidden_kinds: Vec<&'a str>,
    hidden_non_code: i64,
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
            // Older indexes/Knot-server responses predate the kind-filter
            // disclosure; absent fields degrade to "nothing was hidden".
            kind_filter: resolution
                .get("kind_filter")
                .and_then(Value::as_str)
                .unwrap_or("code_default"),
            hidden_kinds: resolution
                .get("hidden_kinds")
                .and_then(Value::as_array)
                .map(|a| a.iter().filter_map(Value::as_str).collect())
                .unwrap_or_default(),
            hidden_non_code: resolution
                .get("hidden_non_code")
                .and_then(Value::as_i64)
                .unwrap_or(0),
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

    /// How many matched entities the kind filter removed (`0` when the
    /// resolution block predates the disclosure or nothing was hidden).
    pub(crate) fn hidden_non_code(&self) -> i64 {
        self.hidden_non_code
    }

    /// Stable `resolution.kind_filter` label (`code_default` / `any` /
    /// `explicit`).
    pub(crate) fn kind_filter(&self) -> &'a str {
        self.kind_filter
    }

    /// Shared wording for the hidden-matches disclosure. Empty when there is
    /// nothing to disclose, so callers can unconditionally append the return
    /// value. `markdown` toggles the styling: `**emphasis**` and backticked
    /// option names for the Markdown answer, bare text for the CLI table.
    /// A single parameterized helper so the two renderers cannot drift
    /// (extracted after `cargo dupes` flagged the copy pair).
    pub(crate) fn hidden_notice(&self, markdown: bool) -> String {
        if self.hidden_non_code <= 0 {
            return String::new();
        }
        let entity_word = if self.hidden_non_code == 1 {
            "entity"
        } else {
            "entities"
        };
        let kinds = if self.hidden_kinds.is_empty() {
            "non-code kinds".to_string()
        } else if self.hidden_kinds.len() == 1 {
            format!("kind {}", self.hidden_kinds[0])
        } else {
            format!("kinds {}", self.hidden_kinds.join(", "))
        };
        if markdown {
            format!(
                "**Non-code matches hidden** — {n} {entity_word} matched `{q}` but are \
                 documentation/config/build metadata ({kinds}), not code definitions. \
                 Pass `kinds=all` to include them, or a specific kind \
                 (e.g. `kinds=build_dependency`) to narrow explicitly.",
                n = self.hidden_non_code,
                q = self.query,
            )
        } else {
            format!(
                "Non-code matches hidden — {n} {entity_word} matched `{q}` but are \
                 documentation/config/build metadata ({kinds}), not code definitions. \
                 Pass kinds=all to include them, or a specific kind \
                 (e.g. kinds=build_dependency) to narrow explicitly.",
                n = self.hidden_non_code,
                q = self.query,
            )
        }
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

    /// Caveat stating that the relationship-bucket counts cover only the
    /// shown targets. Empty when the target list is complete, so callers can
    /// unconditionally append the return value:
    ///
    /// ```text
    /// Counts below are partial — they cover only the 25 of 112 targets shown.
    /// ```
    pub(crate) fn partial_counts_caveat(&self) -> String {
        if !self.truncated {
            return String::new();
        }
        format!(
            "Counts below are partial — they cover only the {} of {} targets shown.",
            self.count(),
            self.total_targets()
        )
    }

    /// One `- \`fqn\` (kind) at \`file:line\`  (repo: name)` bullet per resolved target.
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
            let repo = target.get("repo_name").and_then(Value::as_str);
            out.push_str(&format!(
                "- `{}` ({}) at {}\n",
                fqn,
                kind,
                format_file_line(&format!("{}:{}", file_path, start_line), repo)
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
    fn target_bullets_include_repo_annotation() {
        let refs = json!({
            "resolution": {
                "tier": "exact_name",
                "targets": [{
                    "fqn": "Ns.GestureOwner.Off",
                    "kind": "csharp_record",
                    "file_path": "src/GestureOwner.cs",
                    "start_line": 15,
                    "repo_name": "scope_alpha"
                }]
            }
        });
        let view = ResolutionView::from_references(&refs).expect("resolution");
        assert_eq!(
            view.target_bullets(),
            "- `Ns.GestureOwner.Off` (csharp_record) at `src/GestureOwner.cs:15`  (repo: scope_alpha)\n"
        );
    }

    #[test]
    fn target_bullets_omit_annotation_when_repo_empty() {
        let refs = json!({
            "resolution": {
                "tier": "exact_name",
                "targets": [{
                    "fqn": "Ns.Off",
                    "kind": "method",
                    "file_path": "src/Off.cs",
                    "start_line": 3,
                    "repo_name": ""
                }]
            }
        });
        let view = ResolutionView::from_references(&refs).expect("resolution");
        assert_eq!(
            view.target_bullets(),
            "- `Ns.Off` (method) at `src/Off.cs:3`\n"
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

    // ---- kind-filter disclosure ----

    fn hidden_resolution(n: i64, kinds: &[&str]) -> Value {
        json!({
            "resolution": {
                "query": "cargo",
                "tier": "fuzzy",
                "truncated": false,
                "total_targets": 0,
                "kind_filter": "code_default",
                "hidden_non_code": n,
                "hidden_kinds": kinds,
                "targets": []
            }
        })
    }

    #[test]
    fn resolution_view_defaults_hidden_fields_when_absent() {
        // Pre-fix wire shape: no kind-filter fields at all.
        let refs = sample();
        let view = ResolutionView::from_references(&refs).expect("resolution");
        assert_eq!(view.hidden_non_code(), 0);
        assert_eq!(view.kind_filter(), "code_default");
        assert_eq!(view.hidden_notice(true), "");
        assert_eq!(view.hidden_notice(false), "");
    }

    #[test]
    fn hidden_notice_names_the_kinds_and_how_to_opt_in() {
        let refs = hidden_resolution(93, &["build_dependency", "cargo_package"]);
        let view = ResolutionView::from_references(&refs).expect("resolution");
        let markdown = view.hidden_notice(true);
        assert!(markdown.contains("**Non-code matches hidden** — 93 entities matched `cargo`"));
        assert!(markdown.contains("build_dependency, cargo_package"));
        assert!(markdown.contains("kinds=all"));
        assert!(!markdown.contains("may be unused"));

        let plain = view.hidden_notice(false);
        assert!(plain.contains("Non-code matches hidden — 93"));
        assert!(!plain.contains("**"));
    }

    #[test]
    fn hidden_notice_singularizes_and_handles_empty_kinds() {
        let refs = hidden_resolution(1, &[]);
        let view = ResolutionView::from_references(&refs).expect("res");
        assert!(view.hidden_notice(true).contains("1 entity matched"));
        assert!(view.hidden_notice(true).contains("(non-code kinds)"));

        let refs = hidden_resolution(2, &["md_section"]);
        let view = ResolutionView::from_references(&refs).expect("res");
        assert!(view.hidden_notice(true).contains("2 entities matched"));
        assert!(view.hidden_notice(true).contains("kind md_section"));
    }
}
