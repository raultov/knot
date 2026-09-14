//! Shared entity-kind taxonomy and filtering for CLI and MCP tools.
//!
//! A single place for the wire-format kind tables (the snake_case strings
//! Neo4j stores) and the kind-filter machinery, so `search_hybrid_context`,
//! `find_callers` and future consumers cannot drift apart on what counts as
//! "code" versus "metadata".
//!
//! Wire format note: `EntityKind`'s wire form is its `Display` impl
//! (`src/models/entity.rs`), and every list here must stay in lockstep with
//! it. Unknown kinds score neutral everywhere (no filtering surprises).

// ---------------------------------------------------------------------------
// Kind tables
// ---------------------------------------------------------------------------

/// Wire-format strings of definition kinds that describe behavior. Must stay
/// in lockstep with `src/models/entity.rs`.
pub const CALLABLE_KINDS: &[&str] = &[
    // Generic (language-agnostic Display forms)
    "function",
    "method",
    // Kotlin
    "kotlin_function",
    "kotlin_method",
    // Rust
    "rust_function",
    "rust_method",
    "rust_macro_def",
    // Python
    "python_function",
    "python_method",
    // C / C++
    "c_function",
    "cpp_method",
    "macro_definition",
    // C#
    "csharp_method",
    "csharp_constructor",
    "csharp_local_function",
    "csharp_operator",
    "csharp_indexer",
    // Groovy
    "groovy_method",
    "groovy_function",
    // Stylesheets / Varnish
    "scss_function",
    "scss_mixin",
    "vcl_subroutine",
    "vcl_builtin_sub",
    "vcc_function",
    "vcc_method",
];

/// Wire-format strings of type-declaration kinds.
pub const TYPE_KINDS: &[&str] = &[
    // Generic
    "class",
    "interface",
    "enum",
    // Kotlin
    "kotlin_class",
    "kotlin_interface",
    "kotlin_object",
    "kotlin_companion_object",
    "kotlin_enum",
    // Rust
    "rust_struct",
    "rust_enum",
    "rust_union",
    "rust_trait",
    "rust_type_alias",
    // Python
    "python_class",
    // C / C++
    "c_struct",
    "cpp_class",
    // Groovy
    "groovy_class",
    "groovy_interface",
    "groovy_trait",
    "groovy_enum",
    // C#
    "csharp_class",
    "csharp_interface",
    "csharp_struct",
    "csharp_record",
    "csharp_enum",
    "csharp_delegate",
];

/// Prose kinds whose embeds are long natural-language bodies.
pub const PROSE_KINDS: &[&str] = &["markdown_section", "markdown_document"];

/// Configuration and build-system kinds.
pub const CONFIG_BUILD_KINDS: &[&str] = &[
    "config_property",
    "build_dependency",
    "build_plugin",
    "build_task",
    "pipeline_stage",
    "pipeline_step",
    "cargo_package",
    "cargo_feature",
    "workspace_member",
    "project_identity",
    "helm_value",
];

/// Kubernetes/Helm manifest kinds — infrastructure metadata, never code.
pub const K8S_HELM_KINDS: &[&str] = &[
    "k8s_deployment",
    "k8s_service",
    "k8s_configmap",
    "k8s_secret",
    "k8s_ingress",
    "k8s_namespace",
    "k8s_resource",
    "helm_chart",
    "helm_value",
    "helm_template_var",
];

/// Whether `kind` is documentation / configuration / build / infrastructure
/// metadata: indexed so it can be *searched*, but never the subject of a
/// "who calls this?" question.
///
/// Deliberately a **deny-list** (not an allow-list of code kinds): web and
/// VCL kinds (`html_id`, `css_class`, `vcl_backend`, …) are legitimate
/// reference targets of `REFERENCES_DOM`/`USES_CSS_CLASS`/`USES_BACKEND`
/// edges, and an allow-list would silently drop every future `EntityKind`.
pub fn is_non_code_kind(kind: &str) -> bool {
    PROSE_KINDS.contains(&kind)
        || CONFIG_BUILD_KINDS.contains(&kind)
        || K8S_HELM_KINDS.contains(&kind)
}

// ---------------------------------------------------------------------------
// Alias expansion (shared with search_hybrid_context's `kinds` parameter)
// ---------------------------------------------------------------------------

/// Expand a user-supplied kind filter into concrete wire-format kinds.
///
/// Accepted aliases (case-insensitive, comma-separated input):
/// - `definition` — every callable and type kind
/// - `callable` / `function` / `method` — every callable kind
/// - `class` / `type` / `struct` — every type kind
/// - `docs` / `prose` — every documentation kind
/// - `config` / `build` — every configuration, build-system and
///   infrastructure kind
/// - anything else — treated as an exact wire-format kind
///   (`rust_function`, `markdown_section`, …)
///
/// Order is preserved and duplicates removed so the generated Qdrant filter
/// is deterministic.
pub fn expand_kinds(specs: &[&str]) -> Vec<String> {
    let mut expanded: Vec<String> = Vec::new();
    for spec in specs {
        let alias = spec.trim().to_lowercase();
        if alias.is_empty() {
            continue;
        }
        let bucket: Vec<&str> = match alias.as_str() {
            "definition" | "definitions" => CALLABLE_KINDS
                .iter()
                .chain(TYPE_KINDS.iter())
                .copied()
                .collect(),
            "callable" | "callables" | "function" | "functions" | "method" | "methods" => {
                CALLABLE_KINDS.to_vec()
            }
            "class" | "classes" | "type" | "types" | "struct" | "structs" => TYPE_KINDS.to_vec(),
            "docs" | "prose" => PROSE_KINDS.to_vec(),
            "config" | "build" => CONFIG_BUILD_KINDS
                .iter()
                .chain(K8S_HELM_KINDS.iter())
                .copied()
                .collect(),
            exact => vec![exact],
        };
        for kind in bucket {
            if !expanded.contains(&kind.to_string()) {
                expanded.push(kind.to_string());
            }
        }
    }
    expanded
}

/// Parse the raw `kinds` parameter (CLI `--kinds`, MCP `kinds`) into the
/// expanded wire-format kind list. `None`/empty → empty list (no filtering).
pub fn parse_kinds(raw: Option<&str>) -> Vec<String> {
    let Some(raw) = raw else {
        return Vec::new();
    };
    let specs: Vec<&str> = raw
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();
    expand_kinds(&specs)
}

/// Whether an entity passes an (already expanded) kind filter.
/// An empty filter allows everything.
pub fn kinds_allow(expanded: &[String], kind: &str) -> bool {
    expanded.is_empty() || expanded.iter().any(|k| k == kind)
}

// ---------------------------------------------------------------------------
// KindFilter: the find_callers target-resolution filter
// ---------------------------------------------------------------------------

/// How `find_callers` target resolution scopes the entity kinds it may
/// resolve a query against.
///
/// The default is [`KindFilter::CodeOnly`]: a fuzzy query like `cargo` must
/// not present `Cargo.toml` build dependencies and Markdown sections as
/// resolved targets of an impact-analysis question. Documentation and
/// config/build metadata remain fully reachable through
/// [`KindFilter::Any`] (`kinds=all`) or an explicit allow-list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KindFilter {
    /// Default. Everything except [`is_non_code_kind`] metadata.
    CodeOnly,
    /// `kinds=all` / `kinds=*` — no kind restriction at all.
    Any,
    /// Explicit allow-list of expanded wire-format kinds.
    Only(Vec<String>),
}

impl KindFilter {
    /// Build the filter from the raw `kinds` parameter shared by CLI and MCP.
    ///
    /// `None`/empty → the code-only default. The sentinel values `all`/`*`
    /// disable filtering entirely; any other spec (alias or exact kind) is
    /// expanded into an explicit allow-list.
    pub fn parse(raw: Option<&str>) -> Self {
        let Some(raw) = raw else {
            return Self::CodeOnly;
        };
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return Self::CodeOnly;
        }
        if trimmed.eq_ignore_ascii_case("all") || trimmed == "*" {
            return Self::Any;
        }
        let expanded = parse_kinds(Some(trimmed));
        if expanded.is_empty() {
            return Self::CodeOnly;
        }
        Self::Only(expanded)
    }

    /// Whether a wire-format kind passes this filter.
    pub fn allows(&self, kind: &str) -> bool {
        match self {
            Self::Any => true,
            Self::CodeOnly => !is_non_code_kind(kind),
            Self::Only(expanded) => kinds_allow(expanded, kind),
        }
    }

    /// Stable label emitted in the `resolution.kind_filter` field so
    /// consumers can tell which filtering contract produced the target list.
    pub fn wire_label(&self) -> &'static str {
        match self {
            Self::CodeOnly => "code_default",
            Self::Any => "any",
            Self::Only(_) => "explicit",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- kind tables ----

    #[test]
    fn tables_contain_the_wire_forms_documented_in_the_readme() {
        assert!(CALLABLE_KINDS.contains(&"rust_function"));
        assert!(CALLABLE_KINDS.contains(&"rust_macro_def"));
        assert!(TYPE_KINDS.contains(&"csharp_record"));
    }

    // ---- is_non_code_kind ----

    #[test]
    fn non_code_kinds_cover_docs_config_build_and_infra() {
        for kind in [
            "markdown_section",
            "markdown_document",
            "config_property",
            "build_dependency",
            "build_plugin",
            "build_task",
            "pipeline_stage",
            "pipeline_step",
            "cargo_package",
            "cargo_feature",
            "workspace_member",
            "project_identity",
            "k8s_deployment",
            "k8s_service",
            "k8s_configmap",
            "k8s_secret",
            "k8s_ingress",
            "k8s_namespace",
            "k8s_resource",
            "helm_chart",
            "helm_value",
            "helm_template_var",
        ] {
            assert!(is_non_code_kind(kind), "{kind} must be non-code");
        }
    }

    #[test]
    fn web_and_vcl_kinds_are_code_reference_targets_not_metadata() {
        // Regression guard for the deny-list decision: these kinds are the
        // targets of REFERENCES_DOM / USES_CSS_CLASS / USES_BACKEND edges
        // and must stay reachable by default.
        for kind in [
            "html_id",
            "html_class",
            "css_class",
            "vcl_backend",
            "vcl_probe",
            "vcl_acl",
            "vcc_module",
            "vcc_object",
        ] {
            assert!(!is_non_code_kind(kind), "{kind} must stay code");
        }
    }

    #[test]
    fn code_kinds_pass_the_non_code_check() {
        for kind in [
            "function",
            "method",
            "class",
            "rust_function",
            "rust_struct",
            "csharp_record",
            "kotlin_class",
            "python_function",
            "rust_constant",
        ] {
            assert!(!is_non_code_kind(kind), "{kind} must be code");
        }
    }

    #[test]
    fn unknown_kinds_are_code_by_default() {
        // Future EntityKind variants must not vanish from resolution.
        assert!(!is_non_code_kind("brand_new_kind"));
    }

    // ---- expand_kinds aliases ----

    #[test]
    fn expand_kinds_definition_includes_callables_and_types() {
        let expanded = expand_kinds(&["definition"]);
        assert!(expanded.contains(&"function".to_string()));
        assert!(expanded.contains(&"rust_struct".to_string()));
        assert!(!expanded.contains(&"markdown_section".to_string()));
    }

    #[test]
    fn expand_kinds_alias_and_exact_mix_dedup() {
        let expanded = expand_kinds(&["callable", "rust_function", "markdown_section"]);
        assert!(expanded.contains(&"rust_function".to_string()));
        assert_eq!(
            expanded.iter().filter(|k| *k == "rust_function").count(),
            1,
            "aliases and exact kinds must deduplicate"
        );
        assert!(
            expanded.contains(&"markdown_section".to_string()),
            "markdown_section must be present in {expanded:?}"
        );
    }

    #[test]
    fn expand_kinds_docs_alias_maps_to_prose() {
        let expanded = expand_kinds(&["docs"]);
        assert_eq!(
            expanded,
            PROSE_KINDS
                .iter()
                .map(|s| s.to_string())
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn expand_kinds_config_alias_maps_to_config_build_and_infra() {
        let expanded = expand_kinds(&["config"]);
        assert!(expanded.contains(&"build_dependency".to_string()));
        assert!(expanded.contains(&"cargo_feature".to_string()));
        assert!(expanded.contains(&"k8s_deployment".to_string()));
        assert!(!expanded.contains(&"rust_function".to_string()));
    }

    // ---- KindFilter ----

    #[test]
    fn kind_filter_parse_none_is_code_only() {
        assert_eq!(KindFilter::parse(None), KindFilter::CodeOnly);
        assert_eq!(KindFilter::parse(Some("")), KindFilter::CodeOnly);
        assert_eq!(KindFilter::parse(Some("  ")), KindFilter::CodeOnly);
    }

    #[test]
    fn kind_filter_parse_all_sentinels_disable_filtering() {
        assert_eq!(KindFilter::parse(Some("all")), KindFilter::Any);
        assert_eq!(KindFilter::parse(Some("*")), KindFilter::Any);
        assert_eq!(KindFilter::parse(Some(" ALL ")), KindFilter::Any);
    }

    #[test]
    fn kind_filter_code_only_default_behaviour() {
        let filter = KindFilter::parse(None);
        assert_eq!(filter.wire_label(), "code_default");
        assert!(filter.allows("rust_function"));
        assert!(filter.allows("html_id"));
        assert!(filter.allows("vcl_backend"));
        assert!(!filter.allows("build_dependency"));
        assert!(!filter.allows("markdown_section"));
    }

    #[test]
    fn kind_filter_any_allows_everything() {
        let filter = KindFilter::parse(Some("all"));
        assert_eq!(filter.wire_label(), "any");
        assert!(filter.allows("build_dependency"));
        assert!(filter.allows("markdown_section"));
        assert!(filter.allows("rust_function"));
    }

    #[test]
    fn kind_filter_only_respects_explicit_list() {
        let filter = KindFilter::parse(Some("build_dependency,rust_function"));
        assert_eq!(filter.wire_label(), "explicit");
        assert!(filter.allows("build_dependency"));
        assert!(filter.allows("rust_function"));
        assert!(!filter.allows("markdown_section"));
        assert!(!filter.allows("kotlin_class"));
    }

    #[test]
    fn kind_filter_only_supports_aliases() {
        let filter = KindFilter::parse(Some("callable"));
        assert!(filter.allows("python_method"));
        assert!(filter.allows("rust_function"));
        assert!(!filter.allows("rust_struct"));
    }
}
