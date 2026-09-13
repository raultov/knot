use std::collections::BTreeMap;

use anyhow::Result;
use tracing::info;

use crate::config::Config;
use crate::db::graph::{GraphDb, RepoIdentity, RepoQueryExt, UpsertExt};
use crate::models::{EntityKind, ResolutionEntity};

pub async fn link_cross_repo_dependencies(
    entities: &[ResolutionEntity],
    graph_db: &GraphDb,
    cfg: &Config,
) -> Result<()> {
    let project_identities: Vec<&ResolutionEntity> = entities
        .iter()
        .filter(|e| e.kind == EntityKind::ProjectIdentity)
        .collect();

    let identity = match select_primary_identity(&project_identities, &cfg.repo_path) {
        Some(primary) => {
            let build_system = parse_build_system_from_fqn(&primary.fqn).to_string();
            let (group_id, artifact_id) = parse_artifact_identity(&primary.fqn, &build_system);
            let version = parse_version_from_signature(&primary.signature).to_string();

            graph_db
                .upsert_repository(
                    &cfg.repo_name,
                    &build_system,
                    group_id,
                    artifact_id,
                    &version,
                )
                .await?;

            Some(RepoIdentity {
                build_system,
                group_id: group_id.to_string(),
                artifact_id: artifact_id.to_string(),
                version,
            })
        }
        None => {
            // No ProjectIdentity in this batch: an incremental run that did
            // not re-parse the build manifest, or a repo without one. The
            // full `upsert_repository` would wipe the previously stored
            // identity (its `SET` is unconditional), leaving the repository
            // unmatchable by `find_repository_by_artifact` — so ensure the
            // node instead and read the persisted identity back for the
            // sweep below.
            graph_db.upsert_repository_node(&cfg.repo_name).await?;
            graph_db.find_repository_identity(&cfg.repo_name).await?
        }
    };

    // Candidate dependency names = this batch ∪ everything already persisted
    // for the repo. The union is what makes re-indexing a consumer create
    // edges to a library indexed afterwards, even when the consumer's
    // manifest is unchanged and therefore absent from the batch.
    let mut dep_names: Vec<String> = entities
        .iter()
        .filter(|e| e.kind == EntityKind::BuildDependency)
        .map(|e| e.name.clone())
        .collect();
    dep_names.extend(graph_db.find_build_dependency_names(&cfg.repo_name).await?);
    dep_names.sort();
    dep_names.dedup();

    for dep_name in &dep_names {
        if let Some(matched_repo) = match_dependency_to_repository(dep_name, graph_db).await?
            && matched_repo != cfg.repo_name
        {
            graph_db
                .upsert_repo_dependency(&cfg.repo_name, &matched_repo)
                .await?;
            info!(
                "Cross-repo link: '{}' -> '{}' (via build dependency: {})",
                cfg.repo_name, matched_repo, dep_name
            );
        }
    }

    // Reverse sweep: indexing the *library* side creates DEPENDS_ON edges
    // from already-indexed consumers without re-indexing them. Net contract:
    // re-indexing either side of the relationship creates the edge.
    if let Some(identity) = &identity {
        sweep_existing_consumers(&cfg.repo_name, identity, graph_db).await?;
    }

    Ok(())
}

/// Pick the primary `ProjectIdentity` for a repository.
///
/// NuGet's `PackageId` marker outranks the depth tie — a published package
/// wins over a depth-equal non-package project. Marker-aware selection is
/// build-system-agnostic in effect: only the NuGet parser emits the marker,
/// so non-NuGet identities participate in the unmodified `min_by_key` arm
/// that cross-repo e2e Test 8 pins for Maven/Gradle/Cargo/npm.
fn select_primary_identity<'a>(
    project_identities: &[&'a ResolutionEntity],
    repo_path: &str,
) -> Option<&'a ResolutionEntity> {
    if let Some(marked) = project_identities
        .iter()
        .find(|e| has_package_id_marker(&e.signature))
    {
        return Some(*marked);
    }
    project_identities
        .iter()
        .min_by_key(|e| {
            let p = std::path::Path::new(&e.file_path);
            let rel_path = if p.is_absolute() {
                p.strip_prefix(std::path::Path::new(repo_path)).unwrap_or(p)
            } else {
                p
            };
            rel_path.components().count().saturating_sub(1)
        })
        .copied()
}

/// Marker string emitted by the MSBuild parser when a project's identity
/// came from an explicit `<PackageId>` (see `msbuild.rs:PACKAGE_ID_MARKER`).
pub(crate) const PACKAGE_ID_MARKER: &str = "identity: package_id";

fn has_package_id_marker(signature: &Option<String>) -> bool {
    signature
        .as_deref()
        .is_some_and(|s| s.contains(PACKAGE_ID_MARKER))
}

pub(crate) fn parse_build_system_from_fqn(fqn: &str) -> &str {
    if fqn.starts_with("maven:") {
        "maven"
    } else if fqn.starts_with("gradle:") {
        "gradle"
    } else if fqn.starts_with("cargo:") {
        "cargo"
    } else if fqn.starts_with("npm:") {
        "npm"
    } else if fqn.starts_with("nuget:") {
        "nuget"
    } else {
        "unknown"
    }
}

pub(crate) fn parse_artifact_identity<'a>(fqn: &'a str, build_system: &str) -> (&'a str, &'a str) {
    let prefix = format!("{}:", build_system);
    let rest = fqn.strip_prefix(&prefix).unwrap_or(fqn);

    match build_system {
        "maven" | "gradle" => {
            let mut parts = rest.splitn(2, ':');
            (
                parts.next().unwrap_or("unknown"),
                parts.next().unwrap_or(rest),
            )
        }
        "cargo" | "nuget" => ("", rest),
        "npm" => parse_npm_scoped_name(rest),
        _ => ("", rest),
    }
}

fn parse_npm_scoped_name(name: &str) -> (&str, &str) {
    if name.starts_with('@') {
        let mut parts = name.splitn(2, '/');
        (
            parts.next().unwrap_or("unknown"),
            parts.next().unwrap_or(name),
        )
    } else {
        ("", name)
    }
}

pub(crate) fn parse_version_from_signature(signature: &Option<String>) -> &str {
    signature
        .as_deref()
        .and_then(|s| {
            s.strip_prefix("version: ")
                .and_then(|v| v.split(',').next())
        })
        .unwrap_or("unknown")
}

/// Order probes for the authoritative matcher: unambiguous prefixed
/// identities first, then the Maven-style branch, then cargo, npm and helm.
pub(crate) fn dependency_lookup_probes(dep_name: &str) -> Vec<(&str, &str, &str)> {
    let mut probes = Vec::new();

    // NuGet arm MUST come before the Maven-style branch because the prefix
    // `nuget:` contains no dot — `parse_maven_style_dep` would otherwise
    // strip it and read `nuget:Acme.Auth.Lib:1.0.0` as group="Acme.Auth.Lib",
    // artifact="1.0.0" (see §10.3 ordering hazard).
    if let Some(pkg) = dep_name.strip_prefix("nuget:")
        && let Some(name) = pkg.split(':').next()
    {
        probes.push(("nuget", "", name));
    }

    // Prefixed identities can never be Maven GAVs: the maven-style parse
    // would strip the prefix and misread the name. Skipping the two probes
    // also saves two `find_repository_by_artifact` round-trips per
    // dependency.
    if !dep_name.starts_with("npm:")
        && !dep_name.starts_with("nuget:")
        && !dep_name.starts_with("helm:")
        && let Some((group_id, artifact_id)) = parse_maven_style_dep(dep_name)
    {
        probes.push(("maven", group_id, artifact_id));
        probes.push(("gradle", group_id, artifact_id));
    }

    if let Some(crate_name) = dep_name.split(':').next()
        && !crate_name.contains('.')
        && crate_name != "helm"
        && crate_name != "npm"
        && crate_name != "nuget"
    {
        probes.push(("cargo", "", crate_name));
    }

    if let Some(pkg) = dep_name.strip_prefix("npm:") {
        let name = pkg.split(':').next().unwrap_or(pkg);
        let (group_id, artifact_id) = parse_npm_scoped_name(name);
        probes.push(("npm", group_id, artifact_id));
    }

    if let Some(chart) = dep_name.strip_prefix("helm:") {
        let name = chart.split(':').next().unwrap_or(chart);
        probes.push(("helm", "", name));
    }

    probes
}

async fn match_dependency_to_repository(
    dep_name: &str,
    graph_db: &GraphDb,
) -> Result<Option<String>> {
    for (build_system, group_id, artifact_id) in dependency_lookup_probes(dep_name) {
        if let Some(repo) = graph_db
            .find_repository_by_artifact(group_id, artifact_id, build_system)
            .await?
        {
            return Ok(Some(repo));
        }
    }
    Ok(None)
}

/// The most selective substring of a declared dependency name under which a
/// repository with this identity could appear. `None` when the identity is
/// too weak to sweep safely (empty/`unknown` artifact or maven group).
///
/// This must mirror the name formats the parsers emit:
/// - maven/gradle: `group:artifact:version` (optionally `config:` prefixed);
/// - cargo: `dep:version`;
/// - npm: `npm:dep:version` / `npm:@scope/dep:version`;
/// - nuget: `nuget:Name:version`.
///
/// Unprefixed helm names are deliberately excluded — the forward matcher
/// cannot resolve them today (see docs/agent-skills/deps.md limitations).
pub(crate) fn reverse_sweep_needle(
    build_system: &str,
    group_id: &str,
    artifact_id: &str,
) -> Option<String> {
    let usable = |v: &str| !v.is_empty() && v != "unknown";
    if !usable(artifact_id) {
        return None;
    }
    match build_system {
        "maven" | "gradle" => {
            if usable(group_id) {
                Some(format!("{group_id}:{artifact_id}"))
            } else {
                None
            }
        }
        "cargo" => Some(format!("{artifact_id}:")),
        "npm" => {
            if group_id.starts_with('@') {
                // Scoped identity: group_id holds the scope ("@acme"),
                // artifact_id the package ("ui-kit"); consumers declare
                // `npm:@acme/ui-kit:<range>`.
                Some(format!("npm:{group_id}/{artifact_id}:"))
            } else {
                Some(format!("npm:{artifact_id}:"))
            }
        }
        "nuget" => Some(format!("nuget:{artifact_id}:")),
        _ => None,
    }
}

/// Create `DEPENDS_ON` edges from already-indexed consumers to this
/// repository, so a dependency edge appears the moment the library itself is
/// indexed. Precision is delegated entirely to
/// `match_dependency_to_repository`: the Cypher `CONTAINS` filter only
/// narrows candidates (one `NodeIndexContainsScan` on the name text index),
/// and an edge is created only when the authoritative matcher resolves the
/// raw dependency name back to exactly this repository. There is no second
/// copy of the matching rules.
/// Resolve declared build-dependency names against the repository registry.
///
/// For each name (in input order — never re-sorted, callers rely on the
/// query's ORDER BY for stable output), returns the declared name verbatim
/// together with the repository it resolves to, or `None` when no indexed
/// repository matches. Resolution delegates entirely to
/// [`match_dependency_to_repository`], so the probe precedence is shared
/// with ingest linking — there is no second copy of the matching rules.
pub(crate) async fn resolve_declared_dependencies(
    dep_names: &[String],
    graph_db: &GraphDb,
) -> Result<Vec<(String, Option<String>)>> {
    let mut resolved = Vec::with_capacity(dep_names.len());
    for dep_name in dep_names {
        let repo = match_dependency_to_repository(dep_name, graph_db).await?;
        resolved.push((dep_name.clone(), repo));
    }
    Ok(resolved)
}

/// Indexed repositories that declare `repo_name` as a build dependency,
/// mapped to the first declared name that resolves back to it.
///
/// `Ok(None)` means the question is **unanswerable**: this repository has no
/// matchable build identity (`reverse_sweep_needle` returned `None` — e.g.
/// `build_system: "none"`, or a Maven identity without a group). This is
/// deliberately distinct from `Ok(Some(empty))`, which means every indexed
/// repository's declarations were checked and none resolves to it. Callers
/// (the reverse sweep at ingest, and the reverse diagnostics in
/// `cli_tools::deps`) must never render "nobody declares it" for `None`.
pub(crate) async fn find_declaring_consumers(
    repo_name: &str,
    identity: &RepoIdentity,
    graph_db: &GraphDb,
) -> Result<Option<BTreeMap<String, String>>> {
    let Some(needle) = reverse_sweep_needle(
        &identity.build_system,
        &identity.group_id,
        &identity.artifact_id,
    ) else {
        return Ok(None);
    };

    let candidates = graph_db
        .find_dependency_candidates(&needle, repo_name)
        .await?;

    let mut consumers: BTreeMap<String, String> = BTreeMap::new();
    for (consumer, dep_name) in candidates {
        if consumer == repo_name {
            continue;
        }
        if match_dependency_to_repository(&dep_name, graph_db)
            .await?
            .as_deref()
            == Some(repo_name)
            && !consumers.contains_key(&consumer)
        {
            consumers.insert(consumer, dep_name);
        }
    }
    Ok(Some(consumers))
}

async fn sweep_existing_consumers(
    repo_name: &str,
    identity: &RepoIdentity,
    graph_db: &GraphDb,
) -> Result<usize> {
    // `None` = no matchable identity ⇒ no consumer can declare this repo.
    let Some(consumers) = find_declaring_consumers(repo_name, identity, graph_db).await? else {
        return Ok(0);
    };

    for consumer in consumers.keys() {
        graph_db.upsert_repo_dependency(consumer, repo_name).await?;
        info!("Cross-repo link (reverse sweep): '{consumer}' -> '{repo_name}'");
    }
    Ok(consumers.len())
}

pub(crate) fn parse_maven_style_dep(dep_name: &str) -> Option<(&str, &str)> {
    let after_prefix = if let Some(colon_idx) = dep_name.find(':') {
        let prefix = &dep_name[..colon_idx];
        if prefix.contains('.') {
            dep_name
        } else {
            &dep_name[colon_idx + 1..]
        }
    } else {
        dep_name
    };

    let parts: Vec<&str> = after_prefix.split(':').collect();
    if parts.len() >= 2 {
        Some((parts[0], parts[1]))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_build_system_maven() {
        assert_eq!(
            parse_build_system_from_fqn("maven:com.example:app"),
            "maven"
        );
    }

    #[test]
    fn test_parse_build_system_cargo() {
        assert_eq!(parse_build_system_from_fqn("cargo:my-crate"), "cargo");
    }

    #[test]
    fn test_parse_build_system_npm() {
        assert_eq!(parse_build_system_from_fqn("npm:@scope/package"), "npm");
    }

    #[test]
    fn test_parse_build_system_gradle() {
        assert_eq!(
            parse_build_system_from_fqn("gradle:com.example:app"),
            "gradle"
        );
    }

    // ---- §11.5 NuGet wiring ----

    #[test]
    fn test_parse_build_system_nuget() {
        assert_eq!(parse_build_system_from_fqn("nuget:codemap-mcp"), "nuget");
    }

    #[test]
    fn test_parse_artifact_identity_nuget_flat() {
        // NuGet IDs are flat (no group), just like Cargo.
        let (gid, aid) = parse_artifact_identity("nuget:Acme.Auth.Lib", "nuget");
        assert_eq!(gid, "");
        assert_eq!(aid, "Acme.Auth.Lib");
    }

    #[test]
    fn test_parse_artifact_identity_nuget_with_version() {
        // Even with a version suffix the identity is the bare name.
        let (gid, aid) = parse_artifact_identity("nuget:Tomlyn:0.17.0", "nuget");
        assert_eq!(gid, "");
        assert_eq!(aid, "Tomlyn:0.17.0");
    }

    #[test]
    fn test_match_dependency_nuget_precedes_maven_style() {
        // Documents the ordering hazard: `nuget:` has no dot, so without
        // the explicit NuGet arm `parse_maven_style_dep` would strip the
        // prefix and read group="Acme.Auth.Lib", artifact="1.0.0". This
        // unit test pins the parse_maven_style_dep outcome so any
        // regression that reorders the matcher surfaces immediately.
        let result = parse_maven_style_dep("nuget:Acme.Auth.Lib:1.0.0");
        assert_eq!(
            result,
            Some(("Acme.Auth.Lib", "1.0.0")),
            "parse_maven_style_dep WOULD misfire if the NuGet arm were absent \
             — the NuGet arm in match_dependency_to_repository must run first"
        );
    }

    #[test]
    fn test_has_package_id_marker() {
        let marked = Some("version: 2.8.1, build_system: nuget, identity: package_id".to_string());
        assert!(has_package_id_marker(&marked));

        let unmarked = Some("version: 1.0.0, build_system: nuget".to_string());
        assert!(!has_package_id_marker(&unmarked));

        assert!(!has_package_id_marker(&None));
    }

    #[test]
    fn test_parse_artifact_identity_maven() {
        let (gid, aid) = parse_artifact_identity("maven:com.example:my-app", "maven");
        assert_eq!(gid, "com.example");
        assert_eq!(aid, "my-app");
    }

    #[test]
    fn test_parse_artifact_identity_cargo() {
        let (gid, aid) = parse_artifact_identity("cargo:my-crate", "cargo");
        assert_eq!(gid, "");
        assert_eq!(aid, "my-crate");
    }

    #[test]
    fn test_parse_artifact_identity_npm_scoped() {
        let (gid, aid) = parse_artifact_identity("npm:@scope/my-pkg", "npm");
        assert_eq!(gid, "@scope");
        assert_eq!(aid, "my-pkg");
    }

    #[test]
    fn test_parse_artifact_identity_npm_unscoped() {
        let (gid, aid) = parse_artifact_identity("npm:my-pkg", "npm");
        assert_eq!(gid, "");
        assert_eq!(aid, "my-pkg");
    }

    #[test]
    fn test_primary_selection_prefers_package_id_marker() {
        // Three NuGet identities at depth 2 — one carries the marker.
        // Marker-aware selection picks the marked one regardless of
        // alphabetical order or depth ties.
        use crate::models::{EntityKind, ResolutionEntity};
        use uuid::Uuid;

        let marked = ResolutionEntity {
            uuid: Uuid::new_v4(),
            name: "codemap-mcp".to_string(),
            fqn: "nuget:codemap-mcp".to_string(),
            file_path: "src/CodeMap.Daemon/CodeMap.Daemon.csproj".to_string(),
            kind: EntityKind::ProjectIdentity,
            enclosing_class: None,
            enclosing_class_fqn: None,
            signature: Some(
                "version: 2.8.1, build_system: nuget, identity: package_id".to_string(),
            ),
            reference_intents: vec![],
            relationships: vec![],
            alias_module_path: None,
            original_export_name: None,
            default_export: None,
            is_test_context: false,
        };
        let unmarked_shallow = ResolutionEntity {
            uuid: Uuid::new_v4(),
            name: "CodeMap".to_string(),
            fqn: "nuget:CodeMap".to_string(),
            file_path: "src/CodeMap.Core/Core.csproj".to_string(),
            kind: EntityKind::ProjectIdentity,
            enclosing_class: None,
            enclosing_class_fqn: None,
            signature: Some("version: 1.0.0, build_system: nuget".to_string()),
            reference_intents: vec![],
            relationships: vec![],
            alias_module_path: None,
            original_export_name: None,
            default_export: None,
            is_test_context: false,
        };
        let identities = vec![&unmarked_shallow, &marked];
        let primary = select_primary_identity(&identities, "/repo");
        assert_eq!(
            primary.map(|e| e.fqn.as_str()),
            Some("nuget:codemap-mcp"),
            "marker must win over depth-tied unmarked candidates"
        );
    }

    #[test]
    fn test_primary_selection_falls_back_to_shallowest_without_marker() {
        // No marker → falls back to shallowest-path rule (the existing
        // min_by_key behavior cross-repo e2e Test 8 pins for
        // Maven/Gradle/Cargo/npm).
        use crate::models::{EntityKind, ResolutionEntity};
        use uuid::Uuid;

        let root = ResolutionEntity {
            uuid: Uuid::new_v4(),
            name: "root-app".to_string(),
            fqn: "nuget:root-app".to_string(),
            file_path: "App.csproj".to_string(),
            kind: EntityKind::ProjectIdentity,
            enclosing_class: None,
            enclosing_class_fqn: None,
            signature: Some("version: 1.0.0, build_system: nuget".to_string()),
            reference_intents: vec![],
            relationships: vec![],
            alias_module_path: None,
            original_export_name: None,
            default_export: None,
            is_test_context: false,
        };
        let nested = ResolutionEntity {
            uuid: Uuid::new_v4(),
            name: "nested-app".to_string(),
            fqn: "nuget:nested-app".to_string(),
            file_path: "src/Nested/Nested.csproj".to_string(),
            kind: EntityKind::ProjectIdentity,
            enclosing_class: None,
            enclosing_class_fqn: None,
            signature: Some("version: 1.0.0, build_system: nuget".to_string()),
            reference_intents: vec![],
            relationships: vec![],
            alias_module_path: None,
            original_export_name: None,
            default_export: None,
            is_test_context: false,
        };
        let identities = vec![&nested, &root];
        let primary = select_primary_identity(&identities, "/repo");
        assert_eq!(
            primary.map(|e| e.fqn.as_str()),
            Some("nuget:root-app"),
            "shallowest wins when no marker is present"
        );
    }

    #[test]
    fn test_parse_version_from_signature() {
        assert_eq!(
            parse_version_from_signature(&Some("version: 1.0.0, build_system: maven".to_string())),
            "1.0.0"
        );
    }

    #[test]
    fn test_parse_version_from_signature_nuget_with_marker() {
        // The marker is inert for version extraction — only the `version: `
        // prefix is read.
        assert_eq!(
            parse_version_from_signature(&Some(
                "version: 2.8.1, build_system: nuget, identity: package_id".to_string()
            )),
            "2.8.1"
        );
    }

    #[test]
    fn test_parse_version_from_signature_none() {
        assert_eq!(parse_version_from_signature(&None), "unknown");
    }

    #[test]
    fn test_parse_maven_style_dep_standard() {
        let result = parse_maven_style_dep("org.springframework:spring-core:5.3.29");
        assert_eq!(result, Some(("org.springframework", "spring-core")));
    }

    #[test]
    fn test_parse_maven_style_dep_with_config() {
        let result = parse_maven_style_dep("implementation:org.springframework:spring-core:5.3.29");
        assert!(result.is_some());
        assert_eq!(result.unwrap().0, "org.springframework");
    }

    #[test]
    fn test_parse_maven_style_dep_short() {
        let result = parse_maven_style_dep("com.example:my-lib");
        assert_eq!(result, Some(("com.example", "my-lib")));
    }

    // ─── Dependency matching logic (not just parsing) ─────────────────────

    /// Pins the string form of `EntityKind::BuildDependency` used as a Cypher
    /// literal in `find_build_dependency_names` /
    /// `find_dependency_candidates` (`src/db/graph/query_repo.rs`) — if the
    /// `EntityKind` string table ever drifts, those queries would silently
    /// return nothing.
    #[test]
    fn test_build_dependency_kind_string_matches_cypher_literal() {
        assert_eq!(EntityKind::BuildDependency.to_string(), "build_dependency");
    }

    #[test]
    fn test_probes_npm_scoped() {
        let probes = dependency_lookup_probes("npm:@acme/ui-kit:^1.0.0");
        // The version range contains a ':' and must not leak into the id.
        assert_eq!(probes, vec![("npm", "@acme", "ui-kit")]);
    }

    #[test]
    fn test_probes_npm_unscoped() {
        let probes = dependency_lookup_probes("npm:react-hook-form:^7.84.0");
        assert_eq!(probes, vec![("npm", "", "react-hook-form")]);
    }

    #[test]
    fn test_probes_npm_skips_maven_style() {
        // npm deps are never Maven GAVs — the maven/gradle probes must be
        // skipped entirely (saves two round-trips and avoids prefix
        // misreads).
        let probes = dependency_lookup_probes("npm:react-hook-form:^7.84.0");
        assert!(
            probes
                .iter()
                .all(|(bs, _, _)| *bs != "maven" && *bs != "gradle"),
            "npm deps must not probe maven/gradle: {probes:?}"
        );
    }

    #[test]
    fn test_probes_nuget_first() {
        let probes = dependency_lookup_probes("nuget:Acme.Auth.Lib:1.0.0");
        assert_eq!(probes.first(), Some(&("nuget", "", "Acme.Auth.Lib")));
        assert!(
            probes
                .iter()
                .all(|(bs, _, _)| *bs != "maven" && *bs != "gradle"),
            "nuget deps must not probe maven/gradle (§10.3 ordering hazard): {probes:?}"
        );
    }

    #[test]
    fn test_probes_maven() {
        let probes = dependency_lookup_probes("org.springframework:spring-core:5.3.29");
        assert_eq!(
            probes,
            vec![
                ("maven", "org.springframework", "spring-core"),
                ("gradle", "org.springframework", "spring-core"),
            ]
        );
    }

    #[test]
    fn test_probes_gradle_with_config() {
        let probes = dependency_lookup_probes("implementation:org.example:real-dep:1.0.0");
        // The spurious ("cargo", "", "implementation") probe is pre-existing
        // behaviour of the original matcher ("implementation" has no dot and
        // is not a reserved prefix); it is harmless — no crate is named
        // "implementation" — and pinned here to document the order.
        assert_eq!(
            probes,
            vec![
                ("maven", "org.example", "real-dep"),
                ("gradle", "org.example", "real-dep"),
                ("cargo", "", "implementation"),
            ]
        );
    }

    #[test]
    fn test_probes_cargo() {
        let probes = dependency_lookup_probes("serde:1.0");
        assert_eq!(probes, vec![("cargo", "", "serde")]);
    }

    #[test]
    fn test_probes_helm_prefixed() {
        let probes = dependency_lookup_probes("helm:postgresql:14.11.0");
        assert_eq!(probes, vec![("helm", "", "postgresql")]);
    }

    /// Round-trip: what the primary-identity writer stored via
    /// `parse_artifact_identity` must be found by the matcher's probes for a
    /// dependency declared with the parser's name format.
    #[test]
    fn test_probe_matches_stored_identity_round_trip() {
        // scoped npm
        let (gid, aid) = parse_artifact_identity("npm:@acme/ui-kit", "npm");
        let declared = "npm:@acme/ui-kit:^1.0.0";
        assert!(dependency_lookup_probes(declared).contains(&("npm", gid, aid)));

        // unscoped npm
        let (gid, aid) = parse_artifact_identity("npm:chrome-devtools-mcp", "npm");
        let declared = "npm:chrome-devtools-mcp:^1.9.0";
        assert!(dependency_lookup_probes(declared).contains(&("npm", gid, aid)));

        // cargo
        let (gid, aid) = parse_artifact_identity("cargo:cdp-lite", "cargo");
        let declared = "cdp-lite:0.2";
        assert!(dependency_lookup_probes(declared).contains(&("cargo", gid, aid)));

        // nuget
        let (gid, aid) = parse_artifact_identity("nuget:Acme.Auth.Lib", "nuget");
        let declared = "nuget:Acme.Auth.Lib:1.0.0";
        assert!(dependency_lookup_probes(declared).contains(&("nuget", gid, aid)));

        // maven
        let (gid, aid) = parse_artifact_identity("maven:com.zaxxer:HikariCP", "maven");
        let declared = "com.zaxxer:HikariCP:5.1.0";
        assert!(dependency_lookup_probes(declared).contains(&("maven", gid, aid)));
    }

    // ─── Reverse sweep needles ─────────────────────────────────────────────

    #[test]
    fn test_reverse_sweep_needle_npm_scoped() {
        let needle = reverse_sweep_needle("npm", "@acme", "ui-kit");
        assert_eq!(needle.as_deref(), Some("npm:@acme/ui-kit:"));
    }

    #[test]
    fn test_reverse_sweep_needle_npm_unscoped() {
        let needle = reverse_sweep_needle("npm", "", "job-watch-ui");
        assert_eq!(needle.as_deref(), Some("npm:job-watch-ui:"));
    }

    #[test]
    fn test_reverse_sweep_needle_maven_gradle() {
        assert_eq!(
            reverse_sweep_needle("maven", "com.acme", "auth-lib").as_deref(),
            Some("com.acme:auth-lib")
        );
        assert_eq!(
            reverse_sweep_needle("gradle", "com.acme", "auth-lib").as_deref(),
            Some("com.acme:auth-lib")
        );
    }

    #[test]
    fn test_reverse_sweep_needle_cargo_nuget() {
        assert_eq!(
            reverse_sweep_needle("cargo", "", "cdp-lite").as_deref(),
            Some("cdp-lite:")
        );
        assert_eq!(
            reverse_sweep_needle("nuget", "", "Acme.Auth.Lib").as_deref(),
            Some("nuget:Acme.Auth.Lib:")
        );
    }

    #[test]
    fn test_reverse_sweep_needle_none_for_weak_identity() {
        assert_eq!(reverse_sweep_needle("npm", "", ""), None);
        assert_eq!(reverse_sweep_needle("npm", "", "unknown"), None);
        assert_eq!(reverse_sweep_needle("maven", "", "auth-lib"), None);
        assert_eq!(reverse_sweep_needle("maven", "unknown", "auth-lib"), None);
        assert_eq!(reverse_sweep_needle("none", "", "job-watch-ui"), None);
        assert_eq!(reverse_sweep_needle("helm", "", "postgresql"), None);
    }

    /// Closure test: for every supported identity, the needle must actually
    /// appear inside the dependency name a consumer would declare in that
    /// build system's parser format. This is the invariant the reverse sweep
    /// silently depends on; any parser that changes its name format must
    /// update this table.
    #[test]
    fn test_reverse_sweep_needle_is_substring_of_declared_names() {
        let cases: &[(&str, &str, &str, &[&str])] = &[
            (
                "npm",
                "@acme",
                "ui-kit",
                &["npm:@acme/ui-kit:^1.0.0", "npm:@acme/ui-kit:workspace:*"],
            ),
            ("npm", "", "job-watch-ui", &["npm:job-watch-ui:^0.4.5"]),
            (
                "cargo",
                "",
                "cdp-lite",
                &["cdp-lite:0.2", "cdp-lite:unknown"],
            ),
            (
                "maven",
                "com.acme",
                "auth-lib",
                &[
                    "com.acme:auth-lib:1.0.0",
                    "implementation:com.acme:auth-lib:1.0.0",
                ],
            ),
            (
                "gradle",
                "com.acme",
                "auth-lib",
                &[
                    "com.acme:auth-lib:1.0.0",
                    "implementation:com.acme:auth-lib:1.0.0",
                ],
            ),
            ("nuget", "", "Acme.Auth.Lib", &["nuget:Acme.Auth.Lib:1.0.0"]),
        ];

        for (build_system, group_id, artifact_id, declared_names) in cases {
            let needle =
                reverse_sweep_needle(build_system, group_id, artifact_id).unwrap_or_else(|| {
                    panic!("needle must exist for {build_system}:{group_id}:{artifact_id}")
                });
            for name in declared_names.iter() {
                assert!(
                    name.contains(&needle),
                    "needle '{needle}' must be a substring of declared name '{name}' \
                     (identity {build_system}:{group_id}:{artifact_id})"
                );
            }
        }
    }
}
