| Query | Repo | Target | Target cosine | Window cutoff (@400) | Cosine rank | Final rank | Source |
|---|---|---|---|---|---|---|---|
| find relevant code by meaning across the repository | knot | run_search_hybrid_context | - | 0.6455 | absent | ABSENT | outside-pool |
| acquire a client for talking to the database | HikariCP | getConnection | 0.6744 | 0.5832 | 3 | 4 | direct |
| get the callers of a symbol | csharp-code-map | GetCallersAsync | 0.7847 | 0.6803 | 1 | 6 | direct |
| find who invokes a given symbol | csharp-code-map | GetCallersAsync | 0.6838 | 0.6695 | 225 | ABSENT | direct |
| log a user in and issue a session token | job-watch-ui | login | 0.6821 | 0.5953 | 27 | 81 | direct |
| borrow a connection from the pool | HikariCP | getConnection | 0.7420 | 0.5848 | 3 | 2 | direct |
| authenticate user with email and password | job-watch | login | 0.7096 | 0.5855 | 3 | 1 | direct |
| take screenshot | chrome-devtools-mcp | screenshot | 0.6939 | 0.5349 | 3 | 1 | direct |
| capture the current view as an image | chrome-devtools-mcp | screenshot | 0.5912 | 0.5533 | 47 | 2 | direct |
