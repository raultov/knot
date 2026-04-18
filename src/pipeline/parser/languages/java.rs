use crate::models::{CallIntent, ReferenceIntent};
use crate::pipeline::parser::utils::node_text;
use tree_sitter::Node;

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
        _ => {}
    }

    // Recursively process children
    let mut child = node.child(0);
    while let Some(c) = child {
        collect_all_reference_intents_java(c, source, intents);
        child = c.next_sibling();
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

/// Extract type references from Java type annotations.
///
/// Recursively searches for `type_identifier` nodes in:
/// - Method parameters
/// - Constructor parameters (dependency injection)
/// - Field types
/// - Return types
///
/// Example:
/// ```java
/// public class AppComponent {
///   private final AnalyticsService analytics;
///   private final SeoService seo;
///   
///   public AppComponent(AnalyticsService analytics, SeoService seo) {
///     this.analytics = analytics;
///     this.seo = seo;
///   }
///   
///   public ResultType process(DataService data) {
///     return null;
///   }
/// }
/// ```
///
/// This will extract: AnalyticsService (3 times), SeoService (3 times), DataService, ResultType
pub(crate) fn extract_type_references(
    node: Node<'_>,
    source: &[u8],
    intents: &mut Vec<ReferenceIntent>,
) {
    let line = node.start_position().row + 1;

    // Capture type_identifier nodes (type annotations)
    if node.kind() == "type_identifier" {
        let type_name = node_text(node, source);
        // Only capture capitalized identifiers (likely classes/interfaces)
        if type_name.chars().next().is_some_and(|c| c.is_uppercase()) {
            intents.push(ReferenceIntent::TypeReference { type_name, line });
        }
    }

    // Recursively process children
    let mut child = node.child(0);
    while let Some(c) = child {
        extract_type_references(c, source, intents);
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
        });
    }
}

/// Extract method invocation call intents from a Java method body.
pub(crate) fn extract_call_intents_java(
    node: Node<'_>,
    source: &[u8],
    intents: &mut Vec<CallIntent>,
) {
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
                });
            }
        } else if let Some(method) = method_name {
            // No dot found, so receiver is actually a method name (local call)
            intents.push(CallIntent {
                method,
                receiver: None,
                line,
            });
            // Revert receiver since it's not a receiver
        } else if let Some(receiver_val) = receiver {
            // Single identifier - treat as local call
            intents.push(CallIntent {
                method: receiver_val,
                receiver: None,
                line,
            });
        }
    } else if node.kind() == "object_creation_expression" {
        let line = node.start_position().row + 1;
        let mut child = node.child(0);
        while let Some(c) = child {
            if c.kind() == "type_identifier" || c.kind() == "identifier" {
                intents.push(CallIntent {
                    method: node_text(c, source),
                    receiver: None,
                    line,
                });
                break;
            }
            child = c.next_sibling();
        }
    }

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
                });
            }
        } else if let Some(method) = method_name {
            // No dot found, so receiver is actually a method name (local call)
            intents.push(CallIntent {
                method,
                receiver: None,
                line,
            });
            // Revert receiver since it's not a receiver
        } else if let Some(receiver_val) = receiver {
            // Single identifier - treat as local call
            intents.push(CallIntent {
                method: receiver_val,
                receiver: None,
                line,
            });
        }
    } else if node.kind() == "object_creation_expression" {
        let line = node.start_position().row + 1;
        let mut child = node.child(0);
        while let Some(c) = child {
            if c.kind() == "type_identifier" || c.kind() == "identifier" {
                intents.push(CallIntent {
                    method: node_text(c, source),
                    receiver: None,
                    line,
                });
                break;
            }
            child = c.next_sibling();
        }
    }

    // NO recursive child processing - that's the key difference!
    intents
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_single_call_intent_java_method_invocation() {
        let code = "void test() { obj.method(); }";
        let tree = crate::pipeline::parser::test_utils::parse_java_snippet(code)
            .expect("Failed to parse Java code");

        // Find the method invocation node
        fn find_node<'a>(node: tree_sitter::Node<'a>, kind: &str) -> Option<tree_sitter::Node<'a>> {
            if node.kind() == kind {
                return Some(node);
            }
            let mut i = 0u32;
            while let Some(child) = node.child(i) {
                if let Some(found) = find_node(child, kind) {
                    return Some(found);
                }
                i += 1;
            }
            None
        }

        if let Some(invocation) = find_node(tree.root_node(), "method_invocation") {
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

        fn find_node<'a>(node: tree_sitter::Node<'a>, kind: &str) -> Option<tree_sitter::Node<'a>> {
            if node.kind() == kind {
                return Some(node);
            }
            let mut i = 0u32;
            while let Some(child) = node.child(i) {
                if let Some(found) = find_node(child, kind) {
                    return Some(found);
                }
                i += 1;
            }
            None
        }

        if let Some(invocation) = find_node(tree.root_node(), "method_invocation") {
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
}
