use anyhow::Result;
use tracing::info;

use crate::config::Config;
use crate::db::graph::{GraphDb, RepoQueryExt, UpsertExt};
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

    let primary = select_primary_identity(&project_identities, &cfg.repo_path);

    if let Some(primary) = primary {
        let build_system = parse_build_system_from_fqn(&primary.fqn);
        let (group_id, artifact_id) = parse_artifact_identity(&primary.fqn, build_system);

        graph_db
            .upsert_repository(
                &cfg.repo_name,
                build_system,
                group_id,
                artifact_id,
                parse_version_from_signature(&primary.signature),
            )
            .await?;
    } else {
        graph_db
            .upsert_repository(&cfg.repo_name, "none", "", "", "")
            .await?;
    }

    for entity in entities {
        if entity.kind == EntityKind::BuildDependency
            && let Some(matched_repo) =
                match_dependency_to_repository(&entity.name, graph_db).await?
            && matched_repo != cfg.repo_name
        {
            graph_db
                .upsert_repo_dependency(&cfg.repo_name, &matched_repo)
                .await?;
            info!(
                "Cross-repo link: '{}' -> '{}' (via build dependency: {})",
                cfg.repo_name, matched_repo, entity.name
            );
        }
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

async fn match_dependency_to_repository(
    dep_name: &str,
    graph_db: &GraphDb,
) -> Result<Option<String>> {
    // NuGet arm MUST come before the Maven-style branch because the prefix
    // `nuget:` contains no dot — `parse_maven_style_dep` would otherwise
    // strip it and read `nuget:Acme.Auth.Lib:1.0.0` as group="Acme.Auth.Lib",
    // artifact="1.0.0" (see §10.3 ordering hazard).
    if let Some(pkg) = dep_name.strip_prefix("nuget:")
        && let Some(name) = pkg.split(':').next()
        && let Some(repo) = graph_db
            .find_repository_by_artifact("", name, "nuget")
            .await?
    {
        return Ok(Some(repo));
    }

    if let Some((group_id, artifact_id)) = parse_maven_style_dep(dep_name) {
        if let Some(repo) = graph_db
            .find_repository_by_artifact(group_id, artifact_id, "maven")
            .await?
        {
            return Ok(Some(repo));
        }
        if let Some(repo) = graph_db
            .find_repository_by_artifact(group_id, artifact_id, "gradle")
            .await?
        {
            return Ok(Some(repo));
        }
    }

    if let Some(crate_name) = dep_name.split(':').next()
        && !crate_name.contains('.')
        && crate_name != "helm"
        && crate_name != "npm"
        && crate_name != "nuget"
        && let Some(repo) = graph_db
            .find_repository_by_artifact("", crate_name, "cargo")
            .await?
    {
        return Ok(Some(repo));
    }

    if let Some(pkg) = dep_name.strip_prefix("npm:") {
        let name = pkg.split(':').next().unwrap_or(pkg);
        let (group_id, artifact_id) = parse_npm_scoped_name(name);
        if let Some(repo) = graph_db
            .find_repository_by_artifact(group_id, artifact_id, "npm")
            .await?
        {
            return Ok(Some(repo));
        }
    }

    if let Some(chart) = dep_name.strip_prefix("helm:") {
        let name = chart.split(':').next().unwrap_or(chart);
        if let Some(repo) = graph_db
            .find_repository_by_artifact("", name, "helm")
            .await?
        {
            return Ok(Some(repo));
        }
    }

    Ok(None)
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
        // min_by_key behaviour cross-repo e2e Test 8 pins for
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
}
