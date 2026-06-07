use crate::models::ReferenceIntent;
use tree_sitter::Node;

/// Extract the UTF-8 text of a Tree-sitter node.
pub(crate) fn node_text(node: Node<'_>, source: &[u8]) -> String {
    node.utf8_text(source).unwrap_or_default().trim().to_owned()
}

/// Check if a name starts with an uppercase letter (likely a class/type/component).
pub(crate) fn is_capitalized(name: &str) -> bool {
    name.chars().next().is_some_and(|ch| ch.is_uppercase())
}

/// Extract decorator references from JavaScript/TypeScript decorators (e.g., @Component, @NgModule).
///
/// Recursively searches for `decorator` nodes and extracts capitalized identifiers
/// (likely class/component names) as TypeReference intents.
///
/// Example:
/// ```typescript
/// @NgModule({
///   declarations: [AppComponent, UserComponent],
///   bootstrap: [AppComponent]
/// })
/// export class AppModule {}
/// ```
///
/// This will extract: AppComponent (twice), UserComponent
pub(crate) fn extract_decorator_references(
    node: Node<'_>,
    source: &[u8],
    intents: &mut Vec<ReferenceIntent>,
) {
    let line = node.start_position().row + 1;

    if node.kind() == "decorator" {
        extract_identifiers_from_decorator(node, source, intents, line);
    }

    let mut child = node.child(0);
    while let Some(c) = child {
        extract_decorator_references(c, source, intents);
        child = c.next_sibling();
    }
}

/// Extract capitalized identifiers from decorator arguments (likely class references).
pub(crate) fn extract_identifiers_from_decorator(
    decorator_node: Node<'_>,
    source: &[u8],
    intents: &mut Vec<ReferenceIntent>,
    line: usize,
) {
    let mut child = decorator_node.child(0);
    while let Some(c) = child {
        match c.kind() {
            "identifier" | "type_identifier" => {
                let name = node_text(c, source);
                if is_capitalized(&name) {
                    intents.push(ReferenceIntent::TypeReference {
                        type_name: name,
                        line,
                    });
                }
            }
            _ => {
                extract_identifiers_from_decorator(c, source, intents, line);
            }
        }
        child = c.next_sibling();
    }
}

/// Extract the content of a single-quoted string literal (strips surrounding quotes).
pub(crate) fn extract_single_quoted(s: &str) -> Option<String> {
    let s = s.trim();
    if let Some(inner) = s.strip_prefix('\'')
        && let Some(end) = inner.find('\'')
    {
        return Some(inner[..end].to_string());
    }
    None
}

/// Recursively extracts TypeReference intents from `type_identifier` AST nodes.
///
/// Used by Java, TypeScript, and Kotlin parsers to capture type annotations
/// (constructor parameters, method params, field types, return types).
pub(crate) fn extract_type_references(
    node: tree_sitter::Node<'_>,
    source: &[u8],
    intents: &mut Vec<ReferenceIntent>,
) {
    let line = node.start_position().row + 1;

    if node.kind() == "type_identifier" {
        let type_name = node_text(node, source);
        if is_capitalized(&type_name) {
            intents.push(ReferenceIntent::TypeReference { type_name, line });
        }
    }

    let mut child = node.child(0);
    while let Some(c) = child {
        extract_type_references(c, source, intents);
        child = c.next_sibling();
    }
}

/// Extract a CallIntent from a `new_expression` / `object_creation_expression` node.
/// Returns the constructor name or None if not found.
pub(crate) fn extract_new_expression_name(node: Node<'_>, source: &[u8]) -> Option<String> {
    let mut child = node.child(0);
    while let Some(c) = child {
        if c.kind() == "identifier" || c.kind() == "type_identifier" {
            return Some(node_text(c, source));
        }
        child = c.next_sibling();
    }
    None
}

/// Truncate a string to `max_len` characters, appending "..." if truncated.
pub(crate) fn truncate_string(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        format!("{}...", &s[..max_len.saturating_sub(3)])
    }
}

/// Find the parent node of a given kind by traversing up the AST.
pub(crate) fn find_parent_by_kind<'a>(mut node: Node<'a>, kind: &str) -> Option<Node<'a>> {
    while let Some(parent) = node.parent() {
        if parent.kind() == kind {
            return Some(parent);
        }
        node = parent;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_node_text_basic() {
        let code = "public class Test { }";
        let tree = crate::pipeline::parser::test_utils::parse_java_snippet(code)
            .expect("Failed to parse Java code");

        let root = tree.root_node();
        let text = node_text(root, code.as_bytes());
        // Root text should be the whole code
        assert!(!text.is_empty());
    }

    #[test]
    fn test_node_text_with_whitespace_trimmed() {
        let code = "  public void test()  \n  { }  ";
        let tree = crate::pipeline::parser::test_utils::parse_java_snippet(code)
            .expect("Failed to parse Java code");

        let root = tree.root_node();
        let text = node_text(root, code.as_bytes());
        // Whitespace should be trimmed
        assert!(!text.starts_with(' '));
        assert!(!text.ends_with(' '));
    }

    #[test]
    fn test_find_parent_by_kind_found() {
        let code = "public class Test { public void method() {} }";
        let tree = crate::pipeline::parser::test_utils::parse_java_snippet(code)
            .expect("Failed to parse Java code");
        let source = code.as_bytes();

        // Traverse the tree to find a method identifier
        fn find_identifier<'a>(node: Node<'a>, source: &[u8]) -> Option<Node<'a>> {
            if node.kind() == "identifier"
                && let Ok(text) = node.utf8_text(source)
                && text.contains("method")
            {
                return Some(node);
            }
            let mut i = 0u32;
            while let Some(child) = node.child(i) {
                if let Some(found) = find_identifier(child, source) {
                    return Some(found);
                }
                i += 1;
            }
            None
        }

        if let Some(identifier) = find_identifier(tree.root_node(), source) {
            let parent = find_parent_by_kind(identifier, "method_declaration");
            assert!(parent.is_some(), "Should find method_declaration parent");
        }
    }
}
