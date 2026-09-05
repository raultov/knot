use crate::models::{CallIntent, EntityKind, ParsedEntity};
use crate::pipeline::parser::utils::{is_capitalized, node_text};
use tree_sitter::Node;
use uuid::Uuid;

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
        if is_capitalized(&comp_name) {
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
                    arg_count: None,
                });
            } else {
                // Simple component name
                intents.push(CallIntent {
                    method: comp_name,
                    receiver: None,
                    line,
                    arg_count: None,
                });
            }
        }
        // HTML tags (lowercase first letter) are intentionally skipped
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
                    end_line: line,
                    enclosing_class: None,
                    repo_name: repo_name.to_string(),
                    reference_intents: Vec::new(),
                    calls: Vec::new(),
                    relationships: Vec::new(),
                    embed_text: String::new(),
                    rust_attributes: None,
                    impl_trait: None,
                    impl_target: None,
                    generics: None,
                    lifetimes: None,
                    alias_module_path: None,
                    original_export_name: None,
                    enclosing_class_fqn: None,
                    default_export: None,
                    is_test_context: false,
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
                            end_line: line,
                            enclosing_class: None,
                            repo_name: repo_name.to_string(),
                            reference_intents: Vec::new(),
                            calls: Vec::new(),
                            relationships: Vec::new(),
                            embed_text: String::new(),
                            rust_attributes: None,
                            impl_trait: None,
                            impl_target: None,
                            generics: None,
                            lifetimes: None,
                            alias_module_path: None,
                            original_export_name: None,
                            enclosing_class_fqn: None,
                            default_export: None,
                            is_test_context: false,
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
