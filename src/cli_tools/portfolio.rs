//! Portfolio report: inventory, dependency correlations, and rule-based recommendations.
//!
//! Aggregates repository metadata and `DEPENDS_ON` edges into a structured report
//! suitable for architecture reviews or as grounded input for a GenAI layer.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use crate::config::OutputFormat;
use crate::db::graph::{GraphDb, RepoQueryExt};

use super::repos::run_list_repos;

const HIGH_FAN_IN_THRESHOLD: usize = 3;
const SMALL_REPO_ENTITY_THRESHOLD: i64 = 50;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Criticality {
    High,
    Medium,
    Low,
}

impl Criticality {
    fn as_str(self) -> &'static str {
        match self {
            Self::High => "high",
            Self::Medium => "medium",
            Self::Low => "low",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Recommendation {
    pub action: &'static str,
    pub rationale: String,
}

/// Build a portfolio report for all indexed repositories (optionally filtered by name).
pub async fn run_portfolio(
    filter: Option<&str>,
    depth: u32,
    graph_db: &Arc<GraphDb>,
) -> anyhow::Result<serde_json::Value> {
    let repos_value = run_list_repos(filter, graph_db).await?;
    let repos: Vec<serde_json::Value> = repos_value.as_array().cloned().unwrap_or_default();

    let mut assets = Vec::with_capacity(repos.len());

    for repo in &repos {
        let name = repo_name(repo);
        let depends_on = graph_db
            .find_repo_dependencies(&name, depth)
            .await
            .unwrap_or_default();
        let depended_on_by = graph_db
            .find_repo_dependents(&name)
            .await
            .unwrap_or_default();

        let criticality = derive_criticality(depends_on.len(), depended_on_by.len());
        let recommendations =
            derive_recommendations(repo, &depends_on, &depended_on_by, criticality);

        assets.push(serde_json::json!({
            "name": name,
            "primary_language": repo.get("primary_language").cloned().unwrap_or(serde_json::Value::Null),
            "build_system": repo.get("build_system").cloned().unwrap_or(serde_json::Value::Null),
            "file_count": repo.get("file_count").cloned().unwrap_or(serde_json::json!(0)),
            "entity_count": repo.get("entity_count").cloned().unwrap_or(serde_json::json!(0)),
            "depends_on": depends_on,
            "depended_on_by": depended_on_by,
            "criticality": criticality.as_str(),
            "recommendations": recommendations.iter().map(|r| serde_json::json!({
                "action": r.action,
                "rationale": r.rationale,
            })).collect::<Vec<_>>(),
        }));
    }

    let correlations = build_correlations(&assets);
    let summary = build_summary(&assets);

    Ok(serde_json::json!({
        "summary": summary,
        "assets": assets,
        "correlations": correlations,
    }))
}

fn repo_name(repo: &serde_json::Value) -> String {
    repo.get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_string()
}

fn derive_criticality(depends_on_count: usize, depended_on_by_count: usize) -> Criticality {
    if depended_on_by_count >= HIGH_FAN_IN_THRESHOLD {
        Criticality::High
    } else if depended_on_by_count > 0 || depends_on_count >= HIGH_FAN_IN_THRESHOLD {
        Criticality::Medium
    } else {
        Criticality::Low
    }
}

fn derive_recommendations(
    repo: &serde_json::Value,
    depends_on: &[String],
    depended_on_by: &[String],
    criticality: Criticality,
) -> Vec<Recommendation> {
    let entity_count = repo
        .get("entity_count")
        .and_then(|v| v.as_i64())
        .unwrap_or(0);
    let name = repo_name(repo);
    let mut recs = Vec::new();

    if depended_on_by.len() >= HIGH_FAN_IN_THRESHOLD {
        recs.push(Recommendation {
            action: "invest",
            rationale: format!(
                "'{name}' is depended on by {} repositories — treat as a platform or shared capability",
                depended_on_by.len()
            ),
        });
        recs.push(Recommendation {
            action: "harden",
            rationale:
                "High fan-in increases blast radius; add versioning, monitoring, and ownership"
                    .to_string(),
        });
    }

    if depends_on.is_empty() && depended_on_by.is_empty() {
        if entity_count <= SMALL_REPO_ENTITY_THRESHOLD {
            recs.push(Recommendation {
                action: "retire",
                rationale: format!(
                    "'{name}' has no cross-repo dependencies and few indexed entities ({entity_count}) — candidate for decommissioning review"
                ),
            });
        } else {
            recs.push(Recommendation {
                action: "register",
                rationale: format!(
                    "'{name}' appears isolated in the dependency graph — verify ownership and register in the application inventory"
                ),
            });
        }
    } else if depended_on_by.is_empty() && !depends_on.is_empty() {
        recs.push(Recommendation {
            action: "register",
            rationale: format!(
                "'{name}' consumes other repos but nothing depends on it — confirm it is still required"
            ),
        });
    }

    if depends_on.len() >= HIGH_FAN_IN_THRESHOLD {
        recs.push(Recommendation {
            action: "consolidate",
            rationale: format!(
                "'{name}' depends on {} other repositories — review coupling and duplication",
                depends_on.len()
            ),
        });
    }

    if criticality == Criticality::Low && recs.is_empty() {
        recs.push(Recommendation {
            action: "register",
            rationale: "Standard portfolio entry — document purpose and owner".to_string(),
        });
    }

    recs
}

fn build_summary(assets: &[serde_json::Value]) -> serde_json::Value {
    let mut languages: HashMap<String, usize> = HashMap::new();
    let mut build_systems: HashMap<String, usize> = HashMap::new();

    for asset in assets {
        if let Some(lang) = asset.get("primary_language").and_then(|v| v.as_str())
            && !lang.is_empty()
        {
            *languages.entry(lang.to_string()).or_default() += 1;
        }
        if let Some(bs) = asset.get("build_system").and_then(|v| v.as_str())
            && !bs.is_empty()
        {
            *build_systems.entry(bs.to_string()).or_default() += 1;
        }
    }

    serde_json::json!({
        "repo_count": assets.len(),
        "languages": languages,
        "build_systems": build_systems,
    })
}

fn build_correlations(assets: &[serde_json::Value]) -> Vec<serde_json::Value> {
    let mut correlations = Vec::new();
    let mut seen_hubs: HashSet<String> = HashSet::new();

    for asset in assets {
        let name = asset
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        let dependents: Vec<String> = asset
            .get("depended_on_by")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();

        if dependents.len() >= HIGH_FAN_IN_THRESHOLD && seen_hubs.insert(name.to_string()) {
            correlations.push(serde_json::json!({
                "type": "platform_hub",
                "repo": name,
                "dependents": dependents,
                "note": "Multiple repositories depend on this asset — single point of failure risk",
            }));
        }
    }

    let isolated: Vec<String> = assets
        .iter()
        .filter(|a| {
            let deps = a.get("depends_on").and_then(|v| v.as_array());
            let rev = a.get("depended_on_by").and_then(|v| v.as_array());
            deps.is_some_and(|d| d.is_empty()) && rev.is_some_and(|r| r.is_empty())
        })
        .filter_map(|a| a.get("name").and_then(|v| v.as_str()).map(str::to_string))
        .collect();

    if !isolated.is_empty() {
        correlations.push(serde_json::json!({
            "type": "isolated_repos",
            "repos": isolated,
            "note": "No cross-repo DEPENDS_ON edges — review for shadow IT or retirement",
        }));
    }

    correlations
}

pub fn format_portfolio_output(result: &serde_json::Value, output: OutputFormat) -> String {
    match output {
        OutputFormat::Json => serde_json::to_string_pretty(result).unwrap_or_default(),
        OutputFormat::Markdown => format_portfolio_markdown(result),
        OutputFormat::Table => format_portfolio_table(result),
    }
}

fn format_portfolio_table(result: &serde_json::Value) -> String {
    let mut out = String::new();
    let count = result
        .get("summary")
        .and_then(|s| s.get("repo_count"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    out.push_str(&format!("Portfolio report ({} repositories)\n\n", count));

    if let Some(assets) = result.get("assets").and_then(|v| v.as_array()) {
        out.push_str("REPO | LANGUAGE | BUILD | CRITICALITY | PRIMARY RECOMMENDATION\n");
        out.push_str("-----|----------|-------|-------------|---------------------\n");
        for asset in assets {
            let name = asset.get("name").and_then(|v| v.as_str()).unwrap_or("-");
            let lang = asset
                .get("primary_language")
                .and_then(|v| v.as_str())
                .unwrap_or("-");
            let build = asset
                .get("build_system")
                .and_then(|v| v.as_str())
                .unwrap_or("-");
            let crit = asset
                .get("criticality")
                .and_then(|v| v.as_str())
                .unwrap_or("-");
            let primary = asset
                .get("recommendations")
                .and_then(|v| v.as_array())
                .and_then(|arr| arr.first())
                .and_then(|r| r.get("action"))
                .and_then(|v| v.as_str())
                .unwrap_or("-");
            out.push_str(&format!("{name} | {lang} | {build} | {crit} | {primary}\n"));
        }
    }

    if let Some(correlations) = result.get("correlations").and_then(|v| v.as_array())
        && !correlations.is_empty()
    {
        out.push_str("\nCorrelations:\n");
        for c in correlations {
            let kind = c.get("type").and_then(|v| v.as_str()).unwrap_or("?");
            let repo = c.get("repo").and_then(|v| v.as_str()).unwrap_or("");
            out.push_str(&format!("  [{kind}] {repo}\n"));
        }
    }

    out
}

fn format_portfolio_markdown(result: &serde_json::Value) -> String {
    let mut out = String::new();
    let count = result
        .get("summary")
        .and_then(|s| s.get("repo_count"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    out.push_str(&format!("# Portfolio report ({count} repositories)\n\n"));

    out.push_str("## Assets\n\n");
    out.push_str("| Repo | Language | Build | Entities | Depends on | Used by | Criticality | Recommendation |\n");
    out.push_str("|------|----------|-------|----------|------------|---------|-------------|----------------|\n");

    if let Some(assets) = result.get("assets").and_then(|v| v.as_array()) {
        for asset in assets {
            let name = asset.get("name").and_then(|v| v.as_str()).unwrap_or("-");
            let lang = asset
                .get("primary_language")
                .and_then(|v| v.as_str())
                .unwrap_or("-");
            let build = asset
                .get("build_system")
                .and_then(|v| v.as_str())
                .unwrap_or("-");
            let entities = asset
                .get("entity_count")
                .and_then(|v| v.as_i64())
                .map(|n| n.to_string())
                .unwrap_or_else(|| "-".to_string());
            let depends_on = join_repo_list(asset.get("depends_on"));
            let used_by = join_repo_list(asset.get("depended_on_by"));
            let crit = asset
                .get("criticality")
                .and_then(|v| v.as_str())
                .unwrap_or("-");
            let primary = asset
                .get("recommendations")
                .and_then(|v| v.as_array())
                .and_then(|arr| arr.first())
                .and_then(|r| r.get("action"))
                .and_then(|v| v.as_str())
                .unwrap_or("-");
            out.push_str(&format!(
                "| {name} | {lang} | {build} | {entities} | {depends_on} | {used_by} | {crit} | {primary} |\n"
            ));
        }
    }

    out.push_str("\n## Correlations\n\n");
    if let Some(correlations) = result.get("correlations").and_then(|v| v.as_array()) {
        if correlations.is_empty() {
            out.push_str("_No cross-repo patterns detected._\n");
        } else {
            for c in correlations {
                let kind = c.get("type").and_then(|v| v.as_str()).unwrap_or("unknown");
                let note = c.get("note").and_then(|v| v.as_str()).unwrap_or("");
                out.push_str(&format!("### {kind}\n\n"));
                if let Some(repo) = c.get("repo").and_then(|v| v.as_str()) {
                    out.push_str(&format!("- **{repo}** — {note}\n"));
                    if let Some(deps) = c.get("dependents").or_else(|| c.get("consumers")) {
                        out.push_str(&format!("  - Related: {}\n", join_repo_list(Some(deps))));
                    }
                } else if let Some(repos) = c.get("repos") {
                    out.push_str(&format!(
                        "- Repos: {} — {note}\n",
                        join_repo_list(Some(repos))
                    ));
                }
                out.push('\n');
            }
        }
    }

    out.push_str(
        "\n_Use `knot portfolio --output json` for machine-readable export. \
         Pair with an LLM via `knot-mcp` for narrative summaries per repo._\n",
    );
    out
}

fn join_repo_list(value: Option<&serde_json::Value>) -> String {
    value
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        })
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "—".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_derive_criticality_high_fan_in() {
        assert_eq!(derive_criticality(0, 3), Criticality::High);
        assert_eq!(derive_criticality(1, 5), Criticality::High);
    }

    #[test]
    fn test_derive_criticality_medium() {
        assert_eq!(derive_criticality(3, 1), Criticality::Medium);
        assert_eq!(derive_criticality(0, 1), Criticality::Medium);
    }

    #[test]
    fn test_derive_criticality_low() {
        assert_eq!(derive_criticality(1, 0), Criticality::Low);
        assert_eq!(derive_criticality(0, 0), Criticality::Low);
    }

    #[test]
    fn test_derive_recommendations_platform_hub() {
        let repo = json!({"name": "lib-common", "entity_count": 500});
        let recs = derive_recommendations(
            &repo,
            &["other".to_string()],
            &["a".to_string(), "b".to_string(), "c".to_string()],
            Criticality::High,
        );
        assert!(recs.iter().any(|r| r.action == "invest"));
        assert!(recs.iter().any(|r| r.action == "harden"));
    }

    #[test]
    fn test_derive_recommendations_isolated_small() {
        let repo = json!({"name": "legacy-tool", "entity_count": 10});
        let recs = derive_recommendations(&repo, &[], &[], Criticality::Low);
        assert!(recs.iter().any(|r| r.action == "retire"));
    }

    #[test]
    fn test_derive_recommendations_isolated_large() {
        let repo = json!({"name": "shadow-app", "entity_count": 500});
        let recs = derive_recommendations(&repo, &[], &[], Criticality::Low);
        assert!(recs.iter().any(|r| r.action == "register"));
    }

    #[test]
    fn test_build_summary_counts_languages() {
        let assets = vec![
            json!({"primary_language": "rust", "build_system": "cargo"}),
            json!({"primary_language": "rust", "build_system": "npm"}),
            json!({"primary_language": "java", "build_system": "maven"}),
        ];
        let summary = build_summary(&assets);
        assert_eq!(summary["repo_count"], 3);
        assert_eq!(summary["languages"]["rust"], 2);
        assert_eq!(summary["languages"]["java"], 1);
    }

    #[test]
    fn test_format_portfolio_markdown_includes_assets() {
        let result = json!({
            "summary": {"repo_count": 1},
            "assets": [{
                "name": "my-api",
                "primary_language": "java",
                "build_system": "maven",
                "entity_count": 100,
                "depends_on": ["lib"],
                "depended_on_by": [],
                "criticality": "low",
                "recommendations": [{"action": "register", "rationale": "test"}]
            }],
            "correlations": []
        });
        let md = format_portfolio_markdown(&result);
        assert!(md.contains("# Portfolio report"));
        assert!(md.contains("my-api"));
        assert!(md.contains("register"));
    }

    #[test]
    fn test_join_repo_list_empty() {
        assert_eq!(join_repo_list(Some(&json!([]))), "—");
    }

    #[test]
    fn test_join_repo_list_multiple() {
        assert_eq!(join_repo_list(Some(&json!(["a", "b"]))), "a, b");
    }
}
