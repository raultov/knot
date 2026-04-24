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
        EntityKind::KotlinClass => "KotlinClass",
        EntityKind::KotlinInterface => "KotlinInterface",
        EntityKind::KotlinObject => "KotlinObject",
        EntityKind::KotlinCompanionObject => "KotlinCompanionObject",
        EntityKind::KotlinFunction => "KotlinFunction",
        EntityKind::KotlinMethod => "KotlinMethod",
        EntityKind::KotlinProperty => "KotlinProperty",
        EntityKind::HtmlElement => "HtmlElement",
        EntityKind::HtmlId => "HtmlId",
        EntityKind::HtmlClass => "HtmlClass",
        EntityKind::CssClass => "CssClass",
        EntityKind::CssId => "CssId",
        EntityKind::CssVariable => "CssVariable",
        EntityKind::ScssVariable => "ScssVariable",
        EntityKind::ScssMixin => "ScssMixin",
        EntityKind::ScssFunction => "ScssFunction",
        EntityKind::RustStruct => "RustStruct",
        EntityKind::RustEnum => "RustEnum",
        EntityKind::RustUnion => "RustUnion",
        EntityKind::RustTrait => "RustTrait",
        EntityKind::RustImpl => "RustImpl",
        EntityKind::RustFunction => "RustFunction",
        EntityKind::RustMethod => "RustMethod",
        EntityKind::RustMacroDef => "RustMacroDef",
        EntityKind::RustTypeAlias => "RustTypeAlias",
        EntityKind::RustConstant => "RustConstant",
        EntityKind::RustStatic => "RustStatic",
        EntityKind::RustModule => "RustModule",
        EntityKind::RustMacroInvoke => "RustMacroInvoke",
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
    fn test_kind_to_label_css_class() {
        assert_eq!(kind_to_label(&EntityKind::CssClass), "CssClass");
    }

    #[test]
    fn test_kind_to_label_css_id() {
        assert_eq!(kind_to_label(&EntityKind::CssId), "CssId");
    }

    #[test]
    fn test_kind_to_label_css_variable() {
        assert_eq!(kind_to_label(&EntityKind::CssVariable), "CssVariable");
    }

    #[test]
    fn test_kind_to_label_scss_variable() {
        assert_eq!(kind_to_label(&EntityKind::ScssVariable), "ScssVariable");
    }

    #[test]
    fn test_kind_to_label_scss_mixin() {
        assert_eq!(kind_to_label(&EntityKind::ScssMixin), "ScssMixin");
    }

    #[test]
    fn test_kind_to_label_scss_function() {
        assert_eq!(kind_to_label(&EntityKind::ScssFunction), "ScssFunction");
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
            EntityKind::CssClass,
            EntityKind::CssId,
            EntityKind::CssVariable,
            EntityKind::ScssVariable,
            EntityKind::ScssMixin,
            EntityKind::ScssFunction,
            EntityKind::RustStruct,
            EntityKind::RustEnum,
            EntityKind::RustUnion,
            EntityKind::RustTrait,
            EntityKind::RustImpl,
            EntityKind::RustFunction,
            EntityKind::RustMethod,
            EntityKind::RustMacroDef,
            EntityKind::RustTypeAlias,
            EntityKind::RustConstant,
            EntityKind::RustStatic,
            EntityKind::RustModule,
            EntityKind::RustMacroInvoke,
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
            "CssClass",
            "CssId",
            "CssVariable",
            "ScssVariable",
            "ScssMixin",
            "ScssFunction",
            "RustStruct",
            "RustEnum",
            "RustUnion",
            "RustTrait",
            "RustImpl",
            "RustFunction",
            "RustMethod",
            "RustMacroDef",
            "RustTypeAlias",
            "RustConstant",
            "RustStatic",
            "RustModule",
            "RustMacroInvoke",
        ];

        for (variant, expected_label) in variants.iter().zip(expected_labels.iter()) {
            assert_eq!(kind_to_label(variant), *expected_label);
        }
    }
}
