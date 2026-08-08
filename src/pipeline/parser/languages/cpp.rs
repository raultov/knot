use crate::models::{CallIntent, ReferenceIntent};
use crate::pipeline::parser::utils::node_text;
use tree_sitter::Node;

fn node_name(node: Node<'_>, source: &[u8]) -> Option<String> {
    let name_node = node.child_by_field_name("name")?;
    let name = std::str::from_utf8(&source[name_node.byte_range()]).ok()?;
    Some(name.trim().to_string())
}

/// Recursively builds the FQN for a C++ node by traversing its parents.
/// If it's inside a `class_specifier`, it prepends `ClassName::`.
/// If it's inside a `namespace_definition`, it prepends `NamespaceName::`.
/// If inside a `qualified_identifier`, extracts the scope (ClassName::method).
pub(crate) fn build_cpp_fqn(node: Node<'_>, source: &[u8]) -> Option<String> {
    let mut parts = Vec::new();

    let mut current = node.parent();

    while let Some(parent) = current {
        match parent.kind() {
            "class_specifier" | "struct_specifier" | "namespace_definition" => {
                if let Some(name) = node_name(parent, source) {
                    parts.push(name);
                }
            }
            _ => {}
        }
        current = parent.parent();
    }

    if parts.is_empty() {
        if let Some(qi_fqn) = find_qualified_identifier_in_descendants(node, source) {
            return Some(qi_fqn);
        }
        None
    } else {
        parts.reverse();
        Some(parts.join("::"))
    }
}

fn find_qualified_identifier_in_descendants(node: Node<'_>, source: &[u8]) -> Option<String> {
    fn walk(node: Node<'_>, source: &[u8], parts: &mut Vec<String>) -> bool {
        if node.kind() == "qualified_identifier"
            && let Some(scope_node) = node.child_by_field_name("scope")
            && let Ok(scope) = std::str::from_utf8(&source[scope_node.byte_range()])
        {
            parts.push(scope.trim().to_string());
            return true;
        }
        for i in 0..node.child_count() {
            if let Some(child) = node.child(i as u32)
                && walk(child, source, parts)
            {
                return true;
            }
        }
        false
    }
    let mut parts = Vec::new();
    if walk(node, source, &mut parts) && !parts.is_empty() {
        Some(parts.join("::"))
    } else {
        None
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
            arg_count: call.arg_count,
        });
    }
}

#[expect(
    clippy::too_many_lines,
    reason = "function is verbose but correct — extraction deferred"
)]
#[expect(
    clippy::cognitive_complexity,
    reason = "function is verbose but correct — extraction deferred"
)]
#[expect(
    clippy::excessive_nesting,
    reason = "function is verbose but correct — extraction deferred"
)]
pub(crate) fn extract_call_intents_cpp(
    node: Node<'_>,
    source: &[u8],
    intents: &mut Vec<CallIntent>,
) {
    if node.kind() == "call_expression" {
        let line = node.start_position().row + 1;

        let arg_count = node.child_by_field_name("arguments").map(|args_node| {
            let mut count = 0;
            let mut child = args_node.child(0);
            while let Some(c) = child {
                if c.is_named() {
                    count += 1;
                }
                child = c.next_sibling();
            }
            count
        });

        // In C++, the function being called is usually the first child of call_expression
        if let Some(function_node) = node.child_by_field_name("function") {
            let kind = function_node.kind();
            if kind == "identifier" || kind == "field_identifier" {
                // Direct call or simple field call: foo()
                intents.push(CallIntent {
                    method: node_text(function_node, source),
                    receiver: None,
                    line,
                    arg_count,
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
                        arg_count,
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
                        arg_count,
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
                arg_count: None,
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
                arg_count: None,
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
                        arg_count: None,
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
                    arg_count: None,
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

pub(crate) fn extract_cpp_signature(entity_node: Node<'_>, source: &[u8]) -> Option<String> {
    fn find_func_declarator(node: Node<'_>, depth: u32) -> Option<Node<'_>> {
        if depth > 4 {
            return None;
        }
        if node.kind() == "function_declarator" {
            return Some(node);
        }
        for i in 0..node.child_count() {
            if let Some(child) = node.child(i as u32)
                && let Some(result) = find_func_declarator(child, depth + 1)
            {
                return Some(result);
            }
        }
        None
    }
    find_func_declarator(entity_node, 0).map(|n| node_text(n, source))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::RelationshipType;
    use tree_sitter::Parser;
    use tree_sitter::StreamingIterator;

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

    fn collect_query_captures(
        source: &str,
        language: tree_sitter::Language,
        query_source: &str,
        capture_name: &str,
    ) -> Vec<String> {
        let mut parser = Parser::new();
        parser.set_language(&language).unwrap();
        let tree = parser.parse(source, None).unwrap();
        let query = tree_sitter::Query::new(&language, query_source).unwrap();
        let mut cursor = tree_sitter::QueryCursor::new();
        let mut matches = cursor.matches(&query, tree.root_node(), source.as_bytes());
        let mut names = Vec::new();
        while let Some(m) = {
            matches.advance();
            matches.get()
        } {
            for capture in m.captures {
                if query.capture_names()[capture.index as usize] == capture_name {
                    names.push(node_text(capture.node, source.as_bytes()));
                }
            }
        }
        names
    }

    fn calls_print_relationship(
        entity: &crate::models::ResolutionEntity,
        resolution_entities: &[crate::models::ResolutionEntity],
    ) -> bool {
        entity.relationships.iter().any(|(uuid, rel_type)| {
            *rel_type == RelationshipType::Calls
                && resolution_entities
                    .iter()
                    .any(|target| target.uuid == *uuid && target.name == "print")
        })
    }

    #[test]
    fn test_cpp_operator_overload_and_ref_return() {
        let source = r#"class String {
    String & copy(const char *pstr, unsigned int length) { return *this; }
    String & operator =(const char *pstr) { if (pstr) copy(pstr, 10); return *this; }
};"#;

        let entity_names = collect_query_captures(
            source,
            tree_sitter_cpp::LANGUAGE.into(),
            include_str!("../../../../queries/cpp.scm"),
            "cpp_method.name",
        );

        assert!(
            entity_names.contains(&"copy".to_string()),
            "Expected 'copy' in entities, got: {:?}",
            entity_names
        );
        assert!(
            entity_names.contains(&"operator =".to_string()),
            "Expected 'operator =' in entities, got: {:?}",
            entity_names
        );
    }

    #[test]
    fn test_cpp_pointer_return_and_declarations() {
        // pointer return type, declaration (no body), field_declaration
        let source = r#"
            class A {
            public:
                int * getPtr() { return nullptr; }
                void start();
                long & getRef();
            };
            void start() {}
            long & getRef() { long x = 0; return x; }
        "#;

        let entity_names = collect_query_captures(
            source,
            tree_sitter_cpp::LANGUAGE.into(),
            include_str!("../../../../queries/cpp.scm"),
            "cpp_method.name",
        );

        // getPtr has pointer return (int*)
        assert!(
            entity_names.contains(&"getPtr".to_string()),
            "Expected 'getPtr', got: {:?}",
            entity_names
        );
        // start is a declaration (no body)
        assert!(
            entity_names.contains(&"start".to_string()),
            "Expected 'start', got: {:?}",
            entity_names
        );
        // getRef has reference return (long&)
        assert!(
            entity_names.contains(&"getRef".to_string()),
            "Expected 'getRef', got: {:?}",
            entity_names
        );
    }

    #[test]
    fn test_c_pointer_return() {
        // test c.scm patterns — C has no references, only pointers
        let source = r#"
            int * getPtr() { return 0; }
            void doWork() {}
        "#;

        let entity_names = collect_query_captures(
            source,
            tree_sitter_c::LANGUAGE.into(),
            include_str!("../../../../queries/c.scm"),
            "c_function.name",
        );

        assert!(
            entity_names.contains(&"getPtr".to_string()),
            "Expected 'getPtr', got: {:?}",
            entity_names
        );
        assert!(
            entity_names.contains(&"doWork".to_string()),
            "Expected 'doWork', got: {:?}",
            entity_names
        );
    }

    #[test]
    #[expect(
        clippy::cognitive_complexity,
        reason = "function is verbose but correct — extraction deferred"
    )]
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

    #[test]
    #[expect(
        clippy::too_many_lines,
        reason = "function is verbose but correct — extraction deferred"
    )]
    fn test_cpp_print_println_internal_calls() {
        use crate::models::ResolutionEntity;
        use crate::pipeline::parser::extractor::extract_entities;
        use std::collections::HashMap;

        let source = r#"
size_t Print::print(const char str[]) {
    return write(str);
}

size_t Print::print(char c) {
    return write(c);
}

size_t Print::print(int n, int base) {
    return 0;
}

size_t Print::println(const char c[]) {
    size_t n = print(c);
    n += println();
    return n;
}

size_t Print::println(char c) {
    size_t n = print(c);
    n += println();
    return n;
}

size_t Print::println(int num, int base) {
    size_t n = print(num, base);
    n += println();
    return n;
}

size_t Print::println(void) {
    return print("\r\n");
}
"#;

        let query_source = include_str!("../../../../queries/cpp.scm");

        let entities = extract_entities(
            source,
            tree_sitter_cpp::LANGUAGE.into(),
            query_source,
            "cpp",
            "src/Print.cpp",
            "test_repo",
        )
        .expect("Failed to extract entities from Print.cpp");

        let print_entities: Vec<_> = entities.iter().filter(|e| e.name == "print").collect();
        let println_entities: Vec<_> = entities.iter().filter(|e| e.name == "println").collect();

        assert!(
            print_entities.len() >= 3,
            "Expected at least 3 print overloads, got {}",
            print_entities.len()
        );
        assert!(
            println_entities.len() >= 4,
            "Expected at least 4 println overloads, got {}",
            println_entities.len()
        );

        for entity in &print_entities {
            assert_eq!(
                entity.fqn, "Print::print",
                "print FQN should be Print::print, got {}",
                entity.fqn
            );
            assert_eq!(entity.enclosing_class.as_deref(), Some("Print"));
        }
        for entity in &println_entities {
            assert_eq!(
                entity.fqn, "Print::println",
                "println FQN should be Print::println, got {}",
                entity.fqn
            );
            assert_eq!(entity.enclosing_class.as_deref(), Some("Print"));
        }

        let println_char = println_entities
            .iter()
            .find(|e| e.signature.as_deref().is_some_and(|s| s.contains("char c")))
            .expect("println(char) not found");

        let has_print_call = println_char.reference_intents.iter().any(|intent| {
            matches!(intent, ReferenceIntent::Call { method, receiver, .. }
                if method == "print" && receiver.is_none())
        });
        assert!(
            has_print_call,
            "println(char) should have a Call intent to print, intents: {:?}",
            println_char.reference_intents
        );

        let mut resolution_entities: Vec<ResolutionEntity> =
            entities.iter().map(|e| e.into()).collect();

        crate::pipeline::ingest::resolve_reference_intents_with_context(
            &mut resolution_entities,
            HashMap::new(),
            HashMap::new(),
        );

        let println_char_res = resolution_entities
            .iter()
            .find(|e| {
                e.name == "println" && e.signature.as_deref().is_some_and(|s| s.contains("char c"))
            })
            .expect("println(char) resolution entity not found");

        let calls_print = calls_print_relationship(println_char_res, &resolution_entities);
        assert!(
            calls_print,
            "println(char) should CALL print, relationships: {:?}",
            println_char_res.relationships
        );

        let println_void_res = resolution_entities
            .iter()
            .find(|e| {
                e.name == "println"
                    && e.signature
                        .as_deref()
                        .is_some_and(|s| s.contains("void") || !s.contains(','))
            })
            .or_else(|| {
                resolution_entities.iter().find(|e| {
                    e.name == "println"
                        && e.signature
                            .as_deref()
                            .is_some_and(|s| s.contains("(void)") || s.contains("()"))
                })
            });

        if let Some(pv) = println_void_res {
            let calls_print = calls_print_relationship(pv, &resolution_entities);
            assert!(
                calls_print,
                "println(void) should CALL print, relationships: {:?}",
                pv.relationships
            );
        }
    }
}
