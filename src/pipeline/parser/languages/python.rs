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
        "python.function.name" => Some((text.to_string(), EntityKind::PythonFunction, start_line)),
        "python.method.name" => Some((text.to_string(), EntityKind::PythonMethod, start_line)),
        _ => None,
    }
}
