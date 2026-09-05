use crate::models::{EntityKind, ReferenceIntent};
use crate::pipeline::parser::languages::{
    cpp, csharp, css, groovy, html, java, javascript, kotlin, markdown, python, rust, typescript,
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

/// Immutable context shared by every capture handler: the tree-sitter node
/// that produced the capture, the file bytes, and the language being parsed.
struct CaptureCtx<'a> {
    node: Node<'a>,
    source_bytes: &'a [u8],
    lang_name: &'a str,
}

/// Dispatch call-reference extraction to the language handler shared by the
/// `method.name`, `function.name`, and `constant.name` capture arms.
/// The fallback is TypeScript: those captures only occur in TS/JS/Kotlin
/// grammars (Java uses `method.name`), so anything reaching the fallback is
/// TypeScript.
fn extract_call_reference_intents(
    node: Node<'_>,
    source_bytes: &[u8],
    lang_name: &str,
    reference_intents: &mut Vec<ReferenceIntent>,
) {
    match lang_name {
        "java" => java::extract_reference_intents_java(node, source_bytes, reference_intents),
        "javascript" => {
            javascript::extract_reference_intents_javascript(node, source_bytes, reference_intents);
        }
        "kotlin" => {
            kotlin::extract_reference_intents_kotlin(node, source_bytes, reference_intents);
        }
        _ => {
            typescript::extract_reference_intents_typescript(node, source_bytes, reference_intents);
        }
    }
}

/// Dispatches a single tree-sitter capture to the handler for its capture
/// family. Guard arms are order-sensitive: prefix families (`css.`, `dom.`,
/// ...) must be tested before the trailing ignore-list, which names captures
/// that some of those prefixes also match.
#[expect(
    clippy::too_many_arguments,
    reason = "capture processing requires context from tree-sitter, language, and queries"
)]
pub(crate) fn process_capture<'a>(
    cap_name: &str,
    text: String,
    node: Node<'a>,
    source_bytes: &'a [u8],
    lang_name: &'a str,
    state: &mut CaptureState<'a>,
) {
    let ctx = CaptureCtx {
        node,
        source_bytes,
        lang_name,
    };

    match cap_name {
        "class.name" | "kotlin_class.name" | "interface.name" | "kotlin_interface.name" => {
            handle_class_captures(cap_name, &text, &ctx, state);
        }
        "method.name" | "kotlin_method.name" => {
            handle_method_captures(cap_name, &text, &ctx, state);
        }
        "kotlin_object.name" | "kotlin_companion.name" | "kotlin_property.name" => {
            handle_kotlin_decl_captures(cap_name, &text, &ctx, state);
        }
        "kotlin_function.name" => handle_kotlin_function_capture(&text, &ctx, state),
        "function.name" | "constant.name" => {
            handle_function_captures(cap_name, &text, &ctx, state);
        }
        "enum.name" => {
            state.name = Some(text);
            state.kind = Some(EntityKind::Enum);
            state.start_line = node.start_position().row + 1;
            state.entity_node = find_parent_by_kind(node, "enum_declaration");
        }
        "signature" | "python.signature" => state.signature = Some(text),
        "type.reference" => {
            // Type annotations in signatures, variables, etc.
            state
                .reference_intents
                .push(ReferenceIntent::TypeReference {
                    type_name: text,
                    line: node.start_position().row + 1,
                });
        }
        // CSS/SCSS: Delegate to specialized handler
        name_or_intent
            if name_or_intent.starts_with("css.") || name_or_intent.starts_with("scss.") =>
        {
            handle_css_scss_capture(name_or_intent, &text, &ctx, state);
        }
        // HTML: Delegate to specialized handler
        name_or_intent if name_or_intent.starts_with("html_") => {
            handle_html_capture(name_or_intent, &text, &ctx, state);
        }
        // Rust: Handle Rust entity captures
        name_or_intent if name_or_intent.starts_with("rust.") => {
            handle_rust_capture(name_or_intent, &text, &ctx, state);
        }
        // Python: Handle Python entity captures
        name_or_intent if name_or_intent.starts_with("python.") => {
            handle_python_capture(name_or_intent, &text, &ctx, state);
        }
        // Markdown: Handle Markdown entity captures
        name_or_intent if name_or_intent.starts_with("markdown.") => {
            handle_markdown_capture(name_or_intent, &text, &ctx, state);
        }
        // DOM/CSS references: Delegate to JavaScript handler
        name_or_intent
            if name_or_intent.starts_with("dom.") || name_or_intent.starts_with("css.class_") =>
        {
            handle_dom_css_ref_capture(name_or_intent, &text, &ctx, state);
        }
        // Groovy: Handle Groovy entity captures (JVM-shared grammar with Java)
        name_or_intent if name_or_intent.starts_with("groovy.") => {
            handle_groovy_capture(name_or_intent, &text, &ctx, state);
        }
        // C#: Handle C# entity captures (grammar-gap handling in the module)
        name_or_intent if name_or_intent.starts_with("csharp.") => {
            handle_csharp_capture(name_or_intent, &text, &ctx, state);
        }
        // C/C++ Entities
        name_or_intent
            if name_or_intent.starts_with("cpp_")
                || name_or_intent.starts_with("c_")
                || name_or_intent.starts_with("preproc.") =>
        {
            handle_cpp_captures(name_or_intent, &text, &ctx, state);
        }
        // Ignore unhandled captures
        "dom.receiver" | "dom.action" | "dom.method" | "css.receiver" | "css.classList"
        | "css.className" | "css.method" | "css.keyframe" | "script_src" | "stylesheet_href" => {
            // These captures are either metadata or handled in other passes
        }
        _ => {}
    }
}

/// Class and interface name captures shared by Java, TS/JS, and Kotlin
/// grammars. Kotlin reuses `class_declaration` for classes, interfaces, and
/// enums, so its kind is re-detected from the source keywords.
fn handle_class_captures<'a>(
    cap_name: &str,
    text: &str,
    ctx: &CaptureCtx<'a>,
    state: &mut CaptureState<'a>,
) {
    match cap_name {
        "class.name" => {
            state.name = Some(text.to_string());
            state.kind = Some(EntityKind::Class);
            state.start_line = ctx.node.start_position().row + 1;
            state.entity_node = find_parent_by_kind(ctx.node, "class_declaration")
                .or_else(|| find_parent_by_kind(ctx.node, "abstract_class_declaration"));
        }
        "kotlin_class.name" => {
            state.name = Some(text.to_string());
            state.start_line = ctx.node.start_position().row + 1;
            let entity_node = find_parent_by_kind(ctx.node, "class_declaration");
            state.kind = entity_node.map(|n| detect_kotlin_class_kind(n, ctx.source_bytes));
            state.entity_node = entity_node;
        }
        "interface.name" => {
            state.name = Some(text.to_string());
            state.kind = Some(EntityKind::Interface);
            state.start_line = ctx.node.start_position().row + 1;
            state.entity_node = find_parent_by_kind(ctx.node, "interface_declaration");
        }
        "kotlin_interface.name" => {
            state.name = Some(text.to_string());
            state.kind = Some(EntityKind::KotlinInterface);
            state.start_line = ctx.node.start_position().row + 1;
            state.entity_node = find_parent_by_kind(ctx.node, "interface_declaration");
        }
        _ => {}
    }
}

/// Method name captures. `method.name` is shared by Java/TS/JS grammars and
/// pulls call and type-reference intents from the method body;
/// `kotlin_method.name` is the Kotlin-specific variant.
fn handle_method_captures<'a>(
    cap_name: &str,
    text: &str,
    ctx: &CaptureCtx<'a>,
    state: &mut CaptureState<'a>,
) {
    match cap_name {
        "method.name" => {
            state.name = Some(text.to_string());
            state.kind = Some(EntityKind::Method);
            state.start_line = ctx.node.start_position().row + 1;
            state.entity_node = find_parent_by_kind(ctx.node, "method_declaration")
                .or_else(|| find_parent_by_kind(ctx.node, "method_definition"))
                .or_else(|| find_parent_by_kind(ctx.node, "method_signature"))
                .or_else(|| find_parent_by_kind(ctx.node, "abstract_method_signature"));
            // For methods, extract reference intents from the method body
            if let Some(method_node) = state.entity_node {
                extract_call_reference_intents(
                    method_node,
                    ctx.source_bytes,
                    ctx.lang_name,
                    &mut state.reference_intents,
                );

                // Extract type references from method signatures (parameters, return types)
                if ctx.lang_name == "java" {
                    extract_type_references(
                        method_node,
                        ctx.source_bytes,
                        &mut state.reference_intents,
                    );
                } else if ctx.lang_name == "kotlin" {
                    kotlin::extract_type_references(
                        method_node,
                        ctx.source_bytes,
                        &mut state.reference_intents,
                    );
                }
            }
        }
        "kotlin_method.name" => {
            state.name = Some(text.to_string());
            state.kind = Some(EntityKind::KotlinMethod);
            state.start_line = ctx.node.start_position().row + 1;
            state.entity_node = find_parent_by_kind(ctx.node, "function_declaration");
            // For Kotlin methods, extract reference intents from the method body
            if let Some(method_node) = state.entity_node
                && ctx.lang_name == "kotlin"
            {
                kotlin::extract_reference_intents_kotlin(
                    method_node,
                    ctx.source_bytes,
                    &mut state.reference_intents,
                );
            }
        }
        _ => {}
    }
}

/// Simple Kotlin declaration captures: objects, companion objects, and
/// properties. Only properties pull reference intents.
fn handle_kotlin_decl_captures<'a>(
    cap_name: &str,
    text: &str,
    ctx: &CaptureCtx<'a>,
    state: &mut CaptureState<'a>,
) {
    match cap_name {
        "kotlin_object.name" => {
            state.name = Some(text.to_string());
            state.kind = Some(EntityKind::KotlinObject);
            state.start_line = ctx.node.start_position().row + 1;
            state.entity_node = find_parent_by_kind(ctx.node, "object_declaration");
        }
        "kotlin_companion.name" => {
            state.name = Some(text.to_string());
            state.kind = Some(EntityKind::KotlinCompanionObject);
            state.start_line = ctx.node.start_position().row + 1;
            state.entity_node = find_parent_by_kind(ctx.node, "companion_object");
        }
        "kotlin_property.name" => {
            state.name = Some(text.to_string());
            state.kind = Some(EntityKind::KotlinProperty);
            state.start_line = ctx.node.start_position().row + 1;
            state.entity_node = find_parent_by_kind(ctx.node, "property_declaration");
            // For Kotlin properties, extract reference intents from the property
            if let Some(prop_node) = state.entity_node
                && ctx.lang_name == "kotlin"
            {
                kotlin::extract_reference_intents_kotlin(
                    prop_node,
                    ctx.source_bytes,
                    &mut state.reference_intents,
                );
            }
        }
        _ => {}
    }
}

/// `kotlin_function.name` capture. A Kotlin `function_declaration` enclosed
/// in a class/object/companion/interface is really a method; outside them it
/// is a free function.
fn handle_kotlin_function_capture<'a>(
    text: &str,
    ctx: &CaptureCtx<'a>,
    state: &mut CaptureState<'a>,
) {
    state.name = Some(text.to_string());
    state.start_line = ctx.node.start_position().row + 1;
    state.entity_node = find_parent_by_kind(ctx.node, "function_declaration");

    // Determine if this function is actually a method (enclosed in a
    // class_declaration, object_declaration, companion_object, or interface_declaration).
    let is_method = state.entity_node.is_some_and(|n| {
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

    state.kind = Some(if is_method {
        EntityKind::KotlinMethod
    } else {
        EntityKind::KotlinFunction
    });

    // For Kotlin methods/functions, extract reference intents from the function body
    if let Some(func_node) = state.entity_node
        && ctx.lang_name == "kotlin"
    {
        kotlin::extract_reference_intents_kotlin(
            func_node,
            ctx.source_bytes,
            &mut state.reference_intents,
        );

        // Extract type references from method signatures (parameters, return types)
        // for functions that are really methods inside a class/object
        if is_method {
            kotlin::extract_type_references(
                func_node,
                ctx.source_bytes,
                &mut state.reference_intents,
            );
        }
    }
}

/// Free-function and constant captures shared by JS/TS grammars. Both pull
/// reference intents from the declaration/initializer node.
fn handle_function_captures<'a>(
    cap_name: &str,
    text: &str,
    ctx: &CaptureCtx<'a>,
    state: &mut CaptureState<'a>,
) {
    match cap_name {
        "function.name" => {
            state.name = Some(text.to_string());
            state.kind = Some(EntityKind::Function);
            state.start_line = ctx.node.start_position().row + 1;
            state.entity_node = find_parent_by_kind(ctx.node, "function_declaration")
                .or_else(|| find_parent_by_kind(ctx.node, "lexical_declaration"))
                .or_else(|| find_parent_by_kind(ctx.node, "variable_declaration"))
                .or_else(|| find_parent_by_kind(ctx.node, "export_statement"));
            // For functions, extract reference intents from the function body
            if let Some(func_node) = state.entity_node {
                extract_call_reference_intents(
                    func_node,
                    ctx.source_bytes,
                    ctx.lang_name,
                    &mut state.reference_intents,
                );
            }
        }
        "constant.name" => {
            state.name = Some(text.to_string());
            state.kind = Some(EntityKind::Constant);
            state.start_line = ctx.node.start_position().row + 1;
            state.entity_node = find_parent_by_kind(ctx.node, "lexical_declaration")
                .or_else(|| find_parent_by_kind(ctx.node, "variable_declarator"))
                .or_else(|| find_parent_by_kind(ctx.node, "field_declaration"))
                .or_else(|| find_parent_by_kind(ctx.node, "public_field_definition"))
                .or_else(|| find_parent_by_kind(ctx.node, "field_definition"));

            // Extract reference intents from constant initializers
            // This captures function calls inside const assignments like:
            //   const formattedItems = formatRegistryItems(registryItems)
            //   const config = await getMcpConfig(process.cwd())
            //   val result = someFunction()
            if let Some(const_node) = state.entity_node {
                extract_call_reference_intents(
                    const_node,
                    ctx.source_bytes,
                    ctx.lang_name,
                    &mut state.reference_intents,
                );
            }
        }
        _ => {}
    }
}

/// CSS/SCSS capture: delegates to the CSS handler and promotes its result to
/// an entity capture on the same node.
fn handle_css_scss_capture<'a>(
    name_or_intent: &str,
    text: &str,
    ctx: &CaptureCtx<'a>,
    state: &mut CaptureState<'a>,
) {
    if let Some((entity_name, entity_kind, entity_line)) =
        css::handle_css_capture(name_or_intent, text, ctx.node)
    {
        state.name = Some(entity_name);
        state.kind = Some(entity_kind);
        state.start_line = entity_line;
        state.entity_node = Some(ctx.node);
    }
}

/// HTML capture: delegates to the HTML handler and promotes its result to an
/// entity capture on the same node.
fn handle_html_capture<'a>(
    name_or_intent: &str,
    text: &str,
    ctx: &CaptureCtx<'a>,
    state: &mut CaptureState<'a>,
) {
    if let Some((entity_name, entity_kind, entity_line)) =
        html::handle_html_capture(name_or_intent, text, ctx.node)
    {
        state.name = Some(entity_name);
        state.kind = Some(entity_kind);
        state.start_line = entity_line;
        state.entity_node = Some(ctx.node);
    }
}

/// Rust entity capture (`rust.*`). Entity kinds that carry preceding doc
/// comments on the parent item re-anchor the entity node to that parent.
fn handle_rust_capture<'a>(
    name_or_intent: &str,
    text: &str,
    ctx: &CaptureCtx<'a>,
    state: &mut CaptureState<'a>,
) {
    if let Some((entity_name, entity_kind, entity_line)) =
        rust::handle_rust_capture(name_or_intent, text, ctx.node)
    {
        let rust_kind = entity_kind.clone();
        state.name = Some(entity_name);
        state.kind = Some(entity_kind);
        state.start_line = entity_line;

        // For Rust type aliases, constants, and statics, the captured node is the
        // identifier (type_identifier, identifier, identifier), but comments are
        // preceding siblings of the parent entity node (type_item, const_item, static_item).
        // Get the parent to properly extract preceding comments.
        state.entity_node = if matches!(
            rust_kind,
            EntityKind::RustTypeAlias | EntityKind::RustConstant | EntityKind::RustStatic
        ) {
            ctx.node.parent()
        } else {
            Some(ctx.node)
        };
    }
}

/// Python entity capture (`python.*`). Callable kinds pull call intents from
/// the definition body; all kinds pull decorator intents, and classes pull
/// inheritance intents.
fn handle_python_capture<'a>(
    name_or_intent: &str,
    text: &str,
    ctx: &CaptureCtx<'a>,
    state: &mut CaptureState<'a>,
) {
    if let Some((entity_name, entity_kind, entity_line)) =
        python::handle_python_capture(name_or_intent, text, ctx.node)
    {
        state.name = Some(entity_name);
        state.kind = Some(entity_kind.clone());
        state.start_line = entity_line;
        // Use parent to get the full definition node for proper scope tracking
        // (captures always point to identifiers, not the full definition)
        state.entity_node = ctx.node.parent();

        // Extract call reference intents from Python function/method bodies
        let is_callable = matches!(
            entity_kind,
            EntityKind::PythonFunction | EntityKind::PythonMethod
        );
        #[expect(
            clippy::collapsible_if,
            reason = "separate guards aid readability when both conditions are semantically distinct"
        )]
        if is_callable {
            if let Some(entity_n) = state.entity_node {
                python::extract_reference_intents_python(
                    entity_n,
                    ctx.source_bytes,
                    &mut state.reference_intents,
                );
            }
        }

        // Phase 5: Extract decorators for ALL Python entities
        if let Some(entity_n) = state.entity_node {
            python::extract_decorator_intents_python(
                entity_n,
                ctx.source_bytes,
                &mut state.reference_intents,
            );
        }

        // Phase 5: Extract inheritance (EXTENDS) for Python classes
        if entity_kind == EntityKind::PythonClass
            && let Some(entity_n) = state.entity_node
        {
            python::extract_inheritance_intents_python(
                entity_n,
                ctx.source_bytes,
                &mut state.reference_intents,
            );
        }
    }
}

/// Markdown entity capture (`markdown.*`): delegates to the Markdown handler
/// and promotes its result to an entity capture on the same node.
fn handle_markdown_capture<'a>(
    name_or_intent: &str,
    text: &str,
    ctx: &CaptureCtx<'a>,
    state: &mut CaptureState<'a>,
) {
    if let Some((entity_name, entity_kind, entity_line)) =
        markdown::handle_markdown_capture(name_or_intent, text, ctx.node, ctx.source_bytes)
    {
        state.name = Some(entity_name);
        state.kind = Some(entity_kind);
        state.start_line = entity_line;
        state.entity_node = Some(ctx.node);
    }
}

/// DOM/CSS reference capture: converts `dom.*` and `css.class_*` captures
/// into JavaScript-side reference intents.
fn handle_dom_css_ref_capture<'a>(
    name_or_intent: &str,
    text: &str,
    ctx: &CaptureCtx<'a>,
    state: &mut CaptureState<'a>,
) {
    if let Some(intent) =
        javascript::handle_dom_css_capture(name_or_intent, text, ctx.node.start_position().row + 1)
    {
        state.reference_intents.push(intent);
    }
}

/// Groovy entity capture (`groovy.*`). The grammar is shared with Java, so
/// parent lookup mirrors the Java node kinds.
fn handle_groovy_capture<'a>(
    name_or_intent: &str,
    text: &str,
    ctx: &CaptureCtx<'a>,
    state: &mut CaptureState<'a>,
) {
    if let Some((entity_name, entity_kind, entity_line)) =
        groovy::handle_groovy_capture(name_or_intent, text, ctx.node)
    {
        state.name = Some(entity_name);
        state.kind = Some(entity_kind.clone());
        state.start_line = entity_line;
        // Find parent node using same patterns as Java (shared grammar)
        state.entity_node = match name_or_intent {
            n if n.contains("class") => find_parent_by_kind(ctx.node, "class_declaration"),
            n if n.contains("interface") => find_parent_by_kind(ctx.node, "interface_declaration"),
            n if n.contains("enum") => find_parent_by_kind(ctx.node, "enum_declaration"),
            n if n.contains("method") => find_parent_by_kind(ctx.node, "method_declaration")
                .or_else(|| find_parent_by_kind(ctx.node, "constructor_declaration")),
            _ => ctx.node.parent(),
        };
        // Groovy reference extraction is handled by ad-hoc `extract_method_calls`
        // in extract_entities_groovy_standard(), which uses innermost assignment.
        // Java's tree-sitter ref extraction is unreliable for Groovy because
        // tree-sitter-groovy misparses methods nested inside closures (e.g.,
        // `new AnAction() { @Override void actionPerformed(...) { ... } }`).
    }
}

/// C# entity capture (`csharp.*`). The signature capture rides along with the
/// name capture in the same match and does not introduce an entity; method
/// like kinds pull call + type-reference intents from their bodies.
fn handle_csharp_capture<'a>(
    name_or_intent: &str,
    text: &str,
    ctx: &CaptureCtx<'a>,
    state: &mut CaptureState<'a>,
) {
    // Signature capture rides along with the name capture in the
    // same match; it does not introduce an entity.
    if name_or_intent == "csharp.signature" {
        state.signature = Some(text.to_string());
    } else if let Some(capture) =
        csharp::handle_csharp_capture(name_or_intent, ctx.node, ctx.source_bytes)
    {
        let csharp_kind = capture.kind.clone();
        state.name = Some(capture.name);
        state.kind = Some(capture.kind);
        state.start_line = capture.start_line;
        state.entity_node = Some(capture.entity_node);

        // Extract call + type-reference intents from method-like
        // bodies (constructor bodies included). Class-level
        // inheritance/attribute/type refs are handled in enrich.
        if matches!(
            csharp_kind,
            EntityKind::CSharpMethod
                | EntityKind::CSharpConstructor
                | EntityKind::CSharpLocalFunction
        ) {
            csharp::extract_reference_intents_csharp(
                capture.entity_node,
                ctx.source_bytes,
                &mut state.reference_intents,
            );
        }
    }
}

/// C/C++ captures: classes, structs, namespaces, methods, functions, macros,
/// and `#include` directives.
fn handle_cpp_captures<'a>(
    name_or_intent: &str,
    text: &str,
    ctx: &CaptureCtx<'a>,
    state: &mut CaptureState<'a>,
) {
    match name_or_intent {
        "cpp_class.name" => {
            state.name = Some(text.to_string());
            state.kind = Some(EntityKind::CppClass);
            state.start_line = ctx.node.start_position().row + 1;
            state.entity_node = find_parent_by_kind(ctx.node, "class_specifier");
        }
        "c_struct.name" => {
            state.name = Some(text.to_string());
            state.kind = Some(EntityKind::CStruct);
            state.start_line = ctx.node.start_position().row + 1;
            state.entity_node = find_parent_by_kind(ctx.node, "struct_specifier");
        }
        "cpp_namespace.name" => {
            state.name = Some(text.to_string());
            state.kind = Some(EntityKind::CppNamespace);
            state.start_line = ctx.node.start_position().row + 1;
            state.entity_node = find_parent_by_kind(ctx.node, "namespace_definition");
        }
        "cpp_method.name" => {
            state.name = Some(text.to_string());
            state.kind = Some(EntityKind::CppMethod);
            state.start_line = ctx.node.start_position().row + 1;
            state.entity_node = find_parent_by_kind(ctx.node, "function_definition")
                .or_else(|| find_parent_by_kind(ctx.node, "declaration"))
                .or_else(|| find_parent_by_kind(ctx.node, "field_declaration"));
            if let Some(m_node) = state.entity_node {
                cpp::extract_reference_intents_cpp(
                    m_node,
                    ctx.source_bytes,
                    &mut state.reference_intents,
                );
            }
        }
        "c_function.name" => {
            state.name = Some(text.to_string());
            state.kind = Some(EntityKind::CFunction);
            state.start_line = ctx.node.start_position().row + 1;
            state.entity_node = find_parent_by_kind(ctx.node, "function_definition");
            if let Some(m_node) = state.entity_node {
                cpp::extract_reference_intents_cpp(
                    m_node,
                    ctx.source_bytes,
                    &mut state.reference_intents,
                );
            }
        }
        "preproc.macro" => {
            state.name = Some(text.to_string());
            state.kind = Some(EntityKind::MacroDefinition);
            state.start_line = ctx.node.start_position().row + 1;
            state.entity_node = find_parent_by_kind(ctx.node, "preproc_def");
        }
        "preproc.include" => {
            // Extract included file path and register it as an intent
            let mut path_str = text.to_string();
            if path_str.starts_with('"') || path_str.starts_with('<') {
                path_str = path_str[1..path_str.len() - 1].to_string();
            }
            state.reference_intents.push(ReferenceIntent::Call {
                method: path_str,
                receiver: None,
                line: ctx.node.start_position().row + 1,
                arg_count: None,
            });
        }
        _ => {}
    }
}
