use super::imports::{collect_import_intents_javascript, collect_require_destructure_intents};
use super::jsx::extract_jsx_component_invocation;
use crate::models::{CallIntent, ReferenceIntent};
use crate::pipeline::parser::utils::{
    extract_identifiers_from_decorator, extract_new_expression_name, is_capitalized, node_text,
};
use tree_sitter::Node;

/// Recursively extract all reference intents from JavaScript, returning (intent, byte_pos) pairs.
pub(crate) fn collect_all_reference_intents_javascript(
    node: Node<'_>,
    source: &[u8],
    intents: &mut Vec<(ReferenceIntent, usize)>,
) {
    let byte_pos = node.start_byte();
    let line = node.start_position().row + 1;

    match node.kind() {
        "call_expression" | "new_expression" => {
            // Use non-recursive extraction to avoid double-processing children
            let call_intents = extract_single_call_intent_javascript(node, source);
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
            // Extract decorator references (e.g., @Component({ declarations: [AppComponent] }))
            let mut decorator_refs = Vec::new();
            extract_identifiers_from_decorator(node, source, &mut decorator_refs, line);
            for ref_intent in decorator_refs {
                intents.push((ref_intent, byte_pos));
            }
        }
        "import_statement" => {
            collect_import_intents_javascript(node, source, intents, byte_pos, line, false);
        }
        "lexical_declaration" | "variable_declaration" => {
            collect_require_destructure_intents(node, source, intents, byte_pos, line);
        }
        _ => {}
    }

    // Recursively process children
    let mut child = node.child(0);
    while let Some(c) = child {
        collect_all_reference_intents_javascript(c, source, intents);
        child = c.next_sibling();
    }
}

/// Extract reference intents from a JavaScript function/method body (wrapper for backward compatibility).
pub(crate) fn extract_reference_intents_javascript(
    node: Node<'_>,
    source: &[u8],
    intents: &mut Vec<ReferenceIntent>,
) {
    let mut call_intents = Vec::new();
    extract_call_intents_javascript(node, source, &mut call_intents);
    for call in call_intents {
        intents.push(ReferenceIntent::Call {
            method: call.method,
            receiver: call.receiver,
            line: call.line,
            arg_count: call.arg_count,
        });
    }

    extract_member_accesses_javascript(node, source, intents);
    // Also extract enum/static member usages (e.g., ClassName.STATIC, Constants.VALUE)
    extract_enum_usages_javascript(node, source, intents);
}

/// Extract call expression call intents from a JavaScript function/method body.
///
/// Handles:
/// - Direct calls: `method()`, `this.method()`
/// - Member calls: `obj.method()`, `this.service.method()`
/// - New expressions: `new MyClass()`
/// - JSX components: `<ChartToolbar />`, `<Sheet.Content />`
/// - Callbacks passed as arguments: `app.use(auth.handler)` -> records call to handler
/// - Bind calls: `this.method.bind(this)` -> records call to method
///
/// `this.<member>` reads, writes and arguments are member accesses, not
/// calls (see [`extract_member_accesses_javascript`]).
fn extract_call_intents_javascript(node: Node<'_>, source: &[u8], intents: &mut Vec<CallIntent>) {
    intents.extend(extract_single_call_intent_javascript(node, source));

    let mut child = node.child(0);
    while let Some(c) = child {
        extract_call_intents_javascript(c, source, intents);
        child = c.next_sibling();
    }
}

/// Extract call intents from a SINGLE node without recursive descent.
///
/// This is the non-recursive version of `extract_call_intents_javascript`,
/// designed to be used in contexts where the caller already handles tree traversal.
fn scan_callee(node: Node<'_>, source: &[u8]) -> (Option<String>, Option<String>, bool) {
    let mut method_name: Option<String> = None;
    let mut receiver: Option<String> = None;
    let mut is_bind_call = false;

    let mut child = node.child(0);
    while let Some(c) = child {
        if c.kind() == "member_expression" {
            if let Some(property_node) = c.child_by_field_name("property") {
                let prop_text = node_text(property_node, source);
                if prop_text == "bind" {
                    is_bind_call = true;
                }
                method_name = Some(prop_text);
            }

            if let Some(object_node) = c.child_by_field_name("object") {
                receiver = Some(node_text(object_node, source));
            }
        } else if c.kind() == "identifier" {
            method_name = Some(node_text(c, source));
        }
        child = c.next_sibling();
    }

    (method_name, receiver, is_bind_call)
}

fn split_bind_receiver(receiver: &str) -> Option<(String, Option<String>)> {
    let last_part = receiver.split('.').next_back()?;
    let rec_prefix = if receiver.contains('.') {
        receiver.split('.').next().map(|s| s.to_string())
    } else {
        Some("this".to_string())
    };
    Some((last_part.to_string(), rec_prefix))
}

fn bind_call_intent(receiver: &str, line: usize) -> Option<CallIntent> {
    let (method, rec) = split_bind_receiver(receiver)?;
    Some(CallIntent {
        method,
        receiver: rec,
        line,
        arg_count: None,
    })
}

/// Number of arguments of a `call_expression`, comments excluded. `None`
/// when a spread argument (`...xs`) makes the count unknown, or for a tagged
/// template (`` tag`...` ``). Recorded for TypeScript and JavaScript alike; the
/// resolver decides whether arity is binding for the language.
fn argument_count(call: Node<'_>) -> Option<usize> {
    let args = call.child_by_field_name("arguments")?;
    if args.kind() != "arguments" {
        return None;
    }
    let mut cursor = args.walk();
    let mut count = 0;
    for arg in args.named_children(&mut cursor) {
        match arg.kind() {
            "comment" => {}
            "spread_element" => return None,
            _ => count += 1,
        }
    }
    Some(count)
}

fn call_expression_intents(node: Node<'_>, source: &[u8], intents: &mut Vec<CallIntent>) {
    let line = node.start_position().row + 1;
    let (method_name, receiver, is_bind_call) = scan_callee(node, source);

    if let Some(method) = method_name {
        if is_bind_call {
            if let Some(rec) = receiver
                && let Some(intent) = bind_call_intent(&rec, line)
            {
                intents.push(intent);
            }
        } else {
            intents.push(CallIntent {
                method,
                receiver,
                line,
                arg_count: argument_count(node),
            });
        }
    }

    extract_callback_arguments(node, source, intents, line);
}

fn is_this_member(node: Node<'_>, source: &[u8]) -> bool {
    node.kind() == "member_expression"
        && node
            .child_by_field_name("object")
            .is_some_and(|object| node_text(object, source) == "this")
}

/// A `this.<member>` read, write or argument that is not called (issue #49).
/// The callee of a call (`this.m()`) is recorded by the call path instead.
/// The resolver turns an access to a method or getter into a call, so a
/// callback argument (`app.use(this.handler)`) still produces a CALLS edge
/// while a field argument (`log(this.items)`) produces a reference.
fn this_member_access(node: Node<'_>, source: &[u8]) -> Option<ReferenceIntent> {
    if !is_this_member(node, source) {
        return None;
    }
    let is_callee = node.parent().is_some_and(|p| {
        p.kind() == "call_expression" && p.child_by_field_name("function") == Some(node)
    });
    if is_callee {
        return None;
    }
    let property = node.child_by_field_name("property")?;
    Some(ReferenceIntent::MemberAccess {
        member: node_text(property, source),
        receiver: "this".to_string(),
        line: node.start_position().row + 1,
    })
}

/// Every `this.<member>` read or write in the subtree of `node`.
pub(crate) fn extract_member_accesses_javascript(
    node: Node<'_>,
    source: &[u8],
    intents: &mut Vec<ReferenceIntent>,
) {
    intents.extend(this_member_access(node, source));
    let mut child = node.child(0);
    while let Some(c) = child {
        extract_member_accesses_javascript(c, source, intents);
        child = c.next_sibling();
    }
}

pub(crate) fn extract_single_call_intent_javascript(
    node: Node<'_>,
    source: &[u8],
) -> Vec<CallIntent> {
    let mut intents = Vec::new();

    match node.kind() {
        "call_expression" => {
            call_expression_intents(node, source, &mut intents);
        }
        "new_expression" => {
            let line = node.start_position().row + 1;
            if let Some(name) = extract_new_expression_name(node, source) {
                intents.push(CallIntent {
                    method: name,
                    receiver: None,
                    line,
                    arg_count: None,
                });
            }
        }
        "jsx_self_closing_element" | "jsx_opening_element" => {
            extract_jsx_component_invocation(node, source, &mut intents);
        }
        _ => {}
    }

    intents
}

/// `true` when a binding pattern declares `name`. Default values and object
/// pattern keys are uses, not bindings, so only the left/value side counts.
fn binding_pattern_contains(node: Node<'_>, name: &str, source: &[u8]) -> bool {
    match node.kind() {
        "identifier" | "shorthand_property_identifier_pattern" => node_text(node, source) == name,
        "pair_pattern" => node
            .child_by_field_name("value")
            .is_some_and(|value| binding_pattern_contains(value, name, source)),
        "assignment_pattern" => node
            .child_by_field_name("left")
            .is_some_and(|left| binding_pattern_contains(left, name, source)),
        _ => {
            let mut cursor = node.walk();
            node.named_children(&mut cursor)
                .any(|child| binding_pattern_contains(child, name, source))
        }
    }
}

fn variable_declaration_binds(node: Node<'_>, name: &str, source: &[u8]) -> bool {
    if !matches!(node.kind(), "lexical_declaration" | "variable_declaration") {
        return false;
    }
    let mut cursor = node.walk();
    node.named_children(&mut cursor)
        .filter(|child| child.kind() == "variable_declarator")
        .filter_map(|declarator| declarator.child_by_field_name("name"))
        .any(|pattern| binding_pattern_contains(pattern, name, source))
}

/// Declarations introduced by one lexical scope enclosing an identifier.
fn scope_binds(node: Node<'_>, name: &str, source: &[u8]) -> bool {
    match node.kind() {
        "statement_block" => {
            let mut cursor = node.walk();
            node.named_children(&mut cursor)
                .any(|statement| variable_declaration_binds(statement, name, source))
        }
        "for_statement" => node
            .child_by_field_name("initializer")
            .is_some_and(|initializer| variable_declaration_binds(initializer, name, source)),
        "for_in_statement" => node
            .child_by_field_name("left")
            .is_some_and(|left| binding_pattern_contains(left, name, source)),
        "catch_clause" => node
            .child_by_field_name("parameter")
            .is_some_and(|parameter| binding_pattern_contains(parameter, name, source)),
        _ => false,
    }
}

fn is_function_node(node: Node<'_>) -> bool {
    matches!(
        node.kind(),
        "function_declaration"
            | "function_expression"
            | "generator_function_declaration"
            | "generator_function"
            | "arrow_function"
            | "method_definition"
    )
}

/// `true` when `identifier` is bound by a parameter or local declaration of
/// an enclosing function. Module-level declarations (`program`) and imports
/// are excluded: `app.use(authHandler)` must still record a real callback.
fn is_function_local(identifier: Node<'_>, name: &str, source: &[u8]) -> bool {
    let mut current = identifier.parent();
    while let Some(node) = current {
        if scope_binds(node, name, source) {
            return true;
        }
        if is_function_node(node) {
            let binds_parameter = node
                .child_by_field_name("parameters")
                .or_else(|| node.child_by_field_name("parameter"))
                .is_some_and(|parameters| binding_pattern_contains(parameters, name, source));
            if binds_parameter {
                return true;
            }
        }
        current = node.parent();
    }
    false
}

/// Extract callback arguments from a call expression.
///
/// Detects method references passed as arguments, e.g.:
/// - `app.use(auth.handler)` -> records call to handler
/// - `emitter.on('event', onEvent)` -> records call to onEvent
///
/// `this.<member>` arguments (`app.use(this.handler)`) are member accesses
/// instead (see [`this_member_access`]).
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
            // Look for member_expression arguments (e.g., obj.method). A
            // `this.<member>` argument is a member access (issue #49).
            if a.kind() == "member_expression" && !is_this_member(a, source) {
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
                    && !is_function_local(a, &name, source)
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

/// Check if a string is a JavaScript reserved keyword.
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
            | "function"
            | "new"
            | "delete"
            | "typeof"
            | "instanceof"
            | "in"
            | "of"
            | "static"
            | "interface"
            | "enum"
            | "type"
            | "public"
            | "private"
            | "protected"
            | "readonly"
            | "abstract"
            | "extends"
            | "implements"
            | "declare"
    )
}

/// Extract enum and static member usages from a JavaScript node (e.g., ClassName.STATIC).
///
/// Recursively searches for member_expression nodes where the object is a capitalized identifier,
/// which typically represents class static member access patterns.
pub(crate) fn extract_enum_usages_javascript(
    node: Node<'_>,
    source: &[u8],
    intents: &mut Vec<ReferenceIntent>,
) {
    if node.kind() == "member_expression" {
        // member_expression has: object . property
        // We only want to capture if object is a capitalized identifier (class name)
        if let Some(object_node) = node.child_by_field_name("object")
            && object_node.kind() == "identifier"
        {
            let obj_text = node_text(object_node, source);
            // Check if it starts with capital letter (typical of classes)
            if is_capitalized(&obj_text) {
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
        extract_enum_usages_javascript(c, source, intents);
        child = c.next_sibling();
    }
}
