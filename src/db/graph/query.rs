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

/// Maximum number of targets to return before truncating.
/// Keeps the performance reasonable and prevents huge outputs.
pub const MAX_TARGETS: usize = 25;

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
            "target.fqn CONTAINS $name OR (target.name + COALESCE(target.signature, '')) CONTAINS $name",
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

        // --- rank 4: everything else (build_, k8s_, html_, vtc_, project_identity, …)
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

pub fn relationship_query(rel_label: &str, repo_scoped: bool) -> String {
    let repo_filter = if repo_scoped {
        "WHERE target.repo_name IN $repo_names AND target.uuid IN $target_uuids"
    } else {
        "WHERE target.uuid IN $target_uuids"
    };

    format!(
        "MATCH (entity:Entity)-[:{rel_label}]->(target:Entity)
         {repo_filter}
         RETURN entity.name, entity.kind, entity.file_path, entity.start_line, entity.signature,
                entity.repo_name AS repo_name,
                target.name AS target_name, target.fqn AS target_fqn,
                target.file_path AS target_file_path,
                target.start_line AS target_start_line, target.signature AS target_signature,
                target.repo_name AS target_repo_name
         ORDER BY target.fqn, entity.file_path, entity.start_line"
    )
}

/// Cypher for the `overridden_by` bucket: implementations/overrides declared
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

/// Cypher for the `overrides` bucket: the supertype methods the resolved
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
    if repo_names.len() == 1 {
        "MATCH (src:Entity {file_path: $file_path, repo_name: $repo_name})
              -[r:REFERENCES|CALLS|EXTENDS|IMPLEMENTS]->
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
              -[r:REFERENCES|CALLS|EXTENDS|IMPLEMENTS]->
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
              -[r:REFERENCES|CALLS|EXTENDS|IMPLEMENTS]->
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

/// Cypher for one tier of the reference-target resolution ladder used by
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

impl GraphDb {
    async fn resolve_reference_targets(
        &self,
        name: &str,
        repo_names: &[String],
    ) -> Result<(Vec<TargetRow>, MatchTier, bool)> {
        let tiers = target_resolution_tiers(name);

        let repo_scoped = !repo_names.is_empty();

        for (tier, predicate) in tiers {
            let query_str = reference_target_query(predicate, repo_scoped);

            let mut q = query(&query_str).param("name", name);
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

            if !targets.is_empty() {
                let truncated = targets.len() > MAX_TARGETS;
                if truncated {
                    targets.truncate(MAX_TARGETS);
                }
                return Ok((targets, tier, truncated));
            }
        }

        let default_tier = if name.len() >= MIN_FUZZY_LEN {
            MatchTier::Fuzzy
        } else {
            MatchTier::ExactName
        };
        Ok((Vec::new(), default_tier, false))
    }

    /// Resolve a user-supplied entity name to exactly one root candidate,
    /// walking the same tier ladder as `resolve_reference_targets` with early
    /// stop, then applying `rank_root_candidates` inside the winning tier.
    ///
    /// Returns `(winner, tier, total_candidates)` — `Some(...)` only when the
    /// ladder produced at least one hit. `total_candidates` is the **un-truncated**
    /// tier count (the ladder queries `LIMIT 25` for ranking fairness; the
    /// caller may surface this in the disclosure).
    pub(crate) async fn resolve_subgraph_root(
        &self,
        name: &str,
        repo_name: &str,
    ) -> Result<Option<(RootCandidate, MatchTier, usize)>> {
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
                .param("repo_name", repo_name);

            let mut rows = self.graph.execute(q).await.context(format!(
                "Failed to resolve subgraph root for tier {}",
                tier.as_str()
            ))?;

            let mut candidates = Vec::new();
            while let Ok(Some(row)) = rows.next().await {
                let uuid = row.get::<String>("target.uuid").unwrap_or_default();
                let nm = row.get::<String>("target.name").unwrap_or_default();
                let fqn = row.get::<String>("target.fqn").ok();
                let kind = row.get::<String>("target.kind").ok();
                let signature = row.get::<String>("target.signature").ok();
                let docstring = row.get::<String>("target.docstring").ok();
                let file_path = row.get::<String>("target.file_path").ok();
                let start_line = row.get::<i64>("target.start_line").ok();

                candidates.push(RootCandidate {
                    uuid,
                    name: nm,
                    fqn,
                    kind,
                    signature,
                    docstring,
                    file_path,
                    start_line,
                });
            }

            if !candidates.is_empty() {
                let total = candidates.len();
                let ranked = rank_root_candidates(candidates);
                // Safe: ranked is non-empty (we just confirmed candidates
                // is non-empty before ranking).
                let winner = ranked.into_iter().next().expect("ranked non-empty");
                return Ok(Some((winner, tier, total)));
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
    /// fragment after `WHERE e.file_path ` in the Cypher query (e.g.
    /// `ENDS WITH '/Cargo.toml'`). Returns a list of distinct
    /// `(file_path, repo_name)` pairs that match.
    async fn find_files_by_suffix(
        &self,
        suffix_fragment: &str,
        repo_names: &[String],
    ) -> Result<serde_json::Value>;
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

    /// Find all entities that reference a given entity via any relationship type (CALLS, EXTENDS, IMPLEMENTS, REFERENCES).
    /// Returns results grouped by relationship type.
    async fn find_references(
        &self,
        entity_name: &str,
        repo_names: &[String],
    ) -> Result<serde_json::Value> {
        let mut results = serde_json::json!({
            "calls": [],
            "extends": [],
            "implements": [],
            "references": [],
            "overridden_by": [],
            "overrides": []
        });

        // Stage 1: Resolve targets
        let (targets, tier, truncated) = self
            .resolve_reference_targets(entity_name, repo_names)
            .await?;

        // Add the resolution info
        results["resolution"] = serde_json::json!({
            "query": entity_name,
            "tier": tier,
            "fuzzy": matches!(tier, MatchTier::Fuzzy),
            "truncated": truncated,
            "targets": targets
        });

        if targets.is_empty() {
            return Ok(results);
        }

        let target_uuids: Vec<String> = targets.iter().map(|t| t.uuid.clone()).collect();

        // Stage 2: Query relationships
        let rel_types = [
            ("CALLS", "calls"),
            ("EXTENDS", "extends"),
            ("IMPLEMENTS", "implements"),
            ("REFERENCES", "references"),
        ];

        for (rel_label, result_key) in rel_types {
            let query_str = relationship_query(rel_label, !repo_names.is_empty());
            let rows = self
                .collect_reference_rows(&query_str, &target_uuids, repo_names, rel_label)
                .await?;
            if let Some(arr) = results.get_mut(result_key) {
                *arr = serde_json::json!(rows);
            }
        }

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
        // callers cannot inject arbitrary Cypher; the fragment is built by
        // `ends_with_suffix_query` which only ever interpolates a string
        // literal, so SQL/Cypher injection is not possible here.
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
    })
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

        let result = graph_db.find_references("nonexistent_entity", &[]).await;
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
            .find_references("nonexistent_entity", &["test-repo".to_string()])
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
        MatchTier, RootCandidate, TargetRow, find_callers_query, find_files_by_suffix_query,
        get_file_entities_query, get_file_outgoing_references_query, overridden_by_query,
        overrides_query, rank_root_candidates, reference_target_query, relationship_query,
        root_kind_rank, target_resolution_tiers,
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

    // ---- §7.1 reference repo attribution (reference_repo_attribution_plan.md) ----

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
            // Mirrored projection: the Cypher `target` node is the row's
            // entity and the Cypher `entity` node is the row's target, so
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
}
