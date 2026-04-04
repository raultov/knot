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

use crate::models::{CallIntent, EntityKind, ParsedEntity};

// Built-in query files compiled into the binary.
const DEFAULT_JAVA_QUERY: &str = include_str!("../../queries/java.scm");
const DEFAULT_TS_QUERY: &str = include_str!("../../queries/typescript.scm");

/// Configuration for the parse stage.
pub struct ParseConfig {
    /// Optional filesystem path to a directory containing custom `.scm` query files.
    pub custom_queries_path: Option<String>,
}

/// Parse a collection of source files in parallel and return all extracted entities.
///
/// This function blocks until all files have been processed. It is intended to be
/// called from a `tokio::task::spawn_blocking` context so the async executor is
/// not starved.
pub fn parse_files(files: &[PathBuf], cfg: &ParseConfig) -> Vec<ParsedEntity> {
    files
        .par_iter()
        .flat_map(|path| match parse_single_file(path, cfg) {
            Ok(entities) => entities,
            Err(e) => {
                warn!("Failed to parse {}: {e:#}", path.display());
                vec![]
            }
        })
        .collect()
}

/// Parse a single source file and return its extracted entities.
fn parse_single_file(path: &Path, cfg: &ParseConfig) -> Result<Vec<ParsedEntity>> {
    let source = fs::read_to_string(path)
        .with_context(|| format!("Cannot read file: {}", path.display()))?;

    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default();

    let file_path = path.to_string_lossy().to_string();

    let entities = match ext {
        "java" => {
            let query_src = load_query_source("java.scm", DEFAULT_JAVA_QUERY, cfg);
            extract_entities(
                &source,
                tree_sitter_java::LANGUAGE.into(),
                &query_src,
                "java",
                &file_path,
            )?
        }
        "ts" | "tsx" => {
            let query_src = load_query_source("typescript.scm", DEFAULT_TS_QUERY, cfg);
            let lang: Language = if ext == "tsx" {
                tree_sitter_typescript::LANGUAGE_TSX.into()
            } else {
                tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()
            };
            extract_entities(&source, lang, &query_src, "typescript", &file_path)?
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

    while let Some(m) = {
        matches.advance();
        matches.get()
    } {
        let mut name: Option<String> = None;
        let mut kind: Option<EntityKind> = None;
        let mut signature: Option<String> = None;
        let mut start_line: usize = 0;
        let mut entity_node: Option<Node> = None;
        let mut call_intents: Vec<CallIntent> = Vec::new();

        for cap in m.captures {
            let cap_name = &capture_names[cap.index as usize];
            let node = cap.node;
            let text = node_text(node, source_bytes);

            match cap_name.as_str() {
                "class.name" => {
                    name = Some(text.clone());
                    kind = Some(EntityKind::Class);
                    start_line = node.start_position().row + 1;
                    entity_node = find_parent_by_kind(node, "class_declaration");
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
                        .or_else(|| find_parent_by_kind(node, "method_definition"));
                    // For methods, extract call intents from the method body
                    if let Some(method_node) = entity_node {
                        if lang_name == "java" {
                            extract_call_intents_java(method_node, source_bytes, &mut call_intents);
                        } else {
                            extract_call_intents_typescript(
                                method_node,
                                source_bytes,
                                &mut call_intents,
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
                    // For functions, extract call intents from the function body
                    if let Some(func_node) = entity_node {
                        extract_call_intents_typescript(func_node, source_bytes, &mut call_intents);
                    }
                }
                "signature" => signature = Some(text.clone()),
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

            // Determine FQN and enclosing class based on context
            let (fqn, enclosing_class) =
                compute_fqn_and_context(&name, &kind, start_line, lang_name, &class_contexts);

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
            );
            entity.call_intents = call_intents;
            entity.inline_comments = inline_comments;
            entities.push(entity);
        }
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
    if node.kind() == "class_declaration" || node.kind() == "interface_declaration" {
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

/// Extract method invocation call intents from a Java method body.
fn extract_call_intents_java(node: Node<'_>, source: &[u8], intents: &mut Vec<CallIntent>) {
    if node.kind() == "method_invocation" {
        let mut method_name: Option<String> = None;
        let mut receiver: Option<String> = None;
        let line = node.start_position().row + 1;

        // Parse method_invocation structure:
        // - Has optional receiver (identifier or "this")
        // - Has "." separator if receiver exists
        // - Has identifier for method name
        let mut child = node.child(0);
        let mut found_dot = false;
        while let Some(c) = child {
            let kind = c.kind();
            match kind {
                "identifier" => {
                    if found_dot {
                        // After a dot, this is the method name
                        method_name = Some(node_text(c, source));
                    } else if receiver.is_none() {
                        // Before a dot (or if no dot), could be receiver or method name
                        receiver = Some(node_text(c, source));
                    }
                }
                "this" => {
                    receiver = Some("this".to_string());
                }
                "." => {
                    found_dot = true;
                }
                _ => {}
            }
            child = c.next_sibling();
        }

        // If we found a dot, we know the last identifier is the method
        if found_dot {
            if let Some(method) = method_name {
                intents.push(CallIntent {
                    method,
                    receiver,
                    line,
                });
            }
        } else if let Some(method) = method_name {
            // No dot found, so receiver is actually a method name (local call)
            intents.push(CallIntent {
                method,
                receiver: None,
                line,
            });
            // Revert receiver since it's not a receiver
        } else if let Some(receiver_val) = receiver {
            // Single identifier - treat as local call
            intents.push(CallIntent {
                method: receiver_val,
                receiver: None,
                line,
            });
        }
    } else if node.kind() == "object_creation_expression" {
        let line = node.start_position().row + 1;
        let mut child = node.child(0);
        while let Some(c) = child {
            if c.kind() == "type_identifier" || c.kind() == "identifier" {
                intents.push(CallIntent {
                    method: node_text(c, source),
                    receiver: None,
                    line,
                });
                break;
            }
            child = c.next_sibling();
        }
    }

    // Recursively process children
    let mut child = node.child(0);
    while let Some(c) = child {
        extract_call_intents_java(c, source, intents);
        child = c.next_sibling();
    }
}

/// Extract call expression call intents from a TypeScript function/method body.
fn extract_call_intents_typescript(node: Node<'_>, source: &[u8], intents: &mut Vec<CallIntent>) {
    if node.kind() == "call_expression" {
        let line = node.start_position().row + 1;

        // Parse call_expression structure:
        // - Has a 'function' field which can be:
        //   - identifier (local call)
        //   - member_expression (object.method call)

        let mut method_name: Option<String> = None;
        let mut receiver: Option<String> = None;

        // Look for the function field in the call_expression
        let mut child = node.child(0);
        while let Some(c) = child {
            if c.kind() == "member_expression" {
                // Use Tree-sitter API to extract fields cleanly
                // member_expression has: object . property
                if let Some(property_node) = c.child_by_field_name("property") {
                    method_name = Some(node_text(property_node, source));
                }

                // For the object, we need to extract it as text to handle nested members like "this.browserService"
                if let Some(object_node) = c.child_by_field_name("object") {
                    receiver = Some(node_text(object_node, source));
                }
            } else if c.kind() == "identifier" {
                // Direct identifier in call_expression (local call)
                method_name = Some(node_text(c, source));
            }
            child = c.next_sibling();
        }

        if let Some(method) = method_name {
            intents.push(CallIntent {
                method,
                receiver,
                line,
            });
        }
    } else if node.kind() == "new_expression" {
        let line = node.start_position().row + 1;
        let mut child = node.child(0);
        while let Some(c) = child {
            if c.kind() == "identifier" || c.kind() == "type_identifier" {
                intents.push(CallIntent {
                    method: node_text(c, source),
                    receiver: None,
                    line,
                });
                break;
            }
            child = c.next_sibling();
        }
    }

    // Recursively process children
    let mut child = node.child(0);
    while let Some(c) = child {
        extract_call_intents_typescript(c, source, intents);
        child = c.next_sibling();
    }
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
            "function_declaration",
            "class_declaration",
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
    };

    (fqn, enclosing_class)
}

/// Extract the UTF-8 text of a Tree-sitter node.
fn node_text(node: Node<'_>, source: &[u8]) -> String {
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
