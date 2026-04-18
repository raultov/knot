# Kotlin E2E Tests - Quick Start Guide

## Run Kotlin-Only E2E Tests (Recommended for Quick Validation)

```bash
cd /home/raul/workspace/rust/knot
./tests/run_kotlin_e2e.sh
```

**Duration**: 2-3 minutes
**What it tests**: All 7 Kotlin entity types + MCP server integration

## Run All Language E2E Tests (Full Validation)

```bash
cd /home/raul/workspace/rust/knot
./tests/run_e2e.sh
```

**Duration**: 3-5 minutes
**What it tests**: Java, TypeScript, JavaScript, HTML, CSS, SCSS + Kotlin

## What Gets Tested

### Kotlin-Specific Tests (run_kotlin_e2e.sh)

10 test cases validating:

1. ✅ **Kotlin Classes** - `UserService`, `UserRepository`
2. ✅ **Kotlin Interfaces** - `Repository<T>` (generic)
3. ✅ **Kotlin Objects** - `DatabaseManager` (singleton)
4. ✅ **Kotlin Data Classes** - `User` (with primary constructor)
5. ✅ **Kotlin Companion Objects** - Inside `ConfigManager`
6. ✅ **Top-level Functions** - `greetUser()`, `main()`
7. ✅ **Extension Functions** - `String.isValidEmail()`
8. ✅ **Methods** - `findById()`, `save()`, `connect()`, `disconnect()`
9. ✅ **Annotations** - `@Service`, `@Repository`
10. ✅ **Type References** - `User`, `Repository`, `String`, `Random`

### Entity Types Validated

| Type | Sample |
|------|--------|
| `KotlinClass` | `UserService`, `UserRepository`, `ConfigManager` |
| `KotlinInterface` | `Repository<T>` |
| `KotlinObject` | `DatabaseManager` |
| `KotlinCompanionObject` | Companion object in `ConfigManager` |
| `KotlinFunction` | `greetUser()`, `main()` |
| `KotlinMethod` | `findById()`, `save()`, `connect()` |
| `KotlinProperty` | `val userRepository`, `var users` |

## Sample Test File

The tests use **`tests/testing_files/sample.kt`** which includes:
- 12+ Kotlin classes/interfaces/objects
- Real-world patterns (Service/Repository architecture)
- Annotations (@Service, @Repository)
- Extension functions
- Companion objects
- Data classes
- Type references
- ~100+ lines of representative Kotlin code

## Prerequisites

Make sure you have:
- ✅ Docker installed (`docker --version`)
- ✅ Docker Compose installed (`docker-compose --version`)
- ✅ Rust installed (`cargo --version`)
- ✅ 4GB+ available RAM
- ✅ 15GB+ disk space for Docker images

## Step-by-Step Execution

### 1. Build Release Binary (Optional - tests do this automatically)

```bash
cargo build --release --bin knot-indexer
```

### 2. Run Kotlin E2E Tests

```bash
./tests/run_kotlin_e2e.sh
```

Expected output:
```
========================================
knot Kotlin E2E Integration Test
Phase 5 - Kotlin Support (v0.7.0)
========================================

[1/5] Starting Docker containers...
[2/5] Waiting for services...
✓ Neo4j is ready
✓ Qdrant is ready

[3/5] Indexing Kotlin sample file...
Building knot-indexer...
Running indexer for Kotlin files...
✓ Kotlin file indexed

[4/5] Validating Kotlin entities...

Test 1: Exploring sample.kt...
✓ Found Kotlin class UserService
✓ Found Kotlin class UserRepository

Test 2: Searching for Repository...
✓ Found Kotlin interface Repository

...

========================================
All Kotlin E2E tests passed! ✓
========================================
```

## Troubleshooting

### Issue: "Port already in use"

**Solution**: Kill existing containers
```bash
docker compose -f tests/docker-compose.e2e.yml down -v
```

### Issue: "Docker daemon not running"

**Solution**: Start Docker
```bash
# macOS
open -a Docker

# Linux
sudo systemctl start docker

# Windows
# Start Docker Desktop from Applications
```

### Issue: "Command not found: nc"

**Solution**: Install netcat
```bash
# macOS
brew install netcat

# Ubuntu/Debian
sudo apt-get install netcat-openbsd

# Already installed on most Linux distros
```

### Issue: Tests timeout after 60 seconds

**Solution**: Increase timeout in the script
```bash
# Edit tests/run_kotlin_e2e.sh
# Change: TIMEOUT_SECONDS=60
# To: TIMEOUT_SECONDS=120
```

## What's Happening Behind the Scenes

1. **Docker Setup** (Step 1/5)
   - Spins up isolated Neo4j database (port 17688)
   - Spins up isolated Qdrant database (port 16335)

2. **Wait for Services** (Step 2/5)
   - Checks that databases are ready
   - Gives extra initialization time

3. **Index Kotlin Code** (Step 3/5)
   - Runs `knot-indexer` on `sample.kt`
   - Extracts entities to Neo4j
   - Embeds code to Qdrant

4. **Query via MCP** (Step 4/5)
   - Sends MCP requests to `knot-mcp` server
   - Validates entities are searchable
   - Tests `explore_file` and `search_hybrid_context` tools

5. **Cleanup** (automatic)
   - Removes containers and test data
   - Cleans up Docker volumes

## Integration with CI/CD

### GitHub Actions

```yaml
- name: Run Kotlin E2E Tests
  run: ./tests/run_kotlin_e2e.sh
  timeout-minutes: 10
```

### GitLab CI

```yaml
kotlin_e2e:
  script:
    - ./tests/run_kotlin_e2e.sh
  timeout: 10 minutes
  services:
    - docker:dind
```

### Jenkins

```groovy
stage('Kotlin E2E Tests') {
    steps {
        sh './tests/run_kotlin_e2e.sh'
    }
    options {
        timeout(time: 10, unit: 'MINUTES')
    }
}
```

## Performance Notes

Typical execution times on modern hardware:

| Component | Time |
|-----------|------|
| Build knot | 30-60s |
| Docker setup | 10-15s |
| Index sample.kt | 2-5s |
| Run 10 tests | 3-5s |
| **Total** | **2-3 minutes** |

## Documentation

For detailed information about Kotlin E2E tests, see:
- `tests/KOTLIN_E2E_TESTS.md` - Complete test documentation
- `Cargo.toml` - Version 0.7.0 with tree-sitter-kotlin-ng
- `CHANGELOG.md` - Phase 5 (Kotlin) implementation details

## Success Indicators

✅ All tests pass with green checkmarks
✅ No Docker errors or timeouts
✅ Output shows "All Kotlin E2E tests passed!"
✅ All 7 Kotlin entity types validated
✅ MCP server responds to queries correctly

## Questions?

For issues or questions about Kotlin support:
1. Check `tests/KOTLIN_E2E_TESTS.md` for detailed docs
2. Review `sample.kt` for example Kotlin code
3. Check git commit `fbcc772` for Phase 5 implementation
4. See `src/pipeline/parser/languages/kotlin.rs` for parser details
