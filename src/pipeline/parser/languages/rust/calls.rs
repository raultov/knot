//! Function and method call extraction for Rust source.
//!
//! Walks `call_expression` nodes plus the body of macro `token_tree` nodes
//! (whose interior is not parsed by tree-sitter) to record every call site.
//! The result is a [`ReferenceIntent::Call`] entry attached to the nearest
//! enclosing entity.

use super::utils::find_nearest_entity_by_line;
use crate::models::{ParsedEntity, ReferenceIntent};
use crate::pipeline::parser::utils::node_text;
use tree_sitter::Node;

/// Collect function calls from Rust source and attach to nearest entities.
///
/// Handles:
/// - Direct function calls: `function_name()`
/// - Method calls: `obj.method()`
/// - Scoped calls: `module::function()` or `Type::method()`
pub(crate) fn collect_rust_call_references(
    root: Node<'_>,
    source: &[u8],
    entities: &mut [ParsedEntity],
    _file_path: &str,
    _repo_name: &str,
) {
    let mut call_intents: Vec<(usize, String, Option<String>)> = Vec::new();

    // Start from root, not first child (to process all top-level items)
    collect_call_nodes(&root, source, &mut call_intents);

    for (line, func_name, receiver) in call_intents {
        let target_idx = find_nearest_entity_by_line(entities, line);
        if target_idx < entities.len() {
            // Translate `Self::method` to the enclosing class. Strategy 1 of
            // the resolver (local call) keys on `caller_enclosing_class`, not
            // the literal `Self` keyword, so we substitute here at parse time.
            let receiver = if receiver.as_deref() == Some("Self")
                && let Some(enclosing) = entities[target_idx].enclosing_class.clone()
            {
                Some(enclosing)
            } else {
                receiver
            };
            entities[target_idx]
                .reference_intents
                .push(ReferenceIntent::Call {
                    method: func_name,
                    receiver,
                    line,
                    arg_count: None,
                });
        }
    }
}

/// Recursively collect call_expression nodes from the AST.
pub(crate) fn collect_call_nodes(
    node: &Node<'_>,
    source: &[u8],
    calls: &mut Vec<(usize, String, Option<String>)>,
) {
    if node.kind() == "call_expression" {
        let line = node.start_position().row + 1;

        // Try to extract function name and optional receiver
        if let Some((func_name, receiver)) = extract_call_details(*node, source) {
            calls.push((line, func_name, receiver));
        }
    } else if node.kind() == "token_tree" {
        // Special case: tree-sitter does not parse the inside of macro invocations.
        // It exposes the contents as a `token_tree` of raw tokens.
        // To recover function calls inside `assert!(...)`, `vec![...]`, etc.,
        // we manually scan for the pattern: `identifier` (or `scoped_identifier`)
        // immediately followed by a `token_tree` that starts with `(` or `[`.
        let mut child = node.child(0);
        while let Some(c) = child {
            if c.kind() == "identifier" {
                if let Some(next) = c.next_sibling()
                    && next.kind() == "token_tree"
                {
                    let next_text = node_text(next, source);
                    if next_text.starts_with('(') || next_text.starts_with('[') {
                        let line = c.start_position().row + 1;
                        let func_name = node_text(c, source).to_string();
                        calls.push((line, func_name, None));
                    }
                }
            } else if c.kind() == "scoped_identifier"
                && let Some(next) = c.next_sibling()
                && next.kind() == "token_tree"
            {
                let next_text = node_text(next, source);
                if next_text.starts_with('(') || next_text.starts_with('[') {
                    let line = c.start_position().row + 1;
                    if let Some((func_name, receiver)) = extract_from_scoped_identifier(c, source) {
                        calls.push((line, func_name, receiver));
                    }
                }
            }
            child = c.next_sibling();
        }
    }

    // Recurse to children
    let mut child = node.child(0);
    while let Some(c) = child {
        collect_call_nodes(&c, source, calls);
        child = c.next_sibling();
    }
}

/// Extract function name and receiver from a call_expression node.
fn extract_call_details(node: Node<'_>, source: &[u8]) -> Option<(String, Option<String>)> {
    // Find the function part of the call_expression
    let mut child = node.child(0);
    while let Some(c) = child {
        match c.kind() {
            // Direct function call: identifier
            "identifier" => {
                let func_name = node_text(c, source).to_string();
                return Some((func_name, None));
            }
            // Method call: field_expression (receiver.method)
            "field_expression" => {
                if let Some((method_name, receiver)) = extract_from_field_expression(c, source) {
                    return Some((method_name, Some(receiver)));
                }
            }
            // Scoped call: scoped_identifier (Module::function or Type::method)
            "scoped_identifier" => {
                let (func_name, receiver) = extract_from_scoped_identifier(c, source)?;
                return Some((func_name, receiver));
            }
            _ => {}
        }
        child = c.next_sibling();
    }
    None
}

/// Extract method name and receiver from field_expression (e.g., obj.method)
fn extract_from_field_expression(node: Node<'_>, source: &[u8]) -> Option<(String, String)> {
    let mut method_name = String::new();
    let mut receiver = String::new();
    let mut found_method = false;
    let mut found_receiver = false;

    let mut child = node.child(0);
    while let Some(c) = child {
        match c.kind() {
            "field_identifier" => {
                method_name = node_text(c, source).to_string();
                found_method = true;
            }
            "identifier" => {
                receiver = node_text(c, source).to_string();
                found_receiver = true;
            }
            _ => {}
        }
        child = c.next_sibling();
    }

    if found_method && found_receiver {
        Some((method_name, receiver))
    } else {
        None
    }
}

/// Extract function name and optional receiver from scoped_identifier.
///
/// Examples:
///
/// - `module::function` → `("function", None)` — lowercase module, no receiver.
/// - `Type::method` → `("method", Some("Type"))` — uppercase type, receiver set.
/// - `crate::mcp_handler::KnotMcpHandler::new` → `("new", Some("KnotMcpHandler"))`
///   — only the penultimate segment is used as a receiver; we walk all
///   `identifier` children (recursing into nested `scoped_identifier` nodes)
///   and pick the last one as the method name, then treat the segment
///   immediately before it as the receiver (only if its first character is
///   uppercase ASCII, so modules like `std` or `crate::mcp_handler` are
///   ignored).
fn extract_from_scoped_identifier(
    node: Node<'_>,
    source: &[u8],
) -> Option<(String, Option<String>)> {
    let mut identifiers: Vec<String> = Vec::new();
    collect_scoped_identifiers(node, source, &mut identifiers);

    let last = identifiers.pop()?;
    let method_name = last;
    let receiver = if let Some(prev) = identifiers.last() {
        let first = prev.chars().next();
        if first.is_some_and(|c| c.is_ascii_uppercase()) {
            Some(prev.clone())
        } else {
            None
        }
    } else {
        None
    };

    Some((method_name, receiver))
}

/// Recursively walk a `scoped_identifier` (and its nested children) to
/// collect every `identifier` in left-to-right order. `crate`, `::`, and
/// `super` are treated like ordinary identifiers; if `super` appears it will
/// be lowercase and dropped at the receiver-selection step.
fn collect_scoped_identifiers(node: Node<'_>, source: &[u8], out: &mut Vec<String>) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "identifier" | "type_identifier" => {
                out.push(node_text(child, source).to_string());
            }
            "scoped_identifier" => {
                // Recurse into nested scoped paths.
                collect_scoped_identifiers(child, source, out);
            }
            _ => {}
        }
    }
}
