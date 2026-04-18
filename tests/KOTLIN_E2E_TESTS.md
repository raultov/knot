# Kotlin E2E Integration Tests (v0.7.0)

This document describes the End-to-End (E2E) integration tests for Kotlin support in the knot indexer.

## Overview

The Kotlin E2E tests validate that the knot indexer can:
1. Parse Kotlin source files correctly
2. Extract all Kotlin entity types
3. Index them in both Neo4j (graph) and Qdrant (vector) databases
4. Query them via the MCP server

## Test Files

The tests use the following Kotlin sample file:
- **`testing_files/sample.kt`** - Comprehensive Kotlin sample with various language constructs

### Sample.kt Contents

The sample file includes:
- **Classes**: `UserService`, `UserRepository`, `ConfigManager`
- **Interfaces**: `Repository<T>` (generic interface)
- **Objects**: `DatabaseManager` (singleton)
- **Data Classes**: `User` (with primary constructor properties)
- **Companion Objects**: In `ConfigManager` class
- **Functions**: Top-level functions (`greetUser`, `main`)
- **Extension Functions**: `String.isValidEmail()`
- **Methods**: Various methods in classes
- **Properties**: `val`/`var` declarations with type annotations
- **Annotations**: `@Service`, `@Repository`
- **Type References**: `User`, `Repository`, `String`, `Random`, etc.

## Running the Tests

### Option 1: Kotlin-Specific E2E Tests (Recommended)

For testing only Kotlin features:

```bash
cd /path/to/knot
./tests/run_kotlin_e2e.sh
```

This script:
- Spins up isolated Neo4j and Qdrant instances
- Indexes the Kotlin sample file
- Runs 10 test cases specific to Kotlin
- Validates extraction of all Kotlin entity types
- Cleans up databases after completion

**Expected duration**: 2-3 minutes

### Option 2: Full E2E Tests (All Languages)

For testing all supported languages including Kotlin:

```bash
cd /path/to/knot
./tests/run_e2e.sh
```

This script:
- Runs all previous E2E tests (Java, TypeScript, JavaScript, HTML, CSS)
- Plus the new Kotlin tests
- Validates cross-language linking

**Expected duration**: 3-5 minutes

## Test Cases

### Kotlin-Specific E2E Tests

The `run_kotlin_e2e.sh` script runs the following tests:

| # | Test | Validates |
|---|------|-----------|
| 1 | Explore sample.kt | Kotlin class extraction (UserService, UserRepository) |
| 2 | Find Kotlin interface Repository | Interface declaration parsing |
| 3 | Find Kotlin object DatabaseManager | Object (singleton) extraction |
| 4 | Find Kotlin data class User | Data class with primary constructor properties |
| 5 | Find Kotlin class ConfigManager | Companion object support |
| 6 | Find top-level function greetUser | Top-level function extraction |
| 7 | Find extension function isValidEmail | Extension function on String type |
| 8 | Find Kotlin methods (findById, save) | Method declaration in classes/interfaces |
| 9 | Verify Kotlin annotations | Annotation extraction (@Service, @Repository) |
| 10 | Find Kotlin type references | Type reference resolution (Random, User) |

### Added Tests in Full E2E (run_e2e.sh)

Tests 13-21 in the full E2E script add:

| # | Test | Validates |
|---|------|-----------|
| 13 | Explore sample.kt | Kotlin class extraction |
| 14 | Find Repository | Kotlin interface |
| 15 | Find DatabaseManager | Kotlin object (singleton) |
| 16 | Find User | Kotlin data class |
| 17 | Find ConfigManager | Kotlin companion object |
| 18 | Find greetUser | Top-level function |
| 19 | Find isValidEmail | Extension function |
| 20 | Explore sample.kt for @Service | Annotation extraction |
| 21 | Find findById callers | Method call tracking (informational) |

## Entity Types Tested

The tests validate extraction of all 7 Kotlin entity types defined in v0.7.0:

1. **KotlinClass** - Regular class declarations
2. **KotlinInterface** - Interface declarations (including generics)
3. **KotlinObject** - Object declarations (singletons)
4. **KotlinCompanionObject** - Companion object declarations
5. **KotlinFunction** - Top-level and file-level functions
6. **KotlinMethod** - Methods inside classes and interfaces
7. **KotlinProperty** - Property declarations (val/var)

## Requirements

To run the E2E tests, you need:

- **Docker**: 20.10+ (for running Neo4j and Qdrant)
- **Docker Compose**: 1.29+ (for orchestration)
- **netcat (nc)**: For port availability checks (usually pre-installed)
- **Rust**: 1.70+ with cargo (for building knot)
- **4GB+ RAM**: For running two database containers
- **15GB+ Disk Space**: For container images and data

### Installation

**macOS:**
```bash
brew install docker docker-compose
```

**Ubuntu/Debian:**
```bash
sudo apt-get install docker.io docker-compose
sudo usermod -aG docker $USER  # To run without sudo
```

**Windows:**
```
Install Docker Desktop for Windows
```

## Success Criteria

All tests must pass with output like:

```
========================================
knot Kotlin E2E Integration Test
Phase 5 - Kotlin Support (v0.7.0)
========================================

[1/5] Starting Docker containers...
[2/5] Waiting for services...
[3/5] Indexing Kotlin sample file...
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

Validated Kotlin features (v0.7.0):
  ✓ Kotlin class declarations
  ✓ Kotlin interface declarations
  ✓ Kotlin object declarations (singleton pattern)
  ✓ Kotlin data class declarations
  ✓ Kotlin companion object declarations
  ✓ Kotlin top-level function declarations
  ✓ Kotlin method declarations
  ✓ Kotlin extension function declarations
  ✓ Kotlin property declarations
  ✓ Kotlin annotation extraction
```

## Troubleshooting

### Port Already in Use

If you get "Address already in use" errors:

```bash
# Kill existing containers
docker compose -f tests/docker-compose.e2e.yml down -v

# Or use different ports by modifying docker-compose.e2e.yml
```

### Database Connection Errors

If tests fail with connection errors:

1. Check Docker is running: `docker ps`
2. Wait longer for databases: Modify `TIMEOUT_SECONDS` in the script
3. Check logs: `docker compose -f tests/docker-compose.e2e.yml logs`

### Build Failures

If `cargo build` fails:

```bash
# Clean and rebuild
cargo clean
cargo build --release --bin knot-indexer
```

### Kotlin Parser Issues

If Kotlin entities aren't found:

1. Verify sample.kt exists: `ls tests/testing_files/sample.kt`
2. Check Tree-sitter Kotlin grammar: `grep "tree-sitter-kotlin" Cargo.toml`
3. Review parser logs: Run with `RUST_LOG=debug`

## Performance Benchmarks

Expected timing on typical hardware:

| Step | Duration |
|------|----------|
| Build knot (clean) | 30-60s |
| Spin up databases | 10-15s |
| Index sample.kt | 2-5s |
| Run 10 tests | 3-5s |
| **Total** | **2-3 minutes** |

## Integration with CI/CD

To integrate Kotlin E2E tests in CI/CD:

### GitHub Actions Example

```yaml
- name: Run Kotlin E2E tests
  run: |
    cd /home/runner/work/knot/knot
    ./tests/run_kotlin_e2e.sh
  timeout-minutes: 10
```

### GitLab CI Example

```yaml
kotlin_e2e_tests:
  script:
    - ./tests/run_kotlin_e2e.sh
  timeout: 10 minutes
  services:
    - docker:dind
```

## Test Coverage

The Kotlin E2E tests cover:

- ✅ **Parsing**: All Kotlin AST node types
- ✅ **Extraction**: All 7 entity types
- ✅ **Indexing**: Vector and graph database ingestion
- ✅ **Querying**: MCP server integration
- ✅ **Search**: Hybrid semantic + structural search
- ✅ **Relationships**: Entity-to-entity dependencies

## Known Limitations

1. **Method Call Tracking**: Fine-tuning may be needed for complex Kotlin call patterns
2. **Generic Type Resolution**: May need additional work for deep generic nesting
3. **Lambda Expressions**: Not yet extracted as separate entities
4. **Local Classes**: Extraction may be incomplete

These are areas for future enhancement and don't affect the core Kotlin support.

## Future Enhancements

Planned improvements for Kotlin support:

- [ ] Lambda expression extraction
- [ ] Local class extraction
- [ ] Sealed class special handling
- [ ] Enum class members extraction
- [ ] Visibility modifier tracking
- [ ] Nullability annotation extraction
- [ ] Coroutine context analysis
- [ ] DSL marker annotations

## References

- **Kotlin Tree-Sitter Grammar**: https://github.com/fwcd/tree-sitter-kotlin
- **Knot v0.7.0 Release Notes**: See `CHANGELOG.md`
- **Kotlin Language Specification**: https://kotlinlang.org/spec/
