use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RepoRole {
    Hub,
    Leaf,
    Isolated,
    Balanced,
}

impl RepoRole {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Hub => "hub",
            Self::Leaf => "leaf",
            Self::Isolated => "isolated",
            Self::Balanced => "balanced",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoAsset {
    pub name: String,
    pub entity_count: i64,
    pub file_count: i64,
    pub build_system: String,
    pub primary_language: String,
    pub group_id: String,
    pub artifact_id: String,
    pub version: String,
    pub indexed_at: String,
    pub role: RepoRole,
    pub weight_pct: f64,
    pub dependency_count: usize,
    pub dependent_count: usize,
    /// Short description from package metadata, Cargo.toml, or README.
    pub description: String,
    /// Excerpt from indexed README.md content.
    pub readme_excerpt: String,
    /// Build identity, e.g. `com.example:my-app@1.2.0`.
    pub identity: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CorrelationKind {
    DependsOn,
    Calls,
}

impl CorrelationKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::DependsOn => "depends_on",
            Self::Calls => "calls",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Correlation {
    pub from: String,
    pub to: String,
    pub kind: CorrelationKind,
    pub strength: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Signal {
    pub kind: String,
    pub repo: String,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PortfolioSummary {
    pub repo_count: usize,
    pub total_entities: i64,
    pub total_files: i64,
    pub language_allocation: Vec<(String, f64)>,
    pub build_system_allocation: Vec<(String, f64)>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoPotential {
    pub repo: String,
    pub potential: String,
}

/// Structured Gemini advisor output sections.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AdvisorSections {
    pub inventory: String,
    pub resource_planning: String,
    pub forecast: String,
    pub actions: String,
    pub benchmarks: String,
    pub overall: String,
    pub repo_potentials: Vec<RepoPotential>,
}

impl AdvisorSections {
    pub fn is_empty(&self) -> bool {
        self.inventory.is_empty()
            && self.resource_planning.is_empty()
            && self.forecast.is_empty()
            && self.actions.is_empty()
            && self.benchmarks.is_empty()
            && self.overall.is_empty()
            && self.repo_potentials.is_empty()
    }
}

/// Why the GenAI advisor block is present or absent.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum AdvisorStatus {
    Generated,
    SkippedNoKey,
    SkippedFlag,
    Failed { message: String },
}

impl AdvisorStatus {
    pub fn notice(&self) -> String {
        match self {
            Self::Generated => String::new(),
            Self::SkippedNoKey => {
                "Advisor not generated: KNOT_GEMINI_API_KEY is not set. Add it to ~/.config/knot/.env and re-run.".to_string()
            }
            Self::SkippedFlag => "Advisor skipped via --no-ai.".to_string(),
            Self::Failed { message } => format!("Advisor not generated: Gemini API failed: {message}"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PortfolioReport {
    pub summary: PortfolioSummary,
    pub assets: Vec<RepoAsset>,
    pub correlations: Vec<Correlation>,
    pub signals: Vec<Signal>,
    pub advisor_context: String,
    pub advisor: AdvisorSections,
    pub advisor_status: AdvisorStatus,
    /// Full Gemini response (legacy / debug).
    pub recommendations: String,
    /// Repos excluded from this report (e.g. prowler).
    pub excluded_repos: Vec<String>,
}
