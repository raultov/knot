use crate::models::{CallIntent, ReferenceIntent};
use crate::pipeline::parser::utils::{extract_new_expression_name, node_text};
use tree_sitter::Node;

pub(crate) use crate::pipeline::parser::utils::extract_type_references;

/// Recursively extract all call intents from Java.
pub(crate) fn collect_all_reference_intents_java(
    node: Node<'_>,
    source: &[u8],
    intents: &mut Vec<(ReferenceIntent, usize)>,
) {
    let byte_pos = node.start_byte();
    let line = node.start_position().row + 1;

    match node.kind() {
        "method_invocation" | "object_creation_expression" => {
            // Use non-recursive extraction to avoid double-processing children
            // (this function already handles recursion via the child loop below)
            let call_intents = extract_single_call_intent_java(node, source);
            for call in call_intents {
                intents.push((
                    ReferenceIntent::Call {
                        method: call.method,
                        receiver: call.receiver,
                        line,
                        arg_count: call.arg_count,
                    },
                    byte_pos,
                ));
            }
        }
        "marker_annotation" | "annotation" => {
            // Extract annotation references (e.g., @Component, @Autowired)
            let mut annotation_refs = Vec::new();
            extract_identifiers_from_annotation(node, source, &mut annotation_refs, line);
            for ref_intent in annotation_refs {
                intents.push((ref_intent, byte_pos));
            }
        }
        "type_identifier" => {
            // Extract type references (e.g., constructor parameters, field types)
            let type_name = node_text(node, source);
            // Only capture capitalized identifiers (likely classes/interfaces)
            if type_name.chars().next().is_some_and(|c| c.is_uppercase()) {
                intents.push((ReferenceIntent::TypeReference { type_name, line }, byte_pos));
            }
        }
        "import_declaration" => {
            collect_import_intents_java(node, source, intents, byte_pos, line);
        }
        _ => {}
    }

    // Recursively process children
    let mut child = node.child(0);
    while let Some(c) = child {
        collect_all_reference_intents_java(c, source, intents);
        child = c.next_sibling();
    }
}

fn collect_import_intents_java(
    node: Node<'_>,
    source: &[u8],
    intents: &mut Vec<(ReferenceIntent, usize)>,
    byte_pos: usize,
    line: usize,
) {
    let has_wildcard = node
        .children(&mut node.walk())
        .any(|c| c.kind() == "asterisk");
    if has_wildcard {
        return;
    }

    let is_static = node
        .children(&mut node.walk())
        .any(|c| c.kind() == "static");

    let mut idents: Vec<String> = Vec::new();
    fn collect_scoped_idents(n: Node<'_>, source: &[u8], out: &mut Vec<String>) {
        let mut child = n.child(0);
        while let Some(c) = child {
            if c.kind() == "identifier" {
                out.push(node_text(c, source));
            } else if c.kind() == "scoped_identifier" {
                collect_scoped_idents(c, source, out);
            }
            child = c.next_sibling();
        }
    }

    let mut child = node.child(0);
    while let Some(c) = child {
        if c.kind() == "scoped_identifier" {
            collect_scoped_idents(c, source, &mut idents);
            break;
        }
        child = c.next_sibling();
    }

    if idents.is_empty() {
        return;
    }

    if is_static && idents.len() >= 2 {
        let class_name = idents[idents.len() - 2].clone();
        let member_name = idents.last().unwrap().clone();
        if class_name.chars().next().is_some_and(|c| c.is_uppercase()) {
            intents.push((
                ReferenceIntent::TypeReference {
                    type_name: class_name,
                    line,
                },
                byte_pos,
            ));
        }
        intents.push((
            ReferenceIntent::ValueReference {
                value_name: member_name,
                line,
            },
            byte_pos,
        ));
    } else if let Some(name) = idents.last()
        && name.chars().next().is_some_and(|c| c.is_uppercase())
    {
        intents.push((
            ReferenceIntent::TypeReference {
                type_name: name.clone(),
                line,
            },
            byte_pos,
        ));
    }
}

/// Extract annotation references from Java annotations (e.g., @Component, @Autowired).
///
/// Recursively searches for `marker_annotation` and `annotation` nodes and extracts
/// capitalized identifiers (likely class/component names) as TypeReference intents.
///
/// Example:
/// ```java
/// @Configuration
/// @ComponentScan(basePackageClasses = {AppConfig.class, SecurityConfig.class})
/// public class AppModule {}
/// ```
///
/// This will extract: AppConfig, SecurityConfig
pub(crate) fn extract_annotation_references(
    node: Node<'_>,
    source: &[u8],
    intents: &mut Vec<ReferenceIntent>,
) {
    let line = node.start_position().row + 1;

    // If this is an annotation node, extract references from its arguments
    if matches!(node.kind(), "marker_annotation" | "annotation") {
        extract_identifiers_from_annotation(node, source, intents, line);
    }

    // Recursively process children
    let mut child = node.child(0);
    while let Some(c) = child {
        extract_annotation_references(c, source, intents);
        child = c.next_sibling();
    }
}

/// Extract capitalized identifiers from annotation arguments (likely class references).
fn extract_identifiers_from_annotation(
    annotation_node: Node<'_>,
    source: &[u8],
    intents: &mut Vec<ReferenceIntent>,
    line: usize,
) {
    // Recursively scan all children for identifiers
    let mut child = annotation_node.child(0);
    while let Some(c) = child {
        match c.kind() {
            "identifier" | "type_identifier" => {
                let name = node_text(c, source);
                // Only capture capitalized identifiers (likely classes/components)
                if name.chars().next().is_some_and(|ch| ch.is_uppercase()) {
                    intents.push(ReferenceIntent::TypeReference {
                        type_name: name,
                        line,
                    });
                }
            }
            _ => {
                // Recurse into nested structures (objects, arrays, etc.)
                extract_identifiers_from_annotation(c, source, intents, line);
            }
        }
        child = c.next_sibling();
    }
}

/// Extract reference intents from a Java method body (wrapper for backward compatibility).
pub(crate) fn extract_reference_intents_java(
    node: Node<'_>,
    source: &[u8],
    intents: &mut Vec<ReferenceIntent>,
) {
    let mut call_intents = Vec::new();
    extract_call_intents_java(node, source, &mut call_intents);
    for call in call_intents {
        intents.push(ReferenceIntent::Call {
            method: call.method,
            receiver: call.receiver,
            line: call.line,
            arg_count: call.arg_count,
        });
    }
}

/// Extract method invocation call intents from a Java method body.
pub(crate) fn extract_call_intents_java(
    node: Node<'_>,
    source: &[u8],
    intents: &mut Vec<CallIntent>,
) {
    intents.extend(extract_single_call_intent_java(node, source));

    // Recursively process children
    let mut child = node.child(0);
    while let Some(c) = child {
        extract_call_intents_java(c, source, intents);
        child = c.next_sibling();
    }
}

/// Extract call intents from a SINGLE Java node without recursive descent.
///
/// This is the non-recursive version of `extract_call_intents_java`,
/// designed to be used in contexts where the caller already handles tree traversal
/// (e.g., the fallback pass in `collect_all_reference_intents_java`).
///
/// By extracting only the current node's intent, we avoid double-processing children
/// that would cause duplicate CALLS with incorrect byte_pos/line attribution.
pub(crate) fn extract_single_call_intent_java(node: Node<'_>, source: &[u8]) -> Vec<CallIntent> {
    let mut intents = Vec::new();

    if node.kind() == "method_invocation" {
        let mut method_name: Option<String> = None;
        let mut receiver: Option<String> = None;
        let line = node.start_position().row + 1;

        // Parse method_invocation structure:
        // - Has optional receiver (identifier or "this")
        // - Has "." separator if receiver exists
        // - Has identifier for method name
        let mut child = node.child(0);
        let mut found_dot = false;
        while let Some(c) = child {
            let kind = c.kind();
            match kind {
                "identifier" | "field_access" => {
                    if found_dot {
                        // After a dot, this is the method name
                        method_name = Some(node_text(c, source));
                    } else if receiver.is_none() {
                        // Before a dot (or if no dot), could be receiver or method name
                        receiver = Some(node_text(c, source));
                    }
                }
                "this" => {
                    receiver = Some("this".to_string());
                }
                "." => {
                    found_dot = true;
                }
                _ => {}
            }
            child = c.next_sibling();
        }

        // If we found a dot, we know the last identifier is the method
        if found_dot {
            if let Some(method) = method_name {
                intents.push(CallIntent {
                    method,
                    receiver,
                    line,
                    arg_count: None,
                });
            }
        } else if let Some(method) = method_name {
            // No dot found, so receiver is actually a method name (local call)
            intents.push(CallIntent {
                method,
                receiver: None,
                line,
                arg_count: None,
            });
            // Revert receiver since it's not a receiver
        } else if let Some(receiver_val) = receiver {
            // Single identifier - treat as local call
            intents.push(CallIntent {
                method: receiver_val,
                receiver: None,
                line,
                arg_count: None,
            });
        }
    } else if node.kind() == "object_creation_expression" {
        let line = node.start_position().row + 1;
        if let Some(name) = extract_new_expression_name(node, source) {
            intents.push(CallIntent {
                method: name,
                receiver: None,
                line,
                arg_count: None,
            });
        }
    }

    // NO recursive child processing - that's the key difference!
    intents
}

pub(crate) fn extract_package_name(root: Node<'_>, source: &[u8]) -> Option<String> {
    let mut child = root.child(0);
    while let Some(c) = child {
        if c.kind() == "package_declaration" {
            let mut pkg_child = c.child(0);
            while let Some(pc) = pkg_child {
                if pc.kind() == "identifier" || pc.kind() == "scoped_identifier" {
                    return Some(node_text(pc, source));
                }
                pkg_child = pc.next_sibling();
            }
        }
        child = c.next_sibling();
    }
    None
}

pub(crate) fn extract_class_inheritance_java(
    class_node: Node<'_>,
    source: &[u8],
    intents: &mut Vec<ReferenceIntent>,
) {
    let line = class_node.start_position().row + 1;

    let mut child = class_node.child(0);
    while let Some(c) = child {
        match c.kind() {
            "superclass" => {
                extract_type_names_from_children(c, source, |name| {
                    intents.push(ReferenceIntent::Extends { parent: name, line });
                });
            }
            "super_interfaces" => {
                if let Some(type_list) = find_child_by_kind(c, "type_list") {
                    extract_type_names_from_children(type_list, source, |name| {
                        intents.push(ReferenceIntent::Implements {
                            interface: name,
                            line,
                        });
                    });
                }
            }
            "extends_interfaces" => {
                if let Some(type_list) = find_child_by_kind(c, "type_list") {
                    extract_type_names_from_children(type_list, source, |name| {
                        intents.push(ReferenceIntent::Extends { parent: name, line });
                    });
                }
            }
            _ => {}
        }
        child = c.next_sibling();
    }
}

fn extract_type_names_from_children(node: Node<'_>, source: &[u8], mut emit: impl FnMut(String)) {
    let mut child = node.child(0);
    while let Some(c) = child {
        match c.kind() {
            "type_identifier" => {
                emit(node_text(c, source));
            }
            "scoped_type_identifier" => {
                emit(node_text(c, source));
            }
            "generic_type" => {
                if let Some(inner) = c.child(0) {
                    emit(node_text(inner, source));
                }
            }
            _ => {}
        }
        child = c.next_sibling();
    }
}

fn find_child_by_kind<'a>(node: Node<'a>, kind: &str) -> Option<Node<'a>> {
    let mut child = node.child(0);
    while let Some(c) = child {
        if c.kind() == kind {
            return Some(c);
        }
        child = c.next_sibling();
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_single_call_intent_java_method_invocation() {
        let code = "void test() { obj.method(); }";
        let tree = crate::pipeline::parser::test_utils::parse_java_snippet(code)
            .expect("Failed to parse Java code");

        if let Some(invocation) = find_node_in_tree(tree.root_node(), "method_invocation") {
            let code_bytes = code.as_bytes();
            let intents = extract_single_call_intent_java(invocation, code_bytes);
            assert!(!intents.is_empty());
            assert_eq!(intents[0].method, "method");
            assert_eq!(intents[0].receiver, Some("obj".to_string()));
        }
    }

    #[test]
    fn test_extract_single_call_intent_java_this() {
        let code = "void test() { this.method(); }";
        let tree = crate::pipeline::parser::test_utils::parse_java_snippet(code)
            .expect("Failed to parse Java code");

        if let Some(invocation) = find_node_in_tree(tree.root_node(), "method_invocation") {
            let code_bytes = code.as_bytes();
            let intents = extract_single_call_intent_java(invocation, code_bytes);
            assert!(!intents.is_empty());
            assert_eq!(intents[0].method, "method");
            assert_eq!(intents[0].receiver, Some("this".to_string()));
        }
    }

    #[test]
    fn test_extract_call_intents_java_nested() {
        let code = "void test() { obj.method(other.call()); }";
        let tree = crate::pipeline::parser::test_utils::parse_java_snippet(code)
            .expect("Failed to parse Java code");

        let code_bytes = code.as_bytes();
        let mut intents: Vec<CallIntent> = Vec::new();
        extract_call_intents_java(tree.root_node(), code_bytes, &mut intents);

        // Should find both method and call
        assert!(intents.len() >= 2);
        assert!(intents.iter().any(|i| i.method == "method"));
        assert!(intents.iter().any(|i| i.method == "call"));
    }

    #[test]
    fn test_extract_call_intent_java_field_access() {
        let code = "void test() { this.chatMemory.add(foo); }";
        let tree = crate::pipeline::parser::test_utils::parse_java_snippet(code)
            .expect("Failed to parse Java code");

        let code_bytes = code.as_bytes();
        let mut intents: Vec<CallIntent> = Vec::new();
        extract_call_intents_java(tree.root_node(), code_bytes, &mut intents);

        assert!(!intents.is_empty());
        assert_eq!(intents[0].method, "add");
        assert_eq!(intents[0].receiver, Some("this.chatMemory".to_string()));
    }

    fn find_node_in_tree<'a>(node: Node<'a>, kind: &str) -> Option<Node<'a>> {
        if node.kind() == kind {
            return Some(node);
        }
        let mut i = 0u32;
        while let Some(child) = node.child(i) {
            if let Some(found) = find_node_in_tree(child, kind) {
                return Some(found);
            }
            i += 1;
        }
        None
    }

    #[test]
    fn test_extract_package_name() {
        let code = "package com.example.app;\n\nclass Foo {}";
        let tree = crate::pipeline::parser::test_utils::parse_java_snippet(code)
            .expect("Failed to parse Java code");
        let pkg = extract_package_name(tree.root_node(), code.as_bytes());
        assert_eq!(pkg, Some("com.example.app".to_string()));
    }

    #[test]
    fn test_extract_package_name_none() {
        let code = "class Foo {}";
        let tree = crate::pipeline::parser::test_utils::parse_java_snippet(code)
            .expect("Failed to parse Java code");
        let pkg = extract_package_name(tree.root_node(), code.as_bytes());
        assert_eq!(pkg, None);
    }

    #[test]
    fn test_extract_class_inheritance_extends() {
        let code = "class Child extends Parent {}";
        let tree = crate::pipeline::parser::test_utils::parse_java_snippet(code)
            .expect("Failed to parse Java code");
        let class_node = find_node_in_tree(tree.root_node(), "class_declaration").unwrap();
        let mut intents = Vec::new();
        extract_class_inheritance_java(class_node, code.as_bytes(), &mut intents);
        assert_eq!(intents.len(), 1);
        assert!(
            matches!(&intents[0], ReferenceIntent::Extends { parent, .. } if parent == "Parent")
        );
    }

    #[test]
    fn test_extract_class_inheritance_implements() {
        let code = "class Foo implements Bar, Baz {}";
        let tree = crate::pipeline::parser::test_utils::parse_java_snippet(code)
            .expect("Failed to parse Java code");
        let class_node = find_node_in_tree(tree.root_node(), "class_declaration").unwrap();
        let mut intents = Vec::new();
        extract_class_inheritance_java(class_node, code.as_bytes(), &mut intents);
        assert_eq!(intents.len(), 2);
        assert!(
            matches!(&intents[0], ReferenceIntent::Implements { interface, .. } if interface == "Bar")
        );
        assert!(
            matches!(&intents[1], ReferenceIntent::Implements { interface, .. } if interface == "Baz")
        );
    }

    #[test]
    fn test_extract_class_inheritance_generic_stripping() {
        let code = "class Repo implements Repository<User> {}";
        let tree = crate::pipeline::parser::test_utils::parse_java_snippet(code)
            .expect("Failed to parse Java code");
        let class_node = find_node_in_tree(tree.root_node(), "class_declaration").unwrap();
        let mut intents = Vec::new();
        extract_class_inheritance_java(class_node, code.as_bytes(), &mut intents);
        assert_eq!(intents.len(), 1);
        assert!(
            matches!(&intents[0], ReferenceIntent::Implements { interface, .. } if interface == "Repository")
        );
    }

    #[test]
    fn test_extract_interface_extends() {
        let code = "interface Child extends Parent {}";
        let tree = crate::pipeline::parser::test_utils::parse_java_snippet(code)
            .expect("Failed to parse Java code");
        let class_node = find_node_in_tree(tree.root_node(), "interface_declaration").unwrap();
        let mut intents = Vec::new();
        extract_class_inheritance_java(class_node, code.as_bytes(), &mut intents);
        assert_eq!(intents.len(), 1);
        assert!(
            matches!(&intents[0], ReferenceIntent::Extends { parent, .. } if parent == "Parent")
        );
    }

    #[test]
    fn test_extract_class_inheritance_extends_and_implements() {
        let code = "class Admin extends User implements Serializable, Comparable<Admin> {}";
        let tree = crate::pipeline::parser::test_utils::parse_java_snippet(code)
            .expect("Failed to parse Java code");
        let class_node = find_node_in_tree(tree.root_node(), "class_declaration").unwrap();
        let mut intents = Vec::new();
        extract_class_inheritance_java(class_node, code.as_bytes(), &mut intents);
        assert_eq!(intents.len(), 3);
        let extends = crate::pipeline::parser::test_utils::collect_extends(&intents);
        let implements = crate::pipeline::parser::test_utils::collect_implements(&intents);
        assert_eq!(extends, ["User"]);
        assert!(implements.contains(&"Serializable"));
        assert!(implements.contains(&"Comparable"));
    }

    #[test]
    fn test_extract_class_inheritance_no_inheritance() {
        let code = "class Simple {}";
        let tree = crate::pipeline::parser::test_utils::parse_java_snippet(code)
            .expect("Failed to parse Java code");
        let class_node = find_node_in_tree(tree.root_node(), "class_declaration").unwrap();
        let mut intents = Vec::new();
        extract_class_inheritance_java(class_node, code.as_bytes(), &mut intents);
        assert!(intents.is_empty());
    }

    #[test]
    fn test_import_class_emits_type_reference() {
        let code = "import com.example.Foo;\n\nclass Bar {}";
        let tree = crate::pipeline::parser::test_utils::parse_java_snippet(code)
            .expect("Failed to parse Java code");
        let mut intents = Vec::new();
        collect_all_reference_intents_java(tree.root_node(), code.as_bytes(), &mut intents);
        let has_foo = intents.iter().any(|(i, _)| match i {
            ReferenceIntent::TypeReference { type_name, .. } => type_name == "Foo",
            _ => false,
        });
        assert!(
            has_foo,
            "Should emit TypeReference for Foo from import, got: {:?}",
            intents
        );
    }

    #[test]
    fn test_import_static_emits_type_and_value_reference() {
        let code = "import static com.example.Util.helper;\n\nclass Bar {}";
        let tree = crate::pipeline::parser::test_utils::parse_java_snippet(code)
            .expect("Failed to parse Java code");
        let mut intents = Vec::new();
        collect_all_reference_intents_java(tree.root_node(), code.as_bytes(), &mut intents);
        let has_util = intents.iter().any(|(i, _)| match i {
            ReferenceIntent::TypeReference { type_name, .. } => type_name == "Util",
            _ => false,
        });
        let has_helper = intents.iter().any(|(i, _)| match i {
            ReferenceIntent::ValueReference { value_name, .. } => value_name == "helper",
            _ => false,
        });
        assert!(
            has_util,
            "Should emit TypeReference for Util from static import, got: {:?}",
            intents
        );
        assert!(
            has_helper,
            "Should emit ValueReference for helper from static import, got: {:?}",
            intents
        );
    }

    #[test]
    fn test_import_wildcard_ignored() {
        let code = "import com.example.*;\n\nclass Bar {}";
        let tree = crate::pipeline::parser::test_utils::parse_java_snippet(code)
            .expect("Failed to parse Java code");
        let mut intents = Vec::new();
        collect_all_reference_intents_java(tree.root_node(), code.as_bytes(), &mut intents);
        let import_refs: Vec<_> = intents
            .iter()
            .filter(|(i, _)| matches!(i, ReferenceIntent::TypeReference { .. }))
            .collect();
        assert!(
            import_refs.is_empty(),
            "Wildcard import should emit no type references, got: {:?}",
            import_refs
        );
    }
}
