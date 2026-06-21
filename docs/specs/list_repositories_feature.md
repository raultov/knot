# Specification: List Repositories with Filtering (CLI & MCP)

## 1. Overview
This specification details the addition of a `list_repositories` capability for the `knot-mcp` server, alongside an enhancement to the existing `knot repos` CLI command to support an optional, case-insensitive `filter` parameter. The development process will strictly follow BDD/TDD methodologies.

## 2. BDD Scenarios (Behavior)

### Feature: Repository Listing and Filtering

**Scenario 1: CLI lists all repositories without a filter**
*   **Given** the codebase has indexed repositories "frontend-app" and "backend-api"
*   **When** the user runs `knot repos`
*   **Then** the output should contain both "frontend-app" and "backend-api"

**Scenario 2: CLI filters repositories (case-insensitive)**
*   **Given** the codebase has indexed repositories "frontend-app", "Backend-API", and "auth-service"
*   **When** the user runs `knot repos --filter app`
*   **Then** the output should contain "frontend-app"
*   **And** should NOT contain "Backend-API" or "auth-service"
*   **When** the user runs `knot repos --filter API`
*   **Then** the output should contain "Backend-API" (case-insensitive match)

**Scenario 3: MCP Agent lists all repositories**
*   **Given** the MCP server is connected
*   **When** the agent invokes the `list_repositories` tool without arguments
*   **Then** it receives a markdown table containing all indexed repositories

**Scenario 4: MCP Agent filters repositories**
*   **Given** the MCP server is connected
*   **When** the agent invokes the `list_repositories` tool with `{"filter": "auth"}`
*   **Then** it receives a markdown table containing only repositories matching "auth"

## 3. TDD Steps (Implementation Strategy)

**Rule:** For every step, tests MUST be written or updated *before* or *alongside* the logic implementation, ensuring they fail first if the logic isn't present, then pass.

### Step 3.1: CLI Models (`src/models/cli_args.rs`)
1.  **Test:** Update `test_cli_parser_repos_command` and related tests in `mod tests` to expect a `filter: None` field. Add a new test `test_cli_parser_repos_with_filter` that parses `knot repos --filter search_term`.
2.  **Implementation:** Add `#[arg(short, long)] filter: Option<String>` to the `Commands::Repos` variant.

### Step 3.2: CLI Logic (`src/cli_tools/repos.rs`)
1.  **Test:** In `mod tests`, add a test `test_run_list_repos_with_filter` using a mocked or stubbed `sample_repos()` array to verify that passing `Some("API")` correctly filters out non-matching repositories case-insensitively.
2.  **Implementation:** 
    *   Change signature: `pub async fn run_list_repos(filter: Option<&str>, graph_db: &Arc<GraphDb>) -> anyhow::Result<serde_json::Value>`
    *   Apply case-insensitive filtering logic (`to_lowercase()`) on the `name` field of the returned JSON objects.

### Step 3.3: CLI Binary (`src/bin/knot.rs`)
1.  **Implementation:** Update the `match cli.command` block for `Commands::Repos { filter, output }` to pass `filter.as_deref()` to `cli_tools::run_list_repos`.

### Step 3.4: MCP Tool Definition (`src/mcp_tools/list_repositories.rs`)
1.  **Test:** Create `mod tests` inside this new file. Add tests:
    *   `test_tool_schema_has_optional_filter`: Verify the `filter` parameter is present but NOT required.
2.  **Implementation:** 
    *   Create the file and define `ListRepositoriesTool`.
    *   Build the `Tool` schema matching the CLI capability.
    *   Implement `handle()`: extract `filter`, call `cli_tools::run_list_repos`, and format using `OutputFormat::Markdown`.

### Step 3.5: MCP Handler & Registration (`src/mcp_handler.rs` & `src/mcp_tools/mod.rs`)
1.  **Test:** Update `src/mcp_tools/mod.rs` tests (`test_all_tools_have_valid_names`, etc.) to include `ListRepositoriesTool`.
2.  **Implementation:**
    *   Add `pub mod list_repositories;` to `src/mcp_tools/mod.rs`.
    *   Register the tool in `handle_list_tools_request` in `src/mcp_handler.rs`.
    *   Add the route in `handle_call_tool_request`.
    *   Update `build_server_details` instructions to mention the new tool.

## 4. Documentation
1.  **README.md**: Update the CLI commands section to show `knot repos --filter <name>`. Update the MCP tools list to include `list_repositories`.
2.  **CHANGELOG.md**: Add an entry under `Unreleased` (or `[Added]`) for the new `list_repositories` MCP tool and the `--filter` flag for `knot repos`.
3.  **AGENTS.md / SKILL.md (Optional)**: If the `knot-server-repos` skill files are part of this repository, update them to reflect the new capabilities.

## 5. Quality Gates
Once implemented, run the following to verify:
```bash
cargo fmt -- --check
cargo clippy --all-targets -- -D warnings
cargo test
./tests/run_all_e2e_fast.sh
```
