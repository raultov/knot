use crate::config::OutputFormat;
use crate::portfolio::models::{AdvisorStatus, PortfolioReport};

pub fn format_portfolio_output(report: &PortfolioReport, output: OutputFormat) -> String {
    match output {
        OutputFormat::Json => serde_json::to_string_pretty(report).unwrap_or_default(),
        OutputFormat::Markdown => format_markdown(report),
        OutputFormat::Table => format_table(report),
    }
}

fn format_table(report: &PortfolioReport) -> String {
    use comfy_table::{Cell, CellAlignment, Color, ContentArrangement, Table};

    let mut out = format!(
        "Codebase Portfolio ({} repos, {} entities)\n\n",
        report.summary.repo_count, report.summary.total_entities
    );

    let mut table = Table::new();
    table.set_content_arrangement(ContentArrangement::DynamicFullWidth);
    table.set_header(vec![
        Cell::new("REPO").fg(Color::Green),
        Cell::new("ROLE").fg(Color::Cyan),
        Cell::new("WEIGHT").set_alignment(CellAlignment::Right),
        Cell::new("ENTITIES").set_alignment(CellAlignment::Right),
        Cell::new("LANGUAGE").fg(Color::Yellow),
        Cell::new("INDEXED").fg(Color::Magenta),
    ]);

    for asset in &report.assets {
        table.add_row(vec![
            Cell::new(&asset.name).fg(Color::Green),
            Cell::new(asset.role.as_str()),
            Cell::new(format!("{:.1}%", asset.weight_pct)).set_alignment(CellAlignment::Right),
            Cell::new(asset.entity_count.to_string()).set_alignment(CellAlignment::Right),
            Cell::new(if asset.primary_language.is_empty() {
                "-".to_string()
            } else {
                asset.primary_language.clone()
            }),
            Cell::new(if asset.indexed_at.is_empty() {
                "-".to_string()
            } else {
                asset.indexed_at.chars().take(10).collect()
            }),
        ]);
    }

    out.push_str(&table.to_string());
    out.push_str("\n\nSignals:\n");
    if report.signals.is_empty() {
        out.push_str("  (none)\n");
    } else {
        for s in &report.signals {
            out.push_str(&format!("  - {} [{}]: {}\n", s.kind, s.repo, s.detail));
        }
    }

    append_advisor_table(&mut out, report);
    out
}

fn append_advisor_table(out: &mut String, report: &PortfolioReport) {
    match &report.advisor_status {
        AdvisorStatus::Generated => {
            if !report.advisor.overall.is_empty() {
                out.push_str("\nOverall recommendation:\n");
                out.push_str(&report.advisor.overall);
                out.push('\n');
            }
            if !report.advisor.repo_potentials.is_empty() {
                out.push_str("\nBusiness potential:\n");
                for rp in &report.advisor.repo_potentials {
                    out.push_str(&format!("  {}: {}\n", rp.repo, rp.potential));
                }
            }
        }
        status => {
            let notice = status.notice();
            if !notice.is_empty() {
                out.push_str(&format!("\nAdvisor: {notice}\n"));
            }
        }
    }
}

fn format_markdown(report: &PortfolioReport) -> String {
    let mut out = format!(
        "# Codebase Portfolio ({} repos)\n\n",
        report.summary.repo_count
    );

    if !report.excluded_repos.is_empty() {
        out.push_str(&format!(
            "_Excluded from analysis: {}_\n\n",
            report.excluded_repos.join(", ")
        ));
    }

    out.push_str("## Current state\n\n");
    out.push_str("| REPO | ROLE | WEIGHT | ENTITIES | LANGUAGE | INDEXED |\n");
    out.push_str("|------|------|-------:|---------:|----------|--------|\n");
    for asset in &report.assets {
        out.push_str(&format!(
            "| {} | {} | {:.1}% | {} | {} | {} |\n",
            asset.name,
            asset.role.as_str(),
            asset.weight_pct,
            asset.entity_count,
            if asset.primary_language.is_empty() {
                "-".to_string()
            } else {
                asset.primary_language.clone()
            },
            if asset.indexed_at.is_empty() {
                "-".to_string()
            } else {
                asset.indexed_at.chars().take(10).collect::<String>()
            },
        ));
    }

    out.push_str("\n## Repository documentation\n\n");
    if report
        .assets
        .iter()
        .all(|a| a.description.is_empty() && a.readme_excerpt.is_empty())
    {
        out.push_str("_No indexed README or package descriptions found. Index repos with README.md or package.json for richer docs._\n\n");
    }
    for asset in &report.assets {
        out.push_str(&format!("### {}\n\n", asset.name));
        if !asset.identity.is_empty() {
            out.push_str(&format!("- **Identity:** `{}`\n", asset.identity));
        }
        if !asset.description.is_empty() {
            out.push_str(&format!("- **Description:** {}\n", asset.description));
        }
        if !asset.readme_excerpt.is_empty() {
            out.push_str(&format!("\n{}\n\n", asset.readme_excerpt));
        }
        if asset.description.is_empty() && asset.readme_excerpt.is_empty() {
            out.push_str("_No documentation indexed for this repo._\n\n");
        }
    }

    out.push_str("\n## Correlations\n\n");
    if report.correlations.is_empty() {
        out.push_str("- (none)\n");
    } else {
        for c in &report.correlations {
            out.push_str(&format!(
                "- {} --{}--> {} (strength {})\n",
                c.from,
                c.kind.as_str(),
                c.to,
                c.strength
            ));
        }
    }

    out.push_str("\n## Signals\n\n");
    if report.signals.is_empty() {
        out.push_str("- (none)\n");
    } else {
        for s in &report.signals {
            out.push_str(&format!("- **{}** [{}]: {}\n", s.kind, s.repo, s.detail));
        }
    }

    append_advisor_markdown(&mut out, report);
    out
}

fn append_advisor_markdown(out: &mut String, report: &PortfolioReport) {
    out.push_str("\n## Portfolio Advisor (GenAI)\n\n");

    match &report.advisor_status {
        AdvisorStatus::Generated => {
            out.push_str("_Advisor generated by Gemini._\n\n");
            append_advisor_subsection(out, "Organizational Asset Inventory", &report.advisor.inventory);
            append_advisor_subsection(
                out,
                "Resource Planning and Prioritization",
                &report.advisor.resource_planning,
            );
            append_advisor_subsection(out, "Strategic Forecast", &report.advisor.forecast);
            append_advisor_subsection(out, "Recommended Actions", &report.advisor.actions);
            append_advisor_subsection(out, "Real-World Benchmarks", &report.advisor.benchmarks);
            append_advisor_subsection(
                out,
                "Overall Portfolio Recommendation",
                &report.advisor.overall,
            );

            if !report.advisor.repo_potentials.is_empty() {
                out.push_str("### Business Potential by Repository\n\n");
                for rp in &report.advisor.repo_potentials {
                    out.push_str(&format!("#### {}\n\n", rp.repo));
                    out.push_str(&rp.potential);
                    out.push_str("\n\n");
                }
            } else if !report.recommendations.is_empty() {
                out.push_str("_Structured per-repo potential not parsed; raw response below._\n\n");
                out.push_str(&report.recommendations);
                out.push('\n');
            }
        }
        status => {
            let notice = status.notice();
            if !notice.is_empty() {
                out.push_str(&format!("_{notice}_\n"));
            }
        }
    }
}

fn append_advisor_subsection(out: &mut String, title: &str, body: &str) {
    if body.is_empty() {
        return;
    }
    out.push_str(&format!("### {title}\n\n"));
    out.push_str(body);
    out.push_str("\n\n");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::portfolio::models::{
        AdvisorSections, AdvisorStatus, Correlation, CorrelationKind, PortfolioSummary,
        RepoAsset, RepoPotential, RepoRole,
    };

    fn sample_report() -> PortfolioReport {
        PortfolioReport {
            summary: PortfolioSummary {
                repo_count: 1,
                total_entities: 100,
                total_files: 10,
                language_allocation: vec![],
                build_system_allocation: vec![],
            },
            assets: vec![RepoAsset {
                name: "core".to_string(),
                entity_count: 100,
                file_count: 10,
                build_system: "maven".to_string(),
                primary_language: "java".to_string(),
                group_id: String::new(),
                artifact_id: String::new(),
                version: String::new(),
                indexed_at: "2026-08-01".to_string(),
                role: RepoRole::Hub,
                weight_pct: 100.0,
                dependency_count: 0,
                dependent_count: 3,
                description: String::new(),
                readme_excerpt: String::new(),
                identity: String::new(),
            }],
            correlations: vec![Correlation {
                from: "api".to_string(),
                to: "core".to_string(),
                kind: CorrelationKind::DependsOn,
                strength: 1,
            }],
            signals: vec![],
            advisor_context: String::new(),
            advisor: AdvisorSections {
                inventory: "Healthcare and fintech assets.".to_string(),
                resource_planning: "P0: core platform.".to_string(),
                forecast: "GenAI growth.".to_string(),
                actions: "Index deps.".to_string(),
                benchmarks: "Like Retool.".to_string(),
                overall: "Prioritize core.".to_string(),
                repo_potentials: vec![RepoPotential {
                    repo: "core".to_string(),
                    potential: "Platform hub.".to_string(),
                }],
            },
            advisor_status: AdvisorStatus::Generated,
            recommendations: String::new(),
            excluded_repos: vec!["prowler".to_string()],
        }
    }

    #[test]
    fn test_format_markdown_contains_advisor_sections() {
        let md = format_markdown(&sample_report());
        assert!(md.contains("# Codebase Portfolio"));
        assert!(md.contains("## Portfolio Advisor (GenAI)"));
        assert!(md.contains("### Organizational Asset Inventory"));
        assert!(md.contains("### Resource Planning and Prioritization"));
        assert!(md.contains("### Strategic Forecast"));
        assert!(md.contains("### Recommended Actions"));
        assert!(md.contains("### Real-World Benchmarks"));
        assert!(md.contains("### Overall Portfolio Recommendation"));
        assert!(md.contains("#### core"));
        assert!(md.contains("Excluded from analysis"));
    }

    #[test]
    fn test_format_markdown_skipped_notice() {
        let mut report = sample_report();
        report.advisor_status = AdvisorStatus::SkippedNoKey;
        report.advisor = AdvisorSections::default();
        let md = format_markdown(&report);
        assert!(md.contains("KNOT_GEMINI_API_KEY is not set"));
    }

    #[test]
    fn test_format_json_valid() {
        let json = format_portfolio_output(&sample_report(), OutputFormat::Json);
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["summary"]["repo_count"], 1);
        assert_eq!(parsed["advisor_status"]["status"], "generated");
    }
}
