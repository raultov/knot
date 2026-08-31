//! List Portfolio Tool
//!
//! Builds a codebase portfolio report across all indexed repositories:
//! current state, correlations, risk signals, and optional Gemini recommendations.

use rust_mcp_sdk::schema::*;
use serde_json::json;
use std::collections::HashMap;

use crate::mcp_handler::KnotMcpHandler;

pub struct ListPortfolioTool;

impl ListPortfolioTool {
    pub fn tool() -> Tool {
        let mut properties = HashMap::new();
        properties.insert(
            "filter".to_string(),
            serde_json::from_value(json!({
                "type": "string",
                "description": "Optional case-insensitive substring filter on repository name.",
                "minLength": 1,
                "maxLength": 255
            }))
            .unwrap(),
        );
        properties.insert(
            "skip_ai".to_string(),
            serde_json::from_value(json!({
                "type": "boolean",
                "description": "When true, skip the Gemini API call and return structured data only.",
                "default": false
            }))
            .unwrap(),
        );
        properties.insert(
            "horizon".to_string(),
            serde_json::from_value(json!({
                "type": "string",
                "description": "Forecast horizon for strategic advisor (e.g. 12m, 18m, 24m).",
                "default": "18m"
            }))
            .unwrap(),
        );
        properties.insert(
            "team_size".to_string(),
            serde_json::from_value(json!({
                "type": "integer",
                "description": "Engineering team size hint for resource planning.",
                "minimum": 1
            }))
            .unwrap(),
        );
        properties.insert(
            "focus".to_string(),
            serde_json::from_value(json!({
                "type": "string",
                "description": "Strategic focus hint (e.g. healthcare SaaS)."
            }))
            .unwrap(),
        );

        Tool {
            name: "list_portfolio".to_string(),
            description: Some(
                "Read-only codebase portfolio across ALL indexed repositories. \
                 Returns current state (repo weights, roles), structural correlations (DEPENDS_ON), \
                 runtime coupling (cross-repo CALLS), risk signals, and Gemini advisor sections \
                 (inventory, resource planning, forecast, actions, benchmarks, recommendations) when KNOT_GEMINI_API_KEY is set. \
                 Use BEFORE deep-diving into a single repo to understand the whole workspace. \
                 Complements list_repositories (inventory only) and list_repo_dependencies (single-repo deps)."
                    .to_string(),
            ),
            input_schema: ToolInputSchema::new(vec![], Some(properties), None),
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
        use crate::config::OutputFormat;

        let args = params.arguments.unwrap_or_default();
        let filter = args.get("filter").and_then(|v| v.as_str());
        let skip_ai = args
            .get("skip_ai")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let horizon = args.get("horizon").and_then(|v| v.as_str());
        let team_size = args
            .get("team_size")
            .and_then(|v| v.as_u64())
            .and_then(|n| u32::try_from(n).ok());
        let focus = args.get("focus").and_then(|v| v.as_str());
        let exclude: Vec<String> = args
            .get("exclude")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();

        if handler.graph_db.is_none() {
            return Err(CallToolError::from_message(
                "Server running in offline mode - graph database not available".to_string(),
            ));
        }

        let graph_db = handler.graph_db.as_ref().unwrap();

        let options = crate::portfolio::portfolio_options_from_env(crate::portfolio::PortfolioOptions {
            filter: filter.map(String::from),
            exclude,
            skip_ai,
            horizon: horizon.map(String::from).unwrap_or_default(),
            team_size,
            focus: focus.map(String::from),
            ..Default::default()
        });

        let report = cli_tools::run_portfolio(options, graph_db)
        .await
        .map_err(|e| CallToolError::from_message(format!("Portfolio error: {e}")))?;

        let formatted = cli_tools::format_portfolio_report_output(&report, OutputFormat::Markdown);

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

    #[test]
    fn test_tool_schema() {
        let tool = ListPortfolioTool::tool();
        assert_eq!(tool.name, "list_portfolio");
        let props = tool.input_schema.properties.unwrap();
        assert!(props.contains_key("filter"));
        assert!(props.contains_key("skip_ai"));
        assert!(props.contains_key("horizon"));
        assert!(props.contains_key("team_size"));
        assert!(props.contains_key("focus"));
    }
}
