//! MCP Tools module - implements all available tools for the knot MCP server.

pub mod explore_file;
pub mod find_callers;
pub mod list_repo_dependencies;
pub mod list_repositories;
pub mod search_hybrid_context;

use crate::models::RepoScope;

/// Extract the repository scope from tool-call arguments.
/// Accepts: absent/null → All · string (comma-separated, "all"/"*" sentinel) → parse
/// · array of strings → joined then parsed.
pub(crate) fn repo_scope_from_args(args: &serde_json::Map<String, serde_json::Value>) -> RepoScope {
    RepoScope::from_json(args.get("repo_name"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mcp_tools::{
        explore_file::ExploreFileTool, find_callers::FindCallersTool,
        list_repo_dependencies::ListRepoDependenciesTool, list_repositories::ListRepositoriesTool,
        search_hybrid_context::SearchHybridContextTool,
    };

    #[test]
    fn repo_scope_from_args_absent_is_all() {
        let args = serde_json::Map::new();
        assert_eq!(repo_scope_from_args(&args), RepoScope::All);
    }

    #[test]
    fn repo_scope_from_args_string_all() {
        let mut args = serde_json::Map::new();
        args.insert("repo_name".to_string(), serde_json::json!("all"));
        assert_eq!(repo_scope_from_args(&args), RepoScope::All);
    }

    #[test]
    fn repo_scope_from_args_string_list() {
        let mut args = serde_json::Map::new();
        args.insert("repo_name".to_string(), serde_json::json!("a,b"));
        assert_eq!(
            repo_scope_from_args(&args),
            RepoScope::Many(vec!["a".to_string(), "b".to_string()])
        );
    }

    #[test]
    fn repo_scope_from_args_array() {
        let mut args = serde_json::Map::new();
        args.insert("repo_name".to_string(), serde_json::json!(["a", "b"]));
        assert_eq!(
            repo_scope_from_args(&args),
            RepoScope::Many(vec!["a".to_string(), "b".to_string()])
        );
    }

    #[test]
    fn repo_scope_from_args_null_is_all() {
        let mut args = serde_json::Map::new();
        args.insert("repo_name".to_string(), serde_json::Value::Null);
        assert_eq!(repo_scope_from_args(&args), RepoScope::All);
    }

    #[test]
    fn test_scope_descriptions_mention_all_and_lists() {
        let explore = ExploreFileTool::tool();
        let find_callers = FindCallersTool::tool();
        let search = SearchHybridContextTool::tool();

        let tools = [explore, find_callers, search];
        for tool in tools {
            let props = tool.input_schema.properties.unwrap();
            let repo_prop = props.get("repo_name").unwrap();
            let desc = repo_prop.get("description").unwrap().as_str().unwrap();

            assert!(
                desc.contains("'all'"),
                "tool {} repo_name description does not contain 'all': {}",
                tool.name,
                desc
            );
            assert!(
                desc.contains("comma-separated"),
                "tool {} repo_name description does not contain 'comma-separated': {}",
                tool.name,
                desc
            );
        }
    }

    #[test]
    fn test_all_tools_have_valid_names() {
        let explore = ExploreFileTool::tool();
        let find_callers = FindCallersTool::tool();
        let search = SearchHybridContextTool::tool();
        let deps = ListRepoDependenciesTool::tool();
        let repos = ListRepositoriesTool::tool();

        assert_eq!(explore.name, "explore_file");
        assert_eq!(find_callers.name, "find_callers");
        assert_eq!(search.name, "search_hybrid_context");
        assert_eq!(deps.name, "list_repo_dependencies");
        assert_eq!(repos.name, "list_repositories");
    }

    #[test]
    fn test_all_tools_have_descriptions() {
        let explore = ExploreFileTool::tool();
        let find_callers = FindCallersTool::tool();
        let search = SearchHybridContextTool::tool();
        let deps = ListRepoDependenciesTool::tool();
        let repos = ListRepositoriesTool::tool();

        assert!(explore.description.is_some());
        assert!(find_callers.description.is_some());
        assert!(search.description.is_some());
        assert!(deps.description.is_some());
        assert!(repos.description.is_some());

        assert!(!explore.description.unwrap().is_empty());
        assert!(!find_callers.description.unwrap().is_empty());
        assert!(!search.description.unwrap().is_empty());
        assert!(!deps.description.unwrap().is_empty());
        assert!(!repos.description.unwrap().is_empty());
    }

    #[test]
    fn test_all_tools_have_input_schema() {
        let explore = ExploreFileTool::tool();
        let find_callers = FindCallersTool::tool();
        let search = SearchHybridContextTool::tool();
        let deps = ListRepoDependenciesTool::tool();
        let repos = ListRepositoriesTool::tool();

        // All tools must have required parameters (except list_repositories which has optional filter)
        assert!(!explore.input_schema.required.is_empty());
        assert!(!find_callers.input_schema.required.is_empty());
        assert!(!search.input_schema.required.is_empty());
        assert!(!deps.input_schema.required.is_empty());
        assert!(repos.input_schema.required.is_empty());

        // All tools must have properties defined
        assert!(explore.input_schema.properties.is_some());
        assert!(find_callers.input_schema.properties.is_some());
        assert!(search.input_schema.properties.is_some());
        assert!(deps.input_schema.properties.is_some());
        assert!(repos.input_schema.properties.is_some());
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

    #[test]
    fn test_list_repo_dependencies_schema_has_repo_name() {
        let tool = ListRepoDependenciesTool::tool();
        let props = tool.input_schema.properties.unwrap();

        assert!(props.contains_key("repo_name"));
        assert!(
            tool.input_schema
                .required
                .contains(&"repo_name".to_string())
        );
    }

    #[test]
    fn test_list_repositories_schema_has_optional_filter() {
        let tool = ListRepositoriesTool::tool();
        let props = tool.input_schema.properties.unwrap();

        assert!(props.contains_key("filter"));
        assert!(!tool.input_schema.required.contains(&"filter".to_string()));
    }
}
