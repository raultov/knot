| Query | Repo | Target | Target cosine | Window cutoff (@400) | Cosine rank | Final rank | Source |
|---|---|---|---|---|---|---|---|
| find relevant code by meaning across the repository | knot | run_search_hybrid_context | - | 0.2958 | absent | ABSENT | outside-pool |
| acquire a client for talking to the database | HikariCP | getConnection | 0.2634 | 0.0625 | 3 | 14 | direct |
| get the callers of a symbol | csharp-code-map | GetCallersAsync | 0.4346 | 0.2592 | 6 | 5 | direct |
| find who invokes a given symbol | csharp-code-map | GetCallersAsync | 0.2566 | 0.2233 | 221 | ABSENT | direct |
| log a user in and issue a session token | job-watch-ui | login | 0.3876 | 0.1521 | 13 | 19 | direct |
| borrow a connection from the pool | HikariCP | getConnection | 0.4541 | 0.2239 | 7 | 1 | direct |
| authenticate user with email and password | job-watch | login | 0.2512 | 0.0410 | 48 | 1 | direct |
| take screenshot | chrome-devtools-mcp | screenshot | 0.5742 | 0.1800 | 1 | 1 | direct |
| capture the current view as an image | chrome-devtools-mcp | screenshot | 0.3133 | 0.1221 | 5 | 1 | direct |
