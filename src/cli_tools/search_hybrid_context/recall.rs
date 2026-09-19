//! Candidate-pool assembly: merges the raw cosine window with the recall
//! channels (definition pass, name/token probe, demoted prefix hits, caller
//! bridge — the bridge and coverage annotation live in `coverage`) into one
//! deduplicated pool for the kind-aware re-rank.

use serde_json::json;
use std::collections::HashSet;
use std::path::Path;

use super::SearchContext;
use super::coverage::{
    CallerBridge, PROBE_ROOTS_MAX, TOP_ROOTS, annotate_root_coverage, seed_roots,
};
use super::rank;
use crate::db::vector::{ExactNameProbe, KindScopeSearch, UuidProbe, VectorSearchExt as _};

/// Field name marking which recall channel produced a pool row
/// (`cosine`, `definition`, `probe`, `bridge`). Diagnostic only: stripped
/// from the output by [`strip_internal_fields`] before results are
/// returned, so it never leaks to CLI JSON or MCP payloads.
pub(crate) const CHANNEL_FIELD: &str = "_channel";

/// Remove internal diagnostic fields from every result row.
pub(crate) fn strip_internal_fields(rows: &mut [serde_json::Value]) {
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
pub(super) fn merge_hits(
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
pub(super) fn normalized_path(path: Option<&str>) -> Option<String> {
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

/// Candidate window for the Qdrant pass.
///
/// Path-restricted searches oversample: a large share of the cosine window
/// may fall outside the path, so the re-rank needs a wider pool to fill
/// `max_results` from matching files alone. The oversample is capped so the
/// widened pool cannot turn into an oversized Qdrant round-trip at the top
/// of the `max_results` range.
pub(crate) fn pool_window(max_results: usize, path_restricted: bool) -> usize {
    const PATH_POOL_CEILING: usize = 600;
    if path_restricted {
        rank::candidate_limit(max_results)
            .saturating_mul(3)
            .min(PATH_POOL_CEILING)
    } else {
        rank::candidate_limit(max_results)
    }
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
pub(crate) fn merge_prefix_hits(
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

fn push_if_unique(
    entity: &serde_json::Value,
    seen_uuids: &mut HashSet<String>,
    combined: &mut Vec<serde_json::Value>,
) {
    merge_hits(std::slice::from_ref(entity), seen_uuids, combined, "prefix");
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

    // --- seed_roots (union-of-channels root seed; logic in `coverage`) ---

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
}
