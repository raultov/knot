//! Root-coverage attribution: seeds the caller-recall bridge from the merged
//! candidate pool, recalls callers through the graph, and annotates every
//! pool row with the graph evidence the re-rank's entry-point signal needs.
//! Annotated channels only *annotate*; the serial module is the place where
//! a row loses recall channels: `mod.rs` orchestrates, `recall.rs` gathers
//! the raw pool, and here it is enriched with provenance.

use serde_json::json;
use std::collections::HashSet;

use super::rank;
use super::recall::CHANNEL_FIELD;
use crate::db::graph::{QueryExt as _, RootCoverage};
use crate::db::vector::UuidProbe;
use crate::db::vector::VectorSearchExt as _;

/// How many top vector hits seed the caller-recall bridge. Eight roots
/// cover the helpers a behavioral paraphrase ranks first even when its
/// shared caller's strongest helpers rank 4th–8th in pure cosine order
/// (three were not enough: the entry point then entered the pool with zero
/// coverage evidence). Seeding is capped by the pool window via `min` at
/// the call site (each seed must map to a real bridge request), and the
/// Neo4j fan-in stays bounded by [`CALLER_LINK_LIMIT`] on the Cypher side.
pub(super) const TOP_ROOTS: usize = 8;

/// How many of the seed slots a name/token-probe hit may fill. Probes are
/// lexically plausible but not verified: without the sub-cap one generic-verb
/// probe (e.g. `acquire`, a lock helper) could crowd out most of the
/// cosine/definition roots that give the coverage signal its subject.
/// Restricts probe seeding, never the probe recall channel itself.
pub(crate) const PROBE_ROOTS_MAX: usize = 3;

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
/// the cosine roots that give the coverage signal its subject.
pub(super) fn seed_roots(
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
            let ctx_flag = row
                .get("is_test_context")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            if rank::kind_boost(kind) < 0.0 || rank::is_test_path(path) || ctx_flag {
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
pub(super) fn supersedes_caller(
    caller_production: bool,
    caller_direct: usize,
    callee_direct: usize,
) -> bool {
    caller_production && caller_direct >= 2 && caller_direct > callee_direct
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
/// pool row afterward by [`annotate_root_coverage`], including cosine
/// hits the bridge never fetched (the entry point is usually already in
/// the cosine pool itself).
///
/// Failures collapse to an empty list; the search continues without the
/// bridge.
pub(super) struct CallerBridge<'a> {
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
    pub ctx: &'a super::SearchContext<'a>,
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
        // unused; entities stay bounded by their own Cypher LIMITs.
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

/// Annotate every pool candidate with the graph evidence the re-rank needs:
/// its FQN (absent from the Qdrant payload, required by the generic-name
/// guard in [`rank`]) and how many of the top semantic roots it calls
/// directly / through exactly one helper ([`rank::root_coverage_boost`]
/// turns both counts into the root-set signal). One bounded round trip.
///
/// Best effort: a failed round trip leaves the pool unannotated and the
/// search proceeds on cosine + kind + lexical alone.
pub(super) async fn annotate_root_coverage(
    pool: &mut [serde_json::Value],
    top_roots: &[String],
    repo_names: &[String],
    ctx: &super::SearchContext<'_>,
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
    let by_uuid: std::collections::HashMap<&str, &RootCoverage> = coverage
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
    let pool_meta: std::collections::HashMap<&str, (&str, &str, bool)> = pool
        .iter()
        .filter_map(|hit| {
            Some((
                hit.get("uuid")?.as_str()?,
                (
                    hit.get("kind")?.as_str()?,
                    hit.get("file_path")?.as_str()?,
                    hit.get("is_test_context")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false),
                ),
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
                    .is_some_and(|(kind, file_path, ctx_flag)| {
                        !rank::is_test_row(kind, file_path, *ctx_flag)
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
