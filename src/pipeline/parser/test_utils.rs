//! Test utilities for parsing snippets of code in memory.

use crate::models::ReferenceIntent;
use tree_sitter::{Parser, Tree};

/// Parse a Java code snippet and return the syntax tree.
#[cfg(test)]
pub(crate) fn parse_java_snippet(code: &str) -> Result<Tree, String> {
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_java::LANGUAGE.into())
        .map_err(|e| format!("Failed to set Java language: {e}"))?;

    parser
        .parse(code, None)
        .ok_or_else(|| "Failed to parse Java code snippet".to_string())
}

/// Parse a TypeScript code snippet and return the syntax tree.
#[cfg(test)]
pub(crate) fn parse_typescript_snippet(code: &str) -> Result<Tree, String> {
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into())
        .map_err(|e| format!("Failed to set TypeScript language: {e}"))?;

    parser
        .parse(code, None)
        .ok_or_else(|| "Failed to parse TypeScript code snippet".to_string())
}

/// Parse a TSX code snippet and return the syntax tree.
#[cfg(test)]
pub(crate) fn parse_tsx_snippet(code: &str) -> Result<Tree, String> {
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_typescript::LANGUAGE_TSX.into())
        .map_err(|e| format!("Failed to set TSX language: {e}"))?;

    parser
        .parse(code, None)
        .ok_or_else(|| "Failed to parse TSX code snippet".to_string())
}

/// Parse a JavaScript code snippet and return the syntax tree.
#[cfg(test)]
pub(crate) fn parse_javascript_snippet(code: &str) -> Result<Tree, String> {
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_javascript::LANGUAGE.into())
        .map_err(|e| format!("Failed to set JavaScript language: {e}"))?;

    parser
        .parse(code, None)
        .ok_or_else(|| "Failed to parse JavaScript code snippet".to_string())
}

/// Parse a JSX code snippet and return the syntax tree.
#[cfg(test)]
#[allow(dead_code)]
pub(crate) fn parse_jsx_snippet(code: &str) -> Result<Tree, String> {
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_javascript::LANGUAGE.into())
        .map_err(|e| format!("Failed to set JSX language: {e}"))?;

    parser
        .parse(code, None)
        .ok_or_else(|| "Failed to parse JSX code snippet".to_string())
}

/// Parse a Kotlin code snippet and return the syntax tree.
#[cfg(test)]
pub(crate) fn parse_kotlin_snippet(code: &str) -> Result<Tree, String> {
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_kotlin_ng::LANGUAGE.into())
        .map_err(|e| format!("Failed to set Kotlin language: {e}"))?;

    parser
        .parse(code, None)
        .ok_or_else(|| "Failed to parse Kotlin code snippet".to_string())
}

/// Parse a Rust code snippet and return the syntax tree.
#[cfg(test)]
pub(crate) fn parse_rust_snippet(code: &str) -> Result<Tree, String> {
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_rust::LANGUAGE.into())
        .map_err(|e| format!("Failed to set Rust language: {e}"))?;

    parser
        .parse(code, None)
        .ok_or_else(|| "Failed to parse Rust code snippet".to_string())
}

/// Parse a Markdown snippet and return the syntax tree.
#[cfg(test)]
pub(crate) fn parse_markdown_snippet(code: &str) -> Result<Tree, String> {
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_md::LANGUAGE.into())
        .map_err(|e| format!("Failed to set Markdown language: {e}"))?;

    parser
        .parse(code, None)
        .ok_or_else(|| "Failed to parse Markdown code snippet".to_string())
}

/// Parse a Python code snippet and return the syntax tree.
#[cfg(test)]
pub(crate) fn parse_python_snippet(code: &str) -> Result<Tree, String> {
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_python::LANGUAGE.into())
        .map_err(|e| format!("Failed to set Python language: {e}"))?;

    parser
        .parse(code, None)
        .ok_or_else(|| "Failed to parse Python code snippet".to_string())
}

/// Collect `Extends` parent names from a slice of reference intents.
#[cfg(test)]
pub(crate) fn collect_extends(intents: &[ReferenceIntent]) -> Vec<&str> {
    intents
        .iter()
        .filter_map(|r| {
            if let ReferenceIntent::Extends { parent, .. } = r {
                Some(parent.as_str())
            } else {
                None
            }
        })
        .collect()
}

/// Collect `Implements` interface names from a slice of reference intents.
#[cfg(test)]
pub(crate) fn collect_implements(intents: &[ReferenceIntent]) -> Vec<&str> {
    intents
        .iter()
        .filter_map(|r| {
            if let ReferenceIntent::Implements { interface, .. } = r {
                Some(interface.as_str())
            } else {
                None
            }
        })
        .collect()
}

// ── Generic AST node finders for tests ──────────────────────────

/// Find the first node matching any of the given kinds via recursive DFS.
#[cfg(test)]
pub(crate) fn find_first_node<'a>(
    node: tree_sitter::Node<'a>,
    kinds: &[&str],
) -> Option<tree_sitter::Node<'a>> {
    if kinds.contains(&node.kind()) {
        return Some(node);
    }
    let mut i = 0u32;
    while let Some(child) = node.child(i) {
        if let Some(found) = find_first_node(child, kinds) {
            return Some(found);
        }
        i += 1;
    }
    None
}

/// Find the first `jsx_self_closing_element` or `jsx_opening_element`.
#[cfg(test)]
pub(crate) fn find_jsx_element(node: tree_sitter::Node) -> Option<tree_sitter::Node> {
    find_first_node(node, &["jsx_self_closing_element", "jsx_opening_element"])
}

/// Find the first `jsx_opening_element`.
#[cfg(test)]
pub(crate) fn find_jsx_opening_element(node: tree_sitter::Node) -> Option<tree_sitter::Node> {
    find_first_node(node, &["jsx_opening_element"])
}

/// Find the first `jsx_self_closing_element`.
#[cfg(test)]
pub(crate) fn find_jsx_self_closing(node: tree_sitter::Node) -> Option<tree_sitter::Node> {
    find_first_node(node, &["jsx_self_closing_element"])
}

/// Find the first `call_expression`.
#[cfg(test)]
pub(crate) fn find_call_expression(node: tree_sitter::Node) -> Option<tree_sitter::Node> {
    find_first_node(node, &["call_expression"])
}

/// Find the first `new_expression`.
#[cfg(test)]
pub(crate) fn find_new_expression(node: tree_sitter::Node) -> Option<tree_sitter::Node> {
    find_first_node(node, &["new_expression"])
}

/// Find the first `class_declaration`.
#[cfg(test)]
pub(crate) fn find_class_declaration(node: tree_sitter::Node) -> Option<tree_sitter::Node> {
    find_first_node(node, &["class_declaration"])
}

/// Find the first `interface_declaration`.
#[cfg(test)]
pub(crate) fn find_interface_declaration(node: tree_sitter::Node) -> Option<tree_sitter::Node> {
    find_first_node(node, &["interface_declaration"])
}

/// Find the first `member_expression`.
#[cfg(test)]
pub(crate) fn find_member_expression(node: tree_sitter::Node) -> Option<tree_sitter::Node> {
    find_first_node(node, &["member_expression"])
}

/// Find the first `variable_declaration` or `lexical_declaration`.
#[cfg(test)]
pub(crate) fn find_var_decl(node: tree_sitter::Node) -> Option<tree_sitter::Node> {
    find_first_node(node, &["variable_declaration", "lexical_declaration"])
}

/// Find the first `function_declaration`, `arrow_function`, or `lexical_declaration`.
#[cfg(test)]
pub(crate) fn find_function_decl(node: tree_sitter::Node) -> Option<tree_sitter::Node> {
    find_first_node(
        node,
        &[
            "function_declaration",
            "arrow_function",
            "lexical_declaration",
        ],
    )
}

// ── Shared test assertions ──────────────────────────────────────

/// Assert that `extract_jsx_component_invocation` produces the expected intent.
/// Used by both JavaScript and TypeScript test modules.
#[cfg(test)]
pub(crate) fn assert_jsx_component_invocation<F>(
    code: &str,
    parse: F,
    expected_method: &str,
    expected_receiver: Option<&str>,
) where
    F: Fn(&str) -> Result<Tree, String>,
{
    let tree = parse(code).expect("Failed to parse code");
    let jsx = find_jsx_element(tree.root_node()).expect("No JSX element found");
    let code_bytes = code.as_bytes();
    let mut intents: Vec<crate::models::CallIntent> = Vec::new();
    crate::pipeline::parser::languages::javascript::extract_jsx_component_invocation(
        jsx,
        code_bytes,
        &mut intents,
    );
    assert!(
        !intents.is_empty(),
        "Expected at least one intent for: {code}"
    );
    assert_eq!(intents[0].method, expected_method);
    assert_eq!(
        intents[0].receiver.as_deref(),
        expected_receiver,
        "Receiver mismatch for: {code}"
    );
}

/// Assert that a class/interface node has an `Extends` reference to the expected parent.
/// Works with both JS (`extract_class_inheritance_js`) and TS (`extract_class_inheritance`).
#[cfg(test)]
pub(crate) fn assert_extends(intents: &[ReferenceIntent], expected: &str) {
    let found = intents
        .iter()
        .any(|i| matches!(i, ReferenceIntent::Extends { parent, .. } if parent == expected));
    assert!(
        found,
        "Expected Extends -> '{}', got {:?}",
        expected, intents
    );
}

/// Assert that a class/interface node has an `Implements` reference to the expected interface.
#[cfg(test)]
pub(crate) fn assert_implements(intents: &[ReferenceIntent], expected: &str) {
    let found = intents.iter().any(
        |i| matches!(i, ReferenceIntent::Implements { interface, .. } if interface == expected),
    );
    assert!(
        found,
        "Expected Implements -> '{}', got {:?}",
        expected, intents
    );
}

/// Assert that `extract_class_inheritance_js` on the parsed code yields an `Extends` to `expected_parent`.
#[cfg(test)]
pub(crate) fn assert_js_class_inheritance<F>(code: &str, parse: F, expected_parent: &str)
where
    F: Fn(&str) -> Result<Tree, String>,
{
    let tree = parse(code).expect("Failed to parse code");
    let class_node = find_class_declaration(tree.root_node()).expect("No class_declaration found");
    let mut intents: Vec<ReferenceIntent> = Vec::new();
    crate::pipeline::parser::languages::javascript::extract_class_inheritance_js(
        class_node,
        code.as_bytes(),
        &mut intents,
    );
    assert!(
        !intents.is_empty(),
        "Expected at least one intent for: {code}"
    );
    assert_extends(&intents, expected_parent);
}

/// Assert that a JSX element extracted from parsed code has a single attribute matching `expected_attr` and `expected_value`.
#[cfg(test)]
pub(crate) fn assert_jsx_attribute<F>(
    code: &str,
    parse: F,
    expected_attr: &str,
    expected_value: &str,
) where
    F: Fn(&str) -> Result<Tree, String>,
{
    let tree = parse(code).expect("Failed to parse code");
    let jsx = find_jsx_opening_element(tree.root_node()).expect("No JSX opening element found");
    let attrs = crate::pipeline::parser::languages::javascript::extract_jsx_attributes(
        jsx,
        code.as_bytes(),
    );
    assert_eq!(attrs.len(), 1, "Expected 1 attribute for: {code}");
    assert_eq!(attrs[0].0, expected_attr, "Attr name mismatch for: {code}");
    assert_eq!(
        attrs[0].1, expected_value,
        "Attr value mismatch for: {code}"
    );
}

/// Assert that a JSX element has exactly `expected_count` attributes and that all `expected` (name, value) pairs are present.
#[cfg(test)]
pub(crate) fn assert_jsx_attributes_multi<F>(
    code: &str,
    parse: F,
    expected_count: usize,
    expected: &[(&str, &str)],
) where
    F: Fn(&str) -> Result<Tree, String>,
{
    let tree = parse(code).expect("Failed to parse code");
    let jsx = find_jsx_self_closing(tree.root_node()).expect("No JSX self-closing element found");
    let attrs = crate::pipeline::parser::languages::javascript::extract_jsx_attributes(
        jsx,
        code.as_bytes(),
    );
    assert_eq!(
        attrs.len(),
        expected_count,
        "Attribute count mismatch for: {code}"
    );
    for (name, value) in expected {
        let found = attrs.iter().any(|(n, v, _)| n == *name && v == *value);
        assert!(
            found,
            "Expected attribute {name}={value} not found in {:?} for: {code}",
            attrs
        );
    }
}
