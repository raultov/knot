use crate::models::{EntityKind, ReferenceIntent};
use crate::pipeline::parser::languages::{
    cpp, css, groovy, html, java, javascript, kotlin, python, rust, typescript,
};
use crate::pipeline::parser::utils::*;
use tree_sitter::Node;

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

#[derive(Default)]
pub(crate) struct CaptureState<'a> {
    pub name: Option<String>,
    pub kind: Option<EntityKind>,
    pub signature: Option<String>,
    pub start_line: usize,
    pub entity_node: Option<Node<'a>>,
    pub reference_intents: Vec<ReferenceIntent>,
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn process_capture<'a>(
    cap_name: &str,
    text: String,
    node: Node<'a>,
    source_bytes: &[u8],
    lang_name: &str,
    state: &mut CaptureState<'a>,
) {
    let mut name = state.name.take();
    let mut kind = state.kind.take();
    let mut signature = state.signature.take();
    let mut start_line = state.start_line;
    let mut entity_node = state.entity_node.take();
    let mut reference_intents = std::mem::take(&mut state.reference_intents);

    match cap_name {
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
            if name_or_intent.starts_with("css.") || name_or_intent.starts_with("scss.") =>
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
            if let Some((entity_name, entity_kind, entity_line)) =
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
                    EntityKind::RustTypeAlias | EntityKind::RustConstant | EntityKind::RustStatic
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
            if name_or_intent.starts_with("dom.") || name_or_intent.starts_with("css.class_") =>
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
                    n if n.contains("class") => find_parent_by_kind(node, "class_declaration"),
                    n if n.contains("interface") => {
                        find_parent_by_kind(node, "interface_declaration")
                    }
                    n if n.contains("enum") => find_parent_by_kind(node, "enum_declaration"),
                    n if n.contains("method") => find_parent_by_kind(node, "method_declaration")
                        .or_else(|| find_parent_by_kind(node, "constructor_declaration")),
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
        | "css.className" | "css.method" | "css.keyframe" | "script_src" | "stylesheet_href" => {
            // These captures are either metadata or handled in other passes
        }
        _ => {}
    }

    state.name = name;
    state.kind = kind;
    state.signature = signature;
    state.start_line = start_line;
    state.entity_node = entity_node;
    state.reference_intents = reference_intents;
}
