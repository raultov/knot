use crate::models::{CallIntent, ReferenceIntent};
use crate::pipeline::parser::utils::node_text;
use tree_sitter::Node;

/// Recursively extract all call intents from TypeScript/TSX, returning (intent, byte_pos) pairs.
pub(crate) fn collect_all_reference_intents_typescript(
    node: Node<'_>,
    source: &[u8],
    intents: &mut Vec<(ReferenceIntent, usize)>,
) {
    let byte_pos = node.start_byte();
    let line = node.start_position().row + 1;

    match node.kind() {
        "call_expression" | "new_expression" => {
            // Use non-recursive extraction to avoid double-processing children
            // (this function already handles recursion via the child loop below)
            let call_intents = extract_single_call_intent_typescript(node, source);
            for call in call_intents {
                intents.push((
                    ReferenceIntent::Call {
                        method: call.method,
                        receiver: call.receiver,
                        line,
                        arg_count: call.arg_count,
                    },
                    byte_pos,
                ));
            }
        }
        "jsx_self_closing_element" | "jsx_opening_element" => {
            let mut call_intents = Vec::new();
            extract_jsx_component_invocation(node, source, &mut call_intents);
            for call in call_intents {
                intents.push((
                    ReferenceIntent::Call {
                        method: call.method,
                        receiver: call.receiver,
                        line,
                        arg_count: call.arg_count,
                    },
                    byte_pos,
                ));
            }
        }
        "decorator" => {
            // Extract decorator references (e.g., @NgModule({ declarations: [AppComponent] }))
            let mut decorator_refs = Vec::new();
            extract_identifiers_from_decorator(node, source, &mut decorator_refs, line);
            for ref_intent in decorator_refs {
                intents.push((ref_intent, byte_pos));
            }
        }
        "type_identifier" => {
            // Extract type references (e.g., constructor parameters, property types)
            let type_name = node_text(node, source);
            // Only capture capitalized identifiers (likely classes/interfaces)
            if type_name.chars().next().is_some_and(|c| c.is_uppercase()) {
                intents.push((ReferenceIntent::TypeReference { type_name, line }, byte_pos));
            }
        }
        "export_statement" => {
            // Handle re-exports: `export { A, B } from './x'`
            if let Some(_source) = node.child_by_field_name("source") {
                let is_type_export = node.children(&mut node.walk()).any(|c| c.kind() == "type");
                let mut clause_child = node.child(0);
                while let Some(c) = clause_child {
                    match c.kind() {
                        "export_clause" | "export_type_clause" => {
                            let mut spec_child = c.child(0);
                            while let Some(spec) = spec_child {
                                if spec.kind() == "export_specifier"
                                    && let Some(name_node) = spec.child_by_field_name("name")
                                {
                                    let name = node_text(name_node, source);
                                    if name.chars().next().is_some_and(|ch| ch.is_uppercase()) {
                                        if is_type_export || c.kind() == "export_type_clause" {
                                            intents.push((
                                                ReferenceIntent::TypeReference {
                                                    type_name: name,
                                                    line,
                                                },
                                                byte_pos,
                                            ));
                                        } else {
                                            intents.push((
                                                ReferenceIntent::ValueReference {
                                                    value_name: name,
                                                    line,
                                                },
                                                byte_pos,
                                            ));
                                        }
                                    }
                                }
                                spec_child = spec.next_sibling();
                            }
                        }
                        _ => {}
                    }
                    clause_child = c.next_sibling();
                }
            }
        }
        "identifier" => {
            let name = node_text(node, source);
            if name.chars().next().is_some_and(|c| c.is_uppercase())
                && !TS_GLOBALS.contains(&name.as_str())
                && let Some(parent) = node.parent()
                && !is_identifier_excluded_from_value_ref(parent, node)
            {
                intents.push((
                    ReferenceIntent::ValueReference {
                        value_name: name,
                        line,
                    },
                    byte_pos,
                ));
            }
        }
        _ => {}
    }

    // Recursively process children
    let mut child = node.child(0);
    while let Some(c) = child {
        collect_all_reference_intents_typescript(c, source, intents);
        child = c.next_sibling();
    }
}

/// Extract the root type name from a TypeScript type node, stripping generics
/// and qualified path prefixes. E.g. `Generic<T>` → "Generic", `NS.Parent` → "Parent".
fn extract_root_type_name(node: Node<'_>, source: &[u8]) -> Option<String> {
    match node.kind() {
        "identifier" | "type_identifier" => Some(node_text(node, source)),
        "generic_type" => node.child(0).map(|c| node_text(c, source)),
        "member_expression" | "nested_type_identifier" => node_text(node, source)
            .split('.')
            .next_back()
            .map(|s| s.to_string()),
        _ => None,
    }
}

/// Known JavaScript/TypeScript global builtins that should not generate ValueReference intents.
const TS_GLOBALS: &[&str] = &[
    "Math",
    "JSON",
    "Promise",
    "String",
    "Number",
    "Boolean",
    "Array",
    "Object",
    "Error",
    "Date",
    "RegExp",
    "Map",
    "Set",
    "Symbol",
    "BigInt",
    "WeakMap",
    "WeakSet",
    "Proxy",
    "Reflect",
    "Intl",
    "console",
    "window",
    "document",
    "globalThis",
    "undefined",
    "NaN",
    "Infinity",
    "null",
    "arguments",
    "this",
    "super",
];

/// Returns true if the given identifier node is excluded from value reference extraction.
/// Excludes identifiers in declarative contexts (class/interface/function names),
/// import/export specifiers, member expression properties, and pair keys.
fn is_identifier_excluded_from_value_ref(parent: Node<'_>, node: Node<'_>) -> bool {
    matches!(
        parent.kind(),
        "class_declaration"
            | "interface_declaration"
            | "function_declaration"
            | "method_definition"
            | "import_specifier"
            | "export_specifier"
            | "property_identifier"
            | "shorthand_property_identifier"
    ) || (parent.kind() == "member_expression"
        && parent.child_by_field_name("property").as_ref() == Some(&node))
        || (parent.kind() == "variable_declarator"
            && parent.child_by_field_name("name").as_ref() == Some(&node))
        || (parent.kind() == "pair" && parent.child_by_field_name("key").as_ref() == Some(&node))
}

/// Recursively extract capitalized identifiers used as values (not types) from TS/JS AST.
///
/// Emits `ValueReference` for capitalized identifiers found in:
/// - Object literal values (`{ key: ClassName }`)
/// - Array elements (`[ClassName]`)
/// - Variable initializers (`const x = ClassName`)
/// - Assignment right-hand side (`x = ClassName`)
/// - Function call arguments (`foo(ClassName)`)
/// - Return statements (`return ClassName`)
/// - Spread elements (`[...arr, ClassName]`)
///
/// Excludes identifiers that are:
/// - Part of a declaration (variable name, class name, function name)
/// - Part of a `member_expression.property` (handled by enum_usages)
/// - Known global builtins
pub(crate) fn extract_value_references_typescript(
    node: Node<'_>,
    source: &[u8],
    intents: &mut Vec<ReferenceIntent>,
) {
    let line = node.start_position().row + 1;

    if node.kind() == "identifier" {
        let name = node_text(node, source);
        if name.chars().next().is_some_and(|c| c.is_uppercase())
            && !TS_GLOBALS.contains(&name.as_str())
            && let Some(parent) = node.parent()
            && !is_identifier_excluded_from_value_ref(parent, node)
        {
            intents.push(ReferenceIntent::ValueReference {
                value_name: name,
                line,
            });
        }
        return;
    }

    // Recurse into children
    let mut child = node.child(0);
    while let Some(c) = child {
        extract_value_references_typescript(c, source, intents);
        child = c.next_sibling();
    }
}

/// Extract class inheritance (extends/implements) from TypeScript AST nodes.
/// Handles both class_declaration and interface_declaration AST nodes.
pub(crate) fn extract_class_inheritance(
    entity_node: Node<'_>,
    source: &[u8],
    intents: &mut Vec<ReferenceIntent>,
) {
    let line = entity_node.start_position().row + 1;

    match entity_node.kind() {
        "class_declaration" => {
            let mut child = entity_node.child(0);
            while let Some(c) = child {
                if c.kind() == "class_heritage" {
                    let mut hc = c.child(0);
                    while let Some(heritage) = hc {
                        match heritage.kind() {
                            "extends_clause" => {
                                if let Some(name) = heritage
                                    .child_by_field_name("value")
                                    .and_then(|n| extract_root_type_name(n, source))
                                {
                                    intents.push(ReferenceIntent::Extends { parent: name, line });
                                }
                            }
                            "implements_clause" => {
                                let mut tc = heritage.child(0);
                                while let Some(type_child) = tc {
                                    if let Some(name) = extract_root_type_name(type_child, source) {
                                        intents.push(ReferenceIntent::Implements {
                                            interface: name,
                                            line,
                                        });
                                    }
                                    tc = type_child.next_sibling();
                                }
                            }
                            _ => {}
                        }
                        hc = heritage.next_sibling();
                    }
                }
                child = c.next_sibling();
            }
        }
        "interface_declaration" => {
            let mut child = entity_node.child(0);
            while let Some(c) = child {
                if c.kind() == "extends_type_clause" {
                    let mut tc = c.child_by_field_name("type");
                    while let Some(type_child) = tc {
                        if let Some(name) = extract_root_type_name(type_child, source) {
                            intents.push(ReferenceIntent::Extends { parent: name, line });
                        }
                        tc = type_child.next_named_sibling();
                    }
                }
                child = c.next_sibling();
            }
        }
        _ => {}
    }
}

/// Extract decorator references from TypeScript/TSX decorators (e.g., @NgModule, @Component).
///
/// Recursively searches for `decorator` nodes and extracts capitalized identifiers
/// (likely class/component names) as TypeReference intents.
///
/// Example:
/// ```typescript
/// @NgModule({
///   declarations: [AppComponent, UserComponent],
///   bootstrap: [AppComponent]
/// })
/// export class AppModule {}
/// ```
///
/// This will extract: AppComponent (twice), UserComponent
pub(crate) fn extract_decorator_references(
    node: Node<'_>,
    source: &[u8],
    intents: &mut Vec<ReferenceIntent>,
) {
    let line = node.start_position().row + 1;

    // If this is a decorator node, extract references from its arguments
    if node.kind() == "decorator" {
        extract_identifiers_from_decorator(node, source, intents, line);
    }

    // Recursively process children
    let mut child = node.child(0);
    while let Some(c) = child {
        extract_decorator_references(c, source, intents);
        child = c.next_sibling();
    }
}

/// Extract capitalized identifiers from decorator arguments (likely class references).
fn extract_identifiers_from_decorator(
    decorator_node: Node<'_>,
    source: &[u8],
    intents: &mut Vec<ReferenceIntent>,
    line: usize,
) {
    // Recursively scan all children for identifiers
    let mut child = decorator_node.child(0);
    while let Some(c) = child {
        match c.kind() {
            "identifier" | "type_identifier" => {
                let name = node_text(c, source);
                // Only capture capitalized identifiers (likely classes/components)
                if name.chars().next().is_some_and(|ch| ch.is_uppercase()) {
                    intents.push(ReferenceIntent::TypeReference {
                        type_name: name,
                        line,
                    });
                }
            }
            _ => {
                // Recurse into nested structures (objects, arrays, etc.)
                extract_identifiers_from_decorator(c, source, intents, line);
            }
        }
        child = c.next_sibling();
    }
}

/// Extract type references from TypeScript type annotations.
///
/// Recursively searches for `type_identifier` nodes in:
/// - Constructor parameters (dependency injection)
/// - Method parameters
/// - Property types
/// - Return types
///
/// Example:
/// ```typescript
/// class AppComponent {
///   constructor(private analytics: AnalyticsService, private seo: SeoService) {}
///   
///   process(data: DataService): ResultType {
///     return null;
///   }
/// }
/// ```
///
/// This will extract: AnalyticsService, SeoService, DataService, ResultType
pub(crate) fn extract_type_references(
    node: Node<'_>,
    source: &[u8],
    intents: &mut Vec<ReferenceIntent>,
) {
    let line = node.start_position().row + 1;

    // Capture type_identifier nodes (type annotations)
    if node.kind() == "type_identifier" {
        let type_name = node_text(node, source);
        // Only capture capitalized identifiers (likely classes/interfaces)
        if type_name.chars().next().is_some_and(|c| c.is_uppercase()) {
            intents.push(ReferenceIntent::TypeReference { type_name, line });
        }
    }

    // Recursively process children
    let mut child = node.child(0);
    while let Some(c) = child {
        extract_type_references(c, source, intents);
        child = c.next_sibling();
    }
}

/// Extract reference intents from a TypeScript function/method body (wrapper for backward compatibility).
pub(crate) fn extract_reference_intents_typescript(
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
            arg_count: call.arg_count,
        });
    }

    extract_enum_usages_typescript(node, source, intents);
    extract_type_references(node, source, intents);
    extract_value_references_typescript(node, source, intents);
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
/// - Property/getter access: `this.client`, `this.field` -> records access to property/getter
pub(crate) fn extract_call_intents_typescript(
    node: Node<'_>,
    source: &[u8],
    intents: &mut Vec<CallIntent>,
) {
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
                            arg_count: None,
                        });
                    }
                }
            } else {
                intents.push(CallIntent {
                    method,
                    receiver,
                    line,
                    arg_count: None,
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
                    arg_count: None,
                });
                break;
            }
            child = c.next_sibling();
        }
    } else if node.kind() == "jsx_self_closing_element" || node.kind() == "jsx_opening_element" {
        // JSX component invocation (e.g., <ChartToolbar />, <Sheet.Content />)
        extract_jsx_component_invocation(node, source, intents);
    } else if node.kind() == "member_expression" {
        // Detect property/getter access via `this.property` (e.g., this.client, this.field)
        // This captures accesses like:
        //   - this.client          (getter access)
        //   - this._field          (private field access)
        //   - this.publicProp      (public property access)
        if let Some(object_node) = node.child_by_field_name("object")
            && node_text(object_node, source) == "this"
            && let Some(property_node) = node.child_by_field_name("property")
        {
            let prop_text = node_text(property_node, source);
            let line = node.start_position().row + 1;
            intents.push(CallIntent {
                method: prop_text,
                receiver: Some("this".to_string()),
                line,
                arg_count: None,
            });
        }
    }

    // Recursively process children
    let mut child = node.child(0);
    while let Some(c) = child {
        extract_call_intents_typescript(c, source, intents);
        child = c.next_sibling();
    }
}

/// Extract call intents from a SINGLE node without recursive descent.
///
/// This is the non-recursive version of `extract_call_intents_typescript`,
/// designed to be used in contexts where the caller already handles tree traversal
/// (e.g., the fallback pass in `collect_all_reference_intents_typescript`).
///
/// By extracting only the current node's intent, we avoid double-processing children
/// that would cause duplicate CALLS with incorrect byte_pos/line attribution.
///
/// Handles property/getter access (e.g., `this.client`) as well as call expressions.
pub(crate) fn extract_single_call_intent_typescript(
    node: Node<'_>,
    source: &[u8],
) -> Vec<CallIntent> {
    let mut intents = Vec::new();

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
                            arg_count: None,
                        });
                    }
                }
            } else {
                intents.push(CallIntent {
                    method,
                    receiver,
                    line,
                    arg_count: None,
                });
            }
        }

        // Also scan arguments for callback references (e.g., app.use(this.handler))
        extract_callback_arguments(node, source, &mut intents, line);
    } else if node.kind() == "new_expression" {
        let line = node.start_position().row + 1;
        let mut child = node.child(0);
        while let Some(c) = child {
            if c.kind() == "identifier" || c.kind() == "type_identifier" {
                intents.push(CallIntent {
                    method: node_text(c, source),
                    receiver: None,
                    line,
                    arg_count: None,
                });
                break;
            }
            child = c.next_sibling();
        }
    } else if node.kind() == "jsx_self_closing_element" || node.kind() == "jsx_opening_element" {
        // JSX component invocation (e.g., <ChartToolbar />, <Sheet.Content />)
        extract_jsx_component_invocation(node, source, &mut intents);
    } else if node.kind() == "member_expression" {
        // Detect property/getter access via `this.property` (e.g., this.client, this.field)
        if let Some(object_node) = node.child_by_field_name("object")
            && node_text(object_node, source) == "this"
            && let Some(property_node) = node.child_by_field_name("property")
        {
            let prop_text = node_text(property_node, source);
            let line = node.start_position().row + 1;
            intents.push(CallIntent {
                method: prop_text,
                receiver: Some("this".to_string()),
                line,
                arg_count: None,
            });
        }
    }

    // NO recursive child processing - that's the key difference!
    intents
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
pub(crate) fn extract_jsx_component_invocation(
    node: Node<'_>,
    source: &[u8],
    intents: &mut Vec<CallIntent>,
) {
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
                    arg_count: None,
                });
            } else {
                // Simple component name
                intents.push(CallIntent {
                    method: comp_name,
                    receiver: None,
                    line,
                    arg_count: None,
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
pub(crate) fn extract_callback_arguments(
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
                            arg_count: None,
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
                        arg_count: None,
                    });
                }
            }
            arg = a.next_sibling();
        }
    }
}

/// Check if a string is a TypeScript/JavaScript reserved keyword.
pub(crate) fn is_reserved_keyword(word: &str) -> bool {
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

/// Extract enum and static member usages from a TypeScript node (e.g., EnumName.Value, ClassName.STATIC).
///
/// Recursively searches for member_expression nodes where the object is a capitalized identifier,
/// which typically represents enum or static class member access patterns like WebWorkerEvent.Console.
pub(crate) fn extract_enum_usages_typescript(
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

/// Extract HTML attributes (id, className) from JSX elements in TypeScript/TSX.
///
/// Used to index React components' HTML attributes for cross-language search
/// (e.g., finding which components use a specific CSS class).
///
/// Extracts:
/// - `id="my-id"` → HtmlId entity with name "my-id"
/// - `className="btn primary"` → HtmlClass entities for "btn" and "primary"
///
/// Returns a vector of tuples (attribute_name, attribute_value, line).
#[allow(dead_code)]
pub(crate) fn extract_jsx_attributes(
    node: Node<'_>,
    source: &[u8],
) -> Vec<(String, String, usize)> {
    use crate::pipeline::parser::utils::node_text;

    let mut attributes = Vec::new();

    // JSX attributes are structured as:
    // jsx_attribute
    //   property_identifier (e.g., "id", "className")
    //   jsx_expression | string (the value)
    let mut child = node.child(0);
    while let Some(c) = child {
        if c.kind() == "jsx_attribute" {
            let line = c.start_position().row + 1;
            let mut attr_name = String::new();
            let mut attr_value = String::new();

            // Navigate children to extract property_identifier and value
            let mut attr_child = c.child(0);
            while let Some(ac) = attr_child {
                if ac.kind() == "property_identifier" {
                    attr_name = node_text(ac, source);
                } else if ac.kind() == "string" {
                    // String literal (e.g., "my-id")
                    let raw = node_text(ac, source);
                    // Remove quotes
                    attr_value = raw.trim_matches(|c| c == '"' || c == '\'').to_string();
                } else if ac.kind() == "jsx_expression" {
                    // Expression (e.g., {myVar}) - we skip these for now
                    // Only capture static string values
                    attr_child = ac.next_sibling();
                    continue;
                }
                attr_child = ac.next_sibling();
            }

            // Only capture id and className attributes with non-empty values
            if (attr_name == "id" || attr_name == "className") && !attr_value.is_empty() {
                attributes.push((attr_name, attr_value, line));
            }
        }
        child = c.next_sibling();
    }

    attributes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_reserved_keyword_true() {
        assert!(is_reserved_keyword("true"));
        assert!(is_reserved_keyword("false"));
        assert!(is_reserved_keyword("class"));
        assert!(is_reserved_keyword("function"));
        assert!(is_reserved_keyword("async"));
        assert!(is_reserved_keyword("await"));
    }

    #[test]
    fn test_is_reserved_keyword_false() {
        assert!(!is_reserved_keyword("myVar"));
        assert!(!is_reserved_keyword("handler"));
        assert!(!is_reserved_keyword("MyClass"));
        assert!(!is_reserved_keyword("someFunction"));
    }

    #[test]
    fn test_extract_jsx_component_invocation_simple() {
        let code = "function render() { return <ChartToolbar />; }";
        let tree = crate::pipeline::parser::test_utils::parse_tsx_snippet(code)
            .expect("Failed to parse TSX code");

        fn find_jsx_element(node: tree_sitter::Node) -> Option<tree_sitter::Node> {
            if matches!(
                node.kind(),
                "jsx_self_closing_element" | "jsx_opening_element"
            ) {
                return Some(node);
            }
            let mut i = 0u32;
            while let Some(child) = node.child(i) {
                if let Some(found) = find_jsx_element(child) {
                    return Some(found);
                }
                i += 1;
            }
            None
        }

        if let Some(jsx) = find_jsx_element(tree.root_node()) {
            let code_bytes = code.as_bytes();
            let mut intents: Vec<CallIntent> = Vec::new();
            extract_jsx_component_invocation(jsx, code_bytes, &mut intents);
            assert!(!intents.is_empty());
            assert_eq!(intents[0].method, "ChartToolbar");
            assert!(intents[0].receiver.is_none());
        }
    }

    #[test]
    fn test_extract_jsx_component_invocation_namespaced() {
        let code = "function render() { return <Sheet.Content />; }";
        let tree = crate::pipeline::parser::test_utils::parse_tsx_snippet(code)
            .expect("Failed to parse TSX code");

        fn find_jsx_element(node: tree_sitter::Node) -> Option<tree_sitter::Node> {
            if matches!(
                node.kind(),
                "jsx_self_closing_element" | "jsx_opening_element"
            ) {
                return Some(node);
            }
            let mut i = 0u32;
            while let Some(child) = node.child(i) {
                if let Some(found) = find_jsx_element(child) {
                    return Some(found);
                }
                i += 1;
            }
            None
        }

        if let Some(jsx) = find_jsx_element(tree.root_node()) {
            let code_bytes = code.as_bytes();
            let mut intents: Vec<CallIntent> = Vec::new();
            extract_jsx_component_invocation(jsx, code_bytes, &mut intents);
            assert!(!intents.is_empty());
            assert_eq!(intents[0].method, "Content");
            assert_eq!(intents[0].receiver, Some("Sheet".to_string()));
        }
    }

    #[test]
    fn test_extract_single_call_intent_typescript_simple() {
        let code = "function test() { method(); }";
        let tree = crate::pipeline::parser::test_utils::parse_typescript_snippet(code)
            .expect("Failed to parse TypeScript code");

        fn find_call_expression(node: tree_sitter::Node) -> Option<tree_sitter::Node> {
            if node.kind() == "call_expression" {
                return Some(node);
            }
            let mut i = 0u32;
            while let Some(child) = node.child(i) {
                if let Some(found) = find_call_expression(child) {
                    return Some(found);
                }
                i += 1;
            }
            None
        }

        if let Some(call) = find_call_expression(tree.root_node()) {
            let code_bytes = code.as_bytes();
            let intents = extract_single_call_intent_typescript(call, code_bytes);
            assert!(!intents.is_empty());
            assert_eq!(intents[0].method, "method");
            assert!(intents[0].receiver.is_none());
        }
    }

    #[test]
    fn test_extract_single_call_intent_typescript_member() {
        let code = "function test() { obj.method(); }";
        let tree = crate::pipeline::parser::test_utils::parse_typescript_snippet(code)
            .expect("Failed to parse TypeScript code");

        fn find_call_expression(node: tree_sitter::Node) -> Option<tree_sitter::Node> {
            if node.kind() == "call_expression" {
                return Some(node);
            }
            let mut i = 0u32;
            while let Some(child) = node.child(i) {
                if let Some(found) = find_call_expression(child) {
                    return Some(found);
                }
                i += 1;
            }
            None
        }

        if let Some(call) = find_call_expression(tree.root_node()) {
            let code_bytes = code.as_bytes();
            let intents = extract_single_call_intent_typescript(call, code_bytes);
            assert!(!intents.is_empty());
            assert_eq!(intents[0].method, "method");
            assert_eq!(intents[0].receiver, Some("obj".to_string()));
        }
    }

    #[test]
    fn test_extract_single_call_intent_typescript_new() {
        let code = "function test() { new MyClass(); }";
        let tree = crate::pipeline::parser::test_utils::parse_typescript_snippet(code)
            .expect("Failed to parse TypeScript code");

        fn find_new_expression(node: tree_sitter::Node) -> Option<tree_sitter::Node> {
            if node.kind() == "new_expression" {
                return Some(node);
            }
            let mut i = 0u32;
            while let Some(child) = node.child(i) {
                if let Some(found) = find_new_expression(child) {
                    return Some(found);
                }
                i += 1;
            }
            None
        }

        if let Some(new_expr) = find_new_expression(tree.root_node()) {
            let code_bytes = code.as_bytes();
            let intents = extract_single_call_intent_typescript(new_expr, code_bytes);
            assert!(!intents.is_empty());
            assert_eq!(intents[0].method, "MyClass");
            assert!(intents[0].receiver.is_none());
        }
    }

    fn assert_extends(intents: &[ReferenceIntent], expected: &str) {
        let found = intents
            .iter()
            .any(|i| matches!(i, ReferenceIntent::Extends { parent, .. } if parent == expected));
        assert!(
            found,
            "Expected Extends -> '{}', got {:?}",
            expected, intents
        );
    }

    fn assert_implements(intents: &[ReferenceIntent], expected: &str) {
        let found = intents.iter().any(
            |i| matches!(i, ReferenceIntent::Implements { interface, .. } if interface == expected),
        );
        assert!(
            found,
            "Expected Implements -> '{}', got {:?}",
            expected, intents
        );
    }

    #[test]
    fn test_extract_class_inheritance_extends() {
        let code = "class Child extends Parent { }";
        let tree = crate::pipeline::parser::test_utils::parse_typescript_snippet(code)
            .expect("Failed to parse TypeScript code");

        fn find_class_declaration(node: tree_sitter::Node) -> Option<tree_sitter::Node> {
            if node.kind() == "class_declaration" {
                return Some(node);
            }
            let mut i = 0u32;
            while let Some(child) = node.child(i) {
                if let Some(found) = find_class_declaration(child) {
                    return Some(found);
                }
                i += 1;
            }
            None
        }

        if let Some(class_node) = find_class_declaration(tree.root_node()) {
            let mut intents = Vec::new();
            extract_class_inheritance(class_node, code.as_bytes(), &mut intents);
            assert_extends(&intents, "Parent");
        }
    }

    #[test]
    fn test_extract_class_inheritance_implements() {
        let code = "class Child implements IFoo, IBar { }";
        let tree = crate::pipeline::parser::test_utils::parse_typescript_snippet(code)
            .expect("Failed to parse TypeScript code");

        fn find_class_declaration(node: tree_sitter::Node) -> Option<tree_sitter::Node> {
            if node.kind() == "class_declaration" {
                return Some(node);
            }
            let mut i = 0u32;
            while let Some(child) = node.child(i) {
                if let Some(found) = find_class_declaration(child) {
                    return Some(found);
                }
                i += 1;
            }
            None
        }

        if let Some(class_node) = find_class_declaration(tree.root_node()) {
            let mut intents = Vec::new();
            extract_class_inheritance(class_node, code.as_bytes(), &mut intents);
            assert_implements(&intents, "IFoo");
            assert_implements(&intents, "IBar");
        }
    }

    #[test]
    fn test_extract_class_inheritance_extends_and_implements() {
        let code = "class Child extends Parent implements IFoo { }";
        let tree = crate::pipeline::parser::test_utils::parse_typescript_snippet(code)
            .expect("Failed to parse TypeScript code");

        fn find_class_declaration(node: tree_sitter::Node) -> Option<tree_sitter::Node> {
            if node.kind() == "class_declaration" {
                return Some(node);
            }
            let mut i = 0u32;
            while let Some(child) = node.child(i) {
                if let Some(found) = find_class_declaration(child) {
                    return Some(found);
                }
                i += 1;
            }
            None
        }

        if let Some(class_node) = find_class_declaration(tree.root_node()) {
            let mut intents = Vec::new();
            extract_class_inheritance(class_node, code.as_bytes(), &mut intents);
            assert_extends(&intents, "Parent");
            assert_implements(&intents, "IFoo");
        }
    }

    #[test]
    fn test_extract_class_inheritance_qualified_extends() {
        let code = "class Child extends NS.Parent { }";
        let tree = crate::pipeline::parser::test_utils::parse_typescript_snippet(code)
            .expect("Failed to parse TypeScript code");

        fn find_class_declaration(node: tree_sitter::Node) -> Option<tree_sitter::Node> {
            if node.kind() == "class_declaration" {
                return Some(node);
            }
            let mut i = 0u32;
            while let Some(child) = node.child(i) {
                if let Some(found) = find_class_declaration(child) {
                    return Some(found);
                }
                i += 1;
            }
            None
        }

        if let Some(class_node) = find_class_declaration(tree.root_node()) {
            let mut intents = Vec::new();
            extract_class_inheritance(class_node, code.as_bytes(), &mut intents);
            assert_extends(&intents, "Parent");
        }
    }

    #[test]
    fn test_extract_class_inheritance_generic_extends() {
        let code = "class Child extends Generic<T> { }";
        let tree = crate::pipeline::parser::test_utils::parse_typescript_snippet(code)
            .expect("Failed to parse TypeScript code");

        fn find_class_declaration(node: tree_sitter::Node) -> Option<tree_sitter::Node> {
            if node.kind() == "class_declaration" {
                return Some(node);
            }
            let mut i = 0u32;
            while let Some(child) = node.child(i) {
                if let Some(found) = find_class_declaration(child) {
                    return Some(found);
                }
                i += 1;
            }
            None
        }

        if let Some(class_node) = find_class_declaration(tree.root_node()) {
            let mut intents = Vec::new();
            extract_class_inheritance(class_node, code.as_bytes(), &mut intents);
            assert_extends(&intents, "Generic");
        }
    }

    #[test]
    fn test_extract_interface_extends_simple() {
        let code = "interface IB extends IA { }";
        let tree = crate::pipeline::parser::test_utils::parse_typescript_snippet(code)
            .expect("Failed to parse TypeScript code");

        fn find_interface_declaration(node: tree_sitter::Node) -> Option<tree_sitter::Node> {
            if node.kind() == "interface_declaration" {
                return Some(node);
            }
            let mut i = 0u32;
            while let Some(child) = node.child(i) {
                if let Some(found) = find_interface_declaration(child) {
                    return Some(found);
                }
                i += 1;
            }
            None
        }

        if let Some(iface_node) = find_interface_declaration(tree.root_node()) {
            let mut intents = Vec::new();
            extract_class_inheritance(iface_node, code.as_bytes(), &mut intents);
            assert_extends(&intents, "IA");
        }
    }

    #[test]
    fn test_extract_interface_extends_multiple() {
        let code = "interface IC extends IA, IB { }";
        let tree = crate::pipeline::parser::test_utils::parse_typescript_snippet(code)
            .expect("Failed to parse TypeScript code");

        fn find_interface_declaration(node: tree_sitter::Node) -> Option<tree_sitter::Node> {
            if node.kind() == "interface_declaration" {
                return Some(node);
            }
            let mut i = 0u32;
            while let Some(child) = node.child(i) {
                if let Some(found) = find_interface_declaration(child) {
                    return Some(found);
                }
                i += 1;
            }
            None
        }

        if let Some(iface_node) = find_interface_declaration(tree.root_node()) {
            let mut intents = Vec::new();
            extract_class_inheritance(iface_node, code.as_bytes(), &mut intents);
            assert_extends(&intents, "IA");
            assert_extends(&intents, "IB");
        }
    }

    #[test]
    fn test_extract_callback_arguments_member_expression() {
        let code = "function test() { app.use(this.handler); }";
        let tree = crate::pipeline::parser::test_utils::parse_typescript_snippet(code)
            .expect("Failed to parse TypeScript code");

        fn find_call_expression(node: tree_sitter::Node) -> Option<tree_sitter::Node> {
            if node.kind() == "call_expression" {
                return Some(node);
            }
            let mut i = 0u32;
            while let Some(child) = node.child(i) {
                if let Some(found) = find_call_expression(child) {
                    return Some(found);
                }
                i += 1;
            }
            None
        }

        if let Some(call) = find_call_expression(tree.root_node()) {
            let code_bytes = code.as_bytes();
            let mut intents: Vec<CallIntent> = Vec::new();
            extract_callback_arguments(call, code_bytes, &mut intents, 1);
            // Should find the callback in arguments
            assert!(intents.iter().any(|i| i.method == "handler"));
        }
    }

    #[test]
    fn test_extract_enum_usages_typescript() {
        let code = "const val = Color.RED;";
        let tree = crate::pipeline::parser::test_utils::parse_typescript_snippet(code)
            .expect("Failed to parse TypeScript code");

        let code_bytes = code.as_bytes();
        let mut intents: Vec<ReferenceIntent> = Vec::new();
        extract_enum_usages_typescript(tree.root_node(), code_bytes, &mut intents);
        // Should find Color enum usage
        assert!(intents.iter().any(|i| {
            if let ReferenceIntent::TypeReference { type_name, .. } = i {
                type_name == "Color"
            } else {
                false
            }
        }));
    }

    #[test]
    fn test_extract_call_intents_this_property_access() {
        let code = "class Frame { navigate() { this.client.send(); } }";
        let tree = crate::pipeline::parser::test_utils::parse_typescript_snippet(code)
            .expect("Failed to parse TypeScript code");

        let code_bytes = code.as_bytes();
        let mut intents: Vec<CallIntent> = Vec::new();
        extract_call_intents_typescript(tree.root_node(), code_bytes, &mut intents);

        // Should find both this.client (property access) and send() (method call)
        assert!(
            intents
                .iter()
                .any(|i| i.method == "client" && i.receiver == Some("this".to_string()))
        );
        assert!(intents.iter().any(|i| i.method == "send"));
    }

    #[test]
    fn test_extract_call_intents_this_private_property_access() {
        let code = "class Frame { method() { return this.#client; } }";
        let tree = crate::pipeline::parser::test_utils::parse_typescript_snippet(code)
            .expect("Failed to parse TypeScript code");

        let code_bytes = code.as_bytes();
        let mut intents: Vec<CallIntent> = Vec::new();
        extract_call_intents_typescript(tree.root_node(), code_bytes, &mut intents);

        // Should find this.#client (private property access)
        assert!(
            intents
                .iter()
                .any(|i| i.method == "#client" && i.receiver == Some("this".to_string()))
        );
    }

    #[test]
    fn test_extract_single_call_intent_this_property_access() {
        let code = "function test() { const x = this.myProperty; }";
        let tree = crate::pipeline::parser::test_utils::parse_typescript_snippet(code)
            .expect("Failed to parse TypeScript code");

        fn find_member_expression(node: tree_sitter::Node) -> Option<tree_sitter::Node> {
            if node.kind() == "member_expression" {
                return Some(node);
            }
            let mut i = 0u32;
            while let Some(child) = node.child(i) {
                if let Some(found) = find_member_expression(child) {
                    return Some(found);
                }
                i += 1;
            }
            None
        }

        if let Some(member_expr) = find_member_expression(tree.root_node()) {
            let code_bytes = code.as_bytes();
            let intents = extract_single_call_intent_typescript(member_expr, code_bytes);
            assert!(!intents.is_empty());
            assert_eq!(intents[0].method, "myProperty");
            assert_eq!(intents[0].receiver, Some("this".to_string()));
        }
    }

    #[test]
    fn test_extract_call_intents_this_getter_access() {
        let code = "class Frame { get client(): CDPSession { return this.#client; } navigate() { this.client.send('test'); } }";
        let tree = crate::pipeline::parser::test_utils::parse_typescript_snippet(code)
            .expect("Failed to parse TypeScript code");

        let code_bytes = code.as_bytes();
        let mut intents: Vec<CallIntent> = Vec::new();
        extract_call_intents_typescript(tree.root_node(), code_bytes, &mut intents);

        // Should find:
        // 1. this.#client (private field access in getter)
        // 2. this.client (getter call in navigate)
        // 3. send() (method call on result)
        assert!(
            intents
                .iter()
                .any(|i| i.method == "#client" && i.receiver == Some("this".to_string()))
        );
        assert!(
            intents
                .iter()
                .any(|i| i.method == "client" && i.receiver == Some("this".to_string()))
        );
        assert!(intents.iter().any(|i| i.method == "send"));
    }

    #[test]
    fn test_extract_decorator_references_angular_component() {
        let code = r#"
            @Component({
                selector: 'ngx-app',
                declarations: [AppComponent, UserComponent],
            })
            export class AppModule {}
        "#;
        let tree = crate::pipeline::parser::test_utils::parse_typescript_snippet(code)
            .expect("Failed to parse TypeScript code");

        let code_bytes = code.as_bytes();
        let mut intents: Vec<ReferenceIntent> = Vec::new();
        extract_decorator_references(tree.root_node(), code_bytes, &mut intents);

        // Should find Component, AppComponent, and UserComponent
        assert!(intents.iter().any(|i| {
            matches!(i, ReferenceIntent::TypeReference { type_name, .. } if type_name == "Component")
        }));
        assert!(intents.iter().any(|i| {
            matches!(i, ReferenceIntent::TypeReference { type_name, .. } if type_name == "AppComponent")
        }));
        assert!(intents.iter().any(|i| {
            matches!(i, ReferenceIntent::TypeReference { type_name, .. } if type_name == "UserComponent")
        }));
    }

    #[test]
    fn test_extract_type_references_constructor_params() {
        let code = r#"
            class AppComponent {
                constructor(private analytics: AnalyticsService, private seo: SeoService) {}
            }
        "#;
        let tree = crate::pipeline::parser::test_utils::parse_typescript_snippet(code)
            .expect("Failed to parse TypeScript code");

        let code_bytes = code.as_bytes();
        let mut intents: Vec<ReferenceIntent> = Vec::new();
        extract_type_references(tree.root_node(), code_bytes, &mut intents);

        // Should find AnalyticsService and SeoService
        assert!(intents.iter().any(|i| {
            matches!(i, ReferenceIntent::TypeReference { type_name, .. } if type_name == "AnalyticsService")
        }));
        assert!(intents.iter().any(|i| {
            matches!(i, ReferenceIntent::TypeReference { type_name, .. } if type_name == "SeoService")
        }));
    }

    #[test]
    fn test_extract_type_references_method_params_and_return() {
        let code = r#"
            class Service {
                process(data: DataService): ResultType {
                    return null;
                }
            }
        "#;
        let tree = crate::pipeline::parser::test_utils::parse_typescript_snippet(code)
            .expect("Failed to parse TypeScript code");

        let code_bytes = code.as_bytes();
        let mut intents: Vec<ReferenceIntent> = Vec::new();
        extract_type_references(tree.root_node(), code_bytes, &mut intents);

        // Should find DataService and ResultType
        assert!(intents.iter().any(|i| {
            matches!(i, ReferenceIntent::TypeReference { type_name, .. } if type_name == "DataService")
        }));
        assert!(intents.iter().any(|i| {
            matches!(i, ReferenceIntent::TypeReference { type_name, .. } if type_name == "ResultType")
        }));
    }

    #[test]
    fn test_extract_decorator_references_ngmodule() {
        let code = r#"
            @NgModule({
                declarations: [AppComponent],
                bootstrap: [AppComponent]
            })
            export class AppModule {}
        "#;
        let tree = crate::pipeline::parser::test_utils::parse_typescript_snippet(code)
            .expect("Failed to parse TypeScript code");

        let code_bytes = code.as_bytes();
        let mut intents: Vec<ReferenceIntent> = Vec::new();
        extract_decorator_references(tree.root_node(), code_bytes, &mut intents);

        // Should find NgModule and AppComponent (appears twice in the decorator)
        assert!(intents.iter().any(|i| {
            matches!(i, ReferenceIntent::TypeReference { type_name, .. } if type_name == "NgModule")
        }));
        let app_component_count = intents.iter().filter(|i| {
            matches!(i, ReferenceIntent::TypeReference { type_name, .. } if type_name == "AppComponent")
        }).count();
        assert!(
            app_component_count >= 2,
            "AppComponent should appear at least twice"
        );
    }

    #[test]
    fn test_extract_jsx_attributes_id() {
        let code = r#"function App() { return <div id="main-container">Hello</div>; }"#;
        let tree = crate::pipeline::parser::test_utils::parse_tsx_snippet(code)
            .expect("Failed to parse TSX code");

        fn find_jsx_opening_element(node: tree_sitter::Node) -> Option<tree_sitter::Node> {
            if node.kind() == "jsx_opening_element" {
                return Some(node);
            }
            let mut i = 0u32;
            while let Some(child) = node.child(i) {
                if let Some(found) = find_jsx_opening_element(child) {
                    return Some(found);
                }
                i += 1;
            }
            None
        }

        if let Some(jsx_elem) = find_jsx_opening_element(tree.root_node()) {
            let code_bytes = code.as_bytes();
            let attrs = extract_jsx_attributes(jsx_elem, code_bytes);
            assert_eq!(attrs.len(), 1);
            assert_eq!(attrs[0].0, "id");
            assert_eq!(attrs[0].1, "main-container");
        } else {
            panic!("No JSX opening element found");
        }
    }

    #[test]
    fn test_extract_jsx_attributes_classname() {
        let code =
            r#"function Button() { return <button className="btn primary">Click</button>; }"#;
        let tree = crate::pipeline::parser::test_utils::parse_tsx_snippet(code)
            .expect("Failed to parse TSX code");

        fn find_jsx_opening_element(node: tree_sitter::Node) -> Option<tree_sitter::Node> {
            if node.kind() == "jsx_opening_element" {
                return Some(node);
            }
            let mut i = 0u32;
            while let Some(child) = node.child(i) {
                if let Some(found) = find_jsx_opening_element(child) {
                    return Some(found);
                }
                i += 1;
            }
            None
        }

        if let Some(jsx_elem) = find_jsx_opening_element(tree.root_node()) {
            let code_bytes = code.as_bytes();
            let attrs = extract_jsx_attributes(jsx_elem, code_bytes);
            assert_eq!(attrs.len(), 1);
            assert_eq!(attrs[0].0, "className");
            assert_eq!(attrs[0].1, "btn primary");
        } else {
            panic!("No JSX opening element found");
        }
    }

    #[test]
    fn test_extract_jsx_attributes_multiple() {
        let code =
            r#"function Form() { return <input id="email-input" className="form-control" />; }"#;
        let tree = crate::pipeline::parser::test_utils::parse_tsx_snippet(code)
            .expect("Failed to parse TSX code");

        fn find_jsx_self_closing(node: tree_sitter::Node) -> Option<tree_sitter::Node> {
            if node.kind() == "jsx_self_closing_element" {
                return Some(node);
            }
            let mut i = 0u32;
            while let Some(child) = node.child(i) {
                if let Some(found) = find_jsx_self_closing(child) {
                    return Some(found);
                }
                i += 1;
            }
            None
        }

        if let Some(jsx_elem) = find_jsx_self_closing(tree.root_node()) {
            let code_bytes = code.as_bytes();
            let attrs = extract_jsx_attributes(jsx_elem, code_bytes);
            assert_eq!(attrs.len(), 2);

            // attrs may be in any order depending on AST traversal
            let has_id = attrs
                .iter()
                .any(|(name, val, _)| name == "id" && val == "email-input");
            let has_classname = attrs
                .iter()
                .any(|(name, val, _)| name == "className" && val == "form-control");

            assert!(has_id, "Should extract id attribute");
            assert!(has_classname, "Should extract className attribute");
        } else {
            panic!("No JSX self-closing element found");
        }
    }

    fn find_function_decl(root: tree_sitter::Node) -> Option<tree_sitter::Node> {
        if root.kind() == "function_declaration"
            || root.kind() == "arrow_function"
            || root.kind() == "lexical_declaration"
        {
            return Some(root);
        }
        let mut i = 0u32;
        while let Some(child) = root.child(i) {
            if let Some(found) = find_function_decl(child) {
                return Some(found);
            }
            i += 1;
        }
        None
    }

    #[test]
    fn test_extract_ref_intents_function_param_type() {
        let code = "function foo(ctx: MyContext) { }";
        let tree = crate::pipeline::parser::test_utils::parse_typescript_snippet(code)
            .expect("Failed to parse TypeScript code");

        if let Some(func_node) = find_function_decl(tree.root_node()) {
            let mut intents = Vec::new();
            extract_reference_intents_typescript(func_node, code.as_bytes(), &mut intents);
            let found = intents.iter().any(|i| {
                matches!(i, ReferenceIntent::TypeReference { type_name, .. } if type_name == "MyContext")
            });
            assert!(found, "Expected TypeReference MyContext, got {:?}", intents);
        }
    }

    #[test]
    fn test_extract_ref_intents_function_return_type() {
        let code = "function getData(): Promise<MyResult> { return null as any; }";
        let tree = crate::pipeline::parser::test_utils::parse_typescript_snippet(code)
            .expect("Failed to parse TypeScript code");

        if let Some(func_node) = find_function_decl(tree.root_node()) {
            let mut intents = Vec::new();
            extract_reference_intents_typescript(func_node, code.as_bytes(), &mut intents);
            let found = intents.iter().any(|i| {
                matches!(i, ReferenceIntent::TypeReference { type_name, .. } if type_name == "MyResult")
            });
            assert!(
                found,
                "Expected TypeReference MyResult in return type, got {:?}",
                intents
            );
        }
    }

    #[test]
    fn test_extract_ref_intents_const_type() {
        let code = "const x: MyType = getValue();";
        let tree = crate::pipeline::parser::test_utils::parse_typescript_snippet(code)
            .expect("Failed to parse TypeScript code");

        if let Some(decl_node) = find_function_decl(tree.root_node()) {
            let mut intents = Vec::new();
            extract_reference_intents_typescript(decl_node, code.as_bytes(), &mut intents);
            let found = intents.iter().any(|i| {
                matches!(i, ReferenceIntent::TypeReference { type_name, .. } if type_name == "MyType")
            });
            assert!(found, "Expected TypeReference MyType, got {:?}", intents);
        }
    }

    #[test]
    fn test_extract_value_ref_object_literal() {
        let code = "const REGISTRY = { ozone: HomebridgeOzoneServer }";
        let tree = crate::pipeline::parser::test_utils::parse_typescript_snippet(code)
            .expect("Failed to parse TypeScript code");
        let mut intents = Vec::new();
        extract_value_references_typescript(tree.root_node(), code.as_bytes(), &mut intents);
        let found = intents.iter().any(|i| {
            matches!(i, ReferenceIntent::ValueReference { value_name, .. } if value_name == "HomebridgeOzoneServer")
        });
        assert!(
            found,
            "Expected ValueReference HomebridgeOzoneServer, got {:?}",
            intents
        );
    }

    #[test]
    fn test_extract_value_ref_array_element() {
        let code = "const servers = [HomebridgeOzoneServer, HomebridgeAirQualityServer]";
        let tree = crate::pipeline::parser::test_utils::parse_typescript_snippet(code)
            .expect("Failed to parse TypeScript code");
        let mut intents = Vec::new();
        extract_value_references_typescript(tree.root_node(), code.as_bytes(), &mut intents);
        let found = intents.iter().any(|i| {
            matches!(i, ReferenceIntent::ValueReference { value_name, .. } if value_name == "HomebridgeOzoneServer")
        });
        assert!(
            found,
            "Expected ValueReference for array element, got {:?}",
            intents
        );
    }

    #[test]
    fn test_no_value_ref_for_global() {
        let code = "const x = Math.floor(Promise.resolve())";
        let tree = crate::pipeline::parser::test_utils::parse_typescript_snippet(code)
            .expect("Failed to parse TypeScript code");
        let mut intents = Vec::new();
        extract_value_references_typescript(tree.root_node(), code.as_bytes(), &mut intents);
        let has_math = intents.iter().any(|i| {
            matches!(i, ReferenceIntent::ValueReference { value_name, .. } if value_name == "Math")
        });
        let has_promise = intents.iter().any(|i| {
            matches!(i, ReferenceIntent::ValueReference { value_name, .. } if value_name == "Promise")
        });
        assert!(!has_math, "Math should be filtered as global");
        assert!(!has_promise, "Promise should be filtered as global");
    }

    #[test]
    fn test_collect_re_export_value() {
        let code = "export { HomebridgeOzoneServer } from './foo.js'";
        let tree = crate::pipeline::parser::test_utils::parse_typescript_snippet(code)
            .expect("Failed to parse TypeScript code");
        let mut intents = Vec::new();
        collect_all_reference_intents_typescript(tree.root_node(), code.as_bytes(), &mut intents);
        let found = intents.iter().any(|(i, _)| {
            matches!(i, ReferenceIntent::ValueReference { value_name, .. } if value_name == "HomebridgeOzoneServer")
        });
        assert!(
            found,
            "Expected ValueReference from re-export, got {:?}",
            intents
        );
    }

    #[test]
    fn test_collect_re_export_type() {
        let code = "export type { HomebridgeOzoneServer } from './foo.js'";
        let tree = crate::pipeline::parser::test_utils::parse_typescript_snippet(code)
            .expect("Failed to parse TypeScript code");
        let mut intents = Vec::new();
        collect_all_reference_intents_typescript(tree.root_node(), code.as_bytes(), &mut intents);
        let found = intents.iter().any(|(i, _)| {
            matches!(i, ReferenceIntent::TypeReference { type_name, .. } if type_name == "HomebridgeOzoneServer")
        });
        assert!(
            found,
            "Expected TypeReference from re-export type, got {:?}",
            intents
        );
    }

    #[test]
    fn test_collect_re_export_multi() {
        let code = "export { A, B } from './bar.js'";
        let tree = crate::pipeline::parser::test_utils::parse_typescript_snippet(code)
            .expect("Failed to parse TypeScript code");
        let mut intents = Vec::new();
        collect_all_reference_intents_typescript(tree.root_node(), code.as_bytes(), &mut intents);
        let has_a = intents.iter().any(|(i, _)| {
            matches!(i, ReferenceIntent::ValueReference { value_name, .. } if value_name == "A")
        });
        let has_b = intents.iter().any(|(i, _)| {
            matches!(i, ReferenceIntent::ValueReference { value_name, .. } if value_name == "B")
        });
        assert!(
            has_a && has_b,
            "Expected ValueReferences for both A and B, got {:?}",
            intents
        );
    }
}
