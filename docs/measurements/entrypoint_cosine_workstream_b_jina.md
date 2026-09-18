| Query | Repo | Target | Target cosine | Window cutoff (@400) | Cosine rank | Final rank | Source |
|---|---|---|---|---|---|---|---|
| find relevant code by meaning across the repository | knot | run_search_hybrid_context | - | 0.4360 | absent | ABSENT | outside-pool |
| acquire a client for talking to the database | HikariCP | getConnection | 0.5036 | 0.1933 | 1 | 15 | direct |
| get the callers of a symbol | csharp-code-map | GetCallersAsync | 0.5946 | 0.4254 | 5 | 2 | direct |
| find who invokes a given symbol | csharp-code-map | GetCallersAsync | - | 0.3830 | absent | ABSENT | outside-pool |
| log a user in and issue a session token | job-watch-ui | login | 0.6513 | 0.2312 | 2 | 3 | direct |
| borrow a connection from the pool | HikariCP | getConnection | 0.6620 | 0.2667 | 1 | 4 | direct |
| authenticate user with email and password | job-watch | login | 0.5436 | 0.1481 | 1 | 1 | direct |
| take screenshot | chrome-devtools-mcp | screenshot | 0.6879 | 0.2708 | 1 | 1 | direct |
| capture the current view as an image | chrome-devtools-mcp | screenshot | 0.5081 | 0.2022 | 1 | 1 | direct |
