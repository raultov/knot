use crate::models::EntityKind;
use tree_sitter::Node;

pub(crate) fn handle_markdown_capture(
    cap_name: &str,
    _text: &str,
    node: Node<'_>,
    source_bytes: &[u8],
) -> Option<(String, EntityKind, usize)> {
    let start_line = node.start_position().row + 1;

    match cap_name {
        "markdown.document.name" => Some((
            "Document".to_string(),
            EntityKind::MarkdownDocument,
            start_line,
        )),
        "markdown.section" => {
            let heading_text = section_heading_text(node, source_bytes)?;
            Some((heading_text, EntityKind::MarkdownSection, start_line))
        }
        _ => None,
    }
}

/// Walks a `section` node to find the text of its ATX heading.
///
/// Returns `None` if the section has no `atx_heading` child or if the
/// heading is missing its `heading_content` field.
fn section_heading_text(section: Node<'_>, source: &[u8]) -> Option<String> {
    let mut cursor = section.walk();
    for child in section.children(&mut cursor) {
        if child.kind() == "atx_heading" {
            let inline = child.child_by_field_name("heading_content")?;
            return inline.utf8_text(source).ok().map(str::to_string);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use crate::pipeline::parser::test_utils::parse_markdown_snippet;

    #[test]
    fn test_print_markdown_ast() {
        let code = "# Header 1\nSome text.\n## Header 2\nMore text.";
        let tree = parse_markdown_snippet(code).expect("Failed to parse");
        println!("{}", tree.root_node().to_sexp());
    }
}

#[cfg(test)]
mod section_heading_text_tests {
    use super::section_heading_text;
    use crate::pipeline::parser::test_utils::parse_markdown_snippet;
    use tree_sitter::Node;

    /// Find the first `section` node at or below `node` whose first ATX
    /// heading's text matches `heading_text`. Returns `None` if no such
    /// section exists in the subtree.
    fn find_section_by_heading<'a>(
        node: Node<'a>,
        source: &[u8],
        heading_text: &str,
    ) -> Option<Node<'a>> {
        if node.kind() == "section"
            && let Some(text) = section_heading_text(node, source)
            && text == heading_text
        {
            return Some(node);
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if let Some(found) = find_section_by_heading(child, source, heading_text) {
                return Some(found);
            }
        }
        None
    }

    /// Parse `code` and return the first `section` node found (depth-first).
    /// Returns `None` if no section exists in the tree.
    fn first_section_node<'tree>(tree: &'tree tree_sitter::Tree) -> Option<Node<'tree>> {
        fn walk<'a>(node: Node<'a>) -> Option<Node<'a>> {
            if node.kind() == "section" {
                return Some(node);
            }
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if let Some(s) = walk(child) {
                    return Some(s);
                }
            }
            None
        }
        walk(tree.root_node())
    }

    #[test]
    fn extracts_h1_heading_text() {
        let code = "# Hello World\n\nSome paragraph.\n";
        let tree = parse_markdown_snippet(code).expect("parse");
        let section = first_section_node(&tree).expect("section");
        assert_eq!(
            section_heading_text(section, code.as_bytes()),
            Some("Hello World".to_string()),
        );
    }

    #[test]
    fn extracts_h2_heading_text() {
        let code = "## Setup\n\nBody.\n";
        let tree = parse_markdown_snippet(code).expect("parse");
        let section = first_section_node(&tree).expect("section");
        assert_eq!(
            section_heading_text(section, code.as_bytes()),
            Some("Setup".to_string()),
        );
    }

    #[test]
    fn extracts_deepest_heading_correctly() {
        // tree-sitter-md nests sections, so a doc with H1 > H2 > H3 has
        // three nested sections. Make sure each section returns its OWN
        // heading text, not its parent's.
        let code = "# Top\n\n## Middle\n\n### Bottom\n\nBody.\n";
        let tree = parse_markdown_snippet(code).expect("parse");
        let source = code.as_bytes();

        let top = find_section_by_heading(tree.root_node(), source, "Top").expect("Top section");
        let middle =
            find_section_by_heading(tree.root_node(), source, "Middle").expect("Middle section");
        let bottom =
            find_section_by_heading(tree.root_node(), source, "Bottom").expect("Bottom section");

        assert_eq!(section_heading_text(top, source), Some("Top".to_string()),);
        assert_eq!(
            section_heading_text(middle, source),
            Some("Middle".to_string()),
        );
        assert_eq!(
            section_heading_text(bottom, source),
            Some("Bottom".to_string()),
        );
    }

    #[test]
    fn handles_heading_with_special_characters() {
        // The heading text can include punctuation, code spans, etc.
        // We don't try to strip Markdown — return whatever's inside `inline`.
        let code = "## What's New in v2.0?\n\nDetails.\n";
        let tree = parse_markdown_snippet(code).expect("parse");
        let section = first_section_node(&tree).expect("section");
        assert_eq!(
            section_heading_text(section, code.as_bytes()),
            Some("What's New in v2.0?".to_string()),
        );
    }

    #[test]
    fn returns_none_for_non_section_node() {
        // section_heading_text expects a `section` node. If we hand it
        // something else (e.g. the document root or a paragraph), the
        // walk over its children won't find an `atx_heading` and we
        // should get None.
        let code = "Just a paragraph, no heading.\n";
        let tree = parse_markdown_snippet(code).expect("parse");
        // Root is `document`, not `section`. Children are likely paragraphs.
        let root = tree.root_node();
        assert_eq!(section_heading_text(root, code.as_bytes()), None);
    }
}
