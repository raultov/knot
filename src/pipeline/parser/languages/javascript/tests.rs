use super::*;
use crate::models::ReferenceIntent;
use crate::pipeline::parser::test_utils::{
    find_call_expression, find_new_expression, find_var_decl,
};

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
    crate::pipeline::parser::test_utils::assert_jsx_component_invocation(
        "function render() { return <ChartToolbar />; }",
        crate::pipeline::parser::test_utils::parse_javascript_snippet,
        "ChartToolbar",
        None,
    );
}

#[test]
fn test_extract_jsx_component_invocation_namespaced() {
    crate::pipeline::parser::test_utils::assert_jsx_component_invocation(
        "function render() { return <Sheet.Content />; }",
        crate::pipeline::parser::test_utils::parse_javascript_snippet,
        "Content",
        Some("Sheet"),
    );
}

#[test]
fn test_extract_single_call_intent_javascript_simple() {
    let code = "function test() { method(); }";
    let tree = crate::pipeline::parser::test_utils::parse_javascript_snippet(code)
        .expect("Failed to parse JavaScript code");

    let call = find_call_expression(tree.root_node()).expect("Call expression not found");
    let code_bytes = code.as_bytes();
    let intents = extract_single_call_intent_javascript(call, code_bytes);
    assert!(!intents.is_empty());
    assert_eq!(intents[0].method, "method");
    assert!(intents[0].receiver.is_none());
}

#[test]
fn test_extract_single_call_intent_javascript_member() {
    let code = "function test() { obj.method(); }";
    let tree = crate::pipeline::parser::test_utils::parse_javascript_snippet(code)
        .expect("Failed to parse JavaScript code");

    let call = find_call_expression(tree.root_node()).expect("Call expression not found");
    let code_bytes = code.as_bytes();
    let intents = extract_single_call_intent_javascript(call, code_bytes);
    assert!(!intents.is_empty());
    assert_eq!(intents[0].method, "method");
    assert_eq!(intents[0].receiver, Some("obj".to_string()));
}

#[test]
fn test_extract_single_call_intent_javascript_new() {
    let code = "function test() { new MyClass(); }";
    let tree = crate::pipeline::parser::test_utils::parse_javascript_snippet(code)
        .expect("Failed to parse JavaScript code");

    let new_expr = find_new_expression(tree.root_node()).expect("New expression not found");
    let code_bytes = code.as_bytes();
    let intents = extract_single_call_intent_javascript(new_expr, code_bytes);
    assert!(!intents.is_empty());
    assert_eq!(intents[0].method, "MyClass");
    assert!(intents[0].receiver.is_none());
}

#[test]
fn test_extract_single_call_intent_javascript_bind() {
    let code = "function test() { this.handleClick.bind(this); }";
    let tree = crate::pipeline::parser::test_utils::parse_javascript_snippet(code)
        .expect("Failed to parse JavaScript code");

    let call = find_call_expression(tree.root_node()).expect("Call expression not found");
    let code_bytes = code.as_bytes();
    let intents = extract_single_call_intent_javascript(call, code_bytes);
    assert!(!intents.is_empty());
    assert_eq!(intents[0].method, "handleClick");
    assert_eq!(intents[0].receiver, Some("this".to_string()));
}

#[test]
fn test_extract_single_call_intent_javascript_this_prop() {
    let code = "function test() { console.log(this.myProperty); }";
    let tree = crate::pipeline::parser::test_utils::parse_javascript_snippet(code)
        .expect("Failed to parse JavaScript code");

    // find member_expression: this.myProperty
    fn find_this_member(node: tree_sitter::Node<'_>) -> Option<tree_sitter::Node<'_>> {
        if node.kind() == "member_expression"
            && let Some(obj) = node.child_by_field_name("object")
            && obj.kind() == "this"
        {
            return Some(node);
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if let Some(n) = find_this_member(child) {
                return Some(n);
            }
        }
        None
    }

    let member = find_this_member(tree.root_node()).expect("this member expression not found");
    let code_bytes = code.as_bytes();
    let intents = extract_single_call_intent_javascript(member, code_bytes);
    assert!(!intents.is_empty());
    assert_eq!(intents[0].method, "myProperty");
    assert_eq!(intents[0].receiver, Some("this".to_string()));
}

#[test]
fn test_extract_class_inheritance_js() {
    crate::pipeline::parser::test_utils::assert_js_class_inheritance(
        "class Child extends Parent { }",
        crate::pipeline::parser::test_utils::parse_javascript_snippet,
        "Parent",
    );
}

#[test]
fn test_extract_class_inheritance_js_qualified() {
    crate::pipeline::parser::test_utils::assert_js_class_inheritance(
        "class Child extends NS.Parent { }",
        crate::pipeline::parser::test_utils::parse_javascript_snippet,
        "Parent",
    );
}

#[test]
fn test_extract_jsx_attributes_multiple() {
    // Uses parse_tsx_snippet in typescript.rs (same function, TSX superset of JSX)
    crate::pipeline::parser::test_utils::assert_jsx_attributes_multi(
        r#"function Form() { return <input id="email-input" className="form-control" />; }"#,
        crate::pipeline::parser::test_utils::parse_javascript_snippet,
        2,
        &[("id", "email-input"), ("className", "form-control")],
    );
}

#[test]
fn test_extract_jsx_attributes_classname() {
    crate::pipeline::parser::test_utils::assert_jsx_attribute(
        r#"function Button() { return <button className="btn primary">Click</button>; }"#,
        crate::pipeline::parser::test_utils::parse_javascript_snippet,
        "className",
        "btn primary",
    );
}

#[test]
fn test_extract_require_module_path() {
    let code = "var MyJsAlias = require('./alias_target_js');";
    let tree = crate::pipeline::parser::test_utils::parse_javascript_snippet(code)
        .expect("Failed to parse JavaScript code");

    let var_node = find_var_decl(tree.root_node()).unwrap();
    let path = extract_require_module_path(var_node, code.as_bytes());
    assert_eq!(path.as_deref(), Some("./alias_target_js"));
}

#[test]
fn test_js_import_named_emits_ref() {
    let code = "import { Foo } from './types';";
    let tree = crate::pipeline::parser::test_utils::parse_javascript_snippet(code)
        .expect("Failed to parse JavaScript code");
    let mut intents = Vec::new();
    collect_all_reference_intents_javascript(tree.root_node(), code.as_bytes(), &mut intents);
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
fn test_js_import_aliased_uses_original() {
    let code = "import { Foo as Bar } from './types';";
    let tree = crate::pipeline::parser::test_utils::parse_javascript_snippet(code)
        .expect("Failed to parse JavaScript code");
    let mut intents = Vec::new();
    collect_all_reference_intents_javascript(tree.root_node(), code.as_bytes(), &mut intents);
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
        "Should emit for Foo (original), got: {:?}",
        intents
    );
    assert!(
        !has_bar,
        "Should NOT emit for Bar (alias), got: {:?}",
        intents
    );
}

#[test]
fn test_js_require_destructure_emits_refs() {
    let code = "const { Foo, helper } = require('./m');";
    let tree = crate::pipeline::parser::test_utils::parse_javascript_snippet(code)
        .expect("Failed to parse JavaScript code");
    let mut intents = Vec::new();
    collect_all_reference_intents_javascript(tree.root_node(), code.as_bytes(), &mut intents);
    let has_foo = intents.iter().any(|(i, _)| match i {
        ReferenceIntent::ValueReference { value_name, .. } => value_name == "Foo",
        _ => false,
    });
    assert!(
        has_foo,
        "Should emit ValueReference for Foo from require destructure, got: {:?}",
        intents
    );
}
