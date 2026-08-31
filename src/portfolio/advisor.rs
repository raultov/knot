use super::insights::{derive_portfolio_insights, format_insights_markdown};
use super::models::{Correlation, CorrelationKind, PortfolioSummary, RepoAsset, Signal};

/// Optional planning context passed into the advisor prompt.
#[derive(Debug, Clone, Default)]
pub struct AdvisorPlanningContext {
    pub horizon: String,
    pub team_size: Option<u32>,
    pub focus: Option<String>,
}

pub fn build_advisor_context(
    summary: &PortfolioSummary,
    assets: &[RepoAsset],
    correlations: &[Correlation],
    signals: &[Signal],
    planning: &AdvisorPlanningContext,
) -> String {
    let insights = derive_portfolio_insights(assets);
    let mut out = String::from("# Codebase Portfolio — Advisor Brief\n\n");
    out.push_str(&format_insights_markdown(&insights, summary.repo_count));
    append_allocation(&mut out, summary, assets);
    append_repo_docs(&mut out, assets);
    append_language_allocation(&mut out, summary);
    append_correlations(&mut out, correlations);
    append_signals(&mut out, signals);
    append_planning_context(&mut out, planning);
    append_task(&mut out);
    out
}

fn append_allocation(out: &mut String, summary: &PortfolioSummary, assets: &[RepoAsset]) {
    out.push_str("## Current allocation (by entity count)\n\n");
    out.push_str(&format!(
        "Total: {} repos, {} entities, {} files\n\n",
        summary.repo_count, summary.total_entities, summary.total_files
    ));
    for asset in assets {
        out.push_str(&format!(
            "- **{}** — role={}, weight={:.1}%, entities={}, language={}, build={}, indexed={}\n",
            asset.name,
            asset.role.as_str(),
            asset.weight_pct,
            asset.entity_count,
            empty_dash(&asset.primary_language),
            empty_dash(&asset.build_system),
            if asset.indexed_at.is_empty() {
                "unknown"
            } else {
                &asset.indexed_at
            },
        ));
        if !asset.identity.is_empty() {
            out.push_str(&format!("  - identity: `{}`\n", asset.identity));
        }
        if !asset.description.is_empty() {
            out.push_str(&format!("  - description: {}\n", asset.description));
        }
    }
    out.push('\n');
}

fn empty_dash(value: &str) -> &str {
    if value.is_empty() { "-" } else { value }
}

fn append_repo_docs(out: &mut String, assets: &[RepoAsset]) {
    out.push_str("## Repository documentation\n\n");
    for asset in assets {
        out.push_str(&format!("### {}\n\n", asset.name));
        if asset.description.is_empty() && asset.readme_excerpt.is_empty() {
            out.push_str("_No indexed README or package description found._\n\n");
            continue;
        }
        if !asset.description.is_empty() {
            out.push_str(&format!("**Summary:** {}\n\n", asset.description));
        }
        if !asset.readme_excerpt.is_empty() {
            out.push_str(&format!(
                "**README excerpt:**\n\n{}\n\n",
                asset.readme_excerpt
            ));
        }
    }
}

fn append_language_allocation(out: &mut String, summary: &PortfolioSummary) {
    out.push_str("## Language allocation\n\n");
    for (lang, pct) in &summary.language_allocation {
        out.push_str(&format!("- {lang}: {pct:.1}%\n"));
    }
    out.push('\n');
}

fn append_correlations(out: &mut String, correlations: &[Correlation]) {
    out.push_str("## Dependency graph (structural)\n\n");
    let dep_edges: Vec<_> = correlations
        .iter()
        .filter(|c| c.kind == CorrelationKind::DependsOn)
        .collect();
    if dep_edges.is_empty() {
        out.push_str("- (no DEPENDS_ON edges detected)\n");
    } else {
        for c in dep_edges {
            out.push_str(&format!("- {} --depends_on--> {}\n", c.from, c.to));
        }
    }

    out.push_str("\n## Runtime coupling (cross-repo calls)\n\n");
    let call_edges: Vec<_> = correlations
        .iter()
        .filter(|c| c.kind == CorrelationKind::Calls)
        .collect();
    if call_edges.is_empty() {
        out.push_str("- (no cross-repo CALLS detected)\n");
    } else {
        for c in call_edges {
            out.push_str(&format!(
                "- {} --calls--> {} ({} call edges)\n",
                c.from, c.to, c.strength
            ));
        }
    }
    out.push('\n');
}

fn append_signals(out: &mut String, signals: &[Signal]) {
    out.push_str("## Risk / opportunity signals\n\n");
    if signals.is_empty() {
        out.push_str("- (none)\n");
    } else {
        for s in signals {
            out.push_str(&format!("- **{}** [{}]: {}\n", s.kind, s.repo, s.detail));
        }
    }
    out.push('\n');
}

fn append_planning_context(out: &mut String, planning: &AdvisorPlanningContext) {
    out.push_str("## Planning context\n\n");
    out.push_str(&format!("- Forecast horizon: {}\n", planning.horizon));
    if let Some(n) = planning.team_size {
        out.push_str(&format!("- Engineering team size: {n}\n"));
    } else {
        out.push_str("- Engineering team size: (not specified — infer from portfolio scale)\n");
    }
    if let Some(ref focus) = planning.focus {
        out.push_str(&format!("- Strategic focus hint: {focus}\n"));
    }
    out.push('\n');
}

fn append_task(out: &mut String) {
    out.push_str(
        "## Task\n\n\
         Produce a strategic portfolio analysis in **exactly** this markdown structure:\n\n\
         ```\n\
         ## Organizational Asset Inventory\n\
         <What capabilities, domains, IP, and platforms the org already owns; group by domain cluster; note reusable components and duplicates>\n\n\
         ## Resource Planning and Prioritization\n\
         <P0/P1/P2 ranked initiatives; where to focus engineering effort; consolidate vs spin-out vs sunset; sequencing for next 2-3 quarters>\n\n\
         ## Strategic Forecast\n\
         <Market outlook for the forecast horizon; trends aligned to portfolio domains; which bets are timely vs saturated>\n\n\
         ## Recommended Actions\n\
         <5-10 concrete next steps tied to specific repo names; include indexing/engineering and product actions>\n\n\
         ## Real-World Benchmarks\n\
         <3-5 brief case studies naming specific companies/products analogous to portfolio assets; what they did right; lesson for this org>\n\n\
         ## Overall Portfolio Recommendation\n\
         <3-6 bullets: workspace-level strategy, which products to prioritize, synergies, risks, indexing/engineering order>\n\n\
         ## Business Potential by Repository\n\
         ### <RepoName>\n\
         <2-4 sentences: what this product could become, market/domain fit, monetization or platform role, maturity gap>\n\
         (... repeat ### for EVERY repository listed above, skip none ...)\n\
         ```\n\n\
         Be specific to README excerpts, derived insights, and tech stack. \
         Call out underutilized assets and consolidation opportunities explicitly. \
         Rank high-potential assets in the overall section.\n",
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::portfolio::models::{Correlation, CorrelationKind, RepoRole};

    #[test]
    fn test_advisor_context_contains_repo_names() {
        let summary = PortfolioSummary {
            repo_count: 1,
            total_entities: 100,
            total_files: 10,
            language_allocation: vec![("java".to_string(), 100.0)],
            build_system_allocation: vec![("maven".to_string(), 100.0)],
        };
        let assets = vec![RepoAsset {
            name: "my-api".to_string(),
            entity_count: 100,
            file_count: 10,
            build_system: "maven".to_string(),
            primary_language: "java".to_string(),
            group_id: String::new(),
            artifact_id: String::new(),
            version: String::new(),
            indexed_at: "2026-01-01".to_string(),
            role: RepoRole::Hub,
            weight_pct: 100.0,
            dependency_count: 0,
            dependent_count: 3,
            description: String::new(),
            readme_excerpt: String::new(),
            identity: String::new(),
        }];
        let ctx =
            build_advisor_context(&summary, &assets, &[], &[], &AdvisorPlanningContext::default());
        assert!(ctx.contains("my-api"));
        assert!(ctx.contains("Derived portfolio insights"));
        assert!(ctx.contains("Organizational Asset Inventory"));
    }

    #[test]
    fn test_advisor_context_includes_correlation() {
        let summary = PortfolioSummary {
            repo_count: 2,
            total_entities: 200,
            total_files: 20,
            language_allocation: vec![],
            build_system_allocation: vec![],
        };
        let correlations = vec![Correlation {
            from: "api".to_string(),
            to: "core".to_string(),
            kind: CorrelationKind::Calls,
            strength: 50,
        }];
        let ctx = build_advisor_context(
            &summary,
            &[],
            &correlations,
            &[],
            &AdvisorPlanningContext::default(),
        );
        assert!(ctx.contains("api --calls--> core"));
    }
}
