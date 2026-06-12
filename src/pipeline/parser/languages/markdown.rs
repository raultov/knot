use crate::models::EntityKind;
use tree_sitter::Node;

pub(crate) fn handle_markdown_capture(
    cap_name: &str,
    text: &str,
    node: Node<'_>,
) -> Option<(String, EntityKind, usize)> {
    let start_line = node.start_position().row + 1;

    match cap_name {
        "markdown.document.name" => Some(("Document".to_string(), EntityKind::MarkdownDocument, start_line)),
        "markdown.section.name" => Some((text.to_string(), EntityKind::MarkdownSection, start_line)),
        _ => None,
    }
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
