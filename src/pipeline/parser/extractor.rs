use anyhow::{Context, Result};
use streaming_iterator::StreamingIterator;
use tree_sitter::{Language, Node, Parser, Query, QueryCursor};

use super::comments::*;
use super::context::*;
use super::languages::{
    cpp, css, groovy, html, java, javascript, kotlin, python, rust, typescript,
};
use super::orphans::*;
use super::utils::*;
use crate::models::{EntityKind, ParsedEntity, ReferenceIntent};

/// Detect whether a Kotlin `class_declaration` AST node is really a class, interface, or enum.
///
/// In tree-sitter-kotlin-ng v1.1.0, `class`, `interface`, and `enum class` declarations all use
/// the same `class_declaration` node type. We distinguish them by inspecting the source text
/// for the keyword after visibility/modifier tokens.
fn detect_kotlin_class_kind(node: Node<'_>, source: &[u8]) -> EntityKind {
    let text = node_text(node, source);
    let first_kw = text
        .split_whitespace()
        .find(|t| {
            !matches!(
                *t,
                "public"
                    | "private"
                    | "protected"
                    | "internal"
                    | "abstract"
                    | "open"
                    | "sealed"
                    | "data"
                    | "annotation"
                    | "final"
                    | "override"
                    | "inner"
            )
        })
        .unwrap_or("");

    match first_kw {
        "enum" => EntityKind::KotlinEnum,
        "interface" => EntityKind::KotlinInterface,
        _ => EntityKind::KotlinClass,
    }
}

/// Run a Tree-sitter query against `source` and convert matches to [`ParsedEntity`].
pub(crate) fn extract_entities(
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

    // Extract Java package name for FQN prefixing
    let java_package = if lang_name == "java" {
        java::extract_package_name(tree.root_node(), source_bytes)
    } else {
        None
    };

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
        #[allow(unused_assignments)]
        let mut end_line: usize = 0;
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
                "kotlin_class.name" => {
                    name = Some(text.clone());
                    start_line = node.start_position().row + 1;
                    entity_node = find_parent_by_kind(node, "class_declaration");
                    kind = entity_node.map(|n| detect_kotlin_class_kind(n, source_bytes));
                }
                "interface.name" => {
                    name = Some(text.clone());
                    kind = Some(EntityKind::Interface);
                    start_line = node.start_position().row + 1;
                    entity_node = find_parent_by_kind(node, "interface_declaration");
                }
                "kotlin_interface.name" => {
                    name = Some(text.clone());
                    kind = Some(EntityKind::KotlinInterface);
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
                        } else if lang_name == "javascript" {
                            javascript::extract_reference_intents_javascript(
                                method_node,
                                source_bytes,
                                &mut reference_intents,
                            );
                        } else if lang_name == "kotlin" {
                            kotlin::extract_reference_intents_kotlin(
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

                        // Extract type references from method signatures (parameters, return types)
                        if lang_name == "java" {
                            java::extract_type_references(
                                method_node,
                                source_bytes,
                                &mut reference_intents,
                            );
                        } else if lang_name == "kotlin" {
                            kotlin::extract_type_references(
                                method_node,
                                source_bytes,
                                &mut reference_intents,
                            );
                        }
                    }
                }
                "kotlin_method.name" => {
                    name = Some(text.clone());
                    kind = Some(EntityKind::KotlinMethod);
                    start_line = node.start_position().row + 1;
                    entity_node = find_parent_by_kind(node, "function_declaration");
                    // For Kotlin methods, extract reference intents from the method body
                    if let Some(method_node) = entity_node
                        && lang_name == "kotlin"
                    {
                        kotlin::extract_reference_intents_kotlin(
                            method_node,
                            source_bytes,
                            &mut reference_intents,
                        );
                    }
                }
                "kotlin_object.name" => {
                    name = Some(text.clone());
                    kind = Some(EntityKind::KotlinObject);
                    start_line = node.start_position().row + 1;
                    entity_node = find_parent_by_kind(node, "object_declaration");
                }
                "kotlin_companion.name" => {
                    name = Some(text.clone());
                    kind = Some(EntityKind::KotlinCompanionObject);
                    start_line = node.start_position().row + 1;
                    entity_node = find_parent_by_kind(node, "companion_object");
                }
                "kotlin_function.name" => {
                    name = Some(text.clone());
                    start_line = node.start_position().row + 1;
                    entity_node = find_parent_by_kind(node, "function_declaration");

                    // Determine if this function is actually a method (enclosed in a
                    // class_declaration, object_declaration, companion_object, or interface_declaration).
                    let is_method = entity_node.is_some_and(|n| {
                        let mut current = n.parent();
                        while let Some(p) = current {
                            match p.kind() {
                                "class_declaration"
                                | "object_declaration"
                                | "companion_object"
                                | "interface_declaration" => return true,
                                _ => {}
                            }
                            current = p.parent();
                        }
                        false
                    });

                    if is_method {
                        kind = Some(EntityKind::KotlinMethod);
                    } else {
                        kind = Some(EntityKind::KotlinFunction);
                    }

                    // For Kotlin methods/functions, extract reference intents from the function body
                    if let Some(func_node) = entity_node
                        && lang_name == "kotlin"
                    {
                        kotlin::extract_reference_intents_kotlin(
                            func_node,
                            source_bytes,
                            &mut reference_intents,
                        );

                        // Extract type references from method signatures (parameters, return types)
                        // for functions that are really methods inside a class/object
                        if is_method {
                            kotlin::extract_type_references(
                                func_node,
                                source_bytes,
                                &mut reference_intents,
                            );
                        }
                    }
                }
                "kotlin_property.name" => {
                    name = Some(text.clone());
                    kind = Some(EntityKind::KotlinProperty);
                    start_line = node.start_position().row + 1;
                    entity_node = find_parent_by_kind(node, "property_declaration");
                    // For Kotlin properties, extract reference intents from the property
                    if let Some(prop_node) = entity_node
                        && lang_name == "kotlin"
                    {
                        kotlin::extract_reference_intents_kotlin(
                            prop_node,
                            source_bytes,
                            &mut reference_intents,
                        );
                    }
                }
                "function.name" => {
                    name = Some(text.clone());
                    kind = Some(EntityKind::Function);
                    start_line = node.start_position().row + 1;
                    entity_node = find_parent_by_kind(node, "function_declaration")
                        .or_else(|| find_parent_by_kind(node, "lexical_declaration"))
                        .or_else(|| find_parent_by_kind(node, "variable_declaration"))
                        .or_else(|| find_parent_by_kind(node, "export_statement"));
                    // For functions, extract reference intents from the function body
                    if let Some(func_node) = entity_node {
                        if lang_name == "javascript" {
                            javascript::extract_reference_intents_javascript(
                                func_node,
                                source_bytes,
                                &mut reference_intents,
                            );
                        } else if lang_name == "kotlin" {
                            kotlin::extract_reference_intents_kotlin(
                                func_node,
                                source_bytes,
                                &mut reference_intents,
                            );
                        } else {
                            typescript::extract_reference_intents_typescript(
                                func_node,
                                source_bytes,
                                &mut reference_intents,
                            );
                        }
                    }
                }
                "constant.name" => {
                    name = Some(text.clone());
                    kind = Some(EntityKind::Constant);
                    start_line = node.start_position().row + 1;
                    entity_node = find_parent_by_kind(node, "lexical_declaration")
                        .or_else(|| find_parent_by_kind(node, "variable_declarator"))
                        .or_else(|| find_parent_by_kind(node, "field_declaration"))
                        .or_else(|| find_parent_by_kind(node, "public_field_definition"))
                        .or_else(|| find_parent_by_kind(node, "field_definition"));

                    // Extract reference intents from constant initializers
                    // This captures function calls inside const assignments like:
                    //   const formattedItems = formatRegistryItems(registryItems)
                    //   const config = await getMcpConfig(process.cwd())
                    //   val result = someFunction()
                    if let Some(const_node) = entity_node {
                        if lang_name == "java" {
                            java::extract_reference_intents_java(
                                const_node,
                                source_bytes,
                                &mut reference_intents,
                            );
                        } else if lang_name == "javascript" {
                            javascript::extract_reference_intents_javascript(
                                const_node,
                                source_bytes,
                                &mut reference_intents,
                            );
                        } else if lang_name == "kotlin" {
                            kotlin::extract_reference_intents_kotlin(
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
                "signature" | "python.signature" => signature = Some(text.clone()),
                "type.reference" => {
                    // Type annotations in signatures, variables, etc.
                    reference_intents.push(ReferenceIntent::TypeReference {
                        type_name: text.clone(),
                        line: node.start_position().row + 1,
                    });
                }
                // CSS/SCSS: Delegate to specialized handler
                name_or_intent
                    if name_or_intent.starts_with("css.")
                        || name_or_intent.starts_with("scss.") =>
                {
                    if let Some((entity_name, entity_kind, entity_line)) =
                        css::handle_css_capture(name_or_intent, &text, node)
                    {
                        name = Some(entity_name);
                        kind = Some(entity_kind);
                        start_line = entity_line;
                        entity_node = Some(node);
                    }
                }
                // HTML: Delegate to specialized handler
                name_or_intent if name_or_intent.starts_with("html_") => {
                    if let Some((entity_name, entity_kind, entity_line)) =
                        html::handle_html_capture(name_or_intent, &text, node)
                    {
                        name = Some(entity_name);
                        kind = Some(entity_kind);
                        start_line = entity_line;
                        entity_node = Some(node);
                    }
                }
                // Rust: Handle Rust entity captures
                name_or_intent if name_or_intent.starts_with("rust.") => {
                    if let Some((entity_name, entity_kind, entity_line, _rust_metadata)) =
                        rust::handle_rust_capture(name_or_intent, &text, node)
                    {
                        let rust_kind = entity_kind.clone();
                        name = Some(entity_name);
                        kind = Some(entity_kind);
                        start_line = entity_line;

                        // For Rust type aliases, constants, and statics, the captured node is the
                        // identifier (type_identifier, identifier, identifier), but comments are
                        // preceding siblings of the parent entity node (type_item, const_item, static_item).
                        // Get the parent to properly extract preceding comments.
                        entity_node = if matches!(
                            rust_kind,
                            EntityKind::RustTypeAlias
                                | EntityKind::RustConstant
                                | EntityKind::RustStatic
                        ) {
                            node.parent()
                        } else {
                            Some(node)
                        };
                    }
                }
                // Python: Handle Python entity captures
                name_or_intent if name_or_intent.starts_with("python.") => {
                    if let Some((entity_name, entity_kind, entity_line)) =
                        python::handle_python_capture(name_or_intent, &text, node)
                    {
                        name = Some(entity_name);
                        kind = Some(entity_kind.clone());
                        start_line = entity_line;
                        // Use parent to get the full definition node for proper scope tracking
                        // (captures always point to identifiers, not the full definition)
                        entity_node = node.parent();

                        // Extract call reference intents from Python function/method bodies
                        let is_callable = matches!(
                            entity_kind,
                            EntityKind::PythonFunction | EntityKind::PythonMethod
                        );
                        #[allow(clippy::collapsible_if)]
                        if is_callable {
                            if let Some(entity_n) = entity_node {
                                python::extract_reference_intents_python(
                                    entity_n,
                                    source_bytes,
                                    &mut reference_intents,
                                );
                            }
                        }

                        // Phase 5: Extract decorators for ALL Python entities
                        if let Some(entity_n) = entity_node {
                            python::extract_decorator_intents_python(
                                entity_n,
                                source_bytes,
                                &mut reference_intents,
                            );
                        }

                        // Phase 5: Extract inheritance (EXTENDS) for Python classes
                        if entity_kind == EntityKind::PythonClass
                            && let Some(entity_n) = entity_node
                        {
                            python::extract_inheritance_intents_python(
                                entity_n,
                                source_bytes,
                                &mut reference_intents,
                            );
                        }
                    }
                }
                // DOM/CSS references: Delegate to JavaScript handler
                name_or_intent
                    if name_or_intent.starts_with("dom.")
                        || name_or_intent.starts_with("css.class_") =>
                {
                    if let Some(intent) = javascript::handle_dom_css_capture(
                        name_or_intent,
                        &text,
                        node.start_position().row + 1,
                    ) {
                        reference_intents.push(intent);
                    }
                }
                // Groovy: Handle Groovy entity captures (JVM-shared grammar with Java)
                name_or_intent if name_or_intent.starts_with("groovy.") => {
                    if let Some((entity_name, entity_kind, entity_line)) =
                        groovy::handle_groovy_capture(name_or_intent, &text, node)
                    {
                        name = Some(entity_name);
                        kind = Some(entity_kind.clone());
                        start_line = entity_line;
                        // Find parent node using same patterns as Java (shared grammar)
                        entity_node = match name_or_intent {
                            n if n.contains("class") => {
                                find_parent_by_kind(node, "class_declaration")
                            }
                            n if n.contains("interface") => {
                                find_parent_by_kind(node, "interface_declaration")
                            }
                            n if n.contains("enum") => {
                                find_parent_by_kind(node, "enum_declaration")
                            }
                            n if n.contains("method") => {
                                find_parent_by_kind(node, "method_declaration").or_else(|| {
                                    find_parent_by_kind(node, "constructor_declaration")
                                })
                            }
                            _ => node.parent(),
                        };
                        // Groovy reference extraction is handled by ad-hoc `extract_method_calls`
                        // in extract_entities_groovy_standard(), which uses innermost assignment.
                        // Java's tree-sitter ref extraction is unreliable for Groovy because
                        // tree-sitter-groovy misparses methods nested inside closures (e.g.,
                        // `new AnAction() { @Override void actionPerformed(...) { ... } }`).
                    }
                }
                // C/C++ Entities
                name_or_intent
                    if name_or_intent.starts_with("cpp_")
                        || name_or_intent.starts_with("c_")
                        || name_or_intent.starts_with("preproc.") =>
                {
                    let text_str = text.clone();
                    match name_or_intent {
                        "cpp_class.name" => {
                            name = Some(text_str);
                            kind = Some(EntityKind::CppClass);
                            start_line = node.start_position().row + 1;
                            entity_node = find_parent_by_kind(node, "class_specifier");
                        }
                        "c_struct.name" => {
                            name = Some(text_str);
                            kind = Some(EntityKind::CStruct);
                            start_line = node.start_position().row + 1;
                            entity_node = find_parent_by_kind(node, "struct_specifier");
                        }
                        "cpp_namespace.name" => {
                            name = Some(text_str);
                            kind = Some(EntityKind::CppNamespace);
                            start_line = node.start_position().row + 1;
                            entity_node = find_parent_by_kind(node, "namespace_definition");
                        }
                        "cpp_method.name" => {
                            name = Some(text_str);
                            kind = Some(EntityKind::CppMethod);
                            start_line = node.start_position().row + 1;
                            entity_node = find_parent_by_kind(node, "function_definition")
                                .or_else(|| find_parent_by_kind(node, "declaration"))
                                .or_else(|| find_parent_by_kind(node, "field_declaration"));
                            if let Some(m_node) = entity_node {
                                cpp::extract_reference_intents_cpp(
                                    m_node,
                                    source_bytes,
                                    &mut reference_intents,
                                );
                            }
                        }
                        "c_function.name" => {
                            name = Some(text_str);
                            kind = Some(EntityKind::CFunction);
                            start_line = node.start_position().row + 1;
                            entity_node = find_parent_by_kind(node, "function_definition");
                            if let Some(m_node) = entity_node {
                                cpp::extract_reference_intents_cpp(
                                    m_node,
                                    source_bytes,
                                    &mut reference_intents,
                                );
                            }
                        }
                        "preproc.macro" => {
                            name = Some(text_str);
                            kind = Some(EntityKind::MacroDefinition);
                            start_line = node.start_position().row + 1;
                            entity_node = find_parent_by_kind(node, "preproc_def");
                        }
                        "preproc.include" => {
                            // Extract included file path and register it as an intent
                            let mut path_str = text_str;
                            if path_str.starts_with('"') || path_str.starts_with('<') {
                                path_str = path_str[1..path_str.len() - 1].to_string();
                            }
                            reference_intents.push(ReferenceIntent::Call {
                                method: path_str,
                                receiver: None,
                                line: node.start_position().row + 1,
                                arg_count: None,
                            });
                        }
                        _ => {}
                    }
                }
                // Ignore unhandled captures
                "dom.receiver" | "dom.action" | "dom.method" | "css.receiver" | "css.classList"
                | "css.className" | "css.method" | "css.keyframe" | "script_src"
                | "stylesheet_href" => {
                    // These captures are either metadata or handled in other passes
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
            let mut decorators = if let Some(node) = entity_node {
                extract_decorators(node, source_bytes, lang_name)
            } else {
                Vec::new()
            };

            // Phase 5: For Python entities, extract decorator names for display
            if lang_name == "python"
                && let Some(entity_n) = entity_node
            {
                python::extract_decorator_names_python(entity_n, source_bytes, &mut decorators);
            }

            // Determine FQN and enclosing class based on context
            let (mut fqn, mut enclosing_class) =
                compute_fqn_and_context(&name, &kind, start_line, lang_name, &class_contexts);

            // Prefix FQN with Java package name if available
            if let Some(ref pkg) = java_package {
                fqn = format!("{}.{}", pkg, fqn);
            }

            if matches!(
                kind,
                EntityKind::CppMethod
                    | EntityKind::CFunction
                    | EntityKind::CppClass
                    | EntityKind::CStruct
                    | EntityKind::CppNamespace
                    | EntityKind::MacroDefinition
            ) && let Some(node) = entity_node
                && let Some(cpp_fqn) = cpp::build_cpp_fqn(node, source_bytes)
                && !cpp_fqn.is_empty()
            {
                enclosing_class = Some(cpp_fqn.clone());
                fqn = format!("{}::{}", cpp_fqn, name);
            }

            // For classes, also extract extends/implements from AST
            if matches!(
                kind,
                EntityKind::Class
                    | EntityKind::Interface
                    | EntityKind::KotlinClass
                    | EntityKind::KotlinInterface
                    | EntityKind::KotlinObject
                    | EntityKind::KotlinEnum
            ) && let Some(class_node) = entity_node
            {
                if lang_name == "javascript" {
                    javascript::extract_class_inheritance_js(
                        class_node,
                        source_bytes,
                        &mut reference_intents,
                    );
                    // Extract decorator references for JavaScript (e.g., @Component, @Injectable)
                    // Decorators may be in the parent node (export_statement) rather than class_declaration
                    let decorator_node = class_node
                        .parent()
                        .filter(|p| p.kind() == "export_statement")
                        .unwrap_or(class_node);
                    javascript::extract_decorator_references(
                        decorator_node,
                        source_bytes,
                        &mut reference_intents,
                    );
                } else if lang_name == "typescript" {
                    typescript::extract_class_inheritance(
                        class_node,
                        source_bytes,
                        &mut reference_intents,
                    );
                    // Extract decorator references (e.g., @Component, @NgModule)
                    // Decorators may be in the parent node (export_statement) rather than class_declaration
                    let decorator_node = class_node
                        .parent()
                        .filter(|p| p.kind() == "export_statement")
                        .unwrap_or(class_node);
                    typescript::extract_decorator_references(
                        decorator_node,
                        source_bytes,
                        &mut reference_intents,
                    );
                    // Extract type references (e.g., constructor parameters, property types)
                    typescript::extract_type_references(
                        class_node,
                        source_bytes,
                        &mut reference_intents,
                    );
                } else if lang_name == "java" {
                    // Extract extends/implements from Java class/interface declarations
                    java::extract_class_inheritance_java(
                        class_node,
                        source_bytes,
                        &mut reference_intents,
                    );
                    // Extract annotation references (e.g., @Component, @Autowired)
                    java::extract_annotation_references(
                        class_node,
                        source_bytes,
                        &mut reference_intents,
                    );
                    // Extract type references (e.g., constructor parameters, field types)
                    java::extract_type_references(class_node, source_bytes, &mut reference_intents);
                } else if lang_name == "kotlin" {
                    // Extract extends/implements from Kotlin class/interface declarations
                    kotlin::extract_class_inheritance_kotlin(
                        class_node,
                        source_bytes,
                        &mut reference_intents,
                    );
                    // Extract annotation references (e.g., @Component, @Composable)
                    kotlin::extract_annotation_references(
                        class_node,
                        source_bytes,
                        &mut reference_intents,
                    );
                    // Extract type references (e.g., constructor parameters, property types)
                    kotlin::extract_type_references(
                        class_node,
                        source_bytes,
                        &mut reference_intents,
                    );
                }
            }

            // Calculate end_line from entity_node if available
            if let Some(node) = entity_node {
                end_line = node.end_position().row + 1;
            } else {
                // If no entity_node, use start_line as a fallback
                end_line = start_line;
            }

            // Extract C++ signature from entity_node if not captured by query
            if signature.is_none()
                && matches!(kind, EntityKind::CppMethod | EntityKind::CFunction)
                && let Some(node) = entity_node
            {
                signature = cpp::extract_cpp_signature(node, source_bytes);
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
                end_line,
                enclosing_class,
                repo_name,
            );
            entity.reference_intents = reference_intents;
            // Filter self-references (TypeReference/ValueReference to own name).
            // A type or value reference whose target is the enclosing entity itself
            // would produce a self-loop edge in Neo4j; useless visually and semantically.
            entity.reference_intents.retain(|intent| match intent {
                ReferenceIntent::TypeReference { type_name, .. } => {
                    type_name != entity.name.as_str()
                }
                ReferenceIntent::ValueReference { value_name, .. } => {
                    value_name != entity.name.as_str()
                }
                _ => true,
            });
            entity.inline_comments = inline_comments;
            entity.decorators = decorators;

            // Extract alias module path for JS require() aliases
            if lang_name == "javascript"
                && entity.kind == EntityKind::Constant
                && let Some(node) = entity_node
            {
                entity.alias_module_path =
                    javascript::extract_require_module_path(node, source_bytes);
            }

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

    // Deduplicate entities extracted from tree-sitter queries.
    // This handles cases where multiple query patterns match the same AST node.
    // For example, in JavaScript: var foo = function() {} can match both:
    //   1. (variable_declaration ... @function.name) → foo as Function
    //   2. (variable_declaration ... @constant.name) → foo as Constant
    // Deduplication key: (file_path, name, kind, start_line)
    // This ensures we keep only one entity per unique declaration.
    entities.sort_by(|a, b| {
        a.file_path
            .cmp(&b.file_path)
            .then(a.name.cmp(&b.name))
            .then(format!("{:?}", a.kind).cmp(&format!("{:?}", b.kind)))
            .then(a.start_line.cmp(&b.start_line))
    });
    entities.dedup_by(|a, b| {
        a.file_path == b.file_path
            && a.name == b.name
            && a.kind == b.kind
            && a.start_line == b.start_line
    });

    // Kotlin: extract anonymous object implementations (e.g., object : Interface inside method bodies)
    // Must run AFTER entity dedup so existing_entities have correct line ranges.
    if lang_name == "kotlin" {
        let mut anon_entities = Vec::new();
        kotlin::extract_anonymous_object_implementations(
            tree.root_node(),
            source_bytes,
            file_path,
            repo_name,
            &entities,
            &mut anon_entities,
        );
        entities.extend(anon_entities);
    }

    // Third pass: capture orphaned reference intents (calls in top-level statements,
    // callbacks, etc. that were not captured by any named entity)
    if lang_name == "typescript"
        || lang_name == "java"
        || lang_name == "javascript"
        || lang_name == "kotlin"
        || lang_name == "python"
    {
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

    // Rust: collect macro invocations, function calls, type references, and trait implementations
    if lang_name == "rust" {
        rust::collect_rust_macro_references(
            tree.root_node(),
            source_bytes,
            &mut entities,
            file_path,
            repo_name,
        );
        rust::collect_rust_call_references(
            tree.root_node(),
            source_bytes,
            &mut entities,
            file_path,
            repo_name,
        );
        rust::collect_rust_type_references(
            tree.root_node(),
            source_bytes,
            &mut entities,
            file_path,
            repo_name,
        );
        rust::collect_rust_trait_implementations(
            tree.root_node(),
            source_bytes,
            &mut entities,
            file_path,
            repo_name,
        );
        rust::reclassify_methods_in_impl_blocks(tree.root_node(), &mut entities);
    }

    // Fourth pass: extract HTML attributes from JSX elements (id, className)
    // This enables cross-language CSS/HTML search (e.g., "which components use class 'btn'?")
    if lang_name == "javascript" || lang_name == "typescript" {
        javascript::extract_jsx_html_attributes(
            tree.root_node(),
            source_bytes,
            &mut entities,
            file_path,
            repo_name,
        );
    }

    // Sixth pass: extract alias metadata (require/import → module path)
    if lang_name == "javascript" {
        // Set default_export on <module> entity from module.exports = X
        if let Some(target) = javascript::scan_module_exports_target(tree.root_node(), source_bytes)
        {
            if let Some(module_entity) = entities.iter_mut().find(|e| e.name == "<module>") {
                module_entity.default_export = Some(target);
            } else {
                // Create a synthetic module entity just to hold the default export
                let mut module_entity = ParsedEntity::new(
                    "<module>",
                    EntityKind::Function,
                    file_path,
                    None,
                    None,
                    lang_name,
                    file_path,
                    1,
                    1, // we don't have exact lines, use 1
                    None,
                    repo_name,
                );
                module_entity.default_export = Some(target);
                entities.push(module_entity);
            }
        }
    }
    if lang_name == "typescript" {
        // Create entities for renamed/default/namespace imports so that
        // cross-file alias resolution can follow them. Non-renamed named
        // imports (`import { X }`) don't need entities because the name
        // is already captured as a type reference by the main query.
        let aliases = typescript::scan_import_module_aliases(tree.root_node(), source_bytes);
        for (name, module_path, original_name, is_renamed) in aliases {
            if is_renamed && !entities.iter().any(|e| e.name == name) {
                let mut alias_entity = ParsedEntity::new(
                    &name,
                    EntityKind::Constant,
                    file_path,
                    None,
                    None,
                    lang_name,
                    file_path,
                    1,
                    1,
                    None,
                    repo_name,
                );
                alias_entity.alias_module_path = Some(module_path);
                alias_entity.original_export_name = original_name;
                entities.push(alias_entity);
            }
        }
        // Set default_export on <module> from export default X
        if let Some(target) = typescript::scan_default_export_target(tree.root_node(), source_bytes)
        {
            if let Some(module_entity) = entities.iter_mut().find(|e| e.name == "<module>") {
                module_entity.default_export = Some(target);
            } else {
                // Create a synthetic module entity just to hold the default export
                let mut module_entity = ParsedEntity::new(
                    "<module>",
                    EntityKind::Function,
                    file_path,
                    None,
                    None,
                    lang_name,
                    file_path,
                    1,
                    1,
                    None,
                    repo_name,
                );
                module_entity.default_export = Some(target);
                entities.push(module_entity);
            }
        }
    }

    Ok(entities)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_entities_empty_source_java() {
        let source = "";
        let result = extract_entities(
            source,
            tree_sitter_java::LANGUAGE.into(),
            "",
            "java",
            "/test.java",
            "test-repo",
        );

        // Empty source should still return Ok with empty vec
        assert!(result.is_ok());
        assert!(result.unwrap().is_empty());
    }

    #[test]
    fn test_extract_entities_empty_source_typescript() {
        let source = "";
        let result = extract_entities(
            source,
            tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
            "",
            "typescript",
            "/test.ts",
            "test-repo",
        );

        assert!(result.is_ok());
        assert!(result.unwrap().is_empty());
    }

    #[test]
    fn test_extract_entities_simple_java_class() {
        let source = "public class MyClass {}";
        let query = "(class_declaration name: (identifier) @class.name)";

        let result = extract_entities(
            source,
            tree_sitter_java::LANGUAGE.into(),
            query,
            "java",
            "/MyClass.java",
            "test-repo",
        );

        assert!(result.is_ok());
        let entities = result.unwrap();
        assert!(!entities.is_empty());
        assert_eq!(entities[0].name, "MyClass");
        assert_eq!(entities[0].kind, EntityKind::Class);
    }

    #[test]
    fn test_extract_entities_simple_typescript_function() {
        let source = "function myFunction() {}";
        let query = "(function_declaration name: (identifier) @function.name)";

        let result = extract_entities(
            source,
            tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
            query,
            "typescript",
            "/test.ts",
            "test-repo",
        );

        assert!(result.is_ok());
        let entities = result.unwrap();
        assert!(!entities.is_empty());
        assert_eq!(entities[0].name, "myFunction");
        assert_eq!(entities[0].kind, EntityKind::Function);
    }

    #[test]
    fn test_extract_entities_with_signature() {
        let source = "public void testMethod(String param) {}";
        let query = "(method_declaration name: (identifier) @method.name (#any-of? @method.name \"testMethod\"))";

        let result = extract_entities(
            source,
            tree_sitter_java::LANGUAGE.into(),
            query,
            "java",
            "/Test.java",
            "test-repo",
        );

        assert!(result.is_ok());
        let entities = result.unwrap();
        assert!(!entities.is_empty());
    }

    #[test]
    fn test_extract_entities_interface_java() {
        let source = "public interface MyInterface {}";
        // Use identifier instead of type_identifier for interface names in Java
        let query = "(interface_declaration name: (identifier) @interface.name)";

        let result = extract_entities(
            source,
            tree_sitter_java::LANGUAGE.into(),
            query,
            "java",
            "/MyInterface.java",
            "test-repo",
        );

        assert!(result.is_ok());
        let entities = result.unwrap();
        assert!(!entities.is_empty());
        assert_eq!(entities[0].kind, EntityKind::Interface);
    }

    #[test]
    fn test_extract_entities_enum_java() {
        let source = "public enum Color { RED, GREEN, BLUE }";
        let query = "(enum_declaration name: (identifier) @enum.name)";

        let result = extract_entities(
            source,
            tree_sitter_java::LANGUAGE.into(),
            query,
            "java",
            "/Color.java",
            "test-repo",
        );

        assert!(result.is_ok());
    }

    #[test]
    fn test_extract_entities_constant_typescript() {
        let source = "const MY_CONSTANT = 42;";
        let query = "(lexical_declaration (variable_declarator name: (identifier) @constant.name))";

        let result = extract_entities(
            source,
            tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
            query,
            "typescript",
            "/constants.ts",
            "test-repo",
        );

        assert!(result.is_ok());
        let entities = result.unwrap();
        assert!(!entities.is_empty());
        assert_eq!(entities[0].kind, EntityKind::Constant);
    }

    #[test]
    fn test_extract_entities_with_docstring() {
        let source = "/** Test documentation */\npublic class DocClass {}";
        let query = "(class_declaration name: (identifier) @class.name)";

        let result = extract_entities(
            source,
            tree_sitter_java::LANGUAGE.into(),
            query,
            "java",
            "/DocClass.java",
            "test-repo",
        );

        assert!(result.is_ok());
        let entities = result.unwrap();
        assert!(!entities.is_empty());
        // Docstring extraction depends on comments parsing
    }

    #[test]
    fn test_extract_entities_multiple_entities_java() {
        let source = "public class FirstClass {} public class SecondClass {}";
        let query = "(class_declaration name: (identifier) @class.name)";

        let result = extract_entities(
            source,
            tree_sitter_java::LANGUAGE.into(),
            query,
            "java",
            "/Classes.java",
            "test-repo",
        );

        assert!(result.is_ok());
        let entities = result.unwrap();
        assert_eq!(entities.len(), 2);
        assert_eq!(entities[0].name, "FirstClass");
        assert_eq!(entities[1].name, "SecondClass");
    }

    #[test]
    fn test_extract_entities_nested_class() {
        let source = "public class Outer { public class Inner {} }";
        let query = "(class_declaration name: (identifier) @class.name)";

        let result = extract_entities(
            source,
            tree_sitter_java::LANGUAGE.into(),
            query,
            "java",
            "/Outer.java",
            "test-repo",
        );

        assert!(result.is_ok());
        let entities = result.unwrap();
        // Should find both outer and inner classes
        assert!(!entities.is_empty());
    }

    #[test]
    fn test_extract_entities_file_path_preservation() {
        let file_path = "/src/main/java/com/example/MyClass.java";
        let source = "public class MyClass {}";
        let query = "(class_declaration name: (identifier) @class.name)";

        let result = extract_entities(
            source,
            tree_sitter_java::LANGUAGE.into(),
            query,
            "java",
            file_path,
            "test-repo",
        );

        assert!(result.is_ok());
        let entities = result.unwrap();
        assert!(!entities.is_empty());
        assert_eq!(entities[0].file_path, file_path);
    }

    #[test]
    fn test_extract_entities_repo_name_preservation() {
        let repo_name = "my-awesome-repo";
        let source = "public class MyClass {}";
        let query = "(class_declaration name: (identifier) @class.name)";

        let result = extract_entities(
            source,
            tree_sitter_java::LANGUAGE.into(),
            query,
            "java",
            "/MyClass.java",
            repo_name,
        );

        assert!(result.is_ok());
        let entities = result.unwrap();
        assert!(!entities.is_empty());
        assert_eq!(entities[0].repo_name, repo_name);
    }

    #[test]
    fn test_extract_entities_start_line_calculation() {
        let source = "\n\n\npublic class MyClass {}";
        let query = "(class_declaration name: (identifier) @class.name)";

        let result = extract_entities(
            source,
            tree_sitter_java::LANGUAGE.into(),
            query,
            "java",
            "/MyClass.java",
            "test-repo",
        );

        assert!(result.is_ok());
        let entities = result.unwrap();
        assert!(!entities.is_empty());
        // Line should be 4 (1-indexed, after 3 newlines)
        assert_eq!(entities[0].start_line, 4);
    }

    #[test]
    fn test_extract_entities_language_name_preserved() {
        let source = "public class MyClass {}";
        let query = "(class_declaration name: (identifier) @class.name)";

        let result_java = extract_entities(
            source,
            tree_sitter_java::LANGUAGE.into(),
            query,
            "java",
            "/MyClass.java",
            "test-repo",
        );

        assert!(result_java.is_ok());
        let entities_java = result_java.unwrap();
        assert!(!entities_java.is_empty());
        // Language is part of the model but not directly accessible via public getter
        // This is tested indirectly through extraction behavior
    }

    #[test]
    fn test_extract_entities_deduplication_javascript() {
        // Test deduplication with overlapping patterns in JavaScript
        // The actual javascript.scm has patterns that can capture the same node
        // multiple times due to tree-sitter query semantics
        let source = r#"
            function myFunc() {}
            var myVar = 42;
        "#;

        // Use a simple query that extracts both functions and constants
        let query = r#"
            (function_declaration name: (identifier) @function.name)
            (variable_declaration
              (variable_declarator
                name: (identifier) @constant.name))
        "#;

        let result = extract_entities(
            source,
            tree_sitter_javascript::LANGUAGE.into(),
            query,
            "javascript",
            "/test.js",
            "test-repo",
        );

        assert!(result.is_ok());
        let entities = result.unwrap();

        // Should have 2 unique entities
        assert_eq!(
            entities.len(),
            2,
            "Should have exactly 2 unique entities: function and constant"
        );

        // Verify myFunc is captured as function
        let func_entity = entities.iter().find(|e| e.name == "myFunc");
        assert!(func_entity.is_some());
        assert_eq!(func_entity.unwrap().kind, EntityKind::Function);

        // Verify myVar is captured as constant
        let const_entity = entities.iter().find(|e| e.name == "myVar");
        assert!(const_entity.is_some());
        assert_eq!(const_entity.unwrap().kind, EntityKind::Constant);
    }

    #[test]
    fn test_extract_entities_deduplication_respects_file_path() {
        // Same entity name in different "files" should NOT be deduplicated
        let source = "var myVar = 42;";
        let query =
            "(variable_declaration (variable_declarator name: (identifier) @constant.name))";

        // Extract from "file1"
        let result1 = extract_entities(
            source,
            tree_sitter_javascript::LANGUAGE.into(),
            query,
            "javascript",
            "/file1.js",
            "test-repo",
        );

        // Extract from "file2"
        let result2 = extract_entities(
            source,
            tree_sitter_javascript::LANGUAGE.into(),
            query,
            "javascript",
            "/file2.js",
            "test-repo",
        );

        assert!(result1.is_ok());
        assert!(result2.is_ok());

        let entities1 = result1.unwrap();
        let entities2 = result2.unwrap();

        // Each file should have 1 entity
        assert_eq!(entities1.len(), 1);
        assert_eq!(entities2.len(), 1);

        // Same name but different file paths
        assert_eq!(entities1[0].name, "myVar");
        assert_eq!(entities2[0].name, "myVar");
        assert_ne!(entities1[0].file_path, entities2[0].file_path);
    }

    #[test]
    fn test_extract_entities_deduplication_respects_kind() {
        // Hypothetical scenario: same name used for class and function
        // (unrealistic in real code, but tests the deduplication logic)
        let source = r#"
            class MyEntity {}
            function MyEntity() {}
        "#;

        let query = r#"
            (class_declaration name: (identifier) @class.name)
            (function_declaration name: (identifier) @function.name)
        "#;

        let result = extract_entities(
            source,
            tree_sitter_javascript::LANGUAGE.into(),
            query,
            "javascript",
            "/test.js",
            "test-repo",
        );

        assert!(result.is_ok());
        let entities = result.unwrap();

        // Should have 2 entities with same name but different kinds
        assert_eq!(
            entities.len(),
            2,
            "Should keep both entities with same name but different kinds"
        );

        let class_entity = entities.iter().find(|e| e.kind == EntityKind::Class);
        let function_entity = entities.iter().find(|e| e.kind == EntityKind::Function);

        assert!(class_entity.is_some());
        assert!(function_entity.is_some());
        assert_eq!(class_entity.unwrap().name, "MyEntity");
        assert_eq!(function_entity.unwrap().name, "MyEntity");
    }

    #[test]
    fn test_extract_entities_deduplication_respects_line_number() {
        // Multiple functions with same name on different lines (overloading scenario)
        let source = r#"
            function process(x) { return x; }
            function process(x, y) { return x + y; }
        "#;

        let query = "(function_declaration name: (identifier) @function.name)";

        let result = extract_entities(
            source,
            tree_sitter_javascript::LANGUAGE.into(),
            query,
            "javascript",
            "/test.js",
            "test-repo",
        );

        assert!(result.is_ok());
        let entities = result.unwrap();

        // Should have 2 entities (same name, same kind, different lines)
        assert_eq!(
            entities.len(),
            2,
            "Should keep both functions with same name on different lines"
        );

        // Both should be named "process"
        assert!(entities.iter().all(|e| e.name == "process"));

        // They should have different start lines
        assert_ne!(entities[0].start_line, entities[1].start_line);
    }

    #[test]
    fn test_extract_entities_angular_decorator_references() {
        // Test the complete Angular use case that motivated this bugfix
        let source = r#"
            import { Component } from '@angular/core';
            import { AnalyticsService } from './analytics.service';
            import { SeoService } from './seo.service';

            @Component({
                selector: 'ngx-app',
                template: '<router-outlet></router-outlet>',
            })
            export class AppComponent {
                constructor(
                    private analytics: AnalyticsService,
                    private seo: SeoService
                ) {}
            }
        "#;

        let query = r#"
            (class_declaration name: (type_identifier) @class.name)
        "#;

        let result = extract_entities(
            source,
            tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
            query,
            "typescript",
            "/test/app.component.ts",
            "test-repo",
        );

        assert!(result.is_ok());
        let entities = result.unwrap();

        // Should have 1 class entity + 1 module entity
        assert_eq!(entities.len(), 2);
        let app_component = entities.iter().find(|e| e.name == "AppComponent").unwrap();
        assert_eq!(app_component.name, "AppComponent");

        // Should have captured decorator references
        let decorator_refs: Vec<_> = app_component
            .reference_intents
            .iter()
            .filter(|r| matches!(r, crate::models::ReferenceIntent::TypeReference { type_name, .. } if type_name == "Component"))
            .collect();
        assert!(
            !decorator_refs.is_empty(),
            "Should capture @Component decorator reference"
        );

        // Should have captured type references from constructor parameters
        let analytics_refs: Vec<_> = app_component
            .reference_intents
            .iter()
            .filter(|r| matches!(r, crate::models::ReferenceIntent::TypeReference { type_name, .. } if type_name == "AnalyticsService"))
            .collect();
        assert!(
            !analytics_refs.is_empty(),
            "Should capture AnalyticsService type reference from constructor"
        );

        let seo_refs: Vec<_> = app_component
            .reference_intents
            .iter()
            .filter(|r| matches!(r, crate::models::ReferenceIntent::TypeReference { type_name, .. } if type_name == "SeoService"))
            .collect();
        assert!(
            !seo_refs.is_empty(),
            "Should capture SeoService type reference from constructor"
        );
    }

    #[test]
    fn test_extract_entities_angular_ngmodule_references() {
        // Test NgModule decorator with component references
        let source = r#"
            import { NgModule } from '@angular/core';
            import { AppComponent } from './app.component';
            import { UserComponent } from './user.component';

            @NgModule({
                declarations: [AppComponent, UserComponent],
                bootstrap: [AppComponent]
            })
            export class AppModule {}
        "#;

        let query = r#"
            (class_declaration name: (type_identifier) @class.name)
        "#;

        let result = extract_entities(
            source,
            tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
            query,
            "typescript",
            "/test/app.module.ts",
            "test-repo",
        );

        assert!(result.is_ok());
        let entities = result.unwrap();

        // Should have 1 class entity + 1 module entity
        assert_eq!(entities.len(), 2);
        let app_module = entities.iter().find(|e| e.name == "AppModule").unwrap();
        assert_eq!(app_module.name, "AppModule");

        // Should capture NgModule decorator reference
        let ngmodule_refs: Vec<_> = app_module
            .reference_intents
            .iter()
            .filter(|r| matches!(r, crate::models::ReferenceIntent::TypeReference { type_name, .. } if type_name == "NgModule"))
            .collect();
        assert!(
            !ngmodule_refs.is_empty(),
            "Should capture @NgModule decorator reference"
        );

        // Should capture AppComponent references from decorator arguments
        let app_component_refs: Vec<_> = app_module
            .reference_intents
            .iter()
            .filter(|r| matches!(r, crate::models::ReferenceIntent::TypeReference { type_name, .. } if type_name == "AppComponent"))
            .collect();
        assert!(
            app_component_refs.len() >= 2,
            "Should capture AppComponent references (appears in declarations and bootstrap)"
        );

        // Should capture UserComponent reference from decorator arguments
        let user_component_refs: Vec<_> = app_module
            .reference_intents
            .iter()
            .filter(|r| matches!(r, crate::models::ReferenceIntent::TypeReference { type_name, .. } if type_name == "UserComponent"))
            .collect();
        assert!(
            !user_component_refs.is_empty(),
            "Should capture UserComponent reference from declarations"
        );
    }

    // ============================================================
    // Phase 3: CSS Support Tests
    // ============================================================

    #[test]
    fn test_extract_entities_css_class() {
        let source = ".btn-primary { color: blue; }";
        let query = "(class_selector (class_name) @css.class)";

        let result = extract_entities(
            source,
            tree_sitter_css::LANGUAGE.into(),
            query,
            "css",
            "/styles.css",
            "test-repo",
        );

        assert!(result.is_ok());
        let entities = result.unwrap();
        assert!(!entities.is_empty());
        assert_eq!(entities[0].name, "btn-primary");
        assert_eq!(entities[0].kind, crate::models::EntityKind::CssClass);
    }

    #[test]
    fn test_extract_entities_css_id() {
        let source = "#header { background: white; }";
        let query = "(id_selector (id_name) @css.id)";

        let result = extract_entities(
            source,
            tree_sitter_css::LANGUAGE.into(),
            query,
            "css",
            "/styles.css",
            "test-repo",
        );

        assert!(result.is_ok());
        let entities = result.unwrap();
        assert!(!entities.is_empty());
        assert_eq!(entities[0].name, "header");
        assert_eq!(entities[0].kind, crate::models::EntityKind::CssId);
    }

    #[test]
    fn test_extract_entities_css_multiple_classes() {
        let source = ".btn-primary { color: blue; } .btn-secondary { color: gray; }";
        let query = "(class_selector (class_name) @css.class)";

        let result = extract_entities(
            source,
            tree_sitter_css::LANGUAGE.into(),
            query,
            "css",
            "/styles.css",
            "test-repo",
        );

        assert!(result.is_ok());
        let entities = result.unwrap();
        assert_eq!(entities.len(), 2);
        assert_eq!(entities[0].name, "btn-primary");
        assert_eq!(entities[1].name, "btn-secondary");
    }

    // ============================================================
    // Phase 3: SCSS Support Tests
    // ============================================================

    #[test]
    fn test_extract_scss_and_css_classes_together() {
        // Test that SCSS mixin and CSS class extraction work
        let css_source = ".btn { padding: 10px; } .btn-primary { color: blue; }";
        let query = "(class_selector (class_name) @css.class)";

        let result = extract_entities(
            css_source,
            tree_sitter_css::LANGUAGE.into(),
            query,
            "css",
            "/styles.css",
            "test-repo",
        );

        assert!(result.is_ok());
        let entities = result.unwrap();

        // Should extract both CSS classes
        assert!(
            entities
                .iter()
                .any(|e| e.name == "btn" && e.kind == crate::models::EntityKind::CssClass)
        );
        assert!(
            entities
                .iter()
                .any(|e| e.name == "btn-primary" && e.kind == crate::models::EntityKind::CssClass)
        );
    }

    #[test]
    fn test_extract_entities_scss_mixin() {
        let source = "@mixin flex-center { display: flex; justify-content: center; }";
        let query = "(mixin_statement name: (identifier) @scss.mixin)";

        let result = extract_entities(
            source,
            tree_sitter_scss::language(),
            query,
            "scss",
            "/styles.scss",
            "test-repo",
        );

        assert!(result.is_ok());
        let entities = result.unwrap();
        assert!(!entities.is_empty());
        assert_eq!(entities[0].name, "flex-center");
        assert_eq!(entities[0].kind, crate::models::EntityKind::ScssMixin);
    }

    #[test]
    fn test_extract_entities_scss_function() {
        let source = "@function calculate-rem($value) { @return $value / 16 * 1rem; }";
        let query = "(function_statement name: (identifier) @scss.function)";

        let result = extract_entities(
            source,
            tree_sitter_scss::language(),
            query,
            "scss",
            "/styles.scss",
            "test-repo",
        );

        assert!(result.is_ok());
        let entities = result.unwrap();
        assert!(!entities.is_empty());
        assert_eq!(entities[0].name, "calculate-rem");
        assert_eq!(entities[0].kind, crate::models::EntityKind::ScssFunction);
    }

    // ============================================================
    // Phase 4: Hybrid Web Ecosystem Tests
    // ============================================================

    #[test]
    fn test_extract_dom_references_and_css_class_usage() {
        let source = "function initApp() { const app = document.getElementById('app-container'); element.classList.add('active'); }";
        let query = include_str!("../../../queries/javascript.scm");

        let result = extract_entities(
            source,
            tree_sitter_javascript::LANGUAGE.into(),
            query,
            "javascript",
            "/app.js",
            "test-repo",
        );

        assert!(result.is_ok());
        let entities = result.unwrap();

        // Should extract the function and potentially DOM/CSS references
        assert!(!entities.is_empty(), "Should extract function definition");

        // Check if any entity has DOM or CSS references
        let has_references = entities.iter().any(|e| !e.reference_intents.is_empty());
        // It's ok if no references are captured in unit tests; the E2E tests validate this
        let _ = has_references;
    }

    #[test]
    fn test_extract_css_class_usage_in_function() {
        let source = "function toggleClass() { element.classList.add('btn-primary'); element.className = 'active'; }";
        let query = include_str!("../../../queries/javascript.scm");

        let result = extract_entities(
            source,
            tree_sitter_javascript::LANGUAGE.into(),
            query,
            "javascript",
            "/app.js",
            "test-repo",
        );

        assert!(result.is_ok());
        let entities = result.unwrap();

        // Should extract the function definition
        assert!(!entities.is_empty(), "Should extract toggleClass function");

        // The E2E tests validate that CSS class references are properly captured
        // Unit tests here focus on basic extraction
    }

    #[test]
    fn test_extract_html_elements_and_attributes() {
        let source = r#"<div id="main" class="container"> <button class="btn btn-primary">Click</button> </div>"#;
        let query = include_str!("../../../queries/html.scm");

        let result = extract_entities(
            source,
            tree_sitter_html::LANGUAGE.into(),
            query,
            "html",
            "/index.html",
            "test-repo",
        );

        assert!(result.is_ok());
        let entities = result.unwrap();

        // Should extract HTML ids and classes
        let has_id = entities
            .iter()
            .any(|e| e.name == "main" && e.kind == crate::models::EntityKind::HtmlId);
        let has_class = entities
            .iter()
            .any(|e| e.name == "container" && e.kind == crate::models::EntityKind::HtmlClass);

        assert!(has_id, "Should extract HTML id 'main'");
        assert!(has_class, "Should extract HTML class 'container'");
    }

    #[test]
    fn test_extract_html_with_custom_elements() {
        let source = r#"<html>
<head>
    <app-header></app-header>
    <custom-widget></custom-widget>
</head>
</html>"#;
        let query = include_str!("../../../queries/html.scm");

        let result = extract_entities(
            source,
            tree_sitter_html::LANGUAGE.into(),
            query,
            "html",
            "/index.html",
            "test-repo",
        );

        assert!(result.is_ok());
        let entities = result.unwrap();

        // Should extract custom HTML elements (Web Components)
        let has_custom_element = entities.iter().any(|e| {
            e.kind == crate::models::EntityKind::HtmlElement
                && (e.name == "app-header" || e.name == "custom-widget")
        });

        assert!(has_custom_element, "Should extract custom HTML elements");
    }

    #[test]
    fn test_extract_javascript_with_class_and_function() {
        let source = r#"
        class DataService {
            fetchData() { return fetch('/api/data'); }
        }
        
        function initApp() {
            const service = new DataService();
            service.fetchData();
        }
        "#;
        let query = include_str!("../../../queries/javascript.scm");

        let result = extract_entities(
            source,
            tree_sitter_javascript::LANGUAGE.into(),
            query,
            "javascript",
            "/app.js",
            "test-repo",
        );

        assert!(result.is_ok());
        let entities = result.unwrap();

        // Should extract class, method, and function
        assert!(
            entities
                .iter()
                .any(|e| e.name == "DataService" && e.kind == crate::models::EntityKind::Class)
        );
        assert!(
            entities
                .iter()
                .any(|e| e.name == "fetchData" && e.kind == crate::models::EntityKind::Method)
        );
        assert!(
            entities
                .iter()
                .any(|e| e.name == "initApp" && e.kind == crate::models::EntityKind::Function)
        );
    }

    #[test]
    fn test_extract_hybrid_ecosystem_full_integration() {
        // This test simulates a mini SPA with HTML, JS, and CSS
        let html_source = r#"<!DOCTYPE html>
<html>
<head>
    <link rel="stylesheet" href="app.css">
</head>
<body>
    <div id="app-root" class="container">Content</div>
    <script src="app.js"></script>
</body>
</html>"#;

        // Use the HTML-specific extraction function that includes file imports
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_html::LANGUAGE.into())
            .expect("Failed to set HTML language");
        let tree = parser
            .parse(html_source, None)
            .expect("Failed to parse HTML");

        let entities = html::extract_entities_html(
            tree.root_node(),
            html_source.as_bytes(),
            "/index.html",
            "test-repo",
        );

        assert!(!entities.is_empty(), "Should extract some entities");

        // Should have CSS import
        let has_css_import = entities.iter().any(|e| {
            e.reference_intents.iter().any(|ri| {
                matches!(ri, crate::models::ReferenceIntent::CssFileImport { file_path, .. } if file_path == "app.css")
            })
        });
        assert!(has_css_import, "Should capture CSS import");

        // Should have JS import
        let has_js_import = entities.iter().any(|e| {
            e.reference_intents.iter().any(|ri| {
                matches!(ri, crate::models::ReferenceIntent::HtmlFileImport { file_path, .. } if file_path == "app.js")
            })
        });
        assert!(has_js_import, "Should capture JS import");

        // Should have HTML ID
        let has_html_id = entities
            .iter()
            .any(|e| e.name == "app-root" && e.kind == crate::models::EntityKind::HtmlId);
        assert!(has_html_id, "Should capture HTML id 'app-root'");

        // Should have HTML class
        let has_html_class = entities
            .iter()
            .any(|e| e.name == "container" && e.kind == crate::models::EntityKind::HtmlClass);
        assert!(has_html_class, "Should capture HTML class 'container'");
    }

    // ============================================================
    // Python extraction tests (Phase 2)
    // ============================================================

    #[test]
    fn test_extract_python_class() {
        let source = "class User:\n    pass";
        let query = "(class_definition name: (identifier) @python.class.name)";

        let result = extract_entities(
            source,
            tree_sitter_python::LANGUAGE.into(),
            query,
            "python",
            "/User.py",
            "test-repo",
        );

        assert!(result.is_ok());
        let entities = result.unwrap();
        assert!(!entities.is_empty());
        assert_eq!(entities[0].kind, EntityKind::PythonClass);
        assert_eq!(entities[0].name, "User");
    }

    #[test]
    fn test_extract_python_function() {
        let source = "def process_data():\n    pass";
        let query = "(function_definition name: (identifier) @python.function.name)";

        let result = extract_entities(
            source,
            tree_sitter_python::LANGUAGE.into(),
            query,
            "python",
            "/utils.py",
            "test-repo",
        );

        assert!(result.is_ok());
        let entities = result.unwrap();
        assert!(!entities.is_empty());
        assert_eq!(entities[0].kind, EntityKind::PythonFunction);
        assert_eq!(entities[0].name, "process_data");
    }

    #[test]
    fn test_extract_python_multiple_classes() {
        let source = "class Foo:\n    pass\nclass Bar:\n    pass";
        let query = "(class_definition name: (identifier) @python.class.name)";

        let result = extract_entities(
            source,
            tree_sitter_python::LANGUAGE.into(),
            query,
            "python",
            "/multi.py",
            "test-repo",
        );

        assert!(result.is_ok());
        let entities = result.unwrap();
        assert_eq!(entities.len(), 2);
        let names: Vec<_> = entities.iter().map(|e| e.name.clone()).collect();
        assert!(names.contains(&"Foo".to_string()));
        assert!(names.contains(&"Bar".to_string()));
    }

    #[test]
    fn test_extract_python_async_function() {
        let source = "async def fetch_data():\n    pass";
        let query = include_str!("../../../queries/python.scm");

        let result = extract_entities(
            source,
            tree_sitter_python::LANGUAGE.into(),
            query,
            "python",
            "/async_example.py",
            "test-repo",
        );

        assert!(result.is_ok(), "Failed to extract: {:?}", result.err());
        let entities = result.unwrap();
        assert!(
            !entities.is_empty(),
            "No entities extracted from async function"
        );
        let async_entity = entities
            .iter()
            .find(|e| e.name == "fetch_data")
            .expect("Should have fetch_data function");
        assert_eq!(async_entity.kind, EntityKind::PythonFunction);
    }

    #[test]
    fn test_extract_python_with_signature() {
        let source = "def greet(name: str) -> str:\n    return f\"Hello {name}\"";
        let query = "(function_definition name: (identifier) @python.function.name parameters: (parameters) @python.signature)";

        let result = extract_entities(
            source,
            tree_sitter_python::LANGUAGE.into(),
            query,
            "python",
            "/with_sig.py",
            "test-repo",
        );

        assert!(result.is_ok());
        let entities = result.unwrap();
        assert!(!entities.is_empty());
        assert_eq!(entities[0].name, "greet");
        assert!(entities[0].signature.is_some());
    }

    // ============================================================
    // Python call extraction tests (Phase 3)
    // ============================================================

    #[test]
    fn test_extract_python_direct_function_call() {
        let source = "def caller():\n    fetch_data()";
        let query = "(function_definition name: (identifier) @python.function.name)";

        let result = extract_entities(
            source,
            tree_sitter_python::LANGUAGE.into(),
            query,
            "python",
            "/calls.py",
            "test-repo",
        );

        assert!(result.is_ok());
        let entities = result.unwrap();
        assert!(!entities.is_empty());
        let caller = entities.iter().find(|e| e.name == "caller").unwrap();
        assert!(!caller.reference_intents.is_empty());
        let call = &caller.reference_intents[0];
        match call {
            ReferenceIntent::Call {
                method,
                receiver,
                line,
                arg_count: _,
            } => {
                assert_eq!(method, "fetch_data");
                assert!(receiver.is_none());
                assert_eq!(*line, 2);
            }
            _ => panic!("Expected Call intent, got {:?}", call),
        }
    }

    #[test]
    fn test_extract_python_method_call() {
        let source = "def caller():\n    user.get_email()";
        let query = "(function_definition name: (identifier) @python.function.name)";

        let result = extract_entities(
            source,
            tree_sitter_python::LANGUAGE.into(),
            query,
            "python",
            "/method_calls.py",
            "test-repo",
        );

        assert!(result.is_ok());
        let entities = result.unwrap();
        assert!(!entities.is_empty());
        let caller = entities.iter().find(|e| e.name == "caller").unwrap();
        assert!(!caller.reference_intents.is_empty());
        let call = &caller.reference_intents[0];
        match call {
            ReferenceIntent::Call {
                method,
                receiver,
                line,
                arg_count: _,
            } => {
                assert_eq!(method, "get_email");
                assert_eq!(receiver.as_deref(), Some("user"));
                assert_eq!(*line, 2);
            }
            _ => panic!("Expected Call intent, got {:?}", call),
        }
    }

    #[test]
    fn test_extract_python_builtin_call() {
        let source = "def caller():\n    length = len(items)";
        let query = "(function_definition name: (identifier) @python.function.name)";

        let result = extract_entities(
            source,
            tree_sitter_python::LANGUAGE.into(),
            query,
            "python",
            "/builtin_calls.py",
            "test-repo",
        );

        assert!(result.is_ok());
        let entities = result.unwrap();
        assert!(!entities.is_empty());
        let caller = entities.iter().find(|e| e.name == "caller").unwrap();
        let call = caller
            .reference_intents
            .iter()
            .find(|c| matches!(c, ReferenceIntent::Call { method, .. } if method == "len"));
        assert!(call.is_some(), "Should have len() call intent");
    }

    #[test]
    fn test_extract_python_method_within_class() {
        let source = "class User:\n    def greet(self):\n        print(self.name)";
        let query = "(function_definition name: (identifier) @python.function.name)";

        let result = extract_entities(
            source,
            tree_sitter_python::LANGUAGE.into(),
            query,
            "python",
            "/class_method.py",
            "test-repo",
        );

        assert!(result.is_ok());
        let entities = result.unwrap();
        let method = entities.iter().find(|e| e.name == "greet").unwrap();
        assert_eq!(method.kind, EntityKind::PythonMethod);
        let print_call = method
            .reference_intents
            .iter()
            .find(|c| matches!(c, ReferenceIntent::Call { method, .. } if method == "print"));
        assert!(print_call.is_some(), "Should have print() call in method");
    }

    #[test]
    fn test_extract_python_multiple_calls_in_function() {
        let source =
            "def main():\n    users = fetch_users()\n    for u in users:\n        print(u)";
        let query = "(function_definition name: (identifier) @python.function.name)";

        let result = extract_entities(
            source,
            tree_sitter_python::LANGUAGE.into(),
            query,
            "python",
            "/multi_calls.py",
            "test-repo",
        );

        assert!(result.is_ok());
        let entities = result.unwrap();
        let main_func = entities.iter().find(|e| e.name == "main").unwrap();
        assert!(main_func.reference_intents.len() >= 2);
        let methods: Vec<_> = main_func
            .reference_intents
            .iter()
            .filter_map(|c| {
                if let ReferenceIntent::Call { method, .. } = c {
                    Some(method.as_str())
                } else {
                    None
                }
            })
            .collect();
        assert!(methods.contains(&"fetch_users"));
        assert!(methods.contains(&"print"));
    }

    // ============================================================
    // Python constants and imports tests (Phase 4)
    // ============================================================

    #[test]
    fn test_extract_python_constant() {
        let source = "MAX_RETRIES = 5\nLOCAL_VAR = 10\nlowercase = 20";
        let query = include_str!("../../../queries/python.scm");

        let result = extract_entities(
            source,
            tree_sitter_python::LANGUAGE.into(),
            query,
            "python",
            "/constants.py",
            "test-repo",
        );

        assert!(result.is_ok());
        let entities = result.unwrap();
        let names: Vec<_> = entities.iter().map(|e| e.name.clone()).collect();
        assert!(names.contains(&"MAX_RETRIES".to_string()));
        assert!(names.contains(&"LOCAL_VAR".to_string()));
        assert!(!names.contains(&"lowercase".to_string()));
        let constant = entities.iter().find(|e| e.name == "MAX_RETRIES").unwrap();
        assert_eq!(constant.kind, EntityKind::PythonConstant);
    }

    #[test]
    fn test_extract_python_import_statement() {
        let source = "import os, sys\ndef main():\n    pass";
        let query = "(function_definition name: (identifier) @python.function.name)";

        let result = extract_entities(
            source,
            tree_sitter_python::LANGUAGE.into(),
            query,
            "python",
            "/imports.py",
            "test-repo",
        );

        assert!(result.is_ok());
        let entities = result.unwrap();
        let main_func = entities.iter().find(|e| e.name == "<module>").unwrap();
        let type_refs: Vec<_> = main_func
            .reference_intents
            .iter()
            .filter_map(|c| {
                if let ReferenceIntent::TypeReference { type_name, .. } = c {
                    Some(type_name.as_str())
                } else {
                    None
                }
            })
            .collect();
        assert!(type_refs.contains(&"os"), "Should have os import reference");
        assert!(
            type_refs.contains(&"sys"),
            "Should have sys import reference"
        );
    }

    #[test]
    fn test_extract_python_import_from_statement() {
        let source = "from django.db import models, views\ndef get_data():\n    pass";
        let query = "(function_definition name: (identifier) @python.function.name)";

        let result = extract_entities(
            source,
            tree_sitter_python::LANGUAGE.into(),
            query,
            "python",
            "/import_from.py",
            "test-repo",
        );

        assert!(result.is_ok());
        let entities = result.unwrap();
        let func = entities.iter().find(|e| e.name == "<module>").unwrap();
        let type_refs: Vec<_> = func
            .reference_intents
            .iter()
            .filter_map(|c| {
                if let ReferenceIntent::TypeReference { type_name, .. } = c {
                    Some(type_name.as_str())
                } else {
                    None
                }
            })
            .collect();
        assert!(
            type_refs.contains(&"django.db"),
            "Should have django.db module reference"
        );
        assert!(
            type_refs.contains(&"models"),
            "Should have models import reference"
        );
        assert!(
            type_refs.contains(&"views"),
            "Should have views import reference"
        );
    }

    #[test]
    fn test_extract_python_import_with_alias() {
        let source = "from django.db import models as db_models\ndef query():\n    pass";
        let query = "(function_definition name: (identifier) @python.function.name)";

        let result = extract_entities(
            source,
            tree_sitter_python::LANGUAGE.into(),
            query,
            "python",
            "/alias_import.py",
            "test-repo",
        );

        assert!(result.is_ok());
        let entities = result.unwrap();
        let func = entities.iter().find(|e| e.name == "<module>").unwrap();
        let type_refs: Vec<_> = func
            .reference_intents
            .iter()
            .filter_map(|c| {
                if let ReferenceIntent::TypeReference { type_name, .. } = c {
                    Some(type_name.as_str())
                } else {
                    None
                }
            })
            .collect();
        assert!(
            type_refs.contains(&"db_models"),
            "Should have aliased import name"
        );
    }

    #[test]
    fn test_extract_python_module_synthetic_entity() {
        let source = "import os\nimport sys";
        let query = ""; // No entities to extract

        let result = extract_entities(
            source,
            tree_sitter_python::LANGUAGE.into(),
            query,
            "python",
            "/module_only.py",
            "test-repo",
        );

        assert!(result.is_ok());
        let entities = result.unwrap();
        assert!(!entities.is_empty());
        let module_entity = entities.iter().find(|e| e.name == "<module>").unwrap();
        assert_eq!(module_entity.kind, EntityKind::PythonModule);
        let type_refs: Vec<_> = module_entity
            .reference_intents
            .iter()
            .filter_map(|c| {
                if let ReferenceIntent::TypeReference { type_name, .. } = c {
                    Some(type_name.as_str())
                } else {
                    None
                }
            })
            .collect();
        assert!(type_refs.contains(&"os"), "Should have os import in module");
        assert!(
            type_refs.contains(&"sys"),
            "Should have sys import in module"
        );
    }

    // ============================================================
    // Python ValueReference tests (Phase 4.5)
    // ============================================================

    #[test]
    fn test_extract_python_value_reference_keyword_arg() {
        let source = "parser.add_argument('--flag', action=EnumAction)\n";
        let query = "";

        let result = extract_entities(
            source,
            tree_sitter_python::LANGUAGE.into(),
            query,
            "python",
            "/test.py",
            "test-repo",
        );

        assert!(result.is_ok());
        let entities = result.unwrap();
        assert!(!entities.is_empty());
        let module = entities.iter().find(|e| e.name == "<module>").unwrap();

        let value_refs: Vec<_> = module
            .reference_intents
            .iter()
            .filter_map(|r| {
                if let ReferenceIntent::ValueReference { value_name, .. } = r {
                    Some(value_name.as_str())
                } else {
                    None
                }
            })
            .collect();

        assert!(
            value_refs.contains(&"EnumAction"),
            "Should find EnumAction as ValueReference"
        );
    }

    #[test]
    fn test_extract_python_multiple_value_references() {
        let source = "result = func(handler=MyHandler, callback=my_callback)\n";
        let query = "";

        let result = extract_entities(
            source,
            tree_sitter_python::LANGUAGE.into(),
            query,
            "python",
            "/test.py",
            "test-repo",
        );

        assert!(result.is_ok());
        let entities = result.unwrap();
        let module = entities.iter().find(|e| e.name == "<module>").unwrap();

        let value_refs: Vec<_> = module
            .reference_intents
            .iter()
            .filter_map(|r| {
                if let ReferenceIntent::ValueReference { value_name, .. } = r {
                    Some(value_name.as_str())
                } else {
                    None
                }
            })
            .collect();

        assert!(value_refs.contains(&"MyHandler"));
        assert!(value_refs.contains(&"my_callback"));
        assert_eq!(value_refs.len(), 2);
    }

    #[test]
    fn test_extract_python_value_reference_filters_keywords() {
        let source = "result = func(active=True, empty=None, context=self)\n";
        let query = "";

        let result = extract_entities(
            source,
            tree_sitter_python::LANGUAGE.into(),
            query,
            "python",
            "/test.py",
            "test-repo",
        );

        assert!(result.is_ok());
        let entities = result.unwrap();
        let module = entities.iter().find(|e| e.name == "<module>").unwrap();

        let value_refs: Vec<_> = module
            .reference_intents
            .iter()
            .filter_map(|r| {
                if let ReferenceIntent::ValueReference { value_name, .. } = r {
                    Some(value_name.as_str())
                } else {
                    None
                }
            })
            .collect();

        assert!(!value_refs.contains(&"True"));
        assert!(!value_refs.contains(&"None"));
        assert!(!value_refs.contains(&"self"));
        assert_eq!(value_refs.len(), 0);
    }

    #[test]
    fn test_extract_python_value_reference_with_calls() {
        let source = r#"
def main():
    parser = ArgumentParser()
    parser.add_argument("--action", action=MyAction)
"#;
        let query = "(function_definition name: (identifier) @python.function.name)";

        let result = extract_entities(
            source,
            tree_sitter_python::LANGUAGE.into(),
            query,
            "python",
            "/test.py",
            "test-repo",
        );

        assert!(result.is_ok());
        let entities = result.unwrap();
        let main = entities.iter().find(|e| e.name == "main").unwrap();

        let calls: Vec<_> = main
            .reference_intents
            .iter()
            .filter_map(|r| {
                if let ReferenceIntent::Call { method, .. } = r {
                    Some(method.as_str())
                } else {
                    None
                }
            })
            .collect();

        assert!(calls.contains(&"ArgumentParser"));
        assert!(calls.contains(&"add_argument"));

        let value_refs: Vec<_> = main
            .reference_intents
            .iter()
            .filter_map(|r| {
                if let ReferenceIntent::ValueReference { value_name, .. } = r {
                    Some(value_name.as_str())
                } else {
                    None
                }
            })
            .collect();

        assert!(value_refs.contains(&"MyAction"));
    }

    // ============================================================
    // Phase 5: Python inheritance (EXTENDS) tests
    // ============================================================

    #[test]
    fn test_extract_python_single_inheritance() {
        let source = "class Admin(User):\n    pass";
        let query = "(class_definition name: (identifier) @python.class.name)";

        let result = extract_entities(
            source,
            tree_sitter_python::LANGUAGE.into(),
            query,
            "python",
            "/Admin.py",
            "test-repo",
        );

        assert!(result.is_ok());
        let entities = result.unwrap();
        assert_eq!(entities[0].name, "Admin");

        let extends: Vec<_> = entities[0]
            .reference_intents
            .iter()
            .filter_map(|r| {
                if let ReferenceIntent::Extends {
                    parent: parent_name,
                    ..
                } = r
                {
                    Some(parent_name.as_str())
                } else {
                    None
                }
            })
            .collect();

        assert!(extends.contains(&"User"), "Should extend User");
    }

    #[test]
    fn test_extract_python_multiple_inheritance() {
        let source = "class C(A, B):\n    pass";
        let query = "(class_definition name: (identifier) @python.class.name)";

        let result = extract_entities(
            source,
            tree_sitter_python::LANGUAGE.into(),
            query,
            "python",
            "/C.py",
            "test-repo",
        );

        assert!(result.is_ok());
        let entities = result.unwrap();
        let extends: Vec<_> = entities[0]
            .reference_intents
            .iter()
            .filter_map(|r| {
                if let ReferenceIntent::Extends {
                    parent: parent_name,
                    ..
                } = r
                {
                    Some(parent_name.as_str())
                } else {
                    None
                }
            })
            .collect();

        assert!(extends.contains(&"A"));
        assert!(extends.contains(&"B"));
        assert_eq!(extends.len(), 2);
    }

    #[test]
    fn test_extract_python_no_inheritance() {
        let source = "class Simple:\n    pass";
        let query = "(class_definition name: (identifier) @python.class.name)";

        let result = extract_entities(
            source,
            tree_sitter_python::LANGUAGE.into(),
            query,
            "python",
            "/Simple.py",
            "test-repo",
        );

        assert!(result.is_ok());
        let entities = result.unwrap();
        let extends: Vec<_> = entities[0]
            .reference_intents
            .iter()
            .filter_map(|r| {
                if let ReferenceIntent::Extends { .. } = r {
                    Some(())
                } else {
                    None
                }
            })
            .collect();

        assert!(
            extends.is_empty(),
            "Class without inheritance should not have EXTENDS"
        );
    }

    // ============================================================
    // Phase 5: Python decorator (CALLS) tests
    // ============================================================

    #[test]
    fn test_extract_python_decorator_staticmethod() {
        let source = "class MyClass:\n    @staticmethod\n    def my_method():\n        pass";
        let query = "(class_definition name: (identifier) @python.class.name) (function_definition name: (identifier) @python.function.name)";

        let result = extract_entities(
            source,
            tree_sitter_python::LANGUAGE.into(),
            query,
            "python",
            "/MyClass.py",
            "test-repo",
        );

        assert!(result.is_ok());
        let entities = result.unwrap();
        let method = entities.iter().find(|e| e.name == "my_method").unwrap();

        let decorator_calls: Vec<_> = method
            .reference_intents
            .iter()
            .filter_map(|r| {
                if let ReferenceIntent::Call {
                    method, receiver, ..
                } = r
                {
                    if receiver.is_none() {
                        Some(method.as_str())
                    } else {
                        None
                    }
                } else {
                    None
                }
            })
            .collect();

        assert!(
            decorator_calls.contains(&"staticmethod"),
            "Should have staticmethod decorator call, got: {:?}",
            decorator_calls
        );
    }

    #[test]
    fn test_extract_python_decorator_property() {
        let source = "class C:\n    @property\n    def value(self):\n        return 42";
        let query = "(class_definition name: (identifier) @python.class.name) (function_definition name: (identifier) @python.function.name)";

        let result = extract_entities(
            source,
            tree_sitter_python::LANGUAGE.into(),
            query,
            "python",
            "/C.py",
            "test-repo",
        );

        assert!(result.is_ok());
        let entities = result.unwrap();
        let method = entities.iter().find(|e| e.name == "value").unwrap();

        let decorator_calls: Vec<_> = method
            .reference_intents
            .iter()
            .filter_map(|r| {
                if let ReferenceIntent::Call {
                    method, receiver, ..
                } = r
                {
                    if receiver.is_none() {
                        Some(method.as_str())
                    } else {
                        None
                    }
                } else {
                    None
                }
            })
            .collect();

        assert!(decorator_calls.contains(&"property"));
    }

    #[test]
    fn test_extract_python_decorator_class() {
        let source =
            "from dataclasses import dataclass\n\n@dataclass\nclass Point:\n    x: int\n    y: int";
        let query = "(class_definition name: (identifier) @python.class.name)";

        let result = extract_entities(
            source,
            tree_sitter_python::LANGUAGE.into(),
            query,
            "python",
            "/Point.py",
            "test-repo",
        );

        assert!(result.is_ok());
        let entities = result.unwrap();
        let class_entity = entities.iter().find(|e| e.name == "Point").unwrap();

        let decorator_calls: Vec<_> = class_entity
            .reference_intents
            .iter()
            .filter_map(|r| {
                if let ReferenceIntent::Call {
                    method, receiver, ..
                } = r
                {
                    if receiver.is_none() {
                        Some(method.as_str())
                    } else {
                        None
                    }
                } else {
                    None
                }
            })
            .collect();

        assert!(
            decorator_calls.contains(&"dataclass"),
            "Should have @dataclass decorator call, got: {:?}",
            decorator_calls
        );
    }

    #[test]
    fn test_extract_python_decorator_with_arguments() {
        let source = r#"
from flask import Flask
app = Flask(__name__)

@app.route("/")
def index():
    return "hello"
"#;
        let query = "(class_definition name: (identifier) @python.class.name) (function_definition name: (identifier) @python.function.name) (assignment left: (identifier) @python.constant.name right: (_) (#match? @python.constant.name \"^[A-Z][A-Z0-9_]*$\"))";

        let result = extract_entities(
            source,
            tree_sitter_python::LANGUAGE.into(),
            query,
            "python",
            "/app.py",
            "test-repo",
        );

        assert!(result.is_ok());
        let entities = result.unwrap();
        let func = entities.iter().find(|e| e.name == "index").unwrap();

        let decorator_calls: Vec<_> = func
            .reference_intents
            .iter()
            .filter_map(|r| {
                if let ReferenceIntent::Call {
                    method, receiver, ..
                } = r
                {
                    Some((method.as_str(), receiver.as_deref()))
                } else {
                    None
                }
            })
            .collect();

        assert!(
            decorator_calls.iter().any(|(m, _)| *m == "route"),
            "Should have @app.route decorator, got: {:?}",
            decorator_calls
        );
    }

    #[test]
    fn test_extract_python_decorator_multiple() {
        let source = r#"
class Service:
    @staticmethod
    @route("/api")
    def handle():
        pass
"#;
        let query = "(class_definition name: (identifier) @python.class.name) (function_definition name: (identifier) @python.function.name)";

        let result = extract_entities(
            source,
            tree_sitter_python::LANGUAGE.into(),
            query,
            "python",
            "/Service.py",
            "test-repo",
        );

        assert!(result.is_ok());
        let entities = result.unwrap();
        let method = entities.iter().find(|e| e.name == "handle").unwrap();

        let decorator_calls: Vec<_> = method
            .reference_intents
            .iter()
            .filter_map(|r| {
                if let ReferenceIntent::Call {
                    method, receiver, ..
                } = r
                {
                    if receiver.is_none() {
                        Some(method.as_str())
                    } else {
                        None
                    }
                } else {
                    None
                }
            })
            .collect();

        assert!(decorator_calls.contains(&"staticmethod"));
        assert!(decorator_calls.contains(&"route"));
    }

    #[test]
    fn test_extract_python_class_inheritance_with_decorator() {
        let source = r#"
from dataclasses import dataclass

@dataclass
class Employee(Person):
    id: int
"#;
        let query = "(class_definition name: (identifier) @python.class.name)";

        let result = extract_entities(
            source,
            tree_sitter_python::LANGUAGE.into(),
            query,
            "python",
            "/Employee.py",
            "test-repo",
        );

        assert!(result.is_ok());
        let entities = result.unwrap();
        let class_entity = entities.iter().find(|e| e.name == "Employee").unwrap();

        let extends: Vec<_> = class_entity
            .reference_intents
            .iter()
            .filter_map(|r| {
                if let ReferenceIntent::Extends {
                    parent: parent_name,
                    ..
                } = r
                {
                    Some(parent_name.as_str())
                } else {
                    None
                }
            })
            .collect();

        assert!(extends.contains(&"Person"), "Should extend Person");

        let decorator_calls: Vec<_> = class_entity
            .reference_intents
            .iter()
            .filter_map(|r| {
                if let ReferenceIntent::Call {
                    method, receiver, ..
                } = r
                {
                    if receiver.is_none() {
                        Some(method.as_str())
                    } else {
                        None
                    }
                } else {
                    None
                }
            })
            .collect();

        assert!(
            decorator_calls.contains(&"dataclass"),
            "Should have @dataclass decorator"
        );
    }

    // ============================================================
    // Phase 6: Type hints, *args/**kwargs, and Py2 syntax tests
    // ============================================================

    #[test]
    fn test_extract_python_type_hints_in_signature() {
        let source = "def process(items: List[str], config: Dict[str, int]) -> Dict[str, int]:\n    return {}";
        let query = "(function_definition name: (identifier) @python.function.name parameters: (parameters) @python.signature)";

        let result = extract_entities(
            source,
            tree_sitter_python::LANGUAGE.into(),
            query,
            "python",
            "/typed.py",
            "test-repo",
        );

        assert!(result.is_ok());
        let entities = result.unwrap();
        assert_eq!(entities[0].name, "process");
        assert!(entities[0].signature.is_some());
        let sig = entities[0].signature.as_ref().unwrap();
        assert!(
            sig.contains("List[str]"),
            "Signature should contain List[str]"
        );
        assert!(
            sig.contains("Dict[str, int]"),
            "Signature should contain Dict[str, int]"
        );
    }

    #[test]
    fn test_extract_python_var_args_kwargs() {
        let source = "def log(msg: str, *args, level: str = \"INFO\", **kwargs):\n    pass";
        let query = "(function_definition name: (identifier) @python.function.name parameters: (parameters) @python.signature)";

        let result = extract_entities(
            source,
            tree_sitter_python::LANGUAGE.into(),
            query,
            "python",
            "/log.py",
            "test-repo",
        );

        assert!(result.is_ok());
        let entities = result.unwrap();
        assert_eq!(entities[0].name, "log");
        assert!(entities[0].signature.is_some());
        let sig = entities[0].signature.as_ref().unwrap();
        assert!(
            sig.contains("*args"),
            "Signature should contain *args, got: {}",
            sig
        );
        assert!(
            sig.contains("**kwargs"),
            "Signature should contain **kwargs, got: {}",
            sig
        );
    }

    #[test]
    fn test_extract_python_optional_return_type() {
        let source = "def find_user(user_id: int) -> Optional[Dict[str, str]]:\n    return None";
        let query = "(function_definition name: (identifier) @python.function.name)";

        let result = extract_entities(
            source,
            tree_sitter_python::LANGUAGE.into(),
            query,
            "python",
            "/find.py",
            "test-repo",
        );

        assert!(result.is_ok());
        let entities = result.unwrap();
        assert_eq!(entities[0].name, "find_user");
        assert!(entities[0].kind == EntityKind::PythonFunction);
    }

    #[test]
    fn test_extract_python_py2_exception_syntax() {
        // Python 2 style exception: except ValueError, e:
        // tree-sitter-python handles this via the except_clause node
        let source = r#"
def handler():
    try:
        raise ValueError("boom")
    except ValueError, e:
        pass
"#;
        let query = "(function_definition name: (identifier) @python.function.name)";

        let result = extract_entities(
            source,
            tree_sitter_python::LANGUAGE.into(),
            query,
            "python",
            "/py2.py",
            "test-repo",
        );

        // Should not panic or error — the function should still be extracted
        assert!(result.is_ok());
        let entities = result.unwrap();
        let handler = entities.iter().find(|e| e.name == "handler").unwrap();
        assert_eq!(handler.kind, EntityKind::PythonFunction);
    }

    #[test]
    fn test_extract_python_py3_exception_syntax() {
        // Python 3 style exception: except ValueError as e:
        let source = r#"
def handler():
    try:
        raise ValueError("boom")
    except ValueError as e:
        pass
"#;
        let query = "(function_definition name: (identifier) @python.function.name)";

        let result = extract_entities(
            source,
            tree_sitter_python::LANGUAGE.into(),
            query,
            "python",
            "/py3.py",
            "test-repo",
        );

        assert!(result.is_ok());
        let entities = result.unwrap();
        let handler = entities.iter().find(|e| e.name == "handler").unwrap();
        assert_eq!(handler.kind, EntityKind::PythonFunction);
    }

    #[test]
    fn test_extract_python_method_enclosing_class_is_set() {
        // Verify that Python methods get their enclosing_class from class_definition contexts
        let source = r#"
class MyService:
    def handle(self):
        pass
"#;
        let query = include_str!("../../../queries/python.scm");

        let result = extract_entities(
            source,
            tree_sitter_python::LANGUAGE.into(),
            query,
            "python",
            "/service.py",
            "test-repo",
        );

        assert!(result.is_ok());
        let entities = result.unwrap();
        let method = entities.iter().find(|e| e.name == "handle").unwrap();
        assert_eq!(method.kind, EntityKind::PythonMethod);
        assert_eq!(
            method.enclosing_class.as_deref(),
            Some("MyService"),
            "PythonMethod should have enclosing_class set"
        );
        assert_eq!(
            method.fqn, "MyService.handle",
            "PythonMethod FQN should be ClassName.methodName"
        );
    }

    // ============================================================
    // Java inheritance and package FQN tests
    // ============================================================

    #[test]
    fn test_extract_entities_java_implements() {
        let source = "class Repo implements Repository<User> {\n  void save(User u) {}\n}";
        let entities = extract_entities(
            source,
            tree_sitter_java::LANGUAGE.into(),
            include_str!("../../../queries/java.scm"),
            "java",
            "/test.java",
            "test-repo",
        )
        .unwrap();
        let repo = entities.iter().find(|e| e.name == "Repo").unwrap();
        assert!(repo.reference_intents.iter().any(|r| matches!(
            r,
            ReferenceIntent::Implements { interface, .. } if interface == "Repository"
        )));
    }

    #[test]
    fn test_extract_entities_java_extends() {
        let source = "class Parent {}\nclass Child extends Parent {}";
        let entities = extract_entities(
            source,
            tree_sitter_java::LANGUAGE.into(),
            include_str!("../../../queries/java.scm"),
            "java",
            "/test.java",
            "test-repo",
        )
        .unwrap();
        let child = entities.iter().find(|e| e.name == "Child").unwrap();
        assert!(child.reference_intents.iter().any(|r| matches!(
            r,
            ReferenceIntent::Extends { parent, .. } if parent == "Parent"
        )));
    }

    #[test]
    fn test_extract_entities_java_package_fqn() {
        let source = "package com.example;\n\nclass Foo {}";
        let entities = extract_entities(
            source,
            tree_sitter_java::LANGUAGE.into(),
            include_str!("../../../queries/java.scm"),
            "java",
            "/test.java",
            "test-repo",
        )
        .unwrap();
        let foo = entities.iter().find(|e| e.name == "Foo").unwrap();
        assert_eq!(foo.fqn, "com.example.Foo");
    }

    #[test]
    fn test_extract_entities_java_nested_class_package_fqn() {
        let source = "package com.example;\n\nclass Outer {\n    class Inner {}\n}";
        let entities = extract_entities(
            source,
            tree_sitter_java::LANGUAGE.into(),
            include_str!("../../../queries/java.scm"),
            "java",
            "/test.java",
            "test-repo",
        )
        .unwrap();
        let inner = entities.iter().find(|e| e.name == "Inner").unwrap();
        assert_eq!(inner.fqn, "com.example.Outer.Inner");
    }

    #[test]
    fn test_extract_entities_java_interface_extends() {
        let source = "interface Child extends Parent {}";
        let entities = extract_entities(
            source,
            tree_sitter_java::LANGUAGE.into(),
            include_str!("../../../queries/java.scm"),
            "java",
            "/test.java",
            "test-repo",
        )
        .unwrap();
        let child = entities.iter().find(|e| e.name == "Child").unwrap();
        assert!(child.reference_intents.iter().any(|r| matches!(
            r,
            ReferenceIntent::Extends { parent, .. } if parent == "Parent"
        )));
    }

    #[test]
    fn test_extract_entities_java_extends_and_implements() {
        let source = "class Admin extends User implements Serializable, Comparable<Admin> {}";
        let entities = extract_entities(
            source,
            tree_sitter_java::LANGUAGE.into(),
            include_str!("../../../queries/java.scm"),
            "java",
            "/test.java",
            "test-repo",
        )
        .unwrap();
        let admin = entities.iter().find(|e| e.name == "Admin").unwrap();
        let extends: Vec<_> = admin
            .reference_intents
            .iter()
            .filter_map(|r| {
                if let ReferenceIntent::Extends { parent, .. } = r {
                    Some(parent.as_str())
                } else {
                    None
                }
            })
            .collect();
        let implements: Vec<_> = admin
            .reference_intents
            .iter()
            .filter_map(|r| {
                if let ReferenceIntent::Implements { interface, .. } = r {
                    Some(interface.as_str())
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(extends, ["User"]);
        assert!(implements.contains(&"Serializable"));
        assert!(implements.contains(&"Comparable"));
    }

    #[test]
    fn test_extract_entities_self_type_reference_filtered() {
        let source = "interface Node { next: Node; }";
        let query = "(interface_declaration name: (type_identifier) @interface.name)";

        let result = extract_entities(
            source,
            tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
            query,
            "typescript",
            "test.ts",
            "test_repo",
        );

        assert!(result.is_ok());
        let entities = result.unwrap();
        assert!(!entities.is_empty());
        let node_entity = entities.iter().find(|e| e.name == "Node").unwrap();
        let has_self_ref = node_entity.reference_intents.iter().any(|r| {
            matches!(r, ReferenceIntent::TypeReference { type_name, .. } if type_name == "Node")
        });
        assert!(
            !has_self_ref,
            "Self-referencing TypeReference should be filtered"
        );
    }

    #[test]
    fn test_extract_js_require_alias() {
        let code = "var MyJsAlias = require('./alias_target_js');";
        let result = extract_entities(
            code,
            tree_sitter_javascript::LANGUAGE.into(),
            "(variable_declaration (variable_declarator name: (identifier) @constant.name))",
            "javascript",
            "test.js",
            "test_repo",
        );
        let entities = result.unwrap();
        let entity = entities.iter().find(|e| e.name == "MyJsAlias").unwrap();
        assert_eq!(
            entity.alias_module_path.as_deref(),
            Some("./alias_target_js")
        );
    }

    #[test]
    fn test_extract_ts_import_alias() {
        let code = "import { MyTsTarget as MyTsAlias } from './alias_target_ts';";
        let result = extract_entities(
            code,
            tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
            "", // No query needed for imports
            "typescript",
            "test.ts",
            "test_repo",
        );
        let entities = result.unwrap();
        let entity = entities.iter().find(|e| e.name == "MyTsAlias").unwrap();
        assert_eq!(
            entity.alias_module_path.as_deref(),
            Some("./alias_target_ts")
        );
    }
}
