# Knot Workflows: Patterns and Best Practices

This guide shows common workflows and patterns for using the knot CLI effectively in real-world scenarios.

## Core Workflow Patterns

### Pattern 1: Feature Discovery

**Goal:** Understand how a feature is implemented in the codebase

**Steps:**

1. **Start with semantic search:**
   ```bash
   knot search "user login flow" --max-results 10
   ```

2. **Review results and identify key files**

3. **Explore identified files to see structure:**
   ```bash
   knot explore "src/auth/login.ts" --repo my-app
   knot explore "src/controllers/auth.controller.ts" --repo my-app
   ```

4. **Find dependencies using callers:**
   ```bash
   knot callers "loginUser" --repo my-app
   ```

5. **Optionally explore each caller file:**
   ```bash
   knot explore "src/middleware/auth.middleware.ts" --repo my-app
   ```

**When to use:** Onboarding to new project, understanding a feature before modifying it, documentation/learning tasks

---

### Pattern 2: Impact Analysis Before Refactoring

**Goal:** Understand what will break if you change an entity, plan refactoring safely

**Steps:**

1. **Find the entity to refactor:**
   ```bash
   knot search "payment processor" --repo billing-service
   ```

2. **Identify the exact class/method from search results**

3. **Find ALL dependencies:**
   ```bash
   knot callers "PaymentProcessor" --repo billing-service
   ```

4. **For each reference, understand context:**
   ```bash
   knot explore "src/services/transaction.ts" --repo billing-service
   knot explore "src/controllers/payment.controller.ts" --repo billing-service
   ```

5. **Create refactoring plan:**
   - Document all places that need updating
   - Plan for backwards compatibility if needed
   - Create test cases for all dependent code

6. **After refactoring, verify completeness:**
   ```bash
   knot callers "PaymentProcessor" --repo billing-service
   # Should show all previous references still updated
   ```

**When to use:** Before major refactorings, when deprecating APIs, when changing critical infrastructure code

---

### Pattern 3: Dead Code Detection

**Goal:** Identify and safely remove unused code

**Steps:**

1. **Find the method/class you suspect is unused:**
   ```bash
   knot callers "legacyFunction"
   ```

2. **Interpret results:**
   - If "No references found" → likely dead code
   - If found references → still in use (keep it or update all callers)

3. **Verify by exploring the file:**
   ```bash
   knot explore "src/legacy/old-module.ts"
   ```

4. **Check git history:**
   ```bash
   git log --all --oneline -- src/legacy/old-module.ts
   ```

5. **Create PR to remove with full confidence**

**When to use:** Code cleanup, reducing technical debt, performance optimization through code shrinking

---

### Pattern 4: Understanding Architecture

**Goal:** Map out the system architecture and understand how components relate

**Steps:**

1. **Search for core architectural patterns:**
   ```bash
   knot search "dependency injection container" --max-results 15 --repo backend
   knot search "event bus" --max-results 10 --repo backend
   knot search "database layer" --max-results 10 --repo backend
   ```

2. **For each key component, find who uses it:**
   ```bash
   knot callers "ServiceContainer" --repo backend
   knot callers "EventEmitter" --repo backend
   knot callers "DatabaseConnection" --repo backend
   ```

3. **Explore critical files:**
   ```bash
   knot explore "src/core/container.ts"
   knot explore "src/core/events.ts"
   knot explore "src/core/database.ts"
   ```

4. **Visualize relationships**

**When to use:** Architecture documentation, onboarding technical leads, system redesign, identifying architectural debt

---

### Pattern 5: Cross-Language Analysis

**Goal:** Track how code flows across multiple languages (Java backend → TypeScript API, etc.)

**Steps:**

1. **Find the backend service:**
   ```bash
   knot search "user repository" --repo backend
   ```

2. **Identify the class/method from results**

3. **Find TypeScript API endpoints calling it:**
   ```bash
   knot callers "UserRepository" --repo backend
   ```

4. **Check which endpoints reference it:**
   ```bash
   knot search "user API endpoint" --repo frontend-api
   ```

5. **Trace through the full flow:**
   - Identify entry points in TypeScript
   - Find backend calls
   - Track data transformations

**When to use:** Microservices debugging, cross-service refactoring, understanding API contracts

---

## Advanced Workflows

### A/B Testing Different Approaches

**Scenario:** You found multiple implementations of similar functionality. Which one should you use?

```bash
# Find all caching implementations
knot search "cache implementation" --max-results 20

# For each one, see how widely it's used
knot callers "CacheProviderA"
knot callers "CacheProviderB"
knot callers "CacheProviderC"

# High reference count = widely trusted/tested
# Low reference count = newer or experimental

# Explore the most-used one
knot explore "src/cache/trusted-provider.ts"
```

### Dependency Injection Container Discovery

**Scenario:** You need to register a new service. Where does it go?

```bash
# Find the DI container
knot search "dependency injection" --repo backend

# Find all services registered
knot callers "Container"

# Explore the container file
knot explore "src/di-container.ts"

# Check how other services are registered
knot explore "src/services/database.service.ts"
```

### Plugin System Discovery

**Scenario:** You need to add a new plugin. How does the plugin system work?

```bash
# Find the plugin manager
knot search "plugin manager" --max-results 5

# Find all registered plugins
knot callers "PluginRegistry"

# Explore a reference implementation
knot explore "src/plugins/auth-plugin.ts"

# Check the plugin interface
knot explore "src/core/plugin-interface.ts"
```

### Configuration Propagation

**Scenario:** Track how configuration flows through the system

```bash
# Find configuration loader
knot search "configuration" --max-results 5

# Find who loads config
knot callers "ConfigLoader"

# Find who uses the config
knot search "environment config" --max-results 10

# Explore config usage
knot explore "src/config/app.config.ts"
```

---

## Tips for Effective Searching

### 1. Semantic Search First, Then Drill Down

**Good workflow:**
```bash
# Start broad
knot search "authentication"

# Then narrow down
knot explore "src/auth/login.ts"
knot callers "authenticateUser"
```

**Avoid:**
```bash
# Don't start too specific
knot search "JWT bearer token validation with bcrypt"
```

### 2. Use Repository Filtering in Multi-Repo Setups

```bash
# Instead of searching everything
knot search "user service"

# Search specific repo
knot search "user service" --repo backend
knot search "user service" --repo frontend-api
```

### 3. Combine Commands for Deep Context

The power of knot comes from combining commands:

```bash
# Search → Explore → Callers → Explore again
knot search "payment processor"           # Find it
knot explore "src/payment.ts"            # See structure
knot callers "PaymentProcessor"          # Find dependencies
knot explore "src/invoice/invoice.ts"    # Understand usage
```

### 4. Use Signature Fragments for Common Methods

Remember from the callers guide:

```bash
# ❌ Don't do this for common names
knot callers "handle"

# ✅ Do this instead
knot callers "handle(Request"
knot callers "handle(PaymentEvent"
```

### 5. Max Results Strategy

- **For broad searches:** `--max-results 10` or `--max-results 20` to see options
- **For targeted searches:** `--max-results 5` to reduce noise
- **For dead code detection:** Default is fine, you want to see all references

---

## Interpretation Guide

### Reading Search Results

```markdown
Found 3 entity(entities):

## Functions
- `authenticateUser` (line 42)
  - Signature: async authenticateUser(email: string, password: string): Promise<User>
```

**This tells you:**
- Entity name: `authenticateUser`
- Type: Function
- Location: line 42 of some file
- It's async (returns a Promise)
- Takes email and password as strings
- Returns a User object

### Reading Callers Results

```markdown
## Calls (3)
- **`loginHandler`** (method) at `src/handlers/auth.ts:25`
```

**This tells you:**
- 3 places call this entity
- One caller is the `loginHandler` method
- Located at `src/handlers/auth.ts`, line 25
- Caller is a method (not a function or class)

### Reading Explore Results

```markdown
- **`validateToken`** (line 35)
  - Signature: `validateToken(token: string): boolean`
  - Doc: Checks if JWT token is valid
```

**This tells you:**
- Method name: `validateToken`
- Line: 35 (in the file you're exploring)
- Parameters: takes a string token
- Returns: boolean
- Purpose: JWT validation (from documentation)

---

## Performance Considerations

### Response Times

- **Search:** < 1 second (semantic vector search in Qdrant)
- **Callers:** 1-2 seconds (graph traversal in Neo4j)
- **Explore:** < 100ms (direct file lookup)

### For Large Codebases

1. **Use repository filtering:**
   ```bash
   knot search "payment" --repo billing-service  # Not entire codebase
   ```

2. **Limit results:**
   ```bash
   knot search "handler" --max-results 5  # Not 20
   ```

3. **Be more specific in searches:**
   ```bash
   knot search "payment validation logic"  # Not just "payment"
   ```

4. **For common names, always use signature fragments:**
   ```bash
   knot callers "handle(Request"  # Not callers "handle"
   ```

---

## Troubleshooting Common Issues

### "Search returned nothing, but I know the code exists"

**Solutions:**
1. Try different keywords: "login" vs "authentication"
2. Use simpler language: "database" vs "persistence layer"
3. Check repository name: `--repo my-repo`
4. Re-index: `knot-indexer index <repo-path>`

### "Results are too noisy"

**Solutions:**
1. Use more specific search: "validate user credentials" vs "user"
2. Add signature fragments for methods: `callers "accept(List<Document"`
3. Limit results: `--max-results 5`
4. Use semantic search instead of exact names

### "Can't find what I'm looking for with callers"

**Solutions:**
1. Try semantic search first: `knot search "what am I looking for?"`
2. Check the exact name (case-sensitive): `UserService` vs `userService`
3. Use signature fragments to disambiguate: `callers "process(PaymentRequest"`
4. Try exploring the file directly: `knot explore "path/to/file"`

### Database connection errors

**Solutions:**
1. Check environment variables:
   ```bash
   echo $QDRANT_URL          # Should be http://localhost:6333
   echo $NEO4J_URI           # Should be bolt://localhost:7687
   echo $NEO4J_USER          # Should be neo4j
   ```

2. Verify services are running:
   ```bash
   docker ps | grep qdrant   # Qdrant container
   docker ps | grep neo4j    # Neo4j container
   ```

3. Test connectivity:
   ```bash
   curl http://localhost:6333/health
   curl -u neo4j:password bolt://localhost:7687
   ```

---

## Integration with AI Workflows

### For Code Analysis Tasks (LLM-based)

```
Agent goal: "Analyze the authentication system"

Process:
1. knot search "authentication" --max-results 10
2. For each result: knot explore <file_path>
3. For each key entity: knot callers <entity>
4. Synthesize findings into comprehensive analysis
5. Document architectural decisions and dependencies
```

### For Refactoring Tasks

```
Agent goal: "Refactor UserService for better testability"

Process:
1. knot explore "src/services/UserService.ts"
2. knot callers "UserService"
3. For each caller: knot explore <caller_file>
4. Plan refactoring based on dependency analysis
5. Verify completeness with knot callers again
```

### For Bug Investigation

```
Agent goal: "Find why user login is failing"

Process:
1. knot search "user login" --max-results 10
2. knot explore identified files
3. knot callers on suspicious methods
4. Check error handling: knot search "login error"
5. Trace through the call chain
6. Identify the bug location
```

---

## Command Cheat Sheet

### Discovery
```bash
knot search "what you're looking for"
knot explore "src/path/to/file.ts"
knot callers "EntityName"
```

### With options
```bash
knot search "query" --max-results 20 --repo my-repo
knot explore "file.ts" --repo my-repo
knot callers "Entity(Parameter" --repo my-repo
```

### Typical combinations
```bash
# Find and explore
knot search "thing" && knot explore "result-file.ts"

# Analyze impact
knot callers "Class" && knot explore "caller-file.ts"

# Dead code check
knot callers "suspect_function"  # Check for "No references"
```
