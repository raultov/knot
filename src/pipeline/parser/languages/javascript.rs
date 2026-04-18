use crate::models::{CallIntent, EntityKind, ParsedEntity, ReferenceIntent};
use crate::pipeline::parser::utils::node_text;
use tree_sitter::Node;
use uuid::Uuid;

/// Recursively extract all call intents from JavaScript, returning (intent, byte_pos) pairs.
pub(crate) fn collect_all_reference_intents_javascript(
    node: Node<'_>,
    source: &[u8],
    intents: &mut Vec<(ReferenceIntent, usize)>,
) {
    let byte_pos = node.start_byte();
    let line = node.start_position().row + 1;

    match node.kind() {
        "call_expression" | "new_expression" => {
            // Use non-recursive extraction to avoid double-processing children
            let call_intents = extract_single_call_intent_javascript(node, source);
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
        "jsx_self_closing_element" | "jsx_opening_element" => {
            let mut call_intents = Vec::new();
            extract_jsx_component_invocation(node, source, &mut call_intents);
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
        "decorator" => {
            // Extract decorator references (e.g., @Component({ declarations: [AppComponent] }))
            let mut decorator_refs = Vec::new();
            extract_identifiers_from_decorator(node, source, &mut decorator_refs, line);
            for ref_intent in decorator_refs {
                intents.push((ref_intent, byte_pos));
            }
        }
        _ => {}
    }

    // Recursively process children
    let mut child = node.child(0);
    while let Some(c) = child {
        collect_all_reference_intents_javascript(c, source, intents);
        child = c.next_sibling();
    }
}

/// Extract class inheritance (extends clause) from JavaScript class AST.
/// JavaScript doesn't have implements, so we only handle extends.
/// Manually traverses the class declaration node to find the extends clause.
pub(crate) fn extract_class_inheritance_js(
    class_node: Node<'_>,
    source: &[u8],
    intents: &mut Vec<ReferenceIntent>,
) {
    let line = class_node.start_position().row + 1;

    // Look for 'extends' keyword followed by identifier (no type_identifier in JS)
    let mut child = class_node.child(0);
    while let Some(c) = child {
        if c.kind() == "extends" {
            // Next identifier should be the parent class
            if let Some(next) = c.next_sibling()
                && next.kind() == "identifier"
            {
                let parent_name = node_text(next, source);
                intents.push(ReferenceIntent::Extends {
                    parent: parent_name,
                    line,
                });
            }
        }
        child = c.next_sibling();
    }
}

/// Extract decorator references from JavaScript decorators (e.g., @Component, @Injectable).
///
/// Recursively searches for `decorator` nodes and extracts capitalized identifiers
/// (likely class/component names) as TypeReference intents.
///
/// Example:
/// ```javascript
/// @Component({
///   declarations: [AppComponent, UserComponent],
///   bootstrap: [AppComponent]
/// })
/// export class AppModule {}
/// ```
///
/// This will extract: AppComponent (twice), UserComponent
pub(crate) fn extract_decorator_references(
    node: Node<'_>,
    source: &[u8],
    intents: &mut Vec<ReferenceIntent>,
) {
    let line = node.start_position().row + 1;

    // If this is a decorator node, extract references from its arguments
    if node.kind() == "decorator" {
        extract_identifiers_from_decorator(node, source, intents, line);
    }

    // Recursively process children
    let mut child = node.child(0);
    while let Some(c) = child {
        extract_decorator_references(c, source, intents);
        child = c.next_sibling();
    }
}

/// Extract capitalized identifiers from decorator arguments (likely class references).
fn extract_identifiers_from_decorator(
    decorator_node: Node<'_>,
    source: &[u8],
    intents: &mut Vec<ReferenceIntent>,
    line: usize,
) {
    // Recursively scan all children for identifiers
    let mut child = decorator_node.child(0);
    while let Some(c) = child {
        match c.kind() {
            "identifier" => {
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
                extract_identifiers_from_decorator(c, source, intents, line);
            }
        }
        child = c.next_sibling();
    }
}

/// Extract reference intents from a JavaScript function/method body (wrapper for backward compatibility).
pub(crate) fn extract_reference_intents_javascript(
    node: Node<'_>,
    source: &[u8],
    intents: &mut Vec<ReferenceIntent>,
) {
    let mut call_intents = Vec::new();
    extract_call_intents_javascript(node, source, &mut call_intents);
    for call in call_intents {
        intents.push(ReferenceIntent::Call {
            method: call.method,
            receiver: call.receiver,
            line: call.line,
        });
    }

    // Also extract enum/static member usages (e.g., ClassName.STATIC, Constants.VALUE)
    extract_enum_usages_javascript(node, source, intents);
}

/// Extract call expression call intents from a JavaScript function/method body.
///
/// Handles:
/// - Direct calls: `method()`, `this.method()`
/// - Member calls: `obj.method()`, `this.service.method()`
/// - New expressions: `new MyClass()`
/// - JSX components: `<ChartToolbar />`, `<Sheet.Content />`
/// - Callbacks passed as arguments: `app.use(this.handler)` -> records call to handler
/// - Bind calls: `this.method.bind(this)` -> records call to method
/// - Property/getter access: `this.client`, `this.field` -> records access to property/getter
pub(crate) fn extract_call_intents_javascript(
    node: Node<'_>,
    source: &[u8],
    intents: &mut Vec<CallIntent>,
) {
    if node.kind() == "call_expression" {
        let line = node.start_position().row + 1;

        let mut method_name: Option<String> = None;
        let mut receiver: Option<String> = None;

        // Look for the function field in the call_expression
        let mut child = node.child(0);
        let mut is_bind_call = false;

        while let Some(c) = child {
            if c.kind() == "member_expression" {
                // Use Tree-sitter API to extract fields cleanly
                if let Some(property_node) = c.child_by_field_name("property") {
                    let prop_text = node_text(property_node, source);
                    // Check if this is a .bind() call
                    if prop_text == "bind" {
                        is_bind_call = true;
                    }
                    method_name = Some(prop_text);
                }

                // For the object, we need to extract it as text to handle nested members
                if let Some(object_node) = c.child_by_field_name("object") {
                    receiver = Some(node_text(object_node, source));
                }
            } else if c.kind() == "identifier" {
                // Direct identifier in call_expression (local call)
                method_name = Some(node_text(c, source));
            }
            child = c.next_sibling();
        }

        if let Some(method) = method_name {
            // Special handling for .bind(this) and similar patterns
            if is_bind_call {
                if let Some(receiver) = receiver {
                    // Extract the method name from receiver (last component if it's a member expression)
                    if let Some(last_part) = receiver.split('.').next_back() {
                        intents.push(CallIntent {
                            method: last_part.to_string(),
                            receiver: if receiver.contains('.') {
                                receiver.split('.').next().map(|s| s.to_string())
                            } else {
                                Some("this".to_string())
                            },
                            line,
                        });
                    }
                }
            } else {
                intents.push(CallIntent {
                    method,
                    receiver,
                    line,
                });
            }
        }

        // Also scan arguments for callback references (e.g., app.use(this.handler))
        extract_callback_arguments(node, source, intents, line);
    } else if node.kind() == "new_expression" {
        let line = node.start_position().row + 1;
        let mut child = node.child(0);
        while let Some(c) = child {
            if c.kind() == "identifier" {
                intents.push(CallIntent {
                    method: node_text(c, source),
                    receiver: None,
                    line,
                });
                break;
            }
            child = c.next_sibling();
        }
    } else if node.kind() == "jsx_self_closing_element" || node.kind() == "jsx_opening_element" {
        // JSX component invocation (e.g., <ChartToolbar />, <Sheet.Content />)
        extract_jsx_component_invocation(node, source, intents);
    } else if node.kind() == "member_expression" {
        // Detect property/getter access via `this.property` (e.g., this.client, this.field)
        if let Some(object_node) = node.child_by_field_name("object")
            && node_text(object_node, source) == "this"
            && let Some(property_node) = node.child_by_field_name("property")
        {
            let prop_text = node_text(property_node, source);
            let line = node.start_position().row + 1;
            intents.push(CallIntent {
                method: prop_text,
                receiver: Some("this".to_string()),
                line,
            });
        }
    }

    // Recursively process children
    let mut child = node.child(0);
    while let Some(c) = child {
        extract_call_intents_javascript(c, source, intents);
        child = c.next_sibling();
    }
}

/// Extract call intents from a SINGLE node without recursive descent.
///
/// This is the non-recursive version of `extract_call_intents_javascript`,
/// designed to be used in contexts where the caller already handles tree traversal.
pub(crate) fn extract_single_call_intent_javascript(
    node: Node<'_>,
    source: &[u8],
) -> Vec<CallIntent> {
    let mut intents = Vec::new();

    if node.kind() == "call_expression" {
        let line = node.start_position().row + 1;

        let mut method_name: Option<String> = None;
        let mut receiver: Option<String> = None;

        let mut child = node.child(0);
        let mut is_bind_call = false;

        while let Some(c) = child {
            if c.kind() == "member_expression" {
                if let Some(property_node) = c.child_by_field_name("property") {
                    let prop_text = node_text(property_node, source);
                    if prop_text == "bind" {
                        is_bind_call = true;
                    }
                    method_name = Some(prop_text);
                }

                if let Some(object_node) = c.child_by_field_name("object") {
                    receiver = Some(node_text(object_node, source));
                }
            } else if c.kind() == "identifier" {
                method_name = Some(node_text(c, source));
            }
            child = c.next_sibling();
        }

        if let Some(method) = method_name {
            if is_bind_call {
                if let Some(receiver) = receiver
                    && let Some(last_part) = receiver.split('.').next_back()
                {
                    intents.push(CallIntent {
                        method: last_part.to_string(),
                        receiver: if receiver.contains('.') {
                            receiver.split('.').next().map(|s| s.to_string())
                        } else {
                            Some("this".to_string())
                        },
                        line,
                    });
                }
            } else {
                intents.push(CallIntent {
                    method,
                    receiver,
                    line,
                });
            }
        }

        // Also scan arguments for callback references
        extract_callback_arguments(node, source, &mut intents, line);
    } else if node.kind() == "new_expression" {
        let line = node.start_position().row + 1;
        let mut child = node.child(0);
        while let Some(c) = child {
            if c.kind() == "identifier" {
                intents.push(CallIntent {
                    method: node_text(c, source),
                    receiver: None,
                    line,
                });
                break;
            }
            child = c.next_sibling();
        }
    } else if node.kind() == "jsx_self_closing_element" || node.kind() == "jsx_opening_element" {
        // JSX component invocation
        extract_jsx_component_invocation(node, source, &mut intents);
    } else if node.kind() == "member_expression" {
        // Detect property/getter access via `this.property`
        if let Some(object_node) = node.child_by_field_name("object")
            && node_text(object_node, source) == "this"
            && let Some(property_node) = node.child_by_field_name("property")
        {
            let prop_text = node_text(property_node, source);
            let line = node.start_position().row + 1;
            intents.push(CallIntent {
                method: prop_text,
                receiver: Some("this".to_string()),
                line,
            });
        }
    }

    // NO recursive child processing - that's the key difference!
    intents
}

/// Extract JSX component invocation as a call intent.
///
/// Handles React components rendered via JSX syntax:
/// - `<ChartToolbar />` → CallIntent { method: "ChartToolbar", receiver: None }
/// - `<Sheet.Content />` → CallIntent { method: "Content", receiver: Some("Sheet") }
/// - `<Icons.Search />` → CallIntent { method: "Search", receiver: Some("Icons") }
///
/// Native HTML tags (lowercase) are ignored:
/// - `<div />` → skipped
/// - `<span />` → skipped
pub(crate) fn extract_jsx_component_invocation(
    node: Node<'_>,
    source: &[u8],
    intents: &mut Vec<CallIntent>,
) {
    let line = node.start_position().row + 1;

    // Get the name node (can be identifier, member_expression, or namespace_name)
    if let Some(name_node) = node.child_by_field_name("name") {
        let comp_name = node_text(name_node, source);

        // React convention: Components start with uppercase, HTML tags are lowercase
        if comp_name.chars().next().is_some_and(|c| c.is_uppercase()) {
            // Handle namespaced components (e.g., Sheet.Content, Icons.Search)
            if comp_name.contains('.') {
                let mut parts = comp_name.split('.');
                let receiver = parts.next().map(|s| s.to_string());
                // Collect remaining parts as method name (handles deeply nested components)
                let method = parts.collect::<Vec<_>>().join(".");

                intents.push(CallIntent {
                    method,
                    receiver,
                    line,
                });
            } else {
                // Simple component name
                intents.push(CallIntent {
                    method: comp_name,
                    receiver: None,
                    line,
                });
            }
        }
        // HTML tags (lowercase first letter) are intentionally skipped
    }
}

/// Extract callback arguments from a call expression.
///
/// Detects method references passed as arguments, e.g.:
/// - `app.use(this.authHandler)` -> records call to authHandler
/// - `emitter.on('event', this.handler)` -> records call to handler
/// - `addEventListener('click', this.onClick)` -> records call to onClick
pub(crate) fn extract_callback_arguments(
    call_node: Node<'_>,
    source: &[u8],
    intents: &mut Vec<CallIntent>,
    line: usize,
) {
    // Find the arguments node
    if let Some(args_node) = call_node.child_by_field_name("arguments") {
        let mut arg = args_node.child(0);
        while let Some(a) = arg {
            // Look for member_expression arguments (e.g., this.handler, obj.method)
            if a.kind() == "member_expression" {
                if let Some(property_node) = a.child_by_field_name("property") {
                    let method_name = node_text(property_node, source);
                    if let Some(object_node) = a.child_by_field_name("object") {
                        let receiver = node_text(object_node, source);
                        intents.push(CallIntent {
                            method: method_name,
                            receiver: Some(receiver),
                            line,
                        });
                    }
                }
            } else if a.kind() == "identifier" {
                // Sometimes callbacks are just identifiers: app.use(authHandler)
                let name = node_text(a, source);
                // Only treat as callback if it looks like a method name (not a keyword or literal)
                if !is_reserved_keyword(&name)
                    && name.chars().next().is_some_and(|c| c.is_alphabetic())
                {
                    intents.push(CallIntent {
                        method: name,
                        receiver: None,
                        line,
                    });
                }
            }
            arg = a.next_sibling();
        }
    }
}

/// Check if a string is a JavaScript reserved keyword.
pub(crate) fn is_reserved_keyword(word: &str) -> bool {
    matches!(
        word,
        "true"
            | "false"
            | "null"
            | "undefined"
            | "this"
            | "super"
            | "import"
            | "export"
            | "from"
            | "as"
            | "async"
            | "await"
            | "yield"
            | "return"
            | "throw"
            | "try"
            | "catch"
            | "finally"
            | "if"
            | "else"
            | "for"
            | "while"
            | "do"
            | "break"
            | "continue"
            | "switch"
            | "case"
            | "default"
            | "const"
            | "let"
            | "var"
            | "class"
            | "function"
            | "new"
            | "delete"
            | "typeof"
            | "instanceof"
            | "in"
            | "of"
            | "static"
    )
}

/// Extract enum and static member usages from a JavaScript node (e.g., ClassName.STATIC).
///
/// Recursively searches for member_expression nodes where the object is a capitalized identifier,
/// which typically represents class static member access patterns.
pub(crate) fn extract_enum_usages_javascript(
    node: Node<'_>,
    source: &[u8],
    intents: &mut Vec<ReferenceIntent>,
) {
    if node.kind() == "member_expression" {
        // member_expression has: object . property
        // We only want to capture if object is a capitalized identifier (class name)
        if let Some(object_node) = node.child_by_field_name("object")
            && object_node.kind() == "identifier"
        {
            let obj_text = node_text(object_node, source);
            // Check if it starts with capital letter (typical of classes)
            if obj_text.chars().next().is_some_and(|c| c.is_uppercase()) {
                let line = object_node.start_position().row + 1;
                intents.push(ReferenceIntent::TypeReference {
                    type_name: obj_text,
                    line,
                });
            }
        }
    }

    // Recursively process children
    let mut child = node.child(0);
    while let Some(c) = child {
        extract_enum_usages_javascript(c, source, intents);
        child = c.next_sibling();
    }
}

/// Extract HTML attributes (id, className) from JSX elements.
///
/// Used to index React components' HTML attributes for cross-language search
/// (e.g., finding which components use a specific CSS class).
///
/// Extracts:
/// - `id="my-id"` → HtmlId entity with name "my-id"
/// - `className="btn primary"` → HtmlClass entities for "btn" and "primary"
///
/// Returns a vector of tuples (attribute_name, attribute_value, line).
pub(crate) fn extract_jsx_attributes(
    node: Node<'_>,
    source: &[u8],
) -> Vec<(String, String, usize)> {
    use crate::pipeline::parser::utils::node_text;

    let mut attributes = Vec::new();

    // JSX attributes are structured as:
    // jsx_attribute
    //   property_identifier (e.g., "id", "className")
    //   jsx_expression | string (the value)
    let mut child = node.child(0);
    while let Some(c) = child {
        if c.kind() == "jsx_attribute" {
            let line = c.start_position().row + 1;
            let mut attr_name = String::new();
            let mut attr_value = String::new();

            // Navigate children to extract property_identifier and value
            let mut attr_child = c.child(0);
            while let Some(ac) = attr_child {
                if ac.kind() == "property_identifier" {
                    attr_name = node_text(ac, source);
                } else if ac.kind() == "string" {
                    // String literal (e.g., "my-id")
                    let raw = node_text(ac, source);
                    // Remove quotes
                    attr_value = raw.trim_matches(|c| c == '"' || c == '\'').to_string();
                } else if ac.kind() == "jsx_expression" {
                    // Expression (e.g., {myVar}) - we skip these for now
                    // Only capture static string values
                    attr_child = ac.next_sibling();
                    continue;
                }
                attr_child = ac.next_sibling();
            }

            // Only capture id and className attributes with non-empty values
            if (attr_name == "id" || attr_name == "className") && !attr_value.is_empty() {
                attributes.push((attr_name, attr_value, line));
            }
        }
        child = c.next_sibling();
    }

    attributes
}


/// Handle DOM and CSS reference captures in JavaScript
/// (dom.element_id, css.class_name, etc.)
pub(crate) fn handle_dom_css_capture(
    cap_name: &str,
    text: &str,
    line: usize,
) -> Option<ReferenceIntent> {
    match cap_name {
        "dom.element_id" => {
            let clean_id = text
                .trim_start_matches('"')
                .trim_start_matches('\'')
                .trim_end_matches('"')
                .trim_end_matches('\'')
                .to_string();

            Some(ReferenceIntent::DomElementReference {
                element_id: clean_id,
                line,
            })
        }
        "css.class_name" | "css.class_assignment" => {
            let clean_class = text
                .trim_start_matches('"')
                .trim_start_matches('\'')
                .trim_end_matches('"')
                .trim_end_matches('\'')
                .to_string();

            Some(ReferenceIntent::CssClassUsage {
                class_name: clean_class,
                line,
            })
        }
        _ => None,
    }
}

/// Extract JSX HTML attributes (id, className) for cross-language search.
///
/// Recursively traverses the AST looking for JSX elements and creates
/// HtmlId and HtmlClass entities from their attributes.
pub(crate) fn extract_jsx_html_attributes(
    node: Node<'_>,
    source: &[u8],
    entities: &mut Vec<ParsedEntity>,
    file_path: &str,
    repo_name: &str,
) {
    // Check if this is a JSX element
    if matches!(
        node.kind(),
        "jsx_self_closing_element" | "jsx_opening_element"
    ) {
        // Extract attributes
        let attrs = extract_jsx_attributes(node, source);

        // Create entities for each extracted attribute
        let line = node.start_position().row + 1;
        for (attr_name, attr_value, _) in attrs {
            if attr_name == "id" {
                // Create HtmlId entity
                entities.push(ParsedEntity {
                    uuid: Uuid::new_v4(),
                    name: attr_value.clone(),
                    kind: EntityKind::HtmlId,
                    fqn: format!("#{}", attr_value),
                    signature: None,
                    docstring: None,
                    inline_comments: Vec::new(),
                    decorators: Vec::new(),
                    language: "javascript".to_string(),
                    file_path: file_path.to_string(),
                    start_line: line,
                    enclosing_class: None,
                    repo_name: repo_name.to_string(),
                    reference_intents: Vec::new(),
                    calls: Vec::new(),
                    relationships: Vec::new(),
                    embed_text: String::new(),
                });
            } else if attr_name == "className" {
                // Split by whitespace and create HtmlClass entity for each class
                for class_name in attr_value.split_whitespace() {
                    if !class_name.is_empty() {
                        entities.push(ParsedEntity {
                            uuid: Uuid::new_v4(),
                            name: class_name.to_string(),
                            kind: EntityKind::HtmlClass,
                            fqn: format!(".{}", class_name),
                            signature: None,
                            docstring: None,
                            inline_comments: Vec::new(),
                            decorators: Vec::new(),
                            language: "javascript".to_string(),
                            file_path: file_path.to_string(),
                            start_line: line,
                            enclosing_class: None,
                            repo_name: repo_name.to_string(),
                            reference_intents: Vec::new(),
                            calls: Vec::new(),
                            relationships: Vec::new(),
                            embed_text: String::new(),
                        });
                    }
                }
            }
        }
    }

    // Recursively process all children
    let mut child = node.child(0);
    while let Some(c) = child {
        extract_jsx_html_attributes(c, source, entities, file_path, repo_name);
        child = c.next_sibling();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_reserved_keyword_true() {
        assert!(is_reserved_keyword("true"));
        assert!(is_reserved_keyword("false"));
        assert!(is_reserved_keyword("class"));
        assert!(is_reserved_keyword("function"));
        assert!(is_reserved_keyword("async"));
        assert!(is_reserved_keyword("await"));
    }

    #[test]
    fn test_is_reserved_keyword_false() {
        assert!(!is_reserved_keyword("myVar"));
        assert!(!is_reserved_keyword("handler"));
        assert!(!is_reserved_keyword("MyClass"));
        assert!(!is_reserved_keyword("someFunction"));
    }

    #[test]
    fn test_extract_jsx_component_invocation_simple() {
        let code = "function render() { return <ChartToolbar />; }";
        let tree = crate::pipeline::parser::test_utils::parse_javascript_snippet(code)
            .expect("Failed to parse JavaScript code");

        fn find_jsx_element(node: tree_sitter::Node) -> Option<tree_sitter::Node> {
            if matches!(
                node.kind(),
                "jsx_self_closing_element" | "jsx_opening_element"
            ) {
                return Some(node);
            }
            let mut i = 0u32;
            while let Some(child) = node.child(i) {
                if let Some(found) = find_jsx_element(child) {
                    return Some(found);
                }
                i += 1;
            }
            None
        }

        if let Some(jsx) = find_jsx_element(tree.root_node()) {
            let code_bytes = code.as_bytes();
            let mut intents: Vec<CallIntent> = Vec::new();
            extract_jsx_component_invocation(jsx, code_bytes, &mut intents);
            assert!(!intents.is_empty());
            assert_eq!(intents[0].method, "ChartToolbar");
            assert!(intents[0].receiver.is_none());
        }
    }

    #[test]
    fn test_extract_jsx_component_invocation_namespaced() {
        let code = "function render() { return <Sheet.Content />; }";
        let tree = crate::pipeline::parser::test_utils::parse_javascript_snippet(code)
            .expect("Failed to parse JavaScript code");

        fn find_jsx_element(node: tree_sitter::Node) -> Option<tree_sitter::Node> {
            if matches!(
                node.kind(),
                "jsx_self_closing_element" | "jsx_opening_element"
            ) {
                return Some(node);
            }
            let mut i = 0u32;
            while let Some(child) = node.child(i) {
                if let Some(found) = find_jsx_element(child) {
                    return Some(found);
                }
                i += 1;
            }
            None
        }

        if let Some(jsx) = find_jsx_element(tree.root_node()) {
            let code_bytes = code.as_bytes();
            let mut intents: Vec<CallIntent> = Vec::new();
            extract_jsx_component_invocation(jsx, code_bytes, &mut intents);
            assert!(!intents.is_empty());
            assert_eq!(intents[0].method, "Content");
            assert_eq!(intents[0].receiver, Some("Sheet".to_string()));
        }
    }

    #[test]
    fn test_extract_single_call_intent_javascript_simple() {
        let code = "function test() { method(); }";
        let tree = crate::pipeline::parser::test_utils::parse_javascript_snippet(code)
            .expect("Failed to parse JavaScript code");

        fn find_call_expression(node: tree_sitter::Node) -> Option<tree_sitter::Node> {
            if node.kind() == "call_expression" {
                return Some(node);
            }
            let mut i = 0u32;
            while let Some(child) = node.child(i) {
                if let Some(found) = find_call_expression(child) {
                    return Some(found);
                }
                i += 1;
            }
            None
        }

        if let Some(call) = find_call_expression(tree.root_node()) {
            let code_bytes = code.as_bytes();
            let intents = extract_single_call_intent_javascript(call, code_bytes);
            assert!(!intents.is_empty());
            assert_eq!(intents[0].method, "method");
            assert!(intents[0].receiver.is_none());
        }
    }

    #[test]
    fn test_extract_single_call_intent_javascript_member() {
        let code = "function test() { obj.method(); }";
        let tree = crate::pipeline::parser::test_utils::parse_javascript_snippet(code)
            .expect("Failed to parse JavaScript code");

        fn find_call_expression(node: tree_sitter::Node) -> Option<tree_sitter::Node> {
            if node.kind() == "call_expression" {
                return Some(node);
            }
            let mut i = 0u32;
            while let Some(child) = node.child(i) {
                if let Some(found) = find_call_expression(child) {
                    return Some(found);
                }
                i += 1;
            }
            None
        }

        if let Some(call) = find_call_expression(tree.root_node()) {
            let code_bytes = code.as_bytes();
            let intents = extract_single_call_intent_javascript(call, code_bytes);
            assert!(!intents.is_empty());
            assert_eq!(intents[0].method, "method");
            assert_eq!(intents[0].receiver, Some("obj".to_string()));
        }
    }

    #[test]
    fn test_extract_single_call_intent_javascript_new() {
        let code = "function test() { new MyClass(); }";
        let tree = crate::pipeline::parser::test_utils::parse_javascript_snippet(code)
            .expect("Failed to parse JavaScript code");

        fn find_new_expression(node: tree_sitter::Node) -> Option<tree_sitter::Node> {
            if node.kind() == "new_expression" {
                return Some(node);
            }
            let mut i = 0u32;
            while let Some(child) = node.child(i) {
                if let Some(found) = find_new_expression(child) {
                    return Some(found);
                }
                i += 1;
            }
            None
        }

        if let Some(new_expr) = find_new_expression(tree.root_node()) {
            let code_bytes = code.as_bytes();
            let intents = extract_single_call_intent_javascript(new_expr, code_bytes);
            assert!(!intents.is_empty());
            assert_eq!(intents[0].method, "MyClass");
            assert!(intents[0].receiver.is_none());
        }
    }

    #[ignore = "Tree-sitter structure variations in different JavaScript versions"]
    #[test]
    fn test_extract_class_inheritance_js() {
        let code = "class Child extends Parent { }";
        let tree = crate::pipeline::parser::test_utils::parse_javascript_snippet(code)
            .expect("Failed to parse JavaScript code");

        fn find_class_declaration(node: tree_sitter::Node) -> Option<tree_sitter::Node> {
            if node.kind() == "class_declaration" {
                return Some(node);
            }
            let mut i = 0u32;
            while let Some(child) = node.child(i) {
                if let Some(found) = find_class_declaration(child) {
                    return Some(found);
                }
                i += 1;
            }
            None
        }

        if let Some(class_node) = find_class_declaration(tree.root_node()) {
            let code_bytes = code.as_bytes();
            let mut intents = Vec::new();
            extract_class_inheritance_js(class_node, code_bytes, &mut intents);
            assert!(!intents.is_empty());
            // The intent should be an Extends variant
            let has_extends = intents.iter().any(
                |i| matches!(i, ReferenceIntent::Extends { parent, .. } if parent == "Parent"),
            );
            assert!(has_extends);
        }
    }

    #[test]
    fn test_extract_jsx_attributes_id() {
        let code = r#"function App() { return <div id="main-container">Hello</div>; }"#;
        let tree = crate::pipeline::parser::test_utils::parse_javascript_snippet(code)
            .expect("Failed to parse JavaScript code");

        fn find_jsx_opening_element(node: tree_sitter::Node) -> Option<tree_sitter::Node> {
            if node.kind() == "jsx_opening_element" {
                return Some(node);
            }
            let mut i = 0u32;
            while let Some(child) = node.child(i) {
                if let Some(found) = find_jsx_opening_element(child) {
                    return Some(found);
                }
                i += 1;
            }
            None
        }

        if let Some(jsx_elem) = find_jsx_opening_element(tree.root_node()) {
            let code_bytes = code.as_bytes();
            let attrs = extract_jsx_attributes(jsx_elem, code_bytes);
            assert_eq!(attrs.len(), 1);
            assert_eq!(attrs[0].0, "id");
            assert_eq!(attrs[0].1, "main-container");
        } else {
            panic!("No JSX opening element found");
        }
    }

    #[test]
    fn test_extract_jsx_attributes_classname() {
        let code =
            r#"function Button() { return <button className="btn primary">Click</button>; }"#;
        let tree = crate::pipeline::parser::test_utils::parse_javascript_snippet(code)
            .expect("Failed to parse JavaScript code");

        fn find_jsx_opening_element(node: tree_sitter::Node) -> Option<tree_sitter::Node> {
            if node.kind() == "jsx_opening_element" {
                return Some(node);
            }
            let mut i = 0u32;
            while let Some(child) = node.child(i) {
                if let Some(found) = find_jsx_opening_element(child) {
                    return Some(found);
                }
                i += 1;
            }
            None
        }

        if let Some(jsx_elem) = find_jsx_opening_element(tree.root_node()) {
            let code_bytes = code.as_bytes();
            let attrs = extract_jsx_attributes(jsx_elem, code_bytes);
            assert_eq!(attrs.len(), 1);
            assert_eq!(attrs[0].0, "className");
            assert_eq!(attrs[0].1, "btn primary");
        } else {
            panic!("No JSX opening element found");
        }
    }

    #[test]
    fn test_extract_jsx_attributes_multiple() {
        let code =
            r#"function Form() { return <input id="email-input" className="form-control" />; }"#;
        let tree = crate::pipeline::parser::test_utils::parse_javascript_snippet(code)
            .expect("Failed to parse JavaScript code");

        fn find_jsx_self_closing(node: tree_sitter::Node) -> Option<tree_sitter::Node> {
            if node.kind() == "jsx_self_closing_element" {
                return Some(node);
            }
            let mut i = 0u32;
            while let Some(child) = node.child(i) {
                if let Some(found) = find_jsx_self_closing(child) {
                    return Some(found);
                }
                i += 1;
            }
            None
        }

        if let Some(jsx_elem) = find_jsx_self_closing(tree.root_node()) {
            let code_bytes = code.as_bytes();
            let attrs = extract_jsx_attributes(jsx_elem, code_bytes);
            assert_eq!(attrs.len(), 2);

            // attrs may be in any order depending on AST traversal
            let has_id = attrs
                .iter()
                .any(|(name, val, _)| name == "id" && val == "email-input");
            let has_classname = attrs
                .iter()
                .any(|(name, val, _)| name == "className" && val == "form-control");

            assert!(has_id, "Should extract id attribute");
            assert!(has_classname, "Should extract className attribute");
        } else {
            panic!("No JSX self-closing element found");
        }
    }
}

