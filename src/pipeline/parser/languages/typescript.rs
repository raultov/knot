use crate::models::{CallIntent, ReferenceIntent};
use crate::pipeline::parser::utils::{
    extract_identifiers_from_decorator, extract_type_references, is_capitalized, node_text,
};
use tree_sitter::Node;

pub(crate) use super::javascript::extract_enum_usages_javascript as extract_enum_usages_typescript;
pub(crate) use super::javascript::extract_jsx_component_invocation;
pub(crate) use super::javascript::extract_single_call_intent_javascript as extract_single_call_intent_typescript;

// Test-only consumer; cfg(test) keeps the lib profile free of unused-import warnings.
#[cfg(test)]
pub(crate) use super::javascript::extract_callback_arguments;

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
            if is_capitalized(&type_name) {
                intents.push((ReferenceIntent::TypeReference { type_name, line }, byte_pos));
            }
        }
        "export_statement" => {
            extract_export_statement_intents(node, source, intents);
        }
        "import_statement" => {
            let is_type_import = node.children(&mut node.walk()).any(|c| c.kind() == "type");
            super::javascript::collect_import_intents_javascript(
                node,
                source,
                intents,
                byte_pos,
                line,
                is_type_import,
            );
        }
        "identifier" => {
            let name = node_text(node, source);
            if is_capitalized(&name)
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

fn extract_export_statement_intents(
    node: Node<'_>,
    source: &[u8],
    intents: &mut Vec<(ReferenceIntent, usize)>,
) {
    // Handle re-exports: `export { A, B } from './x'`
    if node.child_by_field_name("source").is_none() {
        return;
    }

    let is_type_export = node.children(&mut node.walk()).any(|c| c.kind() == "type");
    let mut clause_child = node.child(0);

    while let Some(c) = clause_child {
        if matches!(c.kind(), "export_clause" | "export_type_clause") {
            process_export_clause_specifiers(c, source, intents, is_type_export);
        }
        clause_child = c.next_sibling();
    }
}

fn process_export_clause_specifiers(
    clause_node: Node<'_>,
    source: &[u8],
    intents: &mut Vec<(ReferenceIntent, usize)>,
    is_type_export: bool,
) {
    let byte_pos = clause_node.start_byte();
    let line = clause_node.start_position().row + 1;
    let mut spec_child = clause_node.child(0);
    while let Some(spec) = spec_child {
        if spec.kind() == "export_specifier"
            && let Some(name_node) = spec.child_by_field_name("name")
        {
            let name = node_text(name_node, source);
            if is_capitalized(&name) {
                if is_type_export || clause_node.kind() == "export_type_clause" {
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
        if is_capitalized(&name)
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
#[expect(
    clippy::excessive_nesting,
    reason = "function is verbose but correct — extraction deferred"
)]
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
    if node.kind() == "new_expression" {
        // TS-specific: also handles type_identifier (e.g., new MyGeneric<string>())
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
    } else {
        // Delegate call_expression, jsx, member_expression to shared JS implementation
        intents.extend(extract_single_call_intent_typescript(node, source));
    }

    // Recursively process children
    let mut child = node.child(0);
    while let Some(c) = child {
        extract_call_intents_typescript(c, source, intents);
        child = c.next_sibling();
    }
}

// ── Alias extraction (import / export default) ──────────────────

/// Scan the AST for import statements and return (local_name, module_path, original_export_name, is_renamed).
/// original_export_name is Some for named imports, None for default/namespace.
/// is_renamed is true when `import { X as Y }` creates a new local name.
pub(crate) fn scan_import_module_aliases(
    root: Node<'_>,
    source: &[u8],
) -> Vec<(String, String, Option<String>, bool)> {
    let mut aliases = Vec::new();

    #[expect(
        clippy::excessive_nesting,
        reason = "function is verbose but correct — extraction deferred"
    )]
    fn walk(
        node: Node<'_>,
        source: &[u8],
        aliases: &mut Vec<(String, String, Option<String>, bool)>,
    ) {
        if node.kind() == "import_statement" {
            let source_node = node.child_by_field_name("source");
            let module_path = source_node.map(|n| {
                let raw = node_text(n, source);
                raw.trim_matches(|c| c == '\'' || c == '"' || c == '`')
                    .to_string()
            });

            if let Some(module_path) = module_path {
                let mut child = node.child(0);
                while let Some(c) = child {
                    if c.kind() == "import_clause" {
                        let mut inner = c.child(0);
                        while let Some(ci) = inner {
                            match ci.kind() {
                                "named_imports" => {
                                    let mut spec_child = ci.child(0);
                                    while let Some(spec) = spec_child {
                                        if spec.kind() == "import_specifier" {
                                            let alias_node = spec.child_by_field_name("alias");
                                            let original = spec.child_by_field_name("name");
                                            let local = alias_node.or(original);
                                            let original_name =
                                                original.map(|n| node_text(n, source));
                                            let is_renamed = alias_node.is_some();
                                            if let Some(name_node) = local {
                                                let name = node_text(name_node, source);
                                                aliases.push((
                                                    name,
                                                    module_path.clone(),
                                                    original_name,
                                                    is_renamed,
                                                ));
                                            }
                                        }
                                        spec_child = spec.next_sibling();
                                    }
                                }
                                "namespace_import" => {
                                    if let Some(name_node) = ci.child_by_field_name("name") {
                                        let name = node_text(name_node, source);
                                        aliases.push((name, module_path.clone(), None, true));
                                    }
                                }
                                "identifier" => {
                                    let name = node_text(ci, source);
                                    aliases.push((name, module_path.clone(), None, true));
                                }
                                _ => {}
                            }
                            inner = ci.next_sibling();
                        }
                    }
                    child = c.next_sibling();
                }
            }
        }
        let mut child = node.child(0);
        while let Some(c) = child {
            walk(c, source, aliases);
            child = c.next_sibling();
        }
    }

    walk(root, source, &mut aliases);
    aliases
}

/// Scan the AST for `export default X` and return the target name.
pub(crate) fn scan_default_export_target(root: Node<'_>, source: &[u8]) -> Option<String> {
    fn walk(node: Node<'_>, source: &[u8]) -> Option<String> {
        if node.kind() == "export_statement"
            && node
                .children(&mut node.walk())
                .any(|c| c.kind() == "default")
        {
            // Find the value child that is an identifier
            let mut child = node.child(0);
            while let Some(c) = child {
                if c.kind() == "identifier" {
                    return Some(node_text(c, source));
                }
                child = c.next_sibling();
            }
        }
        let mut child = node.child(0);
        while let Some(c) = child {
            if let Some(result) = walk(c, source) {
                return Some(result);
            }
            child = c.next_sibling();
        }
        None
    }
    walk(root, source)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pipeline::parser::test_utils::{
        assert_extends, assert_implements, find_call_expression, find_class_declaration,
        find_interface_declaration, find_member_expression, find_new_expression,
    };
    use crate::pipeline::parser::utils::extract_decorator_references;

    #[test]
    fn test_extract_jsx_component_invocation_simple() {
        crate::pipeline::parser::test_utils::assert_jsx_component_invocation(
            "function render() { return <ChartToolbar />; }",
            crate::pipeline::parser::test_utils::parse_tsx_snippet,
            "ChartToolbar",
            None,
        );
    }

    #[test]
    fn test_extract_jsx_component_invocation_namespaced() {
        crate::pipeline::parser::test_utils::assert_jsx_component_invocation(
            "function render() { return <Sheet.Content />; }",
            crate::pipeline::parser::test_utils::parse_tsx_snippet,
            "Content",
            Some("Sheet"),
        );
    }

    #[test]
    fn test_extract_single_call_intent_typescript_simple() {
        let code = "function test() { method(); }";
        let tree = crate::pipeline::parser::test_utils::parse_typescript_snippet(code)
            .expect("Failed to parse TypeScript code");

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

        if let Some(new_expr) = find_new_expression(tree.root_node()) {
            let code_bytes = code.as_bytes();
            let intents = extract_single_call_intent_typescript(new_expr, code_bytes);
            assert!(!intents.is_empty());
            assert_eq!(intents[0].method, "MyClass");
            assert!(intents[0].receiver.is_none());
        }
    }

    fn extract_inheritance_from_code(code: &str) -> Vec<ReferenceIntent> {
        let tree = crate::pipeline::parser::test_utils::parse_typescript_snippet(code)
            .expect("Failed to parse TypeScript code");
        let node = find_class_declaration(tree.root_node())
            .or_else(|| find_interface_declaration(tree.root_node()))
            .expect("No class or interface declaration found");
        let mut intents = Vec::new();
        extract_class_inheritance(node, code.as_bytes(), &mut intents);
        intents
    }

    #[test]
    fn test_extract_class_inheritance_extends() {
        let intents = extract_inheritance_from_code("class Child extends Parent { }");
        assert_extends(&intents, "Parent");
    }

    #[test]
    fn test_extract_class_inheritance_implements() {
        let intents = extract_inheritance_from_code("class Child implements IFoo, IBar { }");
        assert_implements(&intents, "IFoo");
        assert_implements(&intents, "IBar");
    }

    #[test]
    fn test_extract_class_inheritance_extends_and_implements() {
        let intents =
            extract_inheritance_from_code("class Child extends Parent implements IFoo { }");
        assert_extends(&intents, "Parent");
        assert_implements(&intents, "IFoo");
    }

    #[test]
    fn test_extract_class_inheritance_qualified_extends() {
        let intents = extract_inheritance_from_code("class Child extends NS.Parent { }");
        assert_extends(&intents, "Parent");
    }

    #[test]
    fn test_extract_class_inheritance_generic_extends() {
        let intents = extract_inheritance_from_code("class Child extends Generic<T> { }");
        assert_extends(&intents, "Generic");
    }

    #[test]
    fn test_extract_interface_extends_simple() {
        let intents = extract_inheritance_from_code("interface IB extends IA { }");
        assert_extends(&intents, "IA");
    }

    #[test]
    fn test_extract_interface_extends_multiple() {
        let intents = extract_inheritance_from_code("interface IC extends IA, IB { }");
        assert_extends(&intents, "IA");
        assert_extends(&intents, "IB");
    }

    #[test]
    fn test_extract_callback_arguments_member_expression() {
        let code = "function test() { app.use(this.handler); }";
        let tree = crate::pipeline::parser::test_utils::parse_typescript_snippet(code)
            .expect("Failed to parse TypeScript code");

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
        crate::pipeline::parser::test_utils::assert_jsx_attribute(
            r#"function App() { return <div id="main-container">Hello</div>; }"#,
            crate::pipeline::parser::test_utils::parse_tsx_snippet,
            "id",
            "main-container",
        );
    }

    #[test]
    fn test_extract_jsx_attributes_classname() {
        crate::pipeline::parser::test_utils::assert_jsx_attribute(
            r#"function Button() { return <button className="btn primary">Click</button>; }"#,
            crate::pipeline::parser::test_utils::parse_tsx_snippet,
            "className",
            "btn primary",
        );
    }

    fn find_function_decl(root: Node) -> Option<Node> {
        crate::pipeline::parser::test_utils::find_function_decl(root)
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

    #[test]
    fn test_print_import_ast() {
        let code = "import { MyTsTarget as MyTsAlias } from './alias_target_ts';";
        let tree = crate::pipeline::parser::test_utils::parse_typescript_snippet(code)
            .expect("Failed to parse TypeScript code");
        println!("AST: {}", tree.root_node().to_sexp());
        let aliases = scan_import_module_aliases(tree.root_node(), code.as_bytes());
        println!("Aliases: {:?}", aliases);
        assert_eq!(
            aliases,
            vec![(
                "MyTsAlias".to_string(),
                "./alias_target_ts".to_string(),
                Some("MyTsTarget".to_string()),
                true
            )]
        );
    }

    #[test]
    fn test_import_named_specifier_emits_ref() {
        let code = "import { Foo } from './types';";
        let tree = crate::pipeline::parser::test_utils::parse_typescript_snippet(code)
            .expect("Failed to parse TypeScript code");
        let mut intents = Vec::new();
        collect_all_reference_intents_typescript(tree.root_node(), code.as_bytes(), &mut intents);
        let has_foo = intents.iter().any(|(i, _)| match i {
            ReferenceIntent::ValueReference { value_name, .. } => value_name == "Foo",
            _ => false,
        });
        assert!(
            has_foo,
            "Should emit ValueReference for Foo from named import, got: {:?}",
            intents
        );
    }

    #[test]
    fn test_import_aliased_specifier_uses_original_name() {
        let code = "import { Foo as Bar } from './types';";
        let tree = crate::pipeline::parser::test_utils::parse_typescript_snippet(code)
            .expect("Failed to parse TypeScript code");
        let mut intents = Vec::new();
        collect_all_reference_intents_typescript(tree.root_node(), code.as_bytes(), &mut intents);
        let has_foo = intents.iter().any(|(i, _)| match i {
            ReferenceIntent::ValueReference { value_name, .. } => value_name == "Foo",
            _ => false,
        });
        let has_bar = intents.iter().any(|(i, _)| match i {
            ReferenceIntent::ValueReference { value_name, .. } => value_name == "Bar",
            _ => false,
        });
        assert!(
            has_foo,
            "Should emit ValueReference for Foo (original name), got: {:?}",
            intents
        );
        assert!(
            !has_bar,
            "Should NOT emit ValueReference for Bar (alias), got: {:?}",
            intents
        );
    }

    #[test]
    fn test_import_default_emits_by_case() {
        let code = "import MyComponent from './comp';";
        let tree = crate::pipeline::parser::test_utils::parse_typescript_snippet(code)
            .expect("Failed to parse TypeScript code");
        let mut intents = Vec::new();
        collect_all_reference_intents_typescript(tree.root_node(), code.as_bytes(), &mut intents);
        let has_component = intents.iter().any(|(i, _)| match i {
            ReferenceIntent::ValueReference { value_name, .. } => value_name == "MyComponent",
            _ => false,
        });
        assert!(
            has_component,
            "Should emit ValueReference for MyComponent from default import, got: {:?}",
            intents
        );
    }

    #[test]
    fn test_import_side_effect_emits_nothing() {
        let code = "import './polyfill';";
        let tree = crate::pipeline::parser::test_utils::parse_typescript_snippet(code)
            .expect("Failed to parse TypeScript code");
        let mut intents = Vec::new();
        collect_all_reference_intents_typescript(tree.root_node(), code.as_bytes(), &mut intents);
        let import_refs: Vec<_> = intents
            .iter()
            .filter(|(i, _)| {
                matches!(
                    i,
                    ReferenceIntent::ValueReference { .. } | ReferenceIntent::TypeReference { .. }
                )
            })
            .collect();
        assert!(
            import_refs.is_empty(),
            "Side-effect import should emit no references, got: {:?}",
            import_refs
        );
    }
}
