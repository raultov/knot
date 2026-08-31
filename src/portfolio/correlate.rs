use std::collections::{HashMap, HashSet};

use super::models::{Correlation, CorrelationKind, PortfolioSummary, RepoAsset, RepoRole};

pub fn enrich_assets(
    repos: &[serde_json::Value],
    dep_edges: &[(String, String)],
) -> (Vec<RepoAsset>, PortfolioSummary) {
    let mut dep_out: HashMap<String, usize> = HashMap::new();
    let mut dep_in: HashMap<String, usize> = HashMap::new();
    for (from, to) in dep_edges {
        *dep_out.entry(from.clone()).or_insert(0) += 1;
        *dep_in.entry(to.clone()).or_insert(0) += 1;
    }

    let total_entities: i64 = repos
        .iter()
        .filter_map(|r| r.get("entity_count").and_then(|v| v.as_i64()))
        .sum();
    let total_files: i64 = repos
        .iter()
        .filter_map(|r| r.get("file_count").and_then(|v| v.as_i64()))
        .sum();

    let mut language_counts: HashMap<String, i64> = HashMap::new();
    let mut build_counts: HashMap<String, i64> = HashMap::new();

    let assets: Vec<RepoAsset> = repos
        .iter()
        .map(|repo| {
            let name = repo
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let entity_count = repo
                .get("entity_count")
                .and_then(|v| v.as_i64())
                .unwrap_or(0);
            let file_count = repo.get("file_count").and_then(|v| v.as_i64()).unwrap_or(0);
            let build_system = repo
                .get("build_system")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let primary_language = repo
                .get("primary_language")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let group_id = repo
                .get("group_id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let artifact_id = repo
                .get("artifact_id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let version = repo
                .get("version")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let indexed_at = repo
                .get("indexed_at")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();

            if !primary_language.is_empty() {
                *language_counts.entry(primary_language.clone()).or_insert(0) += entity_count;
            }
            let bs_key = if build_system.is_empty() {
                "unknown".to_string()
            } else {
                build_system.clone()
            };
            *build_counts.entry(bs_key).or_insert(0) += entity_count;

            let dependency_count = dep_out.get(&name).copied().unwrap_or(0);
            let dependent_count = dep_in.get(&name).copied().unwrap_or(0);
            let role = classify_role(dependency_count, dependent_count);
            let weight_pct = if total_entities > 0 {
                (entity_count as f64 / total_entities as f64) * 100.0
            } else {
                0.0
            };

            RepoAsset {
                name,
                entity_count,
                file_count,
                build_system,
                primary_language,
                group_id,
                artifact_id,
                version,
                indexed_at,
                role,
                weight_pct,
                dependency_count,
                dependent_count,
                description: String::new(),
                readme_excerpt: String::new(),
                identity: String::new(),
            }
        })
        .collect();

    let summary = PortfolioSummary {
        repo_count: assets.len(),
        total_entities,
        total_files,
        language_allocation: allocation_pct(&language_counts, total_entities),
        build_system_allocation: allocation_pct(&build_counts, total_entities),
    };

    (assets, summary)
}

pub fn build_correlations(
    dep_edges: &[(String, String)],
    call_coupling: &[(String, String, i64)],
) -> Vec<Correlation> {
    let mut correlations = Vec::new();
    let mut seen: HashSet<(String, String, CorrelationKind)> = HashSet::new();

    for (from, to) in dep_edges {
        let key = (from.clone(), to.clone(), CorrelationKind::DependsOn);
        if seen.insert(key) {
            correlations.push(Correlation {
                from: from.clone(),
                to: to.clone(),
                kind: CorrelationKind::DependsOn,
                strength: 1,
            });
        }
    }

    for (from, to, count) in call_coupling {
        let key = (from.clone(), to.clone(), CorrelationKind::Calls);
        if seen.insert(key) {
            correlations.push(Correlation {
                from: from.clone(),
                to: to.clone(),
                kind: CorrelationKind::Calls,
                strength: *count,
            });
        }
    }

    correlations
}

pub fn classify_role(dependency_count: usize, dependent_count: usize) -> RepoRole {
    if dependent_count >= 3 {
        RepoRole::Hub
    } else if dependency_count > 0 && dependent_count == 0 {
        RepoRole::Leaf
    } else if dependency_count == 0 && dependent_count == 0 {
        RepoRole::Isolated
    } else {
        RepoRole::Balanced
    }
}

fn allocation_pct(counts: &HashMap<String, i64>, total: i64) -> Vec<(String, f64)> {
    if total <= 0 {
        return Vec::new();
    }
    let mut items: Vec<(String, f64)> = counts
        .iter()
        .map(|(k, v)| (k.clone(), (*v as f64 / total as f64) * 100.0))
        .collect();
    items.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    items
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_classify_role_hub() {
        assert_eq!(classify_role(1, 3), RepoRole::Hub);
    }

    #[test]
    fn test_classify_role_leaf() {
        assert_eq!(classify_role(2, 0), RepoRole::Leaf);
    }

    #[test]
    fn test_classify_role_isolated() {
        assert_eq!(classify_role(0, 0), RepoRole::Isolated);
    }

    #[test]
    fn test_classify_role_balanced() {
        assert_eq!(classify_role(1, 1), RepoRole::Balanced);
    }

    #[test]
    fn test_build_correlations_merges_dep_and_calls() {
        let deps = vec![("api".to_string(), "core".to_string())];
        let calls = vec![("api".to_string(), "core".to_string(), 42)];
        let corr = build_correlations(&deps, &calls);
        assert_eq!(corr.len(), 2);
        assert!(corr.iter().any(|c| c.kind == CorrelationKind::DependsOn));
        assert!(
            corr.iter()
                .any(|c| c.kind == CorrelationKind::Calls && c.strength == 42)
        );
    }
}
