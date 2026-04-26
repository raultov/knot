use crate::models::{CallIntent, EntityKind, ReferenceIntent};
use crate::pipeline::parser::utils::node_text;
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
        "python.constant.name" => Some((text.to_string(), EntityKind::PythonConstant, start_line)),
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

pub(crate) fn extract_reference_intents_python(
    node: Node<'_>,
    source: &[u8],
    intents: &mut Vec<ReferenceIntent>,
) {
    let mut call_intents = Vec::new();
    extract_call_intents_python(node, source, &mut call_intents);
    for call in call_intents {
        intents.push(ReferenceIntent::Call {
            method: call.method,
            receiver: call.receiver,
            line: call.line,
        });
    }

    let mut import_intents = Vec::new();
    extract_import_intents_python(node, source, &mut import_intents);
    for import in import_intents {
        intents.push(import);
    }

    let mut value_ref_intents = Vec::new();
    extract_value_references_python(node, source, &mut value_ref_intents);
    for value_ref in value_ref_intents {
        intents.push(value_ref);
    }
}

pub(crate) fn extract_call_intents_python(
    node: Node<'_>,
    source: &[u8],
    intents: &mut Vec<CallIntent>,
) {
    if node.kind() == "call" {
        let line = node.start_position().row + 1;

        let function_node = node.child_by_field_name("function");

        if let Some(func) = function_node {
            match func.kind() {
                "identifier" => {
                    let method_name = node_text(func, source);
                    intents.push(CallIntent {
                        method: method_name,
                        receiver: None,
                        line,
                    });
                }
                "attribute" => {
                    let mut method_name: Option<String> = None;
                    let mut receiver: Option<String> = None;

                    if let Some(attr_node) = func.child_by_field_name("attribute") {
                        method_name = Some(node_text(attr_node, source));
                    }
                    if let Some(obj_node) = func.child_by_field_name("object") {
                        receiver = Some(node_text(obj_node, source));
                    }

                    if let Some(method) = method_name {
                        intents.push(CallIntent {
                            method,
                            receiver,
                            line,
                        });
                    }
                }
                _ => {}
            }
        }
    } else if node.kind() == "print_statement" {
        let line = node.start_position().row + 1;
        intents.push(CallIntent {
            method: "print".to_string(),
            receiver: None,
            line,
        });
    }

    let mut child = node.child(0);
    while let Some(c) = child {
        extract_call_intents_python(c, source, intents);
        child = c.next_sibling();
    }
}

pub(crate) fn extract_import_intents_python(
    node: Node<'_>,
    source: &[u8],
    intents: &mut Vec<ReferenceIntent>,
) {
    let line = node.start_position().row + 1;

    if node.kind() == "import_statement" {
        let mut child = node.child(0);
        while let Some(c) = child {
            if c.kind() == "dotted_name" || c.kind() == "identifier" {
                collect_import_names(c, source, intents, line);
            }
            child = c.next_sibling();
        }
    } else if node.kind() == "import_from_statement" {
        for i in 0..node.child_count() {
            if let Some(child) = node.child(i as u32) {
                match child.kind() {
                    "dotted_name" => {
                        let name = node_text(child, source);
                        intents.push(ReferenceIntent::TypeReference {
                            type_name: name,
                            line,
                        });
                    }
                    "aliased_import" => {
                        if let Some(alias_node) = child.child_by_field_name("alias") {
                            let alias_name = node_text(alias_node, source);
                            intents.push(ReferenceIntent::TypeReference {
                                type_name: alias_name,
                                line,
                            });
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    let mut child = node.child(0);
    while let Some(c) = child {
        extract_import_intents_python(c, source, intents);
        child = c.next_sibling();
    }
}

pub(crate) fn extract_value_references_python(
    node: Node<'_>,
    source: &[u8],
    intents: &mut Vec<ReferenceIntent>,
) {
    let line = node.start_position().row + 1;

    if node.kind() == "keyword_argument"
        && let Some(value_node) = node.child_by_field_name("value")
        && value_node.kind() == "identifier"
    {
        let value_name = node_text(value_node, source);
        if !is_python_reserved_value(&value_name) {
            intents.push(ReferenceIntent::ValueReference { value_name, line });
        }
    }

    let mut child = node.child(0);
    while let Some(c) = child {
        extract_value_references_python(c, source, intents);
        child = c.next_sibling();
    }
}

const PYTHON_RESERVED_VALUES: &[&str] = &["self", "cls"];

fn is_python_reserved_value(name: &str) -> bool {
    PYTHON_RESERVED_VALUES.contains(&name)
}

pub(crate) fn extract_inheritance_intents_python(
    entity_node: Node<'_>,
    source: &[u8],
    intents: &mut Vec<ReferenceIntent>,
) {
    if entity_node.kind() != "class_definition" {
        return;
    }

    let line = entity_node.start_position().row + 1;

    for i in 0..entity_node.child_count() {
        if let Some(child) = entity_node.child(i as u32)
            && child.kind() == "argument_list"
        {
            // Walk the argument_list to find parent class identifiers
            extract_superclass_names(child, source, intents, line);
        }
    }
}

fn extract_superclass_names(
    node: Node<'_>,
    source: &[u8],
    intents: &mut Vec<ReferenceIntent>,
    line: usize,
) {
    for i in 0..node.child_count() {
        if let Some(child) = node.child(i as u32) {
            if child.kind() == "identifier" {
                let parent_name = node_text(child, source);
                if !is_python_reserved_value(&parent_name) {
                    intents.push(ReferenceIntent::Extends {
                        parent: parent_name,
                        line,
                    });
                }
            }
            // Recurse into nested structures (e.g., expression_list)
            extract_superclass_names(child, source, intents, line);
        }
    }
}

pub(crate) fn extract_decorator_intents_python(
    entity_node: Node<'_>,
    source: &[u8],
    intents: &mut Vec<ReferenceIntent>,
) {
    // Check if this definition has a decorated_definition parent
    let parent = match entity_node.parent() {
        Some(p) if p.kind() == "decorated_definition" => p,
        _ => return,
    };

    let line = entity_node.start_position().row + 1;

    for i in 0..parent.child_count() {
        if let Some(child) = parent.child(i as u32)
            && child.kind() == "decorator"
        {
            extract_decorator_name(child, source, intents, line);
        }
    }
}

fn extract_decorator_name(
    decorator_node: Node<'_>,
    source: &[u8],
    intents: &mut Vec<ReferenceIntent>,
    line: usize,
) {
    // Case 1: @identifier (e.g., @staticmethod, @property, @dataclass)
    //   decorator → (identifier)
    // Case 2: @call(args) (e.g., @route("/path"), @app.get("/"))
    //   decorator → (call function: (identifier|attribute))
    for i in 0..decorator_node.child_count() {
        if let Some(child) = decorator_node.child(i as u32) {
            let method_name = match child.kind() {
                "identifier" => Some(node_text(child, source)),
                "attribute" => child
                    .child_by_field_name("attribute")
                    .map(|n| node_text(n, source)),
                "call" => {
                    // @decorator(args): extract function name from call
                    child
                        .child_by_field_name("function")
                        .and_then(|func| match func.kind() {
                            "identifier" => Some(node_text(func, source)),
                            "attribute" => func
                                .child_by_field_name("attribute")
                                .map(|n| node_text(n, source)),
                            _ => None,
                        })
                }
                _ => None,
            };

            if let Some(method) = method_name
                && !is_python_reserved_value(&method)
            {
                intents.push(ReferenceIntent::Call {
                    method,
                    receiver: None,
                    line,
                });
            }
        }
    }
}

fn collect_import_names(
    node: Node<'_>,
    source: &[u8],
    intents: &mut Vec<ReferenceIntent>,
    line: usize,
) {
    match node.kind() {
        "identifier" => {
            let name = node_text(node, source);
            intents.push(ReferenceIntent::TypeReference {
                type_name: name,
                line,
            });
        }
        "aliased_import" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                let name = node_text(name_node, source);
                intents.push(ReferenceIntent::TypeReference {
                    type_name: name,
                    line,
                });
            }
        }
        _ => {}
    }

    let mut child = node.child(0);
    while let Some(c) = child {
        collect_import_names(c, source, intents, line);
        child = c.next_sibling();
    }
}

pub(crate) fn extract_decorator_names_python(
    entity_node: Node<'_>,
    source: &[u8],
    names: &mut Vec<String>,
) {
    let parent = match entity_node.parent() {
        Some(p) if p.kind() == "decorated_definition" => p,
        _ => return,
    };

    for i in 0..parent.child_count() {
        if let Some(child) = parent.child(i as u32)
            && child.kind() == "decorator"
        {
            // Skip the leading '@' if present
            let mut start = child.start_byte();
            if start < source.len() && source[start] == b'@' {
                start += 1;
            }
            let end = child.end_byte().min(source.len());
            if start < end
                && let Ok(text) = std::str::from_utf8(&source[start..end])
            {
                names.push(format!("@{}", text.trim()));
            }
        }
    }
}
