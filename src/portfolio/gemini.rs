use anyhow::{Context, Result};
use serde_json::json;

use super::models::{AdvisorSections, RepoPotential};

const SYSTEM_PROMPT: &str = "You are Portfolio AI, a venture and engineering advisor for a multi-repo software portfolio. \
Treat each indexed repository as a product asset. Analyze what the organization already holds, resource planning, \
prioritization, market forecast, concrete actions, and real-world benchmarks. \
Ground every statement in the provided data. Use clear markdown with the EXACT section headers requested in the user task. \
Name specific companies and products in benchmark sections. Use P0/P1/P2 labels for prioritization.";

pub const SECTION_HEADERS: &[&str] = &[
    "## Organizational Asset Inventory",
    "## Resource Planning and Prioritization",
    "## Strategic Forecast",
    "## Recommended Actions",
    "## Real-World Benchmarks",
    "## Overall Portfolio Recommendation",
    "## Business Potential by Repository",
];

const GEMINI_BASE: &str = "https://generativelanguage.googleapis.com/v1beta/models";

/// Gemini API configuration.
#[derive(Debug, Clone, Default)]
pub struct GeminiConfig {
    pub api_key: Option<String>,
    pub model: String,
}

impl GeminiConfig {
    pub fn from_env() -> Self {
        Self {
            api_key: std::env::var("KNOT_GEMINI_API_KEY")
                .ok()
                .filter(|s| !s.is_empty()),
            model: std::env::var("KNOT_GEMINI_MODEL")
                .unwrap_or_else(|_| "gemini-3.6-flash".to_string()),
        }
    }
}

pub async fn generate_recommendations(
    advisor_context: &str,
    config: &GeminiConfig,
) -> Result<String> {
    let api_key = config
        .api_key
        .as_deref()
        .filter(|k| !k.is_empty())
        .context("KNOT_GEMINI_API_KEY is not set")?;

    let url = format!(
        "{}/{}:generateContent?key={}",
        GEMINI_BASE, config.model, api_key
    );

    let body = json!({
        "systemInstruction": {
            "parts": [{ "text": SYSTEM_PROMPT }]
        },
        "contents": [{
            "role": "user",
            "parts": [{ "text": advisor_context }]
        }],
        "generationConfig": {
            "temperature": 0.4,
            "maxOutputTokens": 16384
        }
    });

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(90))
        .build()
        .context("Failed to build HTTP client")?;

    let response = client
        .post(&url)
        .json(&body)
        .send()
        .await
        .context("Gemini API request failed")?;

    let status = response.status();
    let text = response
        .text()
        .await
        .context("Failed to read Gemini response body")?;

    if !status.is_success() {
        anyhow::bail!("Gemini API error ({status}): {text}");
    }

    parse_gemini_response(&text)
}

/// Parsed Gemini sections for structured report output.
#[derive(Debug, Clone, Default)]
pub struct ParsedRecommendations {
    pub sections: AdvisorSections,
    pub raw: String,
}

pub fn parse_recommendation_sections(text: &str) -> ParsedRecommendations {
    let raw = text.trim().to_string();
    let parsed_map = parse_sections(&raw, SECTION_HEADERS);

    let repo_potentials = parse_repo_potentials(
        parsed_map
            .get("## Business Potential by Repository")
            .map(String::as_str)
            .unwrap_or(""),
    );

    let mut overall = parsed_map
        .get("## Overall Portfolio Recommendation")
        .cloned()
        .unwrap_or_default();

    if overall.is_empty() && repo_potentials.is_empty() && parsed_map.is_empty() {
        overall = raw.clone();
    }

    let sections = AdvisorSections {
        inventory: parsed_map
            .get("## Organizational Asset Inventory")
            .cloned()
            .unwrap_or_default(),
        resource_planning: parsed_map
            .get("## Resource Planning and Prioritization")
            .cloned()
            .unwrap_or_default(),
        forecast: parsed_map
            .get("## Strategic Forecast")
            .cloned()
            .unwrap_or_default(),
        actions: parsed_map
            .get("## Recommended Actions")
            .cloned()
            .unwrap_or_default(),
        benchmarks: parsed_map
            .get("## Real-World Benchmarks")
            .cloned()
            .unwrap_or_default(),
        overall,
        repo_potentials,
    };

    ParsedRecommendations { sections, raw }
}

pub fn parse_sections(text: &str, headers: &[&str]) -> std::collections::HashMap<String, String> {
    let mut result = std::collections::HashMap::new();
    for (i, header) in headers.iter().enumerate() {
        let body = extract_from_header(text, header);
        if body.is_empty() {
            continue;
        }
        let content = if i + 1 < headers.len() {
            truncate_at_next_header(&body, &headers[i + 1..])
        } else {
            body
        };
        if !content.is_empty() {
            result.insert((*header).to_string(), content);
        }
    }
    result
}

fn truncate_at_next_header(body: &str, next_headers: &[&str]) -> String {
    let mut end = body.len();
    for next in next_headers {
        if let Some(pos) = body.find(next) {
            end = end.min(pos);
        }
    }
    body[..end].trim().to_string()
}

fn extract_from_header(text: &str, header: &str) -> String {
    let Some(start) = text.find(header) else {
        return String::new();
    };
    text[start + header.len()..].trim_start().to_string()
}

fn parse_repo_potentials(body: &str) -> Vec<RepoPotential> {
    let mut results = Vec::new();
    let mut current_repo: Option<String> = None;
    let mut current_lines: Vec<String> = Vec::new();

    for line in body.lines() {
        if let Some(name) = line.strip_prefix("### ").map(str::trim) {
            if let Some(repo) = current_repo.take() {
                results.push(RepoPotential {
                    repo,
                    potential: current_lines.join("\n").trim().to_string(),
                });
                current_lines.clear();
            }
            current_repo = Some(name.to_string());
        } else if current_repo.is_some() {
            current_lines.push(line.to_string());
        }
    }
    if let Some(repo) = current_repo {
        results.push(RepoPotential {
            repo,
            potential: current_lines.join("\n").trim().to_string(),
        });
    }
    results
}

pub fn parse_gemini_response(body: &str) -> Result<String> {
    let parsed: serde_json::Value =
        serde_json::from_str(body).context("Failed to parse Gemini JSON response")?;

    if let Some(text) = parsed
        .pointer("/candidates/0/content/parts/0/text")
        .and_then(|v| v.as_str())
    {
        return Ok(text.trim().to_string());
    }

    if let Some(err) = parsed.pointer("/error/message").and_then(|v| v.as_str()) {
        anyhow::bail!("Gemini error: {err}");
    }

    anyhow::bail!("Unexpected Gemini response shape")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_gemini_response_success() {
        let body = r#"{
            "candidates": [{
                "content": {
                    "parts": [{ "text": "1. Re-index core first.\n2. Reduce coupling." }]
                }
            }]
        }"#;
        let text = parse_gemini_response(body).unwrap();
        assert!(text.contains("Re-index core"));
    }

    #[test]
    fn test_parse_gemini_response_error_field() {
        let body = r#"{"error": {"message": "API key invalid"}}"#;
        let err = parse_gemini_response(body).unwrap_err();
        assert!(err.to_string().contains("API key invalid"));
    }

    #[test]
    fn test_parse_gemini_response_unexpected_shape() {
        let body = r#"{"candidates": []}"#;
        assert!(parse_gemini_response(body).is_err());
    }

    #[test]
    fn test_parse_all_advisor_sections() {
        let text = r#"## Organizational Asset Inventory

Healthcare and fintech domains present.

## Resource Planning and Prioritization

P0: Consolidate asset_management platform.

## Strategic Forecast

GenAI devtools growth in 18 months.

## Recommended Actions

1. Index DEPENDS_ON edges for Synthapse clients.

## Real-World Benchmarks

Similar to Retool for internal tooling.

## Overall Portfolio Recommendation

Focus on Synthapse and asset_management as core platform assets.

## Business Potential by Repository

### Synthapse

Cloud deployment platform — high potential as internal PaaS for the portfolio.

### FinTool

Niche fintech UI — moderate potential, needs product definition.
"#;
        let parsed = parse_recommendation_sections(text);
        assert!(parsed.sections.inventory.contains("Healthcare"));
        assert!(parsed.sections.resource_planning.contains("P0"));
        assert!(parsed.sections.forecast.contains("GenAI"));
        assert!(parsed.sections.actions.contains("Synthapse"));
        assert!(parsed.sections.benchmarks.contains("Retool"));
        assert!(parsed.sections.overall.contains("Synthapse"));
        assert_eq!(parsed.sections.repo_potentials.len(), 2);
        assert_eq!(parsed.sections.repo_potentials[0].repo, "Synthapse");
    }

    #[test]
    fn test_parse_sections_chain() {
        let text = "## Organizational Asset Inventory\n\nLine one.\n\n## Resource Planning and Prioritization\n\nLine two.";
        let map = parse_sections(text, &SECTION_HEADERS[..2]);
        assert_eq!(map.len(), 2);
        assert!(map["## Organizational Asset Inventory"].contains("Line one"));
        assert!(map["## Resource Planning and Prioritization"].contains("Line two"));
    }
}
