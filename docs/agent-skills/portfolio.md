# Portfolio — Multi-Repo Asset Management

## When to use

Use `knot portfolio` or MCP `list_portfolio` when you need a **workspace-wide** view of all indexed repositories — not a single repo.

| Tool | Scope |
|------|-------|
| `list_repositories` | Inventory only (counts, language) |
| `list_repo_dependencies` | One repo's dependency tree |
| **`list_portfolio`** | **All repos**: weights, roles, correlations, signals, Gemini recommendations |

## CLI

```bash
# Full portfolio + Gemini recommendations (requires KNOT_GEMINI_API_KEY)
knot portfolio --output markdown

# Filter to Synthapse repos
knot portfolio --filter synth --output markdown

# Structured data only (no API call)
knot portfolio --no-ai --output json

# Save markdown report to file (includes repo descriptions + README excerpts)
knot portfolio --output-file portfolio-report.md

# Resource planning hints for the GenAI advisor
knot portfolio --horizon 24m --team-size 5 --focus "healthcare SaaS" --output-file portfolio-report.md

# Save JSON for scripting
knot portfolio --no-ai --output-file portfolio-report.json --output json
```

## Documentation in the report

Each repository asset may include:

| Field | Source |
|-------|--------|
| `identity` | Maven/Cargo/npm project identity from build files |
| `description` | `package.json`, `Cargo.toml`, or project metadata |
| `readme_excerpt` | Indexed `README.md` content (requires markdown in index) |

If README sections are empty, ensure the repo was indexed with markdown files present.

## Gemini setup

Add to `~/.config/knot/.env`:

```bash
KNOT_GEMINI_API_KEY=your_key_here
KNOT_GEMINI_MODEL=gemini-3.6-flash
KNOT_PORTFOLIO_HORIZON=18m
KNOT_PORTFOLIO_TEAM_SIZE=5
KNOT_PORTFOLIO_FOCUS=healthcare SaaS
```

Never commit API keys to the repository.

## MCP

Tool: `list_portfolio`

Parameters:
- `filter` (optional) — case-insensitive repo name substring
- `skip_ai` (optional, default false) — skip Gemini call
- `horizon` (optional, default `18m`) — forecast window for strategic advisor
- `team_size` (optional) — engineering capacity hint for resource planning
- `focus` (optional) — strategic focus hint (e.g. "healthcare SaaS")

Returns markdown with:
- **Current state** — repo roles (hub/leaf/isolated/balanced), entity weights
- **Repository documentation** — identity, description, README excerpts
- **Correlations** — `DEPENDS_ON` (structural) and cross-repo `CALLS` (runtime coupling)
- **Signals** — stale index, high coupling, index-library-first hints
- **Portfolio Advisor (GenAI)** — when `KNOT_GEMINI_API_KEY` is set:
  - **Organizational Asset Inventory** — what the org already holds
  - **Resource Planning and Prioritization** — P0/P1/P2 initiatives
  - **Strategic Forecast** — market outlook for the horizon
  - **Recommended Actions** — concrete next steps per repo
  - **Real-World Benchmarks** — analogies to known products/companies
  - **Overall Portfolio Recommendation** — workspace-level strategy
  - **Business Potential by Repository** — per-repo product/business potential

If the API key is missing or `--no-ai` is used, the advisor section shows an explicit notice instead of failing silently.

**Note:** `prowler` is excluded by default (third-party cloud scanner). Override with `--exclude` for additional repos.

## Typical agent workflow

1. `list_portfolio` — understand the whole workspace
2. `list_repo_dependencies` — drill into one repo's deps
3. `search_hybrid_context` / `find_callers` — deep dive with explicit `repo_name`
