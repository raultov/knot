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
    vector::{ExactNameProbe, KindScopeSearch, UuidProbe, VectorDb, VectorSearchExt},
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

    // Prefix hits of definitions keep the leading slots (scoped name-match
    // contract, see merge_prefix_hits); prose/config/test prefix hits are
    // demoted into the candidate pool for the standard re-rank.
    let rejected_prefix_uuids = merge_prefix_hits(
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
        scored_prefix_uuids: &rejected_prefix_uuids,
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

    // Debug channel markers never reach the enriched output: they exist for
    // `RUST_LOG=search_hybrid_context::rank=debug` diagnosis only.
    strip_internal_fields(&mut combined);

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
                    // `kinds=all`: the call-site enrichment must keep seeing
                    // every reference edge regardless of the search's own
                    // kind filter semantics.
                    .find_references(name, &repo_names, DEFAULT_MAX_TARGETS, Some("all"))
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
    /// UUIDs of prose/config/test prefix hits demoted by
    /// [`merge_prefix_hits`]; scored into the pool through the UUID probe
    /// so they compete under the standard re-rank instead of bypassing it.
    pub scored_prefix_uuids: &'a [String],
    /// Bounded shared handles (vector + graph databases).
    pub ctx: &'a SearchContext<'a>,
}

impl CandidatePool<'_> {
    pub async fn collect(self) -> Vec<serde_json::Value> {
        let mut vector_hits: Vec<serde_json::Value> = Vec::new();
        merge_hits(
            self.search_results,
            self.seen_uuids,
            &mut vector_hits,
            "cosine",
        );

        // Definition channel: a second cosine pass that excludes prose /
        // config / infra kinds. Its reason to exist is that the cosine
        // window is not proportional to its candidate mix: on a
        // documentation-heavy repository the scanned window fills with
        // Markdown before any definition enters, leaving the re-rank's
        // entry-point signal nothing to annotate. Skipping rules in
        // [`should_run_definition_channel`].
        let non_code = crate::cli_tools::kinds::non_code_kinds();
        if should_run_definition_channel(self.expanded_kinds) {
            let definition_hits = self
                .ctx
                .vector_db
                .search_excluding_kinds(KindScopeSearch {
                    vector: self.vector,
                    limit: self.candidate_limit,
                    repo_names: self.repo_names,
                    kinds: self.expanded_kinds,
                    exclude_kinds: &non_code,
                })
                .await
                .unwrap_or_default();
            merge_hits(
                &definition_hits,
                self.seen_uuids,
                &mut vector_hits,
                "definition",
            );
        }

        let probe_hits = name_probe_hits(
            self.vector,
            self.query,
            self.repo_names,
            self.expanded_kinds,
            self.ctx,
        )
        .await;
        merge_hits(&probe_hits, self.seen_uuids, &mut vector_hits, "probe");

        // Demoted prose/config/test prefix hits: scored through the UUID
        // probe so they carry a true cosine and rank under the standard
        // re-rank, never bypassing it. Bounded: at most one row per
        // rejected prefix hit; the direct Neo4j query capped those at
        // `max_results`.
        if !self.scored_prefix_uuids.is_empty() {
            let scored_prefix = self
                .ctx
                .vector_db
                .search_by_uuids(UuidProbe {
                    vector: self.vector,
                    uuids: self.scored_prefix_uuids,
                    repo_names: self.repo_names,
                    kinds: self.expanded_kinds,
                    limit: self.scored_prefix_uuids.len(),
                })
                .await
                .unwrap_or_default();
            merge_hits(&scored_prefix, self.seen_uuids, &mut vector_hits, "prefix");
        }

        // Root seed from the union of channels, then the caller bridge over
        // that seed (depth 1 + depth 2, see CallerBridge). Both taps read
        // the *merged* pool, not the raw cosine hits — the definition
        // channel restored the helpers a prose-heavy cosine window
        // displaced, and they belong in the root set.
        let top_roots = seed_roots(&vector_hits, TOP_ROOTS, PROBE_ROOTS_MAX);
        let bridge = CallerBridge {
            vector: self.vector,
            roots: &top_roots,
            seen_uuids: self.seen_uuids,
            repo_names: self.repo_names,
            expanded_kinds: self.expanded_kinds,
            ctx: self.ctx,
        }
        .run(self.candidate_limit)
        .await;
        merge_hits(&bridge, self.seen_uuids, &mut vector_hits, "bridge");

        // Root-set coverage + FQN for the *whole* pool — cosine hits (the
        // entry point is usually among them), probe hits and bridge hits
        // alike. This is what makes the re-rank's caller-root signal reach
        // entities the direct search already returned; the bridge alone
        // never annotated those.
        annotate_root_coverage(&mut vector_hits, &top_roots, self.repo_names, self.ctx).await;
        vector_hits
    }
}

/// Field name marking which recall channel produced a pool row
/// (`cosine`, `definition`, `probe`, `bridge`). Diagnostic only: stripped
/// from the output by [`strip_internal_fields`] before results are
/// returned, so it never leaks to CLI JSON or MCP payloads.
pub(crate) const CHANNEL_FIELD: &str = "_channel";

/// Remove internal diagnostic fields from every result row.
fn strip_internal_fields(rows: &mut [serde_json::Value]) {
    for row in rows.iter_mut() {
        if let Some(obj) = row.as_object_mut() {
            obj.remove(CHANNEL_FIELD);
        }
    }
}

/// Append not-yet-seen hits to the pool, updating the seen-UUID set.
///
/// Every row is tagged with `channel` under [`CHANNEL_FIELD`] so the rank
/// trace (`RUST_LOG=search_hybrid_context::rank=debug`) can attribute a
/// candidate to the recall channel that surfaced it.
fn merge_hits(
    hits: &[serde_json::Value],
    seen_uuids: &mut HashSet<String>,
    pool: &mut Vec<serde_json::Value>,
    channel: &'static str,
) {
    for hit in hits {
        if let Some(uuid) = hit.get("uuid").and_then(|v| v.as_str())
            && seen_uuids.insert(uuid.to_string())
        {
            let mut row = hit.clone();
            if let Some(obj) = row.as_object_mut() {
                obj.insert(CHANNEL_FIELD.to_string(), json!(channel));
            }
            pool.push(row);
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

/// Whether the definition channel should run for this request.
///
/// No explicit `kinds` filter → yes: the caller wants everything, so both
/// a code-pass and unrestricted cosine are needed to de-bias prose-heavy
/// windows. A kind filter that mixes code and non-code kinds → run it,
/// because the plain window is still free to fill with prose under the
/// mixed filter. Two cases skip it:
/// - every requested kind is **non-code** (an intentionally documentation-
///   scoped search — `kinds=markdown_section`) is served by the plain pass
///   alone and must never be shadowed by a code channel;
/// - every requested kind is **code**: the plain pass already excludes
///   non-code kinds by `must`, so the channel would be an identical query.
fn should_run_definition_channel(user_kinds: &[String]) -> bool {
    if user_kinds.is_empty() {
        return true;
    }
    let any_user_kind = |non_code: bool| {
        user_kinds
            .iter()
            .all(|k| crate::cli_tools::kinds::is_non_code_kind(k) == non_code)
    };
    !(any_user_kind(true) || any_user_kind(false))
}

/// Merge name-prefix hits into the leading slots, honoring the kind and
/// path filters, and collect the *rejected* rows' UUIDs for the pool.
///
/// Scope of the name-match contract (sharpened): a query whose tokens match
/// an entity's whole name earns leading slots only for **definition and
/// web-reference** kinds. Prose and config/build/infra prefix hits (a
/// Markdown section titled like the query, a YAML key) do not take a slot
/// their ranker never gave them — a historically measured failure was
/// chrome-devtools-mcp `take screenshot`, where the `take screenshot`
/// Markdown section took #1 *before* the ranker saw it. Those rows instead
/// enter the candidate pool through `rejected_prefix_uuids` with their true
/// cosine and compete under the standard kind-aware re-rank (documented
/// topics with no competing definition still surface — the re-rank only
/// demotes them against real definitions).
///
/// Test-path prefix hits keep their slot (the test-path penalty stays a
/// ranker concern; the demotion is a metadata decision — enforced by the
/// web E2E fixture where `test_angular.html` is a fixture, not a test).
///
/// Returns the UUIDs of the demoted rows (empty when none).
fn merge_prefix_hits(
    prefix_results: &serde_json::Value,
    expanded_kinds: &[String],
    normalized_path: Option<&str>,
    seen_uuids: &mut HashSet<String>,
    combined: &mut Vec<serde_json::Value>,
) -> Vec<String> {
    let mut rejected_uuids: Vec<String> = Vec::new();
    if let Some(arr) = prefix_results.as_array() {
        for entity in arr {
            let kind = entity.get("kind").and_then(|v| v.as_str()).unwrap_or("");
            let file_path = entity
                .get("file_path")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if !rank::kinds_allow(expanded_kinds, kind)
                || !rank::path_allows(normalized_path, file_path)
            {
                continue;
            }
            if crate::cli_tools::kinds::is_non_code_kind(kind) {
                if let Some(uuid) = entity.get("uuid").and_then(|v| v.as_str())
                    && !seen_uuids.contains(uuid)
                // The master dedup gate (`seen_uuids`) is untouched here:
                // the entity may still enter later via the cosine channel.
                {
                    rejected_uuids.push(uuid.to_string());
                }
                continue;
            }
            // Test-path prefix hits keep their slot: demoting them would
            // re-apply the test-path penalty through the re-rank on files
            // the naming heuristic already misfires on (a fixture named
            // `test_angular.html` is not a test — measured in the web E2E
            // fixture repo). The structural-boost gates inside `rank` keep
            // test paths out of call-provenance regardless.
            push_if_unique(entity, seen_uuids, combined);
        }
    }
    rejected_uuids
}

/// How many results the name/token probe fetches. Measured: at 24 the
/// probe's lexical set was truncated before interface *implementations*
/// whose cosine is deep (csharp-code-map `GetCallersAsync` (QueryEngine)
/// sits behind 24 higher-cosine lexically-related rows at query time) —
/// the probe recalled only the interface declaration, which the graph
/// shows with zero CALLS. 48 doubles the recall within the same
/// round-trip shape; ranking still happens in the full-pool re-rank.
/// (Rationale for not going further: the probe filter is a keyword OR that
/// Qdrant evaluates over the collection — a bigger limit grows both sides
/// of the cost without a ranked benefit once the pool window dominates.)
const PROBE_RESULT_LIMIT: usize = 48;

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
            limit: PROBE_RESULT_LIMIT,
            repo_names,
            kinds: expanded_kinds,
        })
        .await
        .unwrap_or_default()
}

/// Caller-recall bridge over the top semantic hits.
///
/// Roots are seeded by [`seed_roots`] (union of cosine / definition / probe
/// channels, prose and test paths excluded, [`TOP_ROOTS`] capped with the
/// probe sub-cap [`PROBE_ROOTS_MAX`]); their callers (in-repo, capped at
/// [`CALLER_LINK_LIMIT`] Cypher rows) are scored against the query vector
/// in one Qdrant round-trip and returned as full candidate rows with their
/// true cosine. When the direct-callers round did not fill the pool
/// window, a second hop ([`CALLER_LINK_LIMIT_DEPTH2`], Cypher anchored on
/// the same seed) recalls callers-of-callers — entry points that reach a
/// top root through exactly one helper (e.g. an engine-wide
/// `GetCallersAsync` calling `TraverseGraphAsync` which calls the seeded
/// `GetSymbolAsync`). The bridge is **pure recall**: it only adds unseen
/// callers to the pool; the scoring signal (how many top roots each
/// candidate calls, directly or through one helper) is attached to *every*
/// pool row afterwards by [`annotate_root_coverage`], including cosine
/// hits the bridge never fetched (the entry point is usually already in
/// the cosine pool itself).
///
/// Failures collapse to an empty list; the search continues without the
/// bridge.
pub(crate) struct CallerBridge<'a> {
    /// Query embedding.
    pub vector: &'a [f32],
    /// Seeded root UUIDs (from [`seed_roots`] over the merged pool).
    pub roots: &'a [String],
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
    /// Fetch unseen callers of the seeded roots (depth 1, then depth 2 when
    /// the window allows). Capped by the remaining pool capacity so the
    /// bridge can never displace what the direct search already gathered;
    /// every hit is a genuine caller of at least one seeded root (the
    /// retain below already did).
    pub async fn run(&self, pool_cap: usize) -> Vec<serde_json::Value> {
        let direct_links = self
            .ctx
            .graph_db
            .find_caller_links(self.roots, self.repo_names, CALLER_LINK_LIMIT)
            .await
            .unwrap_or_default();
        if direct_links.is_empty() {
            return Vec::new();
        }

        let mut caller_uuids: HashSet<String> = direct_links
            .iter()
            .map(|(caller, _)| caller.clone())
            .collect();
        let mut hits = self.fetch_callers(&caller_uuids, pool_cap).await;

        // Second hop: entry points that reach a root through exactly one
        // helper. Only worth a round trip when depth 1 left pool capacity
        // unused; bothities stay bounded by their own Cypher LIMITs.
        if hits.len() < pool_cap {
            let depth2_links = self
                .ctx
                .graph_db
                .find_caller_links_depth2(self.roots, self.repo_names, CALLER_LINK_LIMIT_DEPTH2)
                .await
                .unwrap_or_default();
            let before = caller_uuids.len();
            for (caller, _) in depth2_links {
                caller_uuids.insert(caller);
            }
            if caller_uuids.len() > before {
                let mut depth2_hits = self
                    .fetch_callers(&caller_uuids, pool_cap - hits.len())
                    .await;
                // Retained against the union (direct + depth-2) caller set
                // inside `fetch_callers`.
                hits.append(&mut depth2_hits);
            }
        }
        hits
    }

    /// Score the callers against the query (one bounded round trip) and
    /// drop unverified rows: keep only genuine callers and drop the
    /// already-seen UUIDs. Qdrant keyword matching can return a point whose
    /// payload `uuid` differs in case, and an annotated boost must always
    /// rest on real CALL edges.
    async fn fetch_callers(
        &self,
        caller_uuids: &HashSet<String>,
        limit: usize,
    ) -> Vec<serde_json::Value> {
        if limit == 0 {
            return Vec::new();
        }
        let visible: Vec<String> = caller_uuids
            .iter()
            .filter(|u| !self.seen_uuids.contains(*u))
            .cloned()
            .collect();
        if visible.is_empty() {
            return Vec::new();
        }
        let mut hits = self
            .ctx
            .vector_db
            .search_by_uuids(UuidProbe {
                vector: self.vector,
                uuids: &visible,
                repo_names: self.repo_names,
                kinds: self.expanded_kinds,
                limit,
            })
            .await
            .unwrap_or_default();
        hits.retain(|hit| {
            hit.get("uuid")
                .and_then(|v| v.as_str())
                .is_some_and(|u| caller_uuids.contains(u))
        });
        hits
    }
}

/// Pick the caller-bridge seed from the merged candidate pool.
///
/// Admission is the same production-code filter the bridge has always
/// applied (`kind_boost >= 0`, not a test path): prose has no callers and
/// a test's callers are other tests. The union-of-channels seed exists
/// because the plain cosine order is *not* the semantic relevance order of
/// the helpers on documentation-heavy repositories — the definition
/// channel restores the code candidates the cosine window displaced.
///
/// Selection is deterministic: cosine descending, ties broken on
/// `(file_path, start_line, uuid)`. Probe-channel rows fill at most
/// `probe_max` seeds behind non-probe rows in the same order, so a
/// lexically plausible but unverified near-lock helper cannot crowd out
/// the cosine roots that give the coverage signal its subject matter.
pub(crate) fn seed_roots(
    rows: &[serde_json::Value],
    top_roots: usize,
    probe_max: usize,
) -> Vec<String> {
    let mut seedable: Vec<(&str, f32, &str, i64, bool)> = rows
        .iter()
        .filter_map(|row| {
            let uuid = row.get("uuid").and_then(|v| v.as_str())?;
            let kind = row.get("kind").and_then(|v| v.as_str()).unwrap_or("");
            let path = row.get("file_path").and_then(|v| v.as_str()).unwrap_or("");
            if rank::kind_boost(kind) < 0.0 || rank::is_test_path(path) {
                return None;
            }
            let cosine = row.get("score").and_then(|v| v.as_f64()).unwrap_or(0.0) as f32;
            let line = row.get("start_line").and_then(|v| v.as_i64()).unwrap_or(0);
            let probe = row.get(CHANNEL_FIELD).and_then(|v| v.as_str()) == Some("probe");
            Some((uuid, cosine, path, line, probe))
        })
        .collect();
    seedable.sort_by(|a, b| {
        b.1.partial_cmp(&a.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.2.cmp(b.2))
            .then_with(|| a.3.cmp(&b.3))
            .then_with(|| a.0.cmp(b.0))
    });
    let mut roots: Vec<String> = Vec::new();
    let mut probes = 0usize;
    for (uuid, _, _, _, probe) in &seedable {
        if roots.len() == top_roots {
            break;
        }
        if *probe {
            if probes >= probe_max {
                continue;
            }
            probes += 1;
        }
        roots.push((*uuid).to_string());
    }
    roots
}

/// Whether a pool caller reassigns provenance from a callee.
///
/// Requires all of:
/// (a) a **production** caller — a test calling the same helpers must not
/// take the entry point's boost away;
/// (b) a **strictly greater** coverage — equal coverage stays with the
/// callee (measured, job-watch-ui: the UI layer covering the same roots
/// as the login entry point must not halve its boost);
/// (c) the caller to carry the **entry-point signature itself** (≥ 2
/// distinct roots) — measured the other way round (HikariCP): a UI/view
/// wrapper reaching one root would otherwise claim provenance from an
/// entry point covering zero or one, and a one-root call is not the
/// orchestration signature the boost rewards.
pub(crate) fn supersedes_caller(
    caller_production: bool,
    caller_direct: usize,
    callee_direct: usize,
) -> bool {
    caller_production && caller_direct >= 2 && caller_direct > callee_direct
}

/// Annotate every pool candidate with the graph evidence the re-rank needs:
/// its FQN (absent from the Qdrant payload, required by the generic-name
/// guard in [`rank`]) and how many of the top semantic roots it calls
/// directly / through exactly one helper ([`rank::root_coverage_boost`]
/// turns both counts into the root-set signal). One bounded round trip.
///
/// Best effort: a failed round trip leaves the pool unannotated and the
/// search proceeds on cosine + kind + lexical alone.
async fn annotate_root_coverage(
    pool: &mut [serde_json::Value],
    top_roots: &[String],
    repo_names: &[String],
    ctx: &SearchContext<'_>,
) {
    if pool.is_empty() || top_roots.is_empty() {
        return;
    }
    let pool_uuids: Vec<String> = pool
        .iter()
        .filter_map(|hit| hit.get("uuid").and_then(|v| v.as_str()))
        .map(String::from)
        .collect();
    if pool_uuids.is_empty() {
        return;
    }
    let coverage = ctx
        .graph_db
        .fetch_root_coverage(&pool_uuids, top_roots, repo_names, pool_uuids.len())
        .await
        .unwrap_or_default();
    if coverage.is_empty() {
        return;
    }
    let by_uuid: std::collections::HashMap<&str, &crate::db::graph::RootCoverage> = coverage
        .iter()
        .map(|row| (row.uuid.as_str(), row))
        .collect();

    // Who calls whom *inside the pool* shadows the coverage signal: a
    // candidate whose pool caller covers at least the same root set is an
    // internal step of that caller, so its provenance is redundant — the
    // boost is halved ([`rank::ORCHESTRATOR_STEP_ATTENUATION`]). Only a
    // production caller reassigns provenance: a test calling the same
    // helpers must not take the entry point's boost away (the test-path
    // guard of the coverage boost applies to the caller too).
    let pool_meta: std::collections::HashMap<&str, (&str, &str)> = pool
        .iter()
        .filter_map(|hit| {
            Some((
                hit.get("uuid")?.as_str()?,
                (hit.get("kind")?.as_str()?, hit.get("file_path")?.as_str()?),
            ))
        })
        .collect();
    let in_pool: HashSet<&str> = pool_uuids.iter().map(String::as_str).collect();
    let mut superseded: HashSet<String> = HashSet::new();
    if let Ok(links) = ctx
        .graph_db
        .find_caller_links(&pool_uuids, repo_names, pool_uuids.len() * 16)
        .await
    {
        for (caller, callee) in links.iter().filter(|(caller, callee)| {
            in_pool.contains(caller.as_str())
                && in_pool.contains(callee.as_str())
                && caller != callee
        }) {
            let (Some(caller_cov), Some(callee_cov)) =
                (by_uuid.get(caller.as_str()), by_uuid.get(callee.as_str()))
            else {
                continue;
            };
            let caller_production =
                pool_meta
                    .get(caller.as_str())
                    .is_some_and(|(kind, file_path)| {
                        rank::kind_boost(kind) >= 0.0 && !rank::is_test_path(file_path)
                    });
            // STRICTLY greater: equal coverage does not steal provenance.
            // Measured (job-watch-ui `log a user in…`): `LoginPage` and
            // `onSubmit` call the entry point and reach the same two top
            // roots; with the previous `>=` the equality halved the entry
            // point's coverage boost and the component displaced it.
            // Provenance only moves to a caller covering a *superset* —
            // equal sets belong to the callee.
            if supersedes_caller(
                caller_production,
                caller_cov.direct_roots,
                callee_cov.direct_roots,
            ) {
                superseded.insert(callee.clone());
            }
        }
    }

    for hit in pool.iter_mut() {
        let uuid = hit.get("uuid").and_then(|v| v.as_str()).unwrap_or("");
        let Some(row) = by_uuid.get(uuid) else {
            continue;
        };
        let superseded_here = superseded.contains(uuid);
        let Some(obj) = hit.as_object_mut() else {
            continue;
        };
        // The pool already carries a `fqn` when the row came from a channel
        // that enriched it from the graph; only fill the Qdrant gap.
        obj.entry("fqn").or_insert_with(|| json!(row.fqn));
        obj.insert("caller_roots".to_string(), json!(row.direct_roots));
        obj.insert(
            "caller_roots_transitive".to_string(),
            json!(row.transitive_roots),
        );
        obj.insert("caller_root_total".to_string(), json!(top_roots.len()));
        obj.insert("caller_out_degree".to_string(), json!(row.out_degree));
        obj.insert("caller_superseded".to_string(), json!(superseded_here));
    }
}

/// How many top vector hits seed the caller-recall bridge. Eight roots
/// cover the helpers a behavioral paraphrase ranks first even when its
/// shared caller's strongest helpers rank 4th–8th in pure cosine order
/// (three were not enough: the entry point then entered the pool with zero
/// coverage evidence). Seeding is capped by the pool window via `min` at
/// the call site (each seed must map to a real bridge request), and the
/// Neo4j fan-in stays bounded by [`CALLER_LINK_LIMIT`] on the Cypher side.
const TOP_ROOTS: usize = 8;

/// How many of the seed slots a name/token-probe hit may fill. Probes are
/// lexically plausible but not verified: without the sub-cap one generic-
/// verb probe (e.g. `acquire`, a lock helper) could crowd out most of the
/// cosine/definition roots that give the coverage signal its subject
/// matter. Restricts probe seeding, never the probe recall channel itself.
const PROBE_ROOTS_MAX: usize = 3;

/// Neo4j row budget for the caller-recall bridge. Scaled with the widened
/// seed so a popular root cannot eat the whole pair list: the Cypher's
/// `ORDER BY caller_uuid, target_uuid LIMIT $limit` truncates by UUID
/// string, and a root followed by 30 callers would otherwise starve every
/// root sorted after it.
const CALLER_LINK_LIMIT: usize = TOP_ROOTS * 32;

/// Neo4j row budget for the bridge's second hop (callers of callers). Kept
/// narrower than the direct budget: depth-2 callers are a recall net for
/// entry points orchestration-through-one-helper, not a substitute for the
/// direct caller set.
const CALLER_LINK_LIMIT_DEPTH2: usize = TOP_ROOTS * 16;

fn push_if_unique(
    entity: &serde_json::Value,
    seen_uuids: &mut HashSet<String>,
    combined: &mut Vec<serde_json::Value>,
) {
    merge_hits(std::slice::from_ref(entity), seen_uuids, combined, "prefix");
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

    // --- definition channel decision matrix ---

    #[test]
    fn definition_channel_runs_without_kind_filter() {
        assert!(should_run_definition_channel(&[]));
    }

    #[test]
    fn definition_channel_runs_for_mixed_kind_filters() {
        let mixed = vec!["rust_function".to_string(), "markdown_section".to_string()];
        assert!(should_run_definition_channel(&mixed));
    }

    #[test]
    fn definition_channel_skipped_for_docs_only_filters() {
        let docs = vec![
            "markdown_section".to_string(),
            "markdown_document".to_string(),
        ];
        assert!(!should_run_definition_channel(&docs));
        let config = vec!["config_property".to_string()];
        assert!(!should_run_definition_channel(&config));
    }

    #[test]
    fn definition_channel_skipped_for_code_only_filters() {
        let code = vec!["rust_function".to_string(), "method".to_string()];
        assert!(!should_run_definition_channel(&code));
    }

    // --- seed_roots (union-of-channels root seed) ---

    /// Fixtures mirroring the measured knot "find relevant code" window:
    /// no code row would survive a cosine-only seed of size 8.
    fn seed_fixture() -> Vec<serde_json::Value> {
        // 6 prose rows (highest cosine) — never seedable.
        let prose: Vec<serde_json::Value> = (0..6)
            .map(|i| {
                json!({"uuid": format!("prose{i}"), "name": "Purpose", "kind": "markdown_section",
                       "file_path": format!("docs/c{i}.md"), "start_line": 1, "score": 0.53 - f64::from(i) * 0.01,
                       "_channel": "cosine"})
            })
            .collect();
        // 4 code rows below all prose (definition channel, cosine displaced).
        let code: Vec<serde_json::Value> = (0..4)
            .map(|i| {
                json!({"uuid": format!("code{i}"), "name": format!("helper{i}"), "kind": "rust_function",
                       "file_path": format!("src/g{i}.rs"), "start_line": 10 + i, "score": 0.38 - f64::from(i) * 0.02,
                       "_channel": "definition"})
            })
            .collect();
        // 2 probe rows interleaved in cosine by the tokenizer.
        let probes: Vec<serde_json::Value> = (0..2)
            .map(|i| {
                json!({"uuid": format!("probe{i}"), "name": "find", "kind": "rust_method",
                       "fqn": format!("app::Things::find{i}"), "file_path": format!("src/f{i}.rs"),
                       "start_line": 5, "score": 0.40 - f64::from(i) * 0.05, "_channel": "probe"})
            })
            .collect();
        // One test-path code row — never seedable despite kind+cosine.
        vec![]
            .into_iter()
            .chain(prose)
            .chain(code)
            .chain(probes)
            .chain(vec![
                json!({"uuid": "testrow", "name": "test_login", "kind": "rust_function",
                               "file_path": "tests/login_test.rs", "start_line": 3, "score": 0.60,
                               "_channel": "cosine"}),
            ])
            .collect()
    }

    #[test]
    fn seed_roots_prefers_code_rows_over_prose() {
        let rows = seed_fixture();
        let roots = seed_roots(&rows, 8, 3);
        // No prose uuid and no test row may appear, whatever their cosine.
        assert!(!roots.iter().any(|u| u.starts_with("prose")), "{roots:?}");
        assert!(!roots.contains(&"testrow".to_string()));
        // Code channels fill the slots first (cosine order within channel).
        let code_count = roots.iter().filter(|u| u.starts_with("code")).count();
        assert_eq!(
            code_count, 4,
            "all non-probe candidates seed first: {roots:?}"
        );
        // Probes backfill the remainder, in cosine order.
        let probe_count = roots.iter().filter(|u| u.starts_with("probe")).count();
        assert_eq!(probe_count, 2);
    }

    #[test]
    fn seed_roots_caps_probe_channel_rows() {
        let mut rows = seed_fixture();
        // Add three more probes: the sub-cap must keep the total at
        // PROBE_ROOTS_MAX even when they out-cosine the code rows.
        for i in 2..5 {
            rows.push(
                json!({"uuid": format!("probe{i}"), "name": "find", "kind": "rust_method",
                             "file_path": format!("src/f{i}.rs"), "start_line": 5,
                             "score": 0.42 - f64::from(i) * 0.01, "_channel": "probe"}),
            );
        }
        let roots = seed_roots(&rows, 8, 3);
        let probe_count = roots.iter().filter(|u| u.starts_with("probe")).count();
        assert_eq!(
            probe_count, PROBE_ROOTS_MAX,
            "probe sub-cap violated: {roots:?}"
        );
        // Non-probe candidates keep filling: with 5 probes, the sub-cap
        // leaves the remaining slots to the four code rows (8 slots −
        // 3 probes − 1 excluded test row never re-enters).
        assert_eq!(roots.len(), PROBE_ROOTS_MAX + 4, "{roots:?}");
        assert_eq!(roots.iter().filter(|u| u.starts_with("code")).count(), 4);
    }

    #[test]
    fn seed_roots_deterministic_on_cosine_ties() {
        let row = |uuid: &str, path: &str, line: i64| {
            json!({"uuid": uuid, "name": "h", "kind": "rust_function", "file_path": path,
                   "start_line": line, "score": 0.5, "_channel": "cosine"})
        };
        let rows = vec![row("b", "src/a.rs", 1), row("a", "src/a.rs", 1)];
        let roots = seed_roots(&rows, 4, 3);
        // Tie-break (file_path, start_line, uuid) → "a" before "b".
        assert_eq!(roots, vec!["a".to_string(), "b".to_string()]);
    }

    // --- supersedes_caller (provenance reassignment) ---

    #[test]
    fn provenance_moves_only_to_full_orchestrator_signatures() {
        // A true outer orchestrator (3 roots) reassigns from a 2-root step.
        assert!(supersedes_caller(true, 3, 2));
        // Equal coverage stays with the callee (measured job-watch-ui case).
        assert!(
            !supersedes_caller(true, 2, 2),
            "equal coverage must not halve the entry point"
        );
        assert!(!supersedes_caller(true, 1, 2), "subset caller never wins");
        // One-covering-root caller is not the orchestration signature the
        // boost rewards — provenance stays with the callee no matter what
        // a 0/1-root entry covers (measured: a view wrapper at 1 root
        // claimed the provenance of the function it calls).
        assert!(
            !supersedes_caller(true, 1, 0),
            "1-root caller is not an orchestrator"
        );
    }

    #[test]
    fn provenance_ignores_test_callers() {
        assert!(!supersedes_caller(false, 5, 2));
    }

    // --- prefix demotion (C3) ---

    #[test]
    fn prefix_prose_hits_do_not_take_leading_slots() {
        let prefix_rows = json!([
            // Definition hit: keeps the leading slot.
            {"uuid": "def", "name": "GetCallersAsync", "kind": "csharp_method",
             "file_path": "src/QueryEngine.cs", "start_line": 3},
            // Web reference-target kind: keeps its slot — the demotion is
            // metadata-only (measured: the web E2E fixture `test_angular.html`
            // is a fixture, not a test, despite the naming heuristic).
            {"uuid": "webro", "name": "app-header", "kind": "html_element",
             "file_path": "test_angular.html", "start_line": 1},
            // Prose hit with the query's own title: demoted, no slot.
            {"uuid": "prose", "name": "take screenshot", "kind": "markdown_section",
             "fqn": "docs/tool-reference.md::… > take screenshot",
             "file_path": "docs/tool-reference.md", "start_line": 12},
            // Config hit: demoted.
            {"uuid": "cfg", "name": "screenshot.format", "kind": "config_property",
             "file_path": "config.yaml", "start_line": 2},
        ]);
        let mut seen = HashSet::new();
        let mut combined = Vec::new();
        let rejected = merge_prefix_hits(&prefix_rows, &[], None, &mut seen, &mut combined);
        let names: Vec<&str> = combined
            .iter()
            .filter_map(|e| e.get("name").and_then(|v| v.as_str()))
            .collect();
        assert_eq!(
            names.iter().position(|n| *n == "GetCallersAsync"),
            Some(0),
            "definition prefix hits keep their leading slot"
        );
        assert_eq!(
            names.iter().position(|n| *n == "app-header"),
            Some(1),
            "web reference-target prefix hits keep their slot"
        );
        assert!(
            names
                .iter()
                .all(|n| *n == "GetCallersAsync" || *n == "app-header"),
            "no prose/config prefix row bypassed the ranker: {names:?}"
        );
        assert_eq!(rejected, vec!["prose".to_string(), "cfg".to_string()]);
        // Rejected rows did not touch the master dedup gate: the candidates
        // can still enter via cosine/definition/probe without a gap.
        assert!(
            seen.contains("def"),
            "leading slot rows are tracked for dedup"
        );
        assert!(!seen.contains("prose"));
        assert!(
            !seen.contains("cfg"),
            "rejection leaves the dedup gate untouched"
        );
        // Non-object docstring fields pass through untouched (fit check).
    }

    // --- channel marker (diagnostic provenance) ---

    #[test]
    fn merge_hits_tags_rows_with_channel_and_dedups() {
        let hits = vec![
            json!({"uuid": "u1", "name": "a"}),
            json!({"uuid": "u2", "name": "b"}),
        ];
        let mut seen = HashSet::new();
        let mut pool = Vec::new();
        merge_hits(&hits, &mut seen, &mut pool, "cosine");
        // A re-run with the same rows adds nothing (seen-UUID dedup)…
        merge_hits(&hits, &mut seen, &mut pool, "probe");
        assert_eq!(pool.len(), 2, "duplicate uuids must not re-enter the pool");
        // …but the first channel wins.
        for row in &pool {
            assert_eq!(
                row.get(CHANNEL_FIELD).and_then(|v| v.as_str()),
                Some("cosine")
            );
        }
    }

    #[test]
    fn strip_internal_fields_removes_channel_marker() {
        let mut rows = vec![
            json!({"uuid": "u1", "name": "a", "_channel": "bridge"}),
            json!([]), // non-object rows are untouched
        ];
        strip_internal_fields(&mut rows);
        assert!(
            rows[0].get(CHANNEL_FIELD).is_none(),
            "internal marker must not survive into output"
        );
        assert_eq!(rows[0]["uuid"], "u1");
        // Non-object values stay intact.
        assert!(rows[1].is_array());
    }

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
