//! FQN construction for C#.
//!
//! Two mechanisms are required (plan §3.2) because neither the Java nor the
//! C++ model suffices alone:
//!
//! 1. [`extract_file_scoped_namespace`] — a file-level pre-pass for
//!    `file_scoped_namespace_declaration` (C# 10+). That node has **no
//!    `body` field**, so types declared after it are *siblings* under
//!    `compilation_unit`, not descendants; a parent-walk from an entity node
//!    will never reach it. It must be discovered by scanning
//!    `compilation_unit` children once per file.
//!
//! 2. [`build_csharp_fqn_prefix`] — an ancestor walk from the entity node
//!    collecting block-form `namespace X { … }` declarations (which *do*
//!    have a `body` and nest) and containing types, joined outermost-first
//!    with `.`.

use crate::pipeline::parser::utils::node_text;
use tree_sitter::Node;

/// File-scoped namespace prefix (C# 10+). Scans direct children of
/// `compilation_unit` for `file_scoped_namespace_declaration`, which has no
/// `body` field — subsequent types are siblings, not descendants.
///
/// Returns the declared namespace text verbatim, e.g. `MyApp.Services`.
/// Returns `None` for files using the block form or with no namespace.
pub(crate) fn extract_file_scoped_namespace(root: Node<'_>, source: &[u8]) -> Option<String> {
    let mut child = root.child(0);
    while let Some(c) = child {
        if c.kind() == "file_scoped_namespace_declaration"
            && let Some(name_node) = c.child_by_field_name("name")
        {
            return Some(node_text(name_node, source));
        }
        child = c.next_sibling();
    }
    None
}

/// Dotted chain of enclosing block namespaces and containing types, built by
/// walking ancestors from the entity node. Returns `None` at file scope.
///
/// The walk starts at `node.parent()` so an entity never includes itself;
/// for a nested type (`class Inner` inside `class Outer`) the result ends at
/// `Outer`, and the caller appends the entity's own name.
///
/// Qualified namespace names (`namespace MyApp.Legacy { … }`) contribute a
/// single segment (`MyApp.Legacy`), matching the source text.
pub(crate) fn build_csharp_fqn_prefix(node: Node<'_>, source: &[u8]) -> Option<String> {
    let mut parts: Vec<String> = Vec::new();
    let mut current = node.parent();

    while let Some(parent) = current {
        match parent.kind() {
            "namespace_declaration" | "file_scoped_namespace_declaration" => {
                if let Some(name_node) = parent.child_by_field_name("name") {
                    parts.push(node_text(name_node, source));
                }
            }
            "class_declaration"
            | "interface_declaration"
            | "struct_declaration"
            | "record_declaration"
            | "enum_declaration" => {
                if let Some(name_node) = parent.child_by_field_name("name") {
                    parts.push(node_text(name_node, source));
                }
            }
            _ => {}
        }
        current = parent.parent();
    }

    if parts.is_empty() {
        None
    } else {
        parts.reverse();
        Some(parts.join("."))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_file_scoped_namespace_found() {
        let code = "using System;\nnamespace MyApp.Services;\n\nclass Foo {}";
        let tree = crate::pipeline::parser::test_utils::parse_csharp_snippet(code)
            .expect("Failed to parse C# code");
        let ns = extract_file_scoped_namespace(tree.root_node(), code.as_bytes());
        assert_eq!(ns, Some("MyApp.Services".to_string()));
    }

    #[test]
    fn test_file_scoped_namespace_none_for_block_form() {
        let code = "namespace MyApp.Legacy\n{\n    class Foo {}\n}";
        let tree = crate::pipeline::parser::test_utils::parse_csharp_snippet(code)
            .expect("Failed to parse C# code");
        let ns = extract_file_scoped_namespace(tree.root_node(), code.as_bytes());
        assert_eq!(ns, None);
    }

    #[test]
    fn test_file_scoped_namespace_none_without_namespace() {
        let code = "class Foo {}";
        let tree = crate::pipeline::parser::test_utils::parse_csharp_snippet(code)
            .expect("Failed to parse C# code");
        let ns = extract_file_scoped_namespace(tree.root_node(), code.as_bytes());
        assert_eq!(ns, None);
    }

    #[test]
    fn test_fqn_prefix_namespaces_and_types() {
        let code = r#"
namespace MyApp.Legacy
{
    namespace Deep
    {
        public class OldStyle
        {
            public class Inner
            {
                public int Depth { get; set; }
            }
        }
    }
}
"#;
        let tree = crate::pipeline::parser::test_utils::parse_csharp_snippet(code)
            .expect("Failed to parse C# code");
        let source = code.as_bytes();

        // From the `Depth` property: prefix must be
        // MyApp.Legacy.Deep.OldStyle.Inner (namespaces outermost-first).
        let mut found: Option<Node<'_>> = None;
        let mut stack = vec![tree.root_node()];
        while let Some(n) = stack.pop() {
            if n.kind() == "property_declaration" {
                found = Some(n);
                break;
            }
            let mut child = n.child(0);
            while let Some(c) = child {
                stack.push(c);
                child = c.next_sibling();
            }
        }
        let prop = found.expect("property_declaration not found");
        let prefix = build_csharp_fqn_prefix(prop, source);
        assert_eq!(
            prefix,
            Some("MyApp.Legacy.Deep.OldStyle.Inner".to_string()),
            "ancestor walk must yield namespaces then containing types"
        );
    }

    #[test]
    fn test_fqn_prefix_none_for_top_level_type() {
        let code = "class Free {}";
        let tree = crate::pipeline::parser::test_utils::parse_csharp_snippet(code)
            .expect("Failed to parse C# code");
        let class_node = crate::pipeline::parser::test_utils::find_first_node(
            tree.root_node(),
            &["class_declaration"],
        )
        .expect("class_declaration not found");
        let prefix = build_csharp_fqn_prefix(class_node, code.as_bytes());
        assert_eq!(prefix, None, "top-level type has no enclosing scope");
    }

    #[test]
    fn test_fqn_prefix_single_block_namespace() {
        let code = "namespace App\n{\n    class Foo { void Bar() {} }\n}";
        let tree = crate::pipeline::parser::test_utils::parse_csharp_snippet(code)
            .expect("Failed to parse C# code");
        let method = crate::pipeline::parser::test_utils::find_first_node(
            tree.root_node(),
            &["method_declaration"],
        )
        .expect("method_declaration not found");
        let prefix = build_csharp_fqn_prefix(method, code.as_bytes());
        assert_eq!(
            prefix,
            Some("App.Foo".to_string()),
            "walk must yield namespace then containing type"
        );
    }
}
