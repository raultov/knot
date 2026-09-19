# Embedding-model quality matrix — knot 1.11.0

The entry-point harness tables for both supported models side by side. The
harnesses are `tests/run_rank_recall_live.sh` (final rank) and
`tests/measure_entrypoint_cosine.sh` (raw cosine + cosine rank), both of which
print the active model and accept `--out`.

**This document is honest about provenance.** Only the BGE-base side has been
measured against the v1.11.0 pool-normalized ranker. The MiniLM column is the
**v1.9.8 raw-cosine** reference; a v1.11.0 MiniLM run is the open §F7
measurement and is listed as such below.

## 1. BGEBaseENV15, 768-dim — v1.11.0 ranker `[measured]`

Source: `entrypoint_cosine_v1_11_bgebase.md` (full re-index of 29 repos, 83,746
points, pool-normalized `normalize_pool_cosines`).

### Baselines (no-regression guardrails)

| Query | Repo | Target | Cosine rank | Final rank | Must (#1)? |
|---|---|---|---|---|---|
| borrow a connection from the pool | HikariCP | getConnection | 5 | **1** | ✅ |
| authenticate user with email and password | job-watch | login | 11 | **6** | ❌ |
| take screenshot | chrome-devtools-mcp | screenshot | 2 | **1** | ✅ |
| capture the current view as an image | chrome-devtools-mcp | screenshot | 11 | **1** | ✅ |

### Targets (residual-recall rows)

| Query | Repo | Target | Cosine rank | Final rank |
|---|---|---|---|---|
| find relevant code by meaning across the repository | knot | run_search_hybrid_context | 458 | ABSENT |
| acquire a client for talking to the database | HikariCP | getConnection | 5 | 7 |
| get the callers of a symbol | csharp-code-map | GetCallersAsync | 1 | 2 |
| find who invokes a given symbol | csharp-code-map | GetCallersAsync | 66 | ABSENT |
| log a user in and issue a session token | job-watch-ui | login | 4 | 14 |

## 2. AllMiniLML6V2, 384-dim — v1.9.8 raw-cosine ranker `[reference only]`

Source: `entrypoint_cosine_baseline.md`. This is the **pre-pool-normalized**
ranker (additive boosts calibrated to MiniLM's cosine band), so it is **not**
directly comparable with §1; it is kept as the v1.10.0-era behaviour that §F7
must not regress.

| Query | Repo | Target | Cosine rank | Final rank |
|---|---|---|---|---|
| find relevant code by meaning across the repository | knot | run_search_hybrid_context | absent | ABSENT |
| acquire a client for talking to the database | HikariCP | getConnection | 7 | 9 |
| get the callers of a symbol | csharp-code-map | GetCallersAsync | 8 | 9 |
| find who invokes a given symbol | csharp-code-map | GetCallersAsync | 250 | ABSENT |
| log a user in and issue a session token | job-watch-ui | login | 14 | 20 |
| borrow a connection from the pool | HikariCP | getConnection | 4 | 1 |
| authenticate user with email and password | job-watch | login | 42 | 1 |
| take screenshot | chrome-devtools-mcp | screenshot | 2 | 1 |
| capture the current view as an image | chrome-devtools-mcp | screenshot | 1 | 1 |

## 3. Cosine rank, side by side

| Target row | MiniLM v1.9.8 | BGE-base v1.11.0 | Δ |
|---|---|---|---|
| knot `run_search_hybrid_context` | absent | 458 | in the pool now |
| HikariCP `getConnection` (acquire) | 7 | 5 | −2 |
| csharp `GetCallersAsync` (get callers) | 8 | 1 | −7 |
| csharp `GetCallersAsync` (find who invokes) | 250 | 66 | −184 |
| job-watch-ui `login` | 14 | 4 | −10 |

BGE-base dominates on cosine rank for every target row. The cost is the
baseline regression in §1 (`authenticate user`, #1 → #6) plus the footprint in
`model_cost_1_11.md`.

## 4. Acceptance contract status

The §F8 contract requires, **for each model**: the four `must` baseline rows at
#1, no target cosine-rank regression against that model's own baseline, and
`run_rank_recall_live.sh` exit 0.

| Model | must rows at #1 | `run_rank_recall_live.sh` | Status |
|---|---|---|---|
| BGEBaseENV15 (v1.11.0 ranker) | 3 / 4 | **fails** (`authenticate user` → #6) | open |
| AllMiniLML6V2 (v1.11.0 ranker) | unknown | unknown | **not measured — §F7** |

This is why the shipped default is MiniLM, not BGE-base: the zero-reindex,
zero-config upgrade from `v1.10.0` must hold, and BGE-base is an opt-in whose
recall gains are real but whose `authenticate user` baseline regresses.

## 5. Open measurements

1. **§F7 — MiniLM under the v1.11.0 ranker.** Run both harnesses on a MiniLM
   index of the six bench repos (D6: `knot`, `HikariCP`, `csharp-code-map`,
   `chrome-devtools-mcp`, `job-watch-ui`, `job-watch`) and compare against §2.
   If a row regresses versus the v1.10.0-era behaviour, fix the rule for both
   models (no per-model constants).
   **Blocker:** the live MiniLM index currently holds 9 repos but lacks
   `HikariCP`, `csharp-code-map` and `chrome-devtools-mcp`, and both harnesses
   skip wholesale when any required repo is absent. Indexing those three is the
   gating action.
2. **BGE `authenticate user` baseline.** #6 under BGE-base. If BGE is to meet
   the same acceptance contract as the default, this is the row to fix — and
   the fix must not be a BGE-only constant.
3. **Re-measure BGE quality after any fix** into a fresh
   `knot_entities_bge768` (the environment reset wiped the previous one).
