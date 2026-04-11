//! MCP Tools module - implements all available tools for the knot MCP server.

pub mod explore_file;
pub mod find_callers;
pub mod search_hybrid_context;

#[cfg(test)]
mod tests {
    use crate::mcp_tools::{
        explore_file::ExploreFileTool, find_callers::FindCallersTool,
        search_hybrid_context::SearchHybridContextTool,
    };

    #[test]
    fn test_all_tools_have_valid_names() {
        let explore = ExploreFileTool::tool();
        let find_callers = FindCallersTool::tool();
        let search = SearchHybridContextTool::tool();

        assert_eq!(explore.name, "explore_file");
        assert_eq!(find_callers.name, "find_callers");
        assert_eq!(search.name, "search_hybrid_context");
    }

    #[test]
    fn test_all_tools_have_descriptions() {
        let explore = ExploreFileTool::tool();
        let find_callers = FindCallersTool::tool();
        let search = SearchHybridContextTool::tool();

        assert!(explore.description.is_some());
        assert!(find_callers.description.is_some());
        assert!(search.description.is_some());

        assert!(!explore.description.unwrap().is_empty());
        assert!(!find_callers.description.unwrap().is_empty());
        assert!(!search.description.unwrap().is_empty());
    }

    #[test]
    fn test_all_tools_have_input_schema() {
        let explore = ExploreFileTool::tool();
        let find_callers = FindCallersTool::tool();
        let search = SearchHybridContextTool::tool();

        // All tools must have required parameters
        assert!(!explore.input_schema.required.is_empty());
        assert!(!find_callers.input_schema.required.is_empty());
        assert!(!search.input_schema.required.is_empty());

        // All tools must have properties defined
        assert!(explore.input_schema.properties.is_some());
        assert!(find_callers.input_schema.properties.is_some());
        assert!(search.input_schema.properties.is_some());
    }

    #[test]
    fn test_explore_file_schema_has_file_path() {
        let tool = ExploreFileTool::tool();
        let props = tool.input_schema.properties.unwrap();

        assert!(props.contains_key("file_path"));
        assert!(
            tool.input_schema
                .required
                .contains(&"file_path".to_string())
        );
    }

    #[test]
    fn test_find_callers_schema_has_entity_name() {
        let tool = FindCallersTool::tool();
        let props = tool.input_schema.properties.unwrap();

        assert!(props.contains_key("entity_name"));
        assert!(
            tool.input_schema
                .required
                .contains(&"entity_name".to_string())
        );
    }

    #[test]
    fn test_search_hybrid_context_schema_has_query() {
        let tool = SearchHybridContextTool::tool();
        let props = tool.input_schema.properties.unwrap();

        assert!(props.contains_key("query"));
        assert!(tool.input_schema.required.contains(&"query".to_string()));
    }

    #[test]
    fn test_all_tools_have_optional_repo_name() {
        let explore = ExploreFileTool::tool();
        let find_callers = FindCallersTool::tool();
        let search = SearchHybridContextTool::tool();

        // repo_name should be in properties for filtering
        let explore_props = explore.input_schema.properties.unwrap();
        let find_props = find_callers.input_schema.properties.unwrap();
        let search_props = search.input_schema.properties.unwrap();

        assert!(explore_props.contains_key("repo_name"));
        assert!(find_props.contains_key("repo_name"));
        assert!(search_props.contains_key("repo_name"));
    }
}
