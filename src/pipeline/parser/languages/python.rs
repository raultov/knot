use crate::models::EntityKind;
use tree_sitter::Node;

pub(crate) fn handle_python_capture(
    cap_name: &str,
    text: &str,
    node: Node<'_>,
) -> Option<(String, EntityKind, usize)> {
    let start_line = node.start_position().row + 1;

    match cap_name {
        "python.class.name" => Some((text.to_string(), EntityKind::PythonClass, start_line)),
        "python.function.name" => {
            let kind = if is_inside_class_body(node) {
                EntityKind::PythonMethod
            } else {
                EntityKind::PythonFunction
            };
            Some((text.to_string(), kind, start_line))
        }
        _ => None,
    }
}

pub(crate) fn is_inside_class_body(node: Node<'_>) -> bool {
    let mut current = node.parent();
    while let Some(parent) = current {
        if parent.kind() == "class_definition" {
            return true;
        }
        current = parent.parent();
    }
    false
}
