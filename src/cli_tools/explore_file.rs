//! Core explore_file logic shared between CLI and MCP
//!
//! Lists all code entities (classes, methods, interfaces, functions)
//! within a specific source file, organized by type.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::db::graph::{GraphDb, QueryExt};

use crate::models::RepoScope;

use crate::cli_tools::json_entities_array;

use crate::cli_tools::append_signature_if_present;

use crate::cli_tools::format_file_line;

/// Normalize caller-supplied input to the canonical form used in the index.
///
/// Implements §4 of `docs/specs/relative_file_paths.md`:
///
/// 1. POSIX separators and no leading `./`.
/// 2. EXACT — caller already supplied the stored form (no further work).
/// 3. LOCAL-ROOT — if the input exists on disk and lives under one of the
///    known local roots (`KNOT_REPO_PATH` first, then CWD), strip the root
///    and retry as a relative path.
/// 4. SUFFIX — for the `find_files` fallback: a path-boundary
///    `ENDS WITH '/' + suffix` query lets callers pass either `Cargo.toml`
///    or `path/to/Cargo.toml` and still hit a stored entity.
pub fn normalize_explore_input(input: &str, repo_root: Option<&Path>) -> String {
    let mut normalized = input.replace('\\', "/");
    if normalized.starts_with("./") {
        normalized.drain(..2);
    }
    if let Some(root) = repo_root
        && let Some(root_str) = root.to_str()
    {
        let root_norm = root_str
            .replace('\\', "/")
            .trim_end_matches('/')
            .to_string();
        let root_with_slash = format!("{root_norm}/");
        if let Some(stripped) = normalized.strip_prefix(&root_with_slash) {
            return stripped.to_string();
        }
        if normalized == root_norm {
            return String::new();
        }
    }
    normalized
}

/// Resolve `input` to the canonical repo-relative form by consulting the
/// local filesystem when the caller passed a path that exists on disk
/// (e.g. an absolute path from a developer's checkout).
///
/// `cwd` and `repo_root` are passed in for testability. Pass
/// `std::env::current_dir().ok()` and `std::env::var("KNOT_REPO_PATH").ok()`
/// from production code.
pub fn resolve_explore_input(input: &str, cwd: Option<&Path>, repo_root: Option<&Path>) -> String {
    let mut candidate = normalize_explore_input(input, repo_root);
    let input_path = Path::new(input);

    if input_path.exists()
        && let Ok(canonical) = std::fs::canonicalize(input_path)
    {
        let roots: [Option<&Path>; 2] = [repo_root, cwd];
        for root in roots.iter().flatten() {
            if let Ok(rel) = canonical.strip_prefix(root) {
                let s = rel.to_string_lossy().replace('\\', "/");
                candidate = s.trim_start_matches("./").to_string();
                break;
            }
        }
    }
    candidate
}

/// Build the suffix query fragment used by the SUFFIX fallback of
/// `run_explore_file`. Exposed for unit tests in §10.1.
///
/// Returns a complete `e.file_path ...` predicate ready to be dropped into
/// a parenthesised `WHERE` clause. For paths that do not begin with `/`
/// (the common relative-path case), the predicate matches both the
/// `/`-bounded form (`/src/index.ts`) and the bare form (`src/index.ts`),
/// since the indexer persists repo-root files without a leading slash.
pub fn ends_with_suffix_query(suffix: &str) -> String {
    if suffix.starts_with('/') {
        format!("e.file_path ENDS WITH '{suffix}'")
    } else {
        format!("(e.file_path ENDS WITH '/{suffix}' OR e.file_path = '{suffix}')")
    }
}

/// Main explore_file function called by both CLI and MCP.
pub async fn run_explore_file(
    file_path: &str,
    repo: &RepoScope,
    graph_db: &Arc<GraphDb>,
) -> anyhow::Result<(String, serde_json::Value)> {
    let cwd = std::env::current_dir().ok();
    let repo_root = std::env::var("KNOT_REPO_PATH").ok().map(PathBuf::from);
    let normalized_path = resolve_explore_input(file_path, cwd.as_deref(), repo_root.as_deref());

    let repo_names = repo.filter_names();

    let entities = graph_db
        .get_file_entities(&normalized_path, &repo_names)
        .await?;
    let outgoing_refs = graph_db
        .get_file_outgoing_references(&normalized_path, &repo_names)
        .await
        .unwrap_or_else(|_| serde_json::json!([]));

    let mut result = serde_json::json!({
        "entities": entities,
        "outgoing_references": outgoing_refs,
    });

    // §4 step 6 — DISAMBIGUATE: when the exact match produces nothing,
    // OR when the path exists in more than one repository under the
    // active scope, surface `ambiguous_path_candidates` so callers can
    // disambiguate. The empty-match case handles the `transition period`
    // (relative query against an old absolute index) and the common case
    // of a bare-filename query like `src/lib.rs` from any CWD; the
    // multi-repo case is what enables issue #19's repo-scope selection.
    let entities_empty = entities.as_array().is_none_or(|a| a.is_empty());
    if !normalized_path.is_empty() {
        let suffix = ends_with_suffix_query(&normalized_path);
        if let Ok(candidates) = graph_db.find_files_by_suffix(&suffix, &repo_names).await
            && let Some(cand_arr) = candidates.as_array()
        {
            let mut distinct_repos = std::collections::HashSet::new();
            for cand in cand_arr {
                if let Some(repo) = cand.get("repo_name").and_then(|v| v.as_str()) {
                    distinct_repos.insert(repo.to_string());
                }
            }
            let is_ambiguous = distinct_repos.len() > 1;
            if (entities_empty || is_ambiguous)
                && let serde_json::Value::Object(map) = &mut result
            {
                map.insert("ambiguous_path_candidates".to_string(), candidates);
            }
        }
    }

    Ok((normalized_path, result))
}

pub fn format_file_entities(file_path: &str, result: &serde_json::Value) -> String {
    let mut output = format!("# Entities in {}\n\n", format_file_line(file_path, None));

    let entities = json_entities_array(result);

    let outgoing_refs = result
        .as_object()
        .and_then(|obj| obj.get("outgoing_references"))
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    let ambiguous_candidates = result
        .as_object()
        .and_then(|obj| obj.get("ambiguous_path_candidates"))
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    if entities.is_empty() && outgoing_refs.is_empty() && ambiguous_candidates.is_empty() {
        output.push_str("No entities found in this file.\n");
        return output;
    }

    if !entities.is_empty() {
        output.push_str(&format!("Found {} entity/entities:\n\n", entities.len()));
        append_entity_groups(&mut output, &entities);
    }

    append_outgoing_references(&mut output, &outgoing_refs);
    append_ambiguous_candidates(&mut output, &ambiguous_candidates);

    output
}

/// Render the `ambiguous_path_candidates` field as a Markdown section so LLM
/// agents and the CLI/MCP grep tests can both detect the disambiguation hint.
/// Each entry is rendered as a JSON object line so callers can parse them.
fn append_ambiguous_candidates(output: &mut String, candidates: &[serde_json::Value]) {
    if candidates.is_empty() {
        return;
    }

    output.push_str(&format!(
        "## ambiguous_path_candidates ({} entries)\n\n",
        candidates.len()
    ));
    for cand in candidates {
        output.push_str(&format!("- `{}`\n", cand));
    }
    output.push('\n');
}

/// One bucket in the entity grouping table — collects every JSON `kind`
/// that maps to a given Markdown section header.
struct KindBucket {
    header: &'static str,
    kinds: &'static [&'static str],
}

/// Ordered table of every recognised entity kind and the Markdown section
/// header it should appear under. Ordering determines the section order
/// in the rendered output.
const KIND_BUCKETS: &[KindBucket] = &[
    KindBucket {
        header: "Classes",
        kinds: &["class", "kotlin_class"],
    },
    KindBucket {
        header: "Interfaces",
        kinds: &["interface", "kotlin_interface"],
    },
    KindBucket {
        header: "Objects (Singletons)",
        kinds: &["kotlin_object"],
    },
    KindBucket {
        header: "Companion Objects",
        kinds: &["kotlin_companion"],
    },
    KindBucket {
        header: "Methods",
        kinds: &["method", "kotlin_method"],
    },
    KindBucket {
        header: "Functions",
        kinds: &["function", "kotlin_function"],
    },
    KindBucket {
        header: "Properties",
        kinds: &["kotlin_property"],
    },
    KindBucket {
        header: "Classes (C#)",
        kinds: &["csharp_class"],
    },
    KindBucket {
        header: "Interfaces (C#)",
        kinds: &["csharp_interface"],
    },
    KindBucket {
        header: "Structs (C#)",
        kinds: &["csharp_struct"],
    },
    KindBucket {
        header: "Records (C#)",
        kinds: &["csharp_record"],
    },
    KindBucket {
        header: "Enums (C#)",
        kinds: &["csharp_enum"],
    },
    KindBucket {
        header: "Methods (C#)",
        kinds: &[
            "csharp_method",
            "csharp_constructor",
            "csharp_local_function",
        ],
    },
    KindBucket {
        header: "Properties & Fields (C#)",
        kinds: &["csharp_property", "csharp_field", "csharp_constant"],
    },
    KindBucket {
        header: "Delegates & Events (C#)",
        kinds: &["csharp_delegate", "csharp_event"],
    },
    KindBucket {
        header: "Operators & Indexers (C#)",
        kinds: &["csharp_operator", "csharp_indexer"],
    },
    KindBucket {
        header: "Namespaces (C#)",
        kinds: &["csharp_namespace"],
    },
    KindBucket {
        header: "Python Classes",
        kinds: &["python_class"],
    },
    KindBucket {
        header: "Python Constants",
        kinds: &["python_constant"],
    },
    KindBucket {
        header: "Python Functions",
        kinds: &["python_function"],
    },
    KindBucket {
        header: "Python Methods",
        kinds: &["python_method"],
    },
    KindBucket {
        header: "Python Modules",
        kinds: &["python_module"],
    },
    KindBucket {
        header: "Structs (Rust)",
        kinds: &["rust_struct"],
    },
    KindBucket {
        header: "Enums (Rust)",
        kinds: &["rust_enum"],
    },
    KindBucket {
        header: "Unions (Rust)",
        kinds: &["rust_union"],
    },
    KindBucket {
        header: "Traits (Rust)",
        kinds: &["rust_trait"],
    },
    KindBucket {
        header: "Impl Blocks (Rust)",
        kinds: &["rust_impl"],
    },
    KindBucket {
        header: "Functions (Rust)",
        kinds: &["rust_function"],
    },
    KindBucket {
        header: "Methods (Rust)",
        kinds: &["rust_method"],
    },
    KindBucket {
        header: "Macros (Rust)",
        kinds: &["rust_macro_def", "rust_macro_invoke"],
    },
    KindBucket {
        header: "Type Aliases (Rust)",
        kinds: &["rust_type_alias"],
    },
    KindBucket {
        header: "Constants (Rust)",
        kinds: &["rust_constant"],
    },
    KindBucket {
        header: "Statics (Rust)",
        kinds: &["rust_static"],
    },
    KindBucket {
        header: "Modules (Rust)",
        kinds: &["rust_module"],
    },
    KindBucket {
        header: "Dependencies",
        kinds: &["build_dependency"],
    },
    KindBucket {
        header: "Plugins",
        kinds: &["build_plugin"],
    },
    KindBucket {
        header: "Tasks",
        kinds: &["build_task"],
    },
    KindBucket {
        header: "Pipeline Stages",
        kinds: &["pipeline_stage"],
    },
    KindBucket {
        header: "Pipeline Steps",
        kinds: &["pipeline_step"],
    },
    KindBucket {
        header: "Classes (Groovy)",
        kinds: &["groovy_class"],
    },
    KindBucket {
        header: "Interfaces (Groovy)",
        kinds: &["groovy_interface"],
    },
    KindBucket {
        header: "Traits (Groovy)",
        kinds: &["groovy_trait"],
    },
    KindBucket {
        header: "Enums (Groovy)",
        kinds: &["groovy_enum"],
    },
    KindBucket {
        header: "Methods (Groovy)",
        kinds: &["groovy_method"],
    },
    KindBucket {
        header: "Functions (Groovy)",
        kinds: &["groovy_function"],
    },
    KindBucket {
        header: "Properties (Groovy)",
        kinds: &["groovy_property"],
    },
    KindBucket {
        header: "Cargo Package",
        kinds: &["cargo_package"],
    },
    KindBucket {
        header: "Cargo Features",
        kinds: &["cargo_feature"],
    },
    KindBucket {
        header: "Workspace Members",
        kinds: &["workspace_member"],
    },
    KindBucket {
        header: "Configuration Properties",
        kinds: &["config_property"],
    },
    KindBucket {
        header: "Kubernetes Resources",
        kinds: &[
            "k8s_deployment",
            "k8s_service",
            "k8s_configmap",
            "k8s_secret",
            "k8s_ingress",
            "k8s_namespace",
            "k8s_resource",
        ],
    },
    KindBucket {
        header: "Helm Chart",
        kinds: &["helm_chart"],
    },
    KindBucket {
        header: "Helm Values",
        kinds: &["helm_value"],
    },
    KindBucket {
        header: "Template Variables",
        kinds: &["helm_template_var"],
    },
];

const OTHERS_HEADER: &str = "Other Entities";

/// Group `entities` by [`KIND_BUCKETS`] and append a Markdown section per
/// non-empty bucket (plus a final `Other Entities` bucket for any kinds
/// not present in the table).
fn append_entity_groups(output: &mut String, entities: &[serde_json::Value]) {
    let mut buckets: Vec<Vec<&serde_json::Value>> = vec![Vec::new(); KIND_BUCKETS.len()];
    let mut others: Vec<&serde_json::Value> = Vec::new();

    for entity in entities {
        let kind = entity.get("kind").and_then(|v| v.as_str()).unwrap_or("");
        match KIND_BUCKETS.iter().position(|b| b.kinds.contains(&kind)) {
            Some(idx) => buckets[idx].push(entity),
            None => others.push(entity),
        }
    }

    for (bucket, items) in KIND_BUCKETS.iter().zip(buckets) {
        if items.is_empty() {
            continue;
        }
        output.push_str(&format!("## {}\n\n", bucket.header));
        for entity in items {
            output.push_str(&format_entity_summary(entity));
        }
    }

    if !others.is_empty() {
        output.push_str(&format!("## {OTHERS_HEADER}\n\n"));
        for entity in others {
            output.push_str(&format_entity_summary(entity));
        }
    }
}

/// Append the "Imports / Referenced Types" section, deduplicating on
/// `(name, kind, file_path)`.
fn append_outgoing_references(output: &mut String, outgoing_refs: &[serde_json::Value]) {
    if outgoing_refs.is_empty() {
        return;
    }

    output.push_str("## Imports / Referenced Types\n\n");
    let mut seen = std::collections::HashSet::new();
    for entry in outgoing_refs {
        let name = entry.get("name").and_then(|v| v.as_str()).unwrap_or("");
        let kind = entry.get("kind").and_then(|v| v.as_str()).unwrap_or("");
        let fp = entry
            .get("file_path")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let line_num = entry.get("line").and_then(|v| v.as_i64()).unwrap_or(0);

        let key = format!("{}:{}:{}", name, kind, fp);
        if !seen.insert(key) {
            continue;
        }
        if line_num > 0 && !fp.is_empty() {
            output.push_str(&format!("- {} ({}) — {}:{}\n", name, kind, fp, line_num));
        } else {
            output.push_str(&format!("- {} ({})\n", name, kind));
        }
    }
    output.push('\n');
}

/// Format entity summary as Markdown
fn format_entity_summary(entity: &serde_json::Value) -> String {
    let mut output = String::new();

    if let Some(name) = entity.get("name").and_then(|v| v.as_str()) {
        output.push_str(&format!("- **`{}`**", name));

        if let Some(start_line) = entity.get("start_line").and_then(|v| v.as_i64()) {
            output.push_str(&format!(" (line {})", start_line));
        }

        output.push('\n');

        if let Some(decorators_array) = entity.get("decorators").and_then(|v| v.as_array())
            && !decorators_array.is_empty()
        {
            let decorator_strs: Vec<String> = decorators_array
                .iter()
                .filter_map(|v| v.as_str())
                .map(|s| s.to_string())
                .collect();
            if !decorator_strs.is_empty() {
                output.push_str(&format!("  - Decorators: {}\n", decorator_strs.join(", ")));
            }
        }

        append_signature_if_present(&mut output, entity);

        if let Some(docstring) = entity
            .get("docstring")
            .and_then(|v| v.as_str())
            .filter(|s| !s.trim().is_empty())
        {
            let doc_preview = docstring.lines().next().unwrap_or("");
            output.push_str(&format!("  - Doc: {}\n", doc_preview));
        }

        output.push('\n');
    }

    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_format_file_entities_empty() {
        let entities = json!([]);
        let formatted = format_file_entities("src/main.java", &entities);
        assert!(formatted.contains("No entities found in this file"));
    }

    #[test]
    fn test_format_file_entities_single_class() {
        let entities = json!([
            {
                "name": "MyClass",
                "kind": "class",
                "start_line": 10,
                "signature": "public class MyClass"
            }
        ]);
        let formatted = format_file_entities("src/main.java", &entities);
        assert!(formatted.contains("## Classes"));
        assert!(formatted.contains("MyClass"));
        assert!(formatted.contains("(line 10)"));
        assert!(formatted.contains("public class MyClass"));
    }

    #[test]
    fn test_format_file_entities_multiple_classes() {
        let entities = json!([
            {
                "name": "Class1",
                "kind": "class",
                "start_line": 10
            },
            {
                "name": "Class2",
                "kind": "class",
                "start_line": 50
            }
        ]);
        let formatted = format_file_entities("src/main.java", &entities);
        assert!(formatted.contains("Found 2 entity/entities"));
        assert!(formatted.contains("Class1"));
        assert!(formatted.contains("Class2"));
    }

    #[test]
    fn test_format_file_entities_groups_by_kind() {
        let entities = json!([
            {"name": "MyClass", "kind": "class"},
            {"name": "MyInterface", "kind": "interface"},
            {"name": "myMethod", "kind": "method"},
            {"name": "myFunction", "kind": "function"}
        ]);
        let formatted = format_file_entities("src/main.java", &entities);
        assert!(formatted.contains("## Classes"));
        assert!(formatted.contains("## Interfaces"));
        assert!(formatted.contains("## Methods"));
        assert!(formatted.contains("## Functions"));
    }

    #[test]
    fn test_format_entity_summary_with_signature() {
        let entity = json!({
            "name": "myMethod",
            "kind": "method",
            "start_line": 20,
            "signature": "public void myMethod(String param)"
        });
        let formatted = format_entity_summary(&entity);
        assert!(formatted.contains("myMethod"));
        assert!(formatted.contains("(line 20)"));
        assert!(formatted.contains("public void myMethod(String param)"));
    }

    #[test]
    fn test_format_entity_summary_with_docstring() {
        let entity = json!({
            "name": "myMethod",
            "kind": "method",
            "docstring": "First line of doc\nSecond line of doc"
        });
        let formatted = format_entity_summary(&entity);
        assert!(formatted.contains("myMethod"));
        assert!(formatted.contains("First line of doc"));
        assert!(!formatted.contains("Second line of doc"));
    }

    #[test]
    fn test_format_entity_summary_ignores_whitespace_docstring() {
        let entity = json!({
            "name": "myMethod",
            "kind": "method",
            "docstring": "   \n  \t"
        });
        let formatted = format_entity_summary(&entity);
        assert!(!formatted.contains("- Doc:"));
    }

    #[test]
    fn test_format_entity_summary_without_optional_fields() {
        let entity = json!({
            "name": "MyClass",
            "kind": "class"
        });
        let formatted = format_entity_summary(&entity);
        assert!(formatted.contains("MyClass"));
        assert!(!formatted.contains("(line"));
        assert!(!formatted.contains("Signature:"));
    }

    #[test]
    fn test_format_file_entities_unknown_kind() {
        let entities = json!([
            {
                "name": "UnknownEntity",
                "kind": "unknown_kind"
            }
        ]);
        let formatted = format_file_entities("src/main.java", &entities);
        // Unknown kinds fall into the "Other Entities" bucket so they remain visible.
        assert!(formatted.contains("UnknownEntity"));
        assert!(formatted.contains("Found 1 entity/entities"));
        assert!(formatted.contains("## Other Entities"));
    }

    #[test]
    fn test_format_file_entities_displays_file_path() {
        let entities = json!([
            {"name": "MyClass", "kind": "class"}
        ]);
        let formatted = format_file_entities("src/main/java/MyClass.java", &entities);
        assert!(formatted.contains("src/main/java/MyClass.java"));
    }

    // ---- §10.1 unit tests for input normalization ----

    #[test]
    fn test_normalize_input_strips_dot_slash_and_backslashes() {
        let root = Path::new("/repo");
        let result = normalize_explore_input("./src\\lib.rs", Some(root));
        assert_eq!(result, "src/lib.rs");
    }

    #[test]
    fn test_normalize_input_passthrough_when_no_root() {
        let result = normalize_explore_input("src/lib.rs", None);
        assert_eq!(result, "src/lib.rs");
    }

    #[test]
    fn test_normalize_input_strips_known_absolute_root() {
        let root = Path::new("/home/user/myrepo");
        let result = normalize_explore_input("/home/user/myrepo/src/lib.rs", Some(root));
        assert_eq!(result, "src/lib.rs");
    }

    #[test]
    fn test_normalize_absolute_unknown_root_passthrough() {
        // Path under no known root must be passed through verbatim
        // (after backslash/dot-slash normalization) — the SUFFIX fallback
        // in `run_explore_file` is what eventually matches it.
        let root = Path::new("/home/user/myrepo");
        let result = normalize_explore_input("/elsewhere/src/lib.rs", Some(root));
        assert_eq!(result, "/elsewhere/src/lib.rs");
    }

    #[test]
    fn test_ends_with_suffix_query_uses_path_boundary() {
        // Spec §4 step 5: the '/' guard prevents `bar/baz.rs` matching
        // `foobar/baz.rs`. For relative paths we additionally OR the bare
        // equality case so repo-root files stored without a leading slash
        // are still discoverable (issue #19 fixtures use `src/index.ts`).
        let fragment = ends_with_suffix_query("src/lib.rs");
        assert_eq!(
            fragment,
            "(e.file_path ENDS WITH '/src/lib.rs' OR e.file_path = 'src/lib.rs')"
        );
    }

    #[test]
    fn test_ends_with_suffix_query_absolute_path_uses_ends_with() {
        // An absolute path keeps the simple `ENDS WITH` form — the leading
        // '/' is already there and there's no need to fall back to equality.
        let fragment = ends_with_suffix_query("/repo/src/lib.rs");
        assert_eq!(fragment, "e.file_path ENDS WITH '/repo/src/lib.rs'");
    }

    // ---- Phase 4 compilation contract: RepoScope flows through ----

    #[test]
    fn test_repo_scope_all_filter_names_is_empty_for_unfiltered_passthrough() {
        let scope = RepoScope::All;
        assert!(scope.is_unfiltered());
        assert!(
            scope.filter_names().is_empty(),
            "RepoScope::All must yield an empty filter list so the DB layer treats it as unfiltered"
        );
    }
}
