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
/// - Callbacks passed as arguments: `app.use(this.handler)` -> records call to handler
/// - Bind calls: `this.method.bind(this)` -> records call to method
/// - Property/getter access: `this.client`, `this.field` -> records access to property/getter
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
#[expect(
    clippy::cognitive_complexity,
    reason = "function is verbose but correct — extraction deferred"
)]
pub(crate) fn extract_single_call_intent_javascript(
    node: Node<'_>,
    source: &[u8],
) -> Vec<CallIntent> {
    let mut intents = Vec::new();

    if node.kind() == "call_expression" {
        let line = node.start_position().row + 1;

        let mut method_name: Option<String> = None;
        let mut receiver: Option<String> = None;

        let mut child = node.child(0);
        let mut is_bind_call = false;

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

        if let Some(method) = method_name {
            if is_bind_call {
                if let Some(receiver) = receiver
                    && let Some(last_part) = receiver.split('.').next_back()
                {
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
            } else {
                intents.push(CallIntent {
                    method,
                    receiver,
                    line,
                    arg_count: None,
                });
            }
        }

        // Also scan arguments for callback references
        extract_callback_arguments(node, source, &mut intents, line);
    } else if node.kind() == "new_expression" {
        let line = node.start_position().row + 1;
        if let Some(name) = extract_new_expression_name(node, source) {
            intents.push(CallIntent {
                method: name,
                receiver: None,
                line,
                arg_count: None,
            });
        }
    } else if node.kind() == "jsx_self_closing_element" || node.kind() == "jsx_opening_element" {
        // JSX component invocation
        extract_jsx_component_invocation(node, source, &mut intents);
    } else if node.kind() == "member_expression" {
        // Detect property/getter access via `this.property`
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
