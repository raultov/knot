# E2E Per-Language Split (v1.4.4)

## Context

The previous E2E architecture had a single multi-language script `tests/run_e2e.sh`
that indexed the entire `tests/testing_files/` directory. After the per-suite
fixture refactor (`2ab0cfd refactor(e2e): adopt per-suite fixture directory
architecture`), Kotlin/Python/Rust fixtures were duplicated into per-language
subdirectories (`kotlin/`, `python/`, `rust/`) but the original copies remained
at the root, causing entity name collisions in the shared-DB CI runs (e.g.,
three `UserService` entities competing for the top-5 search slots, breaking
Test 30 which validates that the Java FQN includes its package prefix).

This spec completes the per-suite migration for the remaining languages
covered by `run_e2e.sh`: **TypeScript, Java, JavaScript, and the Web
ecosystem (HTML/JSX/CSS/SCSS)**.

## Goals

1. **Isolation**: Each language's E2E suite has its own fixture subdirectory
   and is indexed in isolation against a unique `repo_name` and Qdrant
   collection.
2. **Determinism**: Searches for entity names that previously collided across
   languages (e.g., `UserService` in Java + Kotlin) now return a single,
   unambiguous result per suite.
3. **Maintainability**: One script per language, easier to navigate and to
   add new tests for a single language without touching unrelated areas.
4. **Preventive hardening**: Add a deterministic tie-breaker to
   `find_entities_by_name_prefix` so that future name collisions (e.g.,
   multiple homonymous methods inside one repo) don't reintroduce CI flakiness.

## Non-goals

- Refactoring the Groovy fixtures (`sample_full.groovy` is still used by
  `run_groovy_e2e.sh` and a unit test in `groovy.rs`; out of scope).
- Touching Rust/Python/Kotlin suites beyond the one Test 24 port to Kotlin.
- Modifying the indexer or production code beyond the single tie-breaker.

---

## Redundancy analysis (resolved before refactor)

### Kotlin tests in `run_e2e.sh` are 100% redundant with `run_kotlin_e2e.sh`

| `run_e2e.sh` test | `run_kotlin_e2e.sh` test | Redundancy |
|---|---|---|
| 13: UserService (explore) | 1 | exact |
| 14: Repository (search)   | 2 | exact |
| 15: DatabaseManager       | 3 | exact |
| 16: User (data class)     | 4 | exact |
| 17: ConfigManager         | 5 | exact |
| 18: greetUser             | 6 | exact |
| 19: isValidEmail          | 7 | exact |
| 20: @Service annotation   | 9 | exact |
| 21: findById callers      | 8 | exact |
| 24: ": Int" signature     | — | **unique → port to Kotlin** |

→ 9 tests deleted; 1 test ported to `run_kotlin_e2e.sh`.

### Rust tests in `run_e2e.sh`: none.

`run_rust_e2e.sh`, `run_rust_reference_resolution_e2e.sh`, and
`run_rust_test_module_e2e.sh` cover all Rust scenarios already.

---

## Test classification of `run_e2e.sh`

After removing the 9 redundant Kotlin tests, the remaining 38 tests of
`run_e2e.sh` are distributed across 4 new suites:

### TypeScript (12 tests) → `run_typescript_e2e.sh`

Fixture dir: `tests/testing_files/typescript/`

| Test | Description |
|---|---|
| 1   | explore `test_typescript.ts` → AppComponent, AnalyticsService |
| 2   | find_callers AppComponent → AppModule (decorator extraction) |
| 25  | EventData parameter type → trackEvent |
| 33  | EXTENDS CacheService → BaseService |
| 34  | IMPLEMENTS CacheService → IStorage |
| 35  | EXTENDS ICache → IStorage (interface inheritance) |
| 36  | processPayload uses IPayload (type reference) |
| 37  | Prefix search "IPa" → IPayload (name boost) |
| 38  | COMPONENT_REGISTRY → Engine (value reference) |
| 40  | callerTs → MyTsTarget (cross-file import alias) |
| 43, 44, 44b | TsImportFoo / TsImportQux / TsImportBar (import-as) |
| 45  | explore `ts_imports_uses.ts` → Imports / Referenced Types section |

Files moved: `test_typescript.ts`, `alias_source_ts.ts`, `alias_target_ts.ts`,
`ts_imports_types.ts`, `ts_imports_uses.ts`.

### Java (10 tests) → `run_java_e2e.sh`

Fixture dir: `tests/testing_files/java/`

| Test | Description |
|---|---|
| 3   | search UserService Java (basic discovery) |
| 22  | ChatMemory.add → ChatMemoryAdvisor.before (FQN field-access) |
| 23  | registerUser(String...) signature-based callers |
| 26  | IMPLEMENTS LoggingHandler → MessageHandler |
| 27  | IMPLEMENTS UserRepository → Repository |
| 28  | EXTENDS AdminUser → User |
| 29  | EXTENDS AuditableRepository → Repository (interface) |
| 30  | FQN `com.example.knot.test.UserService` (package prefix) |
| 31  | Anonymous class implements (handle invocation) |
| 32  | MessageHandler interface searchable |

Files moved: `test_java.java`.

**Note on Test 30**: This was the original CI failure. With Java isolated,
the only `UserService` in the repo is the Java one → no ordering ambiguity →
deterministic green test, no parser changes needed.

### JavaScript (6 tests) → `run_javascript_e2e.sh`

Fixture dir: `tests/testing_files/javascript/`

| Test | Description |
|---|---|
| 39  | callerJs → MyJsTarget (cross-file require) |
| 41  | CycleA_target search (circular require resolution) |
| 42  | callerInB → CycleA_target relationship preserved |
| 46  | JsImportFoo via import |
| 47  | JsImportQux via require destructuring |
| 48  | explore `js_imports_uses.js` → Imports section |

Files moved: `alias_source_js.js`, `alias_target_js.js`, `alias_cycle_a.js`,
`alias_cycle_b.js`, `js_imports_types.js`, `js_imports_uses.js`.

### Web ecosystem (8 tests) → `run_web_e2e.sh`

Per user decision: JSX is grouped with the web ecosystem (HTML/CSS/SCSS).

Fixture dir: `tests/testing_files/web/`

| Test | Description |
|---|---|
| 4   | explore `test_javascript.jsx` → DataService |
| 5   | HTML app-header / dashboard id / navbar class |
| 6   | JSX id/className attributes (chart-toolbar, btn-primary, profile-card) |
| 7   | CSS btn-primary, header-container |
| 8   | SCSS responsive-grid |
| 9   | btn-primary cross-language (HTML + CSS + JS) |
| 10  | app-container HTML id manipulated in JS |
| 11  | dashboard HTML id in JS |
| 12  | toggle-btn cross-language theme switch |

Files moved: `test_angular.html`, `test_styles.css`, `test_styles.scss`,
`spa_app.html`, `spa_app.css`, `spa_app.js`, `test_javascript.jsx`.

---

## Orphan fixtures audit

The `tests/testing_files/` root contained several leftover fixtures from
previous refactors. Grep verification confirmed which are safe to delete:

| File | Used by (other than `run_e2e.sh`)? | Action |
|---|---|---|
| `sample.kt` | None | DELETE |
| `sample.py` | None | DELETE |
| `sample.rs` | None (the `rust_reference_resolution` path match is on a nested `rust_contains_collision/tests/testing_files/sample.rs`, different file) | DELETE |
| `sample_app.properties` | None | DELETE |
| `sample_application.yml` | None | DELETE |
| `sample_build.gradle` | None | DELETE |
| `sample_Cargo.toml` | None | DELETE |
| `sample_config.json` | None | DELETE |
| `sample_package.json` | None | DELETE |
| `sample_pom.xml` | None | DELETE |
| `sample.jenkinsfile` | None | DELETE |
| `sample_full.groovy` | **`run_groovy_e2e.sh:194` + `groovy.rs:904` (`include_str!`)** | KEEP |
| `.knot/` (local cache) | local state only | DELETE locally |

---

## Deterministic tie-breaker (preventive hardening)

`src/db/graph/query.rs::find_entities_by_name_prefix` currently orders by:

```cypher
ORDER BY
  CASE WHEN toLower(m.name) = toLower($prefix) THEN 0 ELSE 1 END,
  size(m.name)
```

When multiple entities have the same `name` (e.g., a Java `UserService` and
a Kotlin `UserService` indexed in the same repo, or two homonymous methods),
ties on both criteria are broken by Neo4j's internal storage order — which
varies between machines, parallelism levels, and insertion sequences.

This caused the CI flakiness for Test 30. Even though the per-suite isolation
fixes that specific case, the same pattern will reappear whenever two
entities legitimately share a name inside one repo (e.g., overloaded methods,
homonymous types in different modules of the same crate).

**Fix**: Add `m.fqn` and `m.uuid` as final tie-breakers. `m.fqn` provides a
semantically meaningful (lexicographic) ordering, and `m.uuid` is always
unique → fully deterministic across machines.

```cypher
ORDER BY
  CASE WHEN toLower(m.name) = toLower($prefix) THEN 0 ELSE 1 END,
  size(m.name),
  m.fqn,
  m.uuid
```

Applied to both branches of the query (with and without `repo_name` filter).

---

## Per-language script template

Each new script follows the existing `run_kotlin_e2e.sh` / `run_rust_e2e.sh`
pattern:

- Shared Neo4j/Qdrant via `docker-compose.e2e.yml` on ports `17687` / `16334`.
- Honors `KNOT_E2E_EXTERNAL_DB` to skip docker management when invoked from
  the fast orchestrator (`run_all_e2e_fast.sh`).
- `KNOT_REPO_PATH` points to its dedicated subdirectory; `--clean` flag is
  passed to the indexer only when running standalone.
- Builds `knot-indexer`, `knot-mcp`, `knot` in release mode.
- Per-suite unique `REPO_NAME`, `QDRANT_COLLECTION`, `E2E_DATA_DIR`:

| Script | `REPO_NAME` | `QDRANT_COLLECTION` | `E2E_DATA_DIR` |
|---|---|---|---|
| `run_typescript_e2e.sh` | `typescript_e2e_test_repo` | `knot_typescript_e2e_test` | `.e2e_typescript_data` |
| `run_java_e2e.sh` | `java_e2e_test_repo` | `knot_java_e2e_test` | `.e2e_java_data` |
| `run_javascript_e2e.sh` | `javascript_e2e_test_repo` | `knot_javascript_e2e_test` | `.e2e_javascript_data` |
| `run_web_e2e.sh` | `web_e2e_test_repo` | `knot_web_e2e_test` | `.e2e_web_data` |

---

## Execution sequence

1. **Save this spec** (`docs/specs/e2e_per_language_split.md`).
2. **Create fixture dirs** and move fixtures via `git mv` (preserves history).
3. **Write 4 new E2E scripts** with `chmod +x`.
4. **Port Test 24** (Kotlin `: Int` signature fragment) into
   `run_kotlin_e2e.sh` as new Test 16.
5. **Apply tie-breaker** to `src/db/graph/query.rs` (both ORDER BY clauses).
6. **Update `run_all_e2e_fast.sh`**: replace `run_e2e.sh` entry with the 4
   new scripts.
7. **Delete obsolete files**: `run_e2e.sh`, `run_all_e2e.sh`, 11 orphan
   root fixtures, local `.knot/`.
8. **Update `AGENTS.md`**: refresh the "Run a single E2E language suite"
   list with the new scripts.
9. **Validate**:
   - `cargo fmt -- --check`
   - `cargo clippy --all-targets -- -D warnings`
   - `cargo test`
   - Local docker run of each new suite individually + full
     `./tests/run_all_e2e_fast.sh`.

---

## Outcome summary

- **+4 E2E scripts**, **+4 fixture directories**, **+1 Kotlin test ported**.
- **-2 scripts** (`run_e2e.sh`, `run_all_e2e.sh`).
- **-11 orphan fixture files** at the testing_files root.
- **1 patch** in `src/db/graph/query.rs` (deterministic ordering).
- **1 update** in `run_all_e2e_fast.sh`.
- **1 update** in `AGENTS.md`.

The originally failing **Test 30 (Java FQN with package prefix)** becomes
deterministically green because the Java suite is now indexed in isolation —
no Kotlin `UserService` to compete for the top-5 search slots.
