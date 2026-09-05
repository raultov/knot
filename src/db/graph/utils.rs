use crate::models::EntityKind;

/// Map an [`EntityKind`] to its Neo4j node label string.
///
/// Every label is the variant's own name — exactly what the derived `Debug`
/// impl renders — so the mapping is generated instead of listed.
pub(crate) fn kind_to_label(kind: &EntityKind) -> String {
    format!("{kind:?}")
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
    fn test_kind_to_label_csharp() {
        let cases = [
            (EntityKind::CSharpClass, "CSharpClass"),
            (EntityKind::CSharpInterface, "CSharpInterface"),
            (EntityKind::CSharpStruct, "CSharpStruct"),
            (EntityKind::CSharpRecord, "CSharpRecord"),
            (EntityKind::CSharpEnum, "CSharpEnum"),
            (EntityKind::CSharpMethod, "CSharpMethod"),
            (EntityKind::CSharpConstructor, "CSharpConstructor"),
            (EntityKind::CSharpProperty, "CSharpProperty"),
            (EntityKind::CSharpField, "CSharpField"),
            (EntityKind::CSharpConstant, "CSharpConstant"),
            (EntityKind::CSharpDelegate, "CSharpDelegate"),
            (EntityKind::CSharpEvent, "CSharpEvent"),
            (EntityKind::CSharpIndexer, "CSharpIndexer"),
            (EntityKind::CSharpOperator, "CSharpOperator"),
            (EntityKind::CSharpNamespace, "CSharpNamespace"),
            (EntityKind::CSharpLocalFunction, "CSharpLocalFunction"),
        ];
        for (kind, label) in cases {
            assert_eq!(kind_to_label(&kind), label);
        }
    }

    #[test]
    fn test_kind_to_label_all_variants() {
        // kind_to_label derives every label from the variant's own name
        // (`Debug`), so representative spot-checks per language family
        // guard against accidental format changes.
        let cases = [
            (EntityKind::Class, "Class"),
            (EntityKind::Interface, "Interface"),
            (EntityKind::KotlinCompanionObject, "KotlinCompanionObject"),
            (EntityKind::HtmlId, "HtmlId"),
            (EntityKind::CssVariable, "CssVariable"),
            (EntityKind::ScssMixin, "ScssMixin"),
            (EntityKind::RustTypeAlias, "RustTypeAlias"),
            (EntityKind::PythonConstant, "PythonConstant"),
            (EntityKind::GroovyTrait, "GroovyTrait"),
            (EntityKind::CppClass, "CppClass"),
            (EntityKind::CSharpLocalFunction, "CSharpLocalFunction"),
            (EntityKind::CargoPackage, "CargoPackage"),
            (EntityKind::K8sConfigMap, "K8sConfigMap"),
            (EntityKind::HelmTemplateVar, "HelmTemplateVar"),
            (EntityKind::ProjectIdentity, "ProjectIdentity"),
            (EntityKind::VtcVarnishInstance, "VtcVarnishInstance"),
            (EntityKind::VccMethod, "VccMethod"),
            (EntityKind::MarkdownSection, "MarkdownSection"),
        ];
        for (kind, label) in cases {
            assert_eq!(kind_to_label(&kind), label);
        }
    }
}
