use crate::models::EntityKind;

/// Map an [`EntityKind`] to its Neo4j node label string.
pub(crate) fn kind_to_label(kind: &EntityKind) -> &'static str {
    match kind {
        EntityKind::Class => "Class",
        EntityKind::Interface => "Interface",
        EntityKind::Method => "Method",
        EntityKind::Function => "Function",
        EntityKind::Constant => "Constant",
        EntityKind::Enum => "Enum",
        EntityKind::HtmlElement => "HtmlElement",
        EntityKind::HtmlId => "HtmlId",
        EntityKind::HtmlClass => "HtmlClass",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_kind_to_label_class() {
        assert_eq!(kind_to_label(&EntityKind::Class), "Class");
    }

    #[test]
    fn test_kind_to_label_interface() {
        assert_eq!(kind_to_label(&EntityKind::Interface), "Interface");
    }

    #[test]
    fn test_kind_to_label_method() {
        assert_eq!(kind_to_label(&EntityKind::Method), "Method");
    }

    #[test]
    fn test_kind_to_label_function() {
        assert_eq!(kind_to_label(&EntityKind::Function), "Function");
    }

    #[test]
    fn test_kind_to_label_constant() {
        assert_eq!(kind_to_label(&EntityKind::Constant), "Constant");
    }

    #[test]
    fn test_kind_to_label_enum() {
        assert_eq!(kind_to_label(&EntityKind::Enum), "Enum");
    }

    #[test]
    fn test_kind_to_label_html_element() {
        assert_eq!(kind_to_label(&EntityKind::HtmlElement), "HtmlElement");
    }

    #[test]
    fn test_kind_to_label_html_id() {
        assert_eq!(kind_to_label(&EntityKind::HtmlId), "HtmlId");
    }

    #[test]
    fn test_kind_to_label_html_class() {
        assert_eq!(kind_to_label(&EntityKind::HtmlClass), "HtmlClass");
    }

    #[test]
    fn test_kind_to_label_all_variants() {
        let variants = [
            EntityKind::Class,
            EntityKind::Interface,
            EntityKind::Method,
            EntityKind::Function,
            EntityKind::Constant,
            EntityKind::Enum,
            EntityKind::HtmlElement,
            EntityKind::HtmlId,
            EntityKind::HtmlClass,
        ];

        let expected_labels = [
            "Class",
            "Interface",
            "Method",
            "Function",
            "Constant",
            "Enum",
            "HtmlElement",
            "HtmlId",
            "HtmlClass",
        ];

        for (variant, expected_label) in variants.iter().zip(expected_labels.iter()) {
            assert_eq!(kind_to_label(variant), *expected_label);
        }
    }
}
