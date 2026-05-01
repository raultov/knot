# Phase 11: C/C++ Support Implementation Plan

## 1. Architectural Overview & Context
The C/C++ implementation for Knot aims to extract both structural entities and semantic dependencies (Call Graphs, Header Includes, Macro Invocations) to populate our vector (Qdrant) and graph (Neo4j) databases.

Unlike languages like Java or TS, C/C++ introduces several complexities:
- **Separation of Definition and Declaration:** `.h` / `.hpp` vs `.c` / `.cpp`.
- **Namespaces:** `std::`, deeply nested namespaces, and anonymous namespaces.
- **Preprocessor Directives:** `#include`, `#define`, `#ifdef`.
- **Pointers & References:** Complex access patterns (`->`, `.`, `::`).

### 1.1 Dependency Injection
Update `Cargo.toml` to include:
```toml
tree-sitter-c = "0.23"
tree-sitter-cpp = "0.23"
```

## 2. BDD Foundation: E2E Test Suite Formulation
**Goal:** Establish failure cases first. Do NOT write parser logic until `tests/run_cpp_e2e.sh` is written and predictably fails.

**File:** `tests/run_cpp_e2e.sh`
- **Setup:** Spin up Neo4j/Qdrant via docker-compose.
- **Test Scenarios (Inline Source Code):**
  1. **Header Inclusion:** File `main.cpp` `#include "lib.h"`. Ensure `find_callers` or Neo4j queries can link `main.cpp` -> `lib.h`.
  2. **Class & Method Extraction:** A class `MyClass` in namespace `Engine` with a method `start()`. FQN must be `Engine::MyClass::start`.
  3. **Call Graph (Pointers/Refs):** `Engine::MyClass* obj = new Engine::MyClass(); obj->start();`. `find_callers` on `start` must return the calling function.
  4. **Macro Tracking:** `#define MAX_BUF 1024`. Ensure macro usage is tracked as a `ReferenceIntent`.
- **Assertion:** Script executes CLI tool and asserts stdout/Neo4j graph state.

## 3. Data Model Expansion
**File:** `src/models/entity.rs`
Introduce language-specific variants to `EntityKind`:
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

## 4. Tree-Sitter Queries Construction (TDD Phase 1)
Create `queries/c.scm` and `queries/cpp.scm`. Use specific capture names recognized by Knot's engine.

### 4.1 Structural Captures
```scheme
; cpp.scm
(namespace_definition name: (identifier) @cpp_namespace.name)
(class_specifier name: (identifier) @cpp_class.name)
(struct_specifier name: (identifier) @c_struct.name)
(function_definition 
  declarator: (function_declarator declarator: (identifier) @cpp_method.name)
)
```

### 4.2 Preprocessor Captures
```scheme
(preproc_include path: (_) @preproc.include)
(preproc_def name: (identifier) @preproc.macro)
```

## 5. Parser Implementation: FQN & Scope Tracking (TDD Phase 2)
**Files:** `src/pipeline/parser/languages/cpp.rs` & `src/pipeline/parser/languages/c.rs`

### 5.1 FQN Resolution (Critical)
C++ scopes are highly nested. Implement a scope tracker (similar to the brace-counter in Groovy, but ideally using Tree-sitter's AST parent traversal) to construct FQNs.
- Function: `build_cpp_fqn(node: Node, source: &[u8]) -> String`
- Traverses up: if parent is `class_specifier`, prepend `ClassName::`. If parent is `namespace_definition`, prepend `NamespaceName::`.

### 5.2 Unit Tests Strategy
Write tests in `cpp.rs` before implementation:
- `test_cpp_fqn_resolution_nested_namespaces()`
- `test_cpp_extract_class_methods()`
- `test_cpp_macro_definitions()`

## 6. Reference Extraction (Call Graph)
**File:** `src/pipeline/parser/languages/cpp.rs`

Implement `extract_reference_intents_cpp(node, source, intents)`.

### 6.1 Handling Call Expressions
Capture `call_expression`. Extract the receiver carefully, considering:
- **Direct calls:** `foo()` -> Receiver: None, Method: `foo`
- **Object access:** `obj.foo()` -> Receiver: `obj`, Method: `foo`
- **Pointer access:** `ptr->foo()` -> Receiver: `ptr`, Method: `foo`
- **Scope resolution:** `std::vector::size()` -> Receiver: `std::vector`, Method: `size`

### 6.2 Unit Tests Strategy
- `test_cpp_reference_intents_pointers()`
- `test_cpp_reference_intents_namespaces()`

## 7. Extractor Orchestration
**File:** `src/pipeline/parser/extractor.rs`

1. **Register Languages:** Load C and C++ grammars into the Tree-sitter parser instantiation.
2. **Route Captures:** In the main extraction loop, add match arms for `c_*` and `cpp_*` capture names.
3. **Dispatch Ref Extraction:** When a `CppMethod` or `CFunction` is encountered, call `cpp::extract_reference_intents_cpp` on its AST node.

## 8. CI/CD & Final Verification
1. Ensure the E2E script from Step 2 now passes.
2. Update `.github/workflows/ci.yml` to include `./tests/run_cpp_e2e.sh`.
3. Update `docs/specs/multilanguage_roadmap.md` marking Phase 11 as completed.

## 9. Strict Rule: No Commits During Iterations
- Do **not** commit at any point during the development of these phases.
- Iterative TDD and BDD test runs must happen in a dirty working tree.
- Once all E2E and Unit Tests pass and clippy is completely silent, wait for explicit user instruction to commit.