# Knot Search: Semantic Code Discovery

**Command:** `knot search "<query>" [--max-results <N>] [--repo <name>]`

## Purpose

Find code entities by semantic meaning. This is your primary tool for exploratory searches when you don't know exact names or locations.

## Parameters

- **`<query>`** (required): Natural language description of what you're looking for
  - Examples: "user authentication", "error handling", "database connection", "API routes"
  - Good queries describe *what the code does*, not specific names
  - Works best with 2-5 word descriptions

- **`--max-results <N>`**: Limit the number of results (default: 5, max: 20)
  - Use higher values (10-20) when exploring unfamiliar codebases
  - Use lower values (3-5) when you need focused results

- **`--repo <name>`**: Filter to a specific repository (optional)
  - Defaults to auto-detecting the current directory's repository name
  - Use when working with multiple indexed repositories
  - Example: `--repo backend` to search only in the backend repo

## Output Format

Results are formatted as Markdown with:
- Entity names and types (function, class, method, etc.)
- File locations (file path and line number)
- Function/method signatures (parameters and return types)
- Documentation and comments from the source code
- Related dependencies and usage patterns

### Example Output

```markdown
# Search Results for "authentication"

Found 3 entity(entities):

## Functions

- `authenticateUser` (line 42)
  - Signature: `async authenticateUser(email: string, password: string): Promise<User>`
  - Doc: Authenticates a user with email and password using bcrypt
  - File: src/auth/auth.ts
```

## When to Use Search

- **Feature Discovery:** Finding code that handles a specific feature
- **Pattern Location:** Searching for architectural patterns (e.g., "caching strategy")
- **Code Exploration:** When you don't know exact class/function names
- **Cross-Language Analysis:** Finding similar functionality across Java, TypeScript, Kotlin, etc.
- **Refactoring Discovery:** Locating all implementations of a pattern before refactoring

## Query Tips for Better Results

### ✅ Good Semantic Queries

```bash
knot search "user login validation"        # Specific and descriptive
knot search "database connection pooling"  # Describes the pattern
knot search "JWT token refresh"            # Clear functionality
knot search "error logging middleware"     # Specific responsibility
```

### ❌ Poor Queries (Too Vague)

```bash
knot search "user"                    # Too generic, will return everything user-related
knot search "authentication"          # Too broad
knot search "get"                     # Way too vague
```

### ❌ Poor Queries (Too Specific/Exact Names)

```bash
knot search "UserAuthenticationService"   # Use semantic search, not exact names
knot search "authenticate"                # Single word too vague for semantic search
```

## Workflow: Feature Discovery Pattern

### Step 1: Initial Semantic Search
```bash
knot search "user login flow" --max-results 10
```

### Step 2: Review Results
Look for files and functions related to login. Note the file paths and entity names.

### Step 3: Explore Identified Files
Once you find promising results, explore their structure:
```bash
knot explore "src/auth/login.ts" --repo my-app
```

### Step 4: Find Related Code (Optional)
If you identified a key entity, find who uses it:
```bash
knot callers "loginUser" --repo my-app
```

## Performance Notes

- **Speed:** Fast (vector similarity in Qdrant) — typical response < 1 second
- **Accuracy:** Depends on query clarity; semantic searches work best with natural language
- **Large Codebases:** Use `--max-results 20` to see more options; use `--repo` to narrow scope

## Examples by Language

### Java Backend Service

```bash
# Find JWT validation logic
knot search "JWT token validation" --repo backend

# Find database migration strategy
knot search "database schema migration" --repo backend

# Find dependency injection container
knot search "service provider container" --repo backend
```

### TypeScript/Node.js API

```bash
# Find Express route handlers
knot search "HTTP route handler" --repo api

# Find request validation middleware
knot search "validate incoming request" --repo api

# Find async error handling
knot search "handle async errors" --repo api
```

### Kotlin Android App

```bash
# Find view model lifecycle
knot search "Android ViewModel lifecycle" --repo android

# Find Room database queries
knot search "SQL database query" --repo android

# Find dependency injection setup
knot search "Hilt dependency injection" --repo android
```

## Troubleshooting

### "No matching code found for your query"

**Cause:** Query is too specific or doesn't match code terminology

**Solutions:**
- Try broader semantic query: "authentication" instead of "OAuth2 JWT bearer token validation"
- Use simpler language: "error handling" instead of "exception management strategy"
- Ensure index is current: `knot-indexer index <repo-path>`
- Try different keywords: "login" instead of "authentication"

### "Too many results (1000+)"

**Cause:** Query is too generic

**Solutions:**
- Be more specific: "user login validation" instead of "user"
- Add `--max-results 5` to focus on top matches
- Combine with `knot explore` to narrow down to specific files

### Results don't match my codebase

**Cause:** Index may be stale or repository name is incorrect

**Solutions:**
- Re-index: `knot-indexer index <repo-path>`
- Verify repository name: Use `--repo my-actual-repo-name`
- Check connection: Ensure `QDRANT_URL` and `NEO4J_URI` are correct
