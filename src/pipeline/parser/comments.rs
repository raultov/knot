use super::context::ClassContext;
use super::utils::*;
use crate::models::EntityKind;
use tree_sitter::Node;

/// Extract docstring (preceding comments) and inline_comments from an entity node.
///
/// **Docstring extraction (upward pass):**
/// - Walks backward from the node using `prev_sibling()` to find comment nodes
/// - Groups consecutive comment lines together (tolerates single blank lines)
/// - Stops when hitting non-comment, non-whitespace nodes
/// - Handles both `/* */` block comments and `//` line comments
///
/// **Inline comments extraction (downward pass):**
/// - Walks the entity's body looking for comment nodes
/// - For classes: captures comments in the class body but NOT inside nested methods
/// - For methods/functions: captures comments in the method/function body
/// - Aggregates all found comments into a list
#[expect(
    clippy::excessive_nesting,
    reason = "function is verbose but correct — extraction deferred"
)]
pub(crate) fn extract_comments(
    entity_node: Node<'_>,
    source: &[u8],
    lang_name: &str,
    kind: &EntityKind,
    _class_contexts: &[ClassContext],
) -> (Option<String>, Vec<String>) {
    let mut docstring: Option<String> = None;
    let mut inline_comments: Vec<String> = Vec::new();

    // **Upward pass (Docstring):** Find preceding comments
    if let Some(_parent) = entity_node.parent() {
        let mut current = entity_node.prev_sibling();
        let mut comment_buffer: Vec<String> = Vec::new();

        while let Some(node) = current {
            match node.kind() {
                "line_comment" => {
                    // For Rust, the actual doc text is in a child `doc_comment` node
                    let text = if lang_name == "rust" {
                        // Try to find the doc_comment child node by iterating through children
                        let mut doc_text = None;
                        for i in 0..node.child_count() {
                            if let Some(child) = node.child(i as u32)
                                && child.kind() == "doc_comment"
                            {
                                doc_text = Some(node_text(child, source));
                                break;
                            }
                        }
                        // If found, use the doc_comment text; otherwise fallback to full node
                        doc_text.unwrap_or_else(|| node_text(node, source))
                    } else {
                        node_text(node, source)
                    };
                    comment_buffer.insert(0, strip_comment_markers(&text));
                    current = node.prev_sibling();
                }
                "comment" | "block_comment" => {
                    let text = node_text(node, source);
                    let stripped = strip_comment_markers(&text);
                    // C# XML doc comments wrap prose in tags (`<summary>`,
                    // `</summary>`, …). Stripping the known doc tags keeps
                    // the actual prose (which is what gets displayed and
                    // embedded) instead of a leading `<summary>` line.
                    let stripped = if lang_name == "csharp" {
                        strip_xml_doc_tags(&stripped)
                    } else {
                        stripped
                    };
                    comment_buffer.insert(0, stripped);
                    current = node.prev_sibling();
                }
                // Allow single blank lines between comments
                _ if node.utf8_text(source).unwrap_or("").trim().is_empty() => {
                    // Check if there's a comment further back
                    if let Some(next) = node.prev_sibling()
                        && matches!(next.kind(), "comment" | "line_comment" | "block_comment")
                    {
                        current = Some(next);
                        continue;
                    }
                    break;
                }
                _ => break,
            }
        }

        if !comment_buffer.is_empty() {
            let combined = comment_buffer
                .iter()
                .filter(|s| !s.trim().is_empty())
                .cloned()
                .collect::<Vec<_>>()
                .join("\n");
            if !combined.trim().is_empty() {
                docstring = Some(combined);
            }
        }
    }

    // **Downward pass (Inline Comments):** Find comments within the entity body
    // Build a set of extracted entity nodes to avoid capturing their comments
    let extracted_child_entities = extract_child_entity_nodes(entity_node, lang_name);

    extract_inline_comments_recursive(
        entity_node,
        source,
        kind,
        &extracted_child_entities,
        &mut inline_comments,
    );

    (docstring, inline_comments)
}

/// Extract decorators/annotations from an entity node.
///
/// **TypeScript:** Looks for `decorator` nodes preceding the entity.
/// Example: `@OnEvent('foo')` or `@Override`
///
/// **Java:** Looks for `annotation` nodes in the modifiers.
/// Examples: `@Override`, `@GetMapping("/path")`, `@OnEvent('foo')`
///
/// Returns a vector of decorator strings (e.g., `["@Override", "@OnEvent('foo')"]`).
#[expect(
    clippy::cognitive_complexity,
    reason = "function is verbose but correct — extraction deferred"
)]
#[expect(
    clippy::excessive_nesting,
    reason = "function is verbose but correct — extraction deferred"
)]
pub(crate) fn extract_decorators(
    entity_node: Node<'_>,
    source: &[u8],
    lang_name: &str,
) -> Vec<String> {
    let mut decorators: Vec<String> = Vec::new();

    if lang_name == "typescript" {
        // For TypeScript: decorators are separate nodes that precede the declaration
        // Look for decorator nodes that come before this entity
        if let Some(parent) = entity_node.parent() {
            let mut child = parent.child(0);
            let entity_line = entity_node.start_position().row;
            let mut found_decorator_section = false;

            while let Some(c) = child {
                let child_line = c.start_position().row;

                // Stop once we've passed the entity
                if child_line >= entity_line {
                    break;
                }

                if c.kind() == "decorator" {
                    let decorator_text = node_text(c, source);
                    if !decorator_text.is_empty() {
                        decorators.push(decorator_text);
                        found_decorator_section = true;
                    }
                } else if found_decorator_section
                    && !c.utf8_text(source).unwrap_or("").trim().is_empty()
                {
                    // Stop collecting decorators if we hit a non-decorator, non-whitespace node
                    if !matches!(c.kind(), "comment" | "line_comment" | "block_comment") {
                        break;
                    }
                }

                child = c.next_sibling();
            }
        }
    } else if lang_name == "java" {
        // For Java: annotations are in the modifiers section
        let mut child = entity_node.child(0);
        while let Some(c) = child {
            if c.kind() == "modifiers" {
                // Extract annotations from the modifiers node
                let mut modifier_child = c.child(0);
                while let Some(mc) = modifier_child {
                    if matches!(mc.kind(), "annotation" | "marker_annotation") {
                        let annotation_text = node_text(mc, source);
                        if !annotation_text.is_empty() {
                            decorators.push(annotation_text);
                        }
                    }
                    modifier_child = mc.next_sibling();
                }
            }
            child = c.next_sibling();
        }
    } else if lang_name == "kotlin" {
        // For Kotlin: annotations are in the modifiers section (similar to Java)
        let mut child = entity_node.child(0);
        while let Some(c) = child {
            if c.kind() == "modifiers" {
                // Extract annotations from the modifiers node
                let mut modifier_child = c.child(0);
                while let Some(mc) = modifier_child {
                    if mc.kind() == "annotation" {
                        let annotation_text = node_text(mc, source);
                        if !annotation_text.is_empty() {
                            decorators.push(annotation_text);
                        }
                    }
                    modifier_child = mc.next_sibling();
                }
            }
            child = c.next_sibling();
        }
    } else if lang_name == "csharp" {
        // For C#: attributes live in `attribute_list` children of the
        // declaration — NOT inside a `modifiers` node as in Java/Kotlin —
        // so this cannot reuse either existing branch.
        // Example: `[Obsolete("Use UserServiceV2 instead")]`
        let mut child = entity_node.child(0);
        while let Some(c) = child {
            if c.kind() == "attribute_list" {
                let mut attr_child = c.child(0);
                while let Some(ac) = attr_child {
                    if ac.kind() == "attribute" {
                        let attribute_text = node_text(ac, source);
                        if !attribute_text.is_empty() {
                            decorators.push(attribute_text);
                        }
                    }
                    attr_child = ac.next_sibling();
                }
            }
            child = c.next_sibling();
        }
    }

    decorators
}

/// Extract all child method/function/class declarations within a node.
/// Used to prevent parent entities from capturing comments of their children.
pub(crate) fn extract_child_entity_nodes<'a>(node: Node<'a>, lang_name: &str) -> Vec<Node<'a>> {
    let mut children = Vec::new();
    let mut child = node.child(0);

    let entity_kinds = if lang_name == "java" {
        vec![
            "method_declaration",
            "class_declaration",
            "interface_declaration",
        ]
    } else if lang_name == "csharp" {
        // C# child declarations whose comments belong to the child, not the
        // enclosing type.
        vec![
            "method_declaration",
            "constructor_declaration",
            "property_declaration",
            "class_declaration",
            "interface_declaration",
            "struct_declaration",
            "record_declaration",
            "enum_declaration",
        ]
    } else {
        vec![
            "method_definition",
            "method_signature",
            "abstract_method_signature",
            "function_declaration",
            "class_declaration",
            "abstract_class_declaration",
            "interface_declaration",
            "lexical_declaration",
            "export_statement",
        ]
    };

    while let Some(c) = child {
        if entity_kinds.contains(&c.kind()) {
            children.push(c);
        }
        child = c.next_sibling();
    }

    children
}

/// Recursively extract inline comments from within an entity's body,
/// skipping over any child entity declarations.
pub(crate) fn extract_inline_comments_recursive(
    node: Node<'_>,
    source: &[u8],
    _kind: &EntityKind,
    extracted_children: &[Node<'_>],
    comments: &mut Vec<String>,
) {
    if matches!(node.kind(), "comment" | "line_comment" | "block_comment") {
        let text = node_text(node, source);
        let cleaned = strip_comment_markers(&text);
        if !cleaned.trim().is_empty() {
            comments.push(cleaned);
        }
        return;
    }

    // Skip child entities to avoid capturing their comments
    if extracted_children
        .iter()
        .any(|child| child.id() == node.id())
    {
        return;
    }

    let mut child = node.child(0);
    while let Some(c) = child {
        extract_inline_comments_recursive(c, source, _kind, extracted_children, comments);
        child = c.next_sibling();
    }
}

/// Remove common comment markers from a doc-comment string.
///
/// Strips `/**`, `*/`, leading `*`, `//`, `///`, and surrounding whitespace.
pub(crate) fn strip_comment_markers(raw: &str) -> String {
    raw.lines()
        .map(|line| {
            let trimmed = line.trim();
            let trimmed = trimmed.strip_prefix("/**").unwrap_or(trimmed);
            let trimmed = trimmed.strip_prefix("/*").unwrap_or(trimmed);
            let trimmed = trimmed.strip_suffix("*/").unwrap_or(trimmed);
            let trimmed = trimmed.strip_prefix("///").unwrap_or(trimmed);
            let trimmed = trimmed.strip_prefix("//").unwrap_or(trimmed);
            let trimmed = trimmed.strip_prefix('*').unwrap_or(trimmed);
            trimmed.trim()
        })
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

/// XML documentation tags recognized by [`strip_xml_doc_tags`]. Tags whose
/// name (ignoring `/` closers, `?` declarations, and attributes) is in this
/// set are removed together with their angle brackets; unknown text in angle
/// brackets (e.g. a generic `List<T>` mention in prose) is preserved.
const XML_DOC_TAGS: &[&str] = &[
    "summary",
    "remarks",
    "param",
    "typeparam",
    "returns",
    "value",
    "example",
    "exception",
    "permission",
    "see",
    "seealso",
    "paramref",
    "typeparamref",
    "cref",
    "para",
    "c",
    "code",
    "list",
    "item",
    "term",
    "description",
    "inheritdoc",
    "include",
    "br",
    "b",
    "i",
    "langword",
];

/// Strip XML doc-comment tags (`<summary>`, `</summary>`, `<param name="x">`,
/// `<see cref="Y"/>`, …) from a C# doc string so the prose survives.
pub(crate) fn strip_xml_doc_tags(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut rest = raw;

    while let Some(start) = rest.find('<') {
        let Some(end_offset) = rest[start..].find('>') else {
            break;
        };
        let end = start + end_offset;
        out.push_str(&rest[..start]);

        let tag = &rest[start + 1..end];
        let tag_name = tag
            .trim()
            .trim_start_matches('/')
            .trim_start_matches('?')
            .split(|c: char| c.is_whitespace() || c == '/' || c == '>')
            .next()
            .unwrap_or("");
        if !XML_DOC_TAGS.contains(&tag_name) {
            out.push_str(&rest[start..=end]);
        }
        rest = &rest[end + 1..];
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn find_class(node: Node) -> Option<Node> {
        if node.kind() == "class_declaration" {
            return Some(node);
        }
        let mut i = 0u32;
        while let Some(child) = node.child(i) {
            if let Some(found) = find_class(child) {
                return Some(found);
            }
            i += 1;
        }
        None
    }

    #[test]
    fn test_strip_comment_markers_java_block() {
        let raw = "/**\n * This is a doc comment\n * With multiple lines\n */";
        let stripped = strip_comment_markers(raw);
        assert_eq!(stripped, "This is a doc comment\nWith multiple lines");
    }

    #[test]
    fn test_strip_comment_markers_java_line() {
        let raw = "// This is a line comment";
        let stripped = strip_comment_markers(raw);
        assert_eq!(stripped, "This is a line comment");
    }

    #[test]
    fn test_strip_comment_markers_typescript_triple_slash() {
        let raw = "/// TypeScript doc comment\n/// Second line";
        let stripped = strip_comment_markers(raw);
        assert_eq!(stripped, "TypeScript doc comment\nSecond line");
    }

    #[test]
    fn test_strip_comment_markers_mixed_block() {
        let raw = "/* Comment start\n * Middle line\n * End */";
        let stripped = strip_comment_markers(raw);
        assert_eq!(stripped, "Comment start\nMiddle line\nEnd");
    }

    #[test]
    fn test_strip_comment_markers_empty_lines() {
        let raw = "// First\n//\n// Third";
        let stripped = strip_comment_markers(raw);
        assert_eq!(stripped, "First\nThird");
    }

    #[test]
    fn test_strip_comment_markers_whitespace_only() {
        let raw = "//   \n//\n//";
        let stripped = strip_comment_markers(raw);
        assert_eq!(stripped, "");
    }

    #[ignore = "Tree-sitter query structure varies by language"]
    #[test]
    fn test_extract_child_entity_nodes_java() {
        // Test extract_child_entity_nodes with Java code
        let code = "class Test { void method1() {} void method2() {} }";
        let tree = crate::pipeline::parser::test_utils::parse_java_snippet(code)
            .expect("Failed to parse Java code");

        if let Some(class_node) = find_class(tree.root_node()) {
            let children = extract_child_entity_nodes(class_node, "java");
            // Should find methods
            assert!(!children.is_empty());
        }
    }

    #[ignore = "Tree-sitter query structure varies by language"]
    #[test]
    fn test_extract_child_entity_nodes_typescript() {
        let code = "class Test { method1() {} method2() {} }";
        let tree = crate::pipeline::parser::test_utils::parse_typescript_snippet(code)
            .expect("Failed to parse TypeScript code");

        if let Some(class_node) = find_class(tree.root_node()) {
            let children = extract_child_entity_nodes(class_node, "typescript");
            // Should find methods or method definitions
            assert!(!children.is_empty());
        }
    }

    #[test]
    fn test_extract_decorators_kotlin() {
        let code = r#"
@Service
class UserService {
    fun getUser(id: Int): User? {
        return null
    }
}
"#;
        let lang = tree_sitter_kotlin_ng::LANGUAGE.into();
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&lang).unwrap();
        let tree = parser.parse(code, None).unwrap();

        if let Some(class_node) = find_class(tree.root_node()) {
            let decorators = extract_decorators(class_node, code.as_bytes(), "kotlin");
            assert!(
                !decorators.is_empty(),
                "Expected to find decorators on Kotlin class"
            );
            assert!(
                decorators.iter().any(|d| d.contains("Service")),
                "Expected to find @Service decorator"
            );
        } else {
            panic!("Could not find class declaration in Kotlin code");
        }
    }
}
