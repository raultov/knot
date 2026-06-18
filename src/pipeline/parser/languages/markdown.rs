use crate::models::EntityKind;
use tree_sitter::Node;

pub(crate) fn handle_markdown_capture(
    cap_name: &str,
    _text: &str,
    node: Node<'_>,
    source_bytes: &[u8],
) -> Option<(String, EntityKind, usize)> {
    let start_line = node.start_position().row + 1;

    match cap_name {
        //gets overwittten in enrich.
        "markdown.document.name" => Some((
            "Document".to_string(),
            EntityKind::MarkdownDocument,
            start_line,
        )),
        "markdown.section" => {
            let heading_text = section_heading_text(node, source_bytes)?;
            let clean_name = clean_heading_name(&heading_text);
            Some((clean_name, EntityKind::MarkdownSection, start_line))
        }
        _ => None,
    }
}

/// Strip non-alphanumeric inline syntax from a Markdown heading to produce a
/// cleaner entity name. The heading text in `embed_text` is left raw — this
/// only affects the entity's `name` field, which is used for exact lookups
/// and display.
///
/// What this does NOT do (intentionally, to keep the rule simple):
/// - Does not preserve link text vs. URL distinction. A heading
///   `## [foo](bar.md)` becomes `foo bar.md` rather than `foo`.
/// - Does not preserve image alt text vs. URL.
/// - Does not handle reference-style links (`[text][ref]`) specially.
/// - Does not strip HTML tags (`<br>`, `<a>`, etc.).
/// - Does not handle escaped characters (`\*`, `\_`).
///
/// If smarter stripping is needed later, a regex-based version can be added
/// without changing callers.
fn clean_heading_name(raw: &str) -> String {
    let stripped: String = raw
        .chars()
        .map(|c| match c {
            '[' | ']' | '(' | ')' | '`' | '*' | '_' | '#' | '!' => ' ',
            other => other,
        })
        .collect();

    // Collapse runs of whitespace to a single space, then trim ends.
    stripped.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Walks a `section` node to find the text of its ATX heading.
///
/// Returns `None` if the section has no `atx_heading` child or if the
/// heading is missing its `heading_content` field.
pub(crate) fn section_heading_text(section: Node<'_>, source: &[u8]) -> Option<String> {
    let mut cursor = section.walk();
    for child in section.children(&mut cursor) {
        if child.kind() == "atx_heading" {
            let inline = child.child_by_field_name("heading_content")?;
            return inline.utf8_text(source).ok().map(str::to_string);
        }
    }
    None
}

pub(crate) fn build_markdown_fqn(section: Node<'_>, source: &[u8]) -> String {
    let mut chain: Vec<String> = Vec::new();
    let mut current = Some(section);
    while let Some(node) = current {
        if node.kind() == "section" {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "atx_heading" {
                    if let Some(inline) = child.child_by_field_name("heading_content")
                        && let Ok(text) = inline.utf8_text(source)
                    {
                        chain.push(text.to_string());
                    }
                    break;
                }
            }
        }
        current = node.parent();
    }
    chain.reverse();
    chain.join(" > ")
}

/// Returns the document text from the start of the file up to (but not
/// including) the *second* top-level section. This captures any intro
/// content plus the first section (typically the title and its body),
/// giving the document entity a meaningful embedding even when the file
/// starts with `# Title` directly.
///
/// If the file has fewer than two top-level sections, the entire file is
/// returned.
pub(crate) fn extract_document_intro(document: Node<'_>, source: &[u8]) -> String {
    let mut cursor = document.walk();
    let mut section_count = 0;
    for child in document.children(&mut cursor) {
        if child.kind() == "section" {
            section_count += 1;
            if section_count == 2 {
                let end_byte = child.start_byte();
                return std::str::from_utf8(&source[..end_byte])
                    .unwrap_or("")
                    .to_string();
            }
        }
    }
    // Fewer than two sections — embed the whole file.
    document.utf8_text(source).unwrap_or("").to_string()
}

#[cfg(test)]
mod tests {
    use crate::pipeline::parser::test_utils::parse_markdown_snippet;

    #[test]
    fn test_print_markdown_ast() {
        let code = "# Header 1\nSome text.\n## Header 2\nMore text.";
        let tree = parse_markdown_snippet(code).expect("Failed to parse");
        println!("{}", tree.root_node().to_sexp());
    }
}

#[cfg(test)]
mod section_heading_text_tests {
    use super::section_heading_text;
    use crate::pipeline::parser::test_utils::parse_markdown_snippet;
    use tree_sitter::Node;

    /// Find the first `section` node at or below `node` whose first ATX
    /// heading's text matches `heading_text`. Returns `None` if no such
    /// section exists in the subtree.
    fn find_section_by_heading<'a>(
        node: Node<'a>,
        source: &[u8],
        heading_text: &str,
    ) -> Option<Node<'a>> {
        if node.kind() == "section"
            && let Some(text) = section_heading_text(node, source)
            && text == heading_text
        {
            return Some(node);
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if let Some(found) = find_section_by_heading(child, source, heading_text) {
                return Some(found);
            }
        }
        None
    }

    /// Parse `code` and return the first `section` node found (depth-first).
    /// Returns `None` if no section exists in the tree.
    fn first_section_node<'tree>(tree: &'tree tree_sitter::Tree) -> Option<Node<'tree>> {
        fn walk<'a>(node: Node<'a>) -> Option<Node<'a>> {
            if node.kind() == "section" {
                return Some(node);
            }
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if let Some(s) = walk(child) {
                    return Some(s);
                }
            }
            None
        }
        walk(tree.root_node())
    }

    #[test]
    fn extracts_h1_heading_text() {
        let code = "# Hello World\n\nSome paragraph.\n";
        let tree = parse_markdown_snippet(code).expect("parse");
        let section = first_section_node(&tree).expect("section");
        assert_eq!(
            section_heading_text(section, code.as_bytes()),
            Some("Hello World".to_string()),
        );
    }

    #[test]
    fn extracts_h2_heading_text() {
        let code = "## Setup\n\nBody.\n";
        let tree = parse_markdown_snippet(code).expect("parse");
        let section = first_section_node(&tree).expect("section");
        assert_eq!(
            section_heading_text(section, code.as_bytes()),
            Some("Setup".to_string()),
        );
    }

    #[test]
    fn extracts_deepest_heading_correctly() {
        // tree-sitter-md nests sections, so a doc with H1 > H2 > H3 has
        // three nested sections. Make sure each section returns its OWN
        // heading text, not its parent's.
        let code = "# Top\n\n## Middle\n\n### Bottom\n\nBody.\n";
        let tree = parse_markdown_snippet(code).expect("parse");
        let source = code.as_bytes();

        let top = find_section_by_heading(tree.root_node(), source, "Top").expect("Top section");
        let middle =
            find_section_by_heading(tree.root_node(), source, "Middle").expect("Middle section");
        let bottom =
            find_section_by_heading(tree.root_node(), source, "Bottom").expect("Bottom section");

        assert_eq!(section_heading_text(top, source), Some("Top".to_string()),);
        assert_eq!(
            section_heading_text(middle, source),
            Some("Middle".to_string()),
        );
        assert_eq!(
            section_heading_text(bottom, source),
            Some("Bottom".to_string()),
        );
    }

    #[test]
    fn handles_heading_with_special_characters() {
        // The heading text can include punctuation, code spans, etc.
        // We don't try to strip Markdown — return whatever's inside `inline`.
        let code = "## What's New in v2.0?\n\nDetails.\n";
        let tree = parse_markdown_snippet(code).expect("parse");
        let section = first_section_node(&tree).expect("section");
        assert_eq!(
            section_heading_text(section, code.as_bytes()),
            Some("What's New in v2.0?".to_string()),
        );
    }

    #[test]
    fn returns_none_for_non_section_node() {
        // section_heading_text expects a `section` node. If we hand it
        // something else (e.g. the document root or a paragraph), the
        // walk over its children won't find an `atx_heading` and we
        // should get None.
        let code = "Just a paragraph, no heading.\n";
        let tree = parse_markdown_snippet(code).expect("parse");
        // Root is `document`, not `section`. Children are likely paragraphs.
        let root = tree.root_node();
        assert_eq!(section_heading_text(root, code.as_bytes()), None);
    }
}

#[cfg(test)]
mod build_markdown_fqn_tests {
    use super::build_markdown_fqn;
    use crate::pipeline::parser::test_utils::parse_markdown_snippet;
    use tree_sitter::Node;

    /// Find the first `section` node whose own heading text matches
    /// `heading_text`. Walks the tree recursively.
    fn find_section_by_heading<'a>(
        node: Node<'a>,
        source: &[u8],
        heading_text: &str,
    ) -> Option<Node<'a>> {
        if node.kind() == "section" {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "atx_heading"
                    && let Some(inline) = child.child_by_field_name("heading_content")
                    && let Ok(text) = inline.utf8_text(source)
                    && text == heading_text
                {
                    return Some(node);
                }
            }
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if let Some(found) = find_section_by_heading(child, source, heading_text) {
                return Some(found);
            }
        }
        None
    }

    #[test]
    fn single_h1_returns_only_its_heading() {
        let code = "# Hello World\n\nBody.\n";
        let tree = parse_markdown_snippet(code).expect("parse");
        let section = find_section_by_heading(tree.root_node(), code.as_bytes(), "Hello World")
            .expect("Hello World section");
        assert_eq!(build_markdown_fqn(section, code.as_bytes()), "Hello World",);
    }

    #[test]
    fn nested_h2_includes_h1_ancestor() {
        let code = "# Top\n\n## Setup\n\nBody.\n";
        let tree = parse_markdown_snippet(code).expect("parse");
        let setup = find_section_by_heading(tree.root_node(), code.as_bytes(), "Setup")
            .expect("Setup section");
        assert_eq!(build_markdown_fqn(setup, code.as_bytes()), "Top > Setup",);
    }

    #[test]
    fn deeply_nested_h3_includes_full_chain() {
        let code = "# Top\n\n## Middle\n\n### Bottom\n\nBody.\n";
        let tree = parse_markdown_snippet(code).expect("parse");
        let bottom = find_section_by_heading(tree.root_node(), code.as_bytes(), "Bottom")
            .expect("Bottom section");
        assert_eq!(
            build_markdown_fqn(bottom, code.as_bytes()),
            "Top > Middle > Bottom",
        );
    }

    #[test]
    fn sibling_sections_have_distinct_chains() {
        // Two H2s under the same H1: each chain should be the H1 + its
        // own heading, not bleed into the sibling.
        let code = "# Top\n\n## First\n\nBody A.\n\n## Second\n\nBody B.\n";
        let tree = parse_markdown_snippet(code).expect("parse");
        let source = code.as_bytes();

        let first =
            find_section_by_heading(tree.root_node(), source, "First").expect("First section");
        let second =
            find_section_by_heading(tree.root_node(), source, "Second").expect("Second section");

        assert_eq!(build_markdown_fqn(first, source), "Top > First");
        assert_eq!(build_markdown_fqn(second, source), "Top > Second");
    }

    #[test]
    fn duplicate_heading_text_at_different_levels_produces_distinct_chains() {
        // The whole point of the chain: two sections with the same heading
        // text but different parents must produce different FQNs.
        let code = "\
# Doc

## Setup

First Setup body.

## Configuration

### Setup

Second Setup body.
";
        let tree = parse_markdown_snippet(code).expect("parse");
        let source = code.as_bytes();

        // The first "Setup" is at H2 level under "Doc".
        // The second "Setup" is at H3 level under "Configuration".
        // We can't disambiguate by name alone, so check that *both* FQNs
        // appear when we walk every section in the tree.
        let mut found_chains: Vec<String> = Vec::new();
        collect_section_chains(tree.root_node(), source, &mut found_chains);

        assert!(
            found_chains.contains(&"Doc > Setup".to_string()),
            "expected 'Doc > Setup', got {:?}",
            found_chains
        );
        assert!(
            found_chains.contains(&"Doc > Configuration > Setup".to_string()),
            "expected 'Doc > Configuration > Setup', got {:?}",
            found_chains
        );
    }

    /// Walk the tree and call build_markdown_fqn on every section node,
    /// pushing the result into `chains`.
    fn collect_section_chains(node: Node<'_>, source: &[u8], chains: &mut Vec<String>) {
        if node.kind() == "section" {
            chains.push(build_markdown_fqn(node, source));
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            collect_section_chains(child, source, chains);
        }
    }

    #[test]
    fn returns_empty_string_for_non_section_node() {
        // If called on a non-section node (e.g. the document root), the
        // walk finds no ancestor sections and returns an empty string.
        let code = "Just a paragraph.\n";
        let tree = parse_markdown_snippet(code).expect("parse");
        // tree.root_node() is `document`, not `section`.
        let result = build_markdown_fqn(tree.root_node(), code.as_bytes());
        assert_eq!(result, "");
    }

    #[test]
    fn preserves_special_characters_in_heading_text() {
        let code = "# Doc\n\n## What's New in v2.0?\n\nDetails.\n";
        let tree = parse_markdown_snippet(code).expect("parse");
        let section =
            find_section_by_heading(tree.root_node(), code.as_bytes(), "What's New in v2.0?")
                .expect("section");
        assert_eq!(
            build_markdown_fqn(section, code.as_bytes()),
            "Doc > What's New in v2.0?",
        );
    }
}

#[cfg(test)]
mod clean_heading_name_tests {
    use super::clean_heading_name;

    #[test]
    fn passes_through_plain_text() {
        assert_eq!(clean_heading_name("Setup"), "Setup");
    }

    #[test]
    fn strips_link_brackets_and_parens() {
        assert_eq!(clean_heading_name("Use [foo](bar.md)"), "Use foo bar.md");
    }
}
#[cfg(test)]
mod extract_document_intro_tests {
    use super::extract_document_intro;
    use crate::pipeline::parser::test_utils::parse_markdown_snippet;

    /// Helper: parse markdown and run extract_document_intro on the root node.
    fn extract(code: &str) -> String {
        let tree = parse_markdown_snippet(code).expect("parse");
        extract_document_intro(tree.root_node(), code.as_bytes())
    }

    #[test]
    fn empty_file_returns_empty() {
        assert_eq!(extract(""), "");
    }

    #[test]
    fn no_headings_returns_whole_file() {
        let code = "Just a paragraph.\n\nAnother paragraph, no headings here.\n";
        assert_eq!(extract(code), code);
    }

    #[test]
    fn single_section_returns_whole_file() {
        // Only one top-level section means there's no second to stop at —
        // the whole file is the document's intro.
        let code = "# Only Heading\n\nSome body content.\n";
        assert_eq!(extract(code), code);
    }

    #[test]
    fn stops_at_second_top_level_section() {
        // Two H1s — the intro is the first H1 plus its body, ending right
        // before the second H1 starts.
        let code = "# First\n\nFirst body.\n\n# Second\n\nSecond body.\n";
        let intro = extract(code);
        assert!(
            intro.contains("# First"),
            "intro should include the first heading, got: {:?}",
            intro
        );
        assert!(
            intro.contains("First body."),
            "intro should include the first section's body"
        );
        assert!(
            !intro.contains("# Second"),
            "intro must stop before the second heading, got: {:?}",
            intro
        );
        assert!(
            !intro.contains("Second body."),
            "intro must not include the second section's body"
        );
    }

    #[test]
    fn includes_intro_paragraphs_before_first_heading() {
        // Files that have prose before the first heading: the intro
        // should include those paragraphs AND the first section.
        let code = "Intro paragraph here.\n\n# First\n\nFirst body.\n\n# Second\n\nSecond body.\n";
        let intro = extract(code);
        assert!(intro.contains("Intro paragraph here."));
    }

    #[test]
    fn nested_sections_dont_count_toward_the_limit() {
        // The function counts only top-level sections. An H2 nested inside
        // the first H1 is not a "second section" at the top level — the
        // intro should include both the H1 and its nested H2.
        let code = "# Top\n\n## Nested\n\nNested body.\n\n# Second Top\n\nSecond body.\n";
        let intro = extract(code);
        assert!(intro.contains("# Top"));
        assert!(intro.contains("## Nested"));
        assert!(intro.contains("Nested body."));
        assert!(
            !intro.contains("# Second Top"),
            "intro must stop at the second top-level heading, got: {:?}",
            intro
        );
    }

    #[test]
    fn handles_h2_as_top_level_when_no_h1() {
        // Some files start at H2 with no H1 above. The function still
        // treats top-level sections uniformly — stops at the second one
        // regardless of heading level.
        let code = "## First\n\nBody A.\n\n## Second\n\nBody B.\n";
        let intro = extract(code);
        assert!(intro.contains("## First"));
        assert!(intro.contains("Body A."));
        assert!(!intro.contains("## Second"));
    }
}
