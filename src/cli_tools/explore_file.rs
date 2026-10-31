//! Core explore_file logic shared between CLI and MCP
//!
//! Lists all code entities (classes, methods, interfaces, functions)
//! within a specific source file, organized by type.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::db::graph::{GraphDb, QueryExt};

use crate::models::RepoScope;

use crate::cli_tools::json_entities_array;

use crate::cli_tools::{compact_signature, doc_summary};

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
/// a parenthesized `WHERE` clause. For paths that do not begin with `/`
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

    let entities_arr = json_entities_array(&entities);
    let (code_entities, markup_groups, markup_total) = split_markup(&entities_arr);

    let mut result = serde_json::json!({
        "entities": entities,
        "outgoing_references": outgoing_refs,
        "markup_references": markup_groups.iter().map(|g| {
            serde_json::json!({
                "name": g.name,
                "kind": g.kind,
                "count": g.count,
            })
        }).collect::<Vec<_>>(),
        "markup_count": markup_total,
        "code_entity_count": code_entities.len(),
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

    // Explicit three-state signal for programmatic consumers
    // (MCP/HTTP callers): `false` distinguishes "no indexed file matched
    // the path" from "file indexed but carries zero code entities" without
    // parsing the prose.
    if let serde_json::Value::Object(map) = &mut result {
        let file_matched = !(entities_empty
            && map
                .get("ambiguous_path_candidates")
                .and_then(|v| v.as_array())
                .is_none_or(Vec::is_empty));
        map.insert("file_matched".to_string(), serde_json::json!(file_matched));
    }

    Ok((normalized_path, result))
}

pub fn format_file_entities(
    file_path: &str,
    result: &serde_json::Value,
    include_markup: bool,
) -> String {
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
        output.push_str(&format!(
            "No indexed file matched `{}`. The path may not exist, may not be \
             indexed, or may contain no indexed entities.",
            file_path
        ));
        if !file_path.trim().is_empty() {
            output.push_str(
                " A suffix-based lookup (`knot files` or the bare filename) may still find it.\n",
            );
        } else {
            output.push('\n');
        }
        return output;
    }

    if entities.is_empty() && outgoing_refs.is_empty() && !ambiguous_candidates.is_empty() {
        output.push_str(&format!(
            "No indexed file matched `{}` exactly. Did you mean:\n",
            file_path
        ));
        append_ambiguous_candidates(&mut output, &ambiguous_candidates);
        return output;
    }

    if !entities.is_empty() {
        let entities_arr = json_entities_array(result);
        let (code_entities, markup_groups, markup_total) = split_markup(&entities_arr);
        if markup_total == 0 {
            output.push_str(&format!("Found {} entity/entities:\n\n", entities.len()));
        } else {
            output.push_str(&format!(
                "Found {} code entity/entities ({} markup references summarized below):\n\n",
                code_entities.len(),
                markup_total
            ));
        }

        append_entity_groups(&mut output, &code_entities);
        append_markup_section(&mut output, &markup_groups, markup_total, include_markup);
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

/// Ordered table of every recognized entity kind and the Markdown section
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
        kinds: &["kotlin_companion_object"],
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
        header: "Constants",
        kinds: &["constant"],
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
    // --- Additional language / web / config / prose buckets ---
    KindBucket {
        header: "Enums",
        kinds: &["enum", "kotlin_enum"],
    },
    KindBucket {
        header: "CSS Selectors",
        kinds: &["css_class", "css_id"],
    },
    KindBucket {
        header: "CSS Variables",
        kinds: &["css_variable", "scss_variable"],
    },
    KindBucket {
        header: "Mixins & Functions (SCSS)",
        kinds: &["scss_mixin", "scss_function"],
    },
    KindBucket {
        header: "HTML Elements",
        kinds: &["html_element"],
    },
    KindBucket {
        header: "Structs (C)",
        kinds: &["c_struct"],
    },
    KindBucket {
        header: "Functions (C)",
        kinds: &["c_function"],
    },
    KindBucket {
        header: "Classes (C++)",
        kinds: &["cpp_class"],
    },
    KindBucket {
        header: "Methods (C++)",
        kinds: &["cpp_method"],
    },
    KindBucket {
        header: "Namespaces (C++)",
        kinds: &["cpp_namespace"],
    },
    KindBucket {
        header: "Macros (C/C++)",
        kinds: &["macro_definition"],
    },
    KindBucket {
        header: "Subroutines (VCL)",
        kinds: &["vcl_subroutine", "vcl_builtin_sub"],
    },
    KindBucket {
        header: "Backends & Probes (VCL)",
        kinds: &["vcl_backend", "vcl_probe"],
    },
    KindBucket {
        header: "ACLs (VCL)",
        kinds: &["vcl_acl"],
    },
    KindBucket {
        header: "Imports & Instances (VCL)",
        kinds: &["vcl_import", "vcl_object_instance"],
    },
    KindBucket {
        header: "VCL Version",
        kinds: &["vcl_version"],
    },
    KindBucket {
        header: "Test Cases (VTC)",
        kinds: &["vtc_test_case"],
    },
    KindBucket {
        header: "Test Actors (VTC)",
        kinds: &[
            "vtc_server",
            "vtc_client",
            "vtc_varnish_instance",
            "vtc_logexpect",
            "vtc_barrier",
        ],
    },
    KindBucket {
        header: "Modules (VCC)",
        kinds: &["vcc_module", "vcc_object"],
    },
    KindBucket {
        header: "Functions & Methods (VCC)",
        kinds: &["vcc_function", "vcc_method"],
    },
    KindBucket {
        header: "Documentation Sections",
        kinds: &["markdown_document", "markdown_section"],
    },
    KindBucket {
        header: "Project Identity",
        kinds: &["project_identity"],
    },
    KindBucket {
        header: "HTTP Endpoints",
        kinds: &["http_endpoint"],
    },
    KindBucket {
        header: "HTTP Client Calls",
        kinds: &["http_client_call"],
    },
];

const OTHERS_HEADER: &str = "Other Entities";

/// Attribute-usage kinds: the JSX/HTML parsers emit one entity per
/// whitespace-separated token of every `class=`/`className=` and one per
/// `id=`, so a single Tailwind className yields 10-20 of them. They are
/// reference targets of REFERENCES_DOM / USES_CSS_CLASS, not file anatomy.
/// Declaration kinds (`css_class`, `css_id`, `html_element`) are NOT here:
/// they carry their own sections and keep their line numbers.
const MARKUP_KINDS: &[&str] = &["html_class", "html_id"];
const MARKUP_HEADER: &str = "Markup References (not code)";
const MARKUP_PREVIEW_LIMIT: usize = 20;

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct MarkupGroup {
    pub name: String,
    pub kind: String,
    pub count: usize,
}

pub(crate) fn is_markup_kind(kind: &str) -> bool {
    MARKUP_KINDS.contains(&kind)
}

/// Split entities into (code_entities, distinct_markup_groups, total_markup_count).
///
/// Groups sort by count desc, then name asc, then kind asc, so the rendered
/// summary is deterministic and assertable across test runs.
pub(crate) fn split_markup(
    entities: &[serde_json::Value],
) -> (Vec<&serde_json::Value>, Vec<MarkupGroup>, usize) {
    let mut code_entities = Vec::new();
    let mut counts: std::collections::BTreeMap<(String, String), usize> =
        std::collections::BTreeMap::new();
    let mut markup_total = 0;

    for entity in entities {
        let kind = entity.get("kind").and_then(|v| v.as_str()).unwrap_or("");
        if is_markup_kind(kind) {
            markup_total += 1;
            let name = entity.get("name").and_then(|v| v.as_str()).unwrap_or("");
            *counts
                .entry((name.to_string(), kind.to_string()))
                .or_insert(0) += 1;
        } else {
            code_entities.push(entity);
        }
    }

    let mut groups: Vec<MarkupGroup> = counts
        .into_iter()
        .map(|((name, kind), count)| MarkupGroup { name, kind, count })
        .collect();

    // Sort by count desc, then name asc, then kind asc
    groups.sort_by(|a, b| {
        b.count
            .cmp(&a.count)
            .then_with(|| a.name.cmp(&b.name))
            .then_with(|| a.kind.cmp(&b.kind))
    });

    (code_entities, groups, markup_total)
}

/// Group `entities` by [`KIND_BUCKETS`] and append a Markdown section per
/// non-empty bucket (plus a final `Other Entities` bucket for any kinds
/// not present in the table).
fn append_entity_groups(output: &mut String, entities: &[&serde_json::Value]) {
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
        append_entity_group(output, bucket.header, &items);
    }

    if !others.is_empty() {
        append_entity_group(output, OTHERS_HEADER, &others);
    }
}

fn append_entity_group(output: &mut String, header: &str, entities: &[&serde_json::Value]) {
    output.push_str(&format!("## {header}\n\n"));
    for entity in entities {
        output.push_str(&format_entity_summary(entity));
    }
    output.push('\n');
}

/// Render the markup section either as a compact summary or in full when opt-in.
fn append_markup_section(
    output: &mut String,
    groups: &[MarkupGroup],
    total_count: usize,
    include_markup: bool,
) {
    if total_count == 0 {
        return;
    }

    output.push_str(&format!("## {MARKUP_HEADER}\n\n"));

    if include_markup {
        for group in groups {
            output.push_str(&format!(
                "- **`{}`** ({}) ×{}\n",
                group.name, group.kind, group.count
            ));
        }
        output.push('\n');
    } else {
        let distinct_count = groups.len();
        output.push_str(&format!(
            "{} markup references, {} distinct, from `class`/`id` attributes.\n",
            total_count, distinct_count
        ));

        let top_tokens: Vec<String> = groups
            .iter()
            .take(MARKUP_PREVIEW_LIMIT)
            .map(|g| format!("`{}` ×{}", g.name, g.count))
            .collect();

        if !top_tokens.is_empty() {
            output.push_str("Top tokens: ");
            output.push_str(&top_tokens.join(", "));
            if groups.len() > MARKUP_PREVIEW_LIMIT {
                output.push_str(&format!(
                    ", … and {} more",
                    groups.len() - MARKUP_PREVIEW_LIMIT
                ));
            }
            output.push_str(".\n");
        }
        output.push_str(
            "Pass `--include-markup` (CLI) or `include_markup: true` (MCP) to list them all.\n\n",
        );
    }
}

/// Append the "Imports / Referenced Types" section: one line per declaring
/// file listing the names it provides, in first-appearance order (issue
/// #50). Names declared nowhere in the index share a single path-less line.
fn append_outgoing_references(output: &mut String, outgoing_refs: &[serde_json::Value]) {
    if outgoing_refs.is_empty() {
        return;
    }

    let mut groups: Vec<(&str, Vec<&str>)> = Vec::new();
    for entry in outgoing_refs {
        let name = entry.get("name").and_then(|v| v.as_str()).unwrap_or("");
        if name.is_empty() {
            continue;
        }
        let file = entry
            .get("file_path")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let idx = match groups.iter().position(|(f, _)| *f == file) {
            Some(idx) => idx,
            None => {
                groups.push((file, Vec::new()));
                groups.len() - 1
            }
        };
        if !groups[idx].1.contains(&name) {
            groups[idx].1.push(name);
        }
    }

    output.push_str("## Imports / Referenced Types\n\n");
    for (file, names) in groups {
        let names = names.join(", ");
        if file.is_empty() {
            output.push_str(&format!("- {names}\n"));
        } else {
            output.push_str(&format!("- `{file}`: {names}\n"));
        }
    }
    output.push('\n');
}

/// Longest documentation preview, in characters.
const DOC_PREVIEW_MAX_CHARS: usize = 200;

/// The first sentence of `docstring`'s first paragraph ([`doc_summary`]),
/// read across line breaks so a multi-line JSDoc never stops mid-sentence
/// (issue #50). Capped at [`DOC_PREVIEW_MAX_CHARS`].
fn doc_preview(docstring: &str) -> String {
    let paragraph = doc_summary(docstring);
    // A sentence ends at `.`, `!` or `?` followed by whitespace and an
    // uppercase letter (so `e.g. foo` and `v1.2` do not end it).
    let chars: Vec<(usize, char)> = paragraph.char_indices().collect();
    let sentence_end = chars.windows(3).find_map(|w| {
        (matches!(w[0].1, '.' | '!' | '?') && w[1].1.is_whitespace() && w[2].1.is_uppercase())
            .then_some(w[1].0)
    });
    let sentence = &paragraph[..sentence_end.unwrap_or(paragraph.len())];
    if sentence.chars().count() <= DOC_PREVIEW_MAX_CHARS {
        return sentence.to_string();
    }
    let cut: String = sentence.chars().take(DOC_PREVIEW_MAX_CHARS).collect();
    format!("{}…", cut.trim_end())
}

/// Format entity summary as Markdown: `- **`name`** (line N): `signature``,
/// then its decorators and documentation preview, if any.
fn format_entity_summary(entity: &serde_json::Value) -> String {
    let mut output = String::new();

    if let Some(name) = entity.get("name").and_then(|v| v.as_str()) {
        output.push_str(&format!("- **`{}`**", name));

        if let Some(start_line) = entity.get("start_line").and_then(|v| v.as_i64()) {
            output.push_str(&format!(" (line {})", start_line));
        }

        if let Some(signature) = entity
            .get("signature")
            .and_then(|v| v.as_str())
            .and_then(compact_signature)
        {
            output.push_str(&format!(": `{signature}`"));
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

        if let Some(preview) = entity
            .get("docstring")
            .and_then(|v| v.as_str())
            .map(doc_preview)
            .filter(|preview| !preview.is_empty())
        {
            output.push_str(&format!("  - Doc: {preview}\n"));
        }
    }

    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_format_file_entities_empty() {
        // No candidates: the honest not-found wording, not a claim that the
        // file exists but is empty.
        let result = json!({
            "entities": [],
            "outgoing_references": [],
            "file_matched": false
        });
        let formatted = format_file_entities("src/main.java", &result, false);
        assert!(
            formatted.contains("No indexed file matched `src/main.java`"),
            "got: {formatted}"
        );
        assert!(formatted.contains("may not exist, may not be indexed"));
        assert!(!formatted.contains("Did you mean"));
    }

    #[test]
    fn test_format_file_entities_miss_with_candidates_says_so() {
        let result = json!({
            "entities": [],
            "outgoing_references": [],
            "file_matched": false,
            "ambiguous_path_candidates": [
                {"file_path": "src/api/main.java", "repo_name": "app"}
            ]
        });
        let formatted = format_file_entities("main.java", &result, false);
        assert!(
            formatted.contains("No indexed file matched `main.java` exactly"),
            "got: {formatted}"
        );
        assert!(formatted.contains("Did you mean"));
        assert!(formatted.contains("src/api/main.java"));
    }

    #[test]
    fn test_format_file_entities_zero_entities_with_no_candidates_is_not_found() {
        // Documented edge: a file_matched=true shape with zero entities AND
        // zero candidates cannot come from run_explore_file (a file is in
        // the index iff it carries entities), and prose-wise the formatter
        // must keep the not-found wording rather than claim existence.
        let result = json!({
            "entities": [],
            "outgoing_references": [],
            "file_matched": true
        });
        let formatted = format_file_entities("src/empty.rs", &result, false);
        assert!(
            formatted.contains("No indexed file matched `src/empty.rs`"),
            "got: {formatted}"
        );
        assert!(!formatted.contains("Did you mean"));
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
        let formatted = format_file_entities("src/main.java", &entities, false);
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
        let formatted = format_file_entities("src/main.java", &entities, false);
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
        let formatted = format_file_entities("src/main.java", &entities, false);
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

    /// Issue #50: the preview is the first sentence, read across line
    /// breaks, so a multi-line JSDoc never stops mid-sentence.
    #[test]
    fn test_format_entity_summary_with_docstring() {
        let entity = json!({
            "name": "myMethod",
            "kind": "method",
            "docstring": "Resets the cart, e.g. after\ncheckout. Second sentence.\n@param x the x"
        });
        let formatted = format_entity_summary(&entity);
        assert!(formatted.contains("myMethod"));
        assert!(formatted.contains("  - Doc: Resets the cart, e.g. after checkout.\n"));
        assert!(!formatted.contains("Second sentence"));
        assert!(!formatted.contains("@param"));
    }

    #[test]
    fn test_doc_preview_caps_long_sentences() {
        let preview = doc_preview(&"word ".repeat(100));
        assert!(preview.ends_with('…'));
        assert!(preview.chars().count() <= DOC_PREVIEW_MAX_CHARS + 1);
    }

    /// Issue #50: the signature sits on the entity line, on one line; an
    /// empty `()` signature is dropped.
    #[test]
    fn test_format_entity_summary_inlines_compact_signature() {
        let entity = json!({
            "name": "resolve",
            "start_line": 7,
            "signature": "(\n    a: A,\n    b: B,\n)"
        });
        assert_eq!(
            format_entity_summary(&entity),
            "- **`resolve`** (line 7): `(a: A, b: B)`\n"
        );
        let entity = json!({"name": "windowId", "start_line": 9, "signature": "()"});
        assert_eq!(
            format_entity_summary(&entity),
            "- **`windowId`** (line 9)\n"
        );
    }

    /// Issue #50: referenced types are grouped by their declaring file,
    /// names only.
    #[test]
    fn test_outgoing_references_grouped_by_file() {
        let refs = vec![
            json!({"name": "Mouse", "kind": "class", "file_path": "src/Input.ts", "line": 3}),
            json!({"name": "Binding", "kind": "class", "file_path": "src/Binding.ts", "line": 1}),
            json!({"name": "Keyboard", "kind": "class", "file_path": "src/Input.ts", "line": 9}),
            json!({"name": "Mouse", "kind": "class", "file_path": "src/Input.ts", "line": 3}),
            json!({"name": "Unknown", "kind": "class"}),
        ];
        let mut output = String::new();
        append_outgoing_references(&mut output, &refs);
        assert_eq!(
            output,
            "## Imports / Referenced Types\n\n- `src/Input.ts`: Mouse, Keyboard\n- `src/Binding.ts`: Binding\n- Unknown\n\n"
        );
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
        let formatted = format_file_entities("src/main.java", &entities, false);
        // Unknown kinds fall into the "Other Entities" bucket so they remain visible.
        assert!(formatted.contains("UnknownEntity"));
        assert!(formatted.contains("Found 1 entity/entities"));
        assert!(formatted.contains("## Other Entities"));
    }

    #[test]
    fn test_format_file_entities_markup_summarized_and_opt_in() {
        let entities = json!([
            {"name": "myFunc", "kind": "function"},
            {"name": "myConst", "kind": "constant"},
            {"name": "btn-primary", "kind": "html_class"},
            {"name": "btn-primary", "kind": "html_class"},
            {"name": "container", "kind": "html_class"},
            {"name": "my-id", "kind": "html_id"}
        ]);

        // Default summary (include_markup = false)
        let formatted_default = format_file_entities("src/Component.tsx", &entities, false);
        assert!(
            formatted_default
                .contains("Found 2 code entity/entities (4 markup references summarized below)")
        );
        assert!(formatted_default.contains("## Functions"));
        assert!(formatted_default.contains("myFunc"));
        assert!(formatted_default.contains("## Constants"));
        assert!(formatted_default.contains("myConst"));
        assert!(!formatted_default.contains("## Other Entities"));
        assert!(!formatted_default.contains("- **`btn-primary`**"));
        assert!(formatted_default.contains("## Markup References (not code)"));
        assert!(formatted_default.contains("4 markup references, 3 distinct"));
        assert!(formatted_default.contains("`btn-primary` ×2"));
        assert!(formatted_default.contains("Pass `--include-markup`"));

        // Full list (include_markup = true)
        let formatted_full = format_file_entities("src/Component.tsx", &entities, true);
        assert!(formatted_full.contains("- **`btn-primary`** (html_class) ×2"));
        assert!(formatted_full.contains("- **`container`** (html_class) ×1"));
        assert!(formatted_full.contains("- **`my-id`** (html_id) ×1"));
    }

    #[test]
    fn test_format_file_entities_declarative_css_and_html_elements_kept_in_code() {
        let entities = json!([
            {"name": "app-root", "kind": "html_element", "start_line": 5},
            {"name": "main-header", "kind": "css_class", "start_line": 12},
            {"name": "theme-dark", "kind": "css_id", "start_line": 25}
        ]);
        let formatted = format_file_entities("src/styles.css", &entities, false);
        assert!(formatted.contains("Found 3 entity/entities"));
        assert!(formatted.contains("## HTML Elements"));
        assert!(formatted.contains("app-root"));
        assert!(formatted.contains("## CSS Selectors"));
        assert!(formatted.contains("main-header"));
        assert!(formatted.contains("theme-dark"));
        assert!(!formatted.contains("Markup References"));
    }

    #[test]
    fn test_format_file_entities_displays_file_path() {
        let entities = json!([
            {"name": "MyClass", "kind": "class"}
        ]);
        let formatted = format_file_entities("src/main/java/MyClass.java", &entities, false);
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

    #[test]
    fn test_all_wire_kinds_covered_by_buckets_or_markup() {
        let mut missing = Vec::new();
        for kind in crate::models::EntityKind::all_wire_kinds() {
            let in_bucket = KIND_BUCKETS.iter().any(|b| b.kinds.contains(&kind));
            let in_markup = is_markup_kind(kind);
            if !in_bucket && !in_markup {
                missing.push(kind);
            }
        }
        assert!(
            missing.is_empty(),
            "Every EntityKind wire label must be in KIND_BUCKETS or MARKUP_KINDS; unmapped: {missing:?}"
        );
    }

    #[test]
    fn test_all_bucket_kinds_exist_in_entity_kind_wire_labels() {
        let wire_labels: Vec<&str> = crate::models::EntityKind::all_wire_kinds().collect();
        for bucket in KIND_BUCKETS {
            for kind in bucket.kinds {
                assert!(
                    wire_labels.contains(kind),
                    "KIND_BUCKETS entry '{}' points to unknown kind '{}'",
                    bucket.header,
                    kind
                );
            }
        }
    }
}
