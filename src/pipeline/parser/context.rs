use super::languages::rust;
use super::utils::*;
use crate::models::EntityKind;
use tree_sitter::Node;

/// Helper struct to track class context for FQN computation.
#[derive(Debug, Clone)]
pub(crate) struct ClassContext {
    pub(crate) name: String,
    pub(crate) start_line: usize,
    pub(crate) end_line: usize,
}

/// Extract all class/interface/object declarations and their line ranges.
pub(crate) fn extract_class_contexts(
    node: Node<'_>,
    source: &[u8],
    contexts: &mut Vec<ClassContext>,
) {
    if matches!(
        node.kind(),
        "class_declaration"
            | "interface_declaration"
            | "abstract_class_declaration"
            | "class_definition" // Python
            | "object_declaration"
    ) {
        // Find the name child
        let mut child = node.child(0);
        let mut class_name: Option<String> = None;
        while let Some(c) = child {
            if c.kind() == "identifier" || c.kind() == "type_identifier" {
                class_name = Some(node_text(c, source));
                break;
            }
            child = c.next_sibling();
        }

        if let Some(name) = class_name {
            contexts.push(ClassContext {
                name,
                start_line: node.start_position().row + 1,
                end_line: node.end_position().row + 1,
            });
        }
    } else if node.kind() == "impl_item"
        && let Some(self_type) = rust::extract_impl_self_type(node, source)
    {
        // Rust `impl Foo` and `impl Trait for Foo` establish a class context
        // for the self-type. The trait name (e.g. `LogSink`) is ignored — we
        // only want the type being implemented, so methods inside get the
        // FQN `Foo::method` rather than `LogSink::method`.
        contexts.push(ClassContext {
            name: self_type,
            start_line: node.start_position().row + 1,
            end_line: node.end_position().row + 1,
        });
    }

    // Recursively process children
    let mut child = node.child(0);
    while let Some(c) = child {
        extract_class_contexts(c, source, contexts);
        child = c.next_sibling();
    }
}

/// Compute FQN and enclosing_class based on entity context.
pub(crate) fn compute_fqn_and_context(
    name: &str,
    kind: &EntityKind,
    start_line: usize,
    _lang_name: &str,
    class_contexts: &[ClassContext],
) -> (String, Option<String>) {
    // Find which class contains this entity (if any)
    let enclosing_class = class_contexts
        .iter()
        .find(|ctx| start_line > ctx.start_line && start_line < ctx.end_line)
        .map(|ctx| ctx.name.clone());

    // Compute FQN
    let fqn = match kind {
        EntityKind::Class
        | EntityKind::Interface
        | EntityKind::KotlinClass
        | EntityKind::KotlinInterface
        | EntityKind::KotlinEnum
        | EntityKind::CppClass
        | EntityKind::CStruct
        | EntityKind::CppNamespace => {
            // For Java/Kotlin/C++, include enclosing class for nested declarations
            // For C++, the FQN will be updated dynamically in the extractor later
            if let Some(class_name) = &enclosing_class {
                format!("{}.{}", class_name, name)
            } else {
                name.to_string()
            }
        }
        EntityKind::Method | EntityKind::KotlinMethod | EntityKind::CppMethod => {
            // Method FQN: ClassName.methodName
            if let Some(class_name) = &enclosing_class {
                format!("{}.{}", class_name, name)
            } else {
                name.to_string()
            }
        }
        EntityKind::Function | EntityKind::KotlinFunction | EntityKind::CFunction => {
            // Top-level function - just the function name
            name.to_string()
        }
        EntityKind::Constant | EntityKind::KotlinProperty => {
            // Constant FQN: ClassName.CONST_NAME or just CONST_NAME for top-level
            if let Some(class_name) = &enclosing_class {
                format!("{}.{}", class_name, name)
            } else {
                name.to_string()
            }
        }
        EntityKind::Enum => {
            // Enum FQN: EnumName or ClassName.EnumName if nested
            if let Some(class_name) = &enclosing_class {
                format!("{}.{}", class_name, name)
            } else {
                name.to_string()
            }
        }
        // HTML entities already have their FQN computed in the parser
        // (e.g., "#id-name", ".class-name", "<custom-element>")
        EntityKind::HtmlElement | EntityKind::HtmlId | EntityKind::HtmlClass => name.to_string(),
        // CSS entities: FQN is the selector/variable name
        EntityKind::CssClass => format!(".{}", name),
        EntityKind::CssId => format!("#{}", name),
        EntityKind::CssVariable => format!("--{}", name),
        // SCSS entities: FQN is the variable/mixin/function name with prefix
        EntityKind::ScssVariable => format!("${}", name),
        EntityKind::ScssMixin => format!("@mixin {}", name),
        EntityKind::ScssFunction => format!("@function {}", name),
        // Kotlin-specific entities that don't nest like classes
        EntityKind::KotlinObject | EntityKind::KotlinCompanionObject => {
            // For nested objects/companions, include enclosing class name
            if let Some(class_name) = &enclosing_class {
                format!("{}.{}", class_name, name)
            } else {
                name.to_string()
            }
        }
        // Rust entities
        EntityKind::RustStruct
        | EntityKind::RustEnum
        | EntityKind::RustUnion
        | EntityKind::RustTrait
        | EntityKind::RustImpl
        | EntityKind::RustFunction
        | EntityKind::RustMacroDef
        | EntityKind::RustTypeAlias
        | EntityKind::RustConstant
        | EntityKind::RustStatic
        | EntityKind::RustModule => name.to_string(),
        EntityKind::RustMethod => {
            if let Some(class_name) = &enclosing_class {
                format!("{}::{}", class_name, name)
            } else {
                name.to_string()
            }
        }
        EntityKind::RustMacroInvoke => format!("{}!", name),
        // Python entities
        EntityKind::PythonClass
        | EntityKind::PythonFunction
        | EntityKind::PythonModule
        | EntityKind::PythonConstant => name.to_string(),
        EntityKind::PythonMethod => {
            if let Some(class_name) = &enclosing_class {
                format!("{}.{}", class_name, name)
            } else {
                name.to_string()
            }
        }
        // Build Systems & CI/CD entities — use name as FQN
        EntityKind::BuildDependency
        | EntityKind::BuildPlugin
        | EntityKind::BuildTask
        | EntityKind::PipelineStage
        | EntityKind::PipelineStep => name.to_string(),
        // Cargo (Rust build system) entities
        EntityKind::CargoPackage | EntityKind::CargoFeature | EntityKind::WorkspaceMember => {
            name.to_string()
        }
        // Configuration entities
        EntityKind::ConfigProperty => name.to_string(),
        // Kubernetes entities
        EntityKind::K8sDeployment
        | EntityKind::K8sService
        | EntityKind::K8sConfigMap
        | EntityKind::K8sSecret
        | EntityKind::K8sIngress
        | EntityKind::K8sNamespace
        | EntityKind::K8sResource => name.to_string(),
        // Helm entities
        EntityKind::HelmChart | EntityKind::HelmValue | EntityKind::HelmTemplateVar => {
            name.to_string()
        }
        // Project identity for cross-repo linking
        EntityKind::ProjectIdentity => name.to_string(),
        // Groovy entities
        EntityKind::GroovyClass
        | EntityKind::GroovyInterface
        | EntityKind::GroovyTrait
        | EntityKind::GroovyFunction
        | EntityKind::GroovyEnum
        | EntityKind::GroovyProperty => name.to_string(),
        EntityKind::GroovyMethod => {
            if let Some(class_name) = &enclosing_class {
                format!("{}.{}", class_name, name)
            } else {
                name.to_string()
            }
        }
        EntityKind::MacroDefinition => name.to_string(),
    };

    (fqn, enclosing_class)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compute_fqn_and_context_class() {
        let contexts = vec![];
        let (fqn, enclosing_class) =
            compute_fqn_and_context("MyClass", &EntityKind::Class, 10, "java", &contexts);
        assert_eq!(fqn, "MyClass");
        assert!(enclosing_class.is_none());
    }

    #[test]
    fn test_compute_fqn_and_context_method_with_class() {
        let contexts = vec![ClassContext {
            name: "MyClass".to_string(),
            start_line: 5,
            end_line: 20,
        }];
        let (fqn, enclosing_class) =
            compute_fqn_and_context("myMethod", &EntityKind::Method, 10, "java", &contexts);
        assert_eq!(fqn, "MyClass.myMethod");
        assert_eq!(enclosing_class, Some("MyClass".to_string()));
    }

    #[test]
    fn test_compute_fqn_and_context_method_without_class() {
        let contexts = vec![];
        let (fqn, enclosing_class) =
            compute_fqn_and_context("myFunction", &EntityKind::Method, 10, "java", &contexts);
        assert_eq!(fqn, "myFunction");
        assert!(enclosing_class.is_none());
    }

    #[test]
    fn test_compute_fqn_and_context_function() {
        let contexts = vec![];
        let (fqn, enclosing_class) = compute_fqn_and_context(
            "topLevelFunction",
            &EntityKind::Function,
            10,
            "typescript",
            &contexts,
        );
        assert_eq!(fqn, "topLevelFunction");
        assert!(enclosing_class.is_none());
    }

    #[test]
    fn test_compute_fqn_and_context_constant_with_class() {
        let contexts = vec![ClassContext {
            name: "Constants".to_string(),
            start_line: 1,
            end_line: 50,
        }];
        let (fqn, enclosing_class) =
            compute_fqn_and_context("CONST_VALUE", &EntityKind::Constant, 25, "java", &contexts);
        assert_eq!(fqn, "Constants.CONST_VALUE");
        assert_eq!(enclosing_class, Some("Constants".to_string()));
    }

    #[test]
    fn test_compute_fqn_and_context_enum() {
        let contexts = vec![];
        let (fqn, enclosing_class) =
            compute_fqn_and_context("Color", &EntityKind::Enum, 1, "java", &contexts);
        assert_eq!(fqn, "Color");
        assert!(enclosing_class.is_none());
    }

    #[test]
    fn test_extract_class_contexts_java() {
        let code = "public class TestClass { }\npublic interface TestInterface { }";
        let tree = crate::pipeline::parser::test_utils::parse_java_snippet(code)
            .expect("Failed to parse Java code");

        let source = code.as_bytes();
        let mut contexts: Vec<ClassContext> = Vec::new();
        extract_class_contexts(tree.root_node(), source, &mut contexts);

        assert_eq!(contexts.len(), 2);
        assert!(contexts.iter().any(|c| c.name == "TestClass"));
        assert!(contexts.iter().any(|c| c.name == "TestInterface"));
    }

    #[test]
    fn test_extract_class_contexts_nested() {
        let code = "class Outer { \n  class Inner { } \n}";
        let tree = crate::pipeline::parser::test_utils::parse_typescript_snippet(code)
            .expect("Failed to parse TypeScript code");

        let source = code.as_bytes();
        let mut contexts: Vec<ClassContext> = Vec::new();
        extract_class_contexts(tree.root_node(), source, &mut contexts);

        // Both outer and inner classes should be captured
        assert!(!contexts.is_empty());
        assert!(contexts.iter().any(|c| c.name == "Outer"));
    }

    #[test]
    fn test_extract_class_contexts_kotlin_object() {
        let code = "object NodeUtils { fun stream() {} }";
        let tree = crate::pipeline::parser::test_utils::parse_kotlin_snippet(code)
            .expect("Failed to parse Kotlin code");

        let source = code.as_bytes();
        let mut contexts: Vec<ClassContext> = Vec::new();
        extract_class_contexts(tree.root_node(), source, &mut contexts);

        assert_eq!(contexts.len(), 1, "Expected 1 context for object NodeUtils");
        assert!(contexts.iter().any(|c| c.name == "NodeUtils"));
    }

    #[test]
    fn test_extract_class_contexts_kotlin_companion() {
        // tree-sitter-kotlin-ng v1.1.0 requires newlines in class body for companion_object.
        let code = "class Foo {\n    companion object {\n        fun bar() {}\n    }\n}";
        let tree = crate::pipeline::parser::test_utils::parse_kotlin_snippet(code)
            .expect("Failed to parse Kotlin code");

        let source = code.as_bytes();
        let mut contexts: Vec<ClassContext> = Vec::new();
        extract_class_contexts(tree.root_node(), source, &mut contexts);

        // Class Foo is captured via class_declaration.
        // companion_object does NOT create its own context (parent class already covers it).
        assert!(
            contexts.iter().any(|c| c.name == "Foo"),
            "Expected class Foo to be captured as context. Found: {:?}",
            contexts.iter().map(|c| &c.name).collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_compute_fqn_and_context_kotlin_method_in_object() {
        let contexts = vec![ClassContext {
            name: "NodeUtils".to_string(),
            start_line: 1,
            end_line: 3,
        }];
        let (fqn, enclosing_class) =
            compute_fqn_and_context("stream", &EntityKind::KotlinMethod, 2, "kotlin", &contexts);
        assert_eq!(fqn, "NodeUtils.stream");
        assert_eq!(enclosing_class, Some("NodeUtils".to_string()));
    }

    #[test]
    fn test_compute_fqn_and_context_kotlin_object_nested() {
        let contexts = vec![ClassContext {
            name: "OuterClass".to_string(),
            start_line: 1,
            end_line: 50,
        }];
        let (fqn, enclosing_class) = compute_fqn_and_context(
            "InnerObject",
            &EntityKind::KotlinObject,
            25,
            "kotlin",
            &contexts,
        );
        assert_eq!(fqn, "OuterClass.InnerObject");
        assert_eq!(enclosing_class, Some("OuterClass".to_string()));
    }

    #[test]
    fn test_compute_fqn_and_context_kotlin_function_top_level() {
        let contexts = vec![];
        let (fqn, enclosing_class) = compute_fqn_and_context(
            "greetUser",
            &EntityKind::KotlinFunction,
            5,
            "kotlin",
            &contexts,
        );
        assert_eq!(fqn, "greetUser");
        assert!(
            enclosing_class.is_none(),
            "Top-level functions should have no enclosing class"
        );
    }

    #[test]
    fn test_extract_class_contexts_includes_rust_impl_item() {
        // `impl Foo { ... }` should register Foo as a class context, so
        // methods inside get the qualified FQN `Foo::method`.
        let code = r#"
struct Foo;
impl Foo {
    pub fn new() -> Self { Foo }
}
"#;
        let tree = crate::pipeline::parser::test_utils::parse_rust_snippet(code)
            .expect("Failed to parse Rust code");

        let source = code.as_bytes();
        let mut contexts: Vec<ClassContext> = Vec::new();
        extract_class_contexts(tree.root_node(), source, &mut contexts);

        let foo_ctx = contexts
            .iter()
            .find(|c| c.name == "Foo")
            .expect("Expected ClassContext for Foo from impl_item");
        // The context must encompass the inner fn (line 4 in the snippet).
        assert!(foo_ctx.start_line <= 4 && foo_ctx.end_line >= 4);
    }

    #[test]
    fn test_extract_class_contexts_rust_impl_trait_for_uses_self_type() {
        // `impl Bar for Foo` should use `Foo` (self-type), NOT `Bar` (trait).
        let code = r#"
trait Bar { fn new() -> Self; }
struct Foo;
impl Bar for Foo {
    fn new() -> Self { Foo }
}
"#;
        let tree = crate::pipeline::parser::test_utils::parse_rust_snippet(code)
            .expect("Failed to parse Rust code");

        let source = code.as_bytes();
        let mut contexts: Vec<ClassContext> = Vec::new();
        extract_class_contexts(tree.root_node(), source, &mut contexts);

        // Must include Foo (self-type) …
        assert!(
            contexts.iter().any(|c| c.name == "Foo"),
            "Expected self-type Foo to appear as a context, got: {:?}",
            contexts.iter().map(|c| &c.name).collect::<Vec<_>>()
        );
        // … and must NOT include Bar (trait) as a class context, otherwise
        // methods inside would incorrectly get FQN `Bar::new`.
        assert!(
            !contexts.iter().any(|c| c.name == "Bar"),
            "Trait name Bar should NOT appear as a class context, got: {:?}",
            contexts.iter().map(|c| &c.name).collect::<Vec<_>>()
        );
    }
}
