# Workstream B measurement — MultilingualE5Small (384-dim, asymmetric)

Measured 2026-09-17 after re-indexing the six harness repos with `KNOT_EMBED_MODEL=MultilingualE5Small`.
Prefixes: `query: ` (query side) and `passage: ` (passage side).

| Query | Repo | Target | Target cosine | Window cutoff (@400) | Cosine rank | Final rank | Source |
|---|---|---|---|---|---|---|---|
| find relevant code by meaning across the repository | knot | run_search_hybrid_context | 0.8578 | 0.8409 | 50 | 91 | direct |
| acquire a client for talking to the database | HikariCP | getConnection | 0.8243 | 0.7986 | 7 | 23 | direct |
| get the callers of a symbol | csharp-code-map | GetCallersAsync | 0.8578 | 0.8300 | 28 | 33 | direct |
| find who invokes a given symbol | csharp-code-map | GetCallersAsync | 0.8357 | 0.8278 | 175 | ABSENT | direct |
| log a user in and issue a session token | job-watch-ui | login | 0.8555 | 0.8241 | 25 | 83 | direct |
| borrow a connection from the pool | HikariCP | getConnection | 0.8700 | 0.8081 | 2 | 2 | direct |
| authenticate user with email and password | job-watch | login | 0.8287 | 0.8059 | 86 | 8 | direct |
| take screenshot | chrome-devtools-mcp | screenshot | 0.8731 | 0.8147 | 3 | 1 | direct |
| capture the current view as an image | chrome-devtools-mcp | screenshot | 0.8407 | 0.8076 | 3 | 1 | direct |

## Observations

- MultilingualE5Small shifts all cosine values up to the 0.80 - 0.88 range.
- `run_search_hybrid_context` enters the candidate pool inside the window at rank 50 (cosine 0.8578 vs cutoff 0.8409), but finishes 91st after re-ranking.
- `borrow a connection` achieves cosine rank 2 and final rank 2.
- However, relative cosine separation is narrow across the entire pool, leading to `must` harness regressions on multiple baseline rows (`borrow a connection`, `authenticate user`, `get the callers`).
