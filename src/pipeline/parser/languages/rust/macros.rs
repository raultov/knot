//! Macro invocation tracking for Rust source.
//!
//! Records every `name!(...)` / `name![...]` / `name!{...}` invocation and
//! attaches it to the nearest enclosing entity so the resulting graph can
//! answer "what macros does this function use?" queries.

use super::utils::find_nearest_entity_by_line;
use crate::models::{ParsedEntity, ReferenceIntent};
use crate::pipeline::parser::utils::node_text;
use tree_sitter::Node;

/// Collect macro invocations from Rust source and attach to nearest entities.
pub(crate) fn collect_rust_macro_references(
    root: Node<'_>,
    source: &[u8],
    entities: &mut [ParsedEntity],
    _file_path: &str,
    _repo_name: &str,
) {
    let mut macro_invocations: Vec<(usize, String)> = Vec::new();

    if let Some(first_child) = root.child(0) {
        collect_macro_nodes(&first_child, source, &mut macro_invocations);
    }

    for (line, macro_name) in macro_invocations {
        let target_idx = find_nearest_entity_by_line(entities, line);
        if target_idx < entities.len() {
            entities[target_idx]
                .reference_intents
                .push(ReferenceIntent::RustMacroCall { macro_name, line });
        }
    }
}

/// Recursively collect macro invocation nodes from Rust AST.
pub(crate) fn collect_macro_nodes(
    node: &Node<'_>,
    source: &[u8],
    macro_invocations: &mut Vec<(usize, String)>,
) {
    if node.kind() == "macro_invocation" {
        if let Some(macro_id) = node.child(0) {
            let macro_name = node_text(macro_id, source).to_string();
            let line = node.start_position().row + 1;
            macro_invocations.push((line, macro_name));
        }
    } else if let Some(child) = node.child(0) {
        collect_macro_nodes(&child, source, macro_invocations);
    }
    if let Some(sibling) = node.next_sibling() {
        collect_macro_nodes(&sibling, source, macro_invocations);
    }
}
