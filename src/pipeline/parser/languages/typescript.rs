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

/// Extract class inheritance (extends/implements) from TypeScript class AST.
/// Manually traverses the class declaration node to find extends and implements clauses.
pub(crate) fn extract_class_inheritance(
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

/// Extract call intents from a SINGLE node without recursive descent.
///
/// This is the non-recursive version of `extract_call_intents_typescript`,
/// designed to be used in contexts where the caller already handles tree traversal
/// (e.g., the fallback pass in `collect_all_reference_intents_typescript`).
///
/// By extracting only the current node's intent, we avoid double-processing children
/// that would cause duplicate CALLS with incorrect byte_pos/line attribution.
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
                });
                break;
            }
            child = c.next_sibling();
        }
    } else if node.kind() == "jsx_self_closing_element" || node.kind() == "jsx_opening_element" {
        // JSX component invocation (e.g., <ChartToolbar />, <Sheet.Content />)
        extract_jsx_component_invocation(node, source, &mut intents);
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
