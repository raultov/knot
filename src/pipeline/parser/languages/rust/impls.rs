//! Analysis of Rust `impl` blocks: method reclassification, trait
//! implementations, and self-type extraction.
//!
//! Tree-sitter captures every `function_item` as a [`EntityKind::RustFunction`].
//! Those that live inside an `impl_item` are re-tagged here as
//! [`EntityKind::RustMethod`] and their FQN is rewritten to include the
//! self-type so `impl Foo { fn new() }` produces `Foo::new` instead of the
//! bare `new` returned by the function pass.

use crate::models::{EntityKind, ParsedEntity, ReferenceIntent};
use crate::pipeline::parser::utils::node_text;
use tree_sitter::Node;

/// Reclassify functions inside impl blocks as methods.
///
/// Tree-sitter captures all function_item nodes as RustFunction initially.
/// This function identifies which functions are actually methods (inside
/// `impl_item`) and changes their kind to `RustMethod`.
///
/// It also re-computes the FQN because Rust methods are FQN-formatted as
/// `Type::method` (with `::`), whereas Rust functions are stored as a bare
/// name. Without re-computation here, methods inside `impl Foo { fn new() }`
/// would keep the FQN `new` even though their enclosing class is `Foo`.
pub(crate) fn reclassify_methods_in_impl_blocks(
    root: Node<'_>,
    source: &[u8],
    entities: &mut [ParsedEntity],
) {
    // Collect line numbers of all functions inside impl blocks
    let mut method_lines = std::collections::HashSet::new();
    collect_method_lines(&root, &mut method_lines);

    if method_lines.is_empty() {
        return;
    }

    // Build class contexts from impl_item self-types. We reuse the same
    // helper that extract_class_contexts() calls, so behaviour is identical.
    let mut class_contexts: Vec<crate::pipeline::parser::context::ClassContext> = Vec::new();
    collect_impl_class_contexts(&root, source, &mut class_contexts);

    for entity in entities.iter_mut() {
        if entity.kind == EntityKind::RustFunction && method_lines.contains(&entity.start_line) {
            entity.kind = EntityKind::RustMethod;
            // Re-compute the FQN with the new kind so `impl Foo { fn new() }`
            // produces `Foo::new` instead of the bare `new` that the original
            // RustFunction pass produced.
            let (new_fqn, new_enclosing) =
                crate::pipeline::parser::context::compute_fqn_and_context(
                    &entity.name,
                    &EntityKind::RustMethod,
                    entity.start_line,
                    "rust",
                    &class_contexts,
                );
            entity.fqn = new_fqn;
            if entity.enclosing_class.is_none() {
                entity.enclosing_class = new_enclosing;
            }
        }
    }
}

/// Walk the tree and gather `(start_line, end_line, self_type)` for every
/// `impl_item`, so reclassification can resolve the FQN of methods inside it.
fn collect_impl_class_contexts(
    node: &Node<'_>,
    source: &[u8],
    contexts: &mut Vec<crate::pipeline::parser::context::ClassContext>,
) {
    if node.kind() == "impl_item"
        && let Some(self_type) = extract_impl_self_type(*node, source)
    {
        contexts.push(crate::pipeline::parser::context::ClassContext {
            name: self_type,
            start_line: node.start_position().row + 1,
            end_line: node.end_position().row + 1,
        });
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_impl_class_contexts(&child, source, contexts);
    }
}

/// Recursively collect line numbers of function_item nodes inside impl_item.
fn collect_method_lines(node: &Node<'_>, method_lines: &mut std::collections::HashSet<usize>) {
    if node.kind() == "impl_item" {
        // Inside an impl block - collect all function_item children
        let mut child = node.child(0);
        while let Some(c) = child {
            if c.kind() == "function_item" {
                let line = c.start_position().row + 1;
                method_lines.insert(line);
            } else {
                // Recurse to find nested function_items
                collect_method_lines_in_scope(&c, method_lines);
            }
            child = c.next_sibling();
        }
    } else {
        // Not in impl block yet - keep searching
        let mut child = node.child(0);
        while let Some(c) = child {
            collect_method_lines(&c, method_lines);
            child = c.next_sibling();
        }
    }
}

/// Helper to collect function_items within a specific scope (e.g., declaration_list).
fn collect_method_lines_in_scope(
    node: &Node<'_>,
    method_lines: &mut std::collections::HashSet<usize>,
) {
    if node.kind() == "function_item" {
        let line = node.start_position().row + 1;
        method_lines.insert(line);
    }

    let mut child = node.child(0);
    while let Some(c) = child {
        collect_method_lines_in_scope(&c, method_lines);
        child = c.next_sibling();
    }
}

/// Collect trait implementations from Rust impl blocks and attach to target structs/enums.
pub(crate) fn collect_rust_trait_implementations(
    root: Node<'_>,
    source: &[u8],
    entities: &mut [ParsedEntity],
    _file_path: &str,
    _repo_name: &str,
) {
    let mut implementations: Vec<(usize, String, String)> = Vec::new();

    // Start from root, not first child
    collect_impl_nodes(&root, source, &mut implementations);

    // Attach IMPLEMENTS relationships to target entities
    for (line, target_type, trait_name) in implementations {
        // Find the struct/enum that is the target of the impl
        if let Some(target_entity) = entities.iter_mut().find(|e| {
            e.name == target_type
                && matches!(
                    e.kind,
                    EntityKind::RustStruct | EntityKind::RustEnum | EntityKind::RustUnion
                )
        }) {
            target_entity
                .reference_intents
                .push(ReferenceIntent::Implements {
                    interface: trait_name,
                    line,
                });
        }
    }
}

/// Extract the self-type base name from a Rust `impl_item` node.
///
/// For `impl Foo { ... }` → returns `Some("Foo")`.
/// For `impl Bar for Foo { ... }` → returns `Some("Foo")` (the self-type,
///   not the trait).
/// For `impl<T> Foo<T> { ... }` → returns `Some("Foo")` (generics dropped).
/// For unrecognised shapes (lifetimes, exotic paths, parse failures) →
///   returns `None` (the caller should skip silently).
pub(crate) fn extract_impl_self_type(node: Node<'_>, source: &[u8]) -> Option<String> {
    if node.kind() != "impl_item" {
        return None;
    }

    // Walk the impl_item children. Two shapes we must distinguish:
    //
    //   impl <trait> for <type> { ... }
    //     children: impl, <trait-ish>, for, <type>, { ... }
    //
    //   impl <impl-generics> <type> { ... }
    //     children: impl, generic_type(<T>), <type>, { ... }
    //     (the impl-generics is a generic_type whose only child is a
    //      type_parameter, not a real type — its `type` field is absent.)
    //
    // We pick the *first* node that has a real type identity (i.e. a
    // `type` field or a bare `type_identifier`) as the self-type for
    // inherent impls, and the *last* such node for `impl Trait for Type`.
    let mut saw_for = false;
    let mut first_type: Option<String> = None;
    let mut last_type: Option<String> = None;

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "for" => {
                saw_for = true;
            }
            // "for" keyword: switch from trait-name collection to self-type collection
            "lifetime" | "where_clause" | "declaration_list" | "attribute_item" | "token_tree" => {
                continue;
            }
            _ => {
                if let Some(name) = extract_type_base_name(child, source) {
                    if first_type.is_none() {
                        first_type = Some(name.clone());
                    }
                    last_type = Some(name);
                }
            }
        }
    }

    if saw_for {
        // `impl Trait for Type` — Type is the LAST type token.
        last_type.or(first_type)
    } else {
        // `impl Type` — Type is the FIRST type token.
        first_type
    }
}

/// Extract the base type name from a Rust type AST node.
///
/// Handles bare identifiers (`Foo`), scoped paths (`foo::Bar`), generic
/// types (`Foo<T>`), and wrappers (`&T`, `*T`, `[T]`, `(T,)`). For
/// `generic_type` nodes the `type` field is required — an impl-generics
/// placeholder like `<T>` in `impl<T> Foo<T>` has no `type` field and is
/// skipped so we don't confuse it with the real self-type.
fn extract_type_base_name(node: Node<'_>, source: &[u8]) -> Option<String> {
    match node.kind() {
        "type_identifier" => Some(node_text(node, source).to_string()),
        "scoped_type_identifier" => {
            let mut last: Option<String> = None;
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "type_identifier" {
                    last = Some(node_text(child, source).to_string());
                }
            }
            last
        }
        "generic_type" => node
            .child_by_field_name("type")
            .and_then(|t| extract_type_base_name(t, source)),
        "reference_type" | "pointer_type" | "array_type" | "slice_type" | "tuple_type" => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if let Some(name) = extract_type_base_name(child, source)
                    && !name.is_empty()
                {
                    return Some(name);
                }
            }
            None
        }
        _ => None,
    }
}

/// Recursively collect impl_item nodes that implement traits.
fn collect_impl_nodes(
    node: &Node<'_>,
    source: &[u8],
    implementations: &mut Vec<(usize, String, String)>,
) {
    if node.kind() == "impl_item" {
        let line = node.start_position().row + 1;
        let impl_text = node_text(*node, source);

        // Simple pattern matching for "impl Trait for Type"
        // This handles the common case: impl TraitName for TypeName { ... }
        if impl_text.contains(" for ") {
            let mut type_identifiers: Vec<String> = Vec::new();

            // Collect all type_identifier nodes in order
            let mut child = node.child(0);
            while let Some(c) = child {
                if c.kind() == "type_identifier" {
                    type_identifiers.push(node_text(c, source).to_string());
                } else if c.kind() == "generic_type" {
                    // For generic types like Container<T>, extract just the base name
                    if let Some(name_node) = c.child_by_field_name("type")
                        && name_node.kind() == "type_identifier"
                    {
                        type_identifiers.push(node_text(name_node, source).to_string());
                    }
                }
                child = c.next_sibling();
            }

            // In "impl Trait for Type", we get [Trait, Type] as type_identifiers
            if type_identifiers.len() >= 2 {
                let trait_name = type_identifiers[0].clone();
                let target_type = type_identifiers[1].clone();
                implementations.push((line, target_type, trait_name));
            }
        }
        // Note: We ignore inherent impls (impl Type without trait) for now
    }

    // Recurse into children
    let mut child = node.child(0);
    while let Some(c) = child {
        collect_impl_nodes(&c, source, implementations);
        child = c.next_sibling();
    }
}
