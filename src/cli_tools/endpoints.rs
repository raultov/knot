//! Core `list_endpoints` logic shared between CLI and MCP (issue #33).
//!
//! Lists the HTTP endpoints the indexed repositories serve — `<METHOD>
//! <path>` with the handler each one routes to — optionally narrowed by
//! HTTP method and by path.
//!
//! # Semantics
//! - `method` matches case-insensitively; endpoints mapped to every method
//!   (`ANY`, a Spring `@RequestMapping` without `method`) match any filter.
//! - `path` matches segment by segment as a prefix: `/api/owners` matches
//!   `/api/owners` and `/api/owners/{id}` but not `/api/ownership`, and an
//!   endpoint's `{variable}` segment matches any concrete segment, so
//!   `/api/owners/42` finds `GET /api/owners/{id}`.
//! - Ordering is `(repository, path, method)`; at most [`MAX_ENDPOINTS`]
//!   rows are returned and the reply says when more matched.

use std::sync::Arc;

use anyhow::Result;
use serde_json::json;

use crate::db::graph::{GraphDb, QueryExt};
use crate::models::RepoScope;

/// Rows returned at most.
const MAX_ENDPOINTS: usize = 1000;

/// Rows read from the graph at most before filtering.
const MAX_SCANNED: usize = 20_000;

/// Verb of an endpoint that accepts every HTTP method.
const ANY_METHOD: &str = "ANY";

/// List the endpoints of `repo` matching `method` and `path`.
pub async fn run_list_endpoints(
    method: Option<&str>,
    path: Option<&str>,
    repo: &RepoScope,
    graph_db: &Arc<GraphDb>,
) -> Result<serde_json::Value> {
    let repo_names = repo.filter_names();
    let rows = graph_db
        .list_endpoints(&repo_names, MAX_SCANNED + 1)
        .await?;
    let rows = rows.as_array().cloned().unwrap_or_default();
    Ok(filter_endpoints(
        &rows,
        method,
        path,
        rows.len() > MAX_SCANNED,
    ))
}

/// Apply the filters to raw graph rows and shape the reply.
fn filter_endpoints(
    rows: &[serde_json::Value],
    method: Option<&str>,
    path: Option<&str>,
    scan_truncated: bool,
) -> serde_json::Value {
    let mut endpoints: Vec<serde_json::Value> = rows
        .iter()
        .take(MAX_SCANNED)
        .filter_map(endpoint_row)
        .filter(|row| {
            method.is_none_or(|m| method_matches(m, row["method"].as_str().unwrap_or("")))
        })
        .filter(|row| path.is_none_or(|p| path_matches(p, row["path"].as_str().unwrap_or(""))))
        .collect();
    endpoints.sort_by(|a, b| {
        let key = |v: &serde_json::Value| {
            (
                v["repo_name"].as_str().unwrap_or("").to_string(),
                v["path"].as_str().unwrap_or("").to_string(),
                v["method"].as_str().unwrap_or("").to_string(),
            )
        };
        key(a).cmp(&key(b))
    });
    let total = endpoints.len();
    let truncated = total > MAX_ENDPOINTS || scan_truncated;
    endpoints.truncate(MAX_ENDPOINTS);
    json!({
        "endpoints": endpoints,
        "total_matches": total,
        "total_is_lower_bound": scan_truncated,
        "truncated": truncated,
    })
}

/// A graph row split into `method` and `path`; `None` for a malformed name.
fn endpoint_row(row: &serde_json::Value) -> Option<serde_json::Value> {
    let (method, path) = row.get("name")?.as_str()?.split_once(' ')?;
    Some(json!({
        "repo_name": row.get("repo_name"),
        "method": method,
        "path": path,
        "handler_fqn": row.get("handler_fqn"),
        "file_path": row.get("file_path"),
        "start_line": row.get("start_line"),
    }))
}

fn method_matches(filter: &str, method: &str) -> bool {
    method == ANY_METHOD || filter.eq_ignore_ascii_case(method)
}

/// Segment-wise prefix match; an endpoint `{variable}` matches any segment.
pub(crate) fn path_matches(filter: &str, path: &str) -> bool {
    let wanted: Vec<&str> = filter.split('/').filter(|s| !s.is_empty()).collect();
    let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    wanted.len() <= segments.len()
        && wanted
            .iter()
            .zip(&segments)
            .all(|(want, seg)| want == seg || (seg.starts_with('{') && seg.ends_with('}')))
}

/// Markdown table of a `list_endpoints` reply.
pub fn format_list_endpoints_markdown(result: &serde_json::Value) -> String {
    let rows = result
        .get("endpoints")
        .and_then(|v| v.as_array())
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    if rows.is_empty() {
        return "No HTTP endpoints matched.".to_string();
    }
    let mut out = String::from(
        "| Method | Path | Handler | Location | Repository |\n|---|---|---|---|---|\n",
    );
    for row in rows {
        let text = |key: &str| row.get(key).and_then(|v| v.as_str()).unwrap_or("?");
        let line = row.get("start_line").and_then(|v| v.as_i64()).unwrap_or(0);
        out.push_str(&format!(
            "| {} | `{}` | `{}` | {}:{line} | {} |\n",
            text("method"),
            text("path"),
            text("handler_fqn"),
            text("file_path"),
            text("repo_name"),
        ));
    }
    if result
        .get("truncated")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        let total = result
            .get("total_matches")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let bound = if result
            .get("total_is_lower_bound")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
        {
            "more than "
        } else {
            ""
        };
        out.push_str(&format!(
            "\nShowing first {} of {bound}{total} endpoints (truncated) — narrow with method/path/repo.",
            rows.len()
        ));
    }
    out
}

/// Render a `list_endpoints` reply in the requested CLI format.
pub fn format_endpoints_output(
    result: &serde_json::Value,
    output: crate::config::OutputFormat,
) -> String {
    super::format_json_or_markdown(result, output, format_list_endpoints_markdown)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(repo: &str, name: &str, handler: &str) -> serde_json::Value {
        json!({
            "repo_name": repo,
            "name": name,
            "file_path": "src/Api.java",
            "start_line": 7,
            "handler_fqn": handler,
        })
    }

    fn listed(result: &serde_json::Value) -> Vec<String> {
        result["endpoints"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| {
                format!(
                    "{} {}",
                    e["method"].as_str().unwrap(),
                    e["path"].as_str().unwrap()
                )
            })
            .collect()
    }

    fn sample() -> Vec<serde_json::Value> {
        vec![
            row("shop", "POST /api/owners", "a.Owners.save"),
            row("shop", "GET /api/owners/{id}", "a.Owners.read"),
            row("shop", "GET /api/ownership", "a.Owners.ownership"),
            row("shop", "ANY /api/owners/legacy", "a.Owners.legacy"),
            row("shop", "GET /api/owners", "a.Owners.list"),
        ]
    }

    #[test]
    fn test_list_endpoints_orders_by_path_then_method() {
        assert_eq!(
            listed(&filter_endpoints(&sample(), None, None, false)),
            vec![
                "GET /api/owners",
                "POST /api/owners",
                "ANY /api/owners/legacy",
                "GET /api/owners/{id}",
                "GET /api/ownership",
            ]
        );
    }

    #[test]
    fn test_list_endpoints_filters_by_method_and_path() {
        assert_eq!(
            listed(&filter_endpoints(
                &sample(),
                Some("get"),
                Some("/api/owners/42"),
                false
            )),
            vec!["GET /api/owners/{id}"]
        );
        assert_eq!(
            listed(&filter_endpoints(
                &sample(),
                Some("DELETE"),
                Some("/api/owners"),
                false
            )),
            vec!["ANY /api/owners/legacy"]
        );
        assert!(path_matches("api/owners/", "/api/owners"));
        assert!(!path_matches("/api/owners", "/api/ownership"));
        assert!(!path_matches("/api/owners/1/pets", "/api/owners/{id}"));
    }

    #[test]
    fn test_list_endpoints_markdown_and_truncation() {
        let result = filter_endpoints(&sample(), Some("POST"), None, false);
        let markdown = format_list_endpoints_markdown(&result);
        assert!(
            markdown.contains("| POST | `/api/owners` | `a.Owners.save` | src/Api.java:7 | shop |")
        );
        assert!(!markdown.contains("truncated"));
        let many: Vec<serde_json::Value> = (0..MAX_ENDPOINTS + 1)
            .map(|i| row("shop", &format!("GET /r{i}"), "a.R.get"))
            .collect();
        let result = filter_endpoints(&many, None, None, false);
        assert_eq!(result["total_matches"], MAX_ENDPOINTS + 1);
        assert!(format_list_endpoints_markdown(&result).contains("of 1001 endpoints (truncated)"));
        assert_eq!(
            format_list_endpoints_markdown(&filter_endpoints(&[], None, None, false)),
            "No HTTP endpoints matched."
        );
    }
}
