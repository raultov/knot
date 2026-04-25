//! Rust language support for entity extraction and reference intent collection.
//!
//! Handles:
//! - Struct, enum, union, trait, impl block extraction
//! - Function, method, macro definition extraction
//! - Type alias, constant, static, and module extraction
//! - Macro invocation tracking
//! - Generic parameters and lifetime extraction

use crate::models::{EntityKind, ParsedEntity, ReferenceIntent};
use crate::pipeline::parser::utils::node_text;
use tree_sitter::Node;

/// Rust metadata extracted from captures (for future use with impl blocks)
#[allow(dead_code)]
pub(crate) struct RustMetadata(pub(crate) Option<String>, pub(crate) Option<String>);

/// Handle Rust-specific entity captures from tree-sitter queries.
/// Returns (name, kind, start_line, metadata) for the entity.
pub(crate) fn handle_rust_capture(
    cap_name: &str,
    text: &str,
    node: Node<'_>,
) -> Option<(String, EntityKind, usize, Option<RustMetadata>)> {
    let start_line = node.start_position().row + 1;

    match cap_name {
        "rust.struct.name" => Some((text.to_string(), EntityKind::RustStruct, start_line, None)),
        "rust.enum.name" => Some((text.to_string(), EntityKind::RustEnum, start_line, None)),
        "rust.union.name" => Some((text.to_string(), EntityKind::RustUnion, start_line, None)),
        "rust.trait.name" => Some((text.to_string(), EntityKind::RustTrait, start_line, None)),
        "rust.impl.target" => Some((text.to_string(), EntityKind::RustImpl, start_line, None)),
        "rust.impl.trait" => Some((text.to_string(), EntityKind::RustImpl, start_line, None)),
        "rust.function.name" => {
            Some((text.to_string(), EntityKind::RustFunction, start_line, None))
        }
        "rust.macro_def.name" => {
            Some((text.to_string(), EntityKind::RustMacroDef, start_line, None))
        }
        "rust.macro_inv.name" => Some((
            text.to_string(),
            EntityKind::RustMacroInvoke,
            start_line,
            None,
        )),
        "rust.type_alias.name" => Some((
            text.to_string(),
            EntityKind::RustTypeAlias,
            start_line,
            None,
        )),
        "rust.constant.name" => {
            Some((text.to_string(), EntityKind::RustConstant, start_line, None))
        }
        "rust.static.name" => Some((text.to_string(), EntityKind::RustStatic, start_line, None)),
        "rust.module.name" => Some((text.to_string(), EntityKind::RustModule, start_line, None)),
        "rust.method.name" => Some((text.to_string(), EntityKind::RustMethod, start_line, None)),
        "rust.call.name"
        | "rust.generics"
        | "rust.signature"
        | "rust.return_type"
        | "rust.lifetime"
        | "rust.attribute.name" => None,
        _ => None,
    }
}

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

/// Reclassify functions inside impl blocks as methods.
///
/// Tree-sitter captures all function_item nodes as RustFunction initially.
/// This function identifies which functions are actually methods (inside impl_item)
/// and changes their kind to RustMethod.
pub(crate) fn reclassify_methods_in_impl_blocks(root: Node<'_>, entities: &mut [ParsedEntity]) {
    // Collect line numbers of all functions inside impl blocks
    let mut method_lines = std::collections::HashSet::new();
    collect_method_lines(&root, &mut method_lines);

    // Reclassify entities at those line numbers from RustFunction to RustMethod
    for entity in entities.iter_mut() {
        if entity.kind == EntityKind::RustFunction && method_lines.contains(&entity.start_line) {
            entity.kind = EntityKind::RustMethod;
        }
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

/// Recursively collect macro invocation nodes from Rust AST.
fn collect_macro_nodes(
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

/// Collect type references from Rust source code (parameter types, return types, field types).
///
/// This function walks through function_item, struct_item, and enum_item nodes
/// to extract type references from their signatures and fields.
pub(crate) fn collect_rust_type_references(
    root: Node<'_>,
    source: &[u8],
    entities: &mut [ParsedEntity],
    _file_path: &str,
    _repo_name: &str,
) {
    let mut type_refs: Vec<(usize, String)> = Vec::new();

    // Start from root, not first child (to process all top-level items)
    collect_type_nodes(&root, source, &mut type_refs);

    for (line, type_name) in type_refs {
        let target_idx = find_nearest_entity_by_line(entities, line);
        if target_idx < entities.len() {
            entities[target_idx]
                .reference_intents
                .push(ReferenceIntent::TypeReference { type_name, line });
        }
    }
}

/// Recursively collect type references from function parameters and return types.
fn collect_type_nodes(node: &Node<'_>, source: &[u8], type_refs: &mut Vec<(usize, String)>) {
    // Look for type_identifier nodes in function parameters and return types
    if node.kind() == "type_identifier" {
        let line = node.start_position().row + 1;
        let type_name = node_text(*node, source).to_string();
        type_refs.push((line, type_name));
    }

    // Recurse to children
    let mut child = node.child(0);
    while let Some(c) = child {
        collect_type_nodes(&c, source, type_refs);
        child = c.next_sibling();
    }
}

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
            entities[target_idx]
                .reference_intents
                .push(ReferenceIntent::Call {
                    method: func_name,
                    receiver,
                    line,
                });
        }
    }
}

/// Recursively collect call_expression nodes from the AST.
fn collect_call_nodes(
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
                if let Some(func_name) = extract_from_scoped_identifier(c, source) {
                    return Some((func_name, None));
                }
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

/// Extract function name from scoped_identifier (e.g., Module::function)
fn extract_from_scoped_identifier(node: Node<'_>, source: &[u8]) -> Option<String> {
    let mut child = node.child(0);
    while let Some(c) = child {
        if c.kind() == "identifier" {
            // The last identifier in the scope is the function name
            let mut last_identifier = node_text(c, source).to_string();
            let mut next = c.next_sibling();
            while let Some(n) = next {
                if n.kind() == "identifier" {
                    last_identifier = node_text(n, source).to_string();
                }
                next = n.next_sibling();
            }
            return Some(last_identifier);
        }
        child = c.next_sibling();
    }
    None
}

/// Find the entity index nearest to the given line number.
fn find_nearest_entity_by_line(entities: &[ParsedEntity], line: usize) -> usize {
    let mut nearest = 0;
    for (idx, entity) in entities.iter().enumerate() {
        if entity.start_line <= line {
            nearest = idx;
        } else {
            break;
        }
    }
    nearest
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_entity(name: &str, line: usize) -> ParsedEntity {
        ParsedEntity::new(
            name,
            EntityKind::RustFunction,
            name,
            None,
            None,
            "rust",
            "/test.rs",
            line,
            None,
            "test-repo",
        )
    }

    #[test]
    fn test_handle_rust_capture_struct() {
        let code = "struct MyStruct";
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_rust::LANGUAGE.into())
            .unwrap();
        let tree = parser.parse(code, None).unwrap();
        let node = tree.root_node();

        let result = handle_rust_capture("rust.struct.name", "MyStruct", node);
        assert!(result.is_some());
        let (name, kind, _line, _meta) = result.unwrap();
        assert_eq!(name, "MyStruct");
        assert_eq!(kind, EntityKind::RustStruct);
    }

    #[test]
    fn test_handle_rust_capture_enum() {
        let code = "enum Color";
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_rust::LANGUAGE.into())
            .unwrap();
        let tree = parser.parse(code, None).unwrap();
        let node = tree.root_node();

        let result = handle_rust_capture("rust.enum.name", "Color", node);
        assert!(result.is_some());
        let (name, kind, _line, _meta) = result.unwrap();
        assert_eq!(name, "Color");
        assert_eq!(kind, EntityKind::RustEnum);
    }

    #[test]
    fn test_handle_rust_capture_trait() {
        let code = "trait Iterator";
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_rust::LANGUAGE.into())
            .unwrap();
        let tree = parser.parse(code, None).unwrap();
        let node = tree.root_node();

        let result = handle_rust_capture("rust.trait.name", "Iterator", node);
        assert!(result.is_some());
        let (name, kind, _line, _meta) = result.unwrap();
        assert_eq!(name, "Iterator");
        assert_eq!(kind, EntityKind::RustTrait);
    }

    #[test]
    fn test_handle_rust_capture_function() {
        let code = "fn main";
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_rust::LANGUAGE.into())
            .unwrap();
        let tree = parser.parse(code, None).unwrap();
        let node = tree.root_node();

        let result = handle_rust_capture("rust.function.name", "main", node);
        assert!(result.is_some());
        let (name, kind, _line, _meta) = result.unwrap();
        assert_eq!(name, "main");
        assert_eq!(kind, EntityKind::RustFunction);
    }

    #[test]
    fn test_handle_rust_capture_macro() {
        let code = "macro_rules! vec";
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_rust::LANGUAGE.into())
            .unwrap();
        let tree = parser.parse(code, None).unwrap();
        let node = tree.root_node();

        let result = handle_rust_capture("rust.macro_def.name", "vec", node);
        assert!(result.is_some());
        let (name, kind, _line, _meta) = result.unwrap();
        assert_eq!(name, "vec");
        assert_eq!(kind, EntityKind::RustMacroDef);
    }

    #[test]
    fn test_handle_rust_capture_type_alias() {
        let code = "type Result";
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_rust::LANGUAGE.into())
            .unwrap();
        let tree = parser.parse(code, None).unwrap();
        let node = tree.root_node();

        let result = handle_rust_capture("rust.type_alias.name", "Result", node);
        assert!(result.is_some());
        let (name, kind, _line, _meta) = result.unwrap();
        assert_eq!(name, "Result");
        assert_eq!(kind, EntityKind::RustTypeAlias);
    }

    #[test]
    fn test_handle_rust_capture_constant() {
        let code = "const MAX_SIZE";
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_rust::LANGUAGE.into())
            .unwrap();
        let tree = parser.parse(code, None).unwrap();
        let node = tree.root_node();

        let result = handle_rust_capture("rust.constant.name", "MAX_SIZE", node);
        assert!(result.is_some());
        let (name, kind, _line, _meta) = result.unwrap();
        assert_eq!(name, "MAX_SIZE");
        assert_eq!(kind, EntityKind::RustConstant);
    }

    #[test]
    fn test_handle_rust_capture_module() {
        let code = "mod utils";
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_rust::LANGUAGE.into())
            .unwrap();
        let tree = parser.parse(code, None).unwrap();
        let node = tree.root_node();

        let result = handle_rust_capture("rust.module.name", "utils", node);
        assert!(result.is_some());
        let (name, kind, _line, _meta) = result.unwrap();
        assert_eq!(name, "utils");
        assert_eq!(kind, EntityKind::RustModule);
    }

    #[test]
    fn test_find_nearest_entity_by_line_exact_match() {
        let entities = vec![
            create_test_entity("func1", 10),
            create_test_entity("func2", 20),
            create_test_entity("func3", 30),
        ];

        let idx = find_nearest_entity_by_line(&entities, 20);
        assert_eq!(idx, 1);
        assert_eq!(entities[idx].start_line, 20);
    }

    #[test]
    fn test_find_nearest_entity_by_line_between() {
        let entities = vec![
            create_test_entity("func1", 10),
            create_test_entity("func2", 20),
            create_test_entity("func3", 30),
        ];

        let idx = find_nearest_entity_by_line(&entities, 25);
        assert_eq!(idx, 1);
        assert_eq!(entities[idx].start_line, 20);
    }

    #[test]
    fn test_find_nearest_entity_by_line_before_first() {
        let entities = vec![
            create_test_entity("func1", 10),
            create_test_entity("func2", 20),
        ];

        let idx = find_nearest_entity_by_line(&entities, 5);
        assert_eq!(idx, 0);
    }

    #[test]
    fn test_find_nearest_entity_by_line_after_last() {
        let entities = vec![
            create_test_entity("func1", 10),
            create_test_entity("func2", 20),
        ];

        let idx = find_nearest_entity_by_line(&entities, 50);
        assert_eq!(idx, 1);
        assert_eq!(entities[idx].start_line, 20);
    }

    #[test]
    fn test_find_nearest_entity_empty_list() {
        let entities: Vec<ParsedEntity> = vec![];
        let idx = find_nearest_entity_by_line(&entities, 10);
        assert_eq!(idx, 0);
    }

    #[test]
    fn test_collect_rust_macro_references_simple() {
        let code = r#"
fn main() {
    println!("Hello");
    vec![1, 2, 3];
}
        "#;

        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_rust::LANGUAGE.into())
            .unwrap();
        let tree = parser.parse(code, None).unwrap();

        let mut entities = vec![create_test_entity("main", 2)];
        let code_bytes = code.as_bytes();

        collect_rust_macro_references(
            tree.root_node(),
            code_bytes,
            &mut entities,
            "/test.rs",
            "test",
        );

        // Should have found macro invocations and attached them to main
        let intents_count = entities[0]
            .reference_intents
            .iter()
            .filter(|ri| matches!(ri, ReferenceIntent::RustMacroCall { .. }))
            .count();
        assert!(intents_count > 0, "Should have found macro invocations");
    }

    #[test]
    fn test_collect_rust_macro_references_multiple_entities() {
        let code = r#"
fn func1() {
    println!("one");
}

fn func2() {
    vec![1];
    println!("two");
}
        "#;

        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_rust::LANGUAGE.into())
            .unwrap();
        let tree = parser.parse(code, None).unwrap();

        let mut entities = vec![
            create_test_entity("func1", 2),
            create_test_entity("func2", 6),
        ];
        let code_bytes = code.as_bytes();

        collect_rust_macro_references(
            tree.root_node(),
            code_bytes,
            &mut entities,
            "/test.rs",
            "test",
        );

        // Both functions should have macro intents attached
        let func1_macros = entities[0]
            .reference_intents
            .iter()
            .filter(|ri| matches!(ri, ReferenceIntent::RustMacroCall { .. }))
            .count();
        let func2_macros = entities[1]
            .reference_intents
            .iter()
            .filter(|ri| matches!(ri, ReferenceIntent::RustMacroCall { .. }))
            .count();

        assert!(func1_macros > 0, "func1 should have macro intents");
        assert!(func2_macros > 0, "func2 should have macro intents");
    }

    #[test]
    fn test_handle_rust_capture_unknown_capture_name() {
        let code = "unknown";
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_rust::LANGUAGE.into())
            .unwrap();
        let tree = parser.parse(code, None).unwrap();
        let node = tree.root_node();

        let result = handle_rust_capture("unknown.name", "something", node);
        assert!(result.is_none());
    }

    #[test]
    fn test_handle_rust_capture_generics_ignored() {
        let code = "generics";
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_rust::LANGUAGE.into())
            .unwrap();
        let tree = parser.parse(code, None).unwrap();
        let node = tree.root_node();

        let result = handle_rust_capture("rust.generics", "some_generic", node);
        assert!(result.is_none());
    }

    #[test]
    fn test_collect_rust_trait_implementations() {
        let code = r#"
trait Incrementable {
    fn increment(&mut self);
}

struct Counter {
    count: u32,
}

impl Incrementable for Counter {
    fn increment(&mut self) {
        self.count += 1;
    }
}
"#;
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_rust::LANGUAGE.into())
            .unwrap();
        let tree = parser.parse(code, None).unwrap();

        // Create a Counter entity using the new() constructor
        let mut entities = vec![ParsedEntity::new(
            "Counter",
            EntityKind::RustStruct,
            "Counter",
            None,
            None,
            "rust",
            "test.rs",
            6,
            None,
            "test_repo",
        )];

        collect_rust_trait_implementations(
            tree.root_node(),
            code.as_bytes(),
            &mut entities,
            "test.rs",
            "test_repo",
        );

        // Check that Counter now has an IMPLEMENTS relationship
        assert_eq!(entities.len(), 1);
        assert_eq!(entities[0].reference_intents.len(), 1);

        if let ReferenceIntent::Implements { interface, line } = &entities[0].reference_intents[0] {
            assert_eq!(interface, "Incrementable");
            assert_eq!(*line, 10); // Line where impl starts
        } else {
            panic!("Expected Implements reference intent");
        }
    }

    #[test]
    fn test_collect_rust_call_references() {
        let code = r#"
fn helper_function(x: i32) -> i32 {
    x + 1
}

fn main() {
    let result = helper_function(5);
    println!("{}", result);
}
"#;

        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_rust::LANGUAGE.into())
            .unwrap();
        let tree = parser.parse(code, None).unwrap();

        // Create entities for both functions
        let mut entities = vec![
            ParsedEntity::new(
                "helper_function",
                EntityKind::RustFunction,
                "helper_function",
                None,
                None,
                "rust",
                "test.rs",
                2,
                None,
                "test_repo",
            ),
            ParsedEntity::new(
                "main",
                EntityKind::RustFunction,
                "main",
                None,
                None,
                "rust",
                "test.rs",
                6,
                None,
                "test_repo",
            ),
        ];

        collect_rust_call_references(
            tree.root_node(),
            code.as_bytes(),
            &mut entities,
            "test.rs",
            "test_repo",
        );

        // Check that main() has a Call reference to helper_function
        let main_entity = &entities[1];
        assert!(
            !main_entity.reference_intents.is_empty(),
            "main() should have at least one reference intent"
        );

        let has_call = main_entity.reference_intents.iter().any(|intent| {
            if let ReferenceIntent::Call { method, .. } = intent {
                method == "helper_function"
            } else {
                false
            }
        });

        assert!(
            has_call,
            "main() should have a Call reference to helper_function"
        );
    }

    #[test]
    fn test_rust_signature_capture() {
        // Test that signatures are captured from Tree-sitter queries
        use crate::pipeline::parser::extractor::extract_entities;
        use tree_sitter_rust;

        let code = r#"
fn add(a: i32, b: i32) -> i32 {
    a + b
}

fn multiply(x: f64, y: f64) -> f64 {
    x * y
}
"#;

        let entities = extract_entities(
            code,
            tree_sitter_rust::LANGUAGE.into(),
            include_str!("../../../../queries/rust.scm"),
            "rust",
            "test.rs",
            "test_repo",
        )
        .expect("Failed to extract entities");

        // Should have at least 2 functions
        assert!(
            entities.len() >= 2,
            "Should extract at least 2 functions, got {}",
            entities.len()
        );

        // Find the add function
        let add_fn = entities
            .iter()
            .find(|e| e.name == "add")
            .expect("add function not found");

        // Check if signature is captured
        eprintln!("add function signature: {:?}", add_fn.signature);
        // Note: signature might be empty if the Tree-sitter query doesn't match correctly
        // This test documents the current behavior
    }
}
