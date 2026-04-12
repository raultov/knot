//! HTML/Angular template extraction for custom elements and attributes.

use crate::models::{EntityKind, ParsedEntity};
use crate::pipeline::parser::utils::node_text;
use tree_sitter::Node;
use uuid::Uuid;

/// Extract string value from quoted attribute value node
#[allow(dead_code)]
fn extract_string_value(value_node: Node<'_>, source: &[u8]) -> String {
    let text = node_text(value_node, source);
    // Remove quotes if present
    text.trim_matches(|c| c == '"' || c == '\'').to_string()
}

/// Extract id and class attributes from HTML elements
fn extract_html_attribute(
    attr_node: Node<'_>,
    source: &[u8],
    entities: &mut Vec<ParsedEntity>,
    line: usize,
    file_path: &str,
    repo_name: &str,
) {
    // Find attribute_name child
    let mut attr_name = String::new();
    let mut attr_value = String::new();

    let mut child = attr_node.child(0);
    while let Some(c) = child {
        match c.kind() {
            "attribute_name" => {
                attr_name = node_text(c, source);
            }
            "quoted_attribute_value" => {
                // Extract the actual value (without quotes)
                let mut value_child = c.child(0);
                while let Some(vc) = value_child {
                    if vc.kind() == "attribute_value" {
                        attr_value = node_text(vc, source);
                        break;
                    }
                    value_child = vc.next_sibling();
                }
            }
            _ => {}
        }
        child = c.next_sibling();
    }

    if attr_name == "id" && !attr_value.is_empty() {
        entities.push(ParsedEntity {
            uuid: Uuid::new_v4(),
            name: attr_value.clone(),
            kind: EntityKind::HtmlId,
            fqn: format!("#{}", attr_value),
            signature: None,
            docstring: None,
            inline_comments: Vec::new(),
            decorators: Vec::new(),
            language: "html".to_string(),
            file_path: file_path.to_string(),
            start_line: line,
            enclosing_class: None,
            repo_name: repo_name.to_string(),
            reference_intents: Vec::new(),
            calls: Vec::new(),
            relationships: Vec::new(),
            embed_text: String::new(),
        });
    } else if attr_name == "class" && !attr_value.is_empty() {
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
                    language: "html".to_string(),
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

/// Recursively extract custom elements and attributes from HTML AST
fn extract_html_elements(
    node: Node<'_>,
    source: &[u8],
    entities: &mut Vec<ParsedEntity>,
    file_path: &str,
    repo_name: &str,
) {
    if node.kind() == "element" {
        let line = node.start_position().row + 1;

        // Extract start_tag and its children
        let mut child = node.child(0);
        while let Some(c) = child {
            if c.kind() == "start_tag" {
                // Find tag_name and attributes
                let mut tag_child = c.child(0);
                while let Some(tc) = tag_child {
                    match tc.kind() {
                        "tag_name" => {
                            let tag_name = node_text(tc, source);
                            if tag_name.contains('-') {
                                // Web Component / Angular Component
                                entities.push(ParsedEntity {
                                    uuid: Uuid::new_v4(),
                                    name: tag_name.clone(),
                                    kind: EntityKind::HtmlElement,
                                    fqn: format!("<{}>", tag_name),
                                    signature: None,
                                    docstring: None,
                                    inline_comments: Vec::new(),
                                    decorators: Vec::new(),
                                    language: "html".to_string(),
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
                        "attribute" => {
                            extract_html_attribute(
                                tc, source, entities, line, file_path, repo_name,
                            );
                        }
                        _ => {}
                    }
                    tag_child = tc.next_sibling();
                }
            }
            child = c.next_sibling();
        }
    }

    // Recurse to children
    let mut child = node.child(0);
    while let Some(c) = child {
        extract_html_elements(c, source, entities, file_path, repo_name);
        child = c.next_sibling();
    }
}

/// Extract entities from HTML files (Angular templates, Web Components)
pub(crate) fn extract_entities_html(
    root: Node<'_>,
    source: &[u8],
    file_path: &str,
    repo_name: &str,
) -> Vec<ParsedEntity> {
    let mut entities = Vec::new();
    extract_html_elements(root, source, &mut entities, file_path, repo_name);
    entities
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_html_snippet(source: &str) -> tree_sitter::Tree {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_html::LANGUAGE.into())
            .expect("Failed to load HTML grammar");
        parser.parse(source, None).expect("Failed to parse HTML")
    }

    #[test]
    fn test_extract_custom_elements() {
        let source = r#"
<app-user-profile id="profile-main" class="card shadow">
  <p>User content</p>
</app-user-profile>
        "#;

        let tree = parse_html_snippet(source);
        let entities = extract_entities_html(
            tree.root_node(),
            source.as_bytes(),
            "/test/template.html",
            "test_repo",
        );

        // Should find 1 custom element, 1 id, and 2 classes
        let elements: Vec<_> = entities
            .iter()
            .filter(|e| e.kind == EntityKind::HtmlElement)
            .collect();
        assert_eq!(elements.len(), 1);
        assert_eq!(elements[0].name, "app-user-profile");

        let ids: Vec<_> = entities
            .iter()
            .filter(|e| e.kind == EntityKind::HtmlId)
            .collect();
        assert_eq!(ids.len(), 1);
        assert_eq!(ids[0].name, "profile-main");

        let classes: Vec<_> = entities
            .iter()
            .filter(|e| e.kind == EntityKind::HtmlClass)
            .collect();
        assert_eq!(classes.len(), 2);
        assert!(classes.iter().any(|c| c.name == "card"));
        assert!(classes.iter().any(|c| c.name == "shadow"));
    }

    #[test]
    fn test_extract_id_attributes() {
        let source = r#"<div id="app-root" class="container">Content</div>"#;

        let tree = parse_html_snippet(source);
        let entities = extract_entities_html(
            tree.root_node(),
            source.as_bytes(),
            "/test/index.html",
            "test_repo",
        );

        let ids: Vec<_> = entities
            .iter()
            .filter(|e| e.kind == EntityKind::HtmlId)
            .collect();
        assert_eq!(ids.len(), 1);
        assert_eq!(ids[0].name, "app-root");
        assert_eq!(ids[0].fqn, "#app-root");
    }

    #[test]
    fn test_split_class_attributes() {
        let source = r#"<div class="btn btn-primary btn-lg">Button</div>"#;

        let tree = parse_html_snippet(source);
        let entities = extract_entities_html(
            tree.root_node(),
            source.as_bytes(),
            "/test/button.html",
            "test_repo",
        );

        let classes: Vec<_> = entities
            .iter()
            .filter(|e| e.kind == EntityKind::HtmlClass)
            .collect();
        assert_eq!(classes.len(), 3);
        assert!(classes.iter().any(|c| c.name == "btn"));
        assert!(classes.iter().any(|c| c.name == "btn-primary"));
        assert!(classes.iter().any(|c| c.name == "btn-lg"));

        // Verify FQN format
        assert!(classes.iter().all(|c| c.fqn.starts_with('.')));
    }

    #[test]
    fn test_ignore_standard_html_elements() {
        let source = r#"<div><span>Content</span></div>"#;

        let tree = parse_html_snippet(source);
        let entities = extract_entities_html(
            tree.root_node(),
            source.as_bytes(),
            "/test/standard.html",
            "test_repo",
        );

        // No custom elements (div, span have no hyphens)
        let elements: Vec<_> = entities
            .iter()
            .filter(|e| e.kind == EntityKind::HtmlElement)
            .collect();
        assert_eq!(elements.len(), 0);
    }

    #[test]
    fn test_extract_multiple_custom_elements() {
        let source = r#"
<my-header id="header">
  <my-nav class="main-nav"></my-nav>
</my-header>
<my-footer class="footer sticky"></my-footer>
        "#;

        let tree = parse_html_snippet(source);
        let entities = extract_entities_html(
            tree.root_node(),
            source.as_bytes(),
            "/test/layout.html",
            "test_repo",
        );

        let elements: Vec<_> = entities
            .iter()
            .filter(|e| e.kind == EntityKind::HtmlElement)
            .collect();
        assert_eq!(elements.len(), 3);
        assert!(elements.iter().any(|e| e.name == "my-header"));
        assert!(elements.iter().any(|e| e.name == "my-nav"));
        assert!(elements.iter().any(|e| e.name == "my-footer"));

        let ids: Vec<_> = entities
            .iter()
            .filter(|e| e.kind == EntityKind::HtmlId)
            .collect();
        assert_eq!(ids.len(), 1);
        assert_eq!(ids[0].name, "header");

        let classes: Vec<_> = entities
            .iter()
            .filter(|e| e.kind == EntityKind::HtmlClass)
            .collect();
        assert_eq!(classes.len(), 3);
        assert!(classes.iter().any(|c| c.name == "main-nav"));
        assert!(classes.iter().any(|c| c.name == "footer"));
        assert!(classes.iter().any(|c| c.name == "sticky"));
    }
}
