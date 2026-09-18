# Entry-point measurement — v1.11.0 shipped default (BGEBaseENV15, 768-dim)

Measured 2026-09-19 against the live dev index after a **full re-index of
all 29 source-available repositories** with `KNOT_EMBED_MODEL=BGEBaseENV15`
(Qdrant collection recreated at 768 dims; 83,746 points / 30 repos). The
ranker is the Workstream-B **pool-normalized** one: each candidate pool is
min–max normalized before the boosts are added
(`search_hybrid_context::rank::normalize_pool_cosines`), so the boost
constants no longer depend on the model's cosine band.

This supersedes `entrypoint_cosine_workstream_b_bgebase.md`, which measured
the *raw-cosine* ranker on a partial index.

## Baselines (no-regression guardrails)

| Query | Repo | Target | Target cosine | Window cutoff (@400) | Cosine rank | Final rank | Source |
|---|---|---|---|---|---|---|---|
| borrow a connection from the pool | HikariCP | getConnection | 0.5638 | 0.4365 | 5 | **1** | direct |
| authenticate user with email and password | job-watch | login | 0.5758 | 0.4456 | 11 | **6** | direct |
| take screenshot | chrome-devtools-mcp | screenshot | 0.6922 | 0.4897 | 2 | **1** | direct |
| capture the current view as an image | chrome-devtools-mcp | screenshot | 0.5273 | 0.4456 | 11 | **1** | direct |

**3 of 4 `must` baselines rank #1.** `authenticate user` is the residual
(see below).

## Targets (residual-recall rows)

| Query | Repo | Target | Target cosine | Window cutoff (@400) | Cosine rank | Final rank | Source |
|---|---|---|---|---|---|---|---|
| find relevant code by meaning across the repository | knot | run_search_hybrid_context | 0.5899 | 0.5929 | 458 | ABSENT | direct |
| acquire a client for talking to the database | HikariCP | getConnection | 0.5672 | 0.4754 | 5 | 7 | direct |
| get the callers of a symbol | csharp-code-map | GetCallersAsync | 0.7696 | 0.6157 | 1 | 2 | direct |
| find who invokes a given symbol | csharp-code-map | GetCallersAsync | 0.6348 | 0.5884 | 66 | ABSENT | direct |
| log a user in and issue a session token | job-watch-ui | login | 0.6251 | 0.4937 | 4 | 14 | direct |

### Cosine rank vs. the v6 (AllMiniLML6V2) baseline

| Target row | v6 baseline cosine rank | BGE-base cosine rank | Δ |
|---|---|---|---|
| knot `run_search_hybrid_context` | absent (probe 0.2489) | 458 | improved (now in the pool) |
| HikariCP `getConnection` (acquire) | 7 | 5 | improved |
| csharp `GetCallersAsync` (get callers) | 8 | 1 | improved |
| csharp `GetCallersAsync` (find who invokes) | 250 | 66 | improved |
| job-watch-ui `login` | 14 | 4 | improved |

No target row regressed; every row improved on its v6 cosine rank.

## Rank-recall harness (`run_rank_recall_live.sh`)

| Query | Repo | Expected | Measured | Verdict |
|---|---|---|---|---|
| borrow a connection from the pool | HikariCP | #1 getConnection | 1 | PASS |
| authenticate user with email and password | job-watch | #1 login | 6 | **FAIL** |
| capture the current view as an image | chrome-devtools-mcp | #1 screenshot | 1 | PASS |
| take screenshot | chrome-devtools-mcp | #1 screenshot | 1 | PASS |
| acquire a client for talking to the database | HikariCP | info | 7 | INFO |
| get the callers of a symbol | csharp-code-map | ≤5 GetCallersAsync | 1 | PASS |
| log a user in and issue a session token | job-watch-ui | ≤8 login | 8 | PASS |
| find relevant code by meaning across the repository | knot | info | ABSENT | INFO |
| find who invokes a given symbol | csharp-code-map | info | ABSENT | INFO |

## Residual: `authenticate user with email and password` (job-watch)

`login` is **inside** the cosine window (cosine 0.5758 vs cutoff 0.4456,
cosine rank 11) yet finishes at #6. This is a **ranking residual, not a
recall residual** — contrast the two `info` rows above, which are genuinely
outside the window.

Cause (measured with `RUST_LOG=search_hybrid_context::rank=debug`):

| rank | entity | cosine | cosine_norm | direct roots | kind |
|---|---|---|---|---|---|
| 1 | `patch_user` | 0.538 | 0.707 | 2 | rust_function |
| 2 | `normalize_email_rejects_empty_local_or_domain` | 0.597 | 1.000 | 1 | rust_function |
| 3 | `normalize_email_rejects_domain_without_dot` | 0.572 | 0.873 | 1 | rust_function |
| 4 | `normalize_email_rejects_no_at_sign` | 0.571 | 0.867 | 1 | rust_function |
| 5 | `bootstrap_email` | 0.570 | 0.865 | 1 | rust_function |
| 6 | `login` | 0.576 | 0.894 | 1 | rust_function |

BGE-base's stronger semantics promote the *inline* `#[cfg(test)]` helpers in
`src/` (`normalize_email_rejects_*`, `bootstrap_email`) to the top of the
pool, and `patch_user` reaches more of them than `login` does. Coverage — the
rank contract's entry-point signal — then selects `patch_user`.

This is not fixable query-time within Workstream B's scope: the ranker's
only test discriminator is `is_test_path(file_path)`, which matches test
**files**, not `#[cfg(test)]` modules inside `src/`. The graph knows the
entity-level `is_test_context` flag, but it is **not carried in the Qdrant
payload** (`src/db/vector/upsert.rs`), so no query-time component can see it.
Closing this residual means either carrying `is_test_context` into the
payload (a schema change and another full re-index) or reworking the root
seed — both outside the normalization + adoption deliverable.

For reference, the same ranker on the same query under `AllMiniLML6V2` puts
`login` at **#1**; the residual is the price of BGE-base's stronger semantic
recall, which is what every target row above gained.
