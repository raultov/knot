use anyhow::Result;
use std::sync::Arc;

use crate::db::graph::{GraphDb, PortfolioQueryExt as _};

pub struct RawPortfolioData {
    pub repos: Vec<serde_json::Value>,
    pub dep_edges: Vec<(String, String)>,
    pub call_coupling: Vec<(String, String, i64)>,
}

pub async fn collect_portfolio_data(
    graph_db: &Arc<GraphDb>,
    filter: Option<&str>,
) -> Result<RawPortfolioData> {
    let mut repos = graph_db.list_repositories_extended().await?;
    if let Some(filter_str) = filter {
        let filter_lower = filter_str.to_lowercase();
        repos.retain(|repo| {
            repo.get("name")
                .and_then(|v| v.as_str())
                .is_some_and(|name| name.to_lowercase().contains(&filter_lower))
        });
    }

    let dep_edges = graph_db.list_repo_dependency_edges().await?;
    let call_coupling = graph_db.list_cross_repo_call_coupling().await?;

    Ok(RawPortfolioData {
        repos,
        dep_edges,
        call_coupling,
    })
}
