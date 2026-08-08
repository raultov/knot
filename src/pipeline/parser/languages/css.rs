//! CSS and SCSS entity extraction and handling.
//!
//! Handles capture processing for CSS/SCSS specific entity kinds:
//! - CSS classes, IDs, variables
//! - SCSS variables, mixins, functions

use crate::models::EntityKind;
use tree_sitter::Node;

/// Process a CSS or SCSS capture and extract entity information.
///
/// Returns (name, kind, start_line) or None if the capture is not a named entity.
pub(crate) fn handle_css_capture(
    cap_name: &str,
    text: &str,
    node: Node<'_>,
) -> Option<(String, EntityKind, usize)> {
    let start_line = node.start_position().row + 1;

    match cap_name {
        "css.class" => {
            let mut clean_name = text.to_string();
            if clean_name.starts_with('.') {
                clean_name.remove(0);
            }
            Some((clean_name, EntityKind::CssClass, start_line))
        }
        "css.id" => {
            let mut clean_name = text.to_string();
            if clean_name.starts_with('#') {
                clean_name.remove(0);
            }
            Some((clean_name, EntityKind::CssId, start_line))
        }
        "css.variable" => {
            let mut clean_name = text.to_string();
            if clean_name.starts_with("--") {
                clean_name = clean_name[2..].to_string();
            }
            Some((clean_name, EntityKind::CssVariable, start_line))
        }
        "scss.mixin" => Some((text.to_string(), EntityKind::ScssMixin, start_line)),
        "scss.function" => Some((text.to_string(), EntityKind::ScssFunction, start_line)),
        "scss.variable" => {
            let mut clean_name = text.to_string();
            if clean_name.starts_with('$') {
                clean_name.remove(0);
            }
            Some((clean_name, EntityKind::ScssVariable, start_line))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tree_sitter::Parser;

    fn parse_css(source: &str) -> tree_sitter::Tree {
        let mut parser = Parser::new();
        parser
            .set_language(&tree_sitter_css::LANGUAGE.into())
            .expect("failed to load CSS grammar");
        parser.parse(source, None).expect("failed to parse CSS")
    }

    #[test]
    fn test_handle_css_class() {
        let tree = parse_css(".btn-primary {}");
        let node = tree.root_node();
        let result = handle_css_capture("css.class", ".btn-primary", node);

        assert!(result.is_some());
        let (name, kind, start_line) = result.unwrap();
        assert_eq!(name, "btn-primary");
        assert_eq!(kind, EntityKind::CssClass);
        assert_eq!(start_line, 1);
    }

    #[test]
    fn test_handle_css_id() {
        let tree = parse_css("#header {}");
        let node = tree.root_node();
        let result = handle_css_capture("css.id", "#header", node);

        assert!(result.is_some());
        let (name, kind, start_line) = result.unwrap();
        assert_eq!(name, "header");
        assert_eq!(kind, EntityKind::CssId);
        assert_eq!(start_line, 1);
    }

    #[test]
    fn test_handle_scss_mixin() {
        let tree = parse_css("@mixin flex-center {}");
        let node = tree.root_node();
        let result = handle_css_capture("scss.mixin", "flex-center", node);

        assert!(result.is_some());
        let (name, kind, start_line) = result.unwrap();
        assert_eq!(name, "flex-center");
        assert_eq!(kind, EntityKind::ScssMixin);
        assert_eq!(start_line, 1);
    }
}
