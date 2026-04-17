//! CSS and SCSS entity extraction and handling.
//!
//! Handles capture processing for CSS/SCSS specific entity kinds:
//! - CSS classes, IDs, variables
//! - SCSS variables, mixins, functions

use crate::models::{EntityKind, ReferenceIntent};
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

/// Process CSS class usage references (e.g., classList.add('btn-primary'))
#[allow(dead_code)]
pub(crate) fn handle_css_class_usage(text: &str, line: usize) -> ReferenceIntent {
    let clean_class = text
        .trim_start_matches('"')
        .trim_start_matches('\'')
        .trim_end_matches('"')
        .trim_end_matches('\'')
        .to_string();

    ReferenceIntent::CssClassUsage {
        class_name: clean_class,
        line,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_handle_css_class() {
        let node = unsafe {
            // Create a dummy node for testing - in real code, this comes from tree-sitter
            std::mem::zeroed()
        };
        let result = handle_css_capture("css.class", ".btn-primary", node);

        assert!(result.is_some());
        let (name, kind, _) = result.unwrap();
        assert_eq!(name, "btn-primary");
        assert_eq!(kind, EntityKind::CssClass);
    }

    #[test]
    fn test_handle_css_id() {
        let node = unsafe { std::mem::zeroed() };
        let result = handle_css_capture("css.id", "#header", node);

        assert!(result.is_some());
        let (name, kind, _) = result.unwrap();
        assert_eq!(name, "header");
        assert_eq!(kind, EntityKind::CssId);
    }

    #[test]
    fn test_handle_scss_mixin() {
        let node = unsafe { std::mem::zeroed() };
        let result = handle_css_capture("scss.mixin", "flex-center", node);

        assert!(result.is_some());
        let (name, kind, _) = result.unwrap();
        assert_eq!(name, "flex-center");
        assert_eq!(kind, EntityKind::ScssMixin);
    }

    #[test]
    fn test_handle_css_class_usage() {
        let intent = handle_css_class_usage("'btn-primary'", 10);

        match intent {
            ReferenceIntent::CssClassUsage { class_name, line } => {
                assert_eq!(class_name, "btn-primary");
                assert_eq!(line, 10);
            }
            _ => panic!("Expected CssClassUsage"),
        }
    }
}
