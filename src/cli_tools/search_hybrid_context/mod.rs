//! Core search_hybrid_context logic shared between CLI and MCP
//!
//! Performs a hybrid search combining:
//! 1. Prefix name match via Neo4j (exact name prefix, case-insensitive)
//! 2. Semantic search via Qdrant vector similarity (understands code meaning)
//! 3. Kind-aware re-ranking (see [`rank`] — definitions outrank prose, tests
//!    and config/build entities for natural-language queries)
//! 4. Structural expansion via Neo4j graph relationships (understands architecture)
//!
//! Graph enrichment only *annotates* the entities already returned; it never
//! injects callers or helpers as substitute results.

use serde_json::json;
use std::collections::HashSet;
use std::path::Path;
use std::sync::{Arc, Mutex};

use crate::db::{
    graph::{DEFAULT_MAX_TARGETS, GraphDb, QueryExt},
    vector::{ExactNameProbe, UuidProbe, VectorDb, VectorSearchExt},
};
use crate::models::RepoScope;
use crate::pipeline::embed::Embedder;

pub mod rank;

/// Default result count when the caller does not ask for one.
/// Must equal the `default` advertised by the MCP schema
/// ([`crate::mcp_tools::search_hybrid_context`]).
pub const DEFAULT_MAX_RESULTS: usize = 5;

/// Hard ceiling for `max_results`. Must equal the `maximum` advertised by the
/// MCP schema — the drift guard test in `mcp_tools::search_hybrid_context`
/// pins the two together. There is no cursor or pagination: past this bound,
/// callers narrow the search with `kinds` / `path` / `repo_name` or refine
/// the query instead of raising the limit.
pub const MAX_RESULTS_CEILING: usize = 100;

/// A caller-requested result count resolved against the advertised bound.
///
/// Resolved values are always clamped into `1..=MAX_RESULTS_CEILING`:
/// values within the bound pass through unchanged, zero/negative requests
/// floor at 1 (a result count of 0 would mean "return nothing at all"),
/// and requests above the ceiling clamp to it — the caller is told via
/// [`ResolvedLimit::notice`], never silently.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResolvedLimit {
    /// The enforced value to use for the search.
    pub value: usize,
    /// What the caller originally asked for.
    pub requested: usize,
}

impl ResolvedLimit {
    /// Whether the request exceeded the advertised ceiling (or floored).
    pub fn was_clamped(&self) -> bool {
        self.requested != self.value
    }

    /// Caller-facing note when the request had to be adjusted; `None` when
    /// the request was served as asked.
    pub fn notice(&self) -> Option<String> {
        if !self.was_clamped() {
            return None;
        }
        if self.requested == 0 {
            return Some("> Note: `max_results` was floored from 0 to the minimum of 1.\n".into());
        }
        Some(format!(
            "> Note: `max_results` was clamped from {requested} to the advertised maximum of {MAX_RESULTS_CEILING}. \
This tool has no pagination — narrow the search with `kinds` / `path` / `repo_name`, or refine the query.\n",
            requested = self.requested
        ))
    }
}

/// Single source of truth for the `max_results` bound — used by the shared
/// search core, by the MCP tool layer and by the CLI so the enforced value
/// can never drift from the advertised schema.
pub fn resolve_max_results(requested: usize) -> ResolvedLimit {
    ResolvedLimit {
        value: requested.clamp(1, MAX_RESULTS_CEILING),
        requested,
    }
}

/// Bundled database and embedder dependencies for [`run_search_hybrid_context`].
///
/// Holding the three shared handles in one struct keeps the public function
/// within clippy's `too_many_arguments` threshold and lets callers (CLI bin,
/// MCP tool) hand over the same handles without copying.
#[derive(Clone)]
pub struct SearchContext<'a> {
    pub vector_db: &'a Arc<VectorDb>,
    pub graph_db: &'a Arc<GraphDb>,
    pub embedder: &'a Arc<Mutex<Embedder>>,
}

/// User-supplied optional filters, bundled so the shared search entry
/// point stays within clippy's arity threshold.
pub struct SearchFilters<'a> {
    /// Comma-separated kind filter (exact wire-format kinds or aliases).
    pub kinds: Option<&'a str>,
    /// Repo-relative directory prefix or glob (`list_files` matcher).
    pub path: Option<&'a str>,
}

/// Main search function called by both CLI and MCP.
///
/// `filters` bundles the optional restrictions; absent `kinds`/`path`
/// fields mean no filtering.
pub async fn run_search_hybrid_context(
    query: &str,
    max_results: usize,
    repo: &RepoScope,
    filters: SearchFilters<'_>,
    ctx: &SearchContext<'_>,
) -> anyhow::Result<serde_json::Value> {
    // Enforce the advertised bound before anything else so no caller (CLI or
    // MCP) can pull more than `MAX_RESULTS_CEILING` rows out of the pipeline.
    let max_results = resolve_max_results(max_results).value;
    let vector = ctx
        .embedder
        .lock()
        .unwrap()
        .embed_query(query)
        .map_err(|e| anyhow::anyhow!("Embedding failed: {}", e))?;

    let repo_names = repo.filter_names();
    let expanded_kinds = rank::parse_kinds(filters.kinds);
    let normalized_path = normalized_path(filters.path);

    // Path-restricted searches oversample: a large share of the cosine
    // window may fall outside the path, so the re-rank needs a wider pool
    // to fill `max_results` from matching files alone. The oversample is
    // capped so the widened pool cannot turn into an oversized Qdrant
    // round-trip at the top of the `max_results` range.
    const PATH_POOL_CEILING: usize = 600;
    let pool_window = if normalized_path.is_some() {
        rank::candidate_limit(max_results)
            .saturating_mul(3)
            .min(PATH_POOL_CEILING)
    } else {
        rank::candidate_limit(max_results)
    };

    let search_results = ctx
        .vector_db
        .search(&vector, pool_window, &repo_names, &expanded_kinds)
        .await?;

    // Boost: prepend entities whose name matches the query as a case-insensitive prefix
    let prefix_results = ctx
        .graph_db
        .find_entities_by_name_prefix(query, &repo_names, max_results)
        .await
        .unwrap_or_else(|_| json!([]));

    let mut seen_uuids: HashSet<String> = HashSet::new();
    let mut combined: Vec<serde_json::Value> = Vec::new();

    // Prefix hits keep the leading slots (name-match contract), filtered by
    // the kind and path filters when one is set.
    merge_prefix_hits(
        &prefix_results,
        &expanded_kinds,
        normalized_path.as_deref(),
        &mut seen_uuids,
        &mut combined,
    );

    // Vector hits: dedup, name/token probe recall and the caller bridge,
    // merged into one candidate pool for the kind-aware re-rank; the path
    // filter applies to every channel when one is set.
    let pool = CandidatePool {
        vector: &vector,
        query,
        search_results: &search_results,
        seen_uuids: &mut seen_uuids,
        repo_names: &repo_names,
        expanded_kinds: &expanded_kinds,
        candidate_limit: pool_window,
        ctx,
    };
    let mut vector_hits = pool.collect().await;
    if let Some(pattern) = &normalized_path {
        vector_hits.retain(|hit| {
            rank::path_allows(
                Some(pattern),
                hit.get("file_path").and_then(|v| v.as_str()).unwrap_or(""),
            )
        });
    }

    combined.extend(rank::rerank(vector_hits, query));

    if combined.is_empty() {
        return Ok(serde_json::Value::Null);
    }

    combined.truncate(max_results);

    let uuids: Vec<String> = combined
        .iter()
        .map(|entity| {
            entity
                .get("uuid")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string()
        })
        .collect();

    let fetched = ctx
        .graph_db
        .get_entities_with_dependencies(&uuids, &repo_names)
        .await?;

    // `get_entities_with_dependencies` preserves uuid order, so the ranked
    // order survives the round-trip; drop any duplicate rows defensively.
    let context = serde_json::Value::Array(rank::dedup_by_identity(
        fetched.as_array().cloned().unwrap_or_default(),
    ));

    let entity_names: Vec<String> = context
        .as_array()
        .map(|entities| {
            entities
                .iter()
                .filter_map(|entity| {
                    entity
                        .get("name")
                        .and_then(|v| v.as_str())
                        .map(String::from)
                })
                .collect()
        })
        .unwrap_or_default();

    let enriched_context = enrich_with_relationships(&context, &entity_names, ctx.graph_db, repo)
        .await
        .unwrap_or(context);

    Ok(enriched_context)
}

/// Enrich search results with related entities (subclasses, implementers, usages).
///
/// Annotation-only: relationship data is attached to entities that are
/// already in the result list. Callers and helpers are shown as context of a
/// definition, never as substitute rows displacing it.
async fn enrich_with_relationships(
    context: &serde_json::Value,
    _entity_names: &[String],
    graph_db: &Arc<GraphDb>,
    repo: &RepoScope,
) -> anyhow::Result<serde_json::Value> {
    let mut enriched = context.clone();
    let repo_names = repo.filter_names();

    if let Some(entities) = enriched.as_array_mut() {
        for entity in entities.iter_mut() {
            if let Some(name) = entity.get("name").and_then(|v| v.as_str())
                && let Ok(references) = graph_db
                    .find_references(name, &repo_names, DEFAULT_MAX_TARGETS)
                    .await
            {
                enrich_single_entity(entity, &references);
            }
        }
    }

    Ok(enriched)
}

/// Merge the raw vector hits with the recall channels into one candidate
/// pool, deduplicated against `seen_uuids` in place:
///
/// 1. the cosine search results themselves;
/// 2. the name/token probe — identifiers the query literally names, or the
///    identifier words it shares, entering the pool with their true cosine
///    however deep their pure similarity rank (`LookupMaps::build` sits at
///    cosine rank ~500 for "build lookup maps for reference resolution");
/// 3. the caller-recall bridge — the callers of the top semantic hits.
///
/// All recall channels are best effort: a failure drops the channel, never
/// the whole search.
pub(crate) struct CandidatePool<'a> {
    /// Query embedding.
    pub vector: &'a [f32],
    /// Raw query text (feeds the name/token probe).
    pub query: &'a str,
    /// Cosine search results, in pure similarity order.
    pub search_results: &'a [serde_json::Value],
    /// UUIDs already merged (prefix hits); advanced in place.
    pub seen_uuids: &'a mut HashSet<String>,
    /// Repository scope (empty = all).
    pub repo_names: &'a [String],
    /// Wire-format entity kinds to restrict hits to (empty = all).
    pub expanded_kinds: &'a [String],
    /// Over-fetch window size (also caps the caller bridge).
    pub candidate_limit: usize,
    /// Bounded shared handles (vector + graph + embedder).
    pub ctx: &'a SearchContext<'a>,
}

impl CandidatePool<'_> {
    pub async fn collect(self) -> Vec<serde_json::Value> {
        let mut vector_hits: Vec<serde_json::Value> = Vec::new();
        for result in self.search_results {
            if let Some(uuid) = result.get("uuid").and_then(|v| v.as_str())
                && self.seen_uuids.insert(uuid.to_string())
            {
                vector_hits.push(result.clone());
            }
        }

        let probe_hits = name_probe_hits(
            self.vector,
            self.query,
            self.repo_names,
            self.expanded_kinds,
            self.ctx,
        )
        .await;
        merge_hits(&probe_hits, self.seen_uuids, &mut vector_hits);

        let bridge = CallerBridge {
            vector: self.vector,
            search_results: self.search_results,
            seen_uuids: self.seen_uuids,
            repo_names: self.repo_names,
            expanded_kinds: self.expanded_kinds,
            ctx: self.ctx,
        }
        .run(self.candidate_limit)
        .await;
        merge_hits(&bridge, self.seen_uuids, &mut vector_hits);
        vector_hits
    }
}

/// Append not-yet-seen hits to the pool, updating the seen-UUID set.
fn merge_hits(
    hits: &[serde_json::Value],
    seen_uuids: &mut HashSet<String>,
    pool: &mut Vec<serde_json::Value>,
) {
    for hit in hits {
        if let Some(uuid) = hit.get("uuid").and_then(|v| v.as_str())
            && seen_uuids.insert(uuid.to_string())
        {
            pool.push(hit.clone());
        }
    }
}

/// Normalize the caller's `path` input with the shared `explore_file`
/// resolver (`None`/empty → no restriction).
fn normalized_path(path: Option<&str>) -> Option<String> {
    path.filter(|p| !p.trim().is_empty()).map(|p| {
        crate::cli_tools::list_files::normalize_list_path(
            p,
            std::env::current_dir().ok().as_deref(),
            std::env::var("KNOT_REPO_PATH")
                .ok()
                .as_deref()
                .map(Path::new),
        )
    })
}

/// Merge name-prefix hits into the leading slots, honoring the kind and
/// path filters (name-match contract: prefix hits keep their leading
/// slots only if they pass the user's restrictions).
fn merge_prefix_hits(
    prefix_results: &serde_json::Value,
    expanded_kinds: &[String],
    normalized_path: Option<&str>,
    seen_uuids: &mut HashSet<String>,
    combined: &mut Vec<serde_json::Value>,
) {
    if let Some(arr) = prefix_results.as_array() {
        for entity in arr {
            let kind = entity.get("kind").and_then(|v| v.as_str()).unwrap_or("");
            let file_path = entity
                .get("file_path")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if rank::kinds_allow(expanded_kinds, kind)
                && rank::path_allows(normalized_path, file_path)
            {
                push_if_unique(entity, seen_uuids, combined);
            }
        }
    }
}

/// Run the name/token probe against Qdrant: entities whose name equals a
/// significant query token (lowercase + PascalCase variants, ≤ 4 tokens)
/// or whose identifier shares a query token, carrying their true cosine so
/// the re-rank can promote them. Best effort — fails to an empty list.
async fn name_probe_hits(
    vector: &[f32],
    query: &str,
    repo_names: &[String],
    expanded_kinds: &[String],
    ctx: &SearchContext<'_>,
) -> Vec<serde_json::Value> {
    let probe_tokens = rank::significant_query_tokens(query);
    if probe_tokens.is_empty() {
        return Vec::new();
    }
    let probe_names = rank::probe_name_variants(&probe_tokens);
    ctx.vector_db
        .search_exact_names(ExactNameProbe {
            vector,
            names: &probe_names,
            tokens: &probe_tokens,
            limit: 24,
            repo_names,
            kinds: expanded_kinds,
        })
        .await
        .unwrap_or_default()
}

/// Caller-recall bridge over the top semantic hits.
///
/// Roots are the highest-cosine vector hits ([`TOP_ROOTS`] capped); their
/// callers (in-repo, capped at 12) are scored against the query vector in
/// one Qdrant round-trip and returned as full candidate rows with their
/// true cosine. Skips prose and test-path roots — a markdown section has
/// no callers and a test's callers are other tests. Failures collapse to
/// an empty list; the search continues without the bridge.
pub(crate) struct CallerBridge<'a> {
    /// Query embedding.
    pub vector: &'a [f32],
    /// Vector hits in pure cosine order — the bridge seeds from the top.
    pub search_results: &'a [serde_json::Value],
    /// UUIDs already in the candidate pool (skipped by the bridge).
    pub seen_uuids: &'a HashSet<String>,
    /// Repository scope (empty = all).
    pub repo_names: &'a [String],
    /// Wire-format entity kinds to restrict hits to (empty = all).
    pub expanded_kinds: &'a [String],
    /// Bounded shared handles (vector + graph databases).
    pub ctx: &'a SearchContext<'a>,
}

impl CallerBridge<'_> {
    /// Cap by the remaining pool capacity so the bridge can never displace
    /// what the direct search already gathered. Each fetched hit is
    /// annotated with `caller_roots` — how many of the top semantic roots
    /// it calls — so [`rank::final_score_annotated`] can add the root
    /// boost.
    pub async fn run(&self, pool_cap: usize) -> Vec<serde_json::Value> {
        let top_roots: Vec<String> = self
            .search_results
            .iter()
            .filter(|hit| {
                let kind = hit.get("kind").and_then(|v| v.as_str()).unwrap_or("");
                let file_path = hit.get("file_path").and_then(|v| v.as_str()).unwrap_or("");
                rank::kind_boost(kind) >= 0.0 && !rank::is_test_path(file_path)
            })
            .filter_map(|hit| hit.get("uuid").and_then(|v| v.as_str()))
            .take(TOP_ROOTS)
            .map(String::from)
            .collect();
        if top_roots.is_empty() {
            return Vec::new();
        }

        let links = self
            .ctx
            .graph_db
            .find_caller_links(&top_roots, self.repo_names, 48)
            .await
            .unwrap_or_default();
        if links.is_empty() {
            return Vec::new();
        }

        let mut roots_per_caller: std::collections::HashMap<String, HashSet<String>> =
            std::collections::HashMap::new();
        let mut caller_uuids: Vec<String> = Vec::new();
        for (caller, target) in &links {
            if !self.seen_uuids.contains(caller) && !caller_uuids.contains(caller) {
                caller_uuids.push(caller.clone());
            }
            roots_per_caller
                .entry(caller.clone())
                .or_default()
                .insert(target.clone());
        }
        if caller_uuids.is_empty() {
            return Vec::new();
        }

        let mut hits = self
            .ctx
            .vector_db
            .search_by_uuids(UuidProbe {
                vector: self.vector,
                uuids: &caller_uuids,
                repo_names: self.repo_names,
                kinds: self.expanded_kinds,
                limit: pool_cap,
            })
            .await
            .unwrap_or_default();

        let to_roots: Vec<String> = top_roots.clone();
        let roots_map = &roots_per_caller;
        for hit in &mut hits {
            if let Some(uuid) = hit.get("uuid").and_then(|v| v.as_str()) {
                let count = roots_map
                    .get(uuid)
                    .map(|targets| targets.iter().filter(|t| to_roots.contains(*t)).count())
                    .unwrap_or(0);
                if let Some(obj) = hit.as_object_mut() {
                    obj.insert("caller_roots".to_string(), json!(count));
                }
            }
        }
        hits.retain(|hit| {
            hit.get("caller_roots")
                .and_then(|v| v.as_i64())
                .unwrap_or(0)
                > 0
        });
        hits
    }
}

/// How many top vector hits seed the caller-recall bridge. Three roots are
/// enough to cover the helpers a behavioural paraphrase ranks first (its
/// shared caller usually calls two or three of them) while keeping the
/// Neo4j fan-in bounded.
const TOP_ROOTS: usize = 3;

fn push_if_unique(
    entity: &serde_json::Value,
    seen_uuids: &mut HashSet<String>,
    combined: &mut Vec<serde_json::Value>,
) {
    if let Some(uuid) = entity.get("uuid").and_then(|v| v.as_str())
        && seen_uuids.insert(uuid.to_string())
    {
        combined.push(entity.clone());
    }
}

pub(crate) fn extract_subclass_names(extends_arr: &[serde_json::Value]) -> Vec<String> {
    extends_arr
        .iter()
        .filter_map(|e| e.get("name").and_then(|v| v.as_str()).map(String::from))
        .collect()
}

pub(crate) fn extract_implementer_names(implements_arr: &[serde_json::Value]) -> Vec<String> {
    implements_arr
        .iter()
        .filter_map(|e| e.get("name").and_then(|v| v.as_str()).map(String::from))
        .collect()
}

pub(crate) fn format_reference_samples(references_arr: &[serde_json::Value]) -> Vec<String> {
    references_arr
        .iter()
        .take(3)
        .map(|e| {
            let name = e.get("name").and_then(|v| v.as_str()).unwrap_or("?");
            let file = e.get("file_path").and_then(|v| v.as_str()).unwrap_or("?");
            format!("{} in {}", name, file)
        })
        .collect()
}

pub(crate) fn enrich_single_entity(entity: &mut serde_json::Value, references: &serde_json::Value) {
    if let Some(extends_arr) = references.get("extends").and_then(|v| v.as_array())
        && !extends_arr.is_empty()
    {
        let subclasses = extract_subclass_names(extends_arr);
        if let Some(obj) = entity.as_object_mut() {
            obj.insert("subclasses".to_string(), json!(subclasses));
        }
    }

    if let Some(implements_arr) = references.get("implements").and_then(|v| v.as_array())
        && !implements_arr.is_empty()
    {
        let implementers = extract_implementer_names(implements_arr);
        if let Some(obj) = entity.as_object_mut() {
            obj.insert("implementers".to_string(), json!(implementers));
        }
    }

    if let Some(references_arr) = references.get("references").and_then(|v| v.as_array())
        && !references_arr.is_empty()
    {
        let usage_count = references_arr.len();
        let samples = format_reference_samples(references_arr);
        if let Some(obj) = entity.as_object_mut() {
            obj.insert("type_usage_count".to_string(), json!(usage_count));
            obj.insert("type_usage_samples".to_string(), json!(samples));
        }
    }

    if let Some(calls_arr) = references.get("calls").and_then(|v| v.as_array())
        && !calls_arr.is_empty()
    {
        let caller_count = calls_arr.len();
        let samples = format_reference_samples(calls_arr);
        if let Some(obj) = entity.as_object_mut() {
            obj.insert("caller_count".to_string(), json!(caller_count));
            obj.insert("caller_samples".to_string(), json!(samples));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- ResolvedLimit / resolve_max_results (advertised-bound contract) ---

    #[test]
    fn resolve_max_results_clamps_above_ceiling() {
        let resolved = resolve_max_results(10_000);
        assert_eq!(resolved.value, MAX_RESULTS_CEILING);
        assert_eq!(resolved.requested, 10_000);
        assert!(resolved.was_clamped());
    }

    #[test]
    fn resolve_max_results_preserves_values_within_bound() {
        for n in [1, DEFAULT_MAX_RESULTS, 20, MAX_RESULTS_CEILING] {
            let resolved = resolve_max_results(n);
            assert_eq!(resolved.value, n);
            assert!(!resolved.was_clamped());
            assert!(resolved.notice().is_none());
        }
    }

    #[test]
    fn resolve_max_results_floors_at_one() {
        // A result count of 0 would mean "return nothing at all"; floored.
        let resolved = resolve_max_results(0);
        assert_eq!(resolved.value, 1);
        assert!(resolved.was_clamped());
    }

    #[test]
    fn clamp_notice_states_bound_and_refine_rule() {
        let notice = resolve_max_results(500).notice().expect("clamped");
        assert!(notice.contains("500"));
        assert!(notice.contains(MAX_RESULTS_CEILING.to_string().as_str()));
        assert!(notice.contains("kinds"));
        assert!(notice.contains("path"));
        assert!(notice.contains("no pagination"));
    }

    #[test]
    fn extract_subclass_names_empty() {
        let refs = vec![];
        let names = extract_subclass_names(&refs);
        assert_eq!(names.len(), 0);
    }

    #[test]
    fn test_extract_subclass_names_with_data() {
        let refs = vec![
            json!({"name": "ChildClass1", "kind": "class"}),
            json!({"name": "ChildClass2", "kind": "class"}),
        ];
        let names = extract_subclass_names(&refs);
        assert_eq!(names.len(), 2);
        assert!(names.contains(&"ChildClass1".to_string()));
        assert!(names.contains(&"ChildClass2".to_string()));
    }

    #[test]
    fn test_extract_implementer_names() {
        let refs = vec![
            json!({"name": "ImplClass1", "kind": "class"}),
            json!({"name": "ImplClass2", "kind": "class"}),
        ];
        let names = extract_implementer_names(&refs);
        assert_eq!(names.len(), 2);
        assert!(names.contains(&"ImplClass1".to_string()));
    }

    #[test]
    fn test_format_reference_samples_limits_to_three() {
        let refs = vec![
            json!({"name": "usage1", "file_path": "file1.java"}),
            json!({"name": "usage2", "file_path": "file2.java"}),
            json!({"name": "usage3", "file_path": "file3.java"}),
            json!({"name": "usage4", "file_path": "file4.java"}),
            json!({"name": "usage5", "file_path": "file5.java"}),
        ];
        let samples = format_reference_samples(&refs);
        assert_eq!(samples.len(), 3);
        assert!(samples[0].contains("usage1"));
        assert!(samples[1].contains("usage2"));
        assert!(samples[2].contains("usage3"));
    }

    #[test]
    fn test_format_reference_samples() {
        let refs = vec![json!({"name": "caller1", "file_path": "caller.java"})];
        let samples = format_reference_samples(&refs);
        assert_eq!(samples.len(), 1);
        assert!(samples[0].contains("caller1"));
        assert!(samples[0].contains("caller.java"));
    }

    #[test]
    fn test_enrich_single_entity_with_subclasses() {
        let mut entity = json!({"name": "MyClass", "kind": "class"});
        let references = json!({
            "extends": [
                {"name": "Child1", "kind": "class"},
                {"name": "Child2", "kind": "class"}
            ],
            "implements": [],
            "references": [],
            "calls": []
        });

        enrich_single_entity(&mut entity, &references);

        assert_eq!(
            entity.get("subclasses"),
            Some(&json!(vec!["Child1", "Child2"]))
        );
    }

    #[test]
    fn test_enrich_single_entity_with_all_relationships() {
        let mut entity = json!({"name": "MyInterface"});
        let references = json!({
            "extends": [{"name": "Child1"}],
            "implements": [{"name": "Impl1"}, {"name": "Impl2"}],
            "references": [
                {"name": "ref1", "file_path": "ref1.java"},
                {"name": "ref2", "file_path": "ref2.java"}
            ],
            "calls": [{"name": "caller1", "file_path": "caller.java"}]
        });

        enrich_single_entity(&mut entity, &references);

        assert!(entity.get("subclasses").is_some());
        assert!(entity.get("implementers").is_some());
        assert!(entity.get("type_usage_count").is_some());
        assert!(entity.get("caller_count").is_some());
    }

    #[test]
    fn test_enrich_single_entity_ignores_empty_arrays() {
        let mut entity = json!({"name": "MyClass"});
        let references = json!({
            "extends": [],
            "implements": [],
            "references": [],
            "calls": []
        });

        enrich_single_entity(&mut entity, &references);

        assert!(entity.get("subclasses").is_none());
        assert!(entity.get("implementers").is_none());
        assert!(entity.get("type_usage_count").is_none());
        assert!(entity.get("caller_count").is_none());
    }
}
