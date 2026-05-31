use crate::models::{CallIntent, EntityKind, ParsedEntity, ReferenceIntent};
use crate::pipeline::parser::utils::node_text;
use tree_sitter::Node;

/// Recursively extract all call intents from Kotlin.
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
                        arg_count: call.arg_count,
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
        "type_identifier" | "simple_identifier" | "identifier" => {
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
            "simple_identifier" | "type_identifier" | "identifier" => {
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
    if matches!(node.kind(), "type_identifier" | "user_type" | "identifier") {
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

/// Extract extends/implements relationships from Kotlin class/interface declarations.
///
/// In Kotlin, both superclass and superinterface declarations follow the `:` token as
/// delegation specifiers:
/// - `class Foo : Base(), Iface1, Iface2 by delegate`
/// - `interface Bar : SuperIface1, SuperIface2`
///
/// In tree-sitter-kotlin-ng v1.1.0, the AST for these is:
/// ```text
/// (delegation_specifiers
///   (delegation_specifier
///     (constructor_invocation  ← present → EXTENDS (parent class)
///       (user_type (identifier)))
///     (value_arguments))
///   (delegation_specifier
///     (user_type (identifier)))  ← no parens → IMPLEMENTS (interface)
/// )
/// ```
///
/// For interface declarations, all delegation specifiers are treated as EXTENDS
/// because interfaces extend other interfaces (they do not implement).
pub(crate) fn extract_class_inheritance_kotlin(
    class_node: Node<'_>,
    source: &[u8],
    intents: &mut Vec<ReferenceIntent>,
) {
    // Determine if this class_node is actually an interface declaration.
    // (For interfaces, everything after : is EXTENDS, not IMPLEMENTS.)
    let is_interface = {
        let text = node_text(class_node, source);
        text.split_whitespace()
            .find(|t| {
                !matches!(
                    *t,
                    "public"
                        | "private"
                        | "protected"
                        | "internal"
                        | "abstract"
                        | "open"
                        | "sealed"
                        | "annotation"
                        | "final"
                        | "override"
                )
            })
            .is_some_and(|kw| kw == "interface")
    };

    // Find the delegation_specifiers node
    let mut child = class_node.child(0);
    while let Some(c) = child {
        if c.kind() == "delegation_specifiers" {
            extract_delegation_specifiers(c, source, intents, is_interface);
            return;
        }
        child = c.next_sibling();
    }
}

/// Recursively walk delegation_specifier nodes and emit EXTENDS/IMPLEMENTS intents.
fn extract_delegation_specifiers(
    specifiers_node: Node<'_>,
    source: &[u8],
    intents: &mut Vec<ReferenceIntent>,
    is_interface: bool,
) {
    let mut child = specifiers_node.child(0);
    while let Some(c) = child {
        if c.kind() == "delegation_specifier" {
            extract_single_delegation(c, source, intents, is_interface);
        }
        child = c.next_sibling();
    }
}

/// Extract a single delegation_specifier into an EXTENDS or IMPLEMENTS intent.
fn extract_single_delegation(
    specifier: Node<'_>,
    source: &[u8],
    intents: &mut Vec<ReferenceIntent>,
    is_interface: bool,
) {
    let line = specifier.start_position().row + 1;

    // Extract the type name from user_type → identifier
    let type_name = extract_delegation_type_name(specifier, source);

    if type_name.is_none() {
        return;
    }
    let type_name = type_name.unwrap();

    // Determine if this is a constructor invocation (EXTENDS) or plain user_type (IMPLEMENTS)
    let has_constructor_invocation = specifier
        .child(0)
        .is_some_and(|c| c.kind() == "constructor_invocation");

    let has_explicit_delegation = specifier
        .children(&mut specifier.walk())
        .any(|c| c.kind() == "explicit_delegation");

    if has_constructor_invocation {
        // Parent class — always EXTENDS
        intents.push(ReferenceIntent::Extends {
            parent: type_name,
            line,
        });
    } else if has_explicit_delegation || !is_interface {
        // Delegation by `by` or regular interface implementation
        intents.push(ReferenceIntent::Implements {
            interface: type_name,
            line,
        });
    } else {
        // Interface extending another interface — use EXTENDS
        intents.push(ReferenceIntent::Extends {
            parent: type_name,
            line,
        });
    }
}

/// Walk down from a delegation_specifier to find the user_type → identifier text.
fn extract_delegation_type_name(specifier: Node<'_>, source: &[u8]) -> Option<String> {
    // Structure: (delegation_specifier (constructor_invocation (user_type (identifier))) ...)
    //           or: (delegation_specifier (user_type (identifier)))
    // We need to find the deepest identifier.

    // First try: direct user_type child
    let mut user_type = None;
    let mut child = specifier.child(0);
    while let Some(c) = child {
        match c.kind() {
            "user_type" => {
                user_type = Some(c);
                break;
            }
            "constructor_invocation" => {
                // Navigate into constructor_invocation to find user_type
                let mut ci_child = c.child(0);
                while let Some(cc) = ci_child {
                    if cc.kind() == "user_type" {
                        user_type = Some(cc);
                        break;
                    }
                    ci_child = cc.next_sibling();
                }
                break;
            }
            "explicit_delegation" => {
                // Navigate into explicit_delegation to find user_type
                let mut ed_child = c.child(0);
                while let Some(ec) = ed_child {
                    if ec.kind() == "user_type" {
                        user_type = Some(ec);
                        break;
                    }
                    ed_child = ec.next_sibling();
                }
                break;
            }
            _ => {}
        }
        child = c.next_sibling();
    }

    // From user_type, extract the identifier
    if let Some(ut) = user_type {
        let mut ident_child = ut.child(0);
        while let Some(ic) = ident_child {
            if matches!(
                ic.kind(),
                "identifier" | "type_identifier" | "simple_identifier"
            ) {
                return Some(node_text(ic, source));
            }
            ident_child = ic.next_sibling();
        }
    }

    None
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
            arg_count: call.arg_count,
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
                "simple_identifier" | "identifier" => {
                    // Direct function call
                    method_name = Some(node_text(c, source));
                }
                "postfix_expression" | "navigation_expression" => {
                    // Check for receiver.method pattern
                    extract_receiver_and_method(c, source, &mut receiver, &mut method_name);
                }
                "navigation_suffix" => {
                    // Method name in navigation suffix
                    if let Some(nav_child) = c.child(0)
                        && matches!(nav_child.kind(), "simple_identifier" | "identifier")
                    {
                        method_name = Some(node_text(nav_child, source));
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
                arg_count: None,
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

/// Helper function to extract receiver and method from postfix_expression or navigation_expression.
fn extract_receiver_and_method(
    node: Node<'_>,
    source: &[u8],
    receiver: &mut Option<String>,
    method: &mut Option<String>,
) {
    // If it's a navigation_expression, it usually has [receiver, ., method]
    if node.kind() == "navigation_expression" {
        let count = node.child_count();
        if count >= 3 {
            let last_child = node.child(count as u32 - 1).unwrap();
            let first_child = node.child(0).unwrap();

            if matches!(last_child.kind(), "simple_identifier" | "identifier") {
                *method = Some(node_text(last_child, source));
            }

            if first_child.kind() == "navigation_expression" {
                // Recurse to get the base receiver if needed,
                // but for now let's just take the whole text of the first part as receiver
                *receiver = Some(node_text(first_child, source));
            } else if matches!(
                first_child.kind(),
                "simple_identifier" | "identifier" | "this"
            ) {
                *receiver = Some(node_text(first_child, source));
            }
            return;
        }
    }

    let mut child = node.child(0);
    while let Some(c) = child {
        match c.kind() {
            "simple_identifier" | "identifier" => {
                if receiver.is_none() {
                    *receiver = Some(node_text(c, source));
                } else {
                    // Always take the latest identifier as the potential method name
                    *method = Some(node_text(c, source));
                }
            }
            "this" => {
                *receiver = Some("this".to_string());
            }
            "navigation_suffix" => {
                if let Some(nav_child) = c.child(0)
                    && matches!(nav_child.kind(), "simple_identifier" | "identifier")
                {
                    *method = Some(node_text(nav_child, source));
                }
            }
            "navigation_expression" | "postfix_expression" => {
                extract_receiver_and_method(c, source, receiver, method);
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
                "simple_identifier" | "identifier" => {
                    method_name = Some(node_text(c, source));
                }
                "postfix_expression" | "navigation_expression" => {
                    extract_receiver_and_method(c, source, &mut receiver, &mut method_name);
                }
                "navigation_suffix" => {
                    if let Some(nav_child) = c.child(0)
                        && matches!(nav_child.kind(), "simple_identifier" | "identifier")
                    {
                        method_name = Some(node_text(nav_child, source));
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
                arg_count: None,
            });
        }
    }

    // NO recursive child processing - that's the key difference!
    intents
}

/// Extract anonymous object implementations from Kotlin method bodies.
///
/// Kotlin frequently uses `object : Interface { ... }` expressions (anonymous objects)
/// to implement interfaces inline. These `object_literal` nodes in the AST contain
/// `delegation_specifiers` just like named class declarations.
///
/// This function walks the entire AST looking for `object_literal` nodes with delegation
/// specifiers, finds the enclosing named entity, and creates a synthetic `ParsedEntity`
/// (name: `<anonymous>`, kind: `KotlinObject`) with the appropriate `Implements`/`Extends`
/// reference intents.
pub(crate) fn extract_anonymous_object_implementations(
    root: Node<'_>,
    source: &[u8],
    file_path: &str,
    repo_name: &str,
    existing_entities: &[ParsedEntity],
    out: &mut Vec<ParsedEntity>,
) {
    extract_anonymous_objects_recursive(
        root,
        source,
        file_path,
        repo_name,
        existing_entities,
        out,
        &mut 0u32,
    );
}

fn extract_anonymous_objects_recursive(
    node: Node<'_>,
    source: &[u8],
    file_path: &str,
    repo_name: &str,
    existing_entities: &[ParsedEntity],
    out: &mut Vec<ParsedEntity>,
    counter: &mut u32,
) {
    if node.kind() == "object_literal" {
        // Check if this anonymous object has delegation specifiers (i.e., `object : X, Y`)
        let has_delegation = node
            .children(&mut node.walk())
            .any(|c| c.kind() == "delegation_specifiers");

        if has_delegation {
            let line = node.start_position().row + 1;
            let end_line = node.end_position().row + 1;

            // Find enclosing entity (method, class, etc.) by line range
            let enclosing_fqn = find_enclosing_fqn(line, existing_entities);

            // Synthesize an FQN that includes the line number for uniqueness
            let name = "<anonymous>".to_string();
            let fqn = if let Some(ref enclosing) = enclosing_fqn {
                format!("{enclosing}.<anonymous@{line}>")
            } else {
                format!("<anonymous@{line}>")
            };

            // Extract delegation specifiers using the existing helper
            let mut intents = Vec::new();
            let mut child = node.child(0);
            while let Some(c) = child {
                if c.kind() == "delegation_specifiers" {
                    // Anonymous objects always create IMPLEMENTS for interfaces
                    // and EXTENDS for constructor invocations.
                    extract_delegation_specifiers(c, source, &mut intents, false);
                    break;
                }
                child = c.next_sibling();
            }

            if !intents.is_empty() {
                *counter += 1;
                let mut entity = ParsedEntity::new(
                    &name,
                    EntityKind::KotlinObject,
                    &fqn,
                    None,
                    None,
                    "kotlin",
                    file_path,
                    line,
                    end_line,
                    enclosing_fqn,
                    repo_name,
                );
                entity.reference_intents = intents;
                out.push(entity);
            }
        }
        // Even if no delegation, don't recurse into children of object_literal
        // (they are method overrides, not new anonymous objects).
        return;
    }

    // Recursively walk children
    let mut child = node.child(0);
    while let Some(c) = child {
        extract_anonymous_objects_recursive(
            c,
            source,
            file_path,
            repo_name,
            existing_entities,
            out,
            counter,
        );
        child = c.next_sibling();
    }
}

/// Find the FQN of the nearest enclosing entity that contains the given line.
fn find_enclosing_fqn(line: usize, entities: &[ParsedEntity]) -> Option<String> {
    let mut best: Option<&ParsedEntity> = None;
    for e in entities {
        if e.name == "<anonymous>" {
            continue; // Skip synthetic entities
        }
        if line >= e.start_line && line <= e.end_line {
            match best {
                None => best = Some(e),
                Some(b) => {
                    // Pick the innermost (smallest range)
                    let b_range = b.end_line - b.start_line;
                    let e_range = e.end_line - e.start_line;
                    if e_range < b_range {
                        best = Some(e);
                    }
                }
            }
        }
    }
    best.map(|e| e.fqn.clone())
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

    #[test]
    fn test_extract_call_intent_navigation() {
        let code = "fun main() { userService.getUser(1) }";
        let lang = tree_sitter_kotlin_ng::LANGUAGE.into();
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&lang).unwrap();
        let tree = parser.parse(code, None).unwrap();

        let mut intents = Vec::new();
        extract_call_intents_kotlin(tree.root_node(), code.as_bytes(), &mut intents);

        assert_eq!(intents.len(), 1);
        assert_eq!(intents[0].method, "getUser");
        assert_eq!(intents[0].receiver, Some("userService".to_string()));
    }

    #[test]
    fn test_extract_call_intent_chained_navigation() {
        let code = "fun main() { Config.instance.getUser(1) }";
        let lang = tree_sitter_kotlin_ng::LANGUAGE.into();
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&lang).unwrap();
        let tree = parser.parse(code, None).unwrap();

        let mut intents = Vec::new();
        extract_call_intents_kotlin(tree.root_node(), code.as_bytes(), &mut intents);

        assert_eq!(intents.len(), 1);
        assert_eq!(intents[0].method, "getUser");
        assert!(intents[0].receiver.as_ref().unwrap().contains("Config"));
    }

    // --- Inheritance extraction tests ---

    fn get_class_declaration(root: Node<'_>) -> Node<'_> {
        let mut child = root.child(0);
        while let Some(c) = child {
            if c.kind() == "class_declaration" {
                return c;
            }
            child = c.next_sibling();
        }
        panic!("class_declaration not found in AST");
    }

    fn parse_kotlin(code: &str) -> tree_sitter::Tree {
        let lang = tree_sitter_kotlin_ng::LANGUAGE.into();
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&lang).unwrap();
        parser.parse(code, None).unwrap()
    }

    #[test]
    fn test_extract_class_inheritance_class_with_parent_and_interface() {
        let source = "class Foo : Base(), Iface {\n    fun bar() {}\n}";
        let tree = parse_kotlin(source);
        let node = get_class_declaration(tree.root_node());
        let mut intents = Vec::new();
        extract_class_inheritance_kotlin(node, source.as_bytes(), &mut intents);

        let extends: Vec<_> = intents
            .iter()
            .filter_map(|r| {
                if let ReferenceIntent::Extends { parent, .. } = r {
                    Some(parent.as_str())
                } else {
                    None
                }
            })
            .collect();
        let implements: Vec<_> = intents
            .iter()
            .filter_map(|r| {
                if let ReferenceIntent::Implements { interface, .. } = r {
                    Some(interface.as_str())
                } else {
                    None
                }
            })
            .collect();

        assert_eq!(
            extends,
            &["Base"],
            "Expected EXTENDS Base (constructor invocation)"
        );
        assert_eq!(
            implements,
            &["Iface"],
            "Expected IMPLEMENTS Iface (no constructor)"
        );
    }

    #[test]
    fn test_extract_class_inheritance_interface_extends() {
        let source = "interface Bar : Iface {\n    fun baz()\n}";
        let tree = parse_kotlin(source);
        let node = get_class_declaration(tree.root_node());
        let mut intents = Vec::new();
        extract_class_inheritance_kotlin(node, source.as_bytes(), &mut intents);

        let extends: Vec<_> = intents
            .iter()
            .filter_map(|r| {
                if let ReferenceIntent::Extends { parent, .. } = r {
                    Some(parent.as_str())
                } else {
                    None
                }
            })
            .collect();

        assert_eq!(
            extends,
            &["Iface"],
            "Interface extending another interface should use EXTENDS"
        );
    }

    #[test]
    fn test_extract_class_inheritance_no_inheritance() {
        let source = "class Foo {\n    fun bar() {}\n}";
        let tree = parse_kotlin(source);
        let node = get_class_declaration(tree.root_node());
        let mut intents = Vec::new();
        extract_class_inheritance_kotlin(node, source.as_bytes(), &mut intents);

        assert!(
            intents.is_empty(),
            "Expected no inheritance intents for class without supertypes"
        );
    }

    #[test]
    fn test_extract_class_inheritance_delegation() {
        let source = "class Foo : Iface by delegate {\n    fun bar() {}\n}";
        let tree = parse_kotlin(source);
        let node = get_class_declaration(tree.root_node());
        let mut intents = Vec::new();
        extract_class_inheritance_kotlin(node, source.as_bytes(), &mut intents);

        let implements: Vec<_> = intents
            .iter()
            .filter_map(|r| {
                if let ReferenceIntent::Implements { interface, .. } = r {
                    Some(interface.as_str())
                } else {
                    None
                }
            })
            .collect();

        assert_eq!(
            implements,
            &["Iface"],
            "Expected IMPLEMENTS Iface via delegation"
        );
    }

    #[test]
    fn test_extract_anonymous_object_implements() {
        let source = "class Foo {\n    fun bar() {\n        val x = object : Iface {\n            fun foo() {}\n        }\n    }\n}";
        let tree = parse_kotlin(source);
        // Build enclosing entity to test line-range resolution
        let existing = vec![
            ParsedEntity::new(
                "Foo",
                EntityKind::KotlinClass,
                "Foo",
                None,
                None,
                "kotlin",
                "test.kt",
                1,
                7,
                None,
                "test",
            ),
            ParsedEntity::new(
                "bar",
                EntityKind::KotlinMethod,
                "Foo.bar",
                None,
                None,
                "kotlin",
                "test.kt",
                2,
                6,
                Some("Foo".to_string()),
                "test",
            ),
        ];

        let mut out = Vec::new();
        extract_anonymous_object_implementations(
            tree.root_node(),
            source.as_bytes(),
            "test.kt",
            "test",
            &existing,
            &mut out,
        );

        assert_eq!(out.len(), 1, "Expected 1 anonymous object entity");
        assert_eq!(out[0].name, "<anonymous>");
        assert_eq!(out[0].kind, EntityKind::KotlinObject);
        assert!(
            out[0].fqn.contains("Foo.bar"),
            "FQN should contain enclosing method: {}",
            out[0].fqn
        );
        assert!(
            out[0].fqn.contains("<anonymous@"),
            "FQN should contain <anonymous@LINE>"
        );

        let implements: Vec<_> = out[0]
            .reference_intents
            .iter()
            .filter_map(|r| {
                if let ReferenceIntent::Implements { interface, .. } = r {
                    Some(interface.as_str())
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(
            implements,
            &["Iface"],
            "Expected IMPLEMENTS Iface from anonymous object"
        );
    }

    #[test]
    fn test_extract_anonymous_object_no_inheritance() {
        let source = "fun main() { val x = object { fun foo() {} } }";
        let tree = parse_kotlin(source);
        let existing = vec![ParsedEntity::new(
            "main",
            EntityKind::KotlinFunction,
            "main",
            None,
            None,
            "kotlin",
            "test.kt",
            1,
            1,
            None,
            "test",
        )];

        let mut out = Vec::new();
        extract_anonymous_object_implementations(
            tree.root_node(),
            source.as_bytes(),
            "test.kt",
            "test",
            &existing,
            &mut out,
        );

        assert!(
            out.is_empty(),
            "Anonymous object without inheritance should not create entity"
        );
    }
}
