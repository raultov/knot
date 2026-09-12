# MCP Dispatch API Plan (knot 1.9.3)

**Status:** proposed
**Target release:** knot 1.9.3
**Driver:** knot-server needs to expose the exact same MCP tool surface over an
HTTP `/mcp` endpoint without re-implementing (and therefore drifting from) the
tool table that `knot-mcp` serves over stdio.

---

## 1. Problem statement

`KnotMcpHandler` already owns the canonical MCP tool surface, but it is only
reachable through the `rust_mcp_sdk::mcp_server::ServerHandler` trait:

```rust
async fn handle_list_tools_request(
    &self,
    _request: Option<PaginatedRequestParams>,
    _runtime: Arc<dyn McpServer>,          // <-- unusable for embedders
) -> Result<ListToolsResult, RpcError>;

async fn handle_call_tool_request(
    &self,
    params: CallToolRequestParams,
    _runtime: Arc<dyn McpServer>,          // <-- unusable for embedders
) -> Result<CallToolResult, CallToolError>;
```

Both methods demand an `Arc<dyn McpServer>`. That trait has ~20 required
methods and is only ever constructed by the SDK's own
`server_runtime::create_server_instance`, which is not public API. An embedder
that wants to drive the tools from its own transport (knot-server's stateless
`/mcp`) therefore has two bad options:

1. Fabricate an `Arc<dyn McpServer>` — impractical.
2. Duplicate the 5-arm dispatch `match` and the `Tool::tool()` list downstream —
   works today, drifts tomorrow. Every new knot tool would silently fail to
   appear in knot-server until someone remembers to sync.

Note that both parameters are already named `_runtime` in knot: the tool
implementations never use them. The coupling is purely structural.

## 2. Objective

Promote the tool table and the dispatch `match` to public, runtime-free API on
`KnotMcpHandler`, and reduce the `ServerHandler` impl to a thin delegation.
After this change, adding a tool to knot automatically adds it to every
embedder, by construction rather than by convention.

## 3. Scope

**In scope**

- `src/mcp_handler.rs`: two new public associated items + delegation.
- Extraction of the dry-run message to a named constant so both surfaces (and
  their tests) assert the same string.
- Unit tests covering the new API and the delegation invariants.
- CHANGELOG + README note.

**Out of scope**

- Any change to tool behaviour, schemas, descriptions or output formatting.
- Any change to `build_server_details()`.
- Any HTTP transport inside knot. knot stays stdio-only; HTTP lives in
  knot-server.
- Any change to `knot-mcp`, `knot-indexer` or the `knot` CLI binaries.

## 4. Public API contract

```rust
impl KnotMcpHandler {
    /// The canonical MCP tool surface.
    ///
    /// Single source of truth shared by the stdio server (`knot-mcp`) and by
    /// any embedder that drives the tools from its own transport (for example
    /// knot-server's stateless `/mcp` endpoint). Adding a tool here makes it
    /// visible on every surface at once.
    pub fn tools() -> Vec<Tool>;

    /// Dispatch a tool call without an `Arc<dyn McpServer>` runtime.
    ///
    /// Behaviourally identical to `handle_call_tool_request`, which delegates
    /// here: the runtime argument was never read by any tool.
    pub async fn dispatch(
        &self,
        params: CallToolRequestParams,
    ) -> Result<CallToolResult, CallToolError>;
}

/// Error text returned by `dispatch` when the handler was built with
/// `new_dry_run()`.
pub const DRY_RUN_MESSAGE: &str = "...";
```

### 4.1 Invariants the tests must pin

| # | Invariant | Why it matters |
|---|---|---|
| I1 | `tools()` returns exactly the tools advertised by `handle_list_tools_request` | The whole point: no second list to keep in sync |
| I2 | `dispatch` returns exactly what `handle_call_tool_request` returns, for every input | Embedders must not observe a different surface than stdio clients |
| I3 | The dry-run guard lives in `dispatch`, not in the trait method | Otherwise an embedder bypasses it and hits `None` databases |
| I4 | Unknown tool names yield `CallToolError::unknown_tool` | Callers map this to `CallToolResult { is_error: true }`, per SDK runtime behaviour |
| I5 | `tools()` is a *free* function of no state | Lets embedders answer `tools/list` before any DB connection exists |

### 4.2 Backwards compatibility

This is strictly additive. `ServerHandler` keeps both methods with identical
signatures and identical observable behaviour, so `knot-mcp` and every existing
MCP client are unaffected. The release is a patch bump (1.9.2 → 1.9.3).

---

## 5. BDD specification

Written as executable intent; each scenario maps to one or more `#[test]` /
`#[tokio::test]` functions in §6.

```gherkin
Feature: Runtime-free MCP tool surface
  As an embedder of the knot library
  I want to list and invoke MCP tools without an Arc<dyn McpServer>
  So that I can serve the exact knot tool surface over my own transport

  Background:
    Given the knot MCP tool set is search_hybrid_context, find_callers,
      explore_file, list_repo_dependencies and list_repositories

  Scenario: The tool table is reachable without a handler instance
    When I call KnotMcpHandler::tools()
    Then I receive 5 tools
    And their names are exactly the tool set
    And no database connection was required

  Scenario: The trait method and the free function agree
    Given a dry-run handler
    When I call handle_list_tools_request
    And I call KnotMcpHandler::tools()
    Then both return the same tool names in the same order

  Scenario: Every advertised tool carries a usable schema
    When I call KnotMcpHandler::tools()
    Then every tool has a non-empty name
    And every tool has a non-empty description
    And every tool declares an object input schema

  Scenario: Dispatching an unknown tool is a tool error, not a panic
    Given a dry-run handler
    When I dispatch a call to "no_such_tool"
    Then I receive Err(CallToolError)
    And converting it to a CallToolResult yields is_error = true

  Scenario: A dry-run handler refuses to execute tools
    Given a dry-run handler
    When I dispatch a call to "search_hybrid_context"
    Then I receive Err(CallToolError)
    And the message equals DRY_RUN_MESSAGE

  Scenario: The dry-run guard cannot be bypassed through dispatch
    Given a dry-run handler
    When I dispatch a call to each of the 5 tools in turn
    Then every call is refused with DRY_RUN_MESSAGE

  Scenario: The trait call path and the direct call path agree
    Given a dry-run handler
    When I dispatch a call to "explore_file" directly
    And I would call handle_call_tool_request with the same params
    Then both paths produce the same error message

  Scenario: Offline handlers report missing databases, not dry-run
    Given a handler with no databases but dry_run = false
    When I dispatch a call to "search_hybrid_context"
    Then I receive Err(CallToolError)
    And the message mentions offline mode
    # Guards the ordering of the two guards: dry_run is checked first,
    # the per-tool database guard second.
```

---

## 6. TDD plan

Strict red → green → refactor. Every cycle starts by writing a test that
**fails to compile or fails to pass** against the current tree.

All tests live in `src/mcp_handler.rs` under the existing `#[cfg(test)] mod
tests`, next to the current `build_server_details` tests. No new files, no new
dev-dependencies. `#[tokio::test]` is already available via the `tokio` dep.

### Cycle 1 — `tools()` exists and is correct

**RED.** Add:

```rust
const EXPECTED_TOOLS: [&str; 5] = [
    "search_hybrid_context",
    "find_callers",
    "explore_file",
    "list_repo_dependencies",
    "list_repositories",
];

#[test]
fn tools_returns_the_full_surface() {
    let names: Vec<String> = KnotMcpHandler::tools().into_iter().map(|t| t.name).collect();
    assert_eq!(names, EXPECTED_TOOLS);
}
```

Fails to compile: no `tools()`.

**GREEN.** Add `pub fn tools() -> Vec<Tool>` returning the five `Tool::tool()`
calls in the order currently hardcoded in `handle_list_tools_request`.

**REFACTOR.** Rewrite `handle_list_tools_request` as:

```rust
Ok(ListToolsResult { tools: Self::tools(), meta: None, next_cursor: None })
```

### Cycle 2 — the two list paths cannot diverge (I1)

**RED.**

```rust
#[tokio::test]
async fn list_tools_request_matches_tools_fn() {
    let handler = KnotMcpHandler::new_dry_run();
    let via_trait = handler
        .handle_list_tools_request(None, dummy_runtime())
        .await
        .expect("list tools");
    let via_fn = KnotMcpHandler::tools();
    assert_eq!(
        via_trait.tools.iter().map(|t| &t.name).collect::<Vec<_>>(),
        via_fn.iter().map(|t| &t.name).collect::<Vec<_>>(),
    );
}
```

**Blocker to resolve first:** `handle_list_tools_request` needs an
`Arc<dyn McpServer>`, which is precisely what we cannot build. Two ways out —
pick one during implementation:

- **(a) Preferred.** Do not call the trait method at all. Instead assert the
  delegation structurally: a source-level guard test that reads
  `include_str!("mcp_handler.rs")` and asserts that the body of
  `handle_list_tools_request` contains `Self::tools()` and does **not** contain
  `Tool::tool()`. Crude but honest, cheap, and it fails loudly if someone
  re-inlines the list. This mirrors the drift-guard pattern already used in
  knot-server's `metrics.rs`.
- **(b)** Skip the cross-path assertion for `list_tools` and rely on the fact
  that the delegation is a one-liner reviewed in the diff.

Choose (a). It is the only mechanical protection available, and the invariant it
protects (I1) is the reason this plan exists.

**GREEN.** Already green after Cycle 1's refactor; the guard test simply locks
it in.

### Cycle 3 — tool schemas are well-formed

**RED.**

```rust
#[test]
fn every_tool_has_name_description_and_object_schema() {
    for tool in KnotMcpHandler::tools() {
        assert!(!tool.name.is_empty(), "empty tool name");
        let desc = tool.description.as_deref().unwrap_or_default();
        assert!(!desc.is_empty(), "{} has no description", tool.name);
        assert_eq!(tool.input_schema.type_, "object", "{}", tool.name);
    }
}
```

This should pass immediately (the `#[mcp_tool]` macro guarantees it). Keep it:
it is the regression net for a future hand-written tool.

### Cycle 4 — `DRY_RUN_MESSAGE` constant

**RED.**

```rust
#[test]
fn dry_run_message_is_descriptive() {
    assert!(DRY_RUN_MESSAGE.contains("dry-run mode"));
    assert!(DRY_RUN_MESSAGE.contains("Database connections are not available"));
}
```

Fails to compile: no constant.

**GREEN.** Extract the existing literal from `handle_call_tool_request` into

```rust
pub const DRY_RUN_MESSAGE: &str =
    "Server is running in dry-run mode. Database connections are not available. \
     This mode is used for protocol validation and quality checks only.";
```

and reference it from the guard. The string must stay byte-identical to today's
so no client observes a change.

### Cycle 5 — `dispatch()` refuses in dry-run mode (I3)

**RED.**

```rust
#[tokio::test]
async fn dispatch_refuses_in_dry_run_mode() {
    let handler = KnotMcpHandler::new_dry_run();
    let params = CallToolRequestParams {
        name: "search_hybrid_context".into(),
        arguments: None,
        ..Default::default()
    };
    let err = handler.dispatch(params).await.expect_err("dry-run must refuse");
    assert_eq!(err.to_string(), DRY_RUN_MESSAGE);
}
```

Fails to compile: no `dispatch()`.

**GREEN.** Add `dispatch()` with the dry-run guard followed by the `match`
lifted verbatim from `handle_call_tool_request`.

**REFACTOR.** Rewrite `handle_call_tool_request` as `self.dispatch(params).await`.
Add the same source-level drift guard as Cycle 2: the trait body must contain
`self.dispatch(` and must not contain `SearchHybridContextTool::handle`.

> Confirm during implementation whether `CallToolRequestParams` implements
> `Default`. If it does not, build the struct with all fields spelled out and
> add a small `fn call_params(name: &str) -> CallToolRequestParams` test helper
> so the other cycles stay readable.

### Cycle 6 — every tool is guarded (I3, exhaustively)

**RED.**

```rust
#[tokio::test]
async fn dispatch_refuses_every_tool_in_dry_run_mode() {
    let handler = KnotMcpHandler::new_dry_run();
    for name in EXPECTED_TOOLS {
        let err = handler
            .dispatch(call_params(name))
            .await
            .expect_err("dry-run must refuse");
        assert_eq!(err.to_string(), DRY_RUN_MESSAGE, "tool {name}");
    }
}
```

Green after Cycle 5. Its value is future-proofing: a new tool wired into
`dispatch` after the guard would break this test.

### Cycle 7 — unknown tools (I4)

**RED.**

```rust
#[tokio::test]
async fn dispatch_rejects_unknown_tool() {
    // dry_run = false so the unknown-tool arm is reached rather than the guard.
    let handler = KnotMcpHandler {
        vector_db: None,
        graph_db: None,
        embedder: None,
        dry_run: false,
    };
    let err = handler
        .dispatch(call_params("no_such_tool"))
        .await
        .expect_err("unknown tool must fail");
    assert!(err.to_string().contains("no_such_tool"));

    let result: CallToolResult = err.into();
    assert_eq!(result.is_error, Some(true));
}
```

The second half pins the conversion the SDK runtime performs
(`mcp_server_runtime.rs`: `CallToolError` → `CallToolResult { is_error: true }`,
delivered as a *successful* JSON-RPC response). Embedders must replicate it, so
knot should assert it.

### Cycle 8 — guard ordering (offline vs dry-run)

**RED.**

```rust
#[tokio::test]
async fn offline_handler_reports_missing_databases_not_dry_run() {
    let handler = KnotMcpHandler {
        vector_db: None,
        graph_db: None,
        embedder: None,
        dry_run: false,
    };
    let err = handler
        .dispatch(call_params("search_hybrid_context"))
        .await
        .expect_err("offline must fail");
    let msg = err.to_string();
    assert_ne!(msg, DRY_RUN_MESSAGE);
    assert!(msg.contains("offline mode"), "unexpected message: {msg}");
}
```

Green by construction; documents that `dry_run` is checked first and the
per-tool database guard second, which is the behaviour knot-server will rely on
when it builds a handler with `dry_run: false` and real `Arc`s.

### Cycle 9 — refactor pass

With all tests green:

- Collapse duplication between `EXPECTED_TOOLS` and `tools()` if a clean
  expression exists; if not, leave the literal array — an independently written
  expectation is the point of the test.
- Re-read `dispatch` for `clippy::cognitive_complexity` (warn-level in this
  crate). A 5-arm match plus one guard is well under the threshold.
- Confirm no `#[allow]` was introduced.

---

## 7. Implementation sketch

`src/mcp_handler.rs`, after the `impl KnotMcpHandler` block that holds `new` and
`new_dry_run`:

```rust
/// Error text returned when a dry-run handler is asked to execute a tool.
pub const DRY_RUN_MESSAGE: &str =
    "Server is running in dry-run mode. Database connections are not available. \
     This mode is used for protocol validation and quality checks only.";

impl KnotMcpHandler {
    /// The canonical MCP tool surface, independent of any transport or runtime.
    ///
    /// `knot-mcp` serves it over stdio; embedders such as knot-server serve the
    /// same list over HTTP. Keeping one list here is what makes those surfaces
    /// identical by construction.
    pub fn tools() -> Vec<Tool> {
        vec![
            SearchHybridContextTool::tool(),
            FindCallersTool::tool(),
            ExploreFileTool::tool(),
            ListRepoDependenciesTool::tool(),
            ListRepositoriesTool::tool(),
        ]
    }

    /// Execute a tool call without an `Arc<dyn McpServer>`.
    ///
    /// The SDK's `ServerHandler` demands a runtime handle that no tool reads;
    /// this entry point drops it so an embedder can drive the tools from its
    /// own transport. `handle_call_tool_request` delegates here, so both paths
    /// are the same code.
    pub async fn dispatch(
        &self,
        params: CallToolRequestParams,
    ) -> std::result::Result<CallToolResult, CallToolError> {
        if self.dry_run {
            return Err(CallToolError::from_message(DRY_RUN_MESSAGE.to_string()));
        }

        match params.name.as_str() {
            "search_hybrid_context" => SearchHybridContextTool::handle(params, self).await,
            "find_callers" => FindCallersTool::handle(params, self).await,
            "explore_file" => ExploreFileTool::handle(params, self).await,
            "list_repo_dependencies" => ListRepoDependenciesTool::handle(params, self).await,
            "list_repositories" => ListRepositoriesTool::handle(params, self).await,
            _ => Err(CallToolError::unknown_tool(params.name)),
        }
    }
}
```

And the trait impl shrinks to:

```rust
#[async_trait]
impl ServerHandler for KnotMcpHandler {
    async fn handle_list_tools_request(
        &self,
        _request: Option<PaginatedRequestParams>,
        _runtime: Arc<dyn McpServer>,
    ) -> std::result::Result<ListToolsResult, RpcError> {
        Ok(ListToolsResult {
            tools: Self::tools(),
            meta: None,
            next_cursor: None,
        })
    }

    async fn handle_call_tool_request(
        &self,
        params: CallToolRequestParams,
        _runtime: Arc<dyn McpServer>,
    ) -> std::result::Result<CallToolResult, CallToolError> {
        self.dispatch(params).await
    }
}
```

Net change: roughly +30 lines of API and docs, −12 lines from the trait impl,
plus tests.

---

## 8. Risks

| Risk | Mitigation |
|---|---|
| Someone re-inlines the tool list or the match into the trait impl | Source-level drift guards (Cycles 2 and 5) |
| `DRY_RUN_MESSAGE` extraction silently changes the string | Cycle 4 asserts the substrings; diff review confirms byte equality |
| `CallToolRequestParams` has no `Default`, breaking the test helpers | Resolve in Cycle 5 with an explicit constructor helper |
| Downstream pins `knot = "1.9.2"` and never sees the API | knot-server bumps to `1.9.3` as part of its own plan |

---

## 9. Quality gates

Per `AGENTS.md`, run the `validator` subagent after implementation:

1. `cargo fmt`
2. `cargo clippy --all-targets --all-features -- -D warnings`
3. `cargo test`
4. `./tests/run_all_e2e_fast.sh`

No `#[allow(...)]`. No `unsafe`. Edition 2024.

The E2E suite is the real proof of I2: it drives `knot-mcp` over stdio through
the unchanged trait path. If the tool surface is intact there, the delegation is
correct.

---

## 10. Release checklist

- [ ] Cycles 1–9 complete, all tests green
- [ ] `validator` clean
- [ ] `README.md`: MCP tools section notes that `KnotMcpHandler::tools()` and
      `::dispatch()` are the supported embedding API
- [ ] `CHANGELOG.md`: new `## [1.9.3]` entry under *Added* —
      "`KnotMcpHandler::tools()` and `KnotMcpHandler::dispatch()`: runtime-free
      access to the MCP tool surface for library embedders" — and under
      *Changed* — "`ServerHandler` methods now delegate to the new API; no
      behavioural change for MCP clients"
- [ ] `Cargo.toml` version → `1.9.3`
- [ ] **Ask the maintainer before publishing to crates.io** (`AGENTS.md`)
- [ ] Downstream: knot-server switches from `[patch.crates-io]` to
      `knot = "1.9.3"` before its own release
