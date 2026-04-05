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

use crate::models::{CallIntent, EntityKind, ParsedEntity, ReferenceIntent};

// Built-in query files compiled into the binary.
const DEFAULT_JAVA_QUERY: &str = include_str!("../../queries/java.scm");
const DEFAULT_TS_QUERY: &str = include_str!("../../queries/typescript.scm");
const DEFAULT_TSX_QUERY: &str = include_str!("../../queries/tsx.scm");

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
                            extract_reference_intents_java(
                                method_node,
                                source_bytes,
                                &mut reference_intents,
                            );
                        } else {
                            extract_reference_intents_typescript(
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
                        extract_reference_intents_typescript(
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

                    // Extract enum/static member usages from constant initializers
                    if let Some(const_node) = entity_node
                        && lang_name == "typescript"
                    {
                        extract_enum_usages_typescript(
                            const_node,
                            source_bytes,
                            &mut reference_intents,
                        );
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
                extract_class_inheritance(class_node, source_bytes, &mut reference_intents);
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

/// Extract class inheritance (extends/implements) from TypeScript class AST.
/// Manually traverses the class declaration node to find extends and implements clauses.
fn extract_class_inheritance(
    class_node: Node<'_>,
    source: &[u8],
    intents: &mut Vec<ReferenceIntent>,
) {
    let line = class_node.start_position().row + 1;

    // Look for 'extends' keyword followed by type identifier
    let mut child = class_node.child(0);
    while let Some(c) = child {
        if c.kind() == "extends" {
            // Next type identifier should be the parent class
            if let Some(next) = c.next_sibling()
                && next.kind() == "type_identifier"
            {
                let parent_name = node_text(next, source);
                intents.push(ReferenceIntent::Extends {
                    parent: parent_name,
                    line,
                });
            }
        } else if c.kind() == "implements_clause" {
            // Extract type identifiers from implements clause
            let mut impl_child = c.child(0);
            while let Some(impl_c) = impl_child {
                if impl_c.kind() == "type_identifier" {
                    let interface_name = node_text(impl_c, source);
                    intents.push(ReferenceIntent::Implements {
                        interface: interface_name,
                        line,
                    });
                }
                impl_child = impl_c.next_sibling();
            }
        }
        child = c.next_sibling();
    }
}

/// Extract reference intents from a Java method body (wrapper for backward compatibility).
fn extract_reference_intents_java(
    node: Node<'_>,
    source: &[u8],
    intents: &mut Vec<ReferenceIntent>,
) {
    let mut call_intents = Vec::new();
    extract_call_intents_java(node, source, &mut call_intents);
    for call in call_intents {
        intents.push(ReferenceIntent::Call {
            method: call.method,
            receiver: call.receiver,
            line: call.line,
        });
    }
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

/// Extract reference intents from a TypeScript function/method body (wrapper for backward compatibility).
fn extract_reference_intents_typescript(
    node: Node<'_>,
    source: &[u8],
    intents: &mut Vec<ReferenceIntent>,
) {
    let mut call_intents = Vec::new();
    extract_call_intents_typescript(node, source, &mut call_intents);
    for call in call_intents {
        intents.push(ReferenceIntent::Call {
            method: call.method,
            receiver: call.receiver,
            line: call.line,
        });
    }

    // Also extract enum/static member usages (e.g., WebWorkerEvent.Console)
    extract_enum_usages_typescript(node, source, intents);
}

/// Extract call expression call intents from a TypeScript function/method body.
///
/// Handles:
/// - Direct calls: `method()`, `this.method()`
/// - Member calls: `obj.method()`, `this.service.method()`
/// - New expressions: `new MyClass()`
/// - JSX components: `<ChartToolbar />`, `<Sheet.Content />`
/// - Callbacks passed as arguments: `app.use(this.handler)` -> records call to handler
/// - Bind calls: `this.method.bind(this)` -> records call to method
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
        let mut is_bind_call = false;

        while let Some(c) = child {
            if c.kind() == "member_expression" {
                // Use Tree-sitter API to extract fields cleanly
                // member_expression has: object . property
                if let Some(property_node) = c.child_by_field_name("property") {
                    let prop_text = node_text(property_node, source);
                    // Check if this is a .bind() call
                    if prop_text == "bind" {
                        is_bind_call = true;
                    }
                    method_name = Some(prop_text);
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
            // Special handling for .bind(this) and similar patterns
            if is_bind_call {
                // For .bind() calls, the actual target is in the receiver
                // e.g., this.requestPausedHandler.bind(this) -> we want to record call to requestPausedHandler
                if let Some(receiver) = receiver {
                    // Extract the method name from receiver (last component if it's a member expression)
                    if let Some(last_part) = receiver.split('.').next_back() {
                        intents.push(CallIntent {
                            method: last_part.to_string(),
                            receiver: if receiver.contains('.') {
                                receiver.split('.').next().map(|s| s.to_string())
                            } else {
                                Some("this".to_string())
                            },
                            line,
                        });
                    }
                }
            } else {
                intents.push(CallIntent {
                    method,
                    receiver,
                    line,
                });
            }
        }

        // Also scan arguments for callback references (e.g., app.use(this.handler))
        extract_callback_arguments(node, source, intents, line);
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
    } else if node.kind() == "jsx_self_closing_element" || node.kind() == "jsx_opening_element" {
        // JSX component invocation (e.g., <ChartToolbar />, <Sheet.Content />)
        extract_jsx_component_invocation(node, source, intents);
    }

    // Recursively process children
    let mut child = node.child(0);
    while let Some(c) = child {
        extract_call_intents_typescript(c, source, intents);
        child = c.next_sibling();
    }
}

/// Extract JSX component invocation as a call intent.
///
/// Handles React components rendered via JSX syntax:
/// - `<ChartToolbar />` → CallIntent { method: "ChartToolbar", receiver: None }
/// - `<Sheet.Content />` → CallIntent { method: "Content", receiver: Some("Sheet") }
/// - `<Icons.Search />` → CallIntent { method: "Search", receiver: Some("Icons") }
///
/// Native HTML tags (lowercase) are ignored:
/// - `<div />` → skipped
/// - `<span />` → skipped
fn extract_jsx_component_invocation(node: Node<'_>, source: &[u8], intents: &mut Vec<CallIntent>) {
    let line = node.start_position().row + 1;

    // Get the name node (can be identifier, member_expression, or namespace_name)
    if let Some(name_node) = node.child_by_field_name("name") {
        let comp_name = node_text(name_node, source);

        // React convention: Components start with uppercase, HTML tags are lowercase
        if comp_name.chars().next().is_some_and(|c| c.is_uppercase()) {
            // Handle namespaced components (e.g., Sheet.Content, Icons.Search)
            if comp_name.contains('.') {
                let mut parts = comp_name.split('.');
                let receiver = parts.next().map(|s| s.to_string());
                // Collect remaining parts as method name (handles deeply nested components)
                let method = parts.collect::<Vec<_>>().join(".");

                intents.push(CallIntent {
                    method,
                    receiver,
                    line,
                });
            } else {
                // Simple component name
                intents.push(CallIntent {
                    method: comp_name,
                    receiver: None,
                    line,
                });
            }
        }
        // HTML tags (lowercase first letter) are intentionally skipped
    }
}

/// Extract callback arguments from a call expression.
///
/// Detects method references passed as arguments, e.g.:
/// - `app.use(this.authHandler)` -> records call to authHandler
/// - `emitter.on('event', this.handler)` -> records call to handler
/// - `addEventListener('click', this.onClick)` -> records call to onClick
fn extract_callback_arguments(
    call_node: Node<'_>,
    source: &[u8],
    intents: &mut Vec<CallIntent>,
    line: usize,
) {
    // Find the arguments node
    if let Some(args_node) = call_node.child_by_field_name("arguments") {
        let mut arg = args_node.child(0);
        while let Some(a) = arg {
            // Look for member_expression arguments (e.g., this.handler, obj.method)
            if a.kind() == "member_expression" {
                if let Some(property_node) = a.child_by_field_name("property") {
                    let method_name = node_text(property_node, source);
                    if let Some(object_node) = a.child_by_field_name("object") {
                        let receiver = node_text(object_node, source);
                        intents.push(CallIntent {
                            method: method_name,
                            receiver: Some(receiver),
                            line,
                        });
                    }
                }
            } else if a.kind() == "identifier" {
                // Sometimes callbacks are just identifiers: app.use(authHandler)
                let name = node_text(a, source);
                // Only treat as callback if it looks like a method name (not a keyword or literal)
                if !is_reserved_keyword(&name)
                    && name.chars().next().is_some_and(|c| c.is_alphabetic())
                {
                    intents.push(CallIntent {
                        method: name,
                        receiver: None,
                        line,
                    });
                }
            }
            arg = a.next_sibling();
        }
    }
}

/// Check if a string is a TypeScript/JavaScript reserved keyword.
fn is_reserved_keyword(word: &str) -> bool {
    matches!(
        word,
        "true"
            | "false"
            | "null"
            | "undefined"
            | "this"
            | "super"
            | "import"
            | "export"
            | "from"
            | "as"
            | "async"
            | "await"
            | "yield"
            | "return"
            | "throw"
            | "try"
            | "catch"
            | "finally"
            | "if"
            | "else"
            | "for"
            | "while"
            | "do"
            | "break"
            | "continue"
            | "switch"
            | "case"
            | "default"
            | "const"
            | "let"
            | "var"
            | "class"
            | "interface"
            | "enum"
            | "type"
            | "function"
            | "new"
            | "delete"
            | "typeof"
            | "instanceof"
            | "in"
            | "of"
            | "public"
            | "private"
            | "protected"
            | "static"
            | "readonly"
            | "abstract"
            | "extends"
            | "implements"
            | "declare"
    )
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

/// Extract enum and static member usages from a TypeScript node (e.g., EnumName.Value, ClassName.STATIC).
///
/// Recursively searches for member_expression nodes where the object is a capitalized identifier,
/// which typically represents enum or static class member access patterns like WebWorkerEvent.Console.
fn extract_enum_usages_typescript(
    node: Node<'_>,
    source: &[u8],
    intents: &mut Vec<ReferenceIntent>,
) {
    if node.kind() == "member_expression" {
        // member_expression has: object . property
        // We only want to capture if object is a capitalized identifier (enum/class name)
        if let Some(object_node) = node.child_by_field_name("object")
            && object_node.kind() == "identifier"
        {
            let obj_text = node_text(object_node, source);
            // Check if it starts with capital letter (typical of classes/enums)
            if obj_text.chars().next().is_some_and(|c| c.is_uppercase()) {
                let line = object_node.start_position().row + 1;
                intents.push(ReferenceIntent::TypeReference {
                    type_name: obj_text,
                    line,
                });
            }
        }
    }

    // Recursively process children
    let mut child = node.child(0);
    while let Some(c) = child {
        extract_enum_usages_typescript(c, source, intents);
        child = c.next_sibling();
    }
}
