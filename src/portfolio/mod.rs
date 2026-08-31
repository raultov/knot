//! Portfolio layer — multi-repo asset management over indexed codebases.

mod advisor;
mod collect;
mod correlate;
mod docs;
mod filter;
mod format;
mod gemini;
mod insights;
mod models;
mod signals;

pub use advisor::AdvisorPlanningContext;
pub use format::format_portfolio_output;
pub use gemini::{
    GeminiConfig, ParsedRecommendations, generate_recommendations, parse_gemini_response,
    parse_recommendation_sections,
};
pub use models::{
    AdvisorSections, AdvisorStatus, Correlation, CorrelationKind, PortfolioReport, PortfolioSummary,
    RepoAsset, RepoPotential, RepoRole, Signal,
};
pub use signals::detect_signals;

use std::sync::Arc;

use anyhow::Result;

use crate::db::graph::{GraphDb, PortfolioQueryExt as _};

use self::advisor::build_advisor_context;
use self::collect::collect_portfolio_data;
use self::correlate::{build_correlations, enrich_assets};
use self::docs::attach_documentation;
use self::filter::{apply_exclusions, effective_exclude_list};

/// Options for building a portfolio report.
#[derive(Debug, Clone)]
pub struct PortfolioOptions {
    pub filter: Option<String>,
    pub exclude: Vec<String>,
    pub skip_ai: bool,
    pub gemini: GeminiConfig,
    pub horizon: String,
    pub team_size: Option<u32>,
    pub focus: Option<String>,
}

impl Default for PortfolioOptions {
    fn default() -> Self {
        Self {
            filter: None,
            exclude: Vec::new(),
            skip_ai: false,
            gemini: GeminiConfig::default(),
            horizon: default_horizon(),
            team_size: default_team_size(),
            focus: default_focus(),
        }
    }
}

impl PortfolioOptions {
    pub fn planning_context(&self) -> AdvisorPlanningContext {
        AdvisorPlanningContext {
            horizon: self.horizon.clone(),
            team_size: self.team_size,
            focus: self.focus.clone(),
        }
    }
}

fn default_horizon() -> String {
    std::env::var("KNOT_PORTFOLIO_HORIZON").unwrap_or_else(|_| "18m".to_string())
}

fn default_team_size() -> Option<u32> {
    std::env::var("KNOT_PORTFOLIO_TEAM_SIZE")
        .ok()
        .and_then(|s| s.parse().ok())
}

fn default_focus() -> Option<String> {
    std::env::var("KNOT_PORTFOLIO_FOCUS")
        .ok()
        .filter(|s| !s.is_empty())
}

/// Build a full portfolio report from the graph database, optionally calling Gemini.
pub async fn build_portfolio_report(
    graph_db: &Arc<GraphDb>,
    options: PortfolioOptions,
) -> Result<PortfolioReport> {
    let excluded = effective_exclude_list(&options.exclude);
    let mut raw = collect_portfolio_data(graph_db, options.filter.as_deref()).await?;
    apply_exclusions(&mut raw, &excluded);

    let (mut assets, summary) = enrich_assets(&raw.repos, &raw.dep_edges);

    let repo_names: Vec<String> = assets.iter().map(|a| a.name.clone()).collect();
    let doc_rows = graph_db.list_repo_documentation(&repo_names).await?;
    attach_documentation(&mut assets, &doc_rows);

    let correlations = build_correlations(&raw.dep_edges, &raw.call_coupling);
    let signals = detect_signals(&assets, &correlations, &raw.dep_edges);
    let planning = options.planning_context();
    let advisor_context =
        build_advisor_context(&summary, &assets, &correlations, &signals, &planning);

    let mut advisor = AdvisorSections::default();
    let mut recommendations = String::new();
    let advisor_status = if options.skip_ai {
        AdvisorStatus::SkippedFlag
    } else if options.gemini.api_key.is_none() {
        AdvisorStatus::SkippedNoKey
    } else {
        match generate_recommendations(&advisor_context, &options.gemini).await {
            Ok(text) => {
                let parsed = parse_recommendation_sections(&text);
                advisor = parsed.sections;
                recommendations = parsed.raw;
                AdvisorStatus::Generated
            }
            Err(e) => {
                tracing::warn!("Gemini recommendation failed: {e}");
                AdvisorStatus::Failed {
                    message: e.to_string(),
                }
            }
        }
    };

    Ok(PortfolioReport {
        summary,
        assets,
        correlations,
        signals,
        advisor_context,
        advisor,
        advisor_status,
        recommendations,
        excluded_repos: excluded,
    })
}

pub fn portfolio_options_from_env(mut options: PortfolioOptions) -> PortfolioOptions {
    if options.horizon.is_empty() {
        options.horizon = default_horizon();
    }
    options.team_size = options.team_size.or_else(default_team_size);
    options.focus = options.focus.or_else(default_focus);
    if options.gemini.api_key.is_none() {
        options.gemini = GeminiConfig::from_env();
    }
    options
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_portfolio_options_defaults() {
        let opts = portfolio_options_from_env(PortfolioOptions {
            horizon: String::new(),
            ..PortfolioOptions::default()
        });
        assert_eq!(opts.horizon, "18m");
    }
}
