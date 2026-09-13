//! Kind-aware re-ranking for `search_hybrid_context`.
//!
//! Vector similarity alone lets long natural-language payloads (Markdown
//! sections, test files, config keys, build dependencies) outrank the code
//! definitions a natural-language query is actually looking for: prose
//! embeds are long and lexical while a function's embed is a short
//! `[kind] name + signature` header.
//!
//! This module applies a deterministic, language-agnostic re-rank on top of
//! the cosine score returned by Qdrant:
//!
//! ```text
//! final = cosine + kind_boost + lexical_boost + test_path_penalty
//! ```
//!
//! Everything is query-time: no re-indexing is required, every input is
//! already present in the Qdrant payload (`kind`, `file_path`, `name`) or is
//! the query itself. Ties break on `(file_path, start_line, uuid)` so the
//! order is total and reproducible.
//!
//! Penalties are deliberately moderate: a documentation-only topic (no
//! competing definition) must still surface its best Markdown section, so
//! prose/test penalties offset the systematic cosine advantage of
//! natural-language payloads without burying them.

use std::collections::HashSet;

/// Boost for definition kinds that describe behavior (functions, methods,
/// constructors and their per-language twins). Slightly above [`TYPE_BOOST`]
/// so `Type::method` can outrank its own container when a query describes
/// what the method does (`LookupMaps::build` vs `LookupMaps`).
const CALLABLE_BOOST: f32 = 0.15;

/// Boost for type-declaration kinds (classes, interfaces, structs, enums,
/// traits, records). Below [`CALLABLE_BOOST`] but far above prose.
const TYPE_BOOST: f32 = 0.10;

/// Penalty for prose kinds (Markdown documents/sections). Their embeds are
/// long natural-language payloads that dominate cosine similarity for
/// natural-language queries. Kept moderate so documentation-only topics
/// (no competing definition) still surface their best section.
const PROSE_PENALTY: f32 = -0.20;

/// Penalty for configuration and build-system entities. They are rarely what
/// a behavioral query is looking for, but must stay reachable (config/build
/// E2E suites search for them directly).
const CONFIG_BUILD_PENALTY: f32 = -0.20;

/// Penalty for entities living under test paths. Test names read like
/// natural language ("login with bad password fails") and win cosine
/// against the production definition for the same behavior. The penalty
/// must exceed that systematic cosine advantage for realistic gaps.
const TEST_PATH_PENALTY: f32 = -0.20;

/// Boost when the entity's whole name equals a query token
/// (`build` in "build lookup maps for reference resolution"). Strong enough
/// to flip a method above its own container even on a sizable cosine gap,
/// and to promote a name-probe hit whose cosine is deep but whose
/// identifier the query literally named.
const EXACT_NAME_BOOST: f32 = 0.30;

/// Boost per top semantic root a candidate CALLS (see
/// [`CALLER_ROOT_MAX`] and [`is_test_path`]). Graph evidence: a natural
/// language query ranks the helpers of the behaviour it names, and the
/// production entry point is usually their shared caller (`login` calls
/// `normalize_email`, `verify_credentials_or_fail`, `generate_token` in
/// job-watch). Applied only to callables above the test-path penalty — a
/// test calling the same helpers must NOT climb back via call provenance.
const CALLER_ROOT_BOOST: f32 = 0.12;

/// Maximum number of root links counted for [`CALLER_ROOT_BOOST`].
const CALLER_ROOT_MAX: usize = 3;

/// Weight for partial token overlap between the query and the identifier,
/// scaled by the matched-token ratio.
const TOKEN_OVERLAP_WEIGHT: f32 = 0.08;

/// Wire-format (`EntityKind::Display`) strings of definition kinds that
/// describe behavior. Must stay in lockstep with `src/models/entity.rs`.
const CALLABLE_KINDS: &[&str] = &[
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
const TYPE_KINDS: &[&str] = &[
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
const PROSE_KINDS: &[&str] = &["markdown_section", "markdown_document"];

/// Configuration and build-system kinds.
const CONFIG_BUILD_KINDS: &[&str] = &[
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

/// Query words that carry no identifier meaning. Deliberately small: words
/// like `get` or `user` are meaningful identifier fragments.
const STOPWORDS: &[&str] = &[
    "a", "an", "the", "for", "with", "from", "to", "of", "in", "on", "and", "or", "is", "are",
    "be", "by", "at", "as", "it", "its", "this", "that", "how", "do", "does", "what", "when",
    "where", "which", "my", "me", "i", "you", "your", "we", "our", "their", "his", "her",
];

/// Kind boost for a wire-format kind string. Unknown kinds (HTML, CSS ids,
/// VTC fixtures, …) score neutral so the cosine order between them is kept.
pub fn kind_boost(kind: &str) -> f32 {
    if CALLABLE_KINDS.contains(&kind) {
        CALLABLE_BOOST
    } else if TYPE_KINDS.contains(&kind) {
        TYPE_BOOST
    } else if PROSE_KINDS.contains(&kind) {
        PROSE_PENALTY
    } else if CONFIG_BUILD_KINDS.contains(&kind) {
        CONFIG_BUILD_PENALTY
    } else {
        0.0
    }
}

/// Whether `file_path` points into test code.
///
/// Matched by path segments and file-name conventions only — deliberately
/// conservative so words merely *containing* "test" (`latest.rs`,
/// `contest.rs`) never match.
pub fn is_test_path(file_path: &str) -> bool {
    let normalized = file_path.replace('\\', "/");
    let lower = normalized.to_lowercase();

    // Directory conventions (leading slash added so bare `tests/…` matches).
    let padded = format!("/{lower}");
    for marker in ["/test/", "/tests/", "/__tests__/", "/spec/"] {
        if padded.contains(marker) {
            return true;
        }
    }

    let file_name = normalized.rsplit('/').next().unwrap_or(&normalized);
    let lower_name = file_name.to_lowercase();
    // Suffix conventions: `foo_test.rs`, `foo.test.ts`, `foo.spec.ts`.
    if lower_name.contains("_test.")
        || lower_name.contains(".test.")
        || lower_name.contains(".spec.")
    {
        return true;
    }
    // `test_foo.rs` prefix convention.
    if lower_name.starts_with("test_") {
        return true;
    }
    // Java/TS class-per-file convention: `FooTest.java`, `UserTests.ts`.
    // Case-sensitive on purpose: a lowercase stem ending in "test"
    // (`latest.rs`) is not a test file.
    let stem = file_name.split('.').next().unwrap_or(file_name);
    stem.ends_with("Test") || stem.ends_with("Tests")
}

/// Split an identifier into lowercase tokens at snake_case, kebab-case and
/// camelCase boundaries. Delegates to the shared tokenizer
/// ([`crate::utils::identifiers::identifier_tokens`]) so the ranker and the
/// embed-text builder always agree on token boundaries.
fn identifier_tokens(name: &str) -> Vec<String> {
    crate::utils::identifiers::identifier_tokens(name)
}

/// Lowercase alphanumeric query tokens, minus stopwords and single chars.
pub(crate) fn query_tokens(query: &str) -> Vec<String> {
    query
        .split(|c: char| !c.is_alphanumeric())
        .map(str::to_lowercase)
        .filter(|t| t.len() >= 2 && !STOPWORDS.contains(&t.as_str()))
        .collect()
}

/// Significant query tokens for the name-exact probe: identifier-like
/// tokens (≥ 3 chars) that may literally name an entity. Deduplicated in
/// query order and capped to keep the probe to a single Qdrant round-trip.
pub fn significant_query_tokens(query: &str) -> Vec<String> {
    let mut seen: HashSet<String> = HashSet::new();
    query_tokens(query)
        .into_iter()
        .filter(|t| t.len() >= 3)
        .filter(|t| seen.insert(t.clone()))
        .take(4)
        .collect()
}

/// Case variants for the name-exact probe: each token in lowercase plus its
/// PascalCase form, covering snake_case functions and class-style names
/// (`build` → build, Build; `similaritysearch` → similaritysearch,
/// Similaritysearch).
pub fn probe_name_variants(tokens: &[String]) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    for token in tokens {
        let mut pascal = token.clone();
        if let Some(first) = pascal.get_mut(0..1) {
            first.make_ascii_uppercase();
        }
        for candidate in [token.clone(), pascal] {
            if !names.contains(&candidate) {
                names.push(candidate);
            }
        }
    }
    names
}

/// Boost derived from the caller-recall bridge: how many of the top
/// semantic roots a candidate calls (annotated by the search pipeline in
/// the `caller_roots` field; absent → zero). Production callables only —
/// a candidate under a test path must not reclaim the test-path penalty
/// through call provenance.
fn caller_root_boost(kind: &str, file_path: &str, caller_roots: Option<i64>) -> f32 {
    if kind_boost(kind) < 0.0 || is_test_path(file_path) {
        return 0.0;
    }
    match caller_roots {
        Some(c) => c.clamp(0, CALLER_ROOT_MAX as i64) as f32 * CALLER_ROOT_BOOST,
        None => 0.0,
    }
}

/// Lexical agreement between the query and the entity name.
///
/// An exact whole-name hit against a query token earns [`EXACT_NAME_BOOST`];
/// otherwise partial token overlap earns
/// `TOKEN_OVERLAP_WEIGHT * matched/query_tokens`.
pub fn lexical_boost(query: &str, name: &str) -> f32 {
    let q_tokens = query_tokens(query);
    if q_tokens.is_empty() {
        return 0.0;
    }
    if q_tokens.iter().any(|q| *q == name.to_lowercase()) {
        return EXACT_NAME_BOOST;
    }
    let n_tokens = identifier_tokens(name);
    let matched = n_tokens.iter().filter(|t| q_tokens.contains(t)).count();
    if matched == 0 {
        return 0.0;
    }
    TOKEN_OVERLAP_WEIGHT * (matched as f32 / q_tokens.len() as f32)
}

/// Final ranking score for one candidate.
pub fn final_score(cosine: f32, kind: &str, file_path: &str, query: &str, name: &str) -> f32 {
    let mut score = cosine + kind_boost(kind) + lexical_boost(query, name);
    if is_test_path(file_path) {
        score += TEST_PATH_PENALTY;
    }
    score
}

/// Re-rank candidate entities by their final score, ties broken
/// deterministically on `(file_path, start_line, uuid)`.
///
/// Candidates missing a `score` field (e.g. graph-side prefix hits that were
/// merged before ranking) are not expected here; they score cosine `0.0` and
/// sink below scored hits, which keeps the name-match contract intact —
/// callers prepend those separately.
pub fn rerank(entities: Vec<serde_json::Value>, query: &str) -> Vec<serde_json::Value> {
    let mut scored: Vec<(f32, String, i64, String, serde_json::Value)> = entities
        .into_iter()
        .map(|entity| {
            let kind = entity.get("kind").and_then(|v| v.as_str()).unwrap_or("");
            let file_path = entity
                .get("file_path")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let name = entity.get("name").and_then(|v| v.as_str()).unwrap_or("");
            let start_line = entity
                .get("start_line")
                .and_then(|v| v.as_i64())
                .unwrap_or(0);
            let uuid = entity
                .get("uuid")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let cosine = entity.get("score").and_then(|v| v.as_f64()).unwrap_or(0.0) as f32;
            let caller_roots = entity.get("caller_roots").and_then(|v| v.as_i64());
            let score = final_score(cosine, kind, &file_path, query, name)
                + caller_root_boost(kind, &file_path, caller_roots);
            (score, file_path, start_line, uuid, entity)
        })
        .collect();

    scored.sort_by(|a, b| {
        b.0.partial_cmp(&a.0)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.1.cmp(&b.1))
            .then_with(|| a.2.cmp(&b.2))
            .then_with(|| a.3.cmp(&b.3))
    });

    scored
        .into_iter()
        .map(|(_, _, _, _, entity)| entity)
        .collect()
}

/// Expand a user-supplied kind filter into concrete wire-format kinds.
///
/// Accepted aliases (case-insensitive, comma-separated input):
/// - `definition` — every callable and type kind
/// - `callable` / `function` / `method` — every callable kind
/// - `class` / `type` / `struct` — every type kind
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

/// Whether an entity passes the search's optional path filter.
///
/// Delegates to the shared matcher ([`crate::cli_tools::list_files`]):
/// empty/`.` keeps everything, a directory prefix must match on a `/`
/// boundary (`src/api` never matches `src/api-notes.md`), and `*`/`?`/`**`
/// switch to glob matching. Applies to name-prefix hits and vector hits
/// alike so CLI and MCP behave identically.
pub fn path_allows(pattern: Option<&str>, file_path: &str) -> bool {
    match pattern {
        None => true,
        Some(p) => crate::cli_tools::list_files::path_matches(p, file_path),
    }
}

/// Over-fetch window for the vector search: wide enough that a definition
/// sitting below prose in pure cosine order still enters the candidate set
/// and can be promoted by the re-rank.
///
/// The upper clamp scales with the module's result ceiling
/// ([`super::MAX_RESULTS_CEILING`]) so a request at the ceiling keeps a
/// coherent cosine pool (4× the result count) instead of being padded from
/// the recall channels. Unchanged for every `max_results <= 20`, where the
/// legacy `80` ceiling never bound.
pub fn candidate_limit(max_results: usize) -> usize {
    (max_results * 4).clamp(24, super::MAX_RESULTS_CEILING * 4)
}

/// Deduplicate enriched entities by identity.
///
/// Primary key is the entity UUID (the Qdrant↔Neo4j bridge); as a defensive
/// second pass, rows sharing `(repo_name, fqn, start_line)` collapse to the
/// first occurrence — the UUID identity is exactly
/// `repo:file:fqn:start_line`, so this can only merge the same entity.
/// Rows without a usable FQN keep their (already unique) UUID as key.
pub fn dedup_by_identity(entities: Vec<serde_json::Value>) -> Vec<serde_json::Value> {
    let mut seen: HashSet<String> = HashSet::new();
    entities
        .into_iter()
        .filter(|entity| {
            let uuid = entity
                .get("uuid")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let fqn = entity
                .get("fqn")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let repo = entity
                .get("repo_name")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let line = entity
                .get("start_line")
                .and_then(|v| v.as_i64())
                .unwrap_or(0);
            let key = if fqn.is_empty() {
                uuid
            } else {
                format!("{repo}\u{0}{fqn}\u{0}{line}")
            };
            seen.insert(key)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // --- caller_root_boost ---

    #[test]
    fn caller_root_boost_counts_capped() {
        assert_eq!(caller_root_boost("rust_function", "src/a.rs", None), 0.0);
        assert_eq!(caller_root_boost("function", "src/a.rs", Some(0)), 0.0);
        assert_eq!(
            caller_root_boost("function", "src/a.rs", Some(1)),
            CALLER_ROOT_BOOST
        );
        assert_eq!(
            caller_root_boost("rust_method", "src/a.rs", Some(2)),
            2.0 * CALLER_ROOT_BOOST
        );
        // A candidate calling four top roots still earns three.
        assert_eq!(
            caller_root_boost("function", "src/a.rs", Some(4)),
            3.0 * CALLER_ROOT_BOOST
        );
    }

    #[test]
    fn caller_root_boost_never_rescues_tests_or_prose() {
        // Test-path candidates must not reclaim the test-path penalty
        // through call provenance.
        assert_eq!(
            caller_root_boost("rust_function", "tests/login_test.rs", Some(3)),
            0.0
        );
        // Prose roots have no callers, but guard the kind gate anyway.
        assert_eq!(
            caller_root_boost("markdown_section", "docs/a.md", Some(3)),
            0.0
        );
    }

    #[test]
    fn rerank_promotes_shared_caller_of_top_root() {
        // Bug scenario: the doc-less definition shares the cosine band with
        // the helpers a paraphrase ranks first, but it is their caller. The
        // caller-root boost must lift it into the leading slots.
        let candidates = vec![
            json!({"uuid": "1", "name": "normalize_email", "kind": "rust_function",
                   "file_path": "src/auth/credentials.rs", "start_line": 71, "score": 0.319}),
            json!({"uuid": "2", "name": "verify_password", "kind": "rust_function",
                   "file_path": "src/auth/credentials.rs", "start_line": 41, "score": 0.302}),
            json!({"uuid": "3", "name": "login", "kind": "rust_function",
                   "file_path": "src/api/auth.rs", "start_line": 136, "score": 0.241,
                   "caller_roots": 2}),
            json!({"uuid": "4", "name": "test_login_success", "kind": "rust_function",
                   "file_path": "tests/login_test.rs", "start_line": 10, "score": 0.24,
                   "caller_roots": 2}),
            json!({"uuid": "5", "name": "create_user", "kind": "rust_function",
                   "file_path": "src/api/admin.rs", "start_line": 12, "score": 0.26,
                   "caller_roots": 1}),
        ];
        let ranked = rerank(candidates, "authenticate user with email and password");
        let names: Vec<&str> = ranked
            .iter()
            .filter_map(|e| e.get("name").and_then(|v| v.as_str()))
            .collect();
        let login = names.iter().position(|n| *n == "login").unwrap();
        assert!(
            login <= 2,
            "login must enter the leading slots, got order {names:?}"
        );
        // Tests never climb via call provenance.
        let test = names
            .iter()
            .position(|n| *n == "test_login_success")
            .unwrap();
        assert!(test > login, "test files stay below production callables");
    }

    // --- kind_boost ---

    #[test]
    fn kind_boost_callables_outrank_types() {
        assert!(kind_boost("function") > kind_boost("class"));
        assert!(kind_boost("rust_method") > kind_boost("rust_struct"));
        assert_eq!(kind_boost("java_method_placeholder_unused"), 0.0);
    }

    #[test]
    fn kind_boost_covers_one_kind_per_language() {
        // One callable and one type per supported language family.
        for callable in [
            "method",
            "kotlin_function",
            "rust_function",
            "python_method",
            "c_function",
            "cpp_method",
            "csharp_method",
            "groovy_method",
        ] {
            assert_eq!(kind_boost(callable), CALLABLE_BOOST, "callable {callable}");
        }
        for ty in [
            "class",
            "kotlin_class",
            "rust_struct",
            "python_class",
            "cpp_class",
            "groovy_class",
            "csharp_class",
        ] {
            assert_eq!(kind_boost(ty), TYPE_BOOST, "type {ty}");
        }
    }

    #[test]
    fn kind_boost_penalizes_prose_and_config() {
        assert_eq!(kind_boost("markdown_section"), PROSE_PENALTY);
        assert_eq!(kind_boost("markdown_document"), PROSE_PENALTY);
        assert_eq!(kind_boost("config_property"), CONFIG_BUILD_PENALTY);
        assert_eq!(kind_boost("build_dependency"), CONFIG_BUILD_PENALTY);
        assert_eq!(kind_boost("cargo_package"), CONFIG_BUILD_PENALTY);
        assert_eq!(kind_boost("project_identity"), CONFIG_BUILD_PENALTY);
        // Neutral kinds keep cosine order.
        assert_eq!(kind_boost("constant"), 0.0);
        assert_eq!(kind_boost("css_class"), 0.0);
        assert_eq!(kind_boost("rust_impl"), 0.0);
        assert_eq!(kind_boost("something_new"), 0.0);
    }

    // --- is_test_path ---

    #[test]
    fn test_path_positives() {
        for path in [
            "tests/login_test.rs",
            "src/tests/login.rs",
            "src/test/foo.java",
            "src/__tests__/login.ts",
            "spec/login_spec.rb",
            "tests/foo.rs",
            "login_test.rs",
            "login.test.ts",
            "login.spec.ts",
            "test_login.rs",
            "LoginTest.java",
            "UserServiceTests.java",
        ] {
            assert!(is_test_path(path), "expected test path: {path}");
        }
    }

    #[test]
    fn test_path_negatives_do_not_substring_match() {
        for path in [
            "src/latest.rs",
            "src/contest.rs",
            "src/testing_support.rs",
            "src/login.rs",
            "attestations.rs",
        ] {
            assert!(!is_test_path(path), "expected non-test path: {path}");
        }
    }

    // --- lexical_boost ---

    #[test]
    fn lexical_exact_name_hit() {
        assert_eq!(
            lexical_boost("build lookup maps for reference resolution", "build"),
            EXACT_NAME_BOOST
        );
    }

    #[test]
    fn lexical_partial_overlap_ratio() {
        // similarity + search match 2 of 5 query tokens.
        let boost = lexical_boost(
            "vector store similarity search implementation",
            "similaritySearch",
        );
        assert!((boost - TOKEN_OVERLAP_WEIGHT * (2.0 / 5.0)).abs() < 1e-6);
    }

    #[test]
    fn lexical_snake_case_splits() {
        let boost = lexical_boost(
            "link cross repository build dependencies",
            "link_cross_repo_dependencies",
        );
        // link, cross, dependencies match 3 of 5 tokens (repo != repository).
        assert!((boost - TOKEN_OVERLAP_WEIGHT * (3.0 / 5.0)).abs() < 1e-6);
    }

    #[test]
    fn lexical_no_overlap_is_zero() {
        assert_eq!(
            lexical_boost("authenticate user with email and password", "parse_config"),
            0.0
        );
    }

    #[test]
    fn lexical_stopwords_and_single_chars_ignored() {
        // Only "build" survives as a meaningful token.
        assert_eq!(
            lexical_boost("a build of the thing", "build"),
            EXACT_NAME_BOOST
        );
        assert_eq!(lexical_boost("a to i", "a"), 0.0);
    }

    // --- final_score / rerank ---

    #[test]
    fn rerank_promotes_definition_over_prose_test_and_helper() {
        // The bug report scenario: Markdown, a test file, a helper and a
        // caller all out-cosine the definition; the re-rank must put `login`
        // first anyway.
        let candidates = vec![
            json!({"uuid": "1", "name": "setup", "kind": "markdown_section",
                   "file_path": "docs/AUTH.md", "start_line": 1, "score": 0.82}),
            json!({"uuid": "2", "name": "test_login_success", "kind": "rust_function",
                   "file_path": "tests/login_test.rs", "start_line": 10, "score": 0.75}),
            json!({"uuid": "3", "name": "handle_login", "kind": "rust_function",
                   "file_path": "src/routes.rs", "start_line": 20, "score": 0.58}),
            json!({"uuid": "4", "name": "normalize_email", "kind": "rust_function",
                   "file_path": "src/util.rs", "start_line": 5, "score": 0.60}),
            json!({"uuid": "5", "name": "login", "kind": "rust_function",
                   "file_path": "src/auth.rs", "start_line": 30, "score": 0.62}),
        ];
        let ranked = rerank(candidates, "authenticate user with email and password");
        assert_eq!(ranked[0]["name"], "login", "definition must rank first");
        // Prose and the test file must fall below every production callable.
        let names: Vec<&str> = ranked
            .iter()
            .filter_map(|e| e.get("name").and_then(|v| v.as_str()))
            .collect();
        let login_pos = names.iter().position(|n| *n == "login").unwrap();
        let setup_pos = names.iter().position(|n| *n == "setup").unwrap();
        let test_pos = names
            .iter()
            .position(|n| *n == "test_login_success")
            .unwrap();
        assert!(login_pos < setup_pos && login_pos < test_pos);
    }

    #[test]
    fn rerank_recovers_definition_buried_far_below_prose() {
        // Extreme case from the report: the definition's cosine is so low it
        // was absent from the top-8; the kind/test/prose adjustments alone
        // must still lift it above Markdown and the test file.
        let candidates = vec![
            json!({"uuid": "1", "name": "setup", "kind": "markdown_section",
                   "file_path": "docs/AUTH.md", "start_line": 1, "score": 0.82}),
            json!({"uuid": "2", "name": "test_login_success", "kind": "rust_function",
                   "file_path": "tests/login_test.rs", "start_line": 10, "score": 0.70}),
            json!({"uuid": "3", "name": "login", "kind": "rust_function",
                   "file_path": "src/auth.rs", "start_line": 30, "score": 0.52}),
        ];
        let ranked = rerank(candidates, "authenticate user with email and password");
        assert_eq!(ranked[0]["name"], "login");
        assert_eq!(ranked[1]["name"], "test_login_success");
        assert_eq!(ranked[2]["name"], "setup");
    }

    #[test]
    fn rerank_lets_method_beat_its_own_container() {
        // `LookupMaps::build` (callable, lower cosine) vs the `LookupMaps`
        // struct (type, higher cosine): exact name token + callable boost
        // must flip the order.
        let candidates = vec![
            json!({"uuid": "1", "name": "LookupMaps", "kind": "rust_struct",
                   "file_path": "src/resolve/mod.rs", "start_line": 131, "score": 0.80}),
            json!({"uuid": "2", "name": "build", "kind": "rust_method",
                   "file_path": "src/resolve/mod.rs", "start_line": 144, "score": 0.58}),
        ];
        let ranked = rerank(candidates, "build lookup maps for reference resolution");
        assert_eq!(ranked[0]["name"], "build");
    }

    #[test]
    fn rerank_preserves_cosine_order_within_same_bucket() {
        // HikariCP guardrail: two same-kind hits keep their cosine order.
        let candidates = vec![
            json!({"uuid": "1", "name": "evictConnection", "kind": "method",
                   "file_path": "src/Pool.java", "start_line": 10, "score": 0.91}),
            json!({"uuid": "2", "name": "shutdown", "kind": "method",
                   "file_path": "src/Pool.java", "start_line": 40, "score": 0.87}),
        ];
        let ranked = rerank(candidates, "evict a connection from the pool");
        assert_eq!(ranked[0]["name"], "evictConnection");
        assert_eq!(ranked[1]["name"], "shutdown");
    }

    #[test]
    fn rerank_is_deterministic_on_full_ties() {
        let a = json!({"uuid": "aaa", "name": "foo", "kind": "function",
                       "file_path": "src/a.rs", "start_line": 1, "score": 0.5});
        let b = json!({"uuid": "bbb", "name": "foo", "kind": "function",
                       "file_path": "src/a.rs", "start_line": 1, "score": 0.5});
        let ranked = rerank(vec![b.clone(), a.clone()], "foo");
        assert_eq!(ranked[0]["uuid"], "aaa");
        assert_eq!(ranked[1]["uuid"], "bbb");
    }

    #[test]
    fn rerank_handles_missing_fields_without_panic() {
        let ranked = rerank(
            vec![json!({"name": "mystery"}), json!({"uuid": "u"})],
            "query",
        );
        assert_eq!(ranked.len(), 2);
    }

    // --- kinds ---

    #[test]
    fn expand_kinds_definition_includes_callables_and_types() {
        let expanded = expand_kinds(&["definition"]);
        assert!(expanded.contains(&"rust_function".to_string()));
        assert!(expanded.contains(&"method".to_string()));
        assert!(expanded.contains(&"class".to_string()));
        assert!(expanded.contains(&"rust_struct".to_string()));
        assert!(!expanded.contains(&"markdown_section".to_string()));
        assert!(!expanded.contains(&"config_property".to_string()));
    }

    #[test]
    fn expand_kinds_alias_and_exact_mix_dedup() {
        let expanded = expand_kinds(&["callable", "rust_function", "markdown_section"]);
        // `callable` already covers rust_function; no duplicate.
        assert_eq!(expanded.iter().filter(|k| *k == "rust_function").count(), 1);
        assert!(expanded.contains(&"markdown_section".to_string()));
    }

    #[test]
    fn parse_kinds_splits_and_trims() {
        let expanded = parse_kinds(Some(" definition , markdown_section "));
        assert!(expanded.contains(&"rust_function".to_string()));
        assert!(expanded.contains(&"markdown_section".to_string()));
        assert!(parse_kinds(None).is_empty());
        assert!(parse_kinds(Some("")).is_empty());
        assert!(parse_kinds(Some(" , ")).is_empty());
    }

    #[test]
    fn kinds_allow_empty_filter_allows_all() {
        assert!(kinds_allow(&[], "markdown_section"));
        let expanded = expand_kinds(&["definition"]);
        assert!(kinds_allow(&expanded, "rust_function"));
        assert!(!kinds_allow(&expanded, "markdown_section"));
    }

    // --- candidate_limit ---

    #[test]
    fn candidate_limit_clamped_window() {
        assert_eq!(candidate_limit(1), 24);
        assert_eq!(candidate_limit(5), 24);
        assert_eq!(candidate_limit(10), 40);
        assert_eq!(candidate_limit(20), 80);
    }

    #[test]
    fn candidate_limit_is_unchanged_below_legacy_ceiling() {
        // Backward-compat pin: for every `max_results <= 20` the widened
        // scale never binds, so pre-existing search behavior is untouched.
        for n in 1..=20 {
            assert_eq!(
                candidate_limit(n),
                (n * 4).clamp(24, 80),
                "candidate_limit changed for max_results = {n}"
            );
        }
    }

    #[test]
    fn candidate_limit_scales_to_result_ceiling() {
        // At the advertised maximum the re-rank keeps a coherent cosine
        // pool (4x the result count) instead of padding from recall.
        assert_eq!(candidate_limit(crate::cli_tools::MAX_RESULTS_CEILING), 400);
    }

    // --- name-exact probe ---

    #[test]
    fn significant_query_tokens_filters_and_caps() {
        let tokens = significant_query_tokens("build lookup maps for reference resolution");
        assert_eq!(tokens, vec!["build", "lookup", "maps", "reference"]);
        // Stopwords and short tokens dropped; duplicates collapsed.
        assert_eq!(
            significant_query_tokens("a to do the the build"),
            vec!["build"]
        );
        assert!(significant_query_tokens("of to in").is_empty());
    }

    #[test]
    fn probe_name_variants_cover_snake_and_pascal() {
        let variants = probe_name_variants(&["build".to_string(), "user".to_string()]);
        assert_eq!(
            variants,
            vec![
                "build".to_string(),
                "Build".to_string(),
                "user".to_string(),
                "User".to_string()
            ]
        );
        // Empty input → empty variants.
        assert!(probe_name_variants(&[]).is_empty());
    }

    // --- dedup_by_identity ---

    #[test]
    fn dedup_keeps_first_of_duplicate_identity() {
        let entities = vec![
            json!({"uuid": "1", "name": "user", "fqn": "app::User", "repo_name": "r", "start_line": 3}),
            json!({"uuid": "2", "name": "user", "fqn": "app::User", "repo_name": "r", "start_line": 3}),
            json!({"uuid": "3", "name": "user", "fqn": "other::User", "repo_name": "r", "start_line": 9}),
        ];
        let deduped = dedup_by_identity(entities);
        assert_eq!(deduped.len(), 2);
        assert_eq!(deduped[0]["uuid"], "1");
        assert_eq!(deduped[1]["uuid"], "3");
    }

    #[test]
    fn dedup_rows_without_fqn_survive_via_uuid_key() {
        let entities = vec![
            json!({"uuid": "1", "name": "a", "repo_name": "r", "start_line": 1}),
            json!({"uuid": "2", "name": "b", "repo_name": "r", "start_line": 1}),
        ];
        assert_eq!(dedup_by_identity(entities).len(), 2);
    }
}
