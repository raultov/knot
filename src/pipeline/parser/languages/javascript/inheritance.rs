use crate::models::ReferenceIntent;
use crate::pipeline::parser::utils::node_text;
use tree_sitter::Node;

/// Extract class inheritance (extends clause) from JavaScript class AST.
/// JavaScript doesn't have implements, so we only handle extends.
/// Navigates through the `class_heritage` wrapper to find the parent class expression.
pub(crate) fn extract_class_inheritance_js(
    class_node: Node<'_>,
    source: &[u8],
    intents: &mut Vec<ReferenceIntent>,
) {
    let line = class_node.start_position().row + 1;

    let mut child = class_node.child(0);
    while let Some(c) = child {
        if c.kind() == "class_heritage" {
            let parent_name = extract_js_heritage_name(c, source);
            if let Some(name) = parent_name {
                intents.push(ReferenceIntent::Extends { parent: name, line });
            }
        }
        child = c.next_sibling();
    }
}

fn extract_js_heritage_name(node: Node<'_>, source: &[u8]) -> Option<String> {
    let mut child = node.child(0);
    while let Some(c) = child {
        match c.kind() {
            "identifier" => return Some(node_text(c, source)),
            "member_expression" => {
                return node_text(c, source)
                    .split('.')
                    .next_back()
                    .map(|s| s.to_string());
            }
            _ => {
                if let Some(name) = extract_js_heritage_name(c, source) {
                    return Some(name);
                }
            }
        }
        child = c.next_sibling();
    }
    None
}
