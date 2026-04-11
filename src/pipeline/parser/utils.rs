use tree_sitter::Node;

/// Extract the UTF-8 text of a Tree-sitter node.
pub(crate) fn node_text(node: Node<'_>, source: &[u8]) -> String {
    node.utf8_text(source).unwrap_or_default().trim().to_owned()
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
        fn find_identifier<'a>(
            node: tree_sitter::Node<'a>,
            source: &[u8],
        ) -> Option<tree_sitter::Node<'a>> {
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
