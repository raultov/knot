use crate::models::{CallIntent, ReferenceIntent};
use crate::pipeline::parser::utils::node_text;
use tree_sitter::Node;

/// Recursively builds the FQN for a C++ node by traversing its parents.
/// If it's inside a `class_specifier`, it prepends `ClassName::`.
/// If it's inside a `namespace_definition`, it prepends `NamespaceName::`.
pub(crate) fn build_cpp_fqn(node: Node<'_>, source: &[u8]) -> Option<String> {
    let mut parts = Vec::new();
    let mut current = node.parent();

    while let Some(parent) = current {
        match parent.kind() {
            "class_specifier" | "struct_specifier" => {
                if let Some(name_node) = parent.child_by_field_name("name")
                    && let Ok(name) = std::str::from_utf8(&source[name_node.byte_range()])
                {
                    parts.push(name.trim().to_string());
                }
            }
            "namespace_definition" => {
                if let Some(name_node) = parent.child_by_field_name("name")
                    && let Ok(name) = std::str::from_utf8(&source[name_node.byte_range()])
                {
                    parts.push(name.trim().to_string());
                }
            }
            _ => {}
        }
        current = parent.parent();
    }

    if parts.is_empty() {
        None
    } else {
        parts.reverse();
        Some(parts.join("::"))
    }
}

pub(crate) fn extract_reference_intents_cpp(
    node: Node<'_>,
    source: &[u8],
    intents: &mut Vec<ReferenceIntent>,
) {
    let mut call_intents = Vec::new();
    extract_call_intents_cpp(node, source, &mut call_intents);
    for call in call_intents {
        intents.push(ReferenceIntent::Call {
            method: call.method,
            receiver: call.receiver,
            line: call.line,
        });
    }
}

pub(crate) fn extract_call_intents_cpp(
    node: Node<'_>,
    source: &[u8],
    intents: &mut Vec<CallIntent>,
) {
    if node.kind() == "call_expression" {
        let line = node.start_position().row + 1;
        // In C++, the function being called is usually the first child of call_expression
        if let Some(function_node) = node.child_by_field_name("function") {
            let kind = function_node.kind();
            if kind == "identifier" || kind == "field_identifier" {
                // Direct call or simple field call: foo()
                intents.push(CallIntent {
                    method: node_text(function_node, source),
                    receiver: None,
                    line,
                });
            } else if kind == "field_expression" {
                // Object/Pointer access: obj.foo() or ptr->foo()
                // A field_expression typically has an "argument" (receiver) and a "field" (method name)
                let mut receiver = None;
                let mut method = None;

                let mut child = function_node.child(0);
                while let Some(c) = child {
                    if c.kind() == "identifier" || c.kind() == "this" {
                        if receiver.is_none() {
                            receiver = Some(node_text(c, source));
                        } else if method.is_none() {
                            method = Some(node_text(c, source));
                        }
                    } else if c.kind() == "field_identifier" {
                        method = Some(node_text(c, source));
                    }
                    child = c.next_sibling();
                }

                if let Some(m) = method {
                    intents.push(CallIntent {
                        method: m,
                        receiver,
                        line,
                    });
                }
            } else if kind == "qualified_identifier" {
                // Scope resolution: std::vector::size()
                // The text of this node is typically "std::vector::size"
                let text = node_text(function_node, source);
                let mut parts: Vec<&str> = text.split("::").collect();

                if !parts.is_empty() {
                    let method = parts.pop().unwrap().to_string();
                    let receiver = if parts.is_empty() {
                        None
                    } else {
                        Some(parts.join("::"))
                    };
                    intents.push(CallIntent {
                        method,
                        receiver,
                        line,
                    });
                }
            }
        }
    } else if node.kind() == "identifier" {
        let text = node_text(node, source);
        // Heuristic: if it's all uppercase and has at least one character, it's likely a macro usage or constant.
        if text
            .chars()
            .all(|c| c.is_uppercase() || c == '_' || c.is_numeric())
            && text.chars().any(|c| c.is_alphabetic())
        {
            intents.push(CallIntent {
                method: text,
                receiver: None,
                line: node.start_position().row + 1,
            });
        }
    } else if node.kind() == "preproc_include" {
        if let Some(path_node) = node.child_by_field_name("path") {
            let mut path_str = node_text(path_node, source);
            if path_str.starts_with('"') || path_str.starts_with('<') {
                path_str = path_str[1..path_str.len() - 1].to_string();
            }
            intents.push(CallIntent {
                method: path_str,
                receiver: None,
                line: node.start_position().row + 1,
            });
        }
    } else if node.kind() == "type_identifier" {
        // Type references: class/struct usage in declarations, new expressions, etc.
        // Check if this is part of a qualified_identifier to get full scope
        if let Some(parent) = node.parent() {
            if parent.kind() == "qualified_identifier" {
                // This is part of Engine::MyClass - handle at parent level
                let text = node_text(parent, source);
                let mut parts: Vec<&str> = text.split("::").collect();

                if !parts.is_empty() {
                    let type_name = parts.pop().unwrap().to_string();
                    let receiver = if parts.is_empty() {
                        None
                    } else {
                        Some(parts.join("::"))
                    };
                    intents.push(CallIntent {
                        method: type_name,
                        receiver,
                        line: node.start_position().row + 1,
                    });
                }
                // Don't recurse into qualified_identifier children to avoid duplicates
                return;
            } else {
                // Simple type reference without namespace qualification
                intents.push(CallIntent {
                    method: node_text(node, source),
                    receiver: None,
                    line: node.start_position().row + 1,
                });
            }
        }
    }

    // Recursively process children
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        extract_call_intents_cpp(child, source, intents);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tree_sitter::Parser;

    fn get_parser() -> Parser {
        let mut parser = Parser::new();
        parser
            .set_language(&tree_sitter_cpp::LANGUAGE.into())
            .unwrap();
        parser
    }

    #[test]
    fn test_cpp_fqn_resolution_nested_namespaces() {
        let source = r#"
            namespace Engine {
                namespace Physics {
                    class Body {
                        void update() {}
                    };
                }
            }
        "#;
        let mut parser = get_parser();
        let tree = parser.parse(source, None).unwrap();
        let root = tree.root_node();

        // Find the `update` method node
        let mut update_node = None;
        let mut cursor = root.walk();
        let mut stack = vec![root];
        while let Some(node) = stack.pop() {
            if node.kind() == "function_definition" {
                update_node = Some(node);
                break;
            }
            for child in node.children(&mut cursor) {
                stack.push(child);
            }
        }

        let update_node = update_node.expect("update method not found");
        let fqn = build_cpp_fqn(update_node, source.as_bytes()).unwrap();
        assert_eq!(fqn, "Engine::Physics::Body");
    }

    #[test]
    fn test_cpp_reference_intents_pointers_and_namespaces() {
        let source = r#"
            void test() {
                foo();
                obj.bar();
                ptr->baz();
                std::vector::size();
                this->compute();
            }
        "#;
        let mut parser = get_parser();
        let tree = parser.parse(source, None).unwrap();
        let root = tree.root_node();

        let mut intents = Vec::new();
        extract_reference_intents_cpp(root, source.as_bytes(), &mut intents);

        assert_eq!(intents.len(), 5);

        // Check foo()
        let call_foo = intents
            .iter()
            .find(|i| matches!(i, ReferenceIntent::Call { method, .. } if method == "foo"))
            .unwrap();
        if let ReferenceIntent::Call { receiver, line, .. } = call_foo {
            assert_eq!(*receiver, None);
            assert_eq!(*line, 3);
        }

        // Check obj.bar()
        let call_bar = intents
            .iter()
            .find(|i| matches!(i, ReferenceIntent::Call { method, .. } if method == "bar"))
            .unwrap();
        if let ReferenceIntent::Call { receiver, line, .. } = call_bar {
            assert_eq!(*receiver, Some("obj".to_string()));
            assert_eq!(*line, 4);
        }

        // Check ptr->baz()
        let call_baz = intents
            .iter()
            .find(|i| matches!(i, ReferenceIntent::Call { method, .. } if method == "baz"))
            .unwrap();
        if let ReferenceIntent::Call { receiver, line, .. } = call_baz {
            assert_eq!(*receiver, Some("ptr".to_string()));
            assert_eq!(*line, 5);
        }

        // Check std::vector::size()
        let call_size = intents
            .iter()
            .find(|i| matches!(i, ReferenceIntent::Call { method, .. } if method == "size"))
            .unwrap();
        if let ReferenceIntent::Call { receiver, line, .. } = call_size {
            assert_eq!(*receiver, Some("std::vector".to_string()));
            assert_eq!(*line, 6);
        }

        // Check this->compute()
        let call_compute = intents
            .iter()
            .find(|i| matches!(i, ReferenceIntent::Call { method, .. } if method == "compute"))
            .unwrap();
        if let ReferenceIntent::Call { receiver, line, .. } = call_compute {
            assert_eq!(*receiver, Some("this".to_string()));
            assert_eq!(*line, 7);
        }
    }
}
