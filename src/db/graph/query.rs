use anyhow::{Context, Result};
use neo4rs::query;

use super::GraphDb;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MatchTier {
    ExactFqn,
    FqnSuffix,
    ExactName,
    SignaturePrefix,
    Fuzzy,
}

/// Lightweight projection of an Entity used as a root candidate.
///
/// Distinct from [`TargetRow`] (which lacks `signature`/`docstring`) because
/// the subgraph root is rendered in the response and surfaced through the
/// `root_resolution` disclosure — so the fields the consumer expects to see
/// must be present on the candidate row.
///
/// This is the canonical type; `models::RootCandidateLite` is an alias of it
/// so the wire payload and the db projection cannot drift apart.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RootCandidate {
    pub uuid: String,
    pub name: String,
    pub fqn: Option<String>,
    pub kind: Option<String>,
    pub signature: Option<String>,
    pub docstring: Option<String>,
    pub file_path: Option<String>,
    pub start_line: Option<i64>,
}

impl MatchTier {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::ExactFqn => "exact_fqn",
            Self::FqnSuffix => "fqn_suffix",
            Self::ExactName => "exact_name",
            Self::SignaturePrefix => "signature_prefix",
            Self::Fuzzy => "fuzzy",
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TargetRow {
    pub uuid: String,
    pub name: String,
    pub fqn: String,
    pub kind: String,
    pub file_path: String,
    pub start_line: i64,
    /// Repository the resolved target belongs to (`""` when the node predates
    /// repo attribution). Makes `resolution.targets[]` self-labeling.
    pub repo_name: String,
}

/// Minimum length of the query string for Fuzzy match to be enabled.
/// This prevents very short queries from matching almost everything.
pub const MIN_FUZZY_LEN: usize = 4;

/// Default maximum number of targets to return before truncating.
/// Keeps the performance reasonable and prevents huge outputs.
pub const DEFAULT_MAX_TARGETS: usize = 25;

/// Hard ceiling for the number of targets a single resolution may return.
/// Callers may raise the cap up to this value (MCP `max_targets` /
/// CLI `--max-targets`) but never beyond it.
pub const MAX_TARGETS_CEILING: usize = 500;

pub fn target_resolution_tiers(name: &str) -> Vec<(MatchTier, &'static str)> {
    let mut tiers = Vec::new();

    // A bare name equal to a bare FQN is indistinguishable from a name match;
    // gating on separators keeps intent explicit and lets name homonyms
    // (class vs module-level function) surface via ExactName.
    // This mirrors the FqnSuffix gating.
    if name.contains('.') || name.contains("::") {
        tiers.push((MatchTier::ExactFqn, "target.fqn = $name"));
        tiers.push((
            MatchTier::FqnSuffix,
            "target.fqn ENDS WITH '.' + $name OR target.fqn ENDS WITH '::' + $name",
        ));
    }

    tiers.push((MatchTier::ExactName, "target.name = $name"));

    if name.contains('(') {
        tiers.push((
            MatchTier::SignaturePrefix,
            "(target.name + COALESCE(target.signature, '')) STARTS WITH $name",
        ));
    }

    if name.len() >= MIN_FUZZY_LEN {
        tiers.push((
            MatchTier::Fuzzy,
            "toLower(target.fqn) CONTAINS $name_lower \
             OR toLower(target.name + COALESCE(target.signature, '')) CONTAINS $name_lower",
        ));
    }

    tiers
}

/// Root-preference rank for an entity kind (wire format, i.e. the snake_case
/// string stored in Neo4j — `SubgraphNode.kind` is read straight from the
/// `root.kind` property, and `EntityKind`'s wire form is its `Display` impl,
/// `src/models/entity.rs:148-265`). Lower ranks win.
///
/// Seed list lifted from `is_type_like` (`non_calls.rs:10-47`) minus the
/// namespaces: a `csharp_namespace` named like a type must not outrank the
/// type. Namespaces are containers, ranked below callables.
pub fn root_kind_rank(kind: Option<&str>) -> u8 {
    let Some(kind) = kind else {
        return 4;
    };
    match kind {
        // --- rank 0: type declarations --------------------------------------
        // `class` / `interface` / `enum` (generic, language-agnostic Display forms)
        "class" | "interface" | "enum"
        // Kotlin
        | "kotlin_class" | "kotlin_interface" | "kotlin_object"
        | "kotlin_companion_object" | "kotlin_enum"
        // Rust
        | "rust_struct" | "rust_enum" | "rust_union" | "rust_trait"
        | "rust_type_alias"
        // Python
        | "python_class"
        // C / C++
        | "c_struct" | "cpp_class"
        // Groovy
        | "groovy_class" | "groovy_interface" | "groovy_trait" | "groovy_enum"
        // C#
        | "csharp_class" | "csharp_interface" | "csharp_struct" | "csharp_record"
        | "csharp_enum" | "csharp_delegate" => 0,

        // --- rank 1: callables -----------------------------------------------
        "method" | "function"
        | "kotlin_function" | "kotlin_method"
        | "rust_function" | "rust_method" | "rust_macro_def"
        | "rust_impl"
        | "python_function" | "python_method"
        | "c_function" | "cpp_method"
        | "csharp_method" | "csharp_constructor" | "csharp_local_function"
        | "csharp_operator" | "csharp_indexer"
        | "groovy_method" | "groovy_function"
        | "macro_definition"
        | "scss_function" | "scss_mixin"
        | "vcl_subroutine" | "vcl_builtin_sub"
        | "vcc_function" | "vcc_method" => 1,

        // --- rank 2: members / data ------------------------------------------
        "constant"
        | "kotlin_property"
        | "rust_constant" | "rust_static"
        | "python_constant"
        | "csharp_property" | "csharp_field" | "csharp_constant" | "csharp_event"
        | "groovy_property"
        | "config_property"
        | "helm_value"
        | "css_variable"
        | "scss_variable" => 2,

        // --- rank 3: containers ----------------------------------------------
        "rust_module" | "python_module"
        | "cpp_namespace" | "csharp_namespace"
        | "markdown_document" | "markdown_section" => 3,

        // --- rank 4: everything else (build_, k8s_, HTML_, vtc_, project_identity, …)
        _ => 4,
    }
}

/// Order root candidates by `(root_kind_rank, file_path, start_line, uuid)`.
///
/// The trailing `(file_path, start_line, uuid)` tail is a total order —
/// required because neither `name` nor `fqn` is unique (`<module>` entities
/// share both; partial classes share both; etc.). Mirrors the
/// `m.fqn, m.uuid` tail of `find_entities_by_name_prefix`
/// (`query.rs:529-559`).
///
/// `Option` fields default to a deterministic empty/`0` value so the sort
/// is total even when a row misses any of them.
pub fn rank_root_candidates(mut candidates: Vec<RootCandidate>) -> Vec<RootCandidate> {
    candidates.sort_by(|a, b| {
        let ra = root_kind_rank(a.kind.as_deref());
        let rb = root_kind_rank(b.kind.as_deref());
        ra.cmp(&rb)
            .then_with(|| a.file_path.cmp(&b.file_path))
            .then_with(|| a.start_line.cmp(&b.start_line))
            .then_with(|| a.uuid.cmp(&b.uuid))
    });
    candidates
}

/// Relationship types the `find_references` buckets cover in a single
/// round-trip, via `type(r)` in the projection.
///
/// Produced-and-consumed edges only: `CONTAINS` (containment, not use) and
/// `DEPENDS_ON` (repository level, served by `knot deps`) are deliberately
/// absent, and `GENERIC_BOUND` is never built by the pipeline.
pub fn find_references_rel_labels() -> &'static str {
    "CALLS|EXTENDS|IMPLEMENTS|REFERENCES|MACRO_CALLS|REFERENCES_DOM|USES_CSS_CLASS\
     |IMPORTS_SCRIPT|IMPORTS_STYLESHEET|USES_BACKEND|USES_PROBE|USES_ACL\
     |INCLUDES|IMPORTS_VMOD|DECLARED_UNUSED"
}

pub fn relationship_query(rel_labels: &str, repo_scoped: bool) -> String {
    let repo_filter = if repo_scoped {
        "WHERE target.repo_name IN $repo_names AND target.uuid IN $target_uuids"
    } else {
        "WHERE target.uuid IN $target_uuids"
    };

    format!(
        "MATCH (entity:Entity)-[r:{rel_labels}]->(target:Entity)
         {repo_filter}
         RETURN type(r) AS rel_type,
                entity.name, entity.kind, entity.file_path, entity.start_line, entity.signature,
                entity.repo_name AS repo_name,
                target.name AS target_name, target.fqn AS target_fqn,
                target.file_path AS target_file_path,
                target.start_line AS target_start_line, target.signature AS target_signature,
                target.repo_name AS target_repo_name
         ORDER BY target.fqn, entity.file_path, entity.start_line"
    )
}

/// Cipher for the `overridden_by` bucket: implementations/overrides declared
/// in subtypes of the resolved targets.
pub fn overridden_by_query(repo_scoped: bool) -> String {
    let repo_filter = if repo_scoped {
        "WHERE target.repo_name IN $repo_names AND target.uuid IN $target_uuids"
    } else {
        "WHERE target.uuid IN $target_uuids"
    };

    format!(
        "MATCH (entity:Entity)-[:OVERRIDES*1..]->(target:Entity)
         {repo_filter}
           AND entity.uuid <> target.uuid
         RETURN DISTINCT entity.name, entity.kind, entity.file_path, entity.start_line, entity.signature,
                entity.repo_name AS repo_name,
                target.name AS target_name, target.fqn AS target_fqn,
                target.file_path AS target_file_path,
                target.start_line AS target_start_line, target.signature AS target_signature,
                target.repo_name AS target_repo_name
         ORDER BY target.fqn, entity.file_path, entity.start_line"
    )
}

/// Cipher for the `overrides` bucket: the supertype methods the resolved
/// targets implement or override. The projection is mirrored (target ↔ entity)
/// so both buckets share `parse_reference_row` — including the repo aliases,
/// which are swapped for the same reason (`target.repo_name` labels the row's
/// *entity*, `entity.repo_name` labels the row's *target*).
pub fn overrides_query(repo_scoped: bool) -> String {
    let repo_filter = if repo_scoped {
        "WHERE entity.repo_name IN $repo_names AND entity.uuid IN $target_uuids"
    } else {
        "WHERE entity.uuid IN $target_uuids"
    };

    format!(
        "MATCH (entity:Entity)-[:OVERRIDES*1..]->(target:Entity)
         {repo_filter}
           AND entity.uuid <> target.uuid
         RETURN DISTINCT target.name AS `entity.name`, target.kind AS `entity.kind`,
                target.file_path AS `entity.file_path`, target.start_line AS `entity.start_line`,
                target.signature AS `entity.signature`,
                target.repo_name AS repo_name,
                entity.name AS target_name, entity.fqn AS target_fqn,
                entity.file_path AS target_file_path,
                entity.start_line AS target_start_line, entity.signature AS target_signature,
                entity.repo_name AS target_repo_name
         ORDER BY entity.file_path, entity.start_line"
    )
}

pub fn find_callers_query(repo_names: &[String]) -> String {
    if !repo_names.is_empty() {
        "MATCH (caller:Entity)-[:CALLS]->(callee:Entity)
         WHERE callee.repo_name IN $repo_names
           AND (callee.name = $name
            OR callee.fqn = $name)
         RETURN caller.name, caller.kind, caller.file_path, caller.start_line, caller.signature,
                caller.repo_name AS repo_name"
            .to_string()
    } else {
        "MATCH (caller:Entity)-[:CALLS]->(callee:Entity)
         WHERE callee.name = $name
            OR callee.fqn = $name
         RETURN caller.name, caller.kind, caller.file_path, caller.start_line, caller.signature,
                caller.repo_name AS repo_name"
            .to_string()
    }
}

pub fn get_file_entities_query(repo_names: &[String]) -> String {
    if repo_names.len() == 1 {
        "MATCH (e:Entity {file_path: $file_path, repo_name: $repo_name})
         RETURN e.name, e.kind, e.signature, e.docstring, e.start_line, e.decorators
         ORDER BY e.start_line"
            .to_string()
    } else if repo_names.len() > 1 {
        "MATCH (e:Entity)
         WHERE e.file_path = $file_path AND e.repo_name IN $repo_names
         RETURN e.name, e.kind, e.signature, e.docstring, e.start_line, e.decorators
         ORDER BY e.start_line"
            .to_string()
    } else {
        "MATCH (e:Entity {file_path: $file_path})
         RETURN e.name, e.kind, e.signature, e.docstring, e.start_line, e.decorators
         ORDER BY e.start_line"
            .to_string()
    }
}

pub fn get_file_outgoing_references_query(repo_names: &[String]) -> String {
    // The outgoing-reference edge set mirrors `find_references_rel_labels`
    // minus the file-level IMPORTS/INCLUDES/DECLARED_UNUSED edges, whose
    // targets are file containers rather than the definitions this section
    // lists (IMPORTS_SCRIPT/IMPORTS_STYLESHEET reach `explore` through
    // their own pipeline instead).
    if repo_names.len() == 1 {
        "MATCH (src:Entity {file_path: $file_path, repo_name: $repo_name})
              -[r:CALLS|EXTENDS|IMPLEMENTS|REFERENCES|MACRO_CALLS|REFERENCES_DOM|USES_CSS_CLASS|USES_BACKEND|USES_PROBE|USES_ACL]->
              (dst:Entity)
         WHERE dst.file_path <> $file_path OR NOT dst.repo_name IN $repo_names
         RETURN type(r) AS rel,
                dst.name AS name,
                dst.kind AS kind,
                dst.file_path AS file_path,
                dst.start_line AS line
         ORDER BY rel, name"
            .to_string()
    } else if repo_names.len() > 1 {
        "MATCH (src:Entity)
              -[r:CALLS|EXTENDS|IMPLEMENTS|REFERENCES|MACRO_CALLS|REFERENCES_DOM|USES_CSS_CLASS|USES_BACKEND|USES_PROBE|USES_ACL]->
              (dst:Entity)
         WHERE src.file_path = $file_path AND src.repo_name IN $repo_names
           AND (dst.file_path <> $file_path OR NOT dst.repo_name IN $repo_names)
         RETURN type(r) AS rel,
                dst.name AS name,
                dst.kind AS kind,
                dst.file_path AS file_path,
                dst.start_line AS line
         ORDER BY rel, name"
            .to_string()
    } else {
        "MATCH (src:Entity {file_path: $file_path})
              -[r:CALLS|EXTENDS|IMPLEMENTS|REFERENCES|MACRO_CALLS|REFERENCES_DOM|USES_CSS_CLASS|USES_BACKEND|USES_PROBE|USES_ACL]->
              (dst:Entity)
         WHERE dst.file_path <> $file_path
         RETURN type(r) AS rel,
                dst.name AS name,
                dst.kind AS kind,
                dst.file_path AS file_path,
                dst.start_line AS line
         ORDER BY rel, name"
            .to_string()
    }
}

/// Cipher for the read-only file listing used by the `list_files` CLI
/// subcommand and MCP tool: distinct repo-relative paths of the scoped
/// repositories with their entity counts, deterministically ordered.
/// `prefix` filters paths with a path-boundary `STARTS WITH` so a
/// directory prefix (`src/api`) never matches a sibling path that merely
/// contains the fragment (`docs/src/api-notes.md`). `None` (no prefix)
/// lists every file of the scope; the cap keeps the surface read-only
/// cheap even on monorepos.
pub fn list_files_query(repo_scoped: bool) -> String {
    let prefix_clause = "WHERE e.file_path STARTS WITH $prefix ";
    let repo_clause = if repo_scoped {
        "AND e.repo_name IN $repo_names "
    } else {
        ""
    };
    format!(
        "MATCH (e:Entity) \
         {prefix_clause}{repo_clause}\
         RETURN e.file_path AS file_path, e.repo_name AS repo_name, count(e) AS entity_count \
         ORDER BY e.repo_name, e.file_path \
         LIMIT $limit"
    )
}

pub fn find_files_by_suffix_query(suffix_fragment: &str, repo_names: &[String]) -> String {
    if !repo_names.is_empty() {
        format!(
            "MATCH (e:Entity) \
             WHERE ({suffix_fragment}) AND e.repo_name IN $repo_names \
             RETURN DISTINCT e.file_path AS file_path, e.repo_name AS repo_name \
             ORDER BY e.file_path LIMIT 50"
        )
    } else {
        format!(
            "MATCH (e:Entity) \
             WHERE ({suffix_fragment}) \
             RETURN DISTINCT e.file_path AS file_path, e.repo_name AS repo_name \
             ORDER BY e.file_path LIMIT 50"
        )
    }
}

/// Cipher for the caller-recall bridge: the `(caller_uuid, target_uuid)`
/// pairs that CALL any of the given target UUIDs. Used by
/// `search_hybrid_context` to pull the callers of the strongest semantic
/// hits into the candidate pool and to count how many top roots each
/// caller touches — a shared caller of several roots is strong evidence of
/// the described behaviour's entry point (`login` calls both
/// `normalize_email` and `verify_credentials_or_fail`).
pub fn caller_links_query(repo_scoped: bool) -> String {
    let repo_filter = if repo_scoped {
        "AND caller.repo_name IN $repo_names"
    } else {
        ""
    };
    format!(
        "MATCH (caller:Entity)-[:CALLS]->(target:Entity)
         WHERE target.uuid IN $target_uuids {repo_filter}
         RETURN DISTINCT caller.uuid AS caller_uuid, target.uuid AS target_uuid
         ORDER BY caller_uuid, target_uuid
         LIMIT $limit"
    )
}

/// Cipher for one tier of the reference-target resolution ladder used by
/// `resolve_reference_targets`. `predicate` is the post-`WHERE` match
/// expression produced by `target_resolution_tiers` (e.g.
/// `target.fqn = $name`); `repo_scoped` toggles the `repo_name IN` guard.
pub fn reference_target_query(predicate: &str, repo_scoped: bool) -> String {
    let repo_clause = if repo_scoped {
        "target.repo_name IN $repo_names AND "
    } else {
        ""
    };

    format!(
        "MATCH (target:Entity)
         WHERE {repo_clause}({predicate})
         RETURN target.uuid, target.name, target.fqn, target.kind, target.file_path,
                target.start_line, target.repo_name
         ORDER BY target.fqn"
    )
}

/// Outcome of resolving a subgraph root by name: the ranked winner, the
/// ladder tier that produced it, the un-truncated tier count, and every
/// candidate of the winning tier in rank order (already bounded by the
/// ladder's `LIMIT 25`) — so callers can build the `root_resolution`
/// disclosure without re-running the tier query.
pub(crate) struct ResolvedSubgraphRoot {
    pub winner: RootCandidate,
    pub tier: MatchTier,
    pub total_candidates: usize,
    /// All candidates of the winning tier, in rank order.
    pub ranked: Vec<RootCandidate>,
}

/// Outcome of the reference-target resolution ladder used by
/// [`GraphDbExt::find_references`].
///
/// `total` is the **pre-truncation** tier count: the tier Cypher carries no
/// `LIMIT` clause, so it is the real number of entities the queried name
/// resolved to. `targets` is capped at `max_targets` and is what relationship
/// buckets are built from — consumers must treat bucket counts as partial
/// whenever `truncated` is `true`.
///
/// `hidden_non_code` / `hidden_kinds` count the matched entities excluded by
/// the kind filter ([`crate::cli_tools::kinds::KindFilter`]) so the response
/// can disclose them instead of presenting a silently narrowed view.
pub(crate) struct ResolvedTargets {
    pub targets: Vec<TargetRow>,
    pub tier: MatchTier,
    pub truncated: bool,
    pub total: usize,
    pub hidden_non_code: usize,
    pub hidden_kinds: Vec<String>,
}

/// Cap a resolved target list, preserving the true pre-truncation total.
///
/// Mirrors `finalize_nodes` in `query_subgraph.rs`: the length must be
/// measured **before** `truncate`, otherwise downstream consumers report the
/// sample size as the total (the v1.10.0 truncation-reporting bug).
fn finalize_targets(
    targets: Vec<TargetRow>,
    tier: MatchTier,
    max_targets: usize,
) -> ResolvedTargets {
    let total = targets.len();
    let truncated = total > max_targets;
    let mut targets = targets;
    if truncated {
        targets.truncate(max_targets);
    }
    ResolvedTargets {
        targets,
        tier,
        truncated,
        total,
        hidden_non_code: 0,
        hidden_kinds: Vec::new(),
    }
}

/// Maximum number of distinct hidden kinds named in the disclosure; the
/// full picture is always recoverable via `kinds=all`.
const HIDDEN_KINDS_DISCLOSURE_CAP: usize = 6;

/// Split resolved target rows into `(allowed, hidden)` under `filter`.
///
/// Free function so the partition contract is unit-testable without a
/// database: the ladder treats "every row hidden" the same as "no rows"
/// (it keeps walking tiers), and callers need the hidden side only for the
/// disclosure message.
fn partition_by_kind(
    rows: Vec<TargetRow>,
    filter: &crate::cli_tools::kinds::KindFilter,
) -> (Vec<TargetRow>, Vec<TargetRow>) {
    let mut allowed = Vec::new();
    let mut hidden = Vec::new();
    for row in rows {
        if filter.allows(&row.kind) {
            allowed.push(row);
        } else {
            hidden.push(row);
        }
    }
    (allowed, hidden)
}

/// Fold one tier's hidden rows into the accumulated disclosure state:
/// deduplicating by `uuid` (a row may match in several tiers) and keeping
/// the distinct kind list sorted and capped for rendering.
fn fold_hidden(
    hidden_rows: Vec<TargetRow>,
    hidden_uuids: &mut std::collections::HashSet<String>,
    hidden_kinds: &mut Vec<String>,
) -> usize {
    let mut newly_hidden = 0usize;
    for row in hidden_rows {
        if hidden_uuids.insert(row.uuid.clone()) {
            newly_hidden += 1;
        }
        if !hidden_kinds.contains(&row.kind) {
            hidden_kinds.push(row.kind.clone());
        }
    }
    hidden_kinds.sort();
    newly_hidden
}

impl GraphDb {
    async fn resolve_reference_targets(
        &self,
        name: &str,
        repo_names: &[String],
        max_targets: usize,
        kind_filter: &crate::cli_tools::kinds::KindFilter,
    ) -> Result<ResolvedTargets> {
        let tiers = target_resolution_tiers(name);

        let repo_scoped = !repo_names.is_empty();

        // Hidden rows accumulate across tiers, deduplicated by uuid: a row
        // may match in ExactName and again in Fuzzy when an earlier tier
        // produced no *allowed* hits and the ladder kept walking.
        let mut hidden_uuids = std::collections::HashSet::new();
        let mut hidden_kinds: Vec<String> = Vec::new();
        let mut hidden_non_code = 0usize;

        for (tier, predicate) in tiers {
            let query_str = reference_target_query(predicate, repo_scoped);

            // `name_lower` is only consumed by the Fuzzy tier's predicate;
            // binding it on every tier keeps the call sites uniform (an
            // unused query parameter is harmless in neo4rs).
            let mut q = query(&query_str)
                .param("name", name)
                .param("name_lower", name.to_lowercase());
            if repo_scoped {
                q = q.param("repo_names", repo_names.to_vec());
            }

            let mut rows = self.graph.execute(q).await.context(format!(
                "Failed to resolve targets for tier {}",
                tier.as_str()
            ))?;

            let mut targets = Vec::new();
            while let Ok(Some(row)) = rows.next().await {
                let uuid = row.get::<String>("target.uuid").unwrap_or_default();
                let name = row.get::<String>("target.name").unwrap_or_default();
                let fqn = row.get::<String>("target.fqn").unwrap_or_default();
                let kind = row.get::<String>("target.kind").unwrap_or_default();
                let file_path = row.get::<String>("target.file_path").unwrap_or_default();
                let start_line = row.get::<i64>("target.start_line").unwrap_or(0);
                let repo_name = row.get::<String>("target.repo_name").unwrap_or_default();

                targets.push(TargetRow {
                    uuid,
                    name,
                    fqn,
                    kind,
                    file_path,
                    start_line,
                    repo_name,
                });
            }

            if targets.is_empty() {
                continue;
            }

            let (allowed, hidden) = partition_by_kind(targets, kind_filter);
            hidden_non_code += fold_hidden(hidden, &mut hidden_uuids, &mut hidden_kinds);

            if !allowed.is_empty() {
                let mut resolved = finalize_targets(allowed, tier, max_targets);
                resolved.hidden_non_code = hidden_non_code;
                hidden_kinds.truncate(HIDDEN_KINDS_DISCLOSURE_CAP);
                resolved.hidden_kinds = hidden_kinds;
                return Ok(resolved);
            }
            // Every hit of this tier was filtered out: keep walking the
            // ladder — an all-metadata tier must not shadow a code hit in
            // a later tier.
        }

        let default_tier = if name.len() >= MIN_FUZZY_LEN {
            MatchTier::Fuzzy
        } else {
            MatchTier::ExactName
        };
        Ok(ResolvedTargets {
            targets: Vec::new(),
            tier: default_tier,
            truncated: false,
            total: 0,
            hidden_non_code,
            hidden_kinds: {
                let mut kinds = hidden_kinds;
                kinds.truncate(HIDDEN_KINDS_DISCLOSURE_CAP);
                kinds
            },
        })
    }

    /// Resolve a user-supplied entity name to exactly one root candidate,
    /// walking the same tier ladder as `resolve_reference_targets` with early
    /// stop, then applying `rank_root_candidates` inside the winning tier.
    ///
    /// Returns `Some(...)` only when the ladder produced at least one hit.
    /// `total_candidates` is the **un-truncated** tier count (the ladder
    /// queries `LIMIT 25` for ranking fairness; the caller may surface this
    /// in the disclosure). `ranked` carries the winning tier's full candidate
    /// list from the same query that produced the winner, so callers never
    /// need to re-run the tier.
    pub(crate) async fn resolve_subgraph_root(
        &self,
        name: &str,
        repo_name: &str,
    ) -> Result<Option<ResolvedSubgraphRoot>> {
        let tiers = target_resolution_tiers(name);

        for (tier, predicate) in tiers {
            // Build a query per tier. The predicates are local-name aware
            // (e.g. `target.fqn = $name`) — they are reused verbatim from
            // `target_resolution_tiers(name)`.
            let query_str = format!(
                "MATCH (target:Entity)
                 WHERE target.repo_name = $repo_name AND ({predicate})
                 RETURN target.uuid, target.name, target.fqn, target.kind,
                        target.signature, target.docstring, target.file_path, target.start_line
                 ORDER BY target.fqn
                 LIMIT 25"
            );

            let q = query(&query_str)
                .param("name", name)
                // Unused by every tier except Fuzzy, whose predicate matches
                // on the lowercased form; binding it unconditionally keeps
                // this call site in lockstep with `resolve_reference_targets`.
                .param("name_lower", name.to_lowercase())
                .param("repo_name", repo_name);

            let mut rows = self.graph.execute(q).await.context(format!(
                "Failed to resolve subgraph root for tier {}",
                tier.as_str()
            ))?;

            let mut candidates = Vec::new();
            while let Ok(Some(row)) = rows.next().await {
                candidates.push(RootCandidate {
                    uuid: row.get::<String>("target.uuid").unwrap_or_default(),
                    name: row.get::<String>("target.name").unwrap_or_default(),
                    fqn: row.get::<String>("target.fqn").ok(),
                    kind: row.get::<String>("target.kind").ok(),
                    signature: row.get::<String>("target.signature").ok(),
                    docstring: row.get::<String>("target.docstring").ok(),
                    file_path: row.get::<String>("target.file_path").ok(),
                    start_line: row.get::<i64>("target.start_line").ok(),
                });
            }

            if !candidates.is_empty() {
                let total = candidates.len();
                let ranked = rank_root_candidates(candidates);
                // Safe: ranked is non-empty (candidates was non-empty above).
                let winner = ranked.first().cloned().expect("ranked non-empty");
                return Ok(Some(ResolvedSubgraphRoot {
                    winner,
                    tier,
                    total_candidates: total,
                    ranked,
                }));
            }
        }

        Ok(None)
    }

    /// Run one reference query and collect its rows.
    ///
    /// `label` only feeds the error context so a failure names the bucket that
    /// broke.
    async fn collect_reference_rows(
        &self,
        query_str: &str,
        target_uuids: &[String],
        repo_names: &[String],
        label: &str,
    ) -> Result<Vec<serde_json::Value>> {
        let mut q = query(query_str).param("target_uuids", target_uuids.to_vec());
        if !repo_names.is_empty() {
            q = q.param("repo_names", repo_names.to_vec());
        }

        let mut rows = self
            .graph
            .execute(q)
            .await
            .context(format!("Failed to query Neo4j for {label} relationships"))?;

        let mut collected = Vec::new();
        while let Ok(Some(row)) = rows.next().await {
            collected.push(parse_reference_row(row));
        }
        Ok(collected)
    }
}

/// Extension trait for query and read operations.
#[expect(
    async_fn_in_trait,
    reason = "async trait method is required for the db interfaces"
)]
pub trait QueryExt {
    async fn get_entities_with_dependencies(
        &self,
        uuids: &[String],
        repo_names: &[String],
    ) -> Result<serde_json::Value>;
    async fn find_references(
        &self,
        entity_name: &str,
        repo_names: &[String],
        max_targets: usize,
        kinds: Option<&str>,
    ) -> Result<serde_json::Value>;
    async fn find_callers(
        &self,
        entity_name: &str,
        repo_names: &[String],
    ) -> Result<serde_json::Value>;
    async fn get_file_entities(
        &self,
        file_path: &str,
        repo_names: &[String],
    ) -> Result<serde_json::Value>;
    async fn find_entities_by_name_prefix(
        &self,
        prefix: &str,
        repo_names: &[String],
        limit: usize,
    ) -> Result<serde_json::Value>;
    async fn get_file_outgoing_references(
        &self,
        file_path: &str,
        repo_names: &[String],
    ) -> Result<serde_json::Value>;
    /// Suffix-based fallback used by `explore_file` (§4 of
    /// `docs/specs/relative_file_paths.md`). `suffix_fragment` is the
    /// fragment after `WHERE e.file_path ` in the Cipher query (e.g.
    /// `ENDS WITH '/Cargo.toml'`). Returns a list of distinct
    /// `(file_path, repo_name)` pairs that match.
    async fn find_files_by_suffix(
        &self,
        suffix_fragment: &str,
        repo_names: &[String],
    ) -> Result<serde_json::Value>;

    /// Read-only file listing (see [`list_files_query`]): distinct
    /// `(file_path, repo_name)` pairs with entity counts, ordered by
    /// `(repo_name, file_path)`. `prefix` is the normalized repo-relative
    /// directory prefix ("" = list everything); `limit` caps the row count.
    async fn list_files(
        &self,
        prefix: &str,
        repo_names: &[String],
        limit: usize,
    ) -> Result<serde_json::Value>;

    /// Caller-recall bridge: `(caller_uuid, target_uuid)` CALL edges
    /// crossing any of `target_uuids`, scoped to `repo_names` (empty =
    /// all). `limit` caps the pair count. Used by `search_hybrid_context`
    /// to add the callers of its top semantic hits to the candidate pool
    /// and to weight callers by how many top roots they touch.
    async fn find_caller_links(
        &self,
        target_uuids: &[String],
        repo_names: &[String],
        limit: usize,
    ) -> Result<Vec<(String, String)>>;
}

impl QueryExt for GraphDb {
    /// Fetch entities by UUIDs along with their dependencies (outgoing CALLS relationships).
    async fn get_entities_with_dependencies(
        &self,
        uuids: &[String],
        repo_names: &[String],
    ) -> Result<serde_json::Value> {
        if uuids.is_empty() {
            return Ok(serde_json::json!([]));
        }

        let mut results = Vec::new();

        let repo_clause = if !repo_names.is_empty() {
            " AND m.repo_name IN $repo_names"
        } else {
            ""
        };
        let query_str = format!(
            "MATCH (m:Entity) WHERE m.uuid = $uuid{repo_clause}
             OPTIONAL MATCH (m)-[:CALLS]->(dep:Entity)
             RETURN m.name, m.kind, m.fqn, m.signature, m.docstring, m.file_path,
                    m.start_line, m.repo_name AS repo_name,
                    collect(dep.name) as dependencies"
        );

        for uuid in uuids {
            let mut q = query(&query_str).param("uuid", uuid.as_str());
            if !repo_names.is_empty() {
                q = q.param("repo_names", repo_names.to_vec());
            }

            let mut row = self
                .graph
                .execute(q)
                .await
                .context("Failed to query Neo4j for entity dependencies")?;

            if let Ok(Some(row_data)) = row.next().await {
                let name = row_data.get::<String>("m.name").ok();
                let kind = row_data.get::<String>("m.kind").ok();
                let fqn = row_data.get::<String>("m.fqn").ok();
                let signature = row_data.get::<String>("m.signature").ok();
                let docstring = row_data.get::<String>("m.docstring").ok();
                let file_path = row_data.get::<String>("m.file_path").ok();
                let start_line = row_data.get::<i64>("m.start_line").ok();
                let repo_name = row_data.get::<String>("repo_name").ok();
                let dependencies = row_data
                    .get::<Vec<String>>("dependencies")
                    .unwrap_or_default();

                let entity_json = serde_json::json!({
                    "uuid": uuid,
                    "name": name,
                    "kind": kind,
                    "fqn": fqn,
                    "signature": signature,
                    "docstring": docstring,
                    "file_path": file_path,
                    "start_line": start_line,
                    "repo_name": repo_name,
                    "dependencies": dependencies,
                });

                results.push(entity_json);
            }
        }

        Ok(serde_json::json!(results))
    }

    /// Find all entities that reference a given entity via any produced
    /// relationship type (see [`find_references_rel_labels`]). Returns
    /// results grouped by relationship type.
    ///
    /// `kinds` scopes the target resolution (`None` = default code-kind
    /// filter, `all` = no filtering, otherwise an alias/exact-kind list —
    /// see [`crate::cli_tools::kinds::KindFilter`]).
    async fn find_references(
        &self,
        entity_name: &str,
        repo_names: &[String],
        max_targets: usize,
        kinds: Option<&str>,
    ) -> Result<serde_json::Value> {
        let kind_filter = crate::cli_tools::kinds::KindFilter::parse(kinds);
        // Every bucket the multi-label query can produce is pre-initialized;
        // empty ones stay as `[]` so the wire shape is stable (formatters
        // and tests rely on the keys existing).
        let mut results = serde_json::json!({
            "calls": [],
            "extends": [],
            "implements": [],
            "references": [],
            "macro_calls": [],
            "references_dom": [],
            "uses_css_class": [],
            "imports_script": [],
            "imports_stylesheet": [],
            "uses_backend": [],
            "uses_probe": [],
            "uses_acl": [],
            "includes": [],
            "imports_vmod": [],
            "declared_unused": [],
            "overridden_by": [],
            "overrides": []
        });

        // Clamp the requested cap so a caller cannot push a huge scan silently.
        let max_targets = max_targets.clamp(1, MAX_TARGETS_CEILING);

        // Stage 1: Resolve targets
        let resolved = self
            .resolve_reference_targets(entity_name, repo_names, max_targets, &kind_filter)
            .await?;

        // Add the resolution info — `total_targets` is the real pre-truncation
        // entity count so downstream formatters can quantify truncation; the
        // kind-filter fields disclose what the filter removed.
        results["resolution"] = serde_json::json!({
            "query": entity_name,
            "tier": resolved.tier,
            "fuzzy": matches!(resolved.tier, MatchTier::Fuzzy),
            "truncated": resolved.truncated,
            "total_targets": resolved.total,
            "kind_filter": kind_filter.wire_label(),
            "hidden_non_code": resolved.hidden_non_code,
            "hidden_kinds": resolved.hidden_kinds,
            "targets": resolved.targets
        });

        if resolved.targets.is_empty() {
            return Ok(results);
        }

        let target_uuids: Vec<String> = resolved.targets.iter().map(|t| t.uuid.clone()).collect();

        // Stage 2: one labelled multi-relationship query instead of one
        // round-trip per bucket. The global `ORDER BY` remains the intra-bucket
        // order; the stable partition below preserves it per bucket.
        let query_str = relationship_query(find_references_rel_labels(), !repo_names.is_empty());
        let rows = self
            .collect_reference_rows(&query_str, &target_uuids, repo_names, "references")
            .await?;
        partition_reference_rows(&rows, &mut results);

        // Stage 3: OVERRIDES buckets
        for (result_key, query_str) in [
            ("overridden_by", overridden_by_query(!repo_names.is_empty())),
            ("overrides", overrides_query(!repo_names.is_empty())),
        ] {
            let rows = self
                .collect_reference_rows(&query_str, &target_uuids, repo_names, result_key)
                .await?;
            if let Some(arr) = results.get_mut(result_key) {
                *arr = serde_json::json!(rows);
            }
        }

        Ok(results)
    }

    /// Find all entities that call a given entity (reverse dependency lookup).
    /// **Deprecated:** Use `find_references()` instead for comprehensive relationship tracking.
    async fn find_callers(
        &self,
        entity_name: &str,
        repo_names: &[String],
    ) -> Result<serde_json::Value> {
        let mut results = Vec::new();

        let query_str = find_callers_query(repo_names);

        let mut q = query(&query_str).param("name", entity_name);
        if !repo_names.is_empty() {
            q = q.param("repo_names", repo_names.to_vec());
        }

        let mut rows = self
            .graph
            .execute(q)
            .await
            .context("Failed to query Neo4j for callers")?;

        while let Ok(Some(row)) = rows.next().await {
            let caller_json = serde_json::json!({
                "name": row.get::<String>("caller.name").ok(),
                "kind": row.get::<String>("caller.kind").ok(),
                "file_path": row.get::<String>("caller.file_path").ok(),
                "start_line": row.get::<i64>("caller.start_line").ok(),
                "signature": row.get::<String>("caller.signature").ok(),
                "repo_name": row.get::<String>("repo_name").ok(),
            });
            results.push(caller_json);
        }

        Ok(serde_json::json!(results))
    }

    /// Get all entities within a specific file.
    async fn get_file_entities(
        &self,
        file_path: &str,
        repo_names: &[String],
    ) -> Result<serde_json::Value> {
        let mut results = Vec::new();

        let query_str = get_file_entities_query(repo_names);

        let mut q = query(&query_str).param("file_path", file_path);
        if repo_names.len() == 1 {
            q = q.param("repo_name", repo_names[0].as_str());
        } else if repo_names.len() > 1 {
            q = q.param("repo_names", repo_names.to_vec());
        }

        let mut rows = self
            .graph
            .execute(q)
            .await
            .context("Failed to query Neo4j for file entities")?;

        while let Ok(Some(row)) = rows.next().await {
            let decorators = row.get::<Vec<String>>("e.decorators").unwrap_or_default();

            let entity_json = serde_json::json!({
                "name": row.get::<String>("e.name").ok(),
                "kind": row.get::<String>("e.kind").ok(),
                "signature": row.get::<String>("e.signature").ok(),
                "docstring": row.get::<String>("e.docstring").ok(),
                "start_line": row.get::<i64>("e.start_line").ok(),
                "decorators": decorators,
            });
            results.push(entity_json);
        }

        Ok(serde_json::json!(results))
    }

    async fn find_entities_by_name_prefix(
        &self,
        prefix: &str,
        repo_names: &[String],
        limit: usize,
    ) -> Result<serde_json::Value> {
        let repo_clause = if !repo_names.is_empty() {
            " AND m.repo_name IN $repo_names"
        } else {
            ""
        };

        let query_str = format!(
            "MATCH (m:Entity)
             WHERE toLower(m.name) STARTS WITH toLower($prefix){repo_clause}
             OPTIONAL MATCH (m)-[:CALLS]->(dep:Entity)
             RETURN m.uuid AS uuid, m.name, m.kind, m.fqn, m.signature, m.docstring,
                    m.file_path, m.start_line, m.repo_name AS repo_name,
                    collect(dep.name) as dependencies
             ORDER BY CASE WHEN toLower(m.name) = toLower($prefix) THEN 0 ELSE 1 END,
                      size(m.name),
                      m.fqn,
                      m.uuid
             LIMIT $limit"
        );

        let mut q = query(&query_str)
            .param("prefix", prefix)
            .param("limit", limit as i64);
        if !repo_names.is_empty() {
            q = q.param("repo_names", repo_names.to_vec());
        }

        let mut rows = self
            .graph
            .execute(q)
            .await
            .context("Failed to query Neo4j for entities by name prefix")?;

        let mut results = Vec::new();
        while let Ok(Some(row)) = rows.next().await {
            let entity_json = serde_json::json!({
                "uuid": row.get::<String>("uuid").ok(),
                "name": row.get::<String>("m.name").ok(),
                "kind": row.get::<String>("m.kind").ok(),
                "fqn": row.get::<String>("m.fqn").ok(),
                "signature": row.get::<String>("m.signature").ok(),
                "docstring": row.get::<String>("m.docstring").ok(),
                "file_path": row.get::<String>("m.file_path").ok(),
                "start_line": row.get::<i64>("m.start_line").ok(),
                "repo_name": row.get::<String>("repo_name").ok(),
                "dependencies": row.get::<Vec<String>>("dependencies").unwrap_or_default(),
            });
            results.push(entity_json);
        }

        Ok(serde_json::json!(results))
    }

    async fn get_file_outgoing_references(
        &self,
        file_path: &str,
        repo_names: &[String],
    ) -> Result<serde_json::Value> {
        let mut results = Vec::new();

        let query_str = get_file_outgoing_references_query(repo_names);

        let mut q = query(&query_str).param("file_path", file_path);
        if repo_names.len() == 1 {
            q = q
                .param("repo_name", repo_names[0].as_str())
                .param("repo_names", repo_names.to_vec());
        } else if repo_names.len() > 1 {
            q = q.param("repo_names", repo_names.to_vec());
        }

        let mut rows = self
            .graph
            .execute(q)
            .await
            .context("Failed to query Neo4j for file outgoing references")?;

        while let Ok(Some(row)) = rows.next().await {
            let entry = serde_json::json!({
                "rel": row.get::<String>("rel").ok(),
                "name": row.get::<String>("name").ok(),
                "kind": row.get::<String>("kind").ok(),
                "file_path": row.get::<String>("file_path").ok(),
                "line": row.get::<i64>("line").ok(),
            });
            results.push(entry);
        }

        Ok(serde_json::json!(results))
    }

    async fn find_files_by_suffix(
        &self,
        suffix_fragment: &str,
        repo_names: &[String],
    ) -> Result<serde_json::Value> {
        // `suffix_fragment` is the post-`WHERE` text, e.g.
        // "ENDS WITH '/src/lib.rs'". We hardcode the rest of the WHERE so
        // callers cannot inject arbitrary Cipher; the fragment is built by
        // `ends_with_suffix_query` which only ever interpolates a string
        // literal, so SQL/Cipher injection is not possible here.
        let query_str = find_files_by_suffix_query(suffix_fragment, repo_names);
        let mut q = query(&query_str);
        if !repo_names.is_empty() {
            q = q.param("repo_names", repo_names.to_vec());
        }

        let mut rows = self
            .graph
            .execute(q)
            .await
            .context("Failed to query Neo4j for files by suffix")?;

        let mut results = Vec::new();
        while let Ok(Some(row)) = rows.next().await {
            results.push(serde_json::json!({
                "file_path": row.get::<String>("file_path").ok(),
                "repo_name": row.get::<String>("repo_name").ok(),
            }));
        }
        Ok(serde_json::json!(results))
    }

    /// Read-only file listing (see [`list_files_query`]).
    async fn list_files(
        &self,
        prefix: &str,
        repo_names: &[String],
        limit: usize,
    ) -> Result<serde_json::Value> {
        let query_str = list_files_query(!repo_names.is_empty());
        let mut q = query(&query_str)
            .param("prefix", prefix.to_string())
            .param("limit", limit as i64);
        if !repo_names.is_empty() {
            q = q.param("repo_names", repo_names.to_vec());
        }

        let mut rows = self
            .graph
            .execute(q)
            .await
            .context("Failed to query Neo4j for file listing")?;

        let mut results = Vec::new();
        while let Ok(Some(row)) = rows.next().await {
            results.push(serde_json::json!({
                "file_path": row.get::<String>("file_path").ok(),
                "repo_name": row.get::<String>("repo_name").ok(),
                "entity_count": row.get::<i64>("entity_count").ok(),
            }));
        }
        Ok(serde_json::json!(results))
    }

    /// Caller-recall bridge: `(caller_uuid, target_uuid)` edges. One
    /// bounded query; a failure surfaces to the caller, which treats the
    /// bridge as best-effort.
    async fn find_caller_links(
        &self,
        target_uuids: &[String],
        repo_names: &[String],
        limit: usize,
    ) -> Result<Vec<(String, String)>> {
        if target_uuids.is_empty() {
            return Ok(Vec::new());
        }
        let query_str = caller_links_query(!repo_names.is_empty());
        let mut q = query(&query_str)
            .param("target_uuids", target_uuids.to_vec())
            .param("limit", limit as i64);
        if !repo_names.is_empty() {
            q = q.param("repo_names", repo_names.to_vec());
        }

        let mut rows = self
            .graph
            .execute(q)
            .await
            .context("Failed to query Neo4j for caller-recall bridge")?;

        let mut links = Vec::new();
        while let Ok(Some(row)) = rows.next().await {
            if let (Ok(caller), Ok(target)) = (
                row.get::<String>("caller_uuid"),
                row.get::<String>("target_uuid"),
            ) {
                links.push((caller, target));
            }
        }
        Ok(links)
    }
}

fn parse_reference_row(row: neo4rs::Row) -> serde_json::Value {
    serde_json::json!({
        "name": row.get::<String>("entity.name").ok(),
        "kind": row.get::<String>("entity.kind").ok(),
        "file_path": row.get::<String>("entity.file_path").ok(),
        "start_line": row.get::<i64>("entity.start_line").ok(),
        "signature": row.get::<String>("entity.signature").ok(),
        "repo_name": row.get::<String>("repo_name").ok(),
        "target_name": row.get::<String>("target_name").ok(),
        "target_fqn": row.get::<String>("target_fqn").ok(),
        "target_file_path": row.get::<String>("target_file_path").ok(),
        "target_start_line": row.get::<i64>("target_start_line").ok(),
        "target_signature": row.get::<String>("target_signature").ok(),
        "target_repo_name": row.get::<String>("target_repo_name").ok(),
        // Present only in the multi-label reference query; absent (from the
        // old single-label shape without the rel alias) it maps to `None`,
        // which the partition falls back to `references` for.
        "rel_type": row.get::<String>("rel_type").ok(),
    })
}

/// Bucket-key mapping from a Cypher relationship label to the JSON result
/// key `find_references` fills. Rows from the multi-label query land here;
/// `OVERRIDES` rows need nothing here — the `overridden_by` / `overrides`
/// buckets come from their own mirrored queries (Stage 3).
fn reference_bucket_key(rel_type: &str) -> Option<&'static str> {
    match rel_type {
        "CALLS" => Some("calls"),
        "EXTENDS" => Some("extends"),
        "IMPLEMENTS" => Some("implements"),
        "REFERENCES" => Some("references"),
        "MACRO_CALLS" => Some("macro_calls"),
        "REFERENCES_DOM" => Some("references_dom"),
        "USES_CSS_CLASS" => Some("uses_css_class"),
        "IMPORTS_SCRIPT" => Some("imports_script"),
        "IMPORTS_STYLESHEET" => Some("imports_stylesheet"),
        "USES_BACKEND" => Some("uses_backend"),
        "USES_PROBE" => Some("uses_probe"),
        "USES_ACL" => Some("uses_acl"),
        "INCLUDES" => Some("includes"),
        "IMPORTS_VMOD" => Some("imports_vmod"),
        "DECLARED_UNUSED" => Some("declared_unused"),
        _ => None,
    }
}

/// Distribute multi-label reference rows into the result buckets.
///
/// Deliberately a **stable** partition: the Cypher carries a global
/// `ORDER BY target.fqn, entity.file_path, entity.start_line`, and walking
/// the rows in query order while appending preserves that per-bucket order
/// exactly — the determinism contract the v1.9.4 fix pinned.
///
/// Rows lacking `rel_type` (legacy single-label wire shape) fall back to
/// `references`; rows with an unknown `rel_type` are skipped rather than
/// misfiled.
fn partition_reference_rows(rows: &[serde_json::Value], results: &mut serde_json::Value) {
    for row in rows {
        let bucket = match row.get("rel_type").and_then(|v| v.as_str()) {
            None => "references",
            Some(rel_type) => match reference_bucket_key(rel_type) {
                Some(bucket) => bucket,
                None => continue,
            },
        };
        if let Some(arr) = results.get_mut(bucket).and_then(|v| v.as_array_mut()) {
            arr.push(row.clone());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::GraphDb;
    use super::QueryExt;
    use crate::db::graph::connection::ConnectExt;

    #[ignore = "requires local Neo4j instance running on bolt://localhost:7687"]
    #[tokio::test]
    async fn test_get_entities_with_dependencies_empty() {
        let graph_db = GraphDb::connect("bolt://localhost:7687", "neo4j", "password")
            .await
            .expect("Failed to connect to Neo4j");

        let result = graph_db.get_entities_with_dependencies(&[], &[]).await;
        assert!(result.is_ok());
        let json = result.unwrap();
        assert!(json.is_array());
        assert_eq!(json.as_array().unwrap().len(), 0);
    }

    #[ignore = "requires local Neo4j instance running on bolt://localhost:7687"]
    #[tokio::test]
    async fn test_get_entities_with_dependencies() {
        let graph_db = GraphDb::connect("bolt://localhost:7687", "neo4j", "password")
            .await
            .expect("Failed to connect to Neo4j");

        let uuids = vec!["550e8400-e29b-41d4-a716-446655440000".to_string()];
        let result = graph_db
            .get_entities_with_dependencies(&uuids, &["test-repo".to_string()])
            .await;
        // Should not fail even if UUID doesn't exist
        assert!(result.is_ok());
    }

    #[ignore = "requires local Neo4j instance running on bolt://localhost:7687"]
    #[tokio::test]
    async fn test_find_references() {
        let graph_db = GraphDb::connect("bolt://localhost:7687", "neo4j", "password")
            .await
            .expect("Failed to connect to Neo4j");

        let result = graph_db
            .find_references("nonexistent_entity", &[], DEFAULT_MAX_TARGETS, None)
            .await;
        assert!(result.is_ok());
        let json = result.unwrap();
        assert!(json.is_object());
        assert!(json.get("calls").is_some());
        assert!(json.get("extends").is_some());
        assert!(json.get("implements").is_some());
        assert!(json.get("references").is_some());
    }

    #[ignore = "requires local Neo4j instance running on bolt://localhost:7687"]
    #[tokio::test]
    async fn test_find_references_with_repo() {
        let graph_db = GraphDb::connect("bolt://localhost:7687", "neo4j", "password")
            .await
            .expect("Failed to connect to Neo4j");

        let result = graph_db
            .find_references(
                "nonexistent_entity",
                &["test-repo".to_string()],
                DEFAULT_MAX_TARGETS,
                None,
            )
            .await;
        assert!(result.is_ok());
    }

    #[ignore = "requires local Neo4j instance running on bolt://localhost:7687"]
    #[tokio::test]
    async fn test_find_callers() {
        let graph_db = GraphDb::connect("bolt://localhost:7687", "neo4j", "password")
            .await
            .expect("Failed to connect to Neo4j");

        let result = graph_db.find_callers("nonexistent_entity", &[]).await;
        assert!(result.is_ok());
        let json = result.unwrap();
        assert!(json.is_array());
    }

    #[ignore = "requires local Neo4j instance running on bolt://localhost:7687"]
    #[tokio::test]
    async fn test_find_callers_with_repo() {
        let graph_db = GraphDb::connect("bolt://localhost:7687", "neo4j", "password")
            .await
            .expect("Failed to connect to Neo4j");

        let result = graph_db
            .find_callers("nonexistent_entity", &["test-repo".to_string()])
            .await;
        assert!(result.is_ok());
    }

    #[ignore = "requires local Neo4j instance running on bolt://localhost:7687"]
    #[tokio::test]
    async fn test_get_file_entities() {
        let graph_db = GraphDb::connect("bolt://localhost:7687", "neo4j", "password")
            .await
            .expect("Failed to connect to Neo4j");

        let result = graph_db
            .get_file_entities("/test/path/File.java", &[])
            .await;
        assert!(result.is_ok());
        let json = result.unwrap();
        assert!(json.is_array());
    }

    #[ignore = "requires local Neo4j instance running on bolt://localhost:7687"]
    #[tokio::test]
    async fn test_get_file_entities_with_repo() {
        let graph_db = GraphDb::connect("bolt://localhost:7687", "neo4j", "password")
            .await
            .expect("Failed to connect to Neo4j");

        let result = graph_db
            .get_file_entities("/test/path/File.java", &["test-repo".to_string()])
            .await;
        assert!(result.is_ok());
    }

    #[ignore = "requires local Neo4j instance running on bolt://localhost:7687"]
    #[tokio::test]
    async fn param_binding_uses_repo_names_list() {
        let graph_db = GraphDb::connect("bolt://localhost:7687", "neo4j", "password")
            .await
            .expect("Failed to connect to Neo4j");

        let rows = graph_db
            .collect_reference_rows(
                "MATCH (target:Entity) WHERE target.repo_name IN $repo_names RETURN target.name AS entity.name",
                &["uuid-1".to_string()],
                &["repo1".to_string(), "repo2".to_string()],
                "test",
            )
            .await;
        assert!(rows.is_ok());
    }

    use super::{
        DEFAULT_MAX_TARGETS, MAX_TARGETS_CEILING, MatchTier, RootCandidate, TargetRow,
        caller_links_query, finalize_targets, find_callers_query, find_files_by_suffix_query,
        find_references_rel_labels, fold_hidden, get_file_entities_query,
        get_file_outgoing_references_query, overridden_by_query, overrides_query,
        partition_by_kind, partition_reference_rows, rank_root_candidates, reference_bucket_key,
        reference_target_query, relationship_query, root_kind_rank, target_resolution_tiers,
    };

    #[test]
    fn test_tier_ladder_order_for_plain_name() {
        // A plain name skips FqnSuffix and ExactFqn: every FQN ends with `.<name>`, so the
        // suffix tier would shadow the more precise ExactName tier, and bare name FQN matching is indistinguishable from exact name.
        // "Offline" has len 7, so it gets Fuzzy tier, but no signature prefix because no '('
        let tiers = target_resolution_tiers("Offline");
        let mapped: Vec<MatchTier> = tiers.iter().map(|(t, _)| *t).collect();
        assert_eq!(mapped, vec![MatchTier::ExactName, MatchTier::Fuzzy]);

        // "Off" has len 3, so it does not get Fuzzy
        let tiers = target_resolution_tiers("Off");
        let mapped: Vec<MatchTier> = tiers.iter().map(|(t, _)| *t).collect();
        assert_eq!(mapped, vec![MatchTier::ExactName]);
    }

    #[test]
    fn test_tier_ladder_includes_fqn_suffix_for_qualified_name() {
        for name in ["GestureOwner.Off", "Config::load"] {
            let tiers = target_resolution_tiers(name);
            let mapped: Vec<MatchTier> = tiers.iter().map(|(t, _)| *t).collect();
            assert_eq!(
                mapped,
                vec![
                    MatchTier::ExactFqn,
                    MatchTier::FqnSuffix,
                    MatchTier::ExactName,
                    MatchTier::Fuzzy
                ],
                "unexpected ladder for `{name}`"
            );
        }
    }

    #[test]
    fn test_tier_ladder_includes_signature_prefix_when_parenthesised() {
        let tiers = target_resolution_tiers("accept(List");
        let mapped: Vec<MatchTier> = tiers.iter().map(|(t, _)| *t).collect();
        assert_eq!(
            mapped,
            vec![
                MatchTier::ExactName,
                MatchTier::SignaturePrefix,
                MatchTier::Fuzzy
            ]
        );
    }

    #[test]
    fn test_tier_ladder_omits_fuzzy_for_short_names() {
        let tiers = target_resolution_tiers("Id");
        let mapped: Vec<MatchTier> = tiers.iter().map(|(t, _)| *t).collect();
        assert_eq!(mapped, vec![MatchTier::ExactName]);
    }

    #[test]
    fn test_dotted_query_starts_with_exact_fqn() {
        let tiers = target_resolution_tiers("Foo.bar");
        let mapped: Vec<MatchTier> = tiers.iter().map(|(t, _)| *t).collect();
        assert_eq!(mapped[0], MatchTier::ExactFqn);

        let tiers = target_resolution_tiers("Foo::bar");
        let mapped: Vec<MatchTier> = tiers.iter().map(|(t, _)| *t).collect();
        assert_eq!(mapped[0], MatchTier::ExactFqn);
    }

    #[test]
    fn test_fqn_suffix_predicate_is_separator_anchored() {
        let tiers = target_resolution_tiers("GestureOwner.Off");
        let fqn_suffix_pred = tiers
            .iter()
            .find(|(t, _)| *t == MatchTier::FqnSuffix)
            .unwrap()
            .1;
        assert!(fqn_suffix_pred.contains("ENDS WITH '.' + $name"));
        assert!(fqn_suffix_pred.contains("ENDS WITH '::' + $name"));
        assert!(!fqn_suffix_pred.contains("CONTAINS"));
    }

    #[test]
    fn test_signature_predicate_is_prefix_anchored() {
        let tiers = target_resolution_tiers("accept(List");
        let sig_pred = tiers
            .iter()
            .find(|(t, _)| *t == MatchTier::SignaturePrefix)
            .unwrap()
            .1;
        assert!(sig_pred.contains("STARTS WITH $name"));
        assert!(!sig_pred.contains("CONTAINS"));
    }

    #[test]
    fn test_relationship_query_matches_on_uuid_set() {
        let query_str = relationship_query("CALLS", false);
        assert!(query_str.contains("target.uuid IN $target_uuids"));
        assert!(query_str.contains("ORDER BY target.fqn"));
        assert!(!query_str.contains("$repo_names"));
    }

    #[test]
    fn test_relationship_query_repo_scoped_variant() {
        let query_str = relationship_query("CALLS", true);
        assert!(query_str.contains("target.repo_name IN $repo_names"));
        assert!(query_str.contains("target.uuid IN $target_uuids"));
    }

    // ---- §Kind-filtered resolution + multi-label reference query ----------

    use crate::cli_tools::kinds::KindFilter;

    fn target_row(uuid: &str, kind: &str) -> TargetRow {
        TargetRow {
            uuid: uuid.to_string(),
            name: uuid.to_string(),
            fqn: format!("repo::{uuid}"),
            kind: kind.to_string(),
            file_path: "f.rs".to_string(),
            start_line: 1,
            repo_name: "repo".to_string(),
        }
    }

    #[test]
    fn fuzzy_tier_predicate_is_case_insensitive_and_param_bound() {
        let tiers = target_resolution_tiers("hikari");
        let (_, fuzzy) = tiers
            .iter()
            .find(|(t, _)| *t == MatchTier::Fuzzy)
            .expect("fuzzy tier for 6-char name");
        assert!(fuzzy.contains("toLower("));
        assert!(fuzzy.contains("$name_lower"));
        // The raw case-sensitive form must be gone.
        assert!(!fuzzy.contains("target.fqn CONTAINS $name "));
        assert!(!fuzzy.contains(") CONTAINS $name OR"));
    }

    #[test]
    fn exact_tiers_keep_case_sensitive_exactness() {
        let tiers = target_resolution_tiers("Hikari");
        for (tier, pred) in &tiers {
            if matches!(
                tier,
                MatchTier::ExactFqn | MatchTier::FqnSuffix | MatchTier::ExactName
            ) {
                assert!(
                    pred.contains("$name"),
                    "exact tier {tier:?} must match the raw name"
                );
                assert!(!pred.contains("toLower"));
            }
        }
    }

    #[test]
    fn partition_by_kind_separates_code_from_metadata() {
        let rows = vec![
            target_row("u1", "rust_function"),
            target_row("u2", "build_dependency"),
            target_row("u3", "markdown_section"),
            target_row("u4", "html_id"),
            target_row("u5", "vcl_backend"),
        ];
        let default = KindFilter::parse(None);
        let (allowed, hidden) = partition_by_kind(rows, &default);
        let names: Vec<&str> = allowed.iter().map(|r| r.uuid.as_str()).collect();
        assert_eq!(names, vec!["u1", "u4", "u5"]);
        assert_eq!(hidden.len(), 2);
    }

    #[test]
    fn partition_by_kind_honours_any_filter() {
        let rows = vec![
            target_row("u1", "rust_function"),
            target_row("u2", "build_dependency"),
        ];
        let any = KindFilter::parse(Some("all"));
        let (allowed, hidden) = partition_by_kind(rows, &any);
        assert_eq!(allowed.len(), 2);
        assert!(hidden.is_empty());
    }

    #[test]
    fn fold_hidden_dedupes_by_uuid_and_collects_sorted_kinds() {
        let mut uuids = std::collections::HashSet::new();
        let mut kinds: Vec<String> = Vec::new();
        let n1 = fold_hidden(
            vec![
                target_row("u1", "build_dependency"),
                target_row("u2", "markdown_section"),
            ],
            &mut uuids,
            &mut kinds,
        );
        // Same row seen again in a later tier (uuid duplicate), new kind.
        let n2 = fold_hidden(
            vec![
                target_row("u1", "build_dependency"),
                target_row("u3", "cargo_feature"),
            ],
            &mut uuids,
            &mut kinds,
        );
        assert_eq!(n1, 2);
        assert_eq!(n2, 1, "the repeated uuid must not recount");
        assert_eq!(
            kinds,
            vec!["build_dependency", "cargo_feature", "markdown_section"]
        );
    }

    #[test]
    fn rel_label_list_covers_every_produced_edge_type() {
        let labels = find_references_rel_labels();
        for label in [
            "CALLS",
            "EXTENDS",
            "IMPLEMENTS",
            "REFERENCES",
            "MACRO_CALLS",
            "REFERENCES_DOM",
            "USES_CSS_CLASS",
            "IMPORTS_SCRIPT",
            "IMPORTS_STYLESHEET",
            "USES_BACKEND",
            "USES_PROBE",
            "USES_ACL",
            "INCLUDES",
            "IMPORTS_VMOD",
            "DECLARED_UNUSED",
        ] {
            assert!(labels.contains(label), "{label} missing from {labels}");
        }
        // Deliberately excluded edge types.
        assert!(!labels.contains("CONTAINS"));
        assert!(!labels.contains("DEPENDS_ON"));
        assert!(!labels.contains("OVERRIDES"));
    }

    #[test]
    fn relationship_query_projects_rel_type() {
        let query_str = relationship_query(find_references_rel_labels(), false);
        assert!(query_str.contains("type(r) AS rel_type"));
        assert!(query_str.contains("|MACRO_CALLS"));
        assert!(query_str.contains("ORDER BY target.fqn"));
    }

    #[test]
    fn parse_reference_row_carries_rel_type() {
        // parse_reference_row needs a real neo4rs::Row to run; the mapping
        // it feeds is what this test pins. Every produced label maps to its
        // own bucket; unknown labels are skipped by the partition.
        assert_eq!(reference_bucket_key("CALLS"), Some("calls"));
        assert_eq!(reference_bucket_key("EXTENDS"), Some("extends"));
        assert_eq!(reference_bucket_key("IMPLEMENTS"), Some("implements"));
        assert_eq!(reference_bucket_key("REFERENCES"), Some("references"));
        assert_eq!(reference_bucket_key("MACRO_CALLS"), Some("macro_calls"));
        assert_eq!(
            reference_bucket_key("REFERENCES_DOM"),
            Some("references_dom")
        );
        assert_eq!(
            reference_bucket_key("USES_CSS_CLASS"),
            Some("uses_css_class")
        );
        assert_eq!(
            reference_bucket_key("IMPORTS_SCRIPT"),
            Some("imports_script")
        );
        assert_eq!(
            reference_bucket_key("IMPORTS_STYLESHEET"),
            Some("imports_stylesheet")
        );
        assert_eq!(reference_bucket_key("USES_BACKEND"), Some("uses_backend"));
        assert_eq!(reference_bucket_key("INCLUDES"), Some("includes"));
        assert_eq!(
            reference_bucket_key("DECLARED_UNUSED"),
            Some("declared_unused")
        );
        assert_eq!(reference_bucket_key("CONTAINS"), None);
        assert_eq!(reference_bucket_key("DEPENDS_ON"), None);
    }

    #[test]
    fn partition_reference_rows_is_order_preserving_per_bucket() {
        // The Cypher returns rows globally ordered; the per-bucket order
        // must be the order of arrival (v1.9.4 determinism contract).
        let rows: Vec<serde_json::Value> = [
            ("REFERENCES", "a"),
            ("MACRO_CALLS", "b1"),
            ("MACRO_CALLS", "b2"),
            ("REFERENCES", "c"),
            ("REFERENCES_DOM", "d"),
        ]
        .iter()
        .map(|(rel, name)| {
            serde_json::json!({"rel_type": rel, "name": name, "file_path": "f.rs", "start_line": 1})
        })
        .collect();

        let mut results = serde_json::json!({
            "calls": [],
            "extends": [],
            "implements": [],
            "references": [],
            "macro_calls": [],
            "references_dom": [],
        });
        partition_reference_rows(&rows, &mut results);

        let calls: Vec<&str> = results["references"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r.get("name").and_then(|v| v.as_str()).unwrap())
            .collect();
        assert_eq!(calls, vec!["a", "c"], "bucket keeps arrival order");

        let macros: Vec<&str> = results["macro_calls"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r.get("name").and_then(|v| v.as_str()).unwrap())
            .collect();
        assert_eq!(macros, vec!["b1", "b2"]);
        assert_eq!(results["references_dom"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn partition_reference_rows_falls_back_for_legacy_rows() {
        // Pre-fix rows carry no rel_type; they were REFERENCES-only.
        let rows = vec![serde_json::json!({"name": "a", "file_path": "f.rs"})];
        let mut results = serde_json::json!({
            "calls": [],
            "extends": [],
            "implements": [],
            "references": [],
        });
        partition_reference_rows(&rows, &mut results);
        assert_eq!(results["references"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn test_overridden_by_query_unscoped() {
        let query_str = overridden_by_query(false);
        assert!(query_str.contains("target.uuid IN $target_uuids"));
        assert!(!query_str.contains("$repo_names"));
    }

    #[test]
    fn test_overridden_by_query_repo_scoped() {
        let query_str = overridden_by_query(true);
        assert!(query_str.contains("target.repo_name IN $repo_names"));
        assert!(query_str.contains("target.uuid IN $target_uuids"));
    }

    #[test]
    fn test_overrides_query_unscoped() {
        let query_str = overrides_query(false);
        assert!(query_str.contains("entity.uuid IN $target_uuids"));
        assert!(!query_str.contains("$repo_names"));
    }

    #[test]
    fn test_overrides_query_repo_scoped() {
        let query_str = overrides_query(true);
        assert!(query_str.contains("entity.repo_name IN $repo_names"));
        assert!(query_str.contains("entity.uuid IN $target_uuids"));
    }

    #[test]
    fn test_find_callers_query_unscoped() {
        let query_str = find_callers_query(&[]);
        assert!(!query_str.contains("$repo_names"));
    }

    #[test]
    fn test_find_callers_query_repo_scoped() {
        let query_str = find_callers_query(&["a".to_string()]);
        assert!(query_str.contains("callee.repo_name IN $repo_names"));
    }

    // ---- reference repo attribution (v1.8.1) ----

    #[test]
    fn reference_target_query_preserves_tier_projection() {
        let scoped = reference_target_query("target.fqn = $name", true);
        assert!(scoped.contains("MATCH (target:Entity)"));
        assert!(scoped.contains("WHERE target.repo_name IN $repo_names AND (target.fqn = $name)"));
        assert!(scoped.contains(
            "RETURN target.uuid, target.name, target.fqn, target.kind, target.file_path"
        ));
        assert!(scoped.contains("target.start_line, target.repo_name"));
        assert!(scoped.contains("ORDER BY target.fqn"));

        let unscoped = reference_target_query("target.fqn = $name", false);
        assert!(unscoped.contains("WHERE (target.fqn = $name)"));
        assert!(!unscoped.contains("$repo_names"));
    }

    #[test]
    fn relationship_query_projects_both_repo_names() {
        for repo_scoped in [false, true] {
            let query_str = relationship_query("CALLS", repo_scoped);
            assert!(
                query_str.contains("entity.repo_name AS repo_name"),
                "repo_scoped={repo_scoped}: {query_str}"
            );
            assert!(
                query_str.contains("target.repo_name AS target_repo_name"),
                "repo_scoped={repo_scoped}: {query_str}"
            );
        }
    }

    #[test]
    fn overridden_by_query_projects_both_repo_names() {
        for repo_scoped in [false, true] {
            let query_str = overridden_by_query(repo_scoped);
            assert!(
                query_str.contains("entity.repo_name AS repo_name"),
                "repo_scoped={repo_scoped}: {query_str}"
            );
            assert!(
                query_str.contains("target.repo_name AS target_repo_name"),
                "repo_scoped={repo_scoped}: {query_str}"
            );
        }
    }

    #[test]
    fn overrides_query_projects_mirrored_repo_aliases() {
        for repo_scoped in [false, true] {
            let query_str = overrides_query(repo_scoped);
            // Mirrored projection: the Cipher `target` node is the row's
            // entity and the Cipher `entity` node is the row's target, so
            // the aliases MUST be swapped. Getting this backwards is silent.
            assert!(
                query_str.contains("target.repo_name AS repo_name"),
                "repo_scoped={repo_scoped}: {query_str}"
            );
            assert!(
                query_str.contains("entity.repo_name AS target_repo_name"),
                "repo_scoped={repo_scoped}: {query_str}"
            );
            assert!(
                !query_str.contains("entity.repo_name AS repo_name"),
                "unswapped alias leaked into the mirrored query: {query_str}"
            );
        }
    }

    #[test]
    fn reference_target_query_projects_repo_name() {
        for repo_scoped in [false, true] {
            let query_str = reference_target_query("target.name = $name", repo_scoped);
            assert!(
                query_str.contains("target.repo_name"),
                "repo_scoped={repo_scoped}: {query_str}"
            );
        }
    }

    #[test]
    fn find_callers_query_projects_caller_repo_name() {
        let unscoped = find_callers_query(&[]);
        assert!(unscoped.contains("caller.repo_name AS repo_name"));
        let scoped = find_callers_query(&["a".to_string()]);
        assert!(scoped.contains("caller.repo_name AS repo_name"));
    }

    #[test]
    fn reference_queries_keep_existing_order_by() {
        let rel = relationship_query("CALLS", true);
        assert!(rel.contains("ORDER BY target.fqn, entity.file_path, entity.start_line"));
        let overridden_by = overridden_by_query(true);
        assert!(overridden_by.contains("ORDER BY target.fqn, entity.file_path, entity.start_line"));
        let overrides = overrides_query(true);
        assert!(overrides.contains("ORDER BY entity.file_path, entity.start_line"));
        let tier = reference_target_query("target.name = $name", true);
        assert!(tier.contains("ORDER BY target.fqn"));
    }

    #[test]
    fn target_row_serializes_repo_name() {
        let row = TargetRow {
            uuid: "uuid-1".to_string(),
            name: "work".to_string(),
            fqn: "scope_alpha::src::shared_util::SharedUtil::work".to_string(),
            kind: "method".to_string(),
            file_path: "src/shared_util.ts".to_string(),
            start_line: 3,
            repo_name: "scope_alpha".to_string(),
        };
        let serialized = serde_json::to_string(&row).expect("serialize TargetRow");
        assert!(serialized.contains("\"repo_name\":\"scope_alpha\""));
        let deserialized: TargetRow =
            serde_json::from_str(&serialized).expect("deserialize TargetRow");
        assert_eq!(deserialized.repo_name, "scope_alpha");
    }

    #[test]
    fn test_get_file_entities_query_unscoped() {
        let query_str = get_file_entities_query(&[]);
        assert!(query_str.contains("MATCH (e:Entity {file_path: $file_path})"));
        assert!(!query_str.contains("repo_name"));
    }

    #[test]
    fn caller_links_query_is_scoped_and_capped() {
        let unscoped = caller_links_query(false);
        assert!(unscoped.contains("-[:CALLS]->"));
        assert!(unscoped.contains("target.uuid IN $target_uuids"));
        assert!(
            !unscoped.contains("caller.repo_name"),
            "unscoped must not filter repo"
        );
        assert!(unscoped.contains("LIMIT $limit"));
        assert!(unscoped.contains("target.uuid AS target_uuid"));

        let scoped = caller_links_query(true);
        assert!(scoped.contains("caller.repo_name IN $repo_names"));
    }

    #[test]
    fn test_get_file_entities_query_single_repo() {
        let query_str = get_file_entities_query(&["repo-a".to_string()]);
        assert!(
            query_str.contains("MATCH (e:Entity {file_path: $file_path, repo_name: $repo_name})")
        );
    }

    #[test]
    fn test_get_file_entities_query_multi_repo() {
        let query_str = get_file_entities_query(&["repo-a".to_string(), "repo-b".to_string()]);
        assert!(
            query_str.contains("WHERE e.file_path = $file_path AND e.repo_name IN $repo_names")
        );
    }

    #[test]
    fn test_get_file_outgoing_references_query_unscoped() {
        let query_str = get_file_outgoing_references_query(&[]);
        assert!(query_str.contains("WHERE dst.file_path <> $file_path"));
        assert!(!query_str.contains("$repo_names"));
    }

    #[test]
    fn test_get_file_outgoing_references_query_repo_scoped() {
        let query_str = get_file_outgoing_references_query(&["repo-a".to_string()]);
        assert!(query_str.contains("NOT dst.repo_name IN $repo_names"));
    }

    #[test]
    fn test_find_files_by_suffix_query_unscoped() {
        let query_str = find_files_by_suffix_query("ENDS WITH '/Cargo.toml'", &[]);
        assert!(!query_str.contains("$repo_names"));
    }

    #[test]
    fn test_find_files_by_suffix_query_repo_scoped() {
        let query_str =
            find_files_by_suffix_query("ENDS WITH '/Cargo.toml'", &["repo-a".to_string()]);
        assert!(query_str.contains("e.repo_name IN $repo_names"));
    }

    // ---- §5.1 root_kind_rank ----

    #[test]
    fn test_root_kind_rank_prefers_type_declarations_over_callables() {
        // `csharp_class` (rank 0) beats `csharp_constructor` (rank 1).
        assert!(root_kind_rank(Some("csharp_class")) < root_kind_rank(Some("csharp_constructor")));
        // And the plain generic forms too.
        assert!(root_kind_rank(Some("class")) < root_kind_rank(Some("method")));
        assert!(root_kind_rank(Some("class")) < root_kind_rank(Some("function")));
    }

    #[test]
    fn test_root_kind_rank_containers_rank_below_types() {
        // Namespaces are containers, not type declarations.
        assert!(root_kind_rank(Some("csharp_class")) < root_kind_rank(Some("csharp_namespace")));
        assert!(root_kind_rank(Some("rust_struct")) < root_kind_rank(Some("rust_module")));
        assert!(root_kind_rank(Some("python_class")) < root_kind_rank(Some("python_module")));
        assert!(root_kind_rank(Some("cpp_class")) < root_kind_rank(Some("cpp_namespace")));
    }

    #[test]
    fn test_root_kind_rank_handles_missing_kind() {
        assert_eq!(root_kind_rank(None), 4);
    }

    #[test]
    fn test_root_kind_rank_is_total_over_known_kinds() {
        // Iterate every variant of EntityKind via its Display impl. None
        // should panic and every result must be <= 4.
        use crate::models::EntityKind;
        let kinds = [
            EntityKind::Class,
            EntityKind::Interface,
            EntityKind::Method,
            EntityKind::Function,
            EntityKind::Constant,
            EntityKind::Enum,
            EntityKind::KotlinClass,
            EntityKind::KotlinInterface,
            EntityKind::KotlinObject,
            EntityKind::KotlinCompanionObject,
            EntityKind::KotlinFunction,
            EntityKind::KotlinMethod,
            EntityKind::KotlinProperty,
            EntityKind::KotlinEnum,
            EntityKind::RustStruct,
            EntityKind::RustEnum,
            EntityKind::RustUnion,
            EntityKind::RustTrait,
            EntityKind::RustImpl,
            EntityKind::RustFunction,
            EntityKind::RustMethod,
            EntityKind::RustMacroDef,
            EntityKind::RustTypeAlias,
            EntityKind::RustConstant,
            EntityKind::RustStatic,
            EntityKind::RustModule,
            EntityKind::PythonClass,
            EntityKind::PythonFunction,
            EntityKind::PythonMethod,
            EntityKind::PythonModule,
            EntityKind::PythonConstant,
            EntityKind::CStruct,
            EntityKind::CFunction,
            EntityKind::CppClass,
            EntityKind::CppMethod,
            EntityKind::CppNamespace,
            EntityKind::MacroDefinition,
            EntityKind::CSharpClass,
            EntityKind::CSharpInterface,
            EntityKind::CSharpStruct,
            EntityKind::CSharpRecord,
            EntityKind::CSharpEnum,
            EntityKind::CSharpMethod,
            EntityKind::CSharpConstructor,
            EntityKind::CSharpProperty,
            EntityKind::CSharpField,
            EntityKind::CSharpConstant,
            EntityKind::CSharpDelegate,
            EntityKind::CSharpEvent,
            EntityKind::CSharpIndexer,
            EntityKind::CSharpOperator,
            EntityKind::CSharpNamespace,
            EntityKind::CSharpLocalFunction,
            EntityKind::GroovyClass,
            EntityKind::GroovyInterface,
            EntityKind::GroovyTrait,
            EntityKind::GroovyMethod,
            EntityKind::GroovyFunction,
            EntityKind::GroovyEnum,
            EntityKind::GroovyProperty,
            EntityKind::BuildDependency,
            EntityKind::BuildPlugin,
            EntityKind::ProjectIdentity,
            EntityKind::MarkdownDocument,
            EntityKind::MarkdownSection,
            EntityKind::ConfigProperty,
        ];
        for k in kinds {
            let s = k.to_string();
            let rank = root_kind_rank(Some(&s));
            assert!(rank <= 4, "rank for {s} should be <= 4, got {rank}");
        }
    }

    // ---- §5.2 rank_root_candidates ----

    #[test]
    fn test_rank_root_candidates_prefers_type_over_homonym() {
        // Mirrors the UserService.cs fixture: `csharp_class` at line 12 and
        // `csharp_constructor` at line 18 share the name `UserService`.
        let candidates = vec![
            RootCandidate {
                uuid: "uuid-constructor".to_string(),
                name: "UserService".to_string(),
                fqn: Some("CodeMap.Services.UserService.UserService".to_string()),
                kind: Some("csharp_constructor".to_string()),
                signature: None,
                docstring: None,
                file_path: Some("Services/UserService.cs".to_string()),
                start_line: Some(18),
            },
            RootCandidate {
                uuid: "uuid-class".to_string(),
                name: "UserService".to_string(),
                fqn: Some("CodeMap.Services.UserService".to_string()),
                kind: Some("csharp_class".to_string()),
                signature: None,
                docstring: None,
                file_path: Some("Services/UserService.cs".to_string()),
                start_line: Some(12),
            },
        ];
        let ranked = rank_root_candidates(candidates);
        assert_eq!(ranked.len(), 2);
        assert_eq!(ranked[0].kind.as_deref(), Some("csharp_class"));
        assert_eq!(ranked[1].kind.as_deref(), Some("csharp_constructor"));
    }

    #[test]
    fn test_rank_root_candidates_tie_breaks_by_file_then_line_then_uuid() {
        // Two classes with the same name and kind: file_path breaks the tie.
        let a = RootCandidate {
            uuid: "uuid-a".to_string(),
            name: "Config".to_string(),
            fqn: Some("a.Config".to_string()),
            kind: Some("csharp_class".to_string()),
            signature: None,
            docstring: None,
            file_path: Some("src/z/config.cs".to_string()),
            start_line: Some(10),
        };
        let b = RootCandidate {
            uuid: "uuid-b".to_string(),
            name: "Config".to_string(),
            fqn: Some("a.Config".to_string()),
            kind: Some("csharp_class".to_string()),
            signature: None,
            docstring: None,
            file_path: Some("src/a/config.cs".to_string()),
            start_line: Some(50),
        };
        let ranked = rank_root_candidates(vec![a, b]);
        assert_eq!(ranked[0].file_path.as_deref(), Some("src/a/config.cs"));
        assert_eq!(ranked[1].file_path.as_deref(), Some("src/z/config.cs"));

        // Now same path, different lines: lower line wins.
        let c = RootCandidate {
            uuid: "uuid-c".to_string(),
            name: "Config".to_string(),
            fqn: Some("a.Config".to_string()),
            kind: Some("csharp_class".to_string()),
            signature: None,
            docstring: None,
            file_path: Some("src/a/config.cs".to_string()),
            start_line: Some(100),
        };
        let d = RootCandidate {
            uuid: "uuid-d".to_string(),
            name: "Config".to_string(),
            fqn: Some("a.Config".to_string()),
            kind: Some("csharp_class".to_string()),
            signature: None,
            docstring: None,
            file_path: Some("src/a/config.cs".to_string()),
            start_line: Some(20),
        };
        let ranked = rank_root_candidates(vec![c, d]);
        assert_eq!(ranked[0].start_line, Some(20));
        assert_eq!(ranked[1].start_line, Some(100));

        // Same path, same line, different uuid: lexicographic uuid wins.
        let e = RootCandidate {
            uuid: "uuid-zzz".to_string(),
            name: "Config".to_string(),
            fqn: Some("a.Config".to_string()),
            kind: Some("csharp_class".to_string()),
            signature: None,
            docstring: None,
            file_path: Some("src/a/config.cs".to_string()),
            start_line: Some(20),
        };
        let f = RootCandidate {
            uuid: "uuid-aaa".to_string(),
            name: "Config".to_string(),
            fqn: Some("a.Config".to_string()),
            kind: Some("csharp_class".to_string()),
            signature: None,
            docstring: None,
            file_path: Some("src/a/config.cs".to_string()),
            start_line: Some(20),
        };
        let ranked = rank_root_candidates(vec![e, f]);
        assert_eq!(ranked[0].uuid, "uuid-aaa");
        assert_eq!(ranked[1].uuid, "uuid-zzz");
    }

    #[test]
    fn test_rank_root_candidates_is_stable_for_equal_keys() {
        // Equal (rank, file_path, start_line, uuid) must preserve input order
        // (Vec::sort_by is stable).
        let make = |uuid: &str| RootCandidate {
            uuid: uuid.to_string(),
            name: "Same".to_string(),
            fqn: Some("x.Same".to_string()),
            kind: Some("csharp_class".to_string()),
            signature: None,
            docstring: None,
            file_path: Some("file.cs".to_string()),
            start_line: Some(1),
        };
        let input = vec![make("uuid-1"), make("uuid-2"), make("uuid-3")];
        let ranked = rank_root_candidates(input);
        assert_eq!(
            ranked.iter().map(|c| c.uuid.as_str()).collect::<Vec<_>>(),
            vec!["uuid-1", "uuid-2", "uuid-3"],
            "stable sort must preserve input order for equal keys"
        );
    }

    #[test]
    fn test_rank_root_candidates_handles_missing_fields() {
        // Missing kind → rank 4 (treated as a tail catch-all).
        // Missing file_path / start_line defaults to "" / 0.
        let a = RootCandidate {
            uuid: "uuid-no-kind".to_string(),
            name: "M".to_string(),
            fqn: None,
            kind: None,
            signature: None,
            docstring: None,
            file_path: Some("a/x.cs".to_string()),
            start_line: Some(10),
        };
        let b = RootCandidate {
            uuid: "uuid-class".to_string(),
            name: "M".to_string(),
            fqn: None,
            kind: Some("csharp_class".to_string()),
            signature: None,
            docstring: None,
            file_path: Some("a/x.cs".to_string()),
            start_line: Some(10),
        };
        let ranked = rank_root_candidates(vec![a, b]);
        assert_eq!(ranked[0].kind.as_deref(), Some("csharp_class"));
        assert_eq!(ranked[1].kind, None);
    }

    // ---- §Truncation quantification (v1.10.0) ----------------------------

    fn dummy_targets(n: usize) -> Vec<TargetRow> {
        (0..n)
            .map(|i| TargetRow {
                uuid: format!("uuid-{}", i),
                name: "delete".to_string(),
                fqn: format!("repo::module::delete_{}", i),
                kind: "method".to_string(),
                file_path: format!("src/file_{}.rs", i),
                start_line: i as i64,
                repo_name: String::new(),
            })
            .collect()
    }

    #[test]
    fn finalize_targets_total_is_pre_truncation_count() {
        // 40 rows capped at 25: `total` must report the real 40, not the
        // sample size 25. This is the regression pinned by the v1.10.0 fix —
        // measuring after `truncate` made the truncation notice read
        // "25 targets matched; showing the first 25".
        let resolved =
            finalize_targets(dummy_targets(40), MatchTier::ExactName, DEFAULT_MAX_TARGETS);
        assert_eq!(resolved.total, 40);
        assert_eq!(resolved.targets.len(), DEFAULT_MAX_TARGETS);
        assert!(resolved.truncated);
        assert!(!resolved.targets.is_empty());
    }

    #[test]
    fn finalize_targets_not_truncated_when_under_cap() {
        let resolved =
            finalize_targets(dummy_targets(10), MatchTier::ExactName, DEFAULT_MAX_TARGETS);
        assert_eq!(resolved.total, 10);
        assert_eq!(resolved.targets.len(), 10);
        assert!(!resolved.truncated);
    }

    #[test]
    fn finalize_targets_keeps_everything_when_cap_exceeds_total() {
        let resolved = finalize_targets(dummy_targets(40), MatchTier::ExactName, 100);
        assert_eq!(resolved.total, 40);
        assert_eq!(resolved.targets.len(), 40);
        assert!(!resolved.truncated);
    }

    #[test]
    fn finalize_targets_tier_is_carried_through() {
        let resolved =
            finalize_targets(dummy_targets(5), MatchTier::FqnSuffix, DEFAULT_MAX_TARGETS);
        assert!(matches!(resolved.tier, MatchTier::FqnSuffix));
    }

    #[test]
    fn find_references_resolution_clamps_ceiling() {
        // Pure assertion on the clamp expression used by find_references:
        // requested caps above MAX_TARGETS_CEILING are bounded, and 0 / tiny
        // values are bounded from below so the cap is always >= 1.
        assert_eq!(600usize.clamp(1, MAX_TARGETS_CEILING), MAX_TARGETS_CEILING);
        assert_eq!(0usize.clamp(1, MAX_TARGETS_CEILING), 1);
        assert_eq!(25usize.clamp(1, MAX_TARGETS_CEILING), 25);
    }
}
