| Query | Repo | Target | Target cosine | Window cutoff (@400) | Cosine rank | Final rank | Source |
|---|---|---|---|---|---|---|---|
| find relevant code by meaning across the repository | knot | run_search_hybrid_context | 0.5814* | 0.5928 | absent | ABSENT* | path-probe |
| acquire a client for talking to the database | HikariCP | getConnection | 0.5672 | 0.4754 | 5 | 8 | direct |
| get the callers of a symbol | csharp-code-map | GetCallersAsync | 0.7696 | 0.6157 | 1 | 2 | direct |
| find who invokes a given symbol | csharp-code-map | GetCallersAsync | 0.6348 | 0.5884 | 66 | ABSENT | direct |
| log a user in and issue a session token | job-watch-ui | login | 0.6251 | 0.4925 | 4 | 29 | direct |
| borrow a connection from the pool | HikariCP | getConnection | 0.5638 | 0.4365 | 5 | 1 | direct |
| authenticate user with email and password | job-watch | login | 0.5758 | 0.4456 | 11 | 7 | direct |
| take screenshot | chrome-devtools-mcp | screenshot | 0.6922 | 0.4897 | 2 | 1 | direct |
| capture the current view as an image | chrome-devtools-mcp | screenshot | 0.5273 | 0.4456 | 11 | 1 | direct |
