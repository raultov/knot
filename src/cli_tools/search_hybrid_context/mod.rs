//! Core search_hybrid_context logic shared between CLI and MCP
//!
//! Performs a hybrid search combining:
//! 1. Prefix name match via Neo4j (exact name prefix, case-insensitive)
//! 2. Semantic search via Qdrant vector similarity (understands code meaning)
//! 3. Kind-aware re-ranking (see [`rank`] — definitions outrank prose, tests
//!    and config/build entities for natural-language queries)
//! 4. Structural expansion via Neo4j graph relationships (understands architecture)
//!
//! This module is the thin orchestrator; the pieces it stitches together
//! live in split submodules, each with one responsibility:
//!
//! - [`limits`] — the `max_results` bound contract (advertised schema);
//! - [`recall`] — candidate-pool assembly over the recall channels;
//! - [`coverage`] — caller bridge + root-coverage annotation;
//! - [`enrich`] — annotation-only graph relationship enrichment.
//!
//! Graph enrichment only *annotates* the entities already returned; it never
//! injects callers or helpers as substitute results.

use serde_json::json;
use std::collections::HashSet;
use std::sync::{Arc, Mutex};

pub mod coverage;
pub mod enrich;
pub mod limits;
pub mod rank;
pub mod recall;

use crate::db::{
    graph::{GraphDb, QueryExt as _, RepoQueryExt as _},
    vector::{VectorDb, VectorSearchExt as _},
};
use crate::models::RepoScope;
use crate::pipeline::embed::Embedder;
use crate::startup_guard::RepoEmbedMarker as EmbedMarker;

pub use limits::{DEFAULT_MAX_RESULTS, MAX_RESULTS_CEILING, ResolvedLimit, resolve_max_results};
// Diagnostic channel marker consumed by the rank trace (`rank`).
pub(crate) use recall::CHANNEL_FIELD;

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

    let mismatch_note = search_model_mismatch_note(&repo_names, ctx).await;
    if let Some(note) = &mismatch_note {
        tracing::warn!("search_hybrid_context: {note}");
    }

    let expanded_kinds = rank::parse_kinds(filters.kinds);
    let normalized_path = recall::normalized_path(filters.path);
    let pool_window = recall::pool_window(max_results, normalized_path.is_some());

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
    let rejected_prefix_uuids = recall::merge_prefix_hits(
        &prefix_results,
        &expanded_kinds,
        normalized_path.as_deref(),
        &mut seen_uuids,
        &mut combined,
    );

    // Vector hits: dedup, name/token probe recall and the caller bridge,
    // merged into one candidate pool for the kind-aware re-rank; the path
    // filter applies to every channel when one is set.
    let pool = recall::CandidatePool {
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
    recall::strip_internal_fields(&mut combined);

    if combined.is_empty() {
        // Never a silent empty result when a model mismatch explains it.
        return Ok(match mismatch_note {
            Some(note) => json!({ "note": note }),
            None => serde_json::Value::Null,
        });
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

    let enriched_context =
        enrich::enrich_with_relationships(&context, &entity_names, ctx.graph_db, repo)
            .await
            .unwrap_or(context);

    Ok(enriched_context)
}

/// Query-time guard (§F4.5): a repository indexed with another model never
/// matches query vectors of this search — usually an empty result that
/// looks like "nothing found", the exact silent failure the model markers
/// exist to name. Returns an explicit note naming both models when the
/// scoped repositories carry a mismatching marker; `None` when all agree
/// (or the markers are unreadable / the scope is unfiltered — the startup
/// warning covers that case).
async fn search_model_mismatch_note(
    repo_names: &[String],
    ctx: &SearchContext<'_>,
) -> Option<String> {
    if repo_names.is_empty() {
        return None;
    }
    let active_model = ctx.embedder.lock().unwrap().model_name();
    let markers = ctx.graph_db.repo_embed_markers(repo_names).await.ok()?;
    let mismatched: Vec<&EmbedMarker> = markers
        .iter()
        .filter(|m| {
            m.embed_model
                .as_deref()
                .is_some_and(|their| their != active_model)
        })
        .collect();
    if mismatched.is_empty() {
        return None;
    }
    Some(format!(
        "Repositories {} were indexed with embedding model '{}' \
             but this query was searched with '{}': they will not surface \
             in semantic search. Re-index them with '{}' \
             (`knot-indexer --clean`).",
        mismatched
            .iter()
            .map(|m| m.repo_name.clone())
            .collect::<Vec<_>>()
            .join(", "),
        mismatched[0].embed_model.as_deref().unwrap_or("?"),
        active_model,
        active_model
    ))
}
