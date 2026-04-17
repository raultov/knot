# 🗺️ Multi-Language Roadmap for Knot

This document outlines the phased expansion of `knot` from a Java/TypeScript indexer to a comprehensive codebase graph indexer supporting JavaScript, HTML, CSS/SCSS, and eventually Rust. Each phase builds upon the previous, enabling increasingly sophisticated cross-language code understanding.

---

## Overview

**Current State (v0.6.4):**
- ✅ Java support (full AST extraction)
- ✅ TypeScript/TSX/CTS support (modern JavaScript/TypeScript)
- ✅ JavaScript/Node.js support (`.js`, `.mjs`, `.cjs`, `.jsx`)
- ✅ HTML support (`.html`, `.htm` with custom elements, id/class indexing)
- ✅ CSS support (`.css` with class/ID selector extraction)
- ✅ SCSS support (`.scss`, `.sass` with mixins, functions, variables)
- ✅ Typed relationships (CALLS, EXTENDS, IMPLEMENTS, REFERENCES)
- ✅ Dual-database architecture (Qdrant + Neo4j)
- ✅ Three MCP tools (search_hybrid_context, find_callers, explore_file)

**Completed Phases:**
- ✅ **Phase 1**: Native JavaScript (v0.6.1)
- ✅ **Phase 2**: HTML Support (v0.6.3)
- ✅ **Phase 3**: CSS & SCSS Support (v0.6.4)

**Goal:** Extend `knot` to become the standard indexer for hybrid web projects (JS/HTML/CSS) with full cross-language dependency resolution and Rust support in the future.

---

## Phase 4: Hybrid Web Ecosystem (HTML + JS + CSS)

### Objective
Connect the graphs built in Phases 1-3 to enable cross-language dependency analysis. Answer questions like:
- "Where is this CSS class used in the DOM?"
- "What JavaScript modifies this HTML element?"
- "Which stylesheets affect this Web Component?"
- "Which HTML files import this JavaScript file?"

### Key Capabilities

#### 1. DOM Reference Linking
When JavaScript code does:
```javascript
document.getElementById('app')
document.querySelector('#main')
```
Create a link to the HTML element with that `id`.

When JavaScript does:
```javascript
element.classList.add('active')
element.classList.remove('hidden')
element.className = 'new-class'
```
Create a link to the CSS class.

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

#### New Reference Intent Types
Add to `ReferenceIntent` in `src/models/entity.rs`:
```rust
pub enum ReferenceIntent {
    // Existing types...
    DomElementReference { element_id: String, line: usize },
    CssClassUsage { class_name: String, line: usize },
    HtmlFileImport { file_path: String, line: usize },
    CssFileImport { file_path: String, line: usize },
}
```

#### JavaScript Pattern Detection
Update Tree-sitter queries to detect:
```scm
; Detect: document.getElementById('app')
(call_expression
  function: (member_expression
    object: (identifier) @object
    "."
    property: (property_identifier) @method)
  arguments: (arguments
    (string) @dom_id))

; Detect: element.classList.add('class-name')
(call_expression
  function: (member_expression
    object: (member_expression
      object: (identifier) @object
      "."
      property: (property_identifier) @classList)
    "."
    property: (property_identifier) @method)
  arguments: (arguments
    (string) @css_class))

; Detect: element.className = 'class-name'
(assignment_expression
  left: (member_expression
    object: (identifier) @object
    "."
    property: (property_identifier) @className)
  right: (string) @css_class)
```

#### HTML Pattern Detection
Update `queries/html.scm` to capture:
```scm
; Script imports
(element
  tag_name: (tag_name) @tag
  (attribute
    name: (attribute_name) @src_attr
    value: (attribute_value) @src_value))

; Stylesheet imports
(element
  tag_name: (tag_name) @tag
  (attribute
    name: (attribute_name) @href_attr
    value: (attribute_value) @href_value))

; Inline event handlers
(element
  (attribute
    name: (attribute_name) @event_attr
    value: (attribute_value) @handler_fn))
```

#### Neo4j Relationship Types
Create new relationship types in the graph:
```cypher
// Link JS DOM manipulation to HTML elements
MATCH (js_method:Entity {kind: 'Method'})
MATCH (html_id:Entity {kind: 'HtmlId', name: $id_name})
CREATE (js_method)-[:REFERENCES_DOM]->(html_id)

// Link JS CSS class usage to CSS definitions
MATCH (js_method:Entity {kind: 'Method'})
MATCH (css_class:Entity {kind: 'CssClass', name: $class_name})
CREATE (js_method)-[:USES_CSS_CLASS]->(css_class)

// Link HTML to imported JS files
MATCH (html_file:Entity {kind: 'HtmlElement', file: $html_file})
MATCH (js_file:Entity {kind: 'File', file: $js_file})
CREATE (html_file)-[:IMPORTS_SCRIPT]->(js_file)

// Link HTML to imported CSS files
MATCH (html_file:Entity {kind: 'HtmlElement', file: $html_file})
MATCH (css_file:Entity {kind: 'File', file: $css_file})
CREATE (html_file)-[:IMPORTS_STYLESHEET]->(css_file)
```

### Implementation Steps

1. **Update Entity Model** (`src/models/entity.rs`)
   - Add new `ReferenceIntent` variants for cross-language references
   - Ensure backward compatibility with existing code

2. **Enhance Query Patterns** (`queries/*.scm`)
   - Update `javascript.scm` and `typescript.scm` to detect DOM/CSS operations
   - Enhance `html.scm` to capture script/stylesheet imports and inline handlers

3. **Extract Reference Intents** (`src/pipeline/parser/extractor.rs`)
   - Parse captured strings from DOM/CSS patterns
   - Generate `ReferenceIntent` for each detected reference

4. **Resolve Cross-Language References** (`src/db/graph/...`)
   - Implement resolution logic to match intents against indexed entities
   - Create Neo4j relationships for verified cross-language links

5. **E2E Testing**
   - Create interconnected test files (HTML + JS + CSS)
   - Validate that hybrid searches work correctly

### Validation Checklist
- [ ] JavaScript `getElementById()` calls link to HTML IDs
- [ ] JavaScript `classList.add()` calls link to CSS classes
- [ ] JavaScript `className` assignments link to CSS classes
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

#### New Entity Types
```rust
pub enum EntityKind {
    RustStruct,
    RustTrait,
    RustImpl,
    RustMacro,
    RustFunction,
    RustMethod,
}
```

#### Relationship Tracking
- **IMPLEMENTS**: `impl Trait for Type` → Type implements Trait
- **CALLS**: Function/method invocations
- **REFERENCES**: Type usage in signatures
- **USES_MACRO**: When a macro is invoked

#### Special Handling: Trait Implementation
Rust's trait system requires special handling for `impl TraitName for StructName` patterns to properly track which types implement which traits.

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

| Phase | Complexity | Status |
|-------|-----------|--------|
| Phase 1: JavaScript | Low | ✅ Completed (v0.6.1) |
| Phase 2: HTML | Low | ✅ Completed (v0.6.3) |
| Phase 3: CSS/SCSS | Medium | ✅ Completed (v0.6.4) |
| Phase 4: Web Ecosystem | High | ⏳ In Progress (v0.6.5) |
| Phase 5: Rust | High | 📋 Planned |

---

## Breaking Changes & Backward Compatibility

### Phases 1-3 (File Format & Schema)
- ✅ **Backward compatible**: New languages integrate into existing Neo4j/Qdrant structure
- ✅ **No database migration needed**: Add new entity types dynamically
- ✅ **MCP tools unchanged**: Work seamlessly with new entity kinds

### Phase 4 (Cross-Language Relationships)
- ⚠️ **New relationship types**: `REFERENCES_DOM`, `USES_CSS_CLASS`, `IMPORTS_SCRIPT`, `IMPORTS_STYLESHEET`
- ✅ **Optional**: Existing queries continue to work
- ✅ **Gradual rollout**: Enable per-project basis

### Phase 5 (Rust)
- ✅ **Backward compatible**: Rust is an additional language, not a replacement

---

## Contributing & Future Enhancements

### Known Limitations & TODOs
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
