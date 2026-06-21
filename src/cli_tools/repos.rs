//! Core repos logic shared between CLI and MCP.
//!
//! Lists all indexed repositories along with their entity count, file count,
//! build system, and primary language. Intended as a quick orientation tool:
//! "what codebases have I indexed so far?"

use std::sync::Arc;

use crate::config::OutputFormat;
use crate::db::graph::{GraphDb, RepoQueryExt};

/// Fetch the list of indexed repositories from the graph database.
///
/// The returned `serde_json::Value` is always a JSON array. Each element has
/// the shape `{ name, entity_count, file_count, build_system, primary_language }`.
/// This shape matches the rest of the `cli_tools` layer and is rendered
/// differently by [`format_repos_output`] depending on the chosen
/// [`OutputFormat`].
///
/// If `filter` is `Some`, only repositories whose name contains the filter
/// string (case-insensitive) are included in the result.
pub async fn run_list_repos(
    filter: Option<&str>,
    graph_db: &Arc<GraphDb>,
) -> anyhow::Result<serde_json::Value> {
    let repos = graph_db.list_repositories().await?;
    let mut result = serde_json::json!(repos);

    if let Some(filter_str) = filter {
        let filter_lower = filter_str.to_lowercase();
        if let Some(arr) = result.as_array_mut() {
            arr.retain(|repo| {
                repo.get("name")
                    .and_then(|v| v.as_str())
                    .is_some_and(|name| name.to_lowercase().contains(&filter_lower))
            });
        }
    }

    Ok(result)
}

/// Format the repository list for the requested output format.
///
/// - `Table`:    fixed-width ASCII table with `REPO | BUILD SYSTEM | LANGUAGE | FILES | ENTITIES`.
/// - `Json`:     pretty-printed JSON, matching the rest of the CLI.
/// - `Markdown`: GFM table that renders well in chat UIs.
pub fn format_repos_output(result: &serde_json::Value, output: OutputFormat) -> String {
    let repos: Vec<serde_json::Value> = match result.as_array() {
        Some(arr) => arr.clone(),
        None => Vec::new(),
    };

    if repos.is_empty() {
        return "No repositories found.\n".to_string();
    }

    match output {
        OutputFormat::Table => format_repos_table(&repos),
        OutputFormat::Json => serde_json::to_string_pretty(result).unwrap_or_default(),
        OutputFormat::Markdown => format_repos_markdown(&repos),
    }
}

fn format_repos_table(repos: &[serde_json::Value]) -> String {
    use comfy_table::{Cell, CellAlignment, Color, ContentArrangement, Table};

    let mut table = Table::new();
    table.set_content_arrangement(ContentArrangement::DynamicFullWidth);
    table.set_header(vec![
        Cell::new("REPO")
            .set_alignment(CellAlignment::Left)
            .fg(Color::Green),
        Cell::new("BUILD SYSTEM")
            .set_alignment(CellAlignment::Left)
            .fg(Color::Cyan),
        Cell::new("LANGUAGE")
            .set_alignment(CellAlignment::Left)
            .fg(Color::Yellow),
        Cell::new("FILES")
            .set_alignment(CellAlignment::Right)
            .fg(Color::Magenta),
        Cell::new("ENTITIES")
            .set_alignment(CellAlignment::Right)
            .fg(Color::Magenta),
    ]);

    for repo in repos {
        let name = repo
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("-")
            .to_string();
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
        let file_count = repo.get("file_count").and_then(|v| v.as_i64()).unwrap_or(0);
        let entity_count = repo
            .get("entity_count")
            .and_then(|v| v.as_i64())
            .unwrap_or(0);

        table.add_row(vec![
            Cell::new(name).fg(Color::Green),
            Cell::new(if build_system.is_empty() {
                "-".to_string()
            } else {
                build_system
            }),
            Cell::new(if primary_language.is_empty() {
                "-".to_string()
            } else {
                primary_language
            }),
            Cell::new(format_thousands(file_count)).set_alignment(CellAlignment::Right),
            Cell::new(format_thousands(entity_count)).set_alignment(CellAlignment::Right),
        ]);
    }

    format!("Indexed repositories ({}):\n{}\n", repos.len(), table)
}

fn format_repos_markdown(repos: &[serde_json::Value]) -> String {
    let mut out = String::new();
    out.push_str(&format!("# Indexed repositories ({})\n\n", repos.len()));
    out.push_str("| REPO | BUILD SYSTEM | LANGUAGE | FILES | ENTITIES |\n");
    out.push_str("|------|--------------|----------|------:|---------:|\n");

    for repo in repos {
        let name = repo.get("name").and_then(|v| v.as_str()).unwrap_or("-");
        let build_system = repo
            .get("build_system")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let primary_language = repo
            .get("primary_language")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let file_count = repo.get("file_count").and_then(|v| v.as_i64()).unwrap_or(0);
        let entity_count = repo
            .get("entity_count")
            .and_then(|v| v.as_i64())
            .unwrap_or(0);

        out.push_str(&format!(
            "| {} | {} | {} | {} | {} |\n",
            name,
            if build_system.is_empty() {
                "-".to_string()
            } else {
                build_system.to_string()
            },
            if primary_language.is_empty() {
                "-".to_string()
            } else {
                primary_language.to_string()
            },
            format_thousands(file_count),
            format_thousands(entity_count),
        ));
    }

    out
}

/// Format an integer with a thin space as a thousands separator so that large
/// numbers like `4_521` render as `4 521` in the CLI table (matching the
/// expected output in the plan).
fn format_thousands(n: i64) -> String {
    if n < 1000 {
        return n.to_string();
    }
    let s = n.to_string();
    let bytes = s.as_bytes();
    let mut out = String::with_capacity(s.len() + s.len() / 3);
    for (i, c) in bytes.iter().enumerate() {
        if i > 0 && (bytes.len() - i).is_multiple_of(3) {
            out.push(' ');
        }
        out.push(*c as char);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sample_repos() -> serde_json::Value {
        json!([
            {
                "name": "my-api",
                "build_system": "Maven",
                "primary_language": "java",
                "file_count": 132,
                "entity_count": 4521
            },
            {
                "name": "auth-lib",
                "build_system": "Gradle",
                "primary_language": "kotlin",
                "file_count": 48,
                "entity_count": 1203
            },
            {
                "name": "frontend",
                "build_system": "npm",
                "primary_language": "typescript",
                "file_count": 91,
                "entity_count": 2870
            }
        ])
    }

    #[test]
    fn test_format_repos_output_empty() {
        let result = json!([]);
        let formatted = format_repos_output(&result, OutputFormat::Table);
        assert!(formatted.contains("No repositories found"));
    }

    #[test]
    fn test_format_repos_output_empty_json() {
        let result = json!([]);
        let formatted = format_repos_output(&result, OutputFormat::Json);
        assert!(formatted.contains("No repositories found"));
    }

    #[test]
    fn test_format_repos_output_empty_markdown() {
        let result = json!([]);
        let formatted = format_repos_output(&result, OutputFormat::Markdown);
        assert!(formatted.contains("No repositories found"));
    }

    #[test]
    fn test_format_repos_output_with_repos_table() {
        let formatted = format_repos_output(&sample_repos(), OutputFormat::Table);
        assert!(formatted.contains("Indexed repositories"));
        assert!(formatted.contains("my-api"));
        assert!(formatted.contains("auth-lib"));
        assert!(formatted.contains("frontend"));
        assert!(formatted.contains("Maven"));
        assert!(formatted.contains("Gradle"));
        assert!(formatted.contains("npm"));
        assert!(formatted.contains("java"));
        assert!(formatted.contains("kotlin"));
        assert!(formatted.contains("typescript"));
        assert!(formatted.contains("4 521"));
        assert!(formatted.contains("1 203"));
        assert!(formatted.contains("2 870"));
    }

    #[test]
    fn test_format_repos_output_json() {
        let formatted = format_repos_output(&sample_repos(), OutputFormat::Json);
        assert!(formatted.contains("\"name\""));
        assert!(formatted.contains("\"my-api\""));
        assert!(formatted.contains("\"build_system\""));
        assert!(formatted.contains("\"primary_language\""));
        assert!(formatted.contains("\"entity_count\""));
        assert!(formatted.contains("\"file_count\""));
    }

    #[test]
    fn test_format_repos_output_json_is_valid() {
        let formatted = format_repos_output(&sample_repos(), OutputFormat::Json);
        let parsed: serde_json::Value = serde_json::from_str(&formatted)
            .expect("format_repos_output(json) must produce valid JSON");
        let arr = parsed.as_array().expect("result should be an array");
        assert_eq!(arr.len(), 3);
    }

    #[test]
    fn test_format_repos_output_markdown() {
        let formatted = format_repos_output(&sample_repos(), OutputFormat::Markdown);
        assert!(formatted.contains("# Indexed repositories"));
        assert!(formatted.contains("| REPO | BUILD SYSTEM | LANGUAGE | FILES | ENTITIES |"));
        assert!(formatted.contains("|------|--------------|----------|------:|---------:|"));
        assert!(formatted.contains("my-api"));
        assert!(formatted.contains("auth-lib"));
        assert!(formatted.contains("frontend"));
        assert!(formatted.contains("4 521"));
    }

    #[test]
    fn test_format_repos_output_missing_optional_fields_table() {
        let result = json!([
            {
                "name": "bare",
                "build_system": "",
                "primary_language": "",
                "file_count": 0,
                "entity_count": 0
            }
        ]);
        let formatted = format_repos_output(&result, OutputFormat::Table);
        assert!(formatted.contains("bare"));
        // Empty build_system and primary_language are rendered as `-`.
        assert!(formatted.contains('-'));
    }

    #[test]
    fn test_format_repos_output_null_input() {
        let formatted = format_repos_output(&serde_json::Value::Null, OutputFormat::Table);
        assert!(formatted.contains("No repositories found"));
    }

    #[test]
    fn test_format_repos_output_object_input() {
        let result = json!({"unexpected": "shape"});
        let formatted = format_repos_output(&result, OutputFormat::Table);
        assert!(formatted.contains("No repositories found"));
    }

    #[test]
    fn test_format_thousands_below_threshold() {
        assert_eq!(format_thousands(0), "0");
        assert_eq!(format_thousands(5), "5");
        assert_eq!(format_thousands(999), "999");
    }

    #[test]
    fn test_format_thousands_at_threshold() {
        assert_eq!(format_thousands(1000), "1 000");
    }

    #[test]
    fn test_format_thousands_large() {
        assert_eq!(format_thousands(123_456), "123 456");
        assert_eq!(format_thousands(1_000_000), "1 000 000");
    }

    #[test]
    fn test_format_thousands_negative() {
        // Negative numbers are emitted verbatim: we only ever pass
        // non-negative counts in practice.
        assert_eq!(format_thousands(-42), "-42");
    }

    #[test]
    fn test_filter_repos_case_insensitive() {
        let repos = sample_repos();
        let filter_lower = "API".to_lowercase();
        let mut filtered = repos.clone();
        if let Some(arr) = filtered.as_array_mut() {
            arr.retain(|repo| {
                repo.get("name")
                    .and_then(|v| v.as_str())
                    .is_some_and(|name| name.to_lowercase().contains(&filter_lower))
            });
        }
        let arr = filtered.as_array().unwrap();
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0]["name"], "my-api");
    }

    #[test]
    fn test_filter_repos_no_match() {
        let repos = sample_repos();
        let filter_lower = "nonexistent".to_lowercase();
        let mut filtered = repos.clone();
        if let Some(arr) = filtered.as_array_mut() {
            arr.retain(|repo| {
                repo.get("name")
                    .and_then(|v| v.as_str())
                    .is_some_and(|name| name.to_lowercase().contains(&filter_lower))
            });
        }
        let arr = filtered.as_array().unwrap();
        assert!(arr.is_empty());
    }

    #[test]
    fn test_filter_repos_partial_match() {
        let repos = sample_repos();
        let filter_lower = "auth".to_lowercase();
        let mut filtered = repos.clone();
        if let Some(arr) = filtered.as_array_mut() {
            arr.retain(|repo| {
                repo.get("name")
                    .and_then(|v| v.as_str())
                    .is_some_and(|name| name.to_lowercase().contains(&filter_lower))
            });
        }
        let arr = filtered.as_array().unwrap();
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0]["name"], "auth-lib");
    }

    #[test]
    fn test_filter_repos_none_returns_all() {
        let repos = sample_repos();
        let filter: Option<&str> = None;
        let mut filtered = repos.clone();
        if let Some(filter_str) = filter {
            let filter_lower = filter_str.to_lowercase();
            if let Some(arr) = filtered.as_array_mut() {
                arr.retain(|repo| {
                    repo.get("name")
                        .and_then(|v| v.as_str())
                        .is_some_and(|name| name.to_lowercase().contains(&filter_lower))
                });
            }
        }
        let arr = filtered.as_array().unwrap();
        assert_eq!(arr.len(), 3);
    }
}
