use crate::models::EntityKind;

/// Map an [`EntityKind`] to its Neo4j node label string.
#[expect(
    clippy::too_many_lines,
    reason = "Massive match statement is inherently long but readable"
)]
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
        EntityKind::KotlinEnum => "KotlinEnum",
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
        EntityKind::PythonClass => "PythonClass",
        EntityKind::PythonFunction => "PythonFunction",
        EntityKind::PythonMethod => "PythonMethod",
        EntityKind::PythonModule => "PythonModule",
        EntityKind::PythonConstant => "PythonConstant",
        EntityKind::BuildDependency => "BuildDependency",
        EntityKind::BuildPlugin => "BuildPlugin",
        EntityKind::BuildTask => "BuildTask",
        EntityKind::PipelineStage => "PipelineStage",
        EntityKind::PipelineStep => "PipelineStep",
        EntityKind::GroovyClass => "GroovyClass",
        EntityKind::GroovyInterface => "GroovyInterface",
        EntityKind::GroovyTrait => "GroovyTrait",
        EntityKind::GroovyMethod => "GroovyMethod",
        EntityKind::GroovyFunction => "GroovyFunction",
        EntityKind::GroovyEnum => "GroovyEnum",
        EntityKind::GroovyProperty => "GroovyProperty",
        EntityKind::CStruct => "CStruct",
        EntityKind::CFunction => "CFunction",
        EntityKind::CppClass => "CppClass",
        EntityKind::CppMethod => "CppMethod",
        EntityKind::CppNamespace => "CppNamespace",
        EntityKind::MacroDefinition => "MacroDefinition",
        EntityKind::CSharpClass => "CSharpClass",
        EntityKind::CSharpInterface => "CSharpInterface",
        EntityKind::CSharpStruct => "CSharpStruct",
        EntityKind::CSharpRecord => "CSharpRecord",
        EntityKind::CSharpEnum => "CSharpEnum",
        EntityKind::CSharpMethod => "CSharpMethod",
        EntityKind::CSharpConstructor => "CSharpConstructor",
        EntityKind::CSharpProperty => "CSharpProperty",
        EntityKind::CSharpField => "CSharpField",
        EntityKind::CSharpConstant => "CSharpConstant",
        EntityKind::CSharpDelegate => "CSharpDelegate",
        EntityKind::CSharpEvent => "CSharpEvent",
        EntityKind::CSharpIndexer => "CSharpIndexer",
        EntityKind::CSharpOperator => "CSharpOperator",
        EntityKind::CSharpNamespace => "CSharpNamespace",
        EntityKind::CSharpLocalFunction => "CSharpLocalFunction",
        EntityKind::CargoPackage => "CargoPackage",
        EntityKind::CargoFeature => "CargoFeature",
        EntityKind::WorkspaceMember => "WorkspaceMember",
        EntityKind::ConfigProperty => "ConfigProperty",
        EntityKind::K8sDeployment => "K8sDeployment",
        EntityKind::K8sService => "K8sService",
        EntityKind::K8sConfigMap => "K8sConfigMap",
        EntityKind::K8sSecret => "K8sSecret",
        EntityKind::K8sIngress => "K8sIngress",
        EntityKind::K8sNamespace => "K8sNamespace",
        EntityKind::K8sResource => "K8sResource",
        EntityKind::HelmChart => "HelmChart",
        EntityKind::HelmValue => "HelmValue",
        EntityKind::HelmTemplateVar => "HelmTemplateVar",
        EntityKind::ProjectIdentity => "ProjectIdentity",
        EntityKind::VclVersion => "VclVersion",
        EntityKind::VclSubroutine => "VclSubroutine",
        EntityKind::VclBuiltinSub => "VclBuiltinSub",
        EntityKind::VclBackend => "VclBackend",
        EntityKind::VclProbe => "VclProbe",
        EntityKind::VclAcl => "VclAcl",
        EntityKind::VclImport => "VclImport",
        EntityKind::VclObjectInstance => "VclObjectInstance",
        EntityKind::VtcTestCase => "VtcTestCase",
        EntityKind::VtcServer => "VtcServer",
        EntityKind::VtcClient => "VtcClient",
        EntityKind::VtcVarnishInstance => "VtcVarnishInstance",
        EntityKind::VtcLogexpect => "VtcLogexpect",
        EntityKind::VtcBarrier => "VtcBarrier",
        EntityKind::VccModule => "VccModule",
        EntityKind::VccFunction => "VccFunction",
        EntityKind::VccObject => "VccObject",
        EntityKind::VccMethod => "VccMethod",
        EntityKind::MarkdownDocument => "MarkdownDocument",
        EntityKind::MarkdownSection => "MarkdownSection",
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
    #[expect(clippy::too_many_lines, reason = "Test cases are inherently long")]
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
            EntityKind::PythonClass,
            EntityKind::PythonFunction,
            EntityKind::PythonMethod,
            EntityKind::PythonModule,
            EntityKind::PythonConstant,
            EntityKind::BuildDependency,
            EntityKind::BuildPlugin,
            EntityKind::BuildTask,
            EntityKind::PipelineStage,
            EntityKind::PipelineStep,
            EntityKind::GroovyClass,
            EntityKind::GroovyInterface,
            EntityKind::GroovyTrait,
            EntityKind::GroovyMethod,
            EntityKind::GroovyFunction,
            EntityKind::GroovyEnum,
            EntityKind::GroovyProperty,
            EntityKind::CargoPackage,
            EntityKind::CargoFeature,
            EntityKind::WorkspaceMember,
            EntityKind::ConfigProperty,
            EntityKind::K8sDeployment,
            EntityKind::K8sService,
            EntityKind::K8sConfigMap,
            EntityKind::K8sSecret,
            EntityKind::K8sIngress,
            EntityKind::K8sNamespace,
            EntityKind::K8sResource,
            EntityKind::HelmChart,
            EntityKind::HelmValue,
            EntityKind::HelmTemplateVar,
            EntityKind::ProjectIdentity,
            EntityKind::VclVersion,
            EntityKind::VclSubroutine,
            EntityKind::VclBuiltinSub,
            EntityKind::VclBackend,
            EntityKind::VclProbe,
            EntityKind::VclAcl,
            EntityKind::VclImport,
            EntityKind::VclObjectInstance,
            EntityKind::VtcTestCase,
            EntityKind::VtcServer,
            EntityKind::VtcClient,
            EntityKind::VtcVarnishInstance,
            EntityKind::VtcLogexpect,
            EntityKind::VtcBarrier,
            EntityKind::VccModule,
            EntityKind::VccFunction,
            EntityKind::VccObject,
            EntityKind::VccMethod,
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
            "PythonClass",
            "PythonFunction",
            "PythonMethod",
            "PythonModule",
            "PythonConstant",
            "BuildDependency",
            "BuildPlugin",
            "BuildTask",
            "PipelineStage",
            "PipelineStep",
            "GroovyClass",
            "GroovyInterface",
            "GroovyTrait",
            "GroovyMethod",
            "GroovyFunction",
            "GroovyEnum",
            "GroovyProperty",
            "CargoPackage",
            "CargoFeature",
            "WorkspaceMember",
            "ConfigProperty",
            "K8sDeployment",
            "K8sService",
            "K8sConfigMap",
            "K8sSecret",
            "K8sIngress",
            "K8sNamespace",
            "K8sResource",
            "HelmChart",
            "HelmValue",
            "HelmTemplateVar",
            "ProjectIdentity",
            "VclVersion",
            "VclSubroutine",
            "VclBuiltinSub",
            "VclBackend",
            "VclProbe",
            "VclAcl",
            "VclImport",
            "VclObjectInstance",
            "VtcTestCase",
            "VtcServer",
            "VtcClient",
            "VtcVarnishInstance",
            "VtcLogexpect",
            "VtcBarrier",
            "VccModule",
            "VccFunction",
            "VccObject",
            "VccMethod",
        ];

        for (variant, expected_label) in variants.iter().zip(expected_labels.iter()) {
            assert_eq!(kind_to_label(variant), *expected_label);
        }
    }
}
