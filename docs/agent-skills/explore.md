# Knot Explore: File Anatomy Discovery

**Command:** `knot explore "<file_path>" [--repo <name>]`

## Purpose

List all classes, methods, functions, and properties in a source file. Quickly understand a file's structure without reading the entire source code.

## Parameters

- **`<file_path>`** (required): Absolute or relative path to source file
  - Examples:
    - `src/main/java/com/app/AuthHandler.java` (Java)
    - `src/services/user.ts` (TypeScript)
    - `src/controllers/payment.kt` (Kotlin)
    - Relative paths work from the repository root

- **`--repo <name>`**: Filter to a specific repository (optional)
  - Defaults to auto-detecting the current directory's repository name
  - Use when working with multiple indexed repositories

## Output Format

Entities are organized by type:
- **Classes and Interfaces**
- **Singleton Objects** (Kotlin)
- **Companion Objects** (Kotlin)
- **Methods and Functions**
- **Properties** (Kotlin)

For each entity, you get:
- **Name**: The entity identifier
- **Line Number**: Where it's defined (for quick navigation)
- **Signature**: Method/function parameters and return type
- **Decorators/Annotations**: Important metadata like `@Component`, `@Override`, etc.
- **Documentation**: First line of the docstring/comment

### Example Output

```markdown
# Entities in `src/handlers/auth.ts`

Found 5 entity/entities:

## Classes

- **`LoginHandler`** (line 10)
  - Signature: `export class LoginHandler`
  - Decorators: @Controller, @UseGuards

## Interfaces

- **`AuthRequest`** (line 25)
  - Signature: `interface AuthRequest`

## Methods

- **`login`** (line 15)
  - Signature: `async login(email: string, password: string)`
  - Doc: Authenticates user with credentials

- **`validateToken`** (line 35)
  - Signature: `validateToken(token: string): boolean`
  - Doc: Checks if JWT token is valid
```

## When to Use Explore

### 1. Quick File Overview

```bash
knot explore "src/services/payment-service.ts"
```

Use when you want to see:
- What classes/interfaces are in the file
- What methods are available
- Quick method signatures without reading full code

### 2. Navigating New Codebases

When joining a project, use explore to quickly map out key files:

```bash
knot explore "src/config/app-config.ts"
knot explore "src/services/user-service.ts"
knot explore "src/controllers/api-controller.ts"
```

### 3. Before Deep Code Reading

Before diving into a complex file:

```bash
knot explore "src/utils/complex-algorithm.ts"
```

Get the structure first, then read specific methods that matter to your task.

### 4. Understanding Class Hierarchies

For classes with inheritance:

```bash
knot explore "src/models/BaseEntity.java"
knot explore "src/models/User.java"  # See if it extends BaseEntity
```

The signatures show inheritance relationships.

### 5. Finding Available Methods

When you need to know what methods a class exposes:

```bash
knot explore "src/services/user-service.ts"
# See all public methods and their signatures
```

## Output Interpretation

### Line Numbers for Navigation

Every entity includes a line number. Use it to jump directly in your editor:

```bash
knot explore "src/services/user.ts"
# Output: - **`getUser`** (line 42)
# Jump to line 42 in your editor
```

### Signatures Reveal Contracts

A signature tells you:
- What parameters the entity expects
- What it returns
- Whether it's async (for async methods)
- Whether it's generic

```
async validateCredentials(email: string, password: string): Promise<User>
```

This tells you:
- **Async:** Doesn't return immediately
- **Parameters:** Expects email and password strings
- **Return:** A Promise resolving to a User object

### Decorators Show Metadata

Decorators/annotations indicate important traits:

```
@Override         # This overrides a parent method
@Deprecated       # Don't use this anymore
@Component        # Spring/Angular component
@Controller       # HTTP endpoint handler
@UseGuards        # Authentication/authorization applied
@Transactional    # Database transaction scope
@Nullable         # Can return null
@NonNull          # Never returns null
```

### Documentation Lines

The first line of docstring/comment is shown:

```
- **`loginUser`** (line 22)
  - Doc: Authenticates user with email and password using bcrypt
```

This gives you the "why" without reading full documentation.

## Examples by Language

### Java File Exploration

```bash
# Explore a Spring service
knot explore "src/main/java/com/app/services/UserService.java" --repo backend

# Output shows:
# - Class declaration
# - All public/private methods with signatures
# - Annotations like @Service, @Autowired, @Transactional
# - Constructor if present
```

### TypeScript File Exploration

```bash
# Explore an API controller
knot explore "src/controllers/user.controller.ts" --repo api

# Output shows:
# - Class declaration
# - All route handlers (methods)
# - Decorators like @Get, @Post, @UseGuards
# - Return types and signatures
```

### Kotlin File Exploration

```bash
# Explore an Android ViewModel
knot explore "src/viewmodels/UserViewModel.kt" --repo android

# Output shows:
# - Class declaration
# - Properties (with types)
# - Methods
# - Companion object (if exists)
# - Lifecycle annotations
```

## Workflow: File Structure Investigation

### Step 1: Quick Overview

```bash
knot explore "src/auth/login.ts"
```

Get the big picture without reading code.

### Step 2: Find Relevant Method

From the output, identify the method that matters for your task.

### Step 3: Deep Dive

```bash
# Now you know the line number, open your editor
# Jump to line 42 where the method is defined
```

### Step 4: Understand Dependencies (Optional)

If you need to understand what the method uses:

```bash
knot callers "loginUser" --repo my-app
```

Find who calls it and what it might depend on.

## Performance Notes

- **Speed:** Very fast (direct file lookup) — typical response < 100ms
- **Large Files:** Works great even for files with 1000+ lines
- **Coverage:** Shows all public and private entities (varies by language)

## Troubleshooting

### "No entities found in file"

**Cause:** File path is incorrect or file doesn't contain entities (config files, etc.)

**Solutions:**
- Verify file path is correct (relative to repository root)
- Ensure it's a source file (not a config, JSON, or other non-code file)
- Try using absolute path: `knot explore "src/main/java/com/app/Service.java"`
- Check repository name: Use `--repo my-repo` if exploring a different repository

### "File not found" or connection errors

**Cause:** Repository name mismatch or database connectivity issue

**Solutions:**
- Verify repository name: `--repo my-repo`
- Ensure Qdrant and Neo4j are running
- Check environment variables (`QDRANT_URL`, `NEO4J_URI`, etc.)
- Re-index the repository: `knot-indexer index <repo-path>`

### Missing decorators or documentation

**Cause:** Index may be out of date or doesn't include comments

**Solutions:**
- Re-index: `knot-indexer index <repo-path>`
- Some language support may not extract all decorators (language-specific)
- Check the original file for documentation format

## Combined Exploration Workflow

### Goal: Understand how a feature is implemented

1. **Search** for the feature:
   ```bash
   knot search "user authentication" --max-results 5
   ```

2. **Explore** the most relevant file:
   ```bash
   knot explore "src/auth/login.ts"
   ```

3. **Find** who uses the key method:
   ```bash
   knot callers "validateCredentials(string" --repo my-app
   ```

4. **Explore** each caller to understand the full chain:
   ```bash
   knot explore "src/controllers/auth.controller.ts"
   ```

This combination gives you complete understanding of a feature's implementation.
