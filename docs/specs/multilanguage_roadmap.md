# 🗺️ Multi-Language Roadmap for Knot

This document outlines the phased expansion of `knot` from a Java/TypeScript indexer to a comprehensive codebase graph indexer supporting JavaScript, HTML, CSS/SCSS, and eventually Rust. Each phase builds upon the previous, enabling increasingly sophisticated cross-language code understanding.

---

## Overview

**Current State (v0.2.6):**
- ✅ Java support (full AST extraction)
- ✅ TypeScript/TSX/CTS support (modern JavaScript/TypeScript)
- ✅ Typed relationships (CALLS, EXTENDS, IMPLEMENTS, REFERENCES)
- ✅ Dual-database architecture (Qdrant + Neo4j)
- ✅ Three MCP tools (search_hybrid_context, find_callers, explore_file)

**Goal:** Extend `knot` to become the standard indexer for hybrid web projects (JS/HTML/CSS) with Rust support in the future.

---

## Phase 1: Native JavaScript (Vanilla & Modules) Support

### Objective
Add support for plain JavaScript files (`.js`, `.mjs`, `.cjs`, `.jsx`) to allow indexing of Node.js applications, browser-based libraries, and JavaScript projects alongside TypeScript.

### Why Phase 1 First?
- Many projects have mixed JS/TS codebases (especially Node.js libraries).
- Tree-sitter has excellent JavaScript support via `tree-sitter-javascript`.
- Minimal architectural changes compared to other languages.
- High user demand for JavaScript/Node.js projects.

### Technical Details

#### Dependencies
Add to `Cargo.toml`:
```toml
tree-sitter-javascript = "0.23"
```

#### File Extensions Supported
- `.js` — Standard JavaScript files
- `.mjs` — ES modules
- `.cjs` — CommonJS modules
- `.jsx` — React/JSX files

#### Code Changes Required

**1. `src/pipeline/input.rs` (Line 13)**
```rust
const SUPPORTED_EXTENSIONS: &[&str] = &["java", "ts", "tsx", "cts", "js", "mjs", "cjs", "jsx"];
```

**2. `src/pipeline/parse.rs` (extract_entities function)**
Add new match arm:
```rust
"js" | "mjs" | "cjs" | "jsx" => {
    let query_src = load_query_source("javascript.scm", DEFAULT_JS_QUERY, parse_cfg);
    let lang: Language = tree_sitter_javascript::LANGUAGE.into();
    extract_entities(
        &source,
        lang,
        &query_src,
        "javascript",
        &file_path,
        &parse_cfg.repo_name,
    )?
}
```

Include the built-in query at the top:
```rust
const DEFAULT_JS_QUERY: &str = include_str!("../../queries/javascript.scm");
```

**3. Create `queries/javascript.scm`**
JavaScript is a subset of TypeScript, but without type annotations. The query file will:
- Capture class declarations (ES6 syntax)
- Capture function declarations (arrow, named, default exports)
- Capture variable declarations (consts, lets, vars)
- Capture call expressions (method calls, function invocations)
- **Omit** type references (`@type.reference`) since plain JS lacks type annotations
- Support JSDoc comment extraction via existing `extract_comments` logic

**Key Queries:**
```scm
; Class declarations
(class_declaration
  name: (identifier) @class.name)

; Function declarations
(function_declaration
  name: (identifier) @function.name
  parameters: (formal_parameters) @signature)

; Arrow functions assigned to variables
(lexical_declaration
  (variable_declarator
    name: (identifier) @function.name
    value: (arrow_function
      parameters: (formal_parameters) @signature)))

; Top-level constants
(lexical_declaration
  (variable_declarator
    name: (identifier) @constant.name))

; Call expressions
(call_expression
  function: (member_expression
    object: (identifier) @call.receiver
    "."
    property: (property_identifier) @call.method))

(call_expression
  function: (identifier) @call.method)
```

#### Entity Extraction
- **Classes**: `class Foo { }` → `EntityKind::Class`
- **Functions**: `function foo() {}`, `const foo = () => {}`, `export default function() {}` → `EntityKind::Function`
- **Constants**: `const MY_VAR = value` → `EntityKind::Constant`
- **Methods**: Methods inside class bodies → `EntityKind::Method`
- **Calls**: Function/method invocations → `ReferenceIntent::Call`

#### Relationship Tracking
- **CALLS**: Method invocations (e.g., `obj.method()`, `asyncFn()`)
- **No EXTENDS/IMPLEMENTS**: Plain JS doesn't have explicit inheritance syntax (uses prototypes or composition)
- **REFERENCES**: Indirect usage tracking (assigned callbacks, passed as arguments)

#### Hybrid Project Support
Projects mixing `.js` and `.ts` files will:
- Be indexed in the same pipeline run
- Share the same Neo4j and Qdrant databases
- Allow cross-language queries (find all references to a JS function from TS code)
- Work seamlessly with existing MCP tools without modification

### Validation Checklist
- [ ] Parse a Node.js project with mixed `.js` and `.ts` files
- [ ] Extract all function declarations, class methods, and constants
- [ ] Track CALLS relationships between JS and TS code
- [ ] Verify JSDoc comments are captured
- [ ] Test `search_hybrid_context` across JS/TS boundary
- [ ] Test `find_callers` finds JS functions called from TS code
- [ ] Zero clippy warnings, full `cargo fmt` compliance

---

## Phase 2: HTML Support

### Objective
Index HTML files to understand document structure, extract embedded scripts/styles, and identify reusable components (Web Components, template definitions).

### Tree-sitter Support
✅ **Confirmed**: `tree-sitter-html` provides full AST support.

### Technical Details

#### Dependencies
Add to `Cargo.toml`:
```toml
tree-sitter-html = "0.23"
```

#### File Extensions Supported
- `.html` — Standard HTML files
- `.htm` — Alternate extension

#### Code Changes Required

**1. Update `src/pipeline/input.rs`**
```rust
const SUPPORTED_EXTENSIONS: &[&str] = &["java", "ts", "tsx", "cts", "js", "mjs", "cjs", "jsx", "html", "htm"];
```

**2. Update `src/pipeline/parse.rs`**
Add match arm for HTML:
```rust
"html" | "htm" => {
    let query_src = load_query_source("html.scm", DEFAULT_HTML_QUERY, parse_cfg);
    let lang: Language = tree_sitter_html::LANGUAGE.into();
    extract_entities(
        &source,
        lang,
        &query_src,
        "html",
        &file_path,
        &parse_cfg.repo_name,
    )?
}
```

**3. Create `queries/html.scm`**

Key entities to extract:
```scm
; Web Components (custom elements)
(element
  tag_name: (tag_name) @html.element
  (attribute
    name: (attribute_name) @attr.name
    value: (attribute_value) @attr.value))

; Elements with id attribute
(element
  (attribute
    name: (attribute_name) @attr.id.name
    value: (attribute_value) @attr.id.value))

; Script blocks (for future cross-language analysis)
(script_element) @html.script

; Style blocks (for future CSS analysis)
(style_element) @html.style

; Comments
(comment) @doc
```

#### New Entity Types
Add to `src/models.rs` (EntityKind enum):
```rust
pub enum EntityKind {
    // ... existing variants
    HtmlElement,      // <custom-element>, <div>, etc.
    HtmlId,          // Element with id attribute
    HtmlClass,       // Element with class attribute
    HtmlTemplate,    // <template> definitions
}
```

#### Relationship Tracking
- **REFERENCES**: When an `id` or `class` is used in an HTML element
- **CONTAINS**: Script/Style blocks within HTML (future refinement for Phase 4)

### Validation Checklist
- [ ] Parse HTML with embedded `<script>` and `<style>` tags
- [ ] Extract Web Components (custom elements)
- [ ] Capture id and class attributes
- [ ] Extract inline comments
- [ ] Verify structure is correctly represented in Neo4j
- [ ] Test `search_hybrid_context` for HTML elements

---

## Phase 3: CSS & SCSS Support

### Objective
Index stylesheets to track selectors, variables, mixins, and functions for cross-language linking with HTML and JavaScript.

### Tree-sitter Support
✅ **Confirmed**: 
- `tree-sitter-css` for CSS
- `tree-sitter-scss` for SCSS/SASS

### Technical Details

#### Dependencies
Add to `Cargo.toml`:
```toml
tree-sitter-css = "0.23"
tree-sitter-scss = "0.24"  # Separate crate for SCSS
```

#### File Extensions Supported
- `.css` — Standard CSS files
- `.scss` — SCSS files
- `.sass` — SASS files (indented syntax)

#### Code Changes Required

**1. Update `src/pipeline/input.rs`**
```rust
const SUPPORTED_EXTENSIONS: &[&str] = &[
    "java", "ts", "tsx", "cts", "js", "mjs", "cjs", "jsx", 
    "html", "htm", "css", "scss", "sass"
];
```

**2. Update `src/pipeline/parse.rs`**
Add match arms for CSS and SCSS:
```rust
"css" => {
    let query_src = load_query_source("css.scm", DEFAULT_CSS_QUERY, parse_cfg);
    extract_entities(
        &source,
        tree_sitter_css::LANGUAGE.into(),
        &query_src,
        "css",
        &file_path,
        &parse_cfg.repo_name,
    )?
}
"scss" | "sass" => {
    let query_src = load_query_source("scss.scm", DEFAULT_SCSS_QUERY, parse_cfg);
    extract_entities(
        &source,
        tree_sitter_scss::LANGUAGE.into(),
        &query_src,
        "scss",
        &file_path,
        &parse_cfg.repo_name,
    )?
}
```

**3. Create `queries/css.scm`**
```scm
; Class selectors
(class_selector
  (class_name) @css.class)

; ID selectors
(id_selector
  (id_name) @css.id)

; CSS Custom Properties (variables)
(property_declaration
  property: (plain_value) @css.variable)

; Keyframe animations
(keyframe_block
  (at_keyword) @css.keyframe)
```

**4. Create `queries/scss.scm`**
```scm
; Mixins
(mixin_statement
  name: (identifier) @scss.mixin)

; Functions
(function_statement
  name: (identifier) @scss.function)

; Variables
(variable_declaration
  name: (variable) @scss.variable)

; Class selectors
(class_selector
  (class_name) @css.class)

; ID selectors
(id_selector
  (id_name) @css.id)
```

#### New Entity Types
Add to `src/models.rs`:
```rust
pub enum EntityKind {
    // ... existing variants
    CssClass,        // .my-class
    CssId,          // #my-id
    CssVariable,    // --my-var (CSS Custom Properties)
    ScssVariable,   // $my-var
    ScssMixin,      // @mixin my-mixin()
    ScssFunction,   // @function my-function()
}
```

#### Relationship Tracking
- **REFERENCES**: When a CSS/SCSS selector is referenced in HTML or JavaScript
- **USES**: When a SCSS mixin or function is imported/used in another SCSS file

### Validation Checklist
- [ ] Parse CSS files with class and ID selectors
- [ ] Extract CSS Custom Properties (variables)
- [ ] Parse SCSS files with mixins and functions
- [ ] Capture SCSS variable definitions
- [ ] Verify all selectors are indexed
- [ ] Test cross-file SCSS imports

---

## Phase 4: Hybrid Web Ecosystem (HTML + JS + CSS)

### Objective
Connect the graphs built in Phases 1-3 to enable cross-language dependency analysis. Answer questions like:
- "Where is this CSS class used in the DOM?"
- "What JavaScript modifies this HTML element?"
- "Which stylesheets affect this Web Component?"

### Key Capabilities

#### 1. DOM Reference Linking
When JavaScript code does:
```javascript
document.getElementById('app')
```
Create a link to the HTML element with `id="app"`.

When JavaScript does:
```javascript
element.classList.add('active')
```
Create a link to `.active` CSS class.

#### 2. HTML-to-JS Linking
When HTML includes:
```html
<script src="main.js"></script>
<button onclick="handleClick()">Click</button>
```
Create relationships to the referenced JavaScript file and function.

#### 3. HTML-to-CSS Linking
When HTML has:
```html
<link rel="stylesheet" href="style.css">
<div class="container"></div>
```
Create relationships to the CSS file and the `.container` class definition.

### Technical Implementation

#### New Reference Types
Add to `ReferenceIntent` in `src/models.rs`:
```rust
pub enum ReferenceIntent {
    // Existing...
    DomElementReference { element_id: String, line: usize },
    CssClassUsage { class_name: String, line: usize },
    HtmlFileImport { file_path: String, line: usize },
    CssFileImport { file_path: String, line: usize },
}
```

#### Cross-Language AST Traversal
Update `extract_reference_intents_typescript()` to detect:
```rust
// Detect: document.getElementById('app')
if method == "getElementById" && receiver.as_ref().map_or(false, |r| r.contains("document")) {
    if let Some(arg_id) = extract_string_literal_from_call() {
        intents.push(ReferenceIntent::DomElementReference {
            element_id: arg_id,
            line,
        });
    }
}

// Detect: element.classList.add('class-name')
if method == "add" && receiver.as_ref().map_or(false, |r| r.contains("classList")) {
    if let Some(class_name) = extract_string_literal_from_call() {
        intents.push(ReferenceIntent::CssClassUsage {
            class_name,
            line,
        });
    }
}
```

#### Ingestion Logic
Update `src/pipeline/ingest.rs` to create Neo4j relationships:
```cypher
// Link JS usage of CSS class to CSS entity
MATCH (js_entity:Entity {repo_name: $repo_name, name: $entity_name})
MATCH (css_class:Entity {repo_name: $repo_name, kind: 'CssClass', name: $class_name})
CREATE (js_entity)-[:USES_CSS_CLASS]->(css_class)
```

### Validation Checklist
- [ ] JavaScript `getElementById()` calls link to HTML IDs
- [ ] JavaScript `classList.add()` calls link to CSS classes
- [ ] HTML `<script>` tags link to JavaScript files
- [ ] HTML `<link rel="stylesheet">` links to CSS files
- [ ] `search_hybrid_context` returns cross-language results
- [ ] `find_callers` on a CSS class shows all JS usages
- [ ] Full integration test with a real single-page application (SPA)

---

## Phase 5: Rust Support (Future)

### Objective
Enable `knot` to index Rust codebases with full semantic understanding of ownership, traits, and macro expansion.

### Tree-sitter Support
✅ **Confirmed**: `tree-sitter-rust` provides comprehensive Rust AST support.

### Technical Details

#### Dependencies
Add to `Cargo.toml`:
```toml
tree-sitter-rust = "0.23"
```

#### File Extensions Supported
- `.rs` — Standard Rust source files
- `.rs.in` — Rust build script templates (optional)

#### Code Changes Required

**1. Update `src/pipeline/input.rs`**
```rust
const SUPPORTED_EXTENSIONS: &[&str] = &[
    "java", "ts", "tsx", "cts", "js", "mjs", "cjs", "jsx", 
    "html", "htm", "css", "scss", "sass", "rs"
];
```

**2. Create `queries/rust.scm`**
```scm
; Struct definitions
(struct_item
  name: (type_identifier) @rust.struct)

; Trait definitions
(trait_item
  name: (type_identifier) @rust.trait)

; Impl blocks
(impl_item
  type: (generic_type
    type: (type_identifier) @rust.impl))

; Trait implementations
(impl_item
  trait: (generic_type
    type: (type_identifier) @rust.impl.trait))

; Macro definitions
(macro_definition
  name: (identifier) @rust.macro)

; Function definitions
(function_item
  name: (identifier) @rust.function)

; Method definitions (in impl blocks)
(function_item
  name: (identifier) @rust.method)
```

#### New Entity Types
Add to `src/models.rs`:
```rust
pub enum EntityKind {
    // ... existing variants
    RustStruct,
    RustTrait,
    RustImpl,
    RustMacro,
    RustFunction,
    RustMethod,
}
```

#### Special Handling: Trait Implementation
Rust's trait system requires special handling because `impl Trait for Type` is split across two entities:
```rust
trait MyTrait {
    fn method(&self);
}

impl MyTrait for MyStruct {
    fn method(&self) { ... }
}
```

**Solution:**
In `src/pipeline/parse.rs`, add special logic to extract `impl` blocks:
```rust
fn extract_trait_impls_rust(impl_node: Node<'_>, source: &[u8], intents: &mut Vec<ReferenceIntent>) {
    // Parse: impl TraitName for StructName
    // Create both IMPLEMENTS and EXTENDS-like relationships
    
    if let Some(trait_node) = impl_node.child_by_field_name("trait") {
        let trait_name = node_text(trait_node, source);
        intents.push(ReferenceIntent::Implements {
            interface: trait_name,
            line: impl_node.start_position().row + 1,
        });
    }
}
```

#### Relationship Tracking
- **IMPLEMENTS**: `impl Trait for Type` → Type implements Trait
- **CALLS**: Function/method invocations
- **REFERENCES**: Type usage in signatures
- **USES_MACRO**: When a macro is invoked

### Why Rust is Phase 5
1. Smaller ecosystem compared to web (JS/HTML/CSS)
2. Requires understanding of ownership semantics (advanced)
3. Lower demand than web ecosystem support
4. Foundation of Phases 1-4 needed first
5. Rust syntax is more complex (macros, lifetimes, generics)

### Validation Checklist
- [ ] Parse a Rust project with structs, traits, and impls
- [ ] Track trait implementations accurately
- [ ] Extract macro invocations as CALLS relationships
- [ ] Handle generics and lifetimes correctly
- [ ] Test `find_callers` on Rust functions
- [ ] Verify cross-crate analysis (if applicable)

---

## Implementation Priority & Timeline

| Phase | Complexity | Timeline | Value | Status |
|-------|-----------|----------|-------|--------|
| Phase 1: JavaScript | Low | 1 week | High | ⏳ Planned |
| Phase 2: HTML | Low | 1 week | Medium | ⏳ Planned |
| Phase 3: CSS/SCSS | Medium | 2 weeks | Medium | ⏳ Planned |
| Phase 4: Web Ecosystem | High | 3-4 weeks | Very High | ⏳ Planned |
| Phase 5: Rust | High | 4-6 weeks | Medium | ⏳ Future |

---

## Breaking Changes & Backward Compatibility

### Phase 1-3 (File Format & Schema)
- ✅ **Backward compatible**: New languages integrate into existing Neo4j/Qdrant structure
- ✅ **No database migration needed**: Add new entity types dynamically
- ✅ **MCP tools unchanged**: Work seamlessly with new entity kinds

### Phase 4 (Cross-Language Relationships)
- ⚠️ **New relationship types**: `USES_CSS_CLASS`, `DOM_ELEMENT_REF`, etc.
- ✅ **Optional**: Existing queries continue to work
- ✅ **Gradual rollout**: Enable per-project basis

### Phase 5 (Rust)
- ✅ **Backward compatible**: Rust is an additional language, not a replacement

---

## Contributing & Future Enhancements

### Known Limitations & TODOs
- **Phase 2**: Script/style block extraction (will be improved in Phase 4)
- **Phase 3**: SCSS compilation (use preprocessor output if needed)
- **Phase 4**: Async DOM operations, event listeners, dynamic class manipulation
- **Phase 5**: Macro expansion tracking, lifetime analysis

### Community Contributions
Contributions in any phase are welcome! Each phase is designed to be modular and independently valuable.

---

## References

- [Tree-sitter Language Support](https://tree-sitter.github.io/tree-sitter/#language-bindings)
- [Tree-sitter JavaScript Grammar](https://github.com/tree-sitter/tree-sitter-javascript)
- [Tree-sitter HTML Grammar](https://github.com/tree-sitter/tree-sitter-html)
- [Tree-sitter CSS Grammar](https://github.com/tree-sitter/tree-sitter-css)
- [Tree-sitter SCSS Grammar](https://github.com/tree-sitter/tree-sitter-scss)
- [Tree-sitter Rust Grammar](https://github.com/tree-sitter/tree-sitter-rust)
