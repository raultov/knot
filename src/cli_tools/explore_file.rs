//! Core explore_file logic shared between CLI and MCP
//!
//! Lists all code entities (classes, methods, interfaces, functions)
//! within a specific source file, organized by type.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::db::graph::{GraphDb, QueryExt};

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
pub fn ends_with_suffix_query(suffix: &str) -> String {
    format!("ENDS WITH '/{suffix}'")
}

/// Main explore_file function called by both CLI and MCP.
pub async fn run_explore_file(
    file_path: &str,
    repo_name: Option<&str>,
    graph_db: &Arc<GraphDb>,
) -> anyhow::Result<(String, serde_json::Value)> {
    let cwd = std::env::current_dir().ok();
    let repo_root = std::env::var("KNOT_REPO_PATH").ok().map(PathBuf::from);
    let normalized_path = resolve_explore_input(file_path, cwd.as_deref(), repo_root.as_deref());

    let entities = graph_db
        .get_file_entities(&normalized_path, repo_name)
        .await?;
    let outgoing_refs = graph_db
        .get_file_outgoing_references(&normalized_path, repo_name)
        .await
        .unwrap_or_else(|_| serde_json::json!([]));

    // §4 step 6 — DISAMBIGUATE: if the exact match produced nothing, fall
    // back to a suffix search. This handles the `transition period`
    // (relative query against an old absolute index) and the common case
    // of a bare-filename query like `src/lib.rs` from any CWD.
    if entities.as_array().is_none_or(|a| a.is_empty())
        && outgoing_refs.as_array().is_none_or(|a| a.is_empty())
        && !normalized_path.is_empty()
    {
        let suffix = ends_with_suffix_query(&normalized_path);
        if let Ok(candidates) = graph_db.find_files_by_suffix(&suffix, repo_name).await
            && candidates.as_array().is_some_and(|a| !a.is_empty())
        {
            return Ok((
                normalized_path,
                serde_json::json!({
                    "entities": entities,
                    "outgoing_references": outgoing_refs,
                    "ambiguous_path_candidates": candidates,
                }),
            ));
        }
    }

    let result = serde_json::json!({
        "entities": entities,
        "outgoing_references": outgoing_refs,
    });

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

    if entities.is_empty() && outgoing_refs.is_empty() {
        output.push_str("No entities found in this file.\n");
        return output;
    }

    if !entities.is_empty() {
        output.push_str(&format!("Found {} entity/entities:\n\n", entities.len()));

        // Group entities by kind for better organization
        let mut classes = Vec::new();
        let mut interfaces = Vec::new();
        let mut objects = Vec::new();
        let mut companions = Vec::new();
        let mut methods = Vec::new();
        let mut functions = Vec::new();
        let mut properties = Vec::new();
        let mut python_classes = Vec::new();
        let mut python_constants = Vec::new();
        let mut python_functions = Vec::new();
        let mut python_methods = Vec::new();
        let mut python_modules = Vec::new();
        let mut rust_structs = Vec::new();
        let mut rust_enums = Vec::new();
        let mut rust_unions = Vec::new();
        let mut rust_traits = Vec::new();
        let mut rust_impls = Vec::new();
        let mut rust_functions = Vec::new();
        let mut rust_methods = Vec::new();
        let mut rust_macros = Vec::new();
        let mut rust_type_aliases = Vec::new();
        let mut rust_constants = Vec::new();
        let mut rust_statics = Vec::new();
        let mut rust_modules = Vec::new();
        let mut build_deps = Vec::new();
        let mut build_plugins = Vec::new();
        let mut build_tasks = Vec::new();
        let mut pipeline_stages = Vec::new();
        let mut pipeline_steps = Vec::new();
        let mut groovy_classes = Vec::new();
        let mut groovy_interfaces = Vec::new();
        let mut groovy_traits = Vec::new();
        let mut groovy_methods = Vec::new();
        let mut groovy_functions = Vec::new();
        let mut groovy_enums = Vec::new();
        let mut groovy_properties = Vec::new();
        let mut cargo_packages = Vec::new();
        let mut cargo_features = Vec::new();
        let mut workspace_members = Vec::new();
        let mut config_properties = Vec::new();
        let mut k8s_resources = Vec::new();
        let mut helm_charts = Vec::new();
        let mut helm_values = Vec::new();
        let mut helm_template_vars = Vec::new();
        let mut others: Vec<&serde_json::Value> = Vec::new();

        for entity in &entities {
            if let Some(kind) = entity.get("kind").and_then(|v| v.as_str()) {
                match kind {
                    "class" | "kotlin_class" => classes.push(entity),
                    "interface" | "kotlin_interface" => interfaces.push(entity),
                    "kotlin_object" => objects.push(entity),
                    "kotlin_companion" => companions.push(entity),
                    "method" | "kotlin_method" => methods.push(entity),
                    "function" | "kotlin_function" => functions.push(entity),
                    "kotlin_property" => properties.push(entity),
                    "python_class" => python_classes.push(entity),
                    "python_constant" => python_constants.push(entity),
                    "python_function" => python_functions.push(entity),
                    "python_method" => python_methods.push(entity),
                    "python_module" => python_modules.push(entity),
                    "rust_struct" => rust_structs.push(entity),
                    "rust_enum" => rust_enums.push(entity),
                    "rust_union" => rust_unions.push(entity),
                    "rust_trait" => rust_traits.push(entity),
                    "rust_impl" => rust_impls.push(entity),
                    "rust_function" => rust_functions.push(entity),
                    "rust_method" => rust_methods.push(entity),
                    "rust_macro_def" | "rust_macro_invoke" => rust_macros.push(entity),
                    "rust_type_alias" => rust_type_aliases.push(entity),
                    "rust_constant" => rust_constants.push(entity),
                    "rust_static" => rust_statics.push(entity),
                    "rust_module" => rust_modules.push(entity),
                    "build_dependency" => build_deps.push(entity),
                    "build_plugin" => build_plugins.push(entity),
                    "build_task" => build_tasks.push(entity),
                    "pipeline_stage" => pipeline_stages.push(entity),
                    "pipeline_step" => pipeline_steps.push(entity),
                    "groovy_class" => groovy_classes.push(entity),
                    "groovy_interface" => groovy_interfaces.push(entity),
                    "groovy_trait" => groovy_traits.push(entity),
                    "groovy_method" => groovy_methods.push(entity),
                    "groovy_function" => groovy_functions.push(entity),
                    "groovy_enum" => groovy_enums.push(entity),
                    "groovy_property" => groovy_properties.push(entity),
                    "cargo_package" => cargo_packages.push(entity),
                    "cargo_feature" => cargo_features.push(entity),
                    "workspace_member" => workspace_members.push(entity),
                    "config_property" => config_properties.push(entity),
                    "k8s_deployment" | "k8s_service" | "k8s_configmap" | "k8s_secret"
                    | "k8s_ingress" | "k8s_namespace" | "k8s_resource" => {
                        k8s_resources.push(entity)
                    }
                    "helm_chart" => helm_charts.push(entity),
                    "helm_value" => helm_values.push(entity),
                    "helm_template_var" => helm_template_vars.push(entity),
                    _ => others.push(entity),
                }
            }
        }

        // Format in order: Classes, Interfaces, Objects, Companions, Methods, Functions, Properties
        if !classes.is_empty() {
            output.push_str("## Classes\n\n");
            for entity in classes {
                output.push_str(&format_entity_summary(entity));
            }
        }

        if !interfaces.is_empty() {
            output.push_str("## Interfaces\n\n");
            for entity in interfaces {
                output.push_str(&format_entity_summary(entity));
            }
        }

        if !objects.is_empty() {
            output.push_str("## Objects (Singletons)\n\n");
            for entity in objects {
                output.push_str(&format_entity_summary(entity));
            }
        }

        if !companions.is_empty() {
            output.push_str("## Companion Objects\n\n");
            for entity in companions {
                output.push_str(&format_entity_summary(entity));
            }
        }

        if !methods.is_empty() {
            output.push_str("## Methods\n\n");
            for entity in methods {
                output.push_str(&format_entity_summary(entity));
            }
        }

        if !functions.is_empty() {
            output.push_str("## Functions\n\n");
            for entity in functions {
                output.push_str(&format_entity_summary(entity));
            }
        }

        if !properties.is_empty() {
            output.push_str("## Properties\n\n");
            for entity in properties {
                output.push_str(&format_entity_summary(entity));
            }
        }

        // Python entities
        if !python_classes.is_empty() {
            output.push_str("## Python Classes\n\n");
            for entity in python_classes {
                output.push_str(&format_entity_summary(entity));
            }
        }

        if !python_constants.is_empty() {
            output.push_str("## Python Constants\n\n");
            for entity in python_constants {
                output.push_str(&format_entity_summary(entity));
            }
        }

        if !python_functions.is_empty() {
            output.push_str("## Python Functions\n\n");
            for entity in python_functions {
                output.push_str(&format_entity_summary(entity));
            }
        }

        if !python_methods.is_empty() {
            output.push_str("## Python Methods\n\n");
            for entity in python_methods {
                output.push_str(&format_entity_summary(entity));
            }
        }

        if !python_modules.is_empty() {
            output.push_str("## Python Modules\n\n");
            for entity in python_modules {
                output.push_str(&format_entity_summary(entity));
            }
        }

        // Rust entities
        if !rust_structs.is_empty() {
            output.push_str("## Structs (Rust)\n\n");
            for entity in rust_structs {
                output.push_str(&format_entity_summary(entity));
            }
        }

        if !rust_enums.is_empty() {
            output.push_str("## Enums (Rust)\n\n");
            for entity in rust_enums {
                output.push_str(&format_entity_summary(entity));
            }
        }

        if !rust_unions.is_empty() {
            output.push_str("## Unions (Rust)\n\n");
            for entity in rust_unions {
                output.push_str(&format_entity_summary(entity));
            }
        }

        if !rust_traits.is_empty() {
            output.push_str("## Traits (Rust)\n\n");
            for entity in rust_traits {
                output.push_str(&format_entity_summary(entity));
            }
        }

        if !rust_impls.is_empty() {
            output.push_str("## Impl Blocks (Rust)\n\n");
            for entity in rust_impls {
                output.push_str(&format_entity_summary(entity));
            }
        }

        if !rust_functions.is_empty() {
            output.push_str("## Functions (Rust)\n\n");
            for entity in rust_functions {
                output.push_str(&format_entity_summary(entity));
            }
        }

        if !rust_methods.is_empty() {
            output.push_str("## Methods (Rust)\n\n");
            for entity in rust_methods {
                output.push_str(&format_entity_summary(entity));
            }
        }

        if !rust_macros.is_empty() {
            output.push_str("## Macros (Rust)\n\n");
            for entity in rust_macros {
                output.push_str(&format_entity_summary(entity));
            }
        }

        if !rust_type_aliases.is_empty() {
            output.push_str("## Type Aliases (Rust)\n\n");
            for entity in rust_type_aliases {
                output.push_str(&format_entity_summary(entity));
            }
        }

        if !rust_constants.is_empty() {
            output.push_str("## Constants (Rust)\n\n");
            for entity in rust_constants {
                output.push_str(&format_entity_summary(entity));
            }
        }

        if !rust_statics.is_empty() {
            output.push_str("## Statics (Rust)\n\n");
            for entity in rust_statics {
                output.push_str(&format_entity_summary(entity));
            }
        }

        if !rust_modules.is_empty() {
            output.push_str("## Modules (Rust)\n\n");
            for entity in rust_modules {
                output.push_str(&format_entity_summary(entity));
            }
        }

        // Build Systems & CI/CD entities
        if !build_deps.is_empty() {
            output.push_str("## Dependencies\n\n");
            for entity in build_deps {
                output.push_str(&format_entity_summary(entity));
            }
        }

        if !build_plugins.is_empty() {
            output.push_str("## Plugins\n\n");
            for entity in build_plugins {
                output.push_str(&format_entity_summary(entity));
            }
        }

        if !build_tasks.is_empty() {
            output.push_str("## Tasks\n\n");
            for entity in build_tasks {
                output.push_str(&format_entity_summary(entity));
            }
        }

        if !pipeline_stages.is_empty() {
            output.push_str("## Pipeline Stages\n\n");
            for entity in pipeline_stages {
                output.push_str(&format_entity_summary(entity));
            }
        }

        if !pipeline_steps.is_empty() {
            output.push_str("## Pipeline Steps\n\n");
            for entity in pipeline_steps {
                output.push_str(&format_entity_summary(entity));
            }
        }

        if !groovy_classes.is_empty() {
            output.push_str("## Classes (Groovy)\n\n");
            for entity in groovy_classes {
                output.push_str(&format_entity_summary(entity));
            }
        }

        if !groovy_interfaces.is_empty() {
            output.push_str("## Interfaces (Groovy)\n\n");
            for entity in groovy_interfaces {
                output.push_str(&format_entity_summary(entity));
            }
        }

        if !groovy_traits.is_empty() {
            output.push_str("## Traits (Groovy)\n\n");
            for entity in groovy_traits {
                output.push_str(&format_entity_summary(entity));
            }
        }

        if !groovy_enums.is_empty() {
            output.push_str("## Enums (Groovy)\n\n");
            for entity in groovy_enums {
                output.push_str(&format_entity_summary(entity));
            }
        }

        if !groovy_methods.is_empty() {
            output.push_str("## Methods (Groovy)\n\n");
            for entity in groovy_methods {
                output.push_str(&format_entity_summary(entity));
            }
        }

        if !groovy_functions.is_empty() {
            output.push_str("## Functions (Groovy)\n\n");
            for entity in groovy_functions {
                output.push_str(&format_entity_summary(entity));
            }
        }

        if !groovy_properties.is_empty() {
            output.push_str("## Properties (Groovy)\n\n");
            for entity in groovy_properties {
                output.push_str(&format_entity_summary(entity));
            }
        }

        if !cargo_packages.is_empty() {
            output.push_str("## Cargo Package\n\n");
            for entity in cargo_packages {
                output.push_str(&format_entity_summary(entity));
            }
        }

        if !cargo_features.is_empty() {
            output.push_str("## Cargo Features\n\n");
            for entity in cargo_features {
                output.push_str(&format_entity_summary(entity));
            }
        }

        if !workspace_members.is_empty() {
            output.push_str("## Workspace Members\n\n");
            for entity in workspace_members {
                output.push_str(&format_entity_summary(entity));
            }
        }

        if !config_properties.is_empty() {
            output.push_str("## Configuration Properties\n\n");
            for entity in config_properties {
                output.push_str(&format_entity_summary(entity));
            }
        }

        if !k8s_resources.is_empty() {
            output.push_str("## Kubernetes Resources\n\n");
            for entity in k8s_resources {
                output.push_str(&format_entity_summary(entity));
            }
        }

        if !helm_charts.is_empty() {
            output.push_str("## Helm Chart\n\n");
            for entity in helm_charts {
                output.push_str(&format_entity_summary(entity));
            }
        }

        if !helm_values.is_empty() {
            output.push_str("## Helm Values\n\n");
            for entity in helm_values {
                output.push_str(&format_entity_summary(entity));
            }
        }

        if !helm_template_vars.is_empty() {
            output.push_str("## Template Variables\n\n");
            for entity in helm_template_vars {
                output.push_str(&format_entity_summary(entity));
            }
        }

        if !others.is_empty() {
            output.push_str("## Other Entities\n\n");
            for entity in others {
                output.push_str(&format_entity_summary(entity));
            }
        }
    }

    if !outgoing_refs.is_empty() {
        output.push_str("## Imports / Referenced Types\n\n");
        let mut seen = std::collections::HashSet::new();
        for entry in &outgoing_refs {
            let name = entry.get("name").and_then(|v| v.as_str()).unwrap_or("");
            let kind = entry.get("kind").and_then(|v| v.as_str()).unwrap_or("");
            let fp = entry
                .get("file_path")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let line_num = entry.get("line").and_then(|v| v.as_i64()).unwrap_or(0);

            let key = format!("{}:{}:{}", name, kind, fp);
            if seen.insert(key) {
                if line_num > 0 && !fp.is_empty() {
                    output.push_str(&format!("- {} ({}) — {}:{}\n", name, kind, fp, line_num));
                } else {
                    output.push_str(&format!("- {} ({})\n", name, kind));
                }
            }
        }
        output.push('\n');
    }

    output
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
        // `foobar/baz.rs`.
        let fragment = ends_with_suffix_query("src/lib.rs");
        assert_eq!(fragment, "ENDS WITH '/src/lib.rs'");
    }
}
