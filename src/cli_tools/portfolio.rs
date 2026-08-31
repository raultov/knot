//! Portfolio CLI/MCP adapter — multi-repo asset management report.

use std::sync::Arc;

use crate::config::OutputFormat;
use crate::db::graph::GraphDb;
use crate::portfolio::{
    PortfolioOptions, build_portfolio_report, format_portfolio_output,
};

pub async fn run_portfolio(
    options: PortfolioOptions,
    graph_db: &Arc<GraphDb>,
) -> anyhow::Result<crate::portfolio::PortfolioReport> {
    build_portfolio_report(graph_db, options).await
}

pub fn format_portfolio_report_output(
    report: &crate::portfolio::PortfolioReport,
    output: OutputFormat,
) -> String {
    format_portfolio_output(report, output)
}
