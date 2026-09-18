> **BGE-small was measured and rejected.** With the v1.11.0 scale-invariant
> ranker it still fails 3 of 4 `must` baselines live — its cosine
> neighbourhood for `borrow a connection` is eviction methods, so the entry
> point loses both cosine position and root coverage. The adopted default is
> `BGEBaseENV15`; see `entrypoint_cosine_v1_11_bgebase.md`.

| Query | Repo | Target | Target cosine | Window cutoff (@400) | Cosine rank | Final rank | Source |
|---|---|---|---|---|---|---|---|
| find relevant code by meaning across the repository | knot | run_search_hybrid_context | 0.6417 | 0.6440 | 423 | ABSENT | direct |
| acquire a client for talking to the database | HikariCP | getConnection | 0.6375 | 0.5383 | 5 | 5 | direct |
| get the callers of a symbol | csharp-code-map | GetCallersAsync | 0.7984 | 0.6619 | 1 | 3 | direct |
| find who invokes a given symbol | csharp-code-map | GetCallersAsync | 0.6560 | 0.6352 | 168 | ABSENT | direct |
| log a user in and issue a session token | job-watch-ui | login | 0.6763 | 0.5779 | 15 | 31 | direct |
| borrow a connection from the pool | HikariCP | getConnection | 0.7006 | 0.5501 | 6 | 12 | direct |
| authenticate user with email and password | job-watch | login | 0.6350 | 0.5231 | 22 | 12 | direct |
| take screenshot | chrome-devtools-mcp | screenshot | 0.7893 | 0.5484 | 2 | 1 | direct |
| capture the current view as an image | chrome-devtools-mcp | screenshot | 0.6610 | 0.5702 | 3 | 2 | direct |
