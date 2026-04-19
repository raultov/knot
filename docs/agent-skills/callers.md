# Knot Callers: Reverse Dependency Lookup

**Command:** `knot callers "<EntityName>" [--repo <name>]`

## Purpose

Find all places where a specific entity is used, referenced, extended, or implemented. This answers critical questions like:
- "Who uses this class/method?"
- "What will break if I change this?"
- "Is this code dead?"
- "How many places depend on this?"

## Parameters

- **`<EntityName>`** (required): Name of the entity to find callers for
  - Can be a class name, interface, function, or method name
  - Supports partial names and signature fragments
  - Examples: "AuthService", "handleRequest", "processPayment"

- **`--repo <name>`**: Filter to a specific repository (optional)
  - Defaults to auto-detecting the current directory's repository name
  - Use when working with multiple indexed repositories

## Output Format

Results are grouped by relationship type:

### **Calls** — Function/method invocations
Shows where this entity is directly called or invoked.

### **Extends** — Class inheritance
Shows which classes inherit from this class (for superclasses).

### **Implements** — Interface implementation
Shows which classes implement this interface (for interfaces).

### **References** — Type usage
Shows where this entity is used in annotations, signatures, or type declarations.

### Example Output

```markdown
# References to `UserService`

Found 5 reference(s):

## Calls (2)

- **`getUserById`** (method) at `src/handlers/user.ts:25`
  - Signature: `async getUserById(id: string)`

- **`updateUser`** (method) at `src/handlers/user.ts:45`
  - Signature: `async updateUser(id: string, data: User)`

## Extends (0)

## Implements (2)

- **`AdminService`** (class) at `src/admin/admin-service.ts:10`

- **`CustomerService`** (class) at `src/customer/customer-service.ts:15`

## References (1)

- **`getServiceProvider`** (function) at `src/utils/providers.ts:5`
```

## ⚠️ CRITICAL: Avoiding Noisy Results with Common Method Names

This is the **most important rule** for using `knot callers` effectively.

### The Problem

Methods like `accept`, `process`, `handle`, `get`, `run`, `execute`, `apply`, `find`, `create`, `set`, `parse`, and `transform` exist in nearly every codebase with different purposes. Searching by the bare name returns **thousands of irrelevant results**.

### The Solution: Use Signature Fragments

**Always include the opening parenthesis `(` and at least part of the first parameter type** when searching for common method names.

### ❌ Bad Examples (Bare Names - Produces Noise)

```bash
knot callers "accept"          # Returns EVERY accept() method
knot callers "process"         # Returns EVERY process() method in the codebase
knot callers "handle"          # Returns EVERY handle() method
knot callers "get"             # Returns EVERY get() method (thousands of results)
```

### ✅ Good Examples (With Signature Fragments - Targeted Results)

```bash
# By parameter type
knot callers "accept(List<Document"     # Only the specific accept() you care about
knot callers "accept(Iterator"          # Filter by parameter type
knot callers "findById(String"          # Specific findById variant
knot callers "process(Event"            # Process that takes an Event
knot callers "handle(Request"           # Handle that takes a Request

# With multiple parameter hints
knot callers "transform(List,String"    # Even more specific
knot callers "create(User,boolean"      # Clear which overload you want

# By return type (if known)
knot callers "get()LookupService"       # Get that returns LookupService
```

### Why This Works

The search looks for entities where the signature contains your fragment string. Even a partial match is far more specific than just the method name. A method `accept(List<Document>)` is very different from `accept(Socket)` or `accept(Channel)`, and including the parameter type distinguishes them.

## When to Use Callers

### 1. Impact Analysis Before Refactoring

```bash
# Before changing PaymentProcessor, find all dependents
knot callers "PaymentProcessor" --repo billing-service

# Before modifying validate() method, find all callers
knot callers "validate(Request" --repo api-service
```

### 2. Dead Code Detection

```bash
knot callers "legacyFunction"

# If output shows "No references found" → likely dead code
# Confirm by exploring the file where it's defined
knot explore "src/legacy/old-module.ts"
```

### 3. Understanding Dependency Chains

```bash
knot callers "DatabaseConnection"
# See what depends on it
# Then check each caller with knot explore
knot explore "src/services/user-service.ts"
```

### 4. Finding All Implementations of an Interface

```bash
# Find all classes implementing this interface
knot callers "PaymentProvider" --repo backend
# Look at the "Implements" section
```

### 5. Tracing Through Call Chains

```bash
# Start with entry point
knot callers "handleLoginRequest"

# Then explore each caller
knot explore "src/auth/middleware.ts"

# Then find callers of those
knot callers "processCredentials(User"
```

## Interpreting Results

### High Reference Count (20+)

- **Interpretation:** This is a critical, widely-used entity
- **Action:** Change it very carefully; test extensively
- **Safety:** Refactor incrementally and validate with automated tests

### Medium Reference Count (5-20)

- **Interpretation:** Important entity with moderate coupling
- **Action:** Review all callers before making breaking changes
- **Safety:** Can usually refactor safely if you update all callers

### Low Reference Count (1-4)

- **Interpretation:** Either specialized or newly added
- **Action:** Can refactor with less risk
- **Safety:** Easier to maintain and modify

### Zero References ("No references found")

Three interpretations:

1. **Dead Code:** Entity exists but is never used
   - **How to verify:** Check the file exists with `knot explore`
   - **Action:** Safe to delete (after code review)

2. **Newly Created API:** Just added and not yet used
   - **How to verify:** Check recent git commits
   - **Action:** Expected and normal

3. **Statically Referenced:** Called via reflection or dynamic dispatch
   - **How to verify:** Search for the name as a string in the codebase
   - **Action:** May still be in use despite showing zero references

## Workflow Examples

### Pattern 1: Safe Refactoring

```bash
# 1. Find the class/method to refactor
knot callers "OldAuthService"

# 2. Review all references
# Typically shows 5-10 callers

# 3. Explore each caller to understand usage
knot explore "src/handlers/login.ts"
knot explore "src/controllers/user.ts"

# 4. After refactoring, run callers again to verify no missed updates
knot callers "OldAuthService"  # Should show "No references" if successfully replaced
```

### Pattern 2: Impact Analysis

```bash
# 1. Identify entity to change
knot search "payment processing" --max-results 5

# 2. Find all dependents
knot callers "PaymentProcessor"  # Returns 12 references

# 3. For each reference, understand the impact
knot explore "src/services/billing.ts"
knot explore "src/controllers/transaction.ts"
# ... repeat for each reference

# 4. Create comprehensive test plan based on findings
```

### Pattern 3: Dead Code Detection

```bash
# 1. Find old function
knot callers "deprecatedFunction"

# 2. If "No references found" → likely dead code

# 3. Verify with explore
knot explore "src/legacy/deprecated.ts"

# 4. Check git history to understand why it exists
git log --all --oneline -- src/legacy/deprecated.ts

# 5. Create PR to remove with confidence
```

## Examples by Language

### Java Examples

```bash
# Find all classes implementing UserRepository interface
knot callers "UserRepository" --repo backend

# Find all callers of validateEmail method (with signature)
knot callers "validateEmail(String" --repo backend

# Find uses of custom annotation
knot callers "CacheEvict" --repo backend
```

### TypeScript Examples

```bash
# Find implementations of auth middleware
knot callers "authMiddleware" --repo api

# Find callers of specific service (avoid bare name)
knot callers "fetch(string" --repo api

# Find uses of custom decorator
knot callers "@Transactional" --repo api
```

### Kotlin Examples

```bash
# Find implementations of Serializer interface
knot callers "CustomSerializer" --repo android

# Find callers of extension function
knot callers "toViewModel(User" --repo android

# Find uses of data class
knot callers "UserModel" --repo android
```

## Troubleshooting

### "No references found" but you know it's used

**Cause:** Entity name may not match exactly (case-sensitive), or it's referenced via reflection/strings

**Solutions:**
- Try different capitalization: `userService` vs `UserService`
- Try the full qualified name if applicable
- Search instead: `knot search "uses this entity"`
- Check for string references: `knot search "handleRequest"` (in strings)

### "Too many results (1000+)"

**Cause:** Searching for a very common name without signature fragment

**Solutions:**
- Add signature fragment: `knot callers "accept(List<Document"` instead of `knot callers "accept"`
- Use a more specific entity name if available
- Check if you're searching the right repository: `--repo my-repo`

### Results contain unrelated entities

**Cause:** Partial name match (e.g., searching "User" finds "UserService", "UserModel", "UserRepository", etc.)

**Solutions:**
- Use signature fragments for methods: `knot callers "handleUser(Request"` instead of `knot callers "User"`
- For classes, consider searching semantically instead: `knot search "user data model"`
- Add parameters to disambiguate: `knot callers "process(PaymentRequest"`

### Connection errors

**Cause:** Neo4j database not running or environment variables incorrect

**Solutions:**
- Verify Neo4j is running and accessible
- Check environment variables:
  - `NEO4J_URI` should be `bolt://localhost:7687` (or your server)
  - `NEO4J_USER` and `NEO4J_PASSWORD` must be correct
- Test connectivity: `curl -u neo4j:password bolt://localhost:7687`
