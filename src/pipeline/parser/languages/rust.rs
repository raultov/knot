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
        "rust.generics"
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
    use uuid::Uuid;

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
        let mut entities = vec![
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
}
