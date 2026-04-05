# JSX/React Component Support Specification (v0.3.0)

## Overview

This specification documents the enhancement to `knot` that enables full AST extraction and indexing of React component invocations through JSX syntax. Previously, `knot` could only track function calls using traditional function invocation syntax (`ChartToolbar()`) and missed React component rendering patterns (`<ChartToolbar />`).

## Problem Statement

React components are invoked using JSX syntax, which creates distinct AST nodes compared to traditional JavaScript/TypeScript function calls:

| Invocation Type | Syntax | AST Node Type | Status |
|---|---|---|---|
| Traditional function call | `ChartToolbar()` | `call_expression` | ✅ Supported (v0.2.6) |
| Constructor call | `new ChartToolbar()` | `new_expression` | ✅ Supported (v0.2.6) |
| JSX self-closing element | `<ChartToolbar />` | `jsx_self_closing_element` | ❌ Missing |
| JSX element with children | `<ChartToolbar>...</ChartToolbar>` | `jsx_opening_element` | ❌ Missing |

This resulted in a critical blind spot: `find_callers [entity_name=ChartToolbar]` would return empty results despite the component being actively used in multiple places via JSX.

## Solution

### Core Changes

#### 1. Enhanced Call Extraction Logic
**Module:** `src/pipeline/parse.rs`
**Function:** `extract_call_intents_typescript()`

The function will be extended to handle JSX nodes in addition to traditional call expressions:

```rust
fn extract_call_intents_typescript(node: Node<'_>, source: &[u8], intents: &mut Vec<CallIntent>) {
    // Existing: call_expression handling
    if node.kind() == "call_expression" {
        // ... existing code ...
    } 
    // Existing: new_expression handling
    else if node.kind() == "new_expression" {
        // ... existing code ...
    }
    // NEW: JSX component invocation handling
    else if node.kind() == "jsx_self_closing_element" || node.kind() == "jsx_opening_element" {
        extract_jsx_component_invocation(node, source, intents);
    }

    // Recursively process children
    let mut child = node.child(0);
    while let Some(c) = child {
        extract_call_intents_typescript(c, source, intents);
        child = c.next_sibling();
    }
}
```

#### 2. New JSX Extraction Helper Function
**Module:** `src/pipeline/parse.rs`
**New Function:** `extract_jsx_component_invocation()`

This dedicated function handles the nuances of JSX component names:

```rust
/// Extract JSX component invocation as a call intent.
///
/// Handles React components rendered via JSX syntax:
/// - `<ChartToolbar />` → CallIntent { method: "ChartToolbar", receiver: None }
/// - `<Sheet.Content />` → CallIntent { method: "Content", receiver: Some("Sheet") }
/// - `<Icons.Search />` → CallIntent { method: "Search", receiver: Some("Icons") }
/// 
/// Native HTML tags (lowercase) are ignored:
/// - `<div />` → skipped
/// - `<span />` → skipped
fn extract_jsx_component_invocation(
    node: Node<'_>,
    source: &[u8],
    intents: &mut Vec<CallIntent>,
) {
    let line = node.start_position().row + 1;

    // Get the name node (can be identifier, member_expression, or namespace_name)
    if let Some(name_node) = node.child_by_field_name("name") {
        let comp_name = node_text(name_node, source);

        // React convention: Components start with uppercase, HTML tags are lowercase
        if comp_name.chars().next().is_some_and(|c| c.is_uppercase()) {
            // Handle namespaced components (e.g., Sheet.Content, Icons.Search)
            if comp_name.contains('.') {
                let mut parts = comp_name.split('.');
                let receiver = parts.next().map(|s| s.to_string());
                // Collect remaining parts as method name (handles deeply nested components)
                let method = parts.collect::<Vec<_>>().join(".");
                
                intents.push(CallIntent { method, receiver, line });
            } else {
                // Simple component name
                intents.push(CallIntent {
                    method: comp_name,
                    receiver: None,
                    line,
                });
            }
        }
        // HTML tags (lowercase first letter) are intentionally skipped
    }
}
```

#### 3. Query Updates
**File:** `queries/typescript.scm`

While the extraction logic in Rust handles JSX, we'll add the query patterns for documentation and future parser refactoring:

```scm
; --- JSX Component Invocation (React/TSX) ---
; Matches: <ChartToolbar />
(jsx_self_closing_element
  name: (_) @call.method)

; Matches: <ChartToolbar>...</ChartToolbar>
(jsx_opening_element
  name: (_) @call.method)
```

### Impact on Data Models

No changes to `src/models.rs` are required. JSX component invocations are stored as `ReferenceIntent::Call` with:
- **method**: Component name (e.g., "ChartToolbar", "Content")
- **receiver**: Namespace if present (e.g., "Sheet" from `<Sheet.Content />`)
- **line**: Line number where JSX is rendered

These are resolved to `RelationshipType::Calls` edges in Neo4j during the ingest stage.

### Behavior Examples

#### Example 1: Simple Component
```tsx
// chart-display.tsx:31
export function ChartDisplay() {
  return <ChartToolbar />;
}
```

**Index Result:**
- Entity: `ChartDisplay` (Function)
- Reference: `ChartToolbar` (method call at line 31)
- Relationship: `ChartDisplay` —CALLS→ `ChartToolbar`

**Query Result:**
```
find_callers [entity_name=ChartToolbar]
Returns:
  CALLS (1)
    - ChartDisplay in chart-display.tsx:31
```

#### Example 2: Namespaced Component
```tsx
// form.tsx:42
export function Form() {
  return <Sheet.Content title="Settings">/</Sheet.Content>;
}
```

**Index Result:**
- Entity: `Form` (Function)
- References:
  - `Sheet` (receiver, TypeReference)
  - `Content` (method call with receiver "Sheet" at line 42)
- Relationships: `Form` —CALLS→ `Sheet` (as receiver), `Form` —CALLS→ `Content`

#### Example 3: HTML Tag (Ignored)
```tsx
// card.tsx:15
export function Card() {
  return <div className="card-container"><h1>Title</h1></div>;
}
```

**Index Result:**
- `<div>` and `<h1>` are NOT indexed (lowercase first letter = HTML tag)
- No entries created for native HTML elements

## Testing Strategy

### Unit Tests
1. Parse a single `.tsx` file with mixed function calls and JSX invocations.
2. Verify that:
   - Traditional calls: `ChartToolbar()` → captured as CALLS
   - JSX calls: `<ChartToolbar />` → captured as CALLS
   - Both resolve to the same target entity

### Integration Tests
Index a real React application (e.g., from the `ui` repository):
1. Run `knot-indexer` on a React monorepo containing `.tsx` files.
2. Execute `find_callers [entity_name=ChartToolbar]` via `knot-mcp`.
3. Verify that results include both:
   - `ChartDisplay.tsx:31` (JSX invocation)
   - Any other files using the component

### Edge Cases
1. **Namespaced components**: `<Icons.Search />` correctly splits into receiver and method.
2. **Deep nesting**: `<some.deeply.nested.Component />` chains properly.
3. **HTML tags**: `<button>`, `<div>`, `<span>` are correctly ignored.
4. **Fragments**: `<>...</>` (React Fragments) don't cause errors.

## Version Update

- **Version Bump**: `0.2.6` → `0.3.0` (minor version increment)
- **Reason**: New feature (JSX indexing) is a significant capability expansion for frontend projects.
- **Breaking Changes**: None. Fully backward compatible.

## Migration Path

Existing `knot` installations can upgrade to `0.3.0` without any database migration:
- New indexed entities will automatically include JSX relationships.
- Existing entities remain unchanged.
- Re-indexing projects will populate the previously missing JSX call relationships.

## Future Enhancements

1. **Props/Attributes Tracking**: Track and index component props passed via JSX attributes (e.g., `onClick={handleClick}`).
2. **JSX Fragment Handling**: Detect React Fragments (`<>...</>`) and properly analyze their contents.
3. **Conditional Rendering**: Track components rendered conditionally (within `&&`, ternary operators).
4. **Dynamic Imports**: Support lazy-loaded components (e.g., `React.lazy()`).

## References

- [Tree-sitter JSX Grammar](https://github.com/tree-sitter/tree-sitter-typescript/blob/main/tsx/src/grammar.json)
- [React Component Invocation Patterns](https://react.dev/learn)
- [TypeScript JSX Syntax](https://www.typescriptlang.org/docs/handbook/jsx.html)
