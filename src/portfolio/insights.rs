//! Deterministic portfolio insights derived from indexed repo metadata.

use std::collections::{BTreeMap, HashMap};

use super::models::{RepoAsset, RepoRole};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MaturityTier {
    Shell,
    Prototype,
    Substantial,
    PlatformScale,
}

impl MaturityTier {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Shell => "shell",
            Self::Prototype => "prototype",
            Self::Substantial => "substantial",
            Self::PlatformScale => "platform-scale",
        }
    }

    pub fn from_entity_count(count: i64) -> Self {
        match count {
            n if n < 500 => Self::Shell,
            n if n < 2_000 => Self::Prototype,
            n if n < 8_000 => Self::Substantial,
            _ => Self::PlatformScale,
        }
    }
}

struct DomainRule {
    name: &'static str,
    keywords: &'static [&'static str],
}

const DOMAIN_RULES: &[DomainRule] = &[
    DomainRule {
        name: "healthcare",
        keywords: &[
            "health", "medical", "patient", "care", "clinical", "hospital", "nutri", "homemed",
        ],
    },
    DomainRule {
        name: "fintech",
        keywords: &[
            "finance", "fintech", "payment", "bank", "trading", "invest", "procure", "invoice",
        ],
    },
    DomainRule {
        name: "ai_marketing",
        keywords: &[
            "ai", "marketing", "campaign", "influencer", "audience", "clone", "viral", "content",
            "prospect",
        ],
    },
    DomainRule {
        name: "devtools_infra",
        keywords: &[
            "terraform", "deploy", "cloud", "infrastructure", "devops", "platform", "index",
            "semantic", "graph", "architecture", "stateboard", "synthapse",
        ],
    },
    DomainRule {
        name: "gaming",
        keywords: &["game", "poker", "rift", "sport", "league", "esport"],
    },
    DomainRule {
        name: "enterprise_saas",
        keywords: &[
            "document", "approval", "access", "workflow", "enterprise", "cqrs", "asset",
            "management",
        ],
    },
    DomainRule {
        name: "multi_agent",
        keywords: &["agent", "multi-agent", "llm", "mas", "autonomous"],
    },
];

pub fn classify_domain(asset: &RepoAsset) -> &'static str {
    let haystack = format!(
        "{} {} {} {}",
        asset.name, asset.description, asset.readme_excerpt, asset.identity
    )
    .to_lowercase();

    for rule in DOMAIN_RULES {
        if rule.keywords.iter().any(|kw| haystack.contains(kw)) {
            return rule.name;
        }
    }
    "general"
}

pub struct PortfolioInsights {
    pub maturity_by_repo: Vec<(String, MaturityTier)>,
    pub domain_clusters: BTreeMap<String, Vec<String>>,
    pub stack_groups: BTreeMap<String, Vec<String>>,
    pub isolated_count: usize,
    pub dominant_repo: Option<String>,
    pub shell_repos: Vec<String>,
}

pub fn derive_portfolio_insights(assets: &[RepoAsset]) -> PortfolioInsights {
    let mut maturity_by_repo = Vec::new();
    let mut domain_clusters: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut stack_groups: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut isolated_count = 0usize;
    let mut dominant_repo = None;
    let mut max_weight = 0.0f64;
    let mut shell_repos = Vec::new();

    for asset in assets {
        let tier = MaturityTier::from_entity_count(asset.entity_count);
        maturity_by_repo.push((asset.name.clone(), tier.clone()));

        if tier == MaturityTier::Shell {
            shell_repos.push(asset.name.clone());
        }

        if asset.role == RepoRole::Isolated {
            isolated_count += 1;
        }

        if asset.weight_pct > max_weight {
            max_weight = asset.weight_pct;
            dominant_repo = Some(asset.name.clone());
        }

        let domain = classify_domain(asset);
        domain_clusters
            .entry(domain.to_string())
            .or_default()
            .push(asset.name.clone());

        let stack_key = format!(
            "{}/{}",
            if asset.primary_language.is_empty() {
                "unknown"
            } else {
                &asset.primary_language
            },
            if asset.build_system.is_empty() {
                "unknown"
            } else {
                &asset.build_system
            }
        );
        stack_groups
            .entry(stack_key)
            .or_default()
            .push(asset.name.clone());
    }

    PortfolioInsights {
        maturity_by_repo,
        domain_clusters,
        stack_groups,
        isolated_count,
        dominant_repo,
        shell_repos,
    }
}

pub fn format_insights_markdown(insights: &PortfolioInsights, repo_count: usize) -> String {
    let mut out = String::from("## Derived portfolio insights\n\n");

    out.push_str(&format!(
        "- **Portfolio shape:** {repo_count} repos, {isolated} isolated (no DEPENDS_ON edges)\n",
        repo_count = repo_count,
        isolated = insights.isolated_count
    ));

    if let Some(ref dominant) = insights.dominant_repo {
        out.push_str(&format!("- **Dominant asset:** {dominant}\n"));
    }

    if !insights.shell_repos.is_empty() {
        out.push_str(&format!(
            "- **Shell repos (<500 entities):** {}\n",
            insights.shell_repos.join(", ")
        ));
    }

    out.push_str("\n### Maturity tiers\n\n");
    let mut by_tier: HashMap<&str, Vec<&str>> = HashMap::new();
    for (name, tier) in &insights.maturity_by_repo {
        by_tier
            .entry(tier.as_str())
            .or_default()
            .push(name.as_str());
    }
    for tier in ["platform-scale", "substantial", "prototype", "shell"] {
        if let Some(repos) = by_tier.get(tier) {
            out.push_str(&format!("- **{tier}:** {}\n", repos.join(", ")));
        }
    }

    out.push_str("\n### Domain clusters (keyword heuristic)\n\n");
    for (domain, repos) in &insights.domain_clusters {
        out.push_str(&format!("- **{domain}:** {}\n", repos.join(", ")));
    }

    out.push_str("\n### Stack overlap\n\n");
    for (stack, repos) in &insights.stack_groups {
        if repos.len() > 1 {
            out.push_str(&format!(
                "- **{stack}** ({} repos): {}\n",
                repos.len(),
                repos.join(", ")
            ));
        }
    }

    out.push('\n');
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::portfolio::models::RepoRole;

    fn asset(name: &str, entities: i64, lang: &str, desc: &str) -> RepoAsset {
        RepoAsset {
            name: name.to_string(),
            entity_count: entities,
            file_count: 10,
            build_system: "npm".to_string(),
            primary_language: lang.to_string(),
            group_id: String::new(),
            artifact_id: String::new(),
            version: String::new(),
            indexed_at: String::new(),
            role: RepoRole::Isolated,
            weight_pct: 5.0,
            dependency_count: 0,
            dependent_count: 0,
            description: desc.to_string(),
            readme_excerpt: String::new(),
            identity: String::new(),
        }
    }

    #[test]
    fn test_maturity_tiers() {
        assert_eq!(
            MaturityTier::from_entity_count(100),
            MaturityTier::Shell
        );
        assert_eq!(
            MaturityTier::from_entity_count(1_000),
            MaturityTier::Prototype
        );
        assert_eq!(
            MaturityTier::from_entity_count(5_000),
            MaturityTier::Substantial
        );
        assert_eq!(
            MaturityTier::from_entity_count(10_000),
            MaturityTier::PlatformScale
        );
    }

    #[test]
    fn test_classify_domain_healthcare() {
        let a = asset("HomeMed", 500, "typescript", "patient care app");
        assert_eq!(classify_domain(&a), "healthcare");
    }

    #[test]
    fn test_derive_insights_finds_shells() {
        let assets = vec![
            asset("big", 10_000, "javascript", "platform"),
            asset("tiny", 50, "json", "empty"),
        ];
        let insights = derive_portfolio_insights(&assets);
        assert_eq!(insights.shell_repos, vec!["tiny".to_string()]);
        assert_eq!(insights.dominant_repo, Some("big".to_string()));
    }

    #[test]
    fn test_format_insights_contains_clusters() {
        let assets = vec![asset("FinTool", 100, "typescript", "fintech payments")];
        let insights = derive_portfolio_insights(&assets);
        let md = format_insights_markdown(&insights, 1);
        assert!(md.contains("fintech"));
        assert!(md.contains("FinTool"));
    }
}
