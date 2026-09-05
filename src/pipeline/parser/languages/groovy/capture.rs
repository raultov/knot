use crate::models::EntityKind;

pub(crate) fn handle_groovy_capture(
    capture_name: &str,
    text: &str,
    _node: tree_sitter::Node,
) -> Option<(String, EntityKind, usize)> {
    let line = _node.start_position().row + 1;
    match capture_name {
        "groovy.class.name" => Some((text.to_string(), EntityKind::GroovyClass, line)),
        "groovy.interface.name" => Some((text.to_string(), EntityKind::GroovyInterface, line)),
        "groovy.enum.name" => Some((text.to_string(), EntityKind::GroovyEnum, line)),
        "groovy.method.name" => Some((text.to_string(), EntityKind::GroovyMethod, line)),
        "groovy.field.name" => Some((text.to_string(), EntityKind::GroovyProperty, line)),
        _ => None,
    }
}
