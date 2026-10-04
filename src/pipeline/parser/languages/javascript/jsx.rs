use crate::models::{CallIntent, EntityKind, ParsedEntity};
use crate::pipeline::parser::utils::{is_capitalized, node_text};
use tree_sitter::Node;

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
                entities.push(ParsedEntity::new(
                    attr_value.clone(),
                    EntityKind::HtmlId,
                    format!("#{}", attr_value),
                    None,
                    None,
                    "javascript",
                    file_path,
                    line,
                    line,
                    None,
                    repo_name,
                ));
            } else if attr_name == "className" {
                // Split by whitespace and create HtmlClass entity for each class
                for class_name in attr_value.split_whitespace() {
                    if !class_name.is_empty() {
                        entities.push(ParsedEntity::new(
                            class_name,
                            EntityKind::HtmlClass,
                            format!(".{}", class_name),
                            None,
                            None,
                            "javascript",
                            file_path,
                            line,
                            line,
                            None,
                            repo_name,
                        ));
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
    use tree_sitter::Parser;

    #[test]
    fn test_jsx_markup_entities_uuids_are_deterministic() {
        let mut parser = Parser::new();
        let language = tree_sitter_javascript::LANGUAGE;
        parser
            .set_language(&language.into())
            .expect("Error loading JS grammar");

        let source = r#"
const App = () => <div id="app-id" className="cls1 cls2" />;
        "#;
        let tree1 = parser.parse(source, None).unwrap();
        let mut entities1 = Vec::new();
        extract_jsx_html_attributes(
            tree1.root_node(),
            source.as_bytes(),
            &mut entities1,
            "App.jsx",
            "test_repo",
        );

        let tree2 = parser.parse(source, None).unwrap();
        let mut entities2 = Vec::new();
        extract_jsx_html_attributes(
            tree2.root_node(),
            source.as_bytes(),
            &mut entities2,
            "App.jsx",
            "test_repo",
        );

        assert_eq!(entities1.len(), 3); // 1 id, 2 classes
        assert_eq!(entities1.len(), entities2.len());

        for (e1, e2) in entities1.iter().zip(entities2.iter()) {
            assert_eq!(
                e1.uuid, e2.uuid,
                "UUID should be deterministic for JSX entity {}",
                e1.name
            );
        }
    }
}
