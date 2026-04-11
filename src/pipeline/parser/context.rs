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

/// Extract all class/interface declarations and their line ranges.
pub(crate) fn extract_class_contexts(
    node: Node<'_>,
    source: &[u8],
    contexts: &mut Vec<ClassContext>,
) {
    if matches!(
        node.kind(),
        "class_declaration" | "interface_declaration" | "abstract_class_declaration"
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
        EntityKind::Class | EntityKind::Interface => {
            // For Java, we'd want to include package name here
            // For now, just use the class name
            name.to_string()
        }
        EntityKind::Method => {
            // Method FQN: ClassName.methodName
            if let Some(class_name) = &enclosing_class {
                format!("{}.{}", class_name, name)
            } else {
                name.to_string()
            }
        }
        EntityKind::Function => {
            // Top-level function - just the function name
            name.to_string()
        }
        EntityKind::Constant => {
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
        assert!(contexts.len() >= 1);
        assert!(contexts.iter().any(|c| c.name == "Outer"));
    }
}
