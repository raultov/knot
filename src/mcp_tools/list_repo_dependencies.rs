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
//! - When `reverse: true`, returns repositories that depend ON the target
//!   (transitive, up to `max_depth`).
//! - Empty results are explained in the response text with a three-way
//!   classification: the repository is not indexed; it declares no build
//!   dependencies; it declares N dependencies of which some RESOLVE to an
//!   indexed repository but have no DEPENDS_ON edge yet (stale graph —
//!   listed with the resolved target and a re-index hint); or none of them
//!   resolves (listed verbatim, uncapped). The reverse direction mirrors
//!   this: consumers that declare the queried repository without an edge
//!   are named instead of a blanket "no repositories depend on it".
//!
//! **Parameter Guidance:**
//! - `repo_name` is required and must match the name used during indexing.
//! - `max_depth` controls transitive depth (1 = direct only, 3 = 3 levels).
//! - `reverse` toggles between forward and reverse dependency lookup.
//!
//! **Supported Build Systems:**
//! Maven (pom.xml), Gradle (build.gradle), Cargo (Cargo.toml), npm (package.json),
//! NuGet (`.csproj` + `Directory.Packages.props` for Central Package Management).

use rust_mcp_sdk::macros::{JsonSchema, mcp_tool};
use rust_mcp_sdk::schema::*;

use crate::mcp_handler::KnotMcpHandler;

/// Input contract for `list_repo_dependencies`.
///
/// The `#[mcp_tool]` macro derives `ListRepoDependenciesTool::tool()` from this
/// declaration, so the JSON Schema advertised over MCP stays in lockstep with
/// the fields documented here.
#[mcp_tool(
    name = "list_repo_dependencies",
    title = "List cross-repository dependencies",
    description = "Read-only cross-repository dependency graph lookup. \
                   Shows which repositories depend on each other via build system declarations (Maven, Gradle, Cargo, npm, NuGet). \
                   Answers 'which repos does this repo depend on?' and 'which repos depend on this repo?'. \
                   \n\nUsage: Use BEFORE cross-repo analysis to discover which other indexed repos are available for call tracing. \
                   Use reverse mode for impact analysis before making breaking changes in shared libraries. \
                   \n\nBehaviour & Return: Read-only graph traversal with no side effects. \
                   Returns a JSON array of repository names. Empty results mean no DEPENDS_ON relationships exist for that repo. \
                   Empty lookups are explained in the response text with a three-way classification: \
                   declares-but-resolves-without-edge (stale graph, re-index hint), declares-but-nothing-resolves (not indexed), \
                   or nothing declared; the reverse direction names consumers that declare the repo without an edge yet. \
                   \n\nParameter guidance: 'repo_name' is required and must match the name used during indexing. \
                   'max_depth' defaults to 3 (1 = direct only) and applies to both directions — in reverse mode it follows dependents transitively. \
                   'reverse' toggles between forward and reverse dependency lookup. \
                   \n\nSupports all build systems indexed by knot: Maven, Gradle, Cargo, npm, NuGet (`.csproj` + Central Package Management via `Directory.Packages.props`). C# repos that previously reported `build_system: \"none\"` now report `\"nuget\"` on re-index; `knot-indexer --clean` is recommended for immediate effect.",
    read_only_hint = true,
    destructive_hint = false,
    idempotent_hint = true,
    open_world_hint = false
)]
#[derive(JsonSchema)]
pub struct ListRepoDependenciesTool {
    #[json_schema(
        description = "Repository name to show dependencies for. Must match the name used during indexing (e.g., 'my-java-repo', 'auth-service'). This is REQUIRED — there is no default.",
        min_length = 1,
        max_length = 255
    )]
    pub repo_name: String,
    #[json_schema(
        description = "Maximum depth for transitive dependency traversal (default: 3, max: 10). Use 1 for direct dependencies only. Requests above 10 are clamped to 10 (and below 1 to 1). Applies to both directions: with reverse=true it follows dependents transitively.",
        minimum = 1,
        maximum = 10,
        default = 3
    )]
    pub max_depth: Option<i64>,
    #[json_schema(
        description = "If true, show repositories that depend ON this repo (reverse lookup). If false (default), show repositories this repo depends ON. Use reverse for impact analysis before breaking changes.",
        default = false
    )]
    pub reverse: Option<bool>,
}

impl ListRepoDependenciesTool {
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

        // Parse and enforce the advertised bound through the single shared
        // resolver so the handler can never drift from the schema. A depth
        // of 0 would also compile into invalid Cypher (`*1..0`).
        let raw_depth: u64 = args
            .get("max_depth")
            .and_then(|v| v.as_u64())
            .unwrap_or(cli_tools::DEFAULT_MAX_DEPTH as u64);
        let requested_depth: u32 = raw_depth.min(u32::MAX as u64) as u32;
        let max_depth = cli_tools::resolve_max_depth(requested_depth);
        let depth_notice = if max_depth != requested_depth {
            if requested_depth == 0 {
                Some("> Note: `max_depth` was floored from 0 to the minimum of 1.\n".to_string())
            } else {
                Some(format!(
                    "> Note: `max_depth` was clamped from {requested_depth} to the advertised maximum of {}.\n",
                    cli_tools::MAX_DEPTH_CEILING
                ))
            }
        } else {
            None
        };

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

        // An empty result is explained honestly in the response text:
        // dependencies declared but no indexed repo resolves, repo not
        // indexed, or genuinely no declared dependencies. Never a bare
        // "No dependencies found."
        let diagnostics = if json_result.as_array().is_some_and(|a| a.is_empty()) {
            cli_tools::collect_deps_diagnostics(repo_name, reverse, graph_db)
                .await
                .ok()
        } else {
            None
        };

        let mut formatted = cli_tools::format_deps_output_with_diagnostics(
            repo_name,
            reverse,
            &json_result,
            diagnostics.as_ref(),
        );
        if let Some(notice) = depth_notice {
            formatted.push('\n');
            formatted.push_str(&notice);
        }

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli_tools::{DEFAULT_MAX_DEPTH, MAX_DEPTH_CEILING};

    #[test]
    fn schema_maximum_equals_enforced_ceiling() {
        // Drift guard: the JSON Schema `maximum` advertised over MCP must
        // equal the clamp the shared core actually enforces.
        let tool = ListRepoDependenciesTool::tool();
        let props = tool.input_schema.properties.unwrap();
        let max_prop = props.get("max_depth").unwrap();
        let maximum = max_prop.get("maximum").unwrap().as_i64().unwrap();
        assert_eq!(maximum, MAX_DEPTH_CEILING as i64);
    }

    #[test]
    fn schema_default_equals_default_constant() {
        let tool = ListRepoDependenciesTool::tool();
        let props = tool.input_schema.properties.unwrap();
        let default = props
            .get("max_depth")
            .unwrap()
            .get("default")
            .unwrap()
            .as_i64()
            .unwrap();
        assert_eq!(default, DEFAULT_MAX_DEPTH as i64);
    }

    #[test]
    fn max_depth_description_documents_clamping() {
        let tool = ListRepoDependenciesTool::tool();
        let props = tool.input_schema.properties.unwrap();
        let desc = props
            .get("max_depth")
            .unwrap()
            .get("description")
            .unwrap()
            .as_str()
            .unwrap();
        assert!(desc.contains("clamped"));
    }
}
