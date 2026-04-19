# 🦀 Rust Integration Plan for Knot (v0.8.x)

## 1. Overview & Objectives

This document outlines the detailed plan to implement comprehensive Rust support in the `knot` codebase indexer. The goal is to provide semantic search, reverse dependency lookup (`find_callers`), and file exploration (`explore_file`) for Rust projects, adhering strictly to **Rust 2024 Edition** idioms.

### Key Challenges & Requirements
- **Complex Type System:** Handling `impl` blocks (inherent and trait implementations), generic bounds, and lifetimes.
- **Macros:** Rust heavily relies on macros (declarative `macro_rules!` and procedural). We must index macro definitions and macro invocations (e.g., `println!()`, `vec![]`).
- **Rich Documentation:** Rust has distinct documentation styles (outer `///` vs inner `//!`), which must be properly extracted and attached to the correct entities.
- **Attributes:** Capturing procedural macros and attributes (e.g., `#[derive(...)]`, `#[tokio::main]`).
- **Module System:** Inline modules and re-exports (`mod`, `use`, `pub use`) create a nested namespace that must be flattened into FQNs.

---

## 2. Phase 1: Foundation & Grammar Setup

**Version Target:** v0.8.2  
**Objective:** Set up the parser, update core domain models, and prepare the testing environment.  
**Deliverables:** Compiler passes, `.rs` files recognized, Rust entities defined in domain model.

### 2.1. Tree-sitter Dependency Injection

**File:** `Cargo.toml`

Add the latest tree-sitter Rust parser:
```toml
tree-sitter-rust = "0.22"  # Latest as of Rust 2024 Edition
```

**Rationale:** The `0.22` version (or latest) includes support for:
- `async` / `await` / `?.` operator
- `impl Trait` in various positions
- `for<'a>` higher-ranked trait bounds
- `const` generics and const trait functions
- `gen` blocks (stabilized in Rust 1.75+)

### 2.2. Domain Model Extension

**File:** `src/domain/entity.rs`

Extend the `EntityType` enum (or create a new `RustEntityKind` enum if using Rust-specific variants):

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum RustEntityKind {
    /// Struct definition (`struct Foo { ... }`)
    Struct,
    
    /// Enum definition (`enum Bar { ... }`)
    Enum,
    
    /// Union definition (`union Baz { ... }`)
    Union,
    
    /// Trait definition (`trait MyTrait { ... }`)
    Trait,
    
    /// Trait or inherent implementation (`impl Foo for Bar { ... }`)
    /// Store reference to target type and trait (if any)
    Impl,
    
    /// Top-level function (`fn foo() { ... }`)
    Function,
    
    /// Method inside an `impl` block (extracted context)
    Method,
    
    /// Macro definition (`macro_rules! my_macro { ... }`)
    MacroDef,
    
    /// Type alias (`type MyAlias = SomeType;`)
    TypeAlias,
    
    /// Constant (`const FOO: i32 = 42;`)
    Constant,
    
    /// Static variable (`static BAR: &str = "hello";`)
    Static,
    
    /// Inline or file module (`mod foo { ... }` or `mod foo;`)
    Module,
    
    /// Macro invocation (used for tracking calls to macros)
    MacroInvocation,
}
```

**Additional Metadata Fields** in the `Entity` struct:
```rust
pub struct Entity {
    // ... existing fields ...
    
    /// For Rust: list of attributes/decorators
    /// e.g., vec!["derive(Clone)", "tokio::main"]
    pub rust_attributes: Option<Vec<String>>,
    
    /// For `impl` blocks: the trait being implemented (if any)
    /// e.g., Some("Debug") for `impl Debug for MyStruct`
    pub impl_trait: Option<String>,
    
    /// For `impl` blocks: the target type being implemented for
    /// e.g., "MyStruct" for `impl MyStruct { ... }`
    pub impl_target: Option<String>,
    
    /// For generic types: the full generic signature
    /// e.g., "<T: Clone, U: Default>" or "<'a, T>"
    pub generics: Option<String>,
    
    /// Lifetime parameters (Rust-specific)
    /// e.g., vec!["'a", "'b"]
    pub lifetimes: Option<Vec<String>>,
}
```

### 2.3. File Extension Registration

**File:** `src/pipeline/parser/mod.rs` (or equivalent language config)

Register Rust as a supported language:
```rust
// In the language registration logic:
("rs", Language::Rust)  // or ("rs", "rust")
```

### 2.4. Testing Infrastructure

**File:** `tests/testing_files/sample.rs`

Create a comprehensive Rust test file covering:
- Structs with and without fields
- Enums with variants
- Traits with associated types
- `impl` blocks (both inherent and trait implementations)
- Functions with generics and lifetimes
- Macros (both definitions and invocations)
- Attributes and derive macros
- Modules and nested modules
- Documentation comments (outer `///` and inner `//!`)

---

## 3. Phase 2: AST Extraction & Tree-sitter Queries

**Version Target:** v0.8.2  
**Objective:** Write precise Tree-sitter queries to extract entities, signatures, and context.  
**Deliverables:** `queries/rust.scm` file with all entity extraction patterns.

### 3.1. Tree-sitter Query File (`queries/rust.scm`)

**Critical Nodes to Capture:**

```scm
;; ==== STRUCTS ====
(struct_item
  name: (type_identifier) @name
  (#set! "kind" "RustStruct")
  (#set! "fqn_prefix" "struct"))

;; ==== ENUMS ====
(enum_item
  name: (type_identifier) @name
  (#set! "kind" "RustEnum")
  (#set! "fqn_prefix" "enum"))

;; ==== UNIONS ====
(union_item
  name: (type_identifier) @name
  (#set! "kind" "RustUnion")
  (#set! "fqn_prefix" "union"))

;; ==== TRAITS ====
(trait_item
  name: (type_identifier) @name
  (#set! "kind" "RustTrait")
  (#set! "fqn_prefix" "trait"))

;; ==== IMPL BLOCKS (Inherent) ====
(impl_item
  type: (generic_type
    type: (type_identifier) @impl_target)
  (#set! "kind" "RustImpl")
  (#set! "impl_trait" "none"))

;; ==== IMPL BLOCKS (Trait) ====
(impl_item
  trait: (generic_type
    type: (type_identifier) @impl_trait)
  type: (generic_type
    type: (type_identifier) @impl_target)
  (#set! "kind" "RustImpl")
  (#set! "impl_trait" @impl_trait))

;; ==== FUNCTIONS (Top-level) ====
(function_item
  name: (identifier) @name
  (#set! "kind" "RustFunction")
  (#set! "fqn_prefix" "fn"))

;; ==== METHODS (inside impl) ====
;; Note: Methods are function_items inside impl_item context
(impl_item
  body: (declaration_list
    (function_item
      name: (identifier) @method_name
      (#set! "kind" "RustMethod"))))

;; ==== MACRO DEFINITIONS ====
(macro_definition
  name: (identifier) @name
  (#set! "kind" "RustMacroDef")
  (#set! "fqn_prefix" "macro"))

;; ==== MACRO INVOCATIONS ====
(macro_invocation
  macro: (identifier) @macro_name
  (#set! "kind" "MacroInvocation"))

;; ==== TYPE ALIASES ====
(type_alias_item
  name: (type_identifier) @name
  (#set! "kind" "RustTypeAlias"))

;; ==== CONSTANTS ====
(const_item
  name: (identifier) @name
  (#set! "kind" "RustConstant"))

;; ==== STATICS ====
(static_item
  name: (identifier) @name
  (#set! "kind" "RustStatic"))

;; ==== MODULES ====
(mod_item
  name: (identifier) @name
  (#set! "kind" "RustModule"))
```

### 3.2. Signature Extraction

For each entity, capture:
- **Generic Parameters:** `<T, U: Clone>`
- **Lifetime Parameters:** `<'a, 'b: 'a>`
- **Where Clauses:** `where T: Iterator + Clone`
- **Attributes:** `#[derive(Clone)]`, `#[tokio::main]`, etc.

**Logic Pseudocode:**
```
For each entity:
  1. Extract name
  2. If generic_type exists, capture full `<...>` portion
  3. If where_clause exists, capture full `where ...` portion
  4. Collect all preceding attributes and store
  5. Build final signature = "name<generics> where clauses"
```

---

## 4. Phase 3: Comment & Documentation Parsing

**Version Target:** v0.8.3  
**Objective:** Accurately extract and categorize all Rust documentation.  
**Deliverables:** Comments properly attached to entities; semantic vectors include full context.

### 4.1. Comment Types in Rust

1. **Outer Doc Comments** (`///` or `/** ... */`):
   - Precede an item
   - Describe the item itself
   - Example: `/// Adds two numbers together`

2. **Inner Doc Comments** (`//!` or `/*! ... */`):
   - Inside a scope (module or crate root)
   - Describe the enclosing context
   - Example: `//! This module provides utilities for string manipulation`

3. **Standard Comments** (`//` or `/* ... */`):
   - Inline or block comments
   - May appear within function bodies
   - Used for implementation notes

### 4.2. Extraction Strategy

**File:** `src/pipeline/parser/extractor.rs` (Rust-specific section)

```rust
/// Extract documentation comments attached to a node
fn extract_rust_docs(node: Node, source: &str) -> Option<String> {
    let mut docs = Vec::new();
    
    // 1. Look for preceding outer doc comments (lines before the node)
    let start_line = node.start_position().row;
    for line_idx in (0..start_line).rev() {
        let line = get_line(source, line_idx);
        if line.trim().starts_with("///") {
            docs.push(line.trim().strip_prefix("///").unwrap_or("").trim());
        } else if line.trim().starts_with("//!") {
            // Inner doc comment found; treat differently
            break;
        } else if !line.trim().is_empty() {
            break;
        }
    }
    
    // 2. Collect inner doc comments at module level
    if node.kind() == "mod_item" || is_crate_root(node) {
        // Look for `//!` or `/*! ... */` at the start of the scope
    }
    
    docs.reverse(); // Restore order (top to bottom)
    if docs.is_empty() {
        None
    } else {
        Some(docs.join("\n"))
    }
}

/// Extract attributes from a Rust entity
fn extract_rust_attributes(node: Node, source: &str) -> Vec<String> {
    let mut attrs = Vec::new();
    
    // Find all `attribute_item` nodes preceding the entity
    for child in node.parent().unwrap().children(&mut tree_sitter::TreeCursor::new(node)) {
        if child.kind() == "attribute_item" && child.end_byte() < node.start_byte() {
            let attr_text = source[child.byte_range()].to_string();
            // Parse `#[derive(...)]` → `"derive(...)"`
            if let Some(inner) = attr_text.strip_prefix("#[").and_then(|s| s.strip_suffix("]")) {
                attrs.push(inner.to_string());
            }
        }
    }
    
    attrs
}
```

### 4.3. Macro Invocation Comment Tracking

When a macro is invoked, extract inline comments that follow or precede it:
```rust
// Send data over the network
send_data!(buffer);  // Blocks until ACK
```

Capture both comments and attach them to the `MacroInvocation` entity for semantic enrichment.

---

## 5. Phase 4: Graph Relationships & Neo4j Integration

**Version Target:** v0.8.4  
**Objective:** Build the complete call graph and dependency relationships.  
**Deliverables:** Neo4j nodes and relationships correctly model Rust semantics.

### 5.1. New Relationship Types

| Relationship | From | To | Semantics |
|--------------|------|-----|-----------|
| `IMPLEMENTS` | RustStruct/Enum | RustTrait | Struct implements trait |
| `CALLS` | RustFunction/Method | RustFunction/Method | Function calls another |
| `MACRO_CALLS` | * | RustMacroDef | Code invokes a macro |
| `CONTAINS` | RustModule/RustImpl | RustFunction/Method/RustStruct | Parent-child containment |
| `REFERENCES` | RustFunction/Method | RustStruct/RustTrait/RustEnum | Type usage in signature |
| `GENERIC_BOUND` | RustFunction/RustStruct | RustTrait | Generic type parameter bound (e.g., `T: Clone`) |

### 5.2. FQN Resolution for Rust

Rust's module system requires proper FQN (Fully Qualified Name) construction:

**Example FQNs:**
- `my_crate::utils::math::add` (top-level function in module)
- `my_crate::MyStruct::new` (associated function)
- `my_crate::MyTrait::my_method` (trait method)
- `my_crate::impl_MyStruct_MyTrait` (impl block identifier, if needed)

**Algorithm:**
```
1. Walk the module hierarchy from root to current node
2. Concatenate with `::`
3. Append entity name
4. For methods: append method name after `::`
5. For impl blocks: optionally create synthetic name combining trait + target type
```

### 5.3. Macro Call Tracking

**Handling Declarative Macros:**
```rust
// In code:
my_macro!(arg1, arg2);

// Extract:
- Caller: current function/method
- Callee: "my_macro" (RustMacroDef)
- Relationship: MACRO_CALLS
```

**Handling Procedural Macros (Attributes):**
```rust
#[derive(Clone, Debug)]
struct MyStruct { ... }

// Extract:
- Attributes: vec!["derive(Clone)", "derive(Debug)"]
- Relationship: Synthetic "DERIVES_FROM" or attach to entity metadata
```

### 5.4. Impl Block Handling

Rust's `impl` blocks are critical for understanding the codebase:

```rust
impl MyTrait for MyStruct {
    fn method(&self) -> u32 { ... }
}
```

**Strategy:**
1. Create a synthetic `RustImpl` node representing the implementation.
2. Set `impl_trait: Some("MyTrait")` and `impl_target: "MyStruct"`.
3. Link methods inside the impl as children of the `RustImpl` node.
4. Create `IMPLEMENTS` relationship from `MyStruct` → `MyTrait`.

---

## 6. Phase 5: MCP, CLI & Dual Testing

**Version Target:** v0.8.5  
**Objective:** Ensure all tools consume and expose Rust data correctly.  
**Deliverables:** `search_hybrid_context`, `find_callers`, `explore_file` work seamlessly with Rust entities.

### 6.1. Tool Verification Checklist

**`search_hybrid_context`:**
- [ ] Semantic search returns `RustStruct`, `RustTrait`, etc. correctly
- [ ] Documentation comments are indexed in embeddings
- [ ] Macro invocations are searchable by semantic meaning
- [ ] Generic signatures are preserved in results

**`find_callers`:**
- [ ] Reverse dependency lookup works for methods
- [ ] Trait implementations are discovered (find who implements `Trait`)
- [ ] Macro invocations are linked to definitions
- [ ] Signature-based search handles Rust generics and lifetimes (e.g., `knot callers "process<T: Clone"`)

**`explore_file`:**
- [ ] File hierarchy shows modules, structs, traits, impl blocks in logical order
- [ ] Attributes are visible alongside entity names
- [ ] Generic parameters are shown in signatures
- [ ] Methods are grouped under their impl blocks

### 6.2. E2E Test Suite

**File:** `tests/run_rust_e2e.sh`

Create 20+ comprehensive tests covering:

1. **Basic Entity Extraction** (5 tests):
   - [ ] Struct extraction with and without fields
   - [ ] Enum extraction with variants
   - [ ] Trait extraction
   - [ ] Function extraction
   - [ ] Module extraction

2. **Trait Implementation** (3 tests):
   - [ ] Inherent impl block extraction
   - [ ] Trait impl detection
   - [ ] Find which types implement a specific trait

3. **Method & Function Calls** (4 tests):
   - [ ] Simple method calls
   - [ ] Chained method calls (`a.b().c()`)
   - [ ] Macro invocations (both rules and attributes)
   - [ ] Trait method calls

4. **Documentation & Attributes** (3 tests):
   - [ ] Outer doc comments (`///`)
   - [ ] Inner doc comments (`//!`)
   - [ ] Derive attributes and custom attributes

5. **Generics & Lifetimes** (3 tests):
   - [ ] Generic function signature extraction
   - [ ] Lifetime parameters in signatures
   - [ ] Where clauses

6. **Dual Testing** (MCP & CLI):
   - [ ] All tests run both `knot-mcp` and `knot` CLI
   - [ ] Results match between both tools

---

## 7. Phase Deliverables Summary

| Phase | Version | Deliverables | Tests |
|-------|---------|--------------|-------|
| Phase 1 | v0.8.1 | Domain model, file registration, test file | Compiler pass |
| Phase 2 | v0.8.2 | `queries/rust.scm`, entity extraction | Parser test |
| Phase 3 | v0.8.3 | Comment extraction, attribute capture | Doc parsing test |
| Phase 4 | v0.8.4 | Graph relationships, FQN resolution | Neo4j integration test |
| Phase 5 | v0.8.5 | Tool parity, E2E tests (20+ cases) | Dual testing (MCP & CLI) |

---

## 8. Known Complexities & Mitigation Strategies

### 8.1. Module & Re-export Handling

**Challenge:** Rust allows `pub use` to re-export items from other modules, complicating FQN resolution.

**Mitigation:**
- Track re-exports as `REFERENCES` relationships.
- When performing caller lookup, resolve aliases to actual definitions.
- Store both "canonical" FQN and "public" FQN if they differ.

### 8.2. Macro Expansion

**Challenge:** Fully expanding macros requires the Rust compiler; tree-sitter only gives macro structure.

**Mitigation:**
- Index macro definitions and invocations separately.
- Track macro calls at the syntactic level (not expanded).
- For procedural macros, capture the derive/attribute name; full expansion is not required.

### 8.3. Lifetimes & Higher-Ranked Bounds

**Challenge:** Signatures with lifetimes (e.g., `fn iter<'a>(&'a self)`) must be preserved exactly.

**Mitigation:**
- Store lifetime parameters in the `lifetimes` field.
- Include lifetime bounds in the `where` clause capture.
- Ensure signature-based search handles `'` characters.

---

## 9. Build Agent Instructions

To implement this plan, follow the sequence **strictly**:

1. **Phase 1:** Add dependency, update domain model, register file extension. Ensure compilation succeeds.
2. **Phase 2:** Write `queries/rust.scm` incrementally. Test parsing against `sample.rs` after each entity type.
3. **Phase 3:** Implement comment extraction in `extractor.rs`. Verify with inline tests.
4. **Phase 4:** Implement graph relationship logic. Verify Neo4j nodes are created correctly.
5. **Phase 5:** Build E2E test suite. Run all tests with dual-testing mode (MCP & CLI).

**Do NOT skip any phase.** Each phase builds on the previous. Skipping will result in incomplete or incorrect behavior.

---

## 10. Success Criteria

- [ ] All 290+ existing unit tests pass
- [ ] 20+ Rust E2E tests pass (dual testing: MCP & CLI)
- [ ] Semantic search returns relevant Rust entities
- [ ] Reverse dependency lookup works for traits and methods
- [ ] File exploration shows clean Rust hierarchy
- [ ] Documentation comments are properly indexed
- [ ] Macro definitions and invocations are tracked
- [ ] Attributes (e.g., `#[derive(...)]`) are captured
- [ ] Generic signatures are preserved in all outputs
- [ ] No regressions in existing language support (Java, Kotlin, TypeScript, etc.)
