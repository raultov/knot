# Phase 11: C/C++ Support Implementation Plan

## Status: ✅ COMPLETED (v1.0.0)

---

## 1. Architectural Overview & Context
The C/C++ implementation for Knot aims to extract both structural entities and semantic dependencies (Call Graphs, Header Includes, Macro Invocations) to populate our vector (Qdrant) and graph (Neo4j) databases.

Unlike languages like Java or TS, C/C++ introduces several complexities:
- **Separation of Definition and Declaration:** `.h` / `.hpp` vs `.c` / `.cpp`.
- **Namespaces:** `std::`, deeply nested namespaces, and anonymous namespaces.
- **Preprocessor Directives:** `#include`, `#define`, `#ifdef`.
- **Pointers & References:** Complex access patterns (`->`, `.`, `::`).

### 1.1 Dependency Injection
✅ Completed - Added to `Cargo.toml`:
```toml
tree-sitter-c = "0.23"
tree-sitter-cpp = "0.23"
```

## 2. BDD Foundation: E2E Test Suite Formulation
✅ Completed - `tests/run_cpp_e2e.sh` created and passing.

**File:** `tests/run_cpp_e2e.sh`
- **Setup:** Spin up Neo4j/Qdrant via docker-compose.
- **Test Scenarios:**
  1. ✅ **FQN Extraction:** Class `MyClass` in namespace `Engine` with method `start()`. FQN is `Engine::MyClass::start`.
  2. ✅ **Call Graph:** `obj->start()` pointer access. `find_callers` on `start` returns `main`.
  3. ✅ **Macro Tracking:** `#define MAX_BUF 1024`. Macro usage in `int buf[MAX_BUF]` is tracked.
  4. ✅ **Type References:** `Engine::MyClass* obj = new Engine::MyClass()` creates type references to `MyClass`.

## 3. Data Model Expansion
✅ Completed - `src/models/entity.rs`

Introduced language-specific variants to `EntityKind`:
```rust
pub enum EntityKind {
    // ... existing
    CStruct,
    CFunction,
    CppClass,
    CppMethod,
    CppNamespace,
    MacroDefinition,
}
```

Neo4j labels mapped in `src/db/graph/utils.rs`.

## 4. Tree-Sitter Queries Construction (TDD Phase 1)
✅ Completed - Created `queries/c.scm` and `queries/cpp.scm`.

### 4.1 Structural Captures
```scheme
; cpp.scm
(namespace_definition name: (namespace_identifier) @cpp_namespace.name)
(class_specifier name: (type_identifier) @cpp_class.name)
(struct_specifier name: (type_identifier) @c_struct.name)
(function_definition
  declarator: (function_declarator declarator: (identifier) @cpp_method.name)
)
```

### 4.2 Preprocessor Captures
```scheme
(preproc_include path: (_) @preproc.include)
(preproc_def name: (identifier) @preproc.macro)
```

**Note:** `namespace_definition` uses `namespace_identifier` node kind (not `identifier`) in tree-sitter-cpp.

## 5. Parser Implementation: FQN & Scope Tracking (TDD Phase 2)
✅ Completed - `src/pipeline/parser/languages/cpp.rs` (307 lines)

### 5.1 FQN Resolution
Implemented `build_cpp_fqn(node: Node, source: &[u8]) -> Option<String>`:
- Traverses AST parents upward
- On `class_specifier` or `struct_specifier`: prepends `ClassName::`
- On `namespace_definition`: prepends `NamespaceName::`
- Returns fully qualified name like `Engine::MyClass`

### 5.2 Unit Tests
- `test_cpp_fqn_resolution_nested_namespaces()` ✅
- `test_cpp_reference_intents_pointers_and_namespaces()` ✅

## 6. Reference Extraction (Call Graph)
✅ Completed - `extract_reference_intents_cpp()` and `extract_call_intents_cpp()` in `cpp.rs`

### 6.1 Call Expression Handling
- **Direct calls:** `foo()` → Receiver: None, Method: `foo`
- **Object access:** `obj.foo()` → Receiver: `obj`, Method: `foo`
- **Pointer access:** `ptr->foo()` → Receiver: `ptr`, Method: `foo`
- **Scope resolution:** `std::vector::size()` → Receiver: `std::vector`, Method: `size`
- **Field expressions:** `this->compute()` → Receiver: `this`, Method: `compute`

### 6.2 Type Reference Handling
- `type_identifier` in declarations: `Engine::MyClass* obj`
- `type_identifier` in `new` expressions: `new Engine::MyClass()`
- Qualified identifiers handled via parent `qualified_identifier` node

### 6.3 Macro Usage Detection
Heuristic: identifiers that are ALL_UPPERCASE with underscores and digits are likely macro usages.

## 7. Extractor Orchestration
✅ Completed - `src/pipeline/parser/extractor.rs`

1. **Register Languages:** C and C++ grammars loaded into Tree-sitter parser.
2. **Route Captures:** Match arms for `c_*`, `cpp_*`, and `preproc.*` capture names.
3. **Dynamic FQN Construction:** For C/C++ entities, `cpp::build_cpp_fqn()` called after base `compute_fqn_and_context()`.
4. **Entity Node Search:** `cpp_method` entity searches `function_definition`, `declaration`, and `field_declaration` parent nodes.

### 7.1 Integration Points
- `src/pipeline/parser/languages/mod.rs`: Registered `pub mod cpp;`
- `src/pipeline/parser/mod.rs`: Added `DEFAULT_C_QUERY`, `DEFAULT_CPP_QUERY` constants and extension dispatch
- `src/pipeline/parser/orphans.rs`: C/C++ call intent extraction integrated
- `src/pipeline/input.rs`: Added `c`, `h`, `cpp`, `hpp`, `cc`, `cxx`, `hh`, `hxx` to `SUPPORTED_EXTENSIONS`

## 8. CI/CD & Final Verification
✅ Completed

1. ✅ E2E script `tests/run_cpp_e2e.sh` passes (4/4 tests).
2. ✅ `.github/workflows/ci.yml` updated to include `./tests/run_cpp_e2e.sh` and `./tests/run_cross_lang_ref_e2e.sh`.
3. ✅ This document (phase_11_cpp_support_plan.md) updated marking Phase 11 as completed.
4. ✅ `tests/run_all_e2e.sh` created for local validation of all 10 E2E test suites.

## 9. Test Results Summary

### Unit Tests (3 new)
- `test_cpp_fqn_resolution_nested_namespaces` ✅
- `test_cpp_reference_intents_pointers_and_namespaces` ✅
- 441 existing unit tests continue to pass

### E2E Tests (4 C++ tests)
- Test 1: FQN extraction for `Engine::MyClass::start` ✅
- Test 2: Call graph tracking for `start()` → `main()` ✅
- Test 3: Macro `MAX_BUF` usage tracking ✅
- Test 4: Type reference tracking for `MyClass` ✅

### All E2E Suites (10 total)
- ✅ JS/TS/Java E2E
- ✅ Kotlin E2E
- ✅ Rust E2E
- ✅ Python E2E
- ✅ Build Systems E2E (fixed file naming issue)
- ✅ Groovy E2E
- ✅ Groovy Cross-Ref E2E (fixed env vars for indexer)
- ✅ Groovy Private Method E2E
- ✅ Cross-Language Ref E2E
- ✅ C/C++ E2E

### Quality Metrics
- ✅ `cargo fmt`: clean
- ✅ `cargo clippy --all-targets -- -D warnings`: clean
- ✅ `cargo build --release`: successful
- ✅ Published to crates.io as v1.0.0