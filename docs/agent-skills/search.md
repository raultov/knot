# Knot Search: Semantic Code Discovery

**Command:** `knot search "<query>" [--max-results <N>] [--repo <name>]`

## Purpose

Find code entities by semantic meaning. This is your primary tool for exploratory searches when you don't know exact names or locations.

## Parameters

- **`<query>`** (required): Natural language description of what you're looking for
  - Examples: "user authentication", "error handling", "database connection", "API routes"
  - Good queries describe *what the code does*, not specific names
  - Works best with 2-5 word descriptions

- **`--max-results <N>`**: Limit the number of results (default: 5, max: 100).
  The bound is enforced — larger values are clamped to 100 and a note is
  printed; there is no cursor or pagination. When 100 results are not
  enough, narrow with `--kinds`, `--path` or `--repo`, or refine the query.
  - Use higher values (10-20) when exploring unfamiliar codebases
  - Use lower values (3-5) when you need focused results

- **`--repo <name>`**: Filter to a specific repository (optional)
  - Defaults to auto-detecting the current directory's repository name
  - Use when working with multiple indexed repositories
  - Example: `--repo backend` to search only in the backend repo

- **`--kinds <spec>`**: Optional entity-kind filter (comma-separated)
  - Aliases: `definition` (all functions/methods/types), `callable` /
    `function` / `method`, `class` / `type` / `struct`
  - Exact wire-format kinds also work: `rust_function`, `markdown_section`,
    `kotlin_class`, `csharp_method`, …
  - Example: `--kinds definition` to search only for code definitions

- **`--path <prefix-or-glob>`**: optional path scope (comma-free)
  - Directory prefix matched on a path boundary (`src/api` never matches
    `src/api-notes.md`) or a glob (`src/**/*_test.rs`).
  - Use `knot files` first to discover the layout, then scope the search
    to the relevant subtree.

## Ranking Contract

Results are **kind-aware**. For a natural-language query describing a
behaviour, the entity's own definition (function/method/class/struct) ranks
at or near the top:

- **Definitions (callables and types) outrank markdown docs, test files,
  config properties and build-dependency entities.** A neutral kind
  (`constant`, …) whose node orchestrates ≥ 2 outgoing calls in the graph
  also counts as behavior (TypeScript MCP tools are `export const …`).
- Definitions reach the pool even on documentation-heavy repositories: a
  second scan excludes all non-code kinds (a documentation-scoped search
  via `--kinds markdown_section` skips that code channel).
- The **shared entry point** of the highest-ranked helpers outranks those
  helpers: a paraphrase ranks the helpers of the behaviour it names, and
  graph coverage at search time promotes their common caller above them
  (measured in the call graph one and two hops deep, seeded from the union
  of the semantic and lexical channels).
- An entity merely named after a generic verb/noun (`find`, `get`,
  `create`, `build`, `acquire`, `borrow`, `current`, …) does not win on the
  bare verb; the full name boost requires a second query token in the
  entity's container context (FQN).
- A method can outrank its own container when the query names it
  (`LookupMaps::build` beats `LookupMaps` for "build lookup maps").
- An identifier the query literally names is probed by exact name and can
  surface even when its pure similarity rank is deep (the probe fetches
  definitions and their deep-cosine lexical relatives alike).
- A query whose tokens match an entity's *name* keeps leading slots only
  for definitions; prose and config/build name hits are demoted into the
  candidate pool and ranked on their own cosine — documentation-only
  topics (no competing definition) still surface their best section.
- Callers and helpers appear as **context attached to** a definition
  (caller samples, subclasses, implementers) — never as substitutes that
  displace it.
- Ordering is deterministic (ties break on file path, line, UUID) and
  duplicate rows are removed.
- Documentation-only topics still surface their best markdown section; to
  search docs exclusively pass `--kinds markdown_section`.

## Recall Contract

The embed text of every code entity carries its full identifier surface, so
paraphrases that never name the identifier are still retrievable:

- **Name, FQN and identifier tokens** (`useChangePassword` → `use change
  password`) are part of the embedded text, in raw and tokenized form.
- **Call names**: the names an entity calls or refers to in its body are
  tokenized into the embed (`Calls: normalize email, verify credentials`).
  A function with no doc comment is therefore findable by the behaviour
  described in natural language — its body's callees share the query's
  vocabulary.
- **Token-level probe**: query words that appear as parts of identifiers
  reach entities regardless of the identifier's spelling (`similarity
  search` reaches `similaritySearch` from the tokens alone).
- **Caller bridge**: the callers of the top semantic hits enter the
  candidate pool; the shared caller of several helpers is usually the
  entry point the paraphrase describes (a definition with `caller_roots`
  evidence earns a bounded boost).
- If a definition still does not surface for a paraphrase, the vocabulary
  genuinely is absent from its embed — try naming the responsibility you
  know (`login`, `hook`, endpoint), or use `find_callers` from one of its
  callees.

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
- **Large Codebases:** Use `--max-results 20` (up to the enforced 100) to see more options; use `--repo` to narrow scope

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
