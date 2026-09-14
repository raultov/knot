//! Core find_callers logic shared between CLI and MCP
//!
//! Performs comprehensive reverse dependency lookup: given an entity name,
//! finds all other entities that reference it through any relationship type
//! (CALLS, EXTENDS, IMPLEMENTS, REFERENCES).

use std::sync::Arc;

use crate::db::graph::{DEFAULT_MAX_TARGETS, GraphDb, QueryExt};

use crate::models::RepoScope;

use crate::cli_tools::json_target_name;

use crate::cli_tools::append_signature_if_present;
use crate::cli_tools::format_file_line;
use crate::cli_tools::resolution::ResolutionView;

/// Main find_callers function called by both CLI and MCP
///
/// `max_targets` raises the resolution cap (default: [`DEFAULT_MAX_TARGETS`],
/// hard ceiling: 500). `None` keeps the default — an explicit `Some(n)` is how
/// callers opt in to the full impact set when the response reports a
/// truncated target list.
///
/// `kinds` scopes the entity kinds the query may resolve against
/// ([`crate::cli_tools::kinds::KindFilter`]): `None` keeps the default
/// code-only scope (documentation/config/build metadata can never be
/// presented as resolved targets), `all` disables filtering, and any other
/// spec is an explicit alias/exact-kind list.
pub async fn run_find_callers(
    entity_name: &str,
    repo: &RepoScope,
    graph_db: &Arc<GraphDb>,
    max_targets: Option<usize>,
    kinds: Option<&str>,
) -> anyhow::Result<serde_json::Value> {
    let repo_names = repo.filter_names();
    let max_targets = max_targets.unwrap_or(DEFAULT_MAX_TARGETS);
    let references = graph_db
        .find_references(entity_name, &repo_names, max_targets, kinds)
        .await?;
    Ok(references)
}

pub fn format_references_result(entity_name: &str, references: &serde_json::Value) -> String {
    let mut output = format!("# References to `{}`\n\n", entity_name);
    output.push_str(&format_resolution_markdown(references));

    // Every relationship type the pipeline produces (see
    // `find_references_rel_labels`) plus the two OVERRIDES projections.
    // Empty buckets are skipped by the loop below, so repositories without
    // web/VCL/macro edges render exactly as before.
    let rel_types = [
        ("calls", "Calls (function/method invocations)"),
        ("extends", "Extends (class inheritance)"),
        ("implements", "Implements (interface implementation)"),
        ("references", "References (type annotations/usages)"),
        ("macro_calls", "Macro calls (macro invocations)"),
        ("references_dom", "DOM references (JS → HTML element id)"),
        ("uses_css_class", "CSS class usage (JS → CSS class)"),
        ("imports_script", "Script imports (HTML → JS file)"),
        ("imports_stylesheet", "Stylesheet imports (HTML → CSS file)"),
        ("uses_backend", "Backend usage (VCL)"),
        ("uses_probe", "Probe usage (VCL)"),
        ("uses_acl", "ACL usage (VCL)"),
        ("includes", "File includes (VCL)"),
        ("imports_vmod", "VMOD imports (VCL)"),
        ("declared_unused", "Declared unused (VCL)"),
        ("overridden_by", "Overridden by (method implementations)"),
        ("overrides", "Overrides (declared supertype methods)"),
    ];

    let total_refs: usize = rel_types
        .iter()
        .filter_map(|(key, _)| references.get(key).and_then(|v| v.as_array()))
        .map(|arr| arr.len())
        .sum();

    if total_refs == 0 {
        // Three honest outcomes instead of a blanket "may be unused":
        //   targets > 0              — entity exists in scope; unused so far.
        //   targets = 0, hidden > 0  — only filtered metadata matched; say so.
        //   targets = 0, hidden = 0  — the name matched nothing at all.
        // A missing/unparseable `resolution` key (old wire shape) cannot
        // distinguish these, so it keeps the legacy wording unchanged.
        match ResolutionView::from_references(references) {
            // A truncated resolution may show zero targets while the true
            // total is large; the name still matched something in scope.
            Some(view) if view.count() > 0 || view.total_targets() > 0 => {
                output.push_str(&format!(
                    "No references found for `{}`. This entity may be unused.\n",
                    entity_name
                ))
            }
            Some(view) if view.hidden_non_code() > 0 => output.push_str(&format!(
                "No code entity matched `{}` — see the disclosure above; the name only \
                 hit documentation/config/build metadata.\n",
                entity_name
            )),
            Some(_) => output.push_str(&format!(
                "No entity named `{}` was found in the indexed scope. \
                 Check the spelling, the repository scope (`--repo`), or search \
                 semantically with `search_hybrid_context` first.\n",
                entity_name
            )),
            None => output.push_str(&format!(
                "No references found for `{}`. This entity may be unused.\n",
                entity_name
            )),
        }
        return output;
    }

    output.push_str(&format!(
        "Found {} reference(s) across all relationship types:\n\n",
        total_refs
    ));

    // Whether the query resolved to more than one candidate. Homonym
    // attribution must be driven by the *resolution*, not by how many
    // targets happened to accumulate references: when 2 targets resolve
    // and only 1 has callers, the reader still needs to know which one.
    let mut multiple_targets = false;
    if let Some(view) = ResolutionView::from_references(references) {
        multiple_targets = view.count() > 1 || view.total_targets() > 1;

        // When the target resolution was truncated, the per-bucket counts
        // cover only the shown targets. State that explicitly (with the real
        // totals) so a sample can never be mistaken for the complete set.
        let caveat = view.partial_counts_caveat();
        if !caveat.is_empty() {
            output.push_str(&format!("> **{}**\n", caveat));
            output.push_str("> Re-run with a fully qualified name, or raise `max_targets`, for the complete set.\n\n");
        }
    }

    for (key, label) in rel_types {
        if let Some(arr) = references.get(key).and_then(|v| v.as_array())
            && !arr.is_empty()
        {
            output.push_str(&format!("## {} ({})\n\n", label, arr.len()));
            output.push_str(&format_relationship_bucket(
                entity_name,
                arr,
                multiple_targets,
            ));
        }
    }

    output
}

/// Render the `resolution` block (which targets the query resolved to, plus
/// fuzzy/truncation caveats) as Markdown. Empty when no block is present.
fn format_resolution_markdown(references: &serde_json::Value) -> String {
    let Some(view) = ResolutionView::from_references(references) else {
        return String::new();
    };

    let mut output = format!("{}:\n", view.summary());
    output.push_str(&view.target_bullets());
    output.push('\n');

    if view.is_fuzzy() {
        output.push_str(&format!(
            "> **Fuzzy match** — no entity matched `{}` exactly. The {} target(s) below were\n\
             > found by substring match and may be unrelated. Re-run with an exact name or a\n\
             > fully qualified name (e.g. `Namespace.Type.Member`) for precise results.\n\n",
            view.query(),
            view.count()
        ));
    }

    if view.is_truncated() {
        output.push_str(&format!(
            "> **Truncated** — {} targets matched; showing the first {} by FQN.\n\n",
            view.total_targets(),
            view.count()
        ));
    }

    let hidden = view.hidden_notice(true);
    if !hidden.is_empty() {
        // The Markdown variant already carries its own `**bold**` headline.
        output.push_str(&format!("> {hidden}\n\n"));
    }

    // When the caller overrode the default code-only scope, state it — an
    // empty-looking target list must be explainable by the reader.
    if view.count() == 0 && view.kind_filter() == "explicit" {
        output.push_str(
            "> Scope: an explicit `kinds` allow-list was applied; empty resolution means no \
             matching entity of those kinds exists in scope.\n\n",
        );
    }

    output
}

/// Render one relationship bucket, attributing each reference to the resolved
/// target it points at.
///
/// The `### Target:` header is emitted when the bucket spans more than one
/// target **or** when the query resolved to more than one target
/// (`multiple_targets`). The second condition is what keeps attribution alive
/// for homonyms where only one candidate has callers: grouping by bucket
/// content alone collapses to a single group and silently drops the header,
/// leaving the reader unable to tell which homonym is referenced.
///
/// Rows without target identity (`None` key) are never given a header — a
/// fabricated `### Target: <query> at unknown` would attribute by bare name,
/// which is precisely what the header exists to avoid. They sort first
/// (`None < Some` under `Ord`) and render directly under the bucket heading.
///
/// The grouping uses a `BTreeMap`, not a `HashMap`: iterating a `HashMap`
/// yields the `### Target:` sections in a process-random order (SipHash keys
/// are seeded per process), which made the rendered output non-reproducible
/// across runs and across nodes serving the same graph. Sorting by the group
/// key (target file path, then start line) keeps the output deterministic.
fn format_relationship_bucket(
    entity_name: &str,
    arr: &[serde_json::Value],
    multiple_targets: bool,
) -> String {
    use std::collections::BTreeMap;

    let mut grouped: BTreeMap<Option<String>, Vec<&serde_json::Value>> = BTreeMap::new();
    for entity in arr {
        grouped
            .entry(target_group_key(entity))
            .or_default()
            .push(entity);
    }

    let show_targets = grouped.len() > 1 || multiple_targets;
    let mut output = String::new();

    for (target_key, entities) in grouped {
        if let Some(target_key) = target_key.filter(|_| show_targets) {
            output.push_str(&format_target_header(entities[0], entity_name, &target_key));
        }
        for entity in entities {
            output.push_str(&format_reference_entry(entity));
        }
    }

    output
}

/// `### Target:` section header plus the target signature line.
fn format_target_header(
    first_entity: &serde_json::Value,
    entity_name: &str,
    target_key: &str,
) -> String {
    // Prefer target_fqn when available — qualified identifiers
    // disambiguate homonyms (e.g., `WidgetA::new` vs `WidgetB::new`).
    let target_name = json_target_name(first_entity, entity_name);
    let target_repo = first_entity
        .get("target_repo_name")
        .and_then(|v| v.as_str());

    let mut output = format!(
        "### Target: `{}` at {}\n\n",
        target_name,
        format_file_line(target_key, target_repo)
    );

    if let Some(target_sig) = first_entity
        .get("target_signature")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
    {
        output.push_str(&format!("Signature: `{}`\n\n", target_sig));
    }

    output
}

/// Grouping key identifying the target a reference points at.
///
/// `None` when the row carries no target identity — such a row cannot be
/// attributed to a homonym and must never be rendered under a fabricated
/// `### Target:` header.
fn target_group_key(entity: &serde_json::Value) -> Option<String> {
    let target_file = entity
        .get("target_file_path")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())?;

    Some(
        match entity.get("target_start_line").and_then(|v| v.as_i64()) {
            Some(target_line) => format!("{}:{}", target_file, target_line),
            None => target_file.to_string(),
        },
    )
}

pub fn format_reference_entry(entity: &serde_json::Value) -> String {
    let mut output = String::new();

    if let Some(name) = entity.get("name").and_then(|v| v.as_str()) {
        if let Some(kind) = entity.get("kind").and_then(|v| v.as_str()) {
            output.push_str(&format!("- **`{}`** ({})", name, kind));
        } else {
            output.push_str(&format!("- **`{}`**", name));
        }
    }

    // Path rendering goes through the shared `format_file_line` helper so the
    // `(repo: ...)` annotation is appended whenever the row carries a repo.
    let repo = entity.get("repo_name").and_then(|v| v.as_str());
    if let Some(file_path) = entity.get("file_path").and_then(|v| v.as_str()) {
        let path = match entity.get("start_line").and_then(|v| v.as_i64()) {
            Some(start_line) => format!("{}:{}", file_path, start_line),
            None => file_path.to_string(),
        };
        output.push_str(" at ");
        output.push_str(&format_file_line(&path, repo));
    }

    output.push('\n');

    append_signature_if_present(&mut output, entity);

    output.push('\n');
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_format_references_result_empty() {
        let references = json!({
            "calls": [],
            "extends": [],
            "implements": [],
            "references": []
        });
        let formatted = format_references_result("MyEntity", &references);
        assert!(formatted.contains("No references found"));
    }

    #[test]
    fn test_format_references_result_with_data() {
        let references = json!({
            "calls": [
                {
                    "name": "caller1",
                    "kind": "method",
                    "file_path": "file1.java",
                    "start_line": 10,
                    "signature": "void caller1()"
                }
            ],
            "extends": [],
            "implements": [],
            "references": []
        });
        let formatted = format_references_result("MyEntity", &references);
        assert!(formatted.contains("caller1"));
        assert!(formatted.contains("file1.java:10"));
    }

    #[test]
    fn test_format_references_result_with_multiple_relationship_types() {
        let references = json!({
            "calls": [{"name": "caller1", "kind": "method", "file_path": "file1.java", "start_line": 10}],
            "extends": [{"name": "ChildClass", "kind": "class", "file_path": "file2.java", "start_line": 20}],
            "implements": [{"name": "ImplClass", "kind": "class", "file_path": "file3.java", "start_line": 30}],
            "references": [{"name": "refUser", "kind": "method", "file_path": "file4.java", "start_line": 40}]
        });
        let formatted = format_references_result("MyEntity", &references);
        assert!(formatted.contains("Found 4 reference(s)"));
        assert!(formatted.contains("Calls (function/method invocations)"));
        assert!(formatted.contains("Extends (class inheritance)"));
        assert!(formatted.contains("Implements (interface implementation)"));
        assert!(formatted.contains("References (type annotations/usages)"));
    }

    #[test]
    fn test_format_reference_entry_complete() {
        let entity = json!({
            "name": "myMethod",
            "kind": "method",
            "file_path": "src/Handler.java",
            "start_line": 42,
            "signature": "public void myMethod() throws Exception"
        });
        let formatted = format_reference_entry(&entity);
        assert!(formatted.contains("myMethod"));
        assert!(formatted.contains("method"));
        assert!(formatted.contains("src/Handler.java:42"));
        assert!(formatted.contains("public void myMethod() throws Exception"));
    }

    #[test]
    fn test_format_reference_entry_without_line_number() {
        let entity = json!({
            "name": "myMethod",
            "kind": "method",
            "file_path": "src/Handler.java"
        });
        let formatted = format_reference_entry(&entity);
        assert!(formatted.contains("myMethod"));
        assert!(formatted.contains("src/Handler.java"));
        assert!(!formatted.contains(":"));
    }

    #[test]
    fn test_format_reference_entry_without_kind() {
        let entity = json!({
            "name": "UnknownEntity",
            "file_path": "src/Unknown.java",
            "start_line": 50
        });
        let formatted = format_reference_entry(&entity);
        assert!(formatted.contains("UnknownEntity"));
        assert!(formatted.contains("src/Unknown.java:50"));
    }

    // ---- §7.2 reference repo attribution renderers ----

    #[test]
    fn format_reference_entry_appends_repo_annotation() {
        let entity = json!({
            "name": "caller1",
            "kind": "method",
            "file_path": "file1.java",
            "start_line": 10,
            "repo_name": "alpha"
        });
        let formatted = format_reference_entry(&entity);
        assert!(formatted.contains("file1.java:10"), "got {formatted}");
        assert!(formatted.contains("(repo: alpha)"), "got {formatted}");
    }

    #[test]
    fn format_reference_entry_without_repo_omits_annotation() {
        let entity = json!({
            "name": "caller1",
            "kind": "method",
            "file_path": "file1.java",
            "start_line": 10
        });
        let formatted = format_reference_entry(&entity);
        assert!(!formatted.contains("(repo:"), "got {formatted}");
    }

    #[test]
    fn format_reference_entry_with_empty_repo_omits_annotation() {
        let entity = json!({
            "name": "caller1",
            "kind": "method",
            "file_path": "file1.java",
            "start_line": 10,
            "repo_name": ""
        });
        let formatted = format_reference_entry(&entity);
        assert!(!formatted.contains("(repo:"), "got {formatted}");
    }

    #[test]
    fn target_header_labels_target_repo() {
        let references = json!({
            "calls": [
                {
                    "name": "caller1", "kind": "function",
                    "file_path": "a.rs", "start_line": 1,
                    "target_name": "find_me",
                    "target_file_path": "orphans.rs", "target_start_line": 92,
                    "repo_name": "alpha", "target_repo_name": "beta"
                },
                {
                    "name": "caller2", "kind": "function",
                    "file_path": "b.rs", "start_line": 2,
                    "target_name": "find_me",
                    "target_file_path": "rust.rs", "target_start_line": 445,
                    "repo_name": "alpha", "target_repo_name": "gamma"
                }
            ],
            "extends": [],
            "implements": [],
            "references": []
        });
        let formatted = format_references_result("find_me", &references);
        assert!(formatted.contains("### Target:"));
        assert!(formatted.contains("(repo: beta)"), "got {formatted}");
        assert!(formatted.contains("(repo: gamma)"), "got {formatted}");
    }

    #[test]
    fn test_format_references_result_only_extends() {
        let references = json!({
            "calls": [],
            "extends": [
                {"name": "ChildClass1", "kind": "class", "file_path": "file1.java", "start_line": 10},
                {"name": "ChildClass2", "kind": "class", "file_path": "file2.java", "start_line": 20}
            ],
            "implements": [],
            "references": []
        });
        let formatted = format_references_result("BaseClass", &references);
        assert!(formatted.contains("Found 2 reference(s)"));
        assert!(formatted.contains("Extends (class inheritance) (2)"));
        assert!(!formatted.contains("Calls (function/method invocations)"));
    }

    #[test]
    fn test_format_references_result_dead_code() {
        let references = json!({
            "calls": [],
            "extends": [],
            "implements": [],
            "references": []
        });
        let formatted = format_references_result("UnusedMethod", &references);
        assert!(formatted.contains("No references found"));
        assert!(formatted.contains("This entity may be unused"));
    }

    #[test]
    fn test_format_references_result_multiple_targets_same_name() {
        let references = json!({
            "calls": [
                {
                    "name": "caller1",
                    "kind": "function",
                    "file_path": "src/parser/orphans.rs",
                    "start_line": 8,
                    "target_name": "find_nearest_entity_by_line",
                    "target_file_path": "src/parser/orphans.rs",
                    "target_start_line": 92,
                    "target_signature": "pub(crate) fn find_nearest_entity_by_line(entities: &[ParsedEntity], target_line: usize) -> usize"
                },
                {
                    "name": "caller2",
                    "kind": "function",
                    "file_path": "src/parser/languages/rust.rs",
                    "start_line": 258,
                    "target_name": "find_nearest_entity_by_line",
                    "target_file_path": "src/parser/languages/rust.rs",
                    "target_start_line": 445,
                    "target_signature": "fn find_nearest_entity_by_line(entities: &[ParsedEntity], line: usize) -> usize"
                },
                {
                    "name": "caller3",
                    "kind": "function",
                    "file_path": "src/parser/orphans.rs",
                    "start_line": 175,
                    "target_name": "find_nearest_entity_by_line",
                    "target_file_path": "src/parser/orphans.rs",
                    "target_start_line": 92,
                    "target_signature": "pub(crate) fn find_nearest_entity_by_line(entities: &[ParsedEntity], target_line: usize) -> usize"
                }
            ],
            "extends": [],
            "implements": [],
            "references": []
        });

        let formatted = format_references_result("find_nearest_entity_by_line", &references);

        assert!(formatted.contains("Found 3 reference(s)"));
        assert!(formatted.contains("### Target:"));
        assert!(formatted.contains("src/parser/orphans.rs:92"));
        assert!(formatted.contains("src/parser/languages/rust.rs:445"));
        assert!(formatted.contains("pub(crate) fn find_nearest_entity_by_line"));
        assert!(formatted.contains("fn find_nearest_entity_by_line"));
        assert!(formatted.contains("caller1"));
        assert!(formatted.contains("caller2"));
        assert!(formatted.contains("caller3"));
    }

    #[test]
    fn test_format_references_result_target_order_is_deterministic() {
        // The `### Target:` sections are grouped in a BTreeMap keyed by
        // `file:line`. A HashMap here would render them in process-random
        // order (SipHash keys are seeded per process), making the output
        // non-reproducible across runs and across nodes serving the same
        // graph. Pin the sorted-by-key order.
        let references = json!({
            "calls": [
                {
                    "name": "caller_z", "kind": "function",
                    "file_path": "src/z.rs", "start_line": 1,
                    "target_name": "shared", "target_file_path": "src/zeta.rs",
                    "target_start_line": 20
                },
                {
                    "name": "caller_a", "kind": "function",
                    "file_path": "src/a.rs", "start_line": 1,
                    "target_name": "shared", "target_file_path": "src/alpha.rs",
                    "target_start_line": 5
                },
                {
                    "name": "caller_m", "kind": "function",
                    "file_path": "src/m.rs", "start_line": 1,
                    "target_name": "shared", "target_file_path": "src/mu.rs",
                    "target_start_line": 99
                }
            ],
            "extends": [],
            "implements": [],
            "references": []
        });

        let formatted = format_references_result("shared", &references);
        let alpha = formatted
            .find("src/alpha.rs:5")
            .expect("alpha target present");
        let mu = formatted.find("src/mu.rs:99").expect("mu target present");
        let zeta = formatted
            .find("src/zeta.rs:20")
            .expect("zeta target present");
        assert!(
            alpha < mu && mu < zeta,
            "targets must render sorted by file:line, not in map order"
        );
    }

    #[test]
    fn test_format_references_result_renders_override_buckets() {
        // Scenario Q3 — formatter renders the new OVERRIDES buckets.
        let references = json!({
            "calls": [],
            "extends": [],
            "implements": [],
            "references": [],
            "overridden_by": [
                {"name": "Session.getUniqueId", "kind": "groovy_method",
                 "file_path": "Session.groovy", "start_line": 26}
            ],
            "overrides": [
                {"name": "ISession.getUniqueId", "kind": "groovy_method",
                 "file_path": "ISession.groovy", "start_line": 3}
            ]
        });
        let formatted = format_references_result("getUniqueId", &references);
        assert!(formatted.contains("Found 2 reference(s)"));
        assert!(formatted.contains("Overridden by (method implementations)"));
        assert!(formatted.contains("Overrides (declared supertype methods)"));
        assert!(formatted.contains("Session.getUniqueId"));
        assert!(formatted.contains("ISession.getUniqueId"));
    }

    #[test]
    fn test_format_references_result_single_target_no_grouping() {
        let references = json!({
            "calls": [
                {
                    "name": "caller1",
                    "kind": "method",
                    "file_path": "file1.java",
                    "start_line": 10,
                    "target_name": "myMethod",
                    "target_file_path": "src/Handler.java",
                    "target_start_line": 42,
                    "target_signature": "public void myMethod()"
                },
                {
                    "name": "caller2",
                    "kind": "method",
                    "file_path": "file2.java",
                    "start_line": 20,
                    "target_name": "myMethod",
                    "target_file_path": "src/Handler.java",
                    "target_start_line": 42,
                    "target_signature": "public void myMethod()"
                }
            ],
            "extends": [],
            "implements": [],
            "references": []
        });

        let formatted = format_references_result("myMethod", &references);

        assert!(formatted.contains("Found 2 reference(s)"));
        assert!(!formatted.contains("### Target:"));
        assert!(formatted.contains("caller1"));
        assert!(formatted.contains("caller2"));
    }

    #[test]
    fn test_format_renders_resolution_header() {
        let references = json!({
            "calls": [{"name": "caller1", "kind": "method", "file_path": "file1.java", "start_line": 10}],
            "extends": [],
            "implements": [],
            "references": [],
            "resolution": {
                "query": "Off",
                "tier": "exact_name",
                "fuzzy": false,
                "truncated": false,
                "targets": [
                    {
                        "uuid": "uuid-123",
                        "name": "Off",
                        "fqn": "OpenLogi.Core.Gestures.GestureOwner.Off",
                        "kind": "csharp_record",
                        "file_path": "src/OpenLogi.Core/Gestures/GestureOwner.cs",
                        "start_line": 15
                    }
                ]
            }
        });
        let formatted = format_references_result("Off", &references);
        assert!(formatted.contains("Resolved to 1 target by exact name match:"));
        assert!(formatted.contains("OpenLogi.Core.Gestures.GestureOwner.Off"));
        assert!(formatted.contains("csharp_record"));
        assert!(formatted.contains("src/OpenLogi.Core/Gestures/GestureOwner.cs:15"));
    }

    #[test]
    fn test_format_renders_fuzzy_warning() {
        let references = json!({
            "calls": [{"name": "caller1", "kind": "method", "file_path": "file1.java", "start_line": 10}],
            "extends": [],
            "implements": [],
            "references": [],
            "resolution": {
                "query": "Offlin",
                "tier": "fuzzy",
                "fuzzy": true,
                "truncated": false,
                "targets": [
                    {
                        "uuid": "uuid-123",
                        "name": "OfflineSlot",
                        "fqn": "OpenLogi.Tests.Hid.InventoryDedupeTests.OfflineSlot",
                        "kind": "csharp_method",
                        "file_path": "src/OpenLogi.Tests/Hid/InventoryDedupeTests.cs",
                        "start_line": 20
                    }
                ]
            }
        });
        let formatted = format_references_result("Offlin", &references);
        assert!(formatted.contains("**Fuzzy match**"));
        assert!(formatted.contains("no entity matched `Offlin` exactly."));
    }

    #[test]
    fn test_format_renders_truncation_notice() {
        let references = json!({
            "calls": [{"name": "caller1", "kind": "method", "file_path": "file1.java", "start_line": 10}],
            "extends": [],
            "implements": [],
            "references": [],
            "resolution": {
                "query": "DuplicateName",
                "tier": "exact_name",
                "fuzzy": false,
                "truncated": true,
                "total_targets": 112,
                "targets": [
                    {
                        "uuid": "uuid-123",
                        "name": "DuplicateName",
                        "fqn": "Some.Namespace.DuplicateName",
                        "kind": "class",
                        "file_path": "src/Duplicate.java",
                        "start_line": 10
                    }
                ]
            }
        });
        let formatted = format_references_result("DuplicateName", &references);
        assert!(
            formatted.contains("**Truncated** — 112 targets matched; showing the first 1 by FQN.")
        );
    }

    #[test]
    fn test_format_without_resolution_key_is_unchanged() {
        let references = json!({
            "calls": [{"name": "caller1", "kind": "method", "file_path": "file1.java", "start_line": 10}],
            "extends": [],
            "implements": [],
            "references": []
        });
        let formatted = format_references_result("myMethod", &references);
        let expected = "# References to `myMethod`\n\n\
                        Found 1 reference(s) across all relationship types:\n\n\
                        ## Calls (function/method invocations) (1)\n\n\
                        - **`caller1`** (method) at `file1.java:10`\n\n";
        assert_eq!(formatted, expected);
    }

    // ---- §Truncation quantification (v1.10.0) ----------------------------

    /// Fixture for the bug-report scenario: one resolution target (truncated
    /// from a larger set) that accumulated 21 caller rows.
    fn truncated_resolution_with_21_callers() -> serde_json::Value {
        let calls: Vec<serde_json::Value> = (0..21)
            .map(|i| {
                json!({
                    "name": format!("caller{}", i),
                    "kind": "function",
                    "file_path": format!("src/caller_{}.rs", i),
                    "start_line": i,
                })
            })
            .collect();
        json!({
            "calls": calls,
            "extends": [],
            "implements": [],
            "references": [],
            "resolution": {
                "query": "delete",
                "tier": "exact_name",
                "fuzzy": false,
                "truncated": true,
                "total_targets": 112,
                "targets": [
                    {
                        "uuid": "uuid-1",
                        "name": "delete",
                        "fqn": "repo::module::delete",
                        "kind": "method",
                        "file_path": "src/target.rs",
                        "start_line": 1
                    }
                ]
            }
        })
    }

    #[test]
    fn format_partial_counts_caveat_is_empty_when_not_truncated() {
        // No resolution key at all.
        let bare = json!({
            "calls": [{"name": "caller1", "kind": "method", "file_path": "f.rs", "start_line": 1}],
            "extends": [],
            "implements": [],
            "references": []
        });
        let view = ResolutionView::from_references(&bare);
        if let Some(view) = view {
            assert_eq!(view.partial_counts_caveat(), "");
        }

        // Resolution present, complete.
        let complete = json!({
            "resolution": {
                "tier": "exact_name",
                "truncated": false,
                "total_targets": 3,
                "targets": [{"fqn": "A"}, {"fqn": "B"}, {"fqn": "C"}]
            }
        });
        let view = ResolutionView::from_references(&complete).expect("resolution");
        assert_eq!(view.partial_counts_caveat(), "");
    }

    #[test]
    fn format_partial_counts_caveat_uses_true_total_not_sample_size() {
        let references = json!({
            "resolution": {
                "tier": "exact_name",
                "truncated": true,
                "total_targets": 112,
                // 1 shown target; the naive post-truncation count (25) or the
                // shown count (1) must never leak in here.
                "targets": [{"fqn": "A"}]
            }
        });
        let view = ResolutionView::from_references(&references).expect("resolution");
        assert_eq!(
            view.partial_counts_caveat(),
            "Counts below are partial — they cover only the 1 of 112 targets shown."
        );
    }

    #[test]
    fn format_renders_partial_counts_caveat_when_targets_truncated() {
        // Regression for the v1.10.0 bug: `total_targets` was never emitted
        // by the DB layer, so this shape was unreachable in production and
        // bucket counts could read as the complete impact set.
        let formatted = format_references_result("delete", &truncated_resolution_with_21_callers());

        // The caveat must be machine-visible and quantified.
        assert!(
            formatted
                .contains("Counts below are partial — they cover only the 1 of 112 targets shown."),
            "got:\n{formatted}"
        );
        assert!(formatted.contains("Re-run with a fully qualified name, or raise `max_targets`"));
    }

    #[test]
    fn format_bucket_counts_remain_samples_of_the_real_thing() {
        // The bucket header states 21 (the rows actually fetched), and the
        // caveat sits right above it quantifying the incompleteness — both
        // in the same output so no reader can miss the distinction.
        let formatted = format_references_result("delete", &truncated_resolution_with_21_callers());
        assert!(formatted.contains("## Calls (function/method invocations) (21)"));
        let caveat = formatted
            .find("Counts below are partial")
            .expect("caveat present");
        let buckets = formatted.find("## Calls").expect("bucket header");
        assert!(caveat < buckets, "caveat must precede the bucket counts");
    }

    #[test]
    fn format_omits_partial_counts_caveat_when_not_truncated() {
        let references = json!({
            "calls": [
                {"name": "caller1", "kind": "method", "file_path": "file1.java", "start_line": 10},
                {"name": "caller2", "kind": "method", "file_path": "file2.java", "start_line": 20}
            ],
            "extends": [],
            "implements": [],
            "references": [],
            "resolution": {
                "query": "myMethod",
                "tier": "exact_name",
                "fuzzy": false,
                "truncated": false,
                "total_targets": 1,
                "targets": [{}]
            }
        });
        let formatted = format_references_result("myMethod", &references);
        assert!(!formatted.contains("Counts below are partial"));
        assert!(!formatted.contains("Truncated"));
    }

    #[test]
    fn format_no_references_with_truncated_resolution_still_discloses_truncation() {
        // Truncated resolution but zero references land in the shown 25: the
        // early-return branch must still be explicit about truncation.
        let references = json!({
            "calls": [],
            "extends": [],
            "implements": [],
            "references": [],
            "resolution": {
                "query": "delete",
                "tier": "exact_name",
                "fuzzy": false,
                "truncated": true,
                "total_targets": 112,
                "targets": []
            }
        });
        let formatted = format_references_result("delete", &references);
        assert!(formatted.contains("No references found"));
        assert!(formatted.contains("**Truncated** — 112 targets matched; showing the first 0"));
        // No partial-counts caveat here: there are no counts to qualify.
        assert!(!formatted.contains("Counts below are partial"));
    }

    // ---- §Homonym attribution driven by the resolution, not the bucket ----

    /// Two resolved homonyms, callers land on only one. The header must name
    /// the referenced homonym — collapsing to the headerless single-group
    /// form erased the attribution entirely.
    #[test]
    fn format_attributes_callers_when_only_one_homonym_has_them() {
        let references = json!({
            "calls": [
                {
                    "name": "handleClick",
                    "kind": "function",
                    "file_path": "src/channels/ChannelsPage.tsx",
                    "start_line": 30,
                    "target_fqn": "channels.ChannelsPage.onDelete",
                    "target_name": "onDelete",
                    "target_file_path": "src/channels/ChannelsPage.tsx",
                    "target_start_line": 20,
                    "target_signature": "function onDelete(channelId: string)"
                }
            ],
            "extends": [],
            "implements": [],
            "references": [],
            "resolution": {
                "query": "onDelete",
                "tier": "exact_name",
                "fuzzy": false,
                "truncated": false,
                "total_targets": 2,
                "targets": [
                    {
                        "fqn": "channels.ChannelsPage.onDelete",
                        "file_path": "src/channels/ChannelsPage.tsx",
                        "start_line": 20
                    },
                    {
                        "fqn": "profile.ProfilePage.onDelete",
                        "file_path": "src/profile/ProfilePage.tsx",
                        "start_line": 69
                    }
                ]
            }
        });

        let formatted = format_references_result("onDelete", &references);

        // The caller must be attributed to the homonym it actually calls.
        assert!(
            formatted.contains("### Target: `channels.ChannelsPage.onDelete`"),
            "got:\n{formatted}"
        );
        assert!(formatted.contains("src/channels/ChannelsPage.tsx:20"));
        assert!(formatted.contains("handleClick"));
        // The callerless homonym must never be attributed any callers.
        assert!(
            !formatted.contains("### Target: `profile.ProfilePage.onDelete`"),
            "callerless homonym must not be attributed:\n{formatted}"
        );
    }

    /// Truncated resolution showing one target of many: the true total
    /// (`total_targets`) alone is enough to force attribution.
    #[test]
    fn format_attributes_callers_when_resolution_truncated_to_one_shown() {
        let references = json!({
            "calls": [
                {
                    "name": "caller1",
                    "kind": "function",
                    "file_path": "src/caller.rs",
                    "start_line": 3,
                    "target_fqn": "repo::module::delete",
                    "target_name": "delete",
                    "target_file_path": "src/target.rs",
                    "target_start_line": 1
                }
            ],
            "extends": [],
            "implements": [],
            "references": [],
            "resolution": {
                "query": "delete",
                "tier": "exact_name",
                "fuzzy": false,
                "truncated": true,
                "total_targets": 112,
                "targets": [
                    {
                        "fqn": "repo::module::delete",
                        "file_path": "src/target.rs",
                        "start_line": 1
                    }
                ]
            }
        });

        let formatted = format_references_result("delete", &references);

        assert!(
            formatted.contains("### Target: `repo::module::delete`"),
            "got:\n{formatted}"
        );
        assert!(formatted.contains("src/target.rs:1"));
    }

    /// A genuinely single-target resolution keeps the concise shape — the
    /// footer of rule: header only when the resolution matched more than
    /// one candidate. Complements the no-resolution-key case above.
    #[test]
    fn format_single_resolved_target_keeps_concise_form() {
        let references = json!({
            "calls": [
                {
                    "name": "caller1",
                    "kind": "method",
                    "file_path": "file1.java",
                    "start_line": 10,
                    "target_fqn": "myapp::Handler::myMethod",
                    "target_name": "myMethod",
                    "target_file_path": "src/Handler.java",
                    "target_start_line": 42
                },
                {
                    "name": "caller2",
                    "kind": "method",
                    "file_path": "file2.java",
                    "start_line": 20,
                    "target_fqn": "myapp::Handler::myMethod",
                    "target_name": "myMethod",
                    "target_file_path": "src/Handler.java",
                    "target_start_line": 42
                }
            ],
            "extends": [],
            "implements": [],
            "references": [],
            "resolution": {
                "query": "myMethod",
                "tier": "exact_fqn",
                "fuzzy": false,
                "truncated": false,
                "total_targets": 1,
                "targets": [
                    {
                        "fqn": "myapp::Handler::myMethod",
                        "file_path": "src/Handler.java",
                        "start_line": 42
                    }
                ]
            }
        });

        let formatted = format_references_result("myMethod", &references);

        assert!(formatted.contains("Found 2 reference(s)"));
        assert!(
            !formatted.contains("### Target:"),
            "single-target resolution must stay headerless:\n{formatted}"
        );
    }

    /// The header condition applies to every relationship bucket, including
    /// the mirrored `overrides` projection.
    #[test]
    fn format_attributes_every_relationship_bucket() {
        let row = json!({
            "name": "someCaller",
            "kind": "method",
            "file_path": "src/caller.java",
            "start_line": 10,
            "target_fqn": "com.acme.MyService.myMethod",
            "target_name": "myMethod",
            "target_file_path": "src/MyService.java",
            "target_start_line": 42
        });
        let references = json!({
            "calls": [row],
            "extends": [row],
            "implements": [row],
            "references": [row],
            "overridden_by": [row],
            "overrides": [row],
            "resolution": {
                "query": "myMethod",
                "tier": "exact_name",
                "fuzzy": false,
                "truncated": false,
                "total_targets": 2,
                "targets": [
                    {"fqn": "com.acme.MyService.myMethod"},
                    {"fqn": "com.other.MyService.myMethod"}
                ]
            }
        });

        let formatted = format_references_result("myMethod", &references);

        assert!(formatted.contains("Calls (function/method invocations)"));
        assert!(formatted.contains("Extends (class inheritance)"));
        assert!(formatted.contains("Implements (interface implementation)"));
        assert!(formatted.contains("References (type annotations/usages)"));
        assert!(formatted.contains("Overridden by (method implementations)"));
        assert!(formatted.contains("Overrides (declared supertype methods)"));
        assert_eq!(
            formatted.matches("### Target:").count(),
            6,
            "every bucket must attribute its caller:\n{formatted}"
        );
    }

    /// Rows without target identity must never receive a fabricated
    /// `### Target: <query> at unknown` header, even when the resolution
    /// resolved to many targets. Attribution by bare name is exactly what
    /// the header exists to avoid.
    #[test]
    fn format_never_fabricates_target_header_without_identity() {
        let formatted = format_references_result("delete", &truncated_resolution_with_21_callers());

        assert!(
            !formatted.contains("### Target:"),
            "unattributable rows must stay headerless:\n{formatted}"
        );
        assert!(!formatted.contains("unknown"));
    }

    // ---- §Kind-filter disclosure (kind-filtered fuzzy targets) ------------

    /// Fixture for the reported bug: a fuzzy query that resolved to a large
    /// set of documentation/build entities, all filtered out by the default
    /// code-kind scope, leaving zero code targets.
    fn all_hidden_resolution() -> serde_json::Value {
        json!({
            "calls": [],
            "extends": [],
            "implements": [],
            "references": [],
            "resolution": {
                "query": "cargo",
                "tier": "fuzzy",
                "fuzzy": true,
                "truncated": false,
                "total_targets": 0,
                "kind_filter": "code_default",
                "hidden_non_code": 93,
                "hidden_kinds": [
                    "build_dependency",
                    "cargo_feature",
                    "cargo_package",
                    "markdown_section",
                    "project_identity"
                ],
                "targets": []
            }
        })
    }

    #[test]
    fn format_discloses_hidden_non_code_matches() {
        let formatted = format_references_result("cargo", &all_hidden_resolution());

        assert!(
            formatted.contains("**Non-code matches hidden** — 93 entities matched `cargo`"),
            "got:\n{formatted}"
        );
        assert!(formatted.contains("build_dependency"));
        assert!(formatted.contains("markdown_section"));
        assert!(formatted.contains("kinds=all"));
        // The blanket "may be unused" claim must not survive a hidden-only
        // resolution: it would read as if a real, dead code entity resolved.
        assert!(
            !formatted.contains("This entity may be unused"),
            "got:\n{formatted}"
        );
        // And it must not claim the entity was never found at all.
        assert!(!formatted.contains("No entity named"));
    }

    #[test]
    fn format_zero_targets_zero_hidden_says_not_found() {
        let references = json!({
            "calls": [],
            "extends": [],
            "implements": [],
            "references": [],
            "resolution": {
                "query": "no_such_thing",
                "tier": "fuzzy",
                "fuzzy": true,
                "truncated": false,
                "total_targets": 0,
                "kind_filter": "code_default",
                "hidden_non_code": 0,
                "hidden_kinds": [],
                "targets": []
            }
        });
        let formatted = format_references_result("no_such_thing", &references);

        assert!(
            formatted.contains("No entity named `no_such_thing` was found in the indexed scope"),
            "got:\n{formatted}"
        );
        assert!(!formatted.contains("This entity may be unused"));
        assert!(!formatted.contains("Non-code matches hidden"));
    }

    #[test]
    fn format_keeps_unused_wording_when_code_target_resolved() {
        // Exactly one code target, zero references: the original dead-code
        // wording is still the correct one.
        let references = json!({
            "calls": [],
            "extends": [],
            "implements": [],
            "references": [],
            "resolution": {
                "query": "legacy_fn",
                "tier": "exact_name",
                "fuzzy": false,
                "truncated": false,
                "total_targets": 1,
                "kind_filter": "code_default",
                "hidden_non_code": 0,
                "hidden_kinds": [],
                "targets": [
                    {"fqn": "app::legacy_fn", "name": "legacy_fn", "kind": "rust_function"}
                ]
            }
        });
        let formatted = format_references_result("legacy_fn", &references);

        assert!(formatted.contains("No references found for `legacy_fn`"));
        assert!(formatted.contains("This entity may be unused"));
        assert!(!formatted.contains("Non-code matches hidden"));
        assert!(!formatted.contains("No entity named"));
    }

    #[test]
    fn format_omits_hidden_notice_when_none_hidden() {
        let references = json!({
            "calls": [{"name": "caller1", "kind": "method", "file_path": "file1.java", "start_line": 10}],
            "extends": [],
            "implements": [],
            "references": [],
            "resolution": {
                "query": "myMethod",
                "tier": "exact_name",
                "fuzzy": false,
                "truncated": false,
                "total_targets": 1,
                "kind_filter": "code_default",
                "hidden_non_code": 0,
                "hidden_kinds": [],
                "targets": [{"fqn": "app.myMethod"}]
            }
        });
        let formatted = format_references_result("myMethod", &references);

        assert!(!formatted.contains("Non-code matches hidden"));
    }

    #[test]
    fn format_renders_extended_relationship_buckets() {
        // Regression coverage for the never-consulted edge types: the
        // formatter must render (and count) the new buckets.
        let references = json!({
            "calls": [],
            "extends": [],
            "implements": [],
            "references": [],
            "macro_calls": [
                {"name": "main", "kind": "rust_function", "file_path": "sample.rs", "start_line": 145}
            ],
            "references_dom": [
                {"name": "submitBtn", "kind": "rust_function", "file_path": "ui.js", "start_line": 12}
            ],
            "uses_css_class": [],
            "imports_script": [],
            "imports_stylesheet": [],
            "uses_backend": [],
            "uses_probe": [],
            "uses_acl": [],
            "includes": [],
            "imports_vmod": [],
            "declared_unused": [],
            "overridden_by": [],
            "overrides": []
        });
        let formatted = format_references_result("init_vec", &references);

        assert!(formatted.contains("Macro calls (macro invocations)"));
        assert!(formatted.contains("DOM references (JS → HTML element id)"));
        // Empty buckets are intentionally not rendered.
        assert!(!formatted.contains("CSS class usage"));
        assert!(formatted.contains("Found 2 reference(s)"));
        assert!(formatted.contains("submitBtn"));

        // CSS-class usage over an html_id target: both are *code* kinds and
        // must keep rendering under the default filter.
        let references = json!({
            "calls": [],
            "extends": [],
            "implements": [],
            "references": [],
            "references_dom": [
                {"name": "main", "kind": "rust_function", "file_path": "ui.js", "start_line": 3}
            ],
            "macro_calls": [],
            "imports_script": [],
            "imports_stylesheet": [],
            "uses_backend": [],
            "uses_probe": [],
            "uses_acl": [],
            "includes": [],
            "imports_vmod": [],
            "declared_unused": [],
        });
        let formatted = format_references_result("copybutton__status", &references);
        assert!(formatted.contains("DOM references"));
    }
}
