//! List Repository Dependencies Tool
//!
//! Shows the dependency graph for a repository, including which dependencies
//! are locally indexed. Enables AI agents to discover cross-repository
//! dependency relationships established through build file analysis.
//!
//! **Key Capabilities:**
//! - **Forward Dependency Lookup**: Discover all repositories that a given
//!   repository depends on via build system declarations (Maven, Gradle,
//!   Cargo, npm).
//! - **Reverse Dependency Lookup**: Find all repositories that depend on a
//!   given repository — critical for impact analysis before making breaking
//!   changes.
//! - **Transitive Traversal**: Follow dependency chains up to a configurable
//!   depth (default 3 levels) to understand the full dependency footprint.
//! - **Indexed vs Unindexed**: Distinguish between dependencies that are
//!   locally indexed (available for cross-repo call resolution) and those
//!   that are not.
//!
//! **Usage Guidelines:**
//! - Use BEFORE `find_callers` when working in a multi-repository codebase
//!   to understand which repos are available for cross-repo analysis.
//! - Use with `reverse: true` to assess the blast radius of a breaking
//!   change in a shared library.
//! - Start with `max_depth: 1` for immediate dependencies and increase
//!   only when deeper transitive analysis is needed.
//!
//! **Behavior & Return:**
//! - Read-only graph traversal with no side effects.
//! - Returns a JSON array of dependency repository names.
//! - When `reverse: true`, returns repositories that depend ON the target.
//! - Empty results mean no DEPENDS_ON relationships exist for that repo
//!   (either it has no build dependencies declared, or dependent repos
//!   haven't been indexed yet).
//!
//! **Parameter Guidance:**
//! - `repo_name` is required and must match the name used during indexing.
//! - `max_depth` controls transitive depth (1 = direct only, 3 = 3 levels).
//! - `reverse` toggles between forward and reverse dependency lookup.
//!
//! **Supported Build Systems:**
//! Maven (pom.xml), Gradle (build.gradle), Cargo (Cargo.toml), npm (package.json),
//! NuGet (`.csproj` + `Directory.Packages.props` for Central Package Management).

use rust_mcp_sdk::schema::*;
use serde_json::json;
use std::collections::HashMap;

use crate::mcp_handler::KnotMcpHandler;

pub struct ListRepoDependenciesTool;

impl ListRepoDependenciesTool {
    pub fn tool() -> Tool {
        let mut properties = HashMap::new();
        properties.insert(
            "repo_name".to_string(),
            serde_json::from_value(json!({
                "type": "string",
                "description": "Repository name to show dependencies for. Must match the name used during indexing (e.g., 'my-java-repo', 'auth-service'). This is REQUIRED — there is no default.",
                "minLength": 1,
                "maxLength": 255
            }))
            .unwrap(),
        );
        properties.insert(
            "max_depth".to_string(),
            serde_json::from_value(json!({
                "type": "integer",
                "description": "Maximum depth for transitive dependency traversal (default: 3). Use 1 for direct dependencies only. Higher values follow chains deeper. Must be between 1 and 10.",
                "minimum": 1,
                "maximum": 10,
                "default": 3
            }))
            .unwrap(),
        );
        properties.insert(
            "reverse".to_string(),
            serde_json::from_value(json!({
                "type": "boolean",
                "description": "If true, show repositories that depend ON this repo (reverse lookup). If false (default), show repositories this repo depends ON. Use reverse for impact analysis before breaking changes.",
                "default": false
            }))
            .unwrap(),
        );

        Tool {
            name: "list_repo_dependencies".to_string(),
description: Some(
                "Read-only cross-repository dependency graph lookup. \
                 Shows which repositories depend on each other via build system declarations (Maven, Gradle, Cargo, npm, NuGet). \
                 Answers 'which repos does this repo depend on?' and 'which repos depend on this repo?'. \
                 \n\nUsage: Use BEFORE cross-repo analysis to discover which other indexed repos are available for call tracing. \
                 Use reverse mode for impact analysis before making breaking changes in shared libraries. \
                 \n\nBehaviour & Return: Read-only graph traversal with no side effects. \
                 Returns a JSON array of repository names. Empty results mean no DEPENDS_ON relationships exist for that repo. \
                 \n\nParameter guidance: 'repo_name' is required and must match the name used during indexing. \
                 'max_depth' defaults to 3 (1 = direct only). 'reverse' toggles between forward and reverse dependency lookup. \
                 \n\nSupports all build systems indexed by knot: Maven, Gradle, Cargo, npm, NuGet (`.csproj` + Central Package Management via `Directory.Packages.props`). C# repos that previously reported `build_system: \"none\"` now report `\"nuget\"` on re-index; `knot-indexer --clean` is recommended for immediate effect."
                    .to_string(),
            ),
            input_schema: ToolInputSchema::new(
                vec!["repo_name".to_string()],
                Some(properties),
                None,
            ),
            annotations: None,
            execution: None,
            icons: vec![],
            meta: None,
            output_schema: None,
            title: None,
        }
    }

    pub async fn handle(
        params: CallToolRequestParams,
        handler: &KnotMcpHandler,
    ) -> std::result::Result<CallToolResult, CallToolError> {
        use crate::cli_tools;

        let args = params
            .arguments
            .ok_or_else(|| CallToolError::from_message("Missing arguments".to_string()))?;

        let repo_name = args
            .get("repo_name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                CallToolError::from_message("Missing required 'repo_name' parameter".to_string())
            })?;

        let max_depth = args.get("max_depth").and_then(|v| v.as_u64()).unwrap_or(3) as u32;

        let reverse = args
            .get("reverse")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        // Check if in offline mode
        if handler.graph_db.is_none() {
            return Err(CallToolError::from_message(
                "Server running in offline mode - graph database not available".to_string(),
            ));
        }

        let graph_db = handler.graph_db.as_ref().unwrap();

        let json_result = cli_tools::run_deps(repo_name, max_depth, reverse, graph_db)
            .await
            .map_err(|e| CallToolError::from_message(format!("Query error: {e}")))?;

        let formatted = cli_tools::format_deps_output(repo_name, reverse, &json_result);

        Ok(CallToolResult {
            content: vec![ContentBlock::TextContent(TextContent::new(
                formatted, None, None,
            ))],
            is_error: None,
            meta: None,
            structured_content: None,
        })
    }
}
