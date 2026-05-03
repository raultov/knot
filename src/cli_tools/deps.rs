//! Core deps logic shared between CLI and MCP.
//!
//! Shows the dependency graph for a repository, including which
//! dependencies are locally indexed via the DEPENDS_ON relationship.

use std::sync::Arc;

use crate::db::graph::{GraphDb, QueryExt};

pub async fn run_deps(
    repo_name: &str,
    max_depth: u32,
    reverse: bool,
    graph_db: &Arc<GraphDb>,
) -> anyhow::Result<serde_json::Value> {
    if reverse {
        let dependents = graph_db.find_repo_dependents(repo_name).await?;
        let result: Vec<serde_json::Value> = dependents
            .into_iter()
            .map(|d| serde_json::json!({ "repo_name": d }))
            .collect();
        Ok(serde_json::json!(result))
    } else {
        let deps = graph_db
            .find_repo_dependencies(repo_name, max_depth)
            .await?;
        let result: Vec<serde_json::Value> = deps
            .into_iter()
            .map(|d| serde_json::json!({ "repo_name": d }))
            .collect();
        Ok(serde_json::json!(result))
    }
}

pub fn format_deps_output(repo_name: &str, reverse: bool, result: &serde_json::Value) -> String {
    let mut output = String::new();

    if reverse {
        output.push_str(&format!(
            "# Repositories that depend on `{}`\n\n",
            repo_name
        ));
    } else {
        output.push_str(&format!("# Dependencies of `{}`\n\n", repo_name));
    }

    if let Some(arr) = result.as_array() {
        if arr.is_empty() {
            output.push_str("No dependencies found.\n");
        } else {
            for dep in arr {
                if let Some(name) = dep.get("repo_name").and_then(|v| v.as_str()) {
                    output.push_str(&format!("+-- {}\n", name));
                }
            }
        }
    } else {
        output.push_str("No dependencies found.\n");
    }

    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_format_deps_output_empty() {
        let result = json!([]);
        let formatted = format_deps_output("my-app", false, &result);
        assert!(formatted.contains("Dependencies of `my-app`"));
        assert!(formatted.contains("No dependencies found"));
    }

    #[test]
    fn test_format_deps_output_with_deps() {
        let result = json!([
            {"repo_name": "auth-lib"},
            {"repo_name": "common-utils"}
        ]);
        let formatted = format_deps_output("my-app", false, &result);
        assert!(formatted.contains("Dependencies of `my-app`"));
        assert!(formatted.contains("+-- auth-lib"));
        assert!(formatted.contains("+-- common-utils"));
    }

    #[test]
    fn test_format_deps_output_reverse() {
        let result = json!([
            {"repo_name": "my-app"},
            {"repo_name": "admin-portal"}
        ]);
        let formatted = format_deps_output("auth-lib", true, &result);
        assert!(formatted.contains("Repositories that depend on `auth-lib`"));
        assert!(formatted.contains("+-- my-app"));
        assert!(formatted.contains("+-- admin-portal"));
    }

    #[test]
    fn test_format_deps_output_single_dep() {
        let result = json!([
            {"repo_name": "core-lib"}
        ]);
        let formatted = format_deps_output("my-app", false, &result);
        assert!(formatted.contains("Dependencies of `my-app`"));
        assert!(formatted.contains("+-- core-lib"));
    }
}
