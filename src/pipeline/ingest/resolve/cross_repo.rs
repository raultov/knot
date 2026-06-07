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

    if let Some(primary) = project_identities.iter().min_by_key(|e| {
        std::path::Path::new(&e.file_path)
            .strip_prefix(std::path::Path::new(&cfg.repo_path))
            .map(|p| p.components().count().saturating_sub(1))
            .unwrap_or(usize::MAX)
    }) {
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

pub(crate) fn parse_build_system_from_fqn(fqn: &str) -> &str {
    if fqn.starts_with("maven:") {
        "maven"
    } else if fqn.starts_with("gradle:") {
        "gradle"
    } else if fqn.starts_with("cargo:") {
        "cargo"
    } else if fqn.starts_with("npm:") {
        "npm"
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
        "cargo" => ("", rest),
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
    fn test_parse_version_from_signature() {
        assert_eq!(
            parse_version_from_signature(&Some("version: 1.0.0, build_system: maven".to_string())),
            "1.0.0"
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
