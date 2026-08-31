# Business opportunities from code intelligence

How to use Knot scan results in **portfolio, architecture, and engineering strategy** conversations — without tying the workflow to any specific EA tool.

Knot answers: *what exists in code, how it connects, and who depends on whom.*  
Business stakeholders need: *what to invest in, retire, consolidate, or modernise.*  
This document bridges the two.

---

## What Knot gives you (and what it does not)

### After indexing an organisation you have

| Asset | Source | Example question |
|-------|--------|------------------|
| **Repository inventory** | `knot repos` | How many codebases do we have? What languages? |
| **Code entities** | Neo4j graph | Classes, functions, APIs, K8s manifests |
| **Semantic search** | Qdrant vectors | “Where is billing implemented?” |
| **Call graphs** | `find_callers` / `knot callers` | Who uses this API? What breaks if we change it? |
| **Repo dependency graph** | `knot deps` | Which services share a library? |
| **Build metadata** | pom.xml, package.json, Cargo.toml, … | Maven vs npm vs Cargo footprint |

### Knot does not provide

- Runtime/production topology (pods, VMs, SaaS subscriptions)
- Cost, licensing, or FinOps data
- Business capability ownership or org chart
- Approved vs shadow IT governance state
- Automatic “application portfolio” classification

Those require other data sources. Knot is the **code-discovery layer**.

The **GenAI layer** (below) turns that raw inventory into a portfolio narrative — what each repo contains, what to do with it, and how repos relate.

---

## GenAI portfolio synthesis layer

Knot stores facts; a language model turns them into **portfolio intelligence**. Connect `knot-mcp` to Cursor, Claude Desktop, or any MCP client, then ask for an org-wide pass. The model calls Knot tools, grounds answers in indexed code, and produces a structured report.

### What the GenAI layer produces

| Deliverable | Knot tools used | LLM role |
|-------------|-----------------|----------|
| **Asset inventory** — list of indexed repos with language, build system, size | `list_repositories` | Normalise names, group by domain, flag outliers |
| **Contents summary** — what each repo does, main modules, key APIs | `search_hybrid_context`, `explore_file` | Summarise search hits into 2–4 sentences per repo |
| **Per-repo recommendations** — retire, consolidate, modernise, harden, invest | `find_callers`, `list_repo_dependencies` | Map signals to [opportunity patterns](#opportunity-patterns) |
| **Correlation map** — who depends on whom, clusters, duplication | `list_repo_dependencies` (forward + reverse) | Describe clusters, SPOFs, overlap between repos |

### Architecture

```
Indexed org (Neo4j + Qdrant)
        │
        ▼
   knot-mcp  ◄──── LLM client (Cursor, Claude, …)
        │
        ├── list_repositories        → inventory
        ├── search_hybrid_context    → domain / purpose per repo
        ├── list_repo_dependencies   → coupling graph
        ├── find_callers             → criticality / dead code
        └── explore_file             → entry points, structure
        │
        ▼
Portfolio report (markdown / JSON / slides)
```

Knot remains the **source of truth**; the model only interprets tool output. No hallucinated repo names if indexing is complete.

### MCP setup (Cursor example)

Add to Cursor MCP settings (paths and password from your environment):

```json
{
  "mcpServers": {
    "knot": {
      "command": "/path/to/knot/target/release/knot-mcp",
      "env": {
        "KNOT_NEO4J_URI": "bolt://localhost:7687",
        "KNOT_NEO4J_PASSWORD": "knot_secret_password",
        "KNOT_QDRANT_URL": "http://localhost:6334",
        "KNOT_REPO_PATH": "/path/to/org-clones"
      }
    }
  }
}
```

Ensure Docker (Neo4j + Qdrant) is running and the workspace is indexed. See [ORG-SCANNING.md](ORG-SCANNING.md) for the full A→Z flow.

### Prompt: full org portfolio pass

Paste into the MCP-enabled chat after indexing:

```text
Using knot MCP tools only, produce a portfolio report for all indexed repositories.

1. Call list_repositories (no filter) for the full inventory.
2. For each repository, in parallel where possible:
   - search_hybrid_context with a query like "main purpose entry point API"
   - list_repo_dependencies for forward and reverse edges
   - One find_callers on a prominent exported symbol if search reveals one
3. Output a table: repo | language | build | one-line purpose | depends_on | depended_on_by | recommendation
4. Add a "Correlations" section: dependency clusters, shared libraries, duplicate domains (repos that search similarly), and single points of failure.
5. Map each recommendation to: register | consolidate | modernise | retire | harden | invest — with one-sentence rationale grounded in tool results.

Do not invent repos or dependencies not returned by tools.
```

### Prompt: single-repo deep dive

```text
For repo <name>, using knot MCP:
- Summarise what it contains (search + explore_file on README or main package file)
- List upstream and downstream repos (list_repo_dependencies both directions)
- Assess criticality (find_callers on top exports)
- Recommend one primary initiative and list risks if we ignore it
```

### Prompt: find consolidation candidates

```text
Using knot MCP across all repos:
- search_hybrid_context for "<domain>" (e.g. authentication, billing, agent orchestration) without repo_name filter if supported, or run per repo
- Group repos with overlapping semantic results
- For each group, compare list_repo_dependencies and recommend consolidate vs keep separate
```

### Example output shape

The model should return something you can paste into Confluence, Notion, or a spreadsheet:

```markdown
## Portfolio summary
- **23 repositories** indexed | **5** TypeScript | **4** Python | …

## Assets

| Repo | Purpose | Stack | Depends on | Used by | Recommendation |
|------|---------|-------|------------|---------|----------------|
| auth-service | Session/JWT API | Go, Docker | lib-common | api-gateway, billing | **Harden** — high fan-in |
| legacy-billing | Invoice PDFs | Java 8 | — | — | **Retire** — no reverse deps, old stack |
| … | … | … | … | … | … |

## Correlations
- **Platform cluster:** lib-common ← auth-service, billing, notifications
- **Duplication:** repo-a and repo-b both implement "invoice" (search overlap) → **Consolidate**
- **SPOF:** lib-common has 8 reverse dependents → **Invest** in versioning and SLA
```

### Built-in portfolio report

Knot ships **`knot portfolio`** — a graph-based report (no LLM required) that produces:

- **Asset list** — every indexed repo with language, build system, entity counts
- **Dependencies** — `depends_on` and `depended_on_by` per repo
- **Recommendations** — rule-based actions: `register`, `retire`, `invest`, `harden`, `consolidate`
- **Correlations** — platform hubs (high fan-in) and isolated repos

```bash
knot portfolio                          # markdown table (default)
knot portfolio --output json            # feed to GenAI or CMDB
knot portfolio --filter auth --depth 3
```

Pair with **knot-mcp + GenAI** for narrative summaries per repo (`search_hybrid_context` on top of the JSON export).

### CLI-only GenAI handoff

Export facts, then feed JSON to any LLM:

```bash
knot repos --output json > inventory.json
for repo in $(knot repos --output json | jq -r '.[].name'); do
  knot deps "$repo" --output json > "deps-$repo.json"
  knot search "main application purpose" --repo "$repo" --max-results 5 --output markdown > "summary-$repo.md"
done
```

Attach `inventory.json` and selected `deps-*.json` / `summary-*.md` files to the chat with the same portfolio prompt.

### Guardrails

| Risk | Mitigation |
|------|------------|
| Stale index | Re-run `index-workspace.sh` before quarterly reviews |
| Incomplete index | `list_repositories` count vs expected clone count |
| Wrong repo in search | `--repo` / `repo_name` must match indexed name exactly |
| Model invents dependencies | Require citations from `list_repo_dependencies` JSON |
| Huge org (50+ repos) | Batch by language or folder; merge reports |

Use **`knot portfolio --output json`** as the grounded baseline before asking an LLM to elaborate per-repo narratives.

---

## Opportunity patterns

Use this table in architecture reviews or quarterly portfolio sessions. Each row is a repeatable finding type.

| # | Knot signal | Business opportunity | Typical action |
|---|-------------|----------------------|----------------|
| 1 | Repo in `knot repos` with no known owner in your CMDB | **Undocumented system** | Register, assign owner, define SLA |
| 2 | Many repos depend on one internal library (`deps --reverse`) | **Platform / shared capability** | Invest in platform team, versioning, docs |
| 3 | Repo with no reverse deps and low internal callers | **Retire or merge candidate** | Decommissioning study |
| 4 | Multiple repos match same domain (`search "invoice"`) | **Functional duplication** | Consolidation initiative |
| 5 | Old stack (language/build from `repos` table) | **Modernisation** | Migrate / replace programme |
| 6 | Very high fan-in on one module (`callers`) | **Critical dependency / SPOF** | Resilience, monitoring investment |
| 7 | Exported API with zero callers | **Dead surface area** | API cleanup, reduce attack surface |
| 8 | Identical dependency sets across many repos | **Standardisation gap** | Golden path, shared templates |
| 9 | K8s/Helm indexed (`--include-config-files`) | **Deployable unit clarity** | Link code repo to runtime service |
| 10 | Circular `deps` between repos | **Architecture violation** | Target-state remediation |

---

## Quarterly workflow

```
Clone org repos  →  knot-indexer (all)  →  Analyst queries  →  Opportunity backlog
```

### Week 1 — Discover

```bash
./scripts/index-workspace.sh ~/code/my-org
knot portfolio --output json > portfolio.json
```

Capture per repo: name, language, build system, file count, entity count.

### Week 2 — Analyse

For each repository (or top N by size / criticality):

```bash
REPO=auth-service

# Domain
knot search "authentication session token" --repo "$REPO" --max-results 10

# Criticality (fan-in from other repos)
knot callers "AuthService" --repo "$REPO"

# Coupling
knot deps "$REPO" --depth 2
knot deps "$REPO" --reverse

# Structure
knot explore "src/lib.rs" --repo "$REPO"   # adjust path
```

| Analysis question | Knot command | Output to record |
|-------------------|--------------|------------------|
| What does it do? | `search` | 2–3 sentence description |
| Who depends on it? | `deps --reverse` | Dependent repo list |
| What does it depend on? | `deps` | Upstream repos / libs |
| How coupled is it? | `callers` (count) | High / medium / low |
| Is code active? | `callers` on main exports | Dead vs live APIs |

### Week 3 — Prioritise opportunities

Score each finding (example rubric):

| Dimension | Low (1) | High (5) |
|-----------|---------|----------|
| Business impact | Internal tool | Revenue-critical path |
| Risk if ignored | Cosmetic | Security / compliance |
| Effort to fix | Days | Quarters |

Map to initiative types: **register**, **consolidate**, **modernise**, **retire**, **harden**.

### Week 4 — Communicate

Deliverables that work in any organisation:

- **Inventory spreadsheet** — from `knot repos --output json`
- **Dependency diagram** — from `knot deps` (manual or export to Mermaid)
- **Top 10 opportunities** — one slide per pattern above
- **Decision log** — consolidate vs retire vs migrate with rationale

---

## Export formats for downstream tools

Knot CLI supports structured output where available:

```bash
knot repos --output json
knot deps my-app --output json
knot search "payment" --repo my-app --output markdown
```

### Suggested inventory schema (JSON → CSV)

For import into spreadsheets, CMDBs, or EA tools:

| Field | Source |
|-------|--------|
| `repo_name` | `knot repos` → `name` |
| `primary_language` | `knot repos` → `primary_language` |
| `build_system` | `knot repos` → `build_system` |
| `file_count` | `knot repos` → `file_count` |
| `entity_count` | `knot repos` → `entity_count` |
| `depends_on` | `knot deps <name> --output json` |
| `depended_on_by` | `knot deps <name> --reverse --output json` |
| `description` | Manual + `knot search` summary |
| `criticality` | Derived from caller fan-in |
| `recommended_action` | From opportunity patterns table |

Example `jq` extract:

```bash
knot repos --output json | jq '.[] | {name, primary_language, build_system, file_count, entity_count}'
```

---

## Persona plays

### Engineering leadership

- **Before deprecation:** `knot callers "OldApi"` across all repos
- **Before extraction:** `knot deps` to see blast radius of splitting a monorepo
- **Tech debt:** zero-caller exported functions → cleanup backlog

### Enterprise architecture

- Current-state dependency map from `knot deps --depth 3`
- Duplicate domain implementations via semantic `search`
- Document target-state **decisions** when consolidation is chosen
- **GenAI portfolio pass** — MCP prompt over full org for inventory + recommendations + correlation map (quarterly artefact)

### Security / compliance

- Shadow repos: indexed in Knot but absent from official inventory
- Dead APIs with zero callers but still deployed
- Cross-repo dependency cycles → supply-chain review

### FinOps / vendor

- npm/Maven/NuGet entities per repo → licence audit input
- Shared SDK usage across repos → vendor concentration

---

## Example: single-repo opportunity pass

```bash
REPO=legacy-billing

knot repos --filter "$REPO"
knot search "invoice subscription" --repo "$REPO" --max-results 8
knot callers "BillingController" --repo "$REPO"
knot deps "$REPO" --reverse
```

**Checklist**

- [ ] Registered in official application inventory?
- [ ] Owner assigned?
- [ ] Dependent systems identified (`deps --reverse`)?
- [ ] Caller count justifies criticality rating?
- [ ] Overlap with another repo (`search` same keywords)?
- [ ] Initiative created: retire / migrate / consolidate?

---

## Gaps and contribution opportunities

Features that would strengthen the business-value story (not all implemented today):

| Gap | User impact | Possible Knot contribution |
|-----|-------------|---------------------------|
| No Git host API | Manual clone step | Org scanner CLI (`gh` integration) |
| No bulk JSON export | Hard to feed CMDB | `knot portfolio --output json` |
| No `externalId` on entities | Re-sync drift | Stable repo URL in Repository node |
| No scheduled re-index | Stale portfolio | Cron / watch mode docs + server |
| Config files off by default | Missing K8s link | Document `--include-config-files` in onboarding |

If you are contributing to Knot, see [ORG-SCANNING.md](ORG-SCANNING.md) for dev setup and [README](../README.md) for architecture.

---

## Quick reference

```bash
knot repos                              # inventory
knot portfolio [--output json]          # portfolio + recommendations + correlations
knot search "<domain>" --repo <name>    # understand purpose
knot callers "<Symbol>" --repo <name>   # criticality / impact
knot deps <name> [--reverse]            # coupling
knot explore "<path>" --repo <name>     # file anatomy
```

**GenAI (MCP):** `list_repositories` + `knot portfolio --output json` + per-repo `search_hybrid_context` for narratives.
