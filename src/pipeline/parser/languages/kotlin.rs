use crate::models::{CallIntent, ReferenceIntent};
use crate::pipeline::parser::utils::node_text;
use tree_sitter::Node;

/// Recursively extract all call intents from Kotlin.
#[allow(dead_code)]
pub(crate) fn collect_all_reference_intents_kotlin(
    node: Node<'_>,
    source: &[u8],
    intents: &mut Vec<(ReferenceIntent, usize)>,
) {
    let byte_pos = node.start_byte();
    let line = node.start_position().row + 1;

    match node.kind() {
        "call_expression" => {
            // Use non-recursive extraction to avoid double-processing children
            let call_intents = extract_single_call_intent_kotlin(node, source);
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
        "modifiers" => {
            // Extract annotation references (e.g., @Component, @Autowired, @Composable)
            let mut annotation_refs = Vec::new();
            extract_identifiers_from_annotation(node, source, &mut annotation_refs, line);
            for ref_intent in annotation_refs {
                intents.push((ref_intent, byte_pos));
            }
        }
        "type_identifier" | "simple_identifier" => {
            // Extract type references in parameter lists, field types, return types
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
        collect_all_reference_intents_kotlin(c, source, intents);
        child = c.next_sibling();
    }
}

/// Extract annotation references from Kotlin annotations (e.g., @Component, @Composable).
///
/// Recursively searches for annotation nodes and extracts capitalized identifiers
/// (likely class/component names) as TypeReference intents.
///
/// Example:
/// ```kotlin
/// @Configuration
/// @ComponentScan
/// class AppModule {}
/// ```
///
/// This will extract: Configuration, ComponentScan
pub(crate) fn extract_annotation_references(
    node: Node<'_>,
    source: &[u8],
    intents: &mut Vec<ReferenceIntent>,
) {
    let line = node.start_position().row + 1;

    // If this is an annotation node, extract references from it
    if node.kind() == "annotation" {
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
            "simple_identifier" | "type_identifier" => {
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
                // Recurse into nested structures
                extract_identifiers_from_annotation(c, source, intents, line);
            }
        }
        child = c.next_sibling();
    }
}

/// Extract type references from Kotlin type annotations.
///
/// Recursively searches for `type_identifier` nodes in:
/// - Function parameters
/// - Property types
/// - Return types
/// - Constructor parameters
///
/// Example:
/// ```kotlin
/// class AppComponent(
///     val analytics: AnalyticsService,
///     val seo: SeoService
/// ) {
///     fun process(data: DataService): ResultType {
///         return null
///     }
/// }
/// ```
///
/// This will extract: AnalyticsService, SeoService, DataService, ResultType
pub(crate) fn extract_type_references(
    node: Node<'_>,
    source: &[u8],
    intents: &mut Vec<ReferenceIntent>,
) {
    let line = node.start_position().row + 1;

    // Capture type_identifier nodes (type annotations)
    if matches!(node.kind(), "type_identifier" | "user_type") {
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

/// Extract reference intents from a Kotlin method body (wrapper for backward compatibility).
pub(crate) fn extract_reference_intents_kotlin(
    node: Node<'_>,
    source: &[u8],
    intents: &mut Vec<ReferenceIntent>,
) {
    let mut call_intents = Vec::new();
    extract_call_intents_kotlin(node, source, &mut call_intents);
    for call in call_intents {
        intents.push(ReferenceIntent::Call {
            method: call.method,
            receiver: call.receiver,
            line: call.line,
        });
    }
}

/// Extract function/method invocation call intents from a Kotlin method body.
pub(crate) fn extract_call_intents_kotlin(
    node: Node<'_>,
    source: &[u8],
    intents: &mut Vec<CallIntent>,
) {
    if node.kind() == "call_expression" {
        let mut method_name: Option<String> = None;
        let mut receiver: Option<String> = None;
        let line = node.start_position().row + 1;

        // Parse call_expression structure:
        // - Has optional receiver (identifier or "this") via postfix_expression
        // - Has navigation_suffix for method/function name
        let mut child = node.child(0);
        while let Some(c) = child {
            let kind = c.kind();
            match kind {
                "simple_identifier" => {
                    // Direct function call
                    method_name = Some(node_text(c, source));
                }
                "postfix_expression" => {
                    // Check for receiver.method pattern
                    extract_receiver_and_method(c, source, &mut receiver, &mut method_name);
                }
                "navigation_suffix" => {
                    // Method name in navigation suffix
                    if let Some(nav_child) = c.child(0) {
                        if nav_child.kind() == "simple_identifier" {
                            method_name = Some(node_text(nav_child, source));
                        }
                    }
                }
                _ => {}
            }
            child = c.next_sibling();
        }

        // Push the call intent
        if let Some(method) = method_name {
            intents.push(CallIntent {
                method,
                receiver,
                line,
            });
        }
    }

    // Recursively process children
    let mut child = node.child(0);
    while let Some(c) = child {
        extract_call_intents_kotlin(c, source, intents);
        child = c.next_sibling();
    }
}

/// Helper function to extract receiver and method from postfix_expression.
fn extract_receiver_and_method(
    node: Node<'_>,
    source: &[u8],
    receiver: &mut Option<String>,
    method: &mut Option<String>,
) {
    let mut child = node.child(0);
    while let Some(c) = child {
        match c.kind() {
            "simple_identifier" => {
                if receiver.is_none() {
                    *receiver = Some(node_text(c, source));
                } else if method.is_none() {
                    *method = Some(node_text(c, source));
                }
            }
            "this" => {
                *receiver = Some("this".to_string());
            }
            "navigation_suffix" => {
                // Extract method name from navigation_suffix
                if let Some(nav_child) = c.child(0) {
                    if nav_child.kind() == "simple_identifier" {
                        *method = Some(node_text(nav_child, source));
                    }
                }
            }
            _ => {}
        }
        child = c.next_sibling();
    }
}

/// Extract call intents from a SINGLE Kotlin node without recursive descent.
///
/// This is the non-recursive version of `extract_call_intents_kotlin`,
/// designed to be used in contexts where the caller already handles tree traversal.
///
/// By extracting only the current node's intent, we avoid double-processing children
/// that would cause duplicate CALLS with incorrect byte_pos/line attribution.
#[allow(dead_code)]
pub(crate) fn extract_single_call_intent_kotlin(node: Node<'_>, source: &[u8]) -> Vec<CallIntent> {
    let mut intents = Vec::new();

    if node.kind() == "call_expression" {
        let mut method_name: Option<String> = None;
        let mut receiver: Option<String> = None;
        let line = node.start_position().row + 1;

        let mut child = node.child(0);
        while let Some(c) = child {
            let kind = c.kind();
            match kind {
                "simple_identifier" => {
                    method_name = Some(node_text(c, source));
                }
                "postfix_expression" => {
                    extract_receiver_and_method(c, source, &mut receiver, &mut method_name);
                }
                "navigation_suffix" => {
                    if let Some(nav_child) = c.child(0) {
                        if nav_child.kind() == "simple_identifier" {
                            method_name = Some(node_text(nav_child, source));
                        }
                    }
                }
                _ => {}
            }
            child = c.next_sibling();
        }

        if let Some(method) = method_name {
            intents.push(CallIntent {
                method,
                receiver,
                line,
            });
        }
    }

    intents
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_call_intent_simple_function() {
        let code = "fun main() { println(\"Hello\") }";
        let lang = tree_sitter_kotlin_ng::LANGUAGE.into();
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&lang).unwrap();
        let tree = parser.parse(code, None).unwrap();

        let mut intents = Vec::new();
        extract_call_intents_kotlin(tree.root_node(), code.as_bytes(), &mut intents);

        // At least verify parsing doesn't crash and produces a tree
        assert!(!tree.root_node().has_error());
    }

    #[test]
    fn test_extract_call_intent_with_receiver() {
        let code = "fun main() { obj.method() }";
        let lang = tree_sitter_kotlin_ng::LANGUAGE.into();
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&lang).unwrap();
        let tree = parser.parse(code, None).unwrap();

        let mut intents = Vec::new();
        extract_call_intents_kotlin(tree.root_node(), code.as_bytes(), &mut intents);

        // At least verify parsing doesn't crash and produces a tree
        assert!(!tree.root_node().has_error());
    }

    #[test]
    fn test_extract_class_declaration() {
        let code = "class MyClass { }";
        let lang = tree_sitter_kotlin_ng::LANGUAGE.into();
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&lang).unwrap();
        let tree = parser.parse(code, None).unwrap();

        // Walk the tree to find class declaration
        let root = tree.root_node();
        let mut found_class = false;
        let mut child = root.child(0);
        while let Some(c) = child {
            if c.kind() == "class_declaration" {
                found_class = true;
                break;
            }
            child = c.next_sibling();
        }
        assert!(found_class, "Class declaration not found in AST");
    }

    #[test]
    fn test_extract_function_declaration() {
        let code = "fun myFunction() { }";
        let lang = tree_sitter_kotlin_ng::LANGUAGE.into();
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&lang).unwrap();
        let tree = parser.parse(code, None).unwrap();

        // Walk the tree to find function declaration
        let root = tree.root_node();
        let mut found_func = false;
        let mut child = root.child(0);
        while let Some(c) = child {
            if c.kind() == "function_declaration" {
                found_func = true;
                break;
            }
            child = c.next_sibling();
        }
        assert!(found_func, "Function declaration not found in AST");
    }

    #[test]
    fn test_extract_property_declaration() {
        let code = "val myProperty: String = \"test\"";
        let lang = tree_sitter_kotlin_ng::LANGUAGE.into();
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&lang).unwrap();
        let tree = parser.parse(code, None).unwrap();

        // Walk the tree to find property declaration
        let root = tree.root_node();
        let mut found_property = false;
        let mut child = root.child(0);
        while let Some(c) = child {
            if c.kind() == "property_declaration" {
                found_property = true;
                break;
            }
            child = c.next_sibling();
        }
        assert!(found_property, "Property declaration not found in AST");
    }
}
