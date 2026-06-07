//! Type reference collection for Rust source.
//!
//! Walks function signatures, struct/enum fields, struct literals, method
//! call paths, and `use` declarations to produce
//! [`ReferenceIntent::TypeReference`] entries attached to the nearest
//! enclosing entity. Inside macro `token_tree` bodies (where tree-sitter no
//! longer parses real AST nodes) the walker falls back to a `::`-based
//! pattern scan, with an O(N) substring-skip optimisation to keep deeply
//! nested macros like `vec![vec![vec![...]]]` from blowing up.

use super::utils::find_nearest_entity_by_line;
use crate::models::{ParsedEntity, ReferenceIntent};
use crate::pipeline::parser::utils::node_text;
use tree_sitter::Node;

/// Translate the literal `Self` keyword to the enclosing class name when present.
///
/// The reference resolver keys on concrete type names; emitting `Self` would
/// never match a real entity. When no enclosing class is in scope (defensive
/// case for malformed source or top-level usage) we leave the name unchanged
/// and log a debug trace.
fn translate_self_in_type_name(type_name: &str, enclosing_class: Option<&str>) -> String {
    if type_name == "Self" {
        if let Some(class) = enclosing_class {
            return class.to_string();
        }
        tracing::debug!("Self reference without enclosing class context");
    }
    type_name.to_string()
}

/// Collect type references from Rust source code (parameter types, return types, field types).
///
/// This function walks through function_item, struct_item, and enum_item nodes
/// to extract type references from their signatures and fields.
pub fn collect_rust_type_references(
    root: Node<'_>,
    source: &[u8],
    entities: &mut [ParsedEntity],
    _file_path: &str,
    _repo_name: &str,
) {
    let mut type_refs: Vec<(usize, String)> = Vec::new();

    // Start from root, not first child (to process all top-level items)
    collect_type_nodes(&root, source, &mut type_refs, None);

    for (line, type_name) in type_refs {
        let target_idx = find_nearest_entity_by_line(entities, line);
        if target_idx < entities.len() {
            let resolved_name = translate_self_in_type_name(
                &type_name,
                entities[target_idx].enclosing_class.as_deref(),
            );
            entities[target_idx]
                .reference_intents
                .push(ReferenceIntent::TypeReference {
                    type_name: resolved_name,
                    line,
                });
        }
    }
}

/// Recursively collect type references from Rust source.
///
/// Captures type references in:
/// 1. Function parameters: `fn foo(cfg: &Config)` → `Config`
/// 2. Return types: `fn foo() -> Config` → `Config`
/// 3. Struct literals: `Config { field: value }` → `Config`
/// 4. Method calls: `Config::load_mcp()` → `Config`
/// 5. Type annotations: `let cfg: Config` → `Config`
///
/// # Performance Optimization
///
/// Uses O(N) substring skipping for nested `token_tree` nodes. When a `token_tree`
/// node is fully contained within the byte range of a parent `token_tree` that was
/// already processed, we skip redundant string allocation and `::` pattern matching.
/// This eliminates exponential blowup for deeply nested macros like `vec![vec![vec![...]]]`.
pub(crate) fn collect_type_nodes(
    node: &Node<'_>,
    source: &[u8],
    type_refs: &mut Vec<(usize, String)>,
    searched_range: Option<(usize, usize)>,
) {
    // CASE 1: type_identifier in function signatures and type annotations
    if node.kind() == "type_identifier" {
        // Filter out type_identifier in pattern matching contexts (e.g., MyEnum::Variant in match arms)
        // These are not true type references but enum variant paths
        let should_capture = if let Some(parent) = node.parent() {
            // EXCLUDE: scoped_identifier parent in pattern matching (e.g., RelationshipType::Calls in match arm)
            // INCLUDE: scoped_identifier parent in value context (e.g., crate::models::EntityKind::Class as value)
            let parent_kind = parent.kind();
            if parent_kind == "scoped_identifier" {
                // Check if we're in a value context - if so, include it
                // The check will be done in CASE 3 for the scoped_identifier itself
                false // Let CASE 3 handle scoped_identifier
            } else {
                true
            }
        } else {
            true
        };

        if should_capture {
            let line = node.start_position().row + 1;
            let type_name = node_text(*node, source).to_string();
            type_refs.push((line, type_name));
        }
    }

    // CASE 2: struct_expression like Config { field: value }
    // In tree-sitter, struct literals have structure:
    // struct_expression
    //   ├─ (generic_type | type_identifier | identifier) "Config"  ← We want to capture this
    //   └─ field_initializer_list
    if node.kind() == "struct_expression"
        && let Some(first_child) = node.child(0)
        && (first_child.kind() == "generic_type"
            || first_child.kind() == "type_identifier"
            || first_child.kind() == "identifier")
    {
        let line = first_child.start_position().row + 1;
        let type_name = node_text(first_child, source).to_string();
        type_refs.push((line, type_name));
    }

    // CASE 2b: identifier / type_identifier inside use_list (braced imports)
    // e.g., use foo::{Bar, Baz}; — Bar and Baz are identifier nodes inside use_list
    // Also handles use_as_clause path: use foo::{Bar as Renamed}; — emit Bar, not Renamed
    if (node.kind() == "identifier" || node.kind() == "type_identifier")
        && let Some(parent) = node.parent()
    {
        let is_in_use_list = parent.kind() == "use_list";
        let is_use_as_path = parent.kind() == "use_as_clause"
            && parent.child_by_field_name("path").as_ref() == Some(node);

        if is_in_use_list || is_use_as_path {
            let line = node.start_position().row + 1;
            let type_name = node_text(*node, source).to_string();
            type_refs.push((line, type_name));
        }
    }

    // CASE 3: scoped_identifier like Config::load_mcp(), EntityKind::HtmlId, or crate::models::EntityKind::Class
    // These are method calls or variant accesses on types. We want to capture the type name.
    //
    // CAPTURE:
    //   - Config::load_mcp() in call_expression ← CAPTURE Config (method call)
    //   - EntityKind::HtmlId in field initializer ← CAPTURE EntityKind (type usage)
    //
    // EXCLUDE:
    //   - MyEnum::Variant in match pattern ← NOT CAPTURED (enum variant path)
    //   - ImportedType::Variant1 in return ← NOT CAPTURED (enum variant value)
    //
    // Key insight: Only capture if the scoped_identifier is either:
    // 1. A direct child of call_expression (method call on type), OR
    // 2. In a use declaration (import statement)
    if node.kind() == "scoped_identifier" {
        let parent_kind_raw = node.parent().map(|p| p.kind()).unwrap_or("");
        let is_scoped_use_list_path = parent_kind_raw == "scoped_use_list";
        let is_nested_scoped_in_use = parent_kind_raw == "scoped_identifier";

        let in_pattern_match = node
            .parent()
            .map(|p| p.kind() == "match_pattern")
            .unwrap_or(false);

        let in_use = 'check_ancestors: {
            let mut current = node.parent();
            while let Some(n) = current {
                if n.kind() == "use_declaration" || n.kind() == "use_item" {
                    break 'check_ancestors true;
                }
                // Stop at call expression
                if n.kind() == "call_expression" || n.kind() == "field_expression" {
                    break 'check_ancestors false;
                }
                current = n.parent();
            }
            false
        };

        // Check if this is in a field_initializer context (struct literal field value)
        let in_field_initializer = 'check_field: {
            let mut current = node.parent();
            while let Some(n) = current {
                if n.kind() == "field_initializer" {
                    break 'check_field true;
                }
                // If we hit call_expression first, we're not in field_initializer
                if n.kind() == "call_expression" {
                    break 'check_field false;
                }
                current = n.parent();
            }
            false
        };

        // Check if this is in an argument context (function/method call argument)
        // We need to check if we're inside an 'arguments' node, but NOT as the callee
        let in_argument = 'check_arg: {
            let mut current = node.parent();
            while let Some(n) = current {
                // Found argument node - we're in an argument position
                if n.kind() == "argument" || n.kind() == "arguments" {
                    break 'check_arg true;
                }
                // If we hit call_expression, check if we're the function part or in arguments
                if n.kind() == "call_expression" {
                    // Check if node is a descendant of the 'arguments' child
                    if let Some(args_node) = n.child_by_field_name("arguments") {
                        // Check if our node is within the arguments node's range
                        if node.start_byte() >= args_node.start_byte()
                            && node.end_byte() <= args_node.end_byte()
                        {
                            break 'check_arg true;
                        }
                    }
                    // Otherwise we're in the function/callee position
                    break 'check_arg false;
                }
                current = n.parent();
            }
            false
        };

        // Only process if NOT in a pattern matching context
        if !in_pattern_match && !is_scoped_use_list_path && !is_nested_scoped_in_use {
            let Some(first_child) = node.child(0) else {
                // Recurse to children
                let child_searched_range = searched_range;
                let mut child = node.child(0);
                while let Some(c) = child {
                    collect_type_nodes(&c, source, type_refs, child_searched_range);
                    child = c.next_sibling();
                }
                return;
            };

            let first_child_ok = first_child.kind() == "generic_type"
                || first_child.kind() == "identifier"
                || first_child.kind() == "type_identifier"
                || (in_use && first_child.kind() == "scoped_identifier");

            if !first_child_ok {
                let child_searched_range = searched_range;
                let mut child = node.child(0);
                while let Some(c) = child {
                    collect_type_nodes(&c, source, type_refs, child_searched_range);
                    child = c.next_sibling();
                }
                return;
            }

            // Check what kind of context we're in
            let parent_kind = node.parent().map(|p| p.kind()).unwrap_or("");

            // Capture if:
            // 1. Direct child of call_expression (method call like Config::load_mcp())
            // 2. In a use declaration (import statement like use crate::models::EntityKind)
            // 3. In a field_initializer context (struct field value like EntityKind::HtmlId)
            // 4. In an argument context (function argument like EntityKind::Class)
            let should_capture =
                parent_kind == "call_expression" || in_use || in_field_initializer || in_argument;

            if should_capture {
                // Collect all identifiers in the scoped_identifier path
                let mut identifiers: Vec<(String, usize)> = Vec::new();
                let mut child = first_child;
                loop {
                    if child.kind() == "identifier" || child.kind() == "type_identifier" {
                        let text = node_text(child, source).to_string();
                        let line = child.start_position().row + 1;
                        identifiers.push((text, line));
                    }
                    if let Some(next) = child.next_sibling() {
                        child = next;
                    } else {
                        break;
                    }
                }

                // Determine the type name and line based on context
                let (type_name, type_line) = if in_use {
                    let last_idx = identifiers.len() - 1;
                    (identifiers[last_idx].0.clone(), identifiers[last_idx].1)
                } else if identifiers.len() >= 3 {
                    let idx = identifiers.len() - 2;
                    (identifiers[idx].0.clone(), identifiers[idx].1)
                } else {
                    (
                        identifiers
                            .first()
                            .map(|(n, _)| n.clone())
                            .unwrap_or_default(),
                        identifiers.first().map(|(_, l)| *l).unwrap_or(1),
                    )
                };

                type_refs.push((type_line, type_name));
            }
        }
    }

    // SPECIAL CASE: Handle token_tree nodes inside macro_invocations
    // Macros aren't expanded by tree-sitter, so scoped identifiers inside macros
    // are stored as raw tokens, not AST nodes. We need to manually extract them.
    //
    // PERFORMANCE OPTIMIZATION (O(N) Substring Skipping):
    // For deeply nested macros like `vec![vec![vec![MyType::new()]]]`, tree-sitter produces
    // a tree of token_tree nodes. The outermost token_tree contains all the text, and
    // each nested macro body is also a token_tree. Without skipping, we would:
    //   1. Extract and search the entire outer token_tree text
    //   2. Recursively do the SAME work for every nested token_tree (exponential blowup)
    //
    // Solution: If a token_tree is fully contained within a parent token_tree's byte range
    // (i.e., `searched_range`), we skip it entirely since the parent already covered that text.
    if node.kind() == "token_tree" {
        let node_start_byte = node.start_byte();
        let node_end_byte = node.end_byte();

        // O(N) SKIP CHECK: If this token_tree is contained within a parent searched range,
        // skip the expensive string allocation and pattern matching.
        if let Some((parent_start, parent_end)) = searched_range
            && node_start_byte >= parent_start
            && node_end_byte <= parent_end
        {
            // This token_tree is fully contained in parent's searched range - skip!
            // Recurse to children WITHOUT passing the searched_range (they're not token_tree roots)
            let mut child = node.child(0);
            while let Some(c) = child {
                collect_type_nodes(&c, source, type_refs, None);
                child = c.next_sibling();
            }
            return;
        }

        let text = &source[node_start_byte..node_end_byte.min(source.len())];
        let text_str = String::from_utf8_lossy(text);

        // Look for patterns like "EntityKind::Class" or "Config::load_mcp"
        // Search for Type::Variant patterns (Type starts with uppercase)
        // Note: This simple pattern matching may have false positives in string literals,
        // but that's acceptable since macro token trees typically don't contain many string literals
        // with type-like patterns.
        for (idx, _) in text_str.match_indices("::") {
            if idx == 0 {
                continue;
            }

            // Skip if this :: appears to be inside a string literal
            // Simple heuristic: count quotes before this position
            let before_context = &text_str[..idx];
            let quote_count = before_context.matches('"').count();
            if quote_count % 2 == 1 {
                // Odd number of quotes means we're inside a string
                continue;
            }

            // Find the start of the type name before ::
            let type_start = before_context
                .rfind(|c: char| !c.is_alphanumeric() && c != '_')
                .map(|p| p + 1)
                .unwrap_or(0);
            let type_name = &before_context[type_start..];

            // Only capture if:
            // 1. Type name is not empty
            // 2. Starts with uppercase (Rust type convention)
            // 3. Is a valid identifier (alphanumeric + underscore only)
            if !type_name.is_empty()
                && type_name.chars().next().unwrap().is_uppercase()
                && type_name.chars().all(|c| c.is_alphanumeric() || c == '_')
            {
                // Calculate the actual line number by counting newlines up to this position
                let byte_offset = node_start_byte + type_start;
                let line = source[..byte_offset]
                    .iter()
                    .filter(|&&b| b == b'\n')
                    .count()
                    + 1;

                // Deduplicate: check if we already have this exact (line, type_name) pair
                if !type_refs.iter().any(|(l, n)| *l == line && n == type_name) {
                    type_refs.push((line, type_name.to_string()));
                }
            }
        }
    }

    // Recurse to children. When we're inside a processed token_tree, pass its range
    // so nested token_trees can skip themselves (O(N) optimization).
    let child_searched_range = if node.kind() == "token_tree" {
        Some((node.start_byte(), node.end_byte()))
    } else {
        searched_range
    };
    let mut child = node.child(0);
    while let Some(c) = child {
        collect_type_nodes(&c, source, type_refs, child_searched_range);
        child = c.next_sibling();
    }
}
