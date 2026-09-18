# Entry-point cosine-window baseline (v1.9.8, AllMiniLML6V2)

Measured 2026-09-17 against the live dev index (16 repos, AllMiniLML6V2,
embed_text schema v5) with `tests/measure_entrypoint_cosine.sh`.

Columns: the target's raw cosine vs. the plain Qdrant window cutoff at
`-m 100` (`candidate_limit(100) = 400`). `*` marks a cosine sampled from a
path-restricted re-search (`-p`), compared against the UNRESTRICTED cutoff —
"absent" in the Cosine-rank column means the target was not in the
unrestricted pool at all. Cosine <= cutoff ⇒ outside the semantic window:
no query-time ranking change can reach the target.

| Query | Repo | Target | Target cosine | Window cutoff (@400) | Cosine rank | Final rank | Source |
|---|---|---|---|---|---|---|---|
| find relevant code by meaning across the repository | knot | run_search_hybrid_context | 0.2489* | 0.2813 | absent | ABSENT* | path-probe |
| acquire a client for talking to the database | HikariCP | getConnection | 0.2365 | 0.0033 | 7 | 9 | direct |
| get the callers of a symbol | csharp-code-map | GetCallersAsync | 0.4000 | 0.2256 | 8 | 9 | direct |
| find who invokes a given symbol | csharp-code-map | GetCallersAsync | 0.2257 | 0.2012 | 250 | ABSENT | direct |
| log a user in and issue a session token | job-watch-ui | login | 0.3670 | 0.1509 | 14 | 20 | direct |
| borrow a connection from the pool | HikariCP | getConnection | 0.5020 | 0.1385 | 4 | 1 | direct |
| authenticate user with email and password | job-watch | login | 0.2413 | 0.0324 | 42 | 1 | direct |
| take screenshot | chrome-devtools-mcp | screenshot | 0.5274 | 0.2044 | 2 | 1 | direct |
| capture the current view as an image | chrome-devtools-mcp | screenshot | 0.3764 | 0.1198 | 1 | 1 | direct |
| create a client to chat with a language model | spring-ai | create | 0.5322 | 0.3494 | 1 | 1 | direct |

## Interpretation (baseline)

- knot `run_search_hybrid_context`: probe-sampled cosine 0.2489 < cutoff 0.2813 —
  outside the pool, outside the window. RECALL (model/embed_text).
- csharp-code-map "find who invokes…": cosine 0.2257, rank 250 of pool — inside
  the pool but far below the 0.2012 cutoff. RECALL.
- job-watch-ui `login`: cosine 0.3670, rank 14, vs cutoff 0.1509 — inside the
  window; the re-ranker finishing at 20 is a ranking residual, smaller than
  the other rows.
- HikariCP `acquire a client`: cosine rank 7, inside the window at
  cutoff 0.0033 — the window is deep enough; ranking residual.
- All baselines finish #1 (`create`, `screenshot` x2, `login` x2 via boost paths).
