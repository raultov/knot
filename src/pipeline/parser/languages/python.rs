use crate::models::{CallIntent, EntityKind, ReferenceIntent};
use crate::pipeline::parser::utils::node_text;
use tree_sitter::Node;

pub(crate) fn handle_python_capture(
    cap_name: &str,
    text: &str,
    node: Node<'_>,
) -> Option<(String, EntityKind, usize)> {
    let start_line = node.start_position().row + 1;

    match cap_name {
        "python.class.name" => Some((text.to_string(), EntityKind::PythonClass, start_line)),
        "python.function.name" => {
            let kind = if is_inside_class_body(node) {
                EntityKind::PythonMethod
            } else {
                EntityKind::PythonFunction
            };
            Some((text.to_string(), kind, start_line))
        }
        "python.constant.name" => Some((text.to_string(), EntityKind::PythonConstant, start_line)),
        _ => None,
    }
}

pub(crate) fn is_inside_class_body(node: Node<'_>) -> bool {
    let mut current = node.parent();
    while let Some(parent) = current {
        if parent.kind() == "class_definition" {
            return true;
        }
        current = parent.parent();
    }
    false
}

pub(crate) fn extract_reference_intents_python(
    node: Node<'_>,
    source: &[u8],
    intents: &mut Vec<ReferenceIntent>,
) {
    let mut call_intents = Vec::new();
    extract_call_intents_python(node, source, &mut call_intents);
    for call in call_intents {
        intents.push(ReferenceIntent::Call {
            method: call.method,
            receiver: call.receiver,
            line: call.line,
            arg_count: call.arg_count,
        });
    }

    let mut import_intents = Vec::new();
    extract_import_intents_python(node, source, &mut import_intents);
    for import in import_intents {
        intents.push(import);
    }

    let mut value_ref_intents = Vec::new();
    extract_value_references_python(node, source, &mut value_ref_intents);
    for value_ref in value_ref_intents {
        intents.push(value_ref);
    }
}

pub(crate) fn extract_call_intents_python(
    node: Node<'_>,
    source: &[u8],
    intents: &mut Vec<CallIntent>,
) {
    if node.kind() == "call" {
        let line = node.start_position().row + 1;

        let function_node = node.child_by_field_name("function");

        if let Some(func) = function_node {
            match func.kind() {
                "identifier" => {
                    let method_name = node_text(func, source);
                    intents.push(CallIntent {
                        method: method_name,
                        receiver: None,
                        line,
                        arg_count: None,
                    });
                }
                "attribute" => {
                    let mut method_name: Option<String> = None;
                    let mut receiver: Option<String> = None;

                    if let Some(attr_node) = func.child_by_field_name("attribute") {
                        method_name = Some(node_text(attr_node, source));
                    }
                    if let Some(obj_node) = func.child_by_field_name("object") {
                        receiver = Some(node_text(obj_node, source));
                    }

                    if let Some(method) = method_name {
                        intents.push(CallIntent {
                            method,
                            receiver,
                            line,
                            arg_count: None,
                        });
                    }
                }
                _ => {}
            }
        }
    } else if node.kind() == "print_statement" {
        let line = node.start_position().row + 1;
        intents.push(CallIntent {
            method: "print".to_string(),
            receiver: None,
            line,
            arg_count: None,
        });
    }

    let mut child = node.child(0);
    while let Some(c) = child {
        extract_call_intents_python(c, source, intents);
        child = c.next_sibling();
    }
}

pub(crate) fn extract_import_intents_python(
    node: Node<'_>,
    source: &[u8],
    intents: &mut Vec<ReferenceIntent>,
) {
    let line = node.start_position().row + 1;

    if node.kind() == "import_statement" {
        let mut child = node.child(0);
        while let Some(c) = child {
            if c.kind() == "dotted_name" || c.kind() == "identifier" {
                collect_import_names(c, source, intents, line);
            }
            child = c.next_sibling();
        }
    } else if node.kind() == "import_from_statement" {
        for i in 0..node.child_count() {
            if let Some(child) = node.child(i as u32) {
                match child.kind() {
                    "dotted_name" => {
                        let name = node_text(child, source);
                        intents.push(ReferenceIntent::TypeReference {
                            type_name: name,
                            line,
                        });
                    }
                    "aliased_import" => {
                        if let Some(alias_node) = child.child_by_field_name("alias") {
                            let alias_name = node_text(alias_node, source);
                            intents.push(ReferenceIntent::TypeReference {
                                type_name: alias_name,
                                line,
                            });
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    let mut child = node.child(0);
    while let Some(c) = child {
        extract_import_intents_python(c, source, intents);
        child = c.next_sibling();
    }
}

pub(crate) fn extract_value_references_python(
    node: Node<'_>,
    source: &[u8],
    intents: &mut Vec<ReferenceIntent>,
) {
    let line = node.start_position().row + 1;

    if node.kind() == "keyword_argument"
        && let Some(value_node) = node.child_by_field_name("value")
        && value_node.kind() == "identifier"
    {
        let value_name = node_text(value_node, source);
        if !is_python_reserved_value(&value_name) {
            intents.push(ReferenceIntent::ValueReference { value_name, line });
        }
    }

    if node.kind() == "attribute"
        && is_attribute_value_context(node)
        && let Some(attr_node) = node.child_by_field_name("attribute")
    {
        let value_name = node_text(attr_node, source);
        if !is_python_reserved_value(&value_name) {
            intents.push(ReferenceIntent::ValueReference { value_name, line });
        }
    }

    let mut child = node.child(0);
    while let Some(c) = child {
        extract_value_references_python(c, source, intents);
        child = c.next_sibling();
    }
}

/// True if the attribute node's trailing identifier is being *read* as a value
/// (or written to as an assignment target), rather than:
/// 1. Acting as the function of a call (already captured by call extraction).
/// 2. Being the `object` of a wider attribute chain (the outermost link in
///    the chain will emit the trailing identifier, avoiding duplicates).
fn is_attribute_value_context(node: Node<'_>) -> bool {
    let Some(parent) = node.parent() else {
        return true;
    };

    if parent.kind() == "call"
        && parent
            .child_by_field_name("function")
            .is_some_and(|f| f.id() == node.id())
    {
        return false;
    }

    if parent.kind() == "attribute"
        && parent
            .child_by_field_name("object")
            .is_some_and(|o| o.id() == node.id())
    {
        return false;
    }

    true
}

const PYTHON_RESERVED_VALUES: &[&str] = &["self", "cls"];

fn is_python_reserved_value(name: &str) -> bool {
    PYTHON_RESERVED_VALUES.contains(&name)
}

pub(crate) fn extract_inheritance_intents_python(
    entity_node: Node<'_>,
    source: &[u8],
    intents: &mut Vec<ReferenceIntent>,
) {
    if entity_node.kind() != "class_definition" {
        return;
    }

    let line = entity_node.start_position().row + 1;

    for i in 0..entity_node.child_count() {
        if let Some(child) = entity_node.child(i as u32)
            && child.kind() == "argument_list"
        {
            // Walk the argument_list to find parent class identifiers
            extract_superclass_names(child, source, intents, line);
        }
    }
}

fn extract_superclass_names(
    node: Node<'_>,
    source: &[u8],
    intents: &mut Vec<ReferenceIntent>,
    line: usize,
) {
    for i in 0..node.child_count() {
        if let Some(child) = node.child(i as u32) {
            if child.kind() == "identifier" {
                let parent_name = node_text(child, source);
                if !is_python_reserved_value(&parent_name) {
                    intents.push(ReferenceIntent::Extends {
                        parent: parent_name,
                        line,
                    });
                }
            }
            // Recurse into nested structures (e.g., expression_list)
            extract_superclass_names(child, source, intents, line);
        }
    }
}

pub(crate) fn extract_decorator_intents_python(
    entity_node: Node<'_>,
    source: &[u8],
    intents: &mut Vec<ReferenceIntent>,
) {
    // Check if this definition has a decorated_definition parent
    let parent = match entity_node.parent() {
        Some(p) if p.kind() == "decorated_definition" => p,
        _ => return,
    };

    let line = entity_node.start_position().row + 1;

    for i in 0..parent.child_count() {
        if let Some(child) = parent.child(i as u32)
            && child.kind() == "decorator"
        {
            extract_decorator_name(child, source, intents, line);
        }
    }
}

fn extract_decorator_name(
    decorator_node: Node<'_>,
    source: &[u8],
    intents: &mut Vec<ReferenceIntent>,
    line: usize,
) {
    // Case 1: @identifier (e.g., @staticmethod, @property, @dataclass)
    //   decorator → (identifier)
    // Case 2: @call(args) (e.g., @route("/path"), @app.get("/"))
    //   decorator → (call function: (identifier|attribute))
    for i in 0..decorator_node.child_count() {
        if let Some(child) = decorator_node.child(i as u32) {
            let method_name = match child.kind() {
                "identifier" => Some(node_text(child, source)),
                "attribute" => child
                    .child_by_field_name("attribute")
                    .map(|n| node_text(n, source)),
                "call" => {
                    // @decorator(args): extract function name from call
                    child
                        .child_by_field_name("function")
                        .and_then(|func| match func.kind() {
                            "identifier" => Some(node_text(func, source)),
                            "attribute" => func
                                .child_by_field_name("attribute")
                                .map(|n| node_text(n, source)),
                            _ => None,
                        })
                }
                _ => None,
            };

            if let Some(method) = method_name
                && !is_python_reserved_value(&method)
            {
                intents.push(ReferenceIntent::Call {
                    method,
                    receiver: None,
                    line,
                    arg_count: None,
                });
            }
        }
    }
}

fn collect_import_names(
    node: Node<'_>,
    source: &[u8],
    intents: &mut Vec<ReferenceIntent>,
    line: usize,
) {
    match node.kind() {
        "identifier" => {
            let name = node_text(node, source);
            intents.push(ReferenceIntent::TypeReference {
                type_name: name,
                line,
            });
        }
        "aliased_import" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                let name = node_text(name_node, source);
                intents.push(ReferenceIntent::TypeReference {
                    type_name: name,
                    line,
                });
            }
        }
        _ => {}
    }

    let mut child = node.child(0);
    while let Some(c) = child {
        collect_import_names(c, source, intents, line);
        child = c.next_sibling();
    }
}

pub(crate) fn extract_decorator_names_python(
    entity_node: Node<'_>,
    source: &[u8],
    names: &mut Vec<String>,
) {
    let parent = match entity_node.parent() {
        Some(p) if p.kind() == "decorated_definition" => p,
        _ => return,
    };

    for i in 0..parent.child_count() {
        if let Some(child) = parent.child(i as u32)
            && child.kind() == "decorator"
        {
            // Skip the leading '@' if present
            let mut start = child.start_byte();
            if start < source.len() && source[start] == b'@' {
                start += 1;
            }
            let end = child.end_byte().min(source.len());
            if start < end
                && let Ok(text) = std::str::from_utf8(&source[start..end])
            {
                names.push(format!("@{}", text.trim()));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pipeline::parser::test_utils::parse_python_snippet;

    fn parse(code: &str) -> tree_sitter::Tree {
        parse_python_snippet(code).expect("Failed to parse Python code")
    }

    // ── handle_python_capture ────────────────────────────────────

    #[test]
    fn test_handle_python_capture_class() {
        let code = "class Foo: pass";
        let tree = parse(code);
        let class_node = crate::pipeline::parser::test_utils::find_first_node(
            tree.root_node(),
            &["class_definition"],
        )
        .expect("class_definition not found");
        // Find the identifier child for the class name
        let name_node = class_node
            .children(&mut class_node.walk())
            .find(|c| c.kind() == "identifier")
            .expect("identifier not found");
        let text = node_text(name_node, code.as_bytes());
        let result = handle_python_capture("python.class.name", &text, name_node);
        assert_eq!(
            result,
            Some(("Foo".to_string(), EntityKind::PythonClass, 1))
        );
    }

    #[test]
    fn test_handle_python_capture_function() {
        let code = "def foo(): pass";
        let tree = parse(code);
        let func_node = crate::pipeline::parser::test_utils::find_first_node(
            tree.root_node(),
            &["function_definition"],
        )
        .expect("function_definition not found");
        let name_node = func_node
            .child_by_field_name("name")
            .expect("name field not found");
        let text = node_text(name_node, code.as_bytes());
        let result = handle_python_capture("python.function.name", &text, name_node);
        assert_eq!(
            result,
            Some(("foo".to_string(), EntityKind::PythonFunction, 1))
        );
    }

    #[test]
    fn test_handle_python_capture_method() {
        let code = "class Foo:\n    def bar(self): pass";
        let tree = parse(code);
        let func_node = crate::pipeline::parser::test_utils::find_first_node(
            tree.root_node(),
            &["function_definition"],
        )
        .expect("function_definition not found");
        let name_node = func_node
            .child_by_field_name("name")
            .expect("name field not found");
        let text = node_text(name_node, code.as_bytes());
        let result = handle_python_capture("python.function.name", &text, name_node);
        assert_eq!(
            result,
            Some(("bar".to_string(), EntityKind::PythonMethod, 2))
        );
    }

    #[test]
    fn test_handle_python_capture_constant() {
        let code = "FOO = 42";
        let tree = parse(code);
        let name_node =
            crate::pipeline::parser::test_utils::find_first_node(tree.root_node(), &["identifier"])
                .expect("identifier not found");
        let text = node_text(name_node, code.as_bytes());
        let result = handle_python_capture("python.constant.name", &text, name_node);
        assert_eq!(
            result,
            Some(("FOO".to_string(), EntityKind::PythonConstant, 1))
        );
    }

    #[test]
    fn test_handle_python_capture_unknown() {
        let code = "x = 1";
        let tree = parse(code);
        let name_node =
            crate::pipeline::parser::test_utils::find_first_node(tree.root_node(), &["identifier"])
                .expect("identifier not found");
        let text = node_text(name_node, code.as_bytes());
        let result = handle_python_capture("unknown.capture", &text, name_node);
        assert_eq!(result, None);
    }

    // ── is_inside_class_body ─────────────────────────────────────

    #[test]
    fn test_is_inside_class_body_true() {
        let code = "class Foo:\n    def bar(self): pass";
        let tree = parse(code);
        let func_node = crate::pipeline::parser::test_utils::find_first_node(
            tree.root_node(),
            &["function_definition"],
        )
        .expect("function_definition not found");
        assert!(is_inside_class_body(func_node));
    }

    #[test]
    fn test_is_inside_class_body_false() {
        let code = "def foo(): pass";
        let tree = parse(code);
        let func_node = crate::pipeline::parser::test_utils::find_first_node(
            tree.root_node(),
            &["function_definition"],
        )
        .expect("function_definition not found");
        assert!(!is_inside_class_body(func_node));
    }

    // ── extract_call_intents_python ──────────────────────────────

    #[test]
    fn test_extract_call_simple_function() {
        let code = "foo()";
        let tree = parse(code);
        let mut intents = Vec::new();
        extract_call_intents_python(tree.root_node(), code.as_bytes(), &mut intents);

        assert_eq!(intents.len(), 1);
        assert_eq!(intents[0].method, "foo");
        assert_eq!(intents[0].receiver, None);
    }

    #[test]
    fn test_extract_call_with_receiver() {
        let code = "obj.method()";
        let tree = parse(code);
        let mut intents = Vec::new();
        extract_call_intents_python(tree.root_node(), code.as_bytes(), &mut intents);

        assert_eq!(intents.len(), 1);
        assert_eq!(intents[0].method, "method");
        assert_eq!(intents[0].receiver, Some("obj".to_string()));
    }

    #[test]
    fn test_extract_call_chained_receiver() {
        let code = "module.obj.method()";
        let tree = parse(code);
        let mut intents = Vec::new();
        extract_call_intents_python(tree.root_node(), code.as_bytes(), &mut intents);

        assert_eq!(intents.len(), 1);
        assert_eq!(intents[0].method, "method");
        assert_eq!(intents[0].receiver, Some("module.obj".to_string()));
    }

    #[test]
    fn test_extract_call_multiple_calls() {
        let code = "foo()\nbar()";
        let tree = parse(code);
        let mut intents = Vec::new();
        extract_call_intents_python(tree.root_node(), code.as_bytes(), &mut intents);

        assert_eq!(intents.len(), 2);
        assert_eq!(intents[0].method, "foo");
        assert_eq!(intents[1].method, "bar");
    }

    #[test]
    fn test_extract_call_nested() {
        let code = "foo(bar())";
        let tree = parse(code);
        let mut intents = Vec::new();
        extract_call_intents_python(tree.root_node(), code.as_bytes(), &mut intents);

        assert_eq!(intents.len(), 2);
        let methods: Vec<&str> = intents.iter().map(|i| i.method.as_str()).collect();
        assert!(methods.contains(&"foo"));
        assert!(methods.contains(&"bar"));
    }

    // ── extract_import_intents_python ────────────────────────────

    #[test]
    fn test_extract_import_simple() {
        let code = "import os";
        let tree = parse(code);
        let mut intents = Vec::new();
        extract_import_intents_python(tree.root_node(), code.as_bytes(), &mut intents);

        assert_eq!(intents.len(), 1);
        match &intents[0] {
            ReferenceIntent::TypeReference { type_name, .. } => {
                assert_eq!(type_name, "os");
            }
            _ => panic!("Expected TypeReference"),
        }
    }

    #[test]
    fn test_extract_import_dotted() {
        let code = "import os.path";
        let tree = parse(code);
        let mut intents = Vec::new();
        extract_import_intents_python(tree.root_node(), code.as_bytes(), &mut intents);

        // Dotted import produces separate references for each segment
        let has_os = intents.iter().any(
            |i| matches!(i, ReferenceIntent::TypeReference { type_name, .. } if type_name == "os"),
        );
        let has_path = intents.iter().any(|i| {
            matches!(i, ReferenceIntent::TypeReference { type_name, .. } if type_name == "path")
        });
        assert!(
            has_os,
            "Should have TypeReference for os, got: {:?}",
            intents
        );
        assert!(
            has_path,
            "Should have TypeReference for path, got: {:?}",
            intents
        );
    }

    #[test]
    fn test_extract_import_from() {
        let code = "from os import path";
        let tree = parse(code);
        let mut intents = Vec::new();
        extract_import_intents_python(tree.root_node(), code.as_bytes(), &mut intents);

        // Should contain a reference to the imported name
        let has_path = intents.iter().any(|i| {
            matches!(i, ReferenceIntent::TypeReference { type_name, .. } if type_name == "path")
        });
        assert!(
            has_path,
            "Should have TypeReference for path, got: {:?}",
            intents
        );
    }

    #[test]
    fn test_extract_import_aliased() {
        let code = "import numpy as np";
        let tree = parse(code);
        let mut intents = Vec::new();
        extract_import_intents_python(tree.root_node(), code.as_bytes(), &mut intents);

        // import_statement with aliased_import: the current implementation only
        // handles dotted_name/identifier children, not aliased_import directly,
        // so this produces 0 intents (known limitation).
        assert!(
            intents.is_empty(),
            "import with alias currently produces 0 intents, got: {:?}",
            intents
        );
    }

    #[test]
    fn test_extract_import_from_aliased() {
        let code = "from os import path as p";
        let tree = parse(code);
        let mut intents = Vec::new();
        extract_import_intents_python(tree.root_node(), code.as_bytes(), &mut intents);

        // from-import with aliased_import: extracts the alias name
        let has_alias = intents.iter().any(
            |i| matches!(i, ReferenceIntent::TypeReference { type_name, .. } if type_name == "p"),
        );
        assert!(
            has_alias,
            "Should have TypeReference for alias 'p', got: {:?}",
            intents
        );
    }

    // ── extract_value_references_python ──────────────────────────

    #[test]
    fn test_extract_value_reference_keyword_arg() {
        let code = "foo(bar=baz)";
        let tree = parse(code);
        let mut intents = Vec::new();
        extract_value_references_python(tree.root_node(), code.as_bytes(), &mut intents);

        assert_eq!(intents.len(), 1);
        match &intents[0] {
            ReferenceIntent::ValueReference { value_name, .. } => {
                assert_eq!(value_name, "baz");
            }
            _ => panic!("Expected ValueReference"),
        }
    }

    #[test]
    fn test_extract_value_reference_filters_self() {
        let code = "foo(self=x)";
        let tree = parse(code);
        let mut intents = Vec::new();
        extract_value_references_python(tree.root_node(), code.as_bytes(), &mut intents);

        // "self" is a reserved value, should be filtered
        let self_refs: Vec<_> = intents
            .iter()
            .filter(|i| matches!(i, ReferenceIntent::ValueReference { value_name, .. } if value_name == "self"))
            .collect();
        assert!(
            self_refs.is_empty(),
            "self should be filtered as reserved value"
        );
    }

    // Regression: chained attribute access used as a VALUE (not a call target)
    // should emit a ValueReference for the trailing identifier so that
    // `engine.chatter.loaded` becomes a reference to the property `loaded`.
    #[test]
    fn test_extract_value_reference_chained_attribute_access() {
        let code = "x = engine.chatter.loaded";
        let tree = parse(code);
        let mut intents = Vec::new();
        extract_value_references_python(tree.root_node(), code.as_bytes(), &mut intents);

        let names: Vec<&str> = intents
            .iter()
            .filter_map(|i| match i {
                ReferenceIntent::ValueReference { value_name, .. } => Some(value_name.as_str()),
                _ => None,
            })
            .collect();

        assert!(
            names.contains(&"loaded"),
            "must emit ValueReference for the trailing identifier `loaded`, got: {:?}",
            names
        );
        assert!(
            !names.contains(&"chatter"),
            "must NOT emit duplicate ValueReference for the inner chain segment `chatter`, got: {:?}",
            names
        );
    }

    // Regression: attribute used as a function argument (Gradio callback style)
    // — `load_btn.click(engine.chatter.load_model, ...)` passes `load_model`
    // as a value. The outer `click` call is captured separately; the inner
    // attribute must still emit a ValueReference for `load_model`.
    #[test]
    fn test_extract_value_reference_attribute_passed_as_argument() {
        let code = "load_btn.click(engine.chatter.load_model, foo)";
        let tree = parse(code);
        let mut intents = Vec::new();
        extract_value_references_python(tree.root_node(), code.as_bytes(), &mut intents);

        let names: Vec<&str> = intents
            .iter()
            .filter_map(|i| match i {
                ReferenceIntent::ValueReference { value_name, .. } => Some(value_name.as_str()),
                _ => None,
            })
            .collect();
        assert!(
            names.contains(&"load_model"),
            "must emit ValueReference for `load_model` when passed as an arg, got: {:?}",
            names
        );
    }

    // Regression: keyword argument whose value is a chained attribute access
    // — `gr.Column(visible=engine.chatter.loaded)`. The existing keyword-arg
    // path only fires for `identifier` values; the new attribute path covers
    // the `attribute` value.
    #[test]
    fn test_extract_value_reference_keyword_arg_with_attribute_value() {
        let code = "gr.Column(visible=engine.chatter.loaded)";
        let tree = parse(code);
        let mut intents = Vec::new();
        extract_value_references_python(tree.root_node(), code.as_bytes(), &mut intents);

        let names: Vec<&str> = intents
            .iter()
            .filter_map(|i| match i {
                ReferenceIntent::ValueReference { value_name, .. } => Some(value_name.as_str()),
                _ => None,
            })
            .collect();
        assert!(
            names.contains(&"loaded"),
            "keyword-arg with chained attribute value must emit ValueReference for `loaded`, got: {:?}",
            names
        );
    }

    // The attribute that is the function field of a call must NOT also emit
    // a ValueReference — that would double-count the call.
    #[test]
    fn test_attribute_call_function_not_emitted_as_value_reference() {
        let code = "engine.chatter.load_model(arg)";
        let tree = parse(code);
        let mut intents = Vec::new();
        extract_value_references_python(tree.root_node(), code.as_bytes(), &mut intents);

        let names: Vec<&str> = intents
            .iter()
            .filter_map(|i| match i {
                ReferenceIntent::ValueReference { value_name, .. } => Some(value_name.as_str()),
                _ => None,
            })
            .collect();
        assert!(
            !names.contains(&"load_model"),
            "must NOT emit ValueReference for the trailing identifier of a call's function (already a Call intent), got: {:?}",
            names
        );
    }

    #[test]
    fn test_extract_value_reference_filters_cls() {
        let code = "foo(cls=x)";
        let tree = parse(code);
        let mut intents = Vec::new();
        extract_value_references_python(tree.root_node(), code.as_bytes(), &mut intents);

        let cls_refs: Vec<_> = intents
            .iter()
            .filter(|i| matches!(i, ReferenceIntent::ValueReference { value_name, .. } if value_name == "cls"))
            .collect();
        assert!(
            cls_refs.is_empty(),
            "cls should be filtered as reserved value"
        );
    }

    // ── extract_inheritance_intents_python ───────────────────────

    #[test]
    fn test_extract_inheritance_single_parent() {
        let code = "class Foo(Bar): pass";
        let tree = parse(code);
        let class_node = crate::pipeline::parser::test_utils::find_first_node(
            tree.root_node(),
            &["class_definition"],
        )
        .expect("class_definition not found");
        let mut intents = Vec::new();
        extract_inheritance_intents_python(class_node, code.as_bytes(), &mut intents);

        let extends = crate::pipeline::parser::test_utils::collect_extends(&intents);
        assert_eq!(extends, &["Bar"]);
    }

    #[test]
    fn test_extract_inheritance_multiple_parents() {
        let code = "class Foo(Bar, Baz): pass";
        let tree = parse(code);
        let class_node = crate::pipeline::parser::test_utils::find_first_node(
            tree.root_node(),
            &["class_definition"],
        )
        .expect("class_definition not found");
        let mut intents = Vec::new();
        extract_inheritance_intents_python(class_node, code.as_bytes(), &mut intents);

        let extends = crate::pipeline::parser::test_utils::collect_extends(&intents);
        assert_eq!(extends.len(), 2);
        assert!(extends.contains(&"Bar"));
        assert!(extends.contains(&"Baz"));
    }

    #[test]
    fn test_extract_inheritance_none() {
        let code = "class Foo: pass";
        let tree = parse(code);
        let class_node = crate::pipeline::parser::test_utils::find_first_node(
            tree.root_node(),
            &["class_definition"],
        )
        .expect("class_definition not found");
        let mut intents = Vec::new();
        extract_inheritance_intents_python(class_node, code.as_bytes(), &mut intents);

        assert!(
            intents.is_empty(),
            "Expected no inheritance intents for class without parents"
        );
    }

    #[test]
    fn test_extract_inheritance_filters_self() {
        let code = "class Foo(self): pass";
        let tree = parse(code);
        let class_node = crate::pipeline::parser::test_utils::find_first_node(
            tree.root_node(),
            &["class_definition"],
        )
        .expect("class_definition not found");
        let mut intents = Vec::new();
        extract_inheritance_intents_python(class_node, code.as_bytes(), &mut intents);

        let self_refs: Vec<_> = intents
            .iter()
            .filter(|i| matches!(i, ReferenceIntent::Extends { parent, .. } if parent == "self"))
            .collect();
        assert!(
            self_refs.is_empty(),
            "self should be filtered as reserved value"
        );
    }

    // ── extract_decorator_intents_python ─────────────────────────

    #[test]
    fn test_extract_decorator_simple() {
        let code = "@staticmethod\ndef foo(): pass";
        let tree = parse(code);
        let func_node = crate::pipeline::parser::test_utils::find_first_node(
            tree.root_node(),
            &["function_definition"],
        )
        .expect("function_definition not found");
        let mut intents = Vec::new();
        extract_decorator_intents_python(func_node, code.as_bytes(), &mut intents);

        assert_eq!(intents.len(), 1);
        match &intents[0] {
            ReferenceIntent::Call {
                method, receiver, ..
            } => {
                assert_eq!(method, "staticmethod");
                assert_eq!(receiver, &None);
            }
            _ => panic!("Expected Call intent"),
        }
    }

    #[test]
    fn test_extract_decorator_with_call() {
        let code = "@route(\"/path\")\ndef foo(): pass";
        let tree = parse(code);
        let func_node = crate::pipeline::parser::test_utils::find_first_node(
            tree.root_node(),
            &["function_definition"],
        )
        .expect("function_definition not found");
        let mut intents = Vec::new();
        extract_decorator_intents_python(func_node, code.as_bytes(), &mut intents);

        assert_eq!(intents.len(), 1);
        match &intents[0] {
            ReferenceIntent::Call { method, .. } => {
                assert_eq!(method, "route");
            }
            _ => panic!("Expected Call intent"),
        }
    }

    #[test]
    fn test_extract_decorator_with_attribute() {
        let code = "@app.route\ndef foo(): pass";
        let tree = parse(code);
        let func_node = crate::pipeline::parser::test_utils::find_first_node(
            tree.root_node(),
            &["function_definition"],
        )
        .expect("function_definition not found");
        let mut intents = Vec::new();
        extract_decorator_intents_python(func_node, code.as_bytes(), &mut intents);

        assert_eq!(intents.len(), 1);
        match &intents[0] {
            ReferenceIntent::Call { method, .. } => {
                assert_eq!(method, "route");
            }
            _ => panic!("Expected Call intent"),
        }
    }

    #[test]
    fn test_extract_decorator_multiple() {
        let code = "@staticmethod\n@property\ndef foo(): pass";
        let tree = parse(code);
        let func_node = crate::pipeline::parser::test_utils::find_first_node(
            tree.root_node(),
            &["function_definition"],
        )
        .expect("function_definition not found");
        let mut intents = Vec::new();
        extract_decorator_intents_python(func_node, code.as_bytes(), &mut intents);

        assert_eq!(intents.len(), 2);
        let methods: Vec<&str> = intents
            .iter()
            .map(|i| match i {
                ReferenceIntent::Call { method, .. } => method.as_str(),
                _ => panic!("Expected Call intent"),
            })
            .collect();
        assert!(methods.contains(&"staticmethod"));
        assert!(methods.contains(&"property"));
    }

    #[test]
    fn test_extract_decorator_no_decorator() {
        let code = "def foo(): pass";
        let tree = parse(code);
        let func_node = crate::pipeline::parser::test_utils::find_first_node(
            tree.root_node(),
            &["function_definition"],
        )
        .expect("function_definition not found");
        let mut intents = Vec::new();
        extract_decorator_intents_python(func_node, code.as_bytes(), &mut intents);

        assert!(
            intents.is_empty(),
            "Expected no decorator intents for undecorated function"
        );
    }

    // ── extract_decorator_names_python ───────────────────────────

    #[test]
    fn test_extract_decorator_names_simple() {
        let code = "@staticmethod\ndef foo(): pass";
        let tree = parse(code);
        let func_node = crate::pipeline::parser::test_utils::find_first_node(
            tree.root_node(),
            &["function_definition"],
        )
        .expect("function_definition not found");
        let mut names = Vec::new();
        extract_decorator_names_python(func_node, code.as_bytes(), &mut names);

        assert_eq!(names, vec!["@staticmethod"]);
    }

    #[test]
    fn test_extract_decorator_names_with_call() {
        let code = "@route(\"/path\")\ndef foo(): pass";
        let tree = parse(code);
        let func_node = crate::pipeline::parser::test_utils::find_first_node(
            tree.root_node(),
            &["function_definition"],
        )
        .expect("function_definition not found");
        let mut names = Vec::new();
        extract_decorator_names_python(func_node, code.as_bytes(), &mut names);

        assert_eq!(names.len(), 1);
        assert!(names[0].starts_with("@route"));
    }

    #[test]
    fn test_extract_decorator_names_multiple() {
        let code = "@staticmethod\n@property\ndef foo(): pass";
        let tree = parse(code);
        let func_node = crate::pipeline::parser::test_utils::find_first_node(
            tree.root_node(),
            &["function_definition"],
        )
        .expect("function_definition not found");
        let mut names = Vec::new();
        extract_decorator_names_python(func_node, code.as_bytes(), &mut names);

        assert_eq!(names.len(), 2);
        assert!(names.contains(&"@staticmethod".to_string()));
        assert!(names.contains(&"@property".to_string()));
    }

    #[test]
    fn test_extract_decorator_names_no_decorator() {
        let code = "def foo(): pass";
        let tree = parse(code);
        let func_node = crate::pipeline::parser::test_utils::find_first_node(
            tree.root_node(),
            &["function_definition"],
        )
        .expect("function_definition not found");
        let mut names = Vec::new();
        extract_decorator_names_python(func_node, code.as_bytes(), &mut names);

        assert!(
            names.is_empty(),
            "Expected no decorator names for undecorated function"
        );
    }

    // ── extract_reference_intents_python (integration) ───────────

    #[test]
    fn test_extract_reference_intents_combined() {
        let code = "import os\n\ndef foo():\n    os.path.join(\"a\", \"b\")";
        let tree = parse(code);
        let mut intents = Vec::new();
        extract_reference_intents_python(tree.root_node(), code.as_bytes(), &mut intents);

        // Should have import reference + call intent
        let has_import = intents.iter().any(
            |i| matches!(i, ReferenceIntent::TypeReference { type_name, .. } if type_name == "os"),
        );
        let has_call = intents
            .iter()
            .any(|i| matches!(i, ReferenceIntent::Call { method, .. } if method == "join"));
        assert!(has_import, "Should have import reference for os");
        assert!(has_call, "Should have call intent for join");
    }
}
