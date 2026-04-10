//! Stage 2 — Parse: AST extraction via Tree-sitter + Rayon.
//!
//! Each source file is parsed in parallel on the Rayon thread pool.
//! Tree-sitter queries extract class declarations, method/function declarations,
//! associated documentation comments, and call-site references.
//!
//! # Custom queries
//! Built-in queries are compiled into the binary at build time (see `queries/`
//! directory). When [`ParseConfig::custom_queries_path`] is set, the parser
//! will instead load `java.scm` and `typescript.scm` from that directory,
//! allowing callers to override extraction logic without recompiling.

use anyhow::{Context, Result};
use rayon::prelude::*;
use std::{
    fs,
    path::{Path, PathBuf},
};
use streaming_iterator::StreamingIterator;
use tracing::{debug, warn};
use tree_sitter::{Language, Node, Parser, Query, QueryCursor};

use crate::models::{EntityKind, ParsedEntity, ReferenceIntent};

mod java;
mod typescript;

// Built-in query files compiled into the binary.
const DEFAULT_JAVA_QUERY: &str = include_str!("../../../queries/java.scm");
const DEFAULT_TS_QUERY: &str = include_str!("../../../queries/typescript.scm");
const DEFAULT_TSX_QUERY: &str = include_str!("../../../queries/tsx.scm");

/// Configuration for the parse stage.
pub struct ParseConfig {
    /// Optional filesystem path to a directory containing custom `.scm` query files.
    pub custom_queries_path: Option<String>,
    /// Logical repository name for multi-repository isolation.
    pub repo_name: String,
}

/// Parse a collection of source files in parallel and return all extracted entities.
///
/// This function blocks until all files have been processed. It is intended to be
/// called from a `tokio::task::spawn_blocking` context so the async executor is
/// not starved.
pub fn parse_files(files: &[PathBuf], parse_cfg: &ParseConfig) -> Vec<ParsedEntity> {
    files
        .par_iter()
        .flat_map(|path| match parse_single_file(path, parse_cfg) {
            Ok(entities) => entities,
            Err(e) => {
                warn!("Failed to parse {}: {e:#}", path.display());
                vec![]
            }
        })
        .collect()
}

/// Parse a single source file and return its extracted entities.
fn parse_single_file(path: &Path, parse_cfg: &ParseConfig) -> Result<Vec<ParsedEntity>> {
    let source = fs::read_to_string(path)
        .with_context(|| format!("Cannot read file: {}", path.display()))?;

    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default();

    let file_path = path.to_string_lossy().to_string();

    let entities = match ext {
        "java" => {
            let query_src = load_query_source("java.scm", DEFAULT_JAVA_QUERY, parse_cfg);
            extract_entities(
                &source,
                tree_sitter_java::LANGUAGE.into(),
                &query_src,
                "java",
                &file_path,
                &parse_cfg.repo_name,
            )?
        }
        "ts" | "tsx" | "cts" => {
            let mut query_src = load_query_source("typescript.scm", DEFAULT_TS_QUERY, parse_cfg);
            let lang: Language = if ext == "tsx" {
                // For TSX files, append TSX-specific rules (JSX component invocations)
                let tsx_rules = load_query_source("tsx.scm", DEFAULT_TSX_QUERY, parse_cfg);
                query_src.push('\n');
                query_src.push_str(&tsx_rules);
                tree_sitter_typescript::LANGUAGE_TSX.into()
            } else {
                tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()
            };
            extract_entities(
                &source,
                lang,
                &query_src,
                "typescript",
                &file_path,
                &parse_cfg.repo_name,
            )?
        }
        other => {
            warn!("Unsupported extension '{other}', skipping");
            vec![]
        }
    };

    debug!("Extracted {} entities from {}", entities.len(), file_path);
    Ok(entities)
}

/// Return the query source string, preferring a custom file when available.
fn load_query_source(filename: &str, default: &str, cfg: &ParseConfig) -> String {
    if let Some(dir) = &cfg.custom_queries_path {
        let custom_path = PathBuf::from(dir).join(filename);
        if custom_path.exists() {
            match fs::read_to_string(&custom_path) {
                Ok(src) => {
                    tracing::info!("Using custom query: {}", custom_path.display());
                    return src;
                }
                Err(e) => warn!(
                    "Failed to load custom query {}: {e} — using built-in",
                    custom_path.display()
                ),
            }
        }
    }
    default.to_owned()
}

/// Run a Tree-sitter query against `source` and convert matches to [`ParsedEntity`].
fn extract_entities(
    source: &str,
    language: Language,
    query_src: &str,
    lang_name: &str,
    file_path: &str,
    repo_name: &str,
) -> Result<Vec<ParsedEntity>> {
    let mut parser = Parser::new();
    parser
        .set_language(&language)
        .context("Failed to set Tree-sitter language")?;

    let tree = parser
        .parse(source, None)
        .context("Tree-sitter failed to parse source")?;

    let query = Query::new(&language, query_src).context("Failed to compile Tree-sitter query")?;

    let source_bytes = source.as_bytes();
    let mut cursor = QueryCursor::new();

    let capture_names: Vec<String> = query
        .capture_names()
        .iter()
        .map(|s| s.to_string())
        .collect();

    let mut entities: Vec<ParsedEntity> = Vec::new();

    // First pass: extract all class/interface names and their line ranges for context
    let mut class_contexts: Vec<ClassContext> = Vec::new();
    extract_class_contexts(tree.root_node(), source_bytes, &mut class_contexts);

    // Second pass: extract entities and resolve their contexts
    let mut matches = cursor.matches(&query, tree.root_node(), source_bytes);
    let mut covered_ranges: Vec<(usize, usize)> = Vec::new();

    while let Some(m) = {
        matches.advance();
        matches.get()
    } {
        let mut name: Option<String> = None;
        let mut kind: Option<EntityKind> = None;
        let mut signature: Option<String> = None;
        let mut start_line: usize = 0;
        let mut entity_node: Option<Node> = None;
        let mut reference_intents: Vec<ReferenceIntent> = Vec::new();

        for cap in m.captures {
            let cap_name = &capture_names[cap.index as usize];
            let node = cap.node;
            let text = node_text(node, source_bytes);

            match cap_name.as_str() {
                "class.name" => {
                    name = Some(text.clone());
                    kind = Some(EntityKind::Class);
                    start_line = node.start_position().row + 1;
                    entity_node = find_parent_by_kind(node, "class_declaration")
                        .or_else(|| find_parent_by_kind(node, "abstract_class_declaration"));
                }
                "interface.name" => {
                    name = Some(text.clone());
                    kind = Some(EntityKind::Interface);
                    start_line = node.start_position().row + 1;
                    entity_node = find_parent_by_kind(node, "interface_declaration");
                }
                "method.name" => {
                    name = Some(text.clone());
                    kind = Some(EntityKind::Method);
                    start_line = node.start_position().row + 1;
                    entity_node = find_parent_by_kind(node, "method_declaration")
                        .or_else(|| find_parent_by_kind(node, "method_definition"))
                        .or_else(|| find_parent_by_kind(node, "method_signature"))
                        .or_else(|| find_parent_by_kind(node, "abstract_method_signature"));
                    // For methods, extract reference intents from the method body
                    if let Some(method_node) = entity_node {
                        if lang_name == "java" {
                            java::extract_reference_intents_java(
                                method_node,
                                source_bytes,
                                &mut reference_intents,
                            );
                        } else {
                            typescript::extract_reference_intents_typescript(
                                method_node,
                                source_bytes,
                                &mut reference_intents,
                            );
                        }
                    }
                }
                "function.name" => {
                    name = Some(text.clone());
                    kind = Some(EntityKind::Function);
                    start_line = node.start_position().row + 1;
                    entity_node = find_parent_by_kind(node, "function_declaration")
                        .or_else(|| find_parent_by_kind(node, "lexical_declaration"))
                        .or_else(|| find_parent_by_kind(node, "export_statement"));
                    // For functions, extract reference intents from the function body
                    if let Some(func_node) = entity_node {
                        typescript::extract_reference_intents_typescript(
                            func_node,
                            source_bytes,
                            &mut reference_intents,
                        );
                    }
                }
                "constant.name" => {
                    name = Some(text.clone());
                    kind = Some(EntityKind::Constant);
                    start_line = node.start_position().row + 1;
                    entity_node = find_parent_by_kind(node, "lexical_declaration")
                        .or_else(|| find_parent_by_kind(node, "variable_declarator"))
                        .or_else(|| find_parent_by_kind(node, "field_declaration"))
                        .or_else(|| find_parent_by_kind(node, "public_field_definition"));

                    // Extract reference intents from constant initializers
                    // This captures function calls inside const assignments like:
                    //   const formattedItems = formatRegistryItems(registryItems)
                    //   const config = await getMcpConfig(process.cwd())
                    if let Some(const_node) = entity_node {
                        if lang_name == "java" {
                            java::extract_reference_intents_java(
                                const_node,
                                source_bytes,
                                &mut reference_intents,
                            );
                        } else {
                            typescript::extract_reference_intents_typescript(
                                const_node,
                                source_bytes,
                                &mut reference_intents,
                            );
                        }
                    }
                }
                "enum.name" => {
                    name = Some(text.clone());
                    kind = Some(EntityKind::Enum);
                    start_line = node.start_position().row + 1;
                    entity_node = find_parent_by_kind(node, "enum_declaration");
                }
                "signature" => signature = Some(text.clone()),
                "type.reference" => {
                    // Type annotations in signatures, variables, etc.
                    reference_intents.push(ReferenceIntent::TypeReference {
                        type_name: text.clone(),
                        line: node.start_position().row + 1,
                    });
                }
                _ => {}
            }
        }

        if let (Some(name), Some(kind)) = (name, kind) {
            // Extract docstring and inline comments dynamically from the entity node
            let (docstring, inline_comments) = if let Some(node) = entity_node {
                extract_comments(node, source_bytes, lang_name, &kind, &class_contexts)
            } else {
                (None, Vec::new())
            };

            // Extract decorators/annotations from the entity node
            let decorators = if let Some(node) = entity_node {
                extract_decorators(node, source_bytes, lang_name)
            } else {
                Vec::new()
            };

            // Determine FQN and enclosing class based on context
            let (fqn, enclosing_class) =
                compute_fqn_and_context(&name, &kind, start_line, lang_name, &class_contexts);

            // For classes, also extract extends/implements from AST
            if matches!(kind, EntityKind::Class | EntityKind::Interface)
                && let Some(class_node) = entity_node
                && lang_name == "typescript"
            {
                typescript::extract_class_inheritance(
                    class_node,
                    source_bytes,
                    &mut reference_intents,
                );
            }

            let mut entity = ParsedEntity::new(
                name,
                kind,
                fqn,
                signature,
                docstring,
                lang_name,
                file_path,
                start_line,
                enclosing_class,
                repo_name,
            );
            entity.reference_intents = reference_intents;
            entity.inline_comments = inline_comments;
            entity.decorators = decorators;

            // Track byte range of this entity for orphan detection
            // Must be done for ALL entities to keep indices aligned with the entities vector
            if let Some(node) = entity_node {
                covered_ranges.push((node.start_byte(), node.end_byte()));
            } else {
                // If we don't have a node, use a dummy range that won't match any orphans
                covered_ranges.push((usize::MAX, usize::MAX));
            }

            entities.push(entity);
        }
    }

    // Third pass: capture orphaned reference intents (calls in top-level statements,
    // callbacks, etc. that were not captured by any named entity)
    if lang_name == "typescript" || lang_name == "java" {
        collect_orphaned_references(
            tree.root_node(),
            source_bytes,
            lang_name,
            &mut entities,
            &covered_ranges,
            file_path,
            repo_name,
        );
    }

    Ok(entities)
}

/// Helper struct to track class context for FQN computation.
#[derive(Debug, Clone)]
struct ClassContext {
    name: String,
    start_line: usize,
    end_line: usize,
}

/// Extract all class/interface declarations and their line ranges.
fn extract_class_contexts(node: Node<'_>, source: &[u8], contexts: &mut Vec<ClassContext>) {
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

/// Third pass: find call_expression / new_expression / jsx nodes that fall outside
/// the byte ranges of extracted entities, and assign them to the nearest entity.
/// If no entities exist, create a synthetic <module> entity.
fn collect_orphaned_references(
    root: Node<'_>,
    source: &[u8],
    lang_name: &str,
    entities: &mut Vec<ParsedEntity>,
    covered_ranges: &[(usize, usize)],
    file_path: &str,
    repo_name: &str,
) {
    // Collect all reference intents from the entire file
    let mut all_intents: Vec<(ReferenceIntent, usize)> = Vec::new();
    collect_all_reference_intents_with_byte_pos(root, source, lang_name, &mut all_intents);

    // Filter to orphaned intents (not covered by any entity)
    let orphaned_intents: Vec<ReferenceIntent> = all_intents
        .into_iter()
        .filter(|(_, byte_pos)| {
            !covered_ranges
                .iter()
                .any(|(start, end)| byte_pos >= start && byte_pos < end)
        })
        .map(|(intent, _)| intent)
        .collect();

    if orphaned_intents.is_empty() {
        return;
    }

    // Assign each orphaned intent to its nearest entity by line number
    if entities.is_empty() {
        // No entities exist: create synthetic <module> entity for all orphans
        let mut module_entity = ParsedEntity::new(
            "<module>",
            EntityKind::Function,
            "<module>",
            None,
            None,
            lang_name,
            file_path,
            1,
            None,
            repo_name,
        );
        module_entity.reference_intents = orphaned_intents;
        entities.push(module_entity);
    } else {
        // Assign each orphan individually to its nearest entity by line
        for intent in orphaned_intents {
            let orphan_line = match &intent {
                ReferenceIntent::Call { line, .. } => *line,
                ReferenceIntent::Extends { line, .. } => *line,
                ReferenceIntent::Implements { line, .. } => *line,
                ReferenceIntent::TypeReference { line, .. } => *line,
            };
            let target_idx = find_nearest_entity_by_line(entities, orphan_line);
            entities[target_idx].reference_intents.push(intent);
        }
    }
}

/// Collect ALL call/new/jsx intents from the entire AST, paired with byte position.
fn collect_all_reference_intents_with_byte_pos(
    node: Node<'_>,
    source: &[u8],
    lang_name: &str,
    intents: &mut Vec<(ReferenceIntent, usize)>,
) {
    if lang_name == "typescript" {
        typescript::collect_all_reference_intents_typescript(node, source, intents);
    } else if lang_name == "java" {
        java::collect_all_reference_intents_java(node, source, intents);
    }
}

/// Find the entity index nearest to the given line number.
/// Prefers the entity immediately preceding the orphan (same or earlier line).
/// Falls back to the closest entity after the orphan if nothing precedes it.
fn find_nearest_entity_by_line(entities: &[ParsedEntity], target_line: usize) -> usize {
    let mut best_idx = 0;
    let mut best_distance = usize::MAX;

    // First pass: find closest entity at or before target_line
    for (idx, entity) in entities.iter().enumerate() {
        let entity_line = entity.start_line;
        if entity_line <= target_line {
            let distance = target_line - entity_line;
            if distance < best_distance {
                best_distance = distance;
                best_idx = idx;
            }
        }
    }

    // If no entity precedes the orphan, fall back to closest entity overall (second pass)
    if best_distance == usize::MAX {
        for (idx, entity) in entities.iter().enumerate() {
            let distance = entity.start_line.abs_diff(target_line);
            if distance < best_distance {
                best_distance = distance;
                best_idx = idx;
            }
        }
    }

    best_idx
}

/// Find the parent node of a given kind by traversing up the AST.
fn find_parent_by_kind<'a>(mut node: Node<'a>, kind: &str) -> Option<Node<'a>> {
    while let Some(parent) = node.parent() {
        if parent.kind() == kind {
            return Some(parent);
        }
        node = parent;
    }
    None
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
fn extract_decorators(entity_node: Node<'_>, source: &[u8], lang_name: &str) -> Vec<String> {
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
    }

    decorators
}

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
fn extract_comments(
    entity_node: Node<'_>,
    source: &[u8],
    lang_name: &str,
    kind: &EntityKind,
    _class_contexts: &[ClassContext],
) -> (Option<String>, Vec<String>) {
    let mut docstring: Option<String> = None;
    let mut inline_comments: Vec<String> = Vec::new();

    // **Pase hacia arriba (Docstring):** Find preceding comments
    if let Some(_parent) = entity_node.parent() {
        let mut current = entity_node.prev_sibling();
        let mut comment_buffer: Vec<String> = Vec::new();

        while let Some(node) = current {
            match node.kind() {
                "comment" | "line_comment" | "block_comment" => {
                    let text = node_text(node, source);
                    comment_buffer.insert(0, strip_comment_markers(&text));
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

    // **Pase hacia abajo (Inline Comments):** Find comments within the entity body
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

/// Extract all child method/function/class declarations within a node.
/// Used to prevent parent entities from capturing comments of their children.
fn extract_child_entity_nodes<'a>(node: Node<'a>, lang_name: &str) -> Vec<Node<'a>> {
    let mut children = Vec::new();
    let mut child = node.child(0);

    let entity_kinds = if lang_name == "java" {
        vec![
            "method_declaration",
            "class_declaration",
            "interface_declaration",
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
fn extract_inline_comments_recursive(
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

/// Compute FQN and enclosing_class based on entity context.
fn compute_fqn_and_context(
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

/// Extract the UTF-8 text of a Tree-sitter node.
pub(crate) fn node_text(node: Node<'_>, source: &[u8]) -> String {
    node.utf8_text(source).unwrap_or_default().trim().to_owned()
}

/// Remove common comment markers from a doc-comment string.
///
/// Strips `/**`, `*/`, leading `*`, `//`, `///`, and surrounding whitespace.
fn strip_comment_markers(raw: &str) -> String {
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
