use super::extract_entities_groovy;
use super::utils::{extract_preceding_docstring, strip_comments_line};
use crate::models::{EntityKind, ParsedEntity, ReferenceIntent};
use crate::pipeline::parser::test_utils::{
    assert_extends, assert_implements, collect_extends, collect_implements,
};

/// Helper: pick the Groovy class entity for `name` from the parser output.
fn pick_class<'a>(entities: &'a [ParsedEntity], name: &str) -> &'a ParsedEntity {
    entities
        .iter()
        .find(|e| e.name == name && e.kind == EntityKind::GroovyClass)
        .unwrap_or_else(|| panic!("Groovy class '{name}' not found in entities"))
}

fn pick_entity<'a>(entities: &'a [ParsedEntity], name: &str, kind: EntityKind) -> &'a ParsedEntity {
    entities
        .iter()
        .find(|e| e.name == name && e.kind == kind)
        .unwrap_or_else(|| {
            panic!(
                "Entity '{name}' ({kind:?}) not found in entities. Available: {:?}",
                entities
                    .iter()
                    .map(|e| (&e.name, &e.kind))
                    .collect::<Vec<_>>()
            )
        })
}

/// Helper: find the Groovy method `name` declared inside class `enclosing`.
fn find_method_in_class<'a>(
    entities: &'a [ParsedEntity],
    name: &str,
    enclosing: &str,
) -> Option<&'a ParsedEntity> {
    entities.iter().find(|e| {
        e.name == name
            && e.kind == EntityKind::GroovyMethod
            && e.enclosing_class.as_deref() == Some(enclosing)
    })
}

/// Helper: find a synthetic accessor entity by method name (its signature
/// carries the `<synthetic Groovy property accessor>` marker).
fn find_synthetic_accessor<'a>(
    entities: &'a [ParsedEntity],
    name: &str,
) -> Option<&'a ParsedEntity> {
    entities.iter().find(|e| {
        e.name == name
            && e.kind == EntityKind::GroovyMethod
            && e.signature
                .as_deref()
                .is_some_and(|s| s.contains("synthetic"))
    })
}

// ---- Groovy Standard (tree-sitter) extraction tests ----

#[test]
fn test_groovy_class_extraction() {
    let source = "class MyGroovyClass { def method() {} }";
    let entities = extract_entities_groovy(source, "test.groovy", "test-repo");
    assert!(
        entities
            .iter()
            .any(|e| e.name == "MyGroovyClass" && e.kind == EntityKind::GroovyClass)
    );
}

#[test]
fn test_groovy_interface_extraction() {
    let source = "interface MyGroovyInterface { void doIt() }";
    let entities = extract_entities_groovy(source, "test.groovy", "test-repo");
    assert!(
        entities
            .iter()
            .any(|e| e.name == "MyGroovyInterface" && e.kind == EntityKind::GroovyInterface)
    );
}

#[test]
fn test_groovy_enum_extraction() {
    let source = "enum Color { RED, GREEN, BLUE }";
    let entities = extract_entities_groovy(source, "test.groovy", "test-repo");
    assert!(
        entities
            .iter()
            .any(|e| e.name == "Color" && e.kind == EntityKind::GroovyEnum)
    );
}

#[test]
fn test_groovy_method_extraction() {
    let source = "class Foo { String greet(String name) { return name } }";
    let entities = extract_entities_groovy(source, "test.groovy", "test-repo");
    let method = entities.iter().find(|e| e.name == "greet");
    assert!(method.is_some(), "Expected method 'greet' to be extracted");
    assert_eq!(method.unwrap().kind, EntityKind::GroovyMethod);
}

#[test]
fn test_groovy_trait_extraction() {
    let source = "trait MyTrait { void doSomething() {} }";
    let entities = extract_entities_groovy(source, "test.groovy", "test-repo");
    assert!(
        entities
            .iter()
            .any(|e| e.name == "MyTrait" && e.kind == EntityKind::GroovyTrait)
    );
}

#[test]
fn test_groovy_property_extraction() {
    let source = "class Foo { String name = 'test' }";
    let entities = extract_entities_groovy(source, "test.groovy", "test-repo");
    assert!(
        entities
            .iter()
            .any(|e| e.name == "name" && e.kind == EntityKind::GroovyProperty)
    );
}

#[test]
fn test_groovy_multiple_classes() {
    let source = "package com.example\nclass First {}\nclass Second {}\nclass Third {}";
    let entities = extract_entities_groovy(source, "test.groovy", "test-repo");
    let class_names: Vec<_> = entities
        .iter()
        .filter(|e| e.kind == EntityKind::GroovyClass)
        .map(|e| e.name.clone())
        .collect();
    assert!(class_names.contains(&"First".to_string()));
    assert!(class_names.contains(&"Second".to_string()));
    assert!(class_names.contains(&"Third".to_string()));
}

#[test]
fn test_groovy_constructor_extraction() {
    let source = "class User { User(String name) { this.name = name } }";
    let entities = extract_entities_groovy(source, "test.groovy", "test-repo");
    assert!(
        entities
            .iter()
            .any(|e| e.name == "User" && e.kind == EntityKind::GroovyMethod)
    );
}

#[test]
fn test_groovy_empty_body_class() {
    let source = "class EmptyClass {}";
    let entities = extract_entities_groovy(source, "test.groovy", "test-repo");
    assert!(
        entities
            .iter()
            .any(|e| e.name == "EmptyClass" && e.kind == EntityKind::GroovyClass)
    );
}

#[test]
fn test_groovy_method_in_class_extracts_correctly() {
    let source = "class Calculator {\n  int add(int a, int b) { return a + b }\n  int subtract(int a, int b) { return a - b }\n}";
    let entities = extract_entities_groovy(source, "test.groovy", "test-repo");
    assert!(
        entities
            .iter()
            .any(|e| e.name == "add" && e.kind == EntityKind::GroovyMethod)
    );
    assert!(
        entities
            .iter()
            .any(|e| e.name == "subtract" && e.kind == EntityKind::GroovyMethod)
    );
}

#[test]
fn test_groovy_parse_sample_full_file_types() {
    let source = include_str!("../../../../../tests/testing_files/sample_full.groovy");
    let entities = extract_entities_groovy(source, "sample_full.groovy", "test-repo");

    assert!(
        entities
            .iter()
            .any(|e| e.name == "UserService" && e.kind == EntityKind::GroovyClass)
    );
    assert!(
        entities
            .iter()
            .any(|e| e.name == "BaseService" && e.kind == EntityKind::GroovyClass)
    );
    assert!(
        entities
            .iter()
            .any(|e| e.name == "DatabaseConfig" && e.kind == EntityKind::GroovyClass)
    );
    assert!(
        entities
            .iter()
            .any(|e| e.name == "Repository" && e.kind == EntityKind::GroovyInterface)
    );
    assert!(
        entities
            .iter()
            .any(|e| e.name == "Auditable" && e.kind == EntityKind::GroovyTrait)
    );
    assert!(
        entities
            .iter()
            .any(|e| e.name == "Status" && e.kind == EntityKind::GroovyEnum)
    );
    assert!(
        entities.len() >= 20,
        "Expected at least 20 entities, got {}",
        entities.len()
    );
}

#[test]
fn test_groovy_parse_sample_full_file_methods_and_properties() {
    let source = include_str!("../../../../../tests/testing_files/sample_full.groovy");
    let entities = extract_entities_groovy(source, "sample_full.groovy", "test-repo");

    assert!(
        entities
            .iter()
            .any(|e| e.name == "scriptMethod" && e.kind == EntityKind::GroovyMethod)
    );
    assert!(
        entities
            .iter()
            .any(|e| e.name == "anotherScriptMethod" && e.kind == EntityKind::GroovyMethod)
    );
    assert!(
        entities
            .iter()
            .any(|e| e.name == "globalConfig" && e.kind == EntityKind::GroovyProperty)
    );
    assert!(
        entities
            .iter()
            .any(|e| e.name == "processDataClosure" && e.kind == EntityKind::GroovyProperty)
    );
    assert!(
        entities
            .iter()
            .any(|e| e.name == "initialize" && e.kind == EntityKind::GroovyMethod)
    );
    assert!(
        entities
            .iter()
            .any(|e| e.name == "calculateTotal" && e.kind == EntityKind::GroovyMethod)
    );
    assert!(
        entities
            .iter()
            .any(|e| e.name == "logAction" && e.kind == EntityKind::GroovyMethod)
    );
    assert!(entities.iter().any(
        |e| e.name == "addition of #num1 and #num2 should be #expected"
            && e.kind == EntityKind::GroovyMethod
    ));
    assert!(
        entities
            .iter()
            .any(|e| e.name == "DEFAULT_ROLE" && e.kind == EntityKind::GroovyProperty)
    );
    assert!(
        entities
            .iter()
            .any(|e| e.name == "maxLoginAttempts" && e.kind == EntityKind::GroovyProperty)
    );
}

#[test]
fn test_groovy_parse_sample_full_file_docstrings() {
    let source = include_str!("../../../../../tests/testing_files/sample_full.groovy");
    let entities = extract_entities_groovy(source, "sample_full.groovy", "test-repo");

    // Docstring extraction: comments in the fixture now surface as docstrings.
    let global_config = entities
        .iter()
        .find(|e| e.name == "globalConfig" && e.kind == EntityKind::GroovyProperty)
        .expect("globalConfig not extracted");
    assert_eq!(
        global_config.docstring.as_deref(),
        Some("1. Top-level script variables and closures")
    );
    let user_service = entities
        .iter()
        .find(|e| e.name == "UserService" && e.kind == EntityKind::GroovyClass)
        .expect("UserService not extracted");
    assert_eq!(
        user_service.docstring.as_deref(),
        Some("7. Main Class with Annotations, Inheritance, Traits, and inner classes")
    );
    let initialize = entities
        .iter()
        .find(|e| {
            e.name == "initialize"
                && e.kind == EntityKind::GroovyMethod
                && e.enclosing_class.as_deref() == Some("UserService")
        })
        .expect("UserService.initialize not extracted");
    assert_eq!(
        initialize.docstring.as_deref(),
        Some("Typed Method overriding base class")
    );
    // Regression: a property with no preceding comment keeps docstring == None.
    let max_login = entities
        .iter()
        .find(|e| e.name == "maxLoginAttempts" && e.kind == EntityKind::GroovyProperty)
        .expect("maxLoginAttempts not extracted");
    assert_eq!(max_login.docstring, None);
}

#[test]
fn test_groovy_fqn_with_package() {
    let source = "package com.acme.app\nclass MyService { String greet(String name) { name } }";
    let entities = extract_entities_groovy(source, "test.groovy", "test-repo");

    let class_entity = entities
        .iter()
        .find(|e| e.name == "MyService")
        .expect("MyService class not extracted");
    assert_eq!(class_entity.fqn, "com.acme.app.MyService");

    let method_entity = entities
        .iter()
        .find(|e| e.name == "greet")
        .expect("greet method not extracted");
    assert_eq!(method_entity.fqn, "com.acme.app.MyService.greet");
    assert_eq!(method_entity.enclosing_class.as_deref(), Some("MyService"));
}

#[test]
fn test_groovy_method_parent_class() {
    let source = "class Calculator {\n  int add(int a, int b) { a + b }\n  def multiply(int x, int y) { x * y }\n}";
    let entities = extract_entities_groovy(source, "test.groovy", "test-repo");

    let add_method = entities
        .iter()
        .find(|e| e.name == "add")
        .expect("add method not extracted");
    assert_eq!(add_method.enclosing_class.as_deref(), Some("Calculator"));
    assert_eq!(add_method.fqn, "Calculator.add");

    let multiply_method = entities
        .iter()
        .find(|e| e.name == "multiply")
        .expect("multiply method not extracted");
    assert_eq!(
        multiply_method.enclosing_class.as_deref(),
        Some("Calculator")
    );
    assert_eq!(multiply_method.fqn, "Calculator.multiply");
}

#[test]
fn test_groovy_interface_method_has_parent() {
    let source = "interface Repository {\n  String findById(String id)\n}";
    let entities = extract_entities_groovy(source, "test.groovy", "test-repo");

    let method = entities
        .iter()
        .find(|e| e.name == "findById")
        .expect("findById not extracted");
    assert_eq!(method.enclosing_class.as_deref(), Some("Repository"));
}

#[test]
fn test_groovy_nested_scope_tracking() {
    let source = "class Outer {\n  class Inner {\n    String getValue() { 'val' }\n  }\n}";
    let entities = extract_entities_groovy(source, "test.groovy", "test-repo");

    let outer = entities
        .iter()
        .find(|e| e.name == "Outer")
        .expect("Outer class not extracted");
    assert_eq!(outer.kind, EntityKind::GroovyClass);

    let inner = entities
        .iter()
        .find(|e| e.name == "Inner")
        .expect("Inner class not extracted");
    assert_eq!(inner.kind, EntityKind::GroovyClass);

    let method = entities
        .iter()
        .find(|e| e.name == "getValue")
        .expect("getValue method not extracted");
    assert_eq!(method.enclosing_class.as_deref(), Some("Inner"));
    assert_eq!(method.fqn, "Inner.getValue");
}

#[test]
fn test_groovy_trait_method_has_parent() {
    let source = "trait Auditable {\n  def logAction(String msg) { println msg }\n}";
    let entities = extract_entities_groovy(source, "test.groovy", "test-repo");

    let method = entities
        .iter()
        .find(|e| e.name == "logAction")
        .expect("logAction not extracted");
    assert_eq!(method.enclosing_class.as_deref(), Some("Auditable"));
    assert_eq!(method.fqn, "Auditable.logAction");
}

#[test]
fn test_groovy_resilience_empty_file() {
    let entities = extract_entities_groovy("", "test.groovy", "test-repo");
    assert!(entities.is_empty());
}

#[test]
fn test_groovy_resilience_malformed() {
    let source = "garbage {{{ // not valid groovy\nclass ";
    let entities = extract_entities_groovy(source, "test.groovy", "test-repo");
    // Should not panic, just return what it can (likely empty)
    assert!(entities.is_empty() || entities.iter().any(|e| e.name == "class"));
}

#[test]
fn test_innermost_assignment_nested_methods() {
    // Replicates code-history-mining UI.groovy pattern:
    // showGrabbingFinishedMessage contains hyperlinkUpdate which calls runAnalyzer.
    // Only hyperlinkUpdate (innermost) should get the reference, NOT the outer container.
    let source = r#"
package com.example

class NestedMethods {
    def showGrabbingFinishedMessage(String message) {
        show(message, new Listener() {
            @Override void hyperlinkUpdate(String event) {
                runAnalyzer("visualize")
            }
        })
    }

    def show(message, Listener listener) {
    }

    private void runAnalyzer(String action) {
        println action
    }
}
"#;
    let entities = extract_entities_groovy(source, "NestedMethods.groovy", "test-repo");

    // hyperlinkUpdate should get the runAnalyzer call
    let hyperlink = entities
        .iter()
        .find(|e| e.name == "hyperlinkUpdate")
        .expect("hyperlinkUpdate not found");
    let hyper_has_run = hyperlink
        .reference_intents
        .iter()
        .any(|r| matches!(r, ReferenceIntent::Call { method, .. } if method == "runAnalyzer"));
    assert!(
        hyper_has_run,
        "hyperlinkUpdate should have CALL to runAnalyzer"
    );

    // showGrabbingFinishedMessage must NOT have the runAnalyzer call
    let outer = entities
        .iter()
        .find(|e| e.name == "showGrabbingFinishedMessage")
        .expect("showGrabbingFinishedMessage not found");
    let outer_has_run = outer
        .reference_intents
        .iter()
        .any(|r| matches!(r, ReferenceIntent::Call { method, .. } if method == "runAnalyzer"));
    assert!(
        !outer_has_run,
        "showGrabbingFinishedMessage should NOT have CALL to runAnalyzer (belongs to hyperlinkUpdate)"
    );
}

#[test]
fn test_groovy_resilience_missing_braces() {
    let source = "class Broken {\n  def method1() { }\n  def method2() { }\n// no closing brace";
    let entities = extract_entities_groovy(source, "test.groovy", "test-repo");
    // Should extract what it can without panicking
    assert!(entities.iter().any(|e| e.name == "Broken"));
    assert!(entities.iter().any(|e| e.name == "method1"));
    assert!(entities.iter().any(|e| e.name == "method2"));
}

// ─────────────────────────────────────────────────────────────────────
// Group: Groovy inheritance intent extraction (Extends / Implements)
// ─────────────────────────────────────────────────────────────────────

#[test]
fn test_groovy_class_extends() {
    let source = "class Ext1 extends PluginExtensionPoint { }";
    let entities = extract_entities_groovy(source, "test.groovy", "test-repo");
    let ext1 = pick_class(&entities, "Ext1");
    assert_extends(&ext1.reference_intents, "PluginExtensionPoint");
    assert!(collect_implements(&ext1.reference_intents).is_empty());
}

#[test]
fn test_groovy_class_implements() {
    let source = "abstract class PluginExtensionPoint implements ExtensionPoint { }";
    let entities = extract_entities_groovy(source, "test.groovy", "test-repo");
    let cls = pick_class(&entities, "PluginExtensionPoint");
    assert_implements(&cls.reference_intents, "ExtensionPoint");
    assert!(collect_extends(&cls.reference_intents).is_empty());
}

#[test]
fn test_groovy_class_extends_and_implements_multiple() {
    let source = "class OrderService extends BaseService implements Auditable, Serializable { }";
    let entities = extract_entities_groovy(source, "test.groovy", "test-repo");
    let cls = pick_class(&entities, "OrderService");
    let extends = collect_extends(&cls.reference_intents);
    let implements = collect_implements(&cls.reference_intents);
    assert_eq!(extends, vec!["BaseService"]);
    assert_eq!(implements.len(), 2);
    assert!(implements.contains(&"Auditable"));
    assert!(implements.contains(&"Serializable"));
}

#[test]
fn test_groovy_extends_with_generics() {
    let source = "class Repo extends AbstractRepo<Order, Long> { }";
    let entities = extract_entities_groovy(source, "test.groovy", "test-repo");
    let cls = pick_class(&entities, "Repo");
    assert_extends(&cls.reference_intents, "AbstractRepo");
}

#[test]
fn test_groovy_generic_bound_is_not_extends() {
    let source = "class Box<T extends Comparable> { }";
    let entities = extract_entities_groovy(source, "test.groovy", "test-repo");
    let cls = pick_class(&entities, "Box");
    assert!(collect_extends(&cls.reference_intents).is_empty());
    assert!(collect_implements(&cls.reference_intents).is_empty());
}

#[test]
fn test_groovy_interface_extends_multiple() {
    let source = "interface EventBus extends Publisher, Subscriber { }";
    let entities = extract_entities_groovy(source, "test.groovy", "test-repo");
    let iface = pick_entity(&entities, "EventBus", EntityKind::GroovyInterface);
    let extends = collect_extends(&iface.reference_intents);
    assert_eq!(extends.len(), 2);
    assert!(extends.contains(&"Publisher"));
    assert!(extends.contains(&"Subscriber"));
    assert!(collect_implements(&iface.reference_intents).is_empty());
}

#[test]
fn test_groovy_trait_implements() {
    let source = "trait Auditable implements Serializable { }";
    let entities = extract_entities_groovy(source, "test.groovy", "test-repo");
    let trait_entity = pick_entity(&entities, "Auditable", EntityKind::GroovyTrait);
    let implements = collect_implements(&trait_entity.reference_intents);
    assert_eq!(implements, vec!["Serializable"]);
    assert!(collect_extends(&trait_entity.reference_intents).is_empty());
}

#[test]
fn test_groovy_enum_implements() {
    let source = "enum Status implements Describable { OK, KO }";
    let entities = extract_entities_groovy(source, "test.groovy", "test-repo");
    let enum_entity = pick_entity(&entities, "Status", EntityKind::GroovyEnum);
    let implements = collect_implements(&enum_entity.reference_intents);
    assert_eq!(implements, vec!["Describable"]);
}

#[test]
fn test_groovy_extends_qualified_name() {
    let source = "class Foo extends nextflow.plugin.BasePlugin { }";
    let entities = extract_entities_groovy(source, "test.groovy", "test-repo");
    let cls = pick_class(&entities, "Foo");
    assert_extends(&cls.reference_intents, "nextflow.plugin.BasePlugin");
}

#[test]
fn test_groovy_extends_multiline_declaration() {
    let source = "class OrderService extends BaseService<Order>\n        implements Auditable, Serializable {\n}";
    let entities = extract_entities_groovy(source, "test.groovy", "test-repo");
    let cls = pick_class(&entities, "OrderService");
    let extends = collect_extends(&cls.reference_intents);
    let implements = collect_implements(&cls.reference_intents);
    assert_eq!(extends, vec!["BaseService"]);
    assert_eq!(implements.len(), 2);
    assert!(implements.contains(&"Auditable"));
    assert!(implements.contains(&"Serializable"));
    // The line on the intent must point at the class declaration's start line.
    for intent in &cls.reference_intents {
        match intent {
            ReferenceIntent::Extends { line, .. } | ReferenceIntent::Implements { line, .. } => {
                assert_eq!(*line, cls.start_line);
            }
            _ => {}
        }
    }
}

#[test]
fn test_groovy_class_without_inheritance_has_no_intents() {
    let source = "class Plain { def m() {} }";
    let entities = extract_entities_groovy(source, "test.groovy", "test-repo");
    let cls = pick_class(&entities, "Plain");
    assert!(collect_extends(&cls.reference_intents).is_empty());
    assert!(collect_implements(&cls.reference_intents).is_empty());
}

#[test]
fn test_groovy_extends_intent_attached_to_class_not_methods() {
    // Class with extends + a method body that contains a CALL.
    // The Extends intent must hang on the class, not on a method.
    let source = r#"
class Ext1 extends PluginExtensionPoint {
    protected void init(Object session) {
        runAnalyzer("foo")
    }
}
"#;
    let entities = extract_entities_groovy(source, "test.groovy", "test-repo");
    let cls = pick_class(&entities, "Ext1");
    assert_extends(&cls.reference_intents, "PluginExtensionPoint");
    let init = entities
        .iter()
        .find(|e| e.name == "init" && e.kind == EntityKind::GroovyMethod)
        .expect("method 'init' not extracted");
    // The method must NOT inherit its parent's Extends intent.
    assert!(
        !init
            .reference_intents
            .iter()
            .any(|r| matches!(r, ReferenceIntent::Extends { .. })),
        "method 'init' should not receive the class's Extends intent"
    );
    // The method should still have its Call intent intact.
    assert!(
        init.reference_intents
            .iter()
            .any(|r| matches!(r, ReferenceIntent::Call { method, .. } if method == "runAnalyzer")),
        "method 'init' should still have CALL to runAnalyzer"
    );
}

#[test]
fn test_groovy_extends_line_number() {
    let source = "\n\nclass Foo extends Bar {\n}\n";
    let entities = extract_entities_groovy(source, "test.groovy", "test-repo");
    let cls = pick_class(&entities, "Foo");
    // The class is declared on line 3 (1-indexed).
    assert_eq!(cls.start_line, 3);
    let extends = collect_extends(&cls.reference_intents);
    assert_eq!(extends, vec!["Bar"]);
    let intent_line = cls
        .reference_intents
        .iter()
        .find_map(|r| match r {
            ReferenceIntent::Extends { line, .. } => Some(*line),
            _ => None,
        })
        .expect("expected Extends intent on Foo");
    assert_eq!(
        intent_line, cls.start_line,
        "Extends intent line must match class declaration line"
    );
}

#[test]
fn test_groovy_extends_ignores_comments() {
    // The commented-out class must NOT produce any intent; only the real class does.
    let source = r#"
// class Fake extends Nope
class Real extends Base {
}
"#;
    let entities = extract_entities_groovy(source, "test.groovy", "test-repo");
    // No Fake entity should exist.
    assert!(
        !entities.iter().any(|e| e.name == "Fake"),
        "Fake should not be extracted from a comment line"
    );
    let cls = pick_class(&entities, "Real");
    assert_extends(&cls.reference_intents, "Base");
}

// ─────────────────────────────────────────────────────────────────────
// Group: GroovyDoc / docstring extraction (extract_preceding_docstring)
// ─────────────────────────────────────────────────────────────────────

/// Helper: materialize lines and run the docstring walker against the
/// 0-based index of the declaration line.
fn doc_of(source: &str, decl_line_idx: usize) -> Option<String> {
    let lines: Vec<&str> = source.lines().collect();
    extract_preceding_docstring(&lines, decl_line_idx)
}

#[test]
fn test_groovy_docstring_block_comment_adjacent() {
    let source = "/**\n * Channel factory initialization. This method is invoked one and only once\n *\n * @param session The current nextflow session\n */\nabstract protected void init(Session session)\n";
    let doc = doc_of(source, 5).expect("expected docstring for init");
    assert!(doc.contains("Channel factory initialization"));
    assert!(doc.contains("@param session The current nextflow session"));
    assert!(!doc.contains("/**"), "markers must be stripped: {doc:?}");
    assert!(!doc.contains("*/"), "markers must be stripped: {doc:?}");
    assert!(
        !doc.lines().any(|l| l.trim_start().starts_with('*')),
        "leading '*' must be stripped: {doc:?}"
    );
}

#[test]
fn test_groovy_docstring_skips_annotations() {
    // Exact shape of the nextflow `checkInit` case: GroovyDoc, then an
    // annotation, then the declaration.
    let source = "/** doc */\n@PackageScope\nsynchronized void checkInit(Object session) {\n";
    let doc = doc_of(source, 2);
    assert_eq!(doc.as_deref(), Some("doc"));
}

#[test]
fn test_groovy_docstring_skips_multiple_annotations() {
    let source = "/** doc */\n@PackageScope\n@Override\nvoid m() {\n";
    let doc = doc_of(source, 3);
    assert_eq!(doc.as_deref(), Some("doc"));
}

#[test]
fn test_groovy_docstring_line_comments_burst() {
    let source = "// a\n// b\nclass Foo {\n";
    let doc = doc_of(source, 2);
    assert_eq!(doc.as_deref(), Some("a\nb"));
}

#[test]
fn test_groovy_docstring_tolerates_single_blank_line() {
    let source = "/** doc */\n\nvoid m() {\n";
    let doc = doc_of(source, 2);
    assert_eq!(doc.as_deref(), Some("doc"));
}

#[test]
fn test_groovy_docstring_two_blank_lines_breaks() {
    let source = "/** doc */\n\n\nvoid m() {\n";
    let doc = doc_of(source, 3);
    assert_eq!(doc, None);
}

#[test]
fn test_groovy_docstring_none_when_absent() {
    let source = "void other() {\nvoid m() {\n";
    let doc = doc_of(source, 1);
    assert_eq!(doc, None);
}

#[test]
fn test_groovy_docstring_stops_at_import() {
    // License header must never leak into the first class's docstring.
    let source = "/*\n * Licensed under the Apache License\n */\npackage com.example\n\nimport foo.Bar\n\nclass Foo {\n";
    let doc = doc_of(source, 7);
    assert_eq!(doc, None);
}

#[test]
fn test_groovy_docstring_empty_comment_is_none() {
    let source = "/** */\nvoid m() {\n";
    assert_eq!(doc_of(source, 1), None);
    let source2 = "//\nvoid m() {\n";
    assert_eq!(doc_of(source2, 1), None);
}

#[test]
fn test_groovy_docstring_first_line_of_file() {
    let source = "class Foo {\n";
    assert_eq!(doc_of(source, 0), None);
}

#[test]
fn test_groovy_docstring_malformed_block_no_panic() {
    // Orphan `*/` with no visible opener: must not panic, returns None.
    let source = "*/\nclass Foo {\n";
    assert_eq!(doc_of(source, 1), None);
    // Orphan closer further down the file.
    let source2 = "package p\n\n * dangling\n */\nclass Foo {\n";
    assert_eq!(doc_of(source2, 4), None);
}

// ─────────────────────────────────────────────────────────────────────
// Group: docstring wiring into extract_entities_groovy
// ─────────────────────────────────────────────────────────────────────

#[test]
fn test_groovy_class_has_docstring() {
    let source = "/**\n * A service class.\n */\nclass MyService {\n}\n";
    let entities = extract_entities_groovy(source, "test.groovy", "test-repo");
    let cls = pick_class(&entities, "MyService");
    assert_eq!(cls.docstring.as_deref(), Some("A service class."));
}

#[test]
fn test_groovy_abstract_method_has_docstring() {
    // Literal fragment of nextflow's PluginExtensionPoint.groovy — the exact
    // regression case: GroovyDoc on an abstract method with no body.
    let source = r#"package nextflow.plugin.extension

abstract class PluginExtensionPoint implements ExtensionPoint {

    private boolean initialised

    /**
     * Channel factory initialization. This method is invoked one and only once
     *
     * @param session The current nextflow session
     */
    abstract protected void init(Session session)
}
"#;
    let entities = extract_entities_groovy(source, "PluginExtensionPoint.groovy", "test-repo");
    let init = pick_entity(&entities, "init", EntityKind::GroovyMethod);
    let doc = init
        .docstring
        .as_deref()
        .expect("init must carry its GroovyDoc");
    assert!(doc.contains("Channel factory initialization"));
    assert!(!doc.contains("/**") && !doc.contains("*/"));
}

#[test]
fn test_groovy_def_method_has_docstring() {
    let source = "class Foo {\n    /** Computes the answer. */\n    def compute() { 42 }\n}\n";
    let entities = extract_entities_groovy(source, "test.groovy", "test-repo");
    let m = pick_entity(&entities, "compute", EntityKind::GroovyMethod);
    assert_eq!(m.docstring.as_deref(), Some("Computes the answer."));
}

#[test]
fn test_groovy_property_has_docstring() {
    let source = "class Foo {\n    // The default role\n    String role = \"USER\"\n}\n";
    let entities = extract_entities_groovy(source, "test.groovy", "test-repo");
    let prop = pick_entity(&entities, "role", EntityKind::GroovyProperty);
    assert_eq!(prop.docstring.as_deref(), Some("The default role"));
}

#[test]
fn test_groovy_multiline_method_has_docstring() {
    // Multi-line signature (`(` without `)` on the first line): the docstring
    // must be located from the real method start line, not from the line
    // where the parser finished scanning the signature.
    let source = r#"class HttpUtil {
    /**
     * Restart the HTTP server.
     */
    private static SimpleHttpServer restartHttpServer(String id, String webRootPath,
                                                       Closure handler = {null},
                                                       Closure errorListener = {}) {
        new SimpleHttpServer()
    }
}
"#;
    let entities = extract_entities_groovy(source, "HttpUtil.groovy", "test-repo");
    let m = pick_entity(&entities, "restartHttpServer", EntityKind::GroovyMethod);
    assert_eq!(m.docstring.as_deref(), Some("Restart the HTTP server."));
}

#[test]
fn test_groovy_method_without_doc_has_none() {
    // Regression: entities without a preceding comment keep docstring == None.
    let source = "class Foo {\n    int add(int a, int b) { a + b }\n}\n";
    let entities = extract_entities_groovy(source, "test.groovy", "test-repo");
    let m = pick_entity(&entities, "add", EntityKind::GroovyMethod);
    assert_eq!(m.docstring, None);
    let cls = pick_class(&entities, "Foo");
    assert_eq!(cls.docstring, None);
}

// ─────────────────────────────────────────────────────────────────────
// Phase 0-3: Groovy property accessors & parser hardening regression
// ─────────────────────────────────────────────────────────────────────

const ISESSION_SRC: &str = r#"
package nf

interface ISession {

    /**
     * The folder where the main script is contained
     */
    Path getBaseDir()

    /**
     * The pipeline script name (without parent path)
     */
    String getScriptName()
}
"#;

const SESSION_SRC: &str = r#"
package nf

class Session implements ISession {

    /**
     * The folder where the main script is contained
     */
    Path baseDir

    /**
     * The pipeline script name (without parent path)
     */
    String scriptName

    void setBaseDir( Path baseDir ) {
        this.baseDir = baseDir
    }
}
"#;

// --- Phase 0: Regression pinning (RED, fail before implementation) ---

#[test]
fn bug_javadoc_body_line_yields_no_entity() {
    let entities = extract_entities_groovy(ISESSION_SRC, "ISession.groovy", "test-repo");
    assert!(
        !entities.iter().any(|e| e.name == "name"),
        "Phantom entity 'name' from Javadoc body line must NOT exist"
    );
    assert!(
        entities
            .iter()
            .any(|e| e.name == "getBaseDir" && e.kind == EntityKind::GroovyMethod),
        "getBaseDir must still be extracted"
    );
    assert!(
        entities
            .iter()
            .any(|e| e.name == "getScriptName" && e.kind == EntityKind::GroovyMethod),
        "getScriptName must still be extracted"
    );
}

#[test]
fn bug_bare_property_is_indexed() {
    let entities = extract_entities_groovy(SESSION_SRC, "Session.groovy", "test-repo");
    let base_dir = entities
        .iter()
        .find(|e| e.name == "baseDir" && e.kind == EntityKind::GroovyProperty);
    assert!(
        base_dir.is_some(),
        "Bare property 'baseDir' must be indexed"
    );
    assert_eq!(
        base_dir.unwrap().fqn,
        "nf.Session.baseDir",
        "FQN should include enclosing class"
    );
}

#[test]
fn bug_property_getter_is_synthesised() {
    let entities = extract_entities_groovy(SESSION_SRC, "Session.groovy", "test-repo");
    let getter = find_method_in_class(&entities, "getBaseDir", "Session");
    assert!(
        getter.is_some(),
        "Synthetic getter 'Session.getBaseDir' must exist"
    );
    assert!(
        getter
            .unwrap()
            .signature
            .as_deref()
            .is_some_and(|s| s.contains("synthetic")),
        "Synthetic getter must carry a synthetic marker in its signature"
    );
}

#[test]
fn bug_groovy_scm_query_compiles() {
    let q = tree_sitter::Query::new(
        &tree_sitter_groovy::LANGUAGE.into(),
        include_str!("../../../../../queries/groovy.scm"),
    );
    assert!(q.is_ok(), "groovy.scm failed to compile: {:?}", q.err());
}

#[test]
fn groovy_scm_captures_expected_patterns() {
    let q = tree_sitter::Query::new(
        &tree_sitter_groovy::LANGUAGE.into(),
        include_str!("../../../../../queries/groovy.scm"),
    )
    .expect("groovy.scm must compile");
    assert!(
        q.pattern_count() >= 12,
        "expected at least 12 patterns, got {}",
        q.pattern_count()
    );
    let required: &[&str] = &[
        "groovy.method.name",
        "groovy.field.name",
        "groovy.class.name",
        "groovy.interface.name",
        "groovy.enum.name",
        "groovy.signature",
    ];
    let capture_names: Vec<String> = q.capture_names().iter().map(|c| c.to_string()).collect();
    for name in required {
        assert!(
            capture_names.iter().any(|c| c == name),
            "capture '{name}' missing from groovy.scm"
        );
    }
}

// --- Phase 1: Comment stripping regression ---

#[test]
fn javadoc_body_with_parens_is_not_a_method() {
    let entities = extract_entities_groovy(ISESSION_SRC, "ISession.groovy", "test-repo");
    for e in &entities {
        if e.kind == EntityKind::GroovyMethod {
            assert!(
                !e.signature
                    .as_deref()
                    .is_some_and(|s| s.contains("parent path")),
                "Javadoc body line '{}' must not be a method entity: {:?}",
                e.name,
                e.signature
            );
        }
    }
    // Confirm the real methods are intact
    assert!(
        entities
            .iter()
            .any(|e| e.name == "getBaseDir" && e.kind == EntityKind::GroovyMethod)
    );
    assert!(
        entities
            .iter()
            .any(|e| e.name == "getScriptName" && e.kind == EntityKind::GroovyMethod)
    );
}

#[test]
fn javadoc_body_does_not_shadow_next_declaration() {
    let entities = extract_entities_groovy(SESSION_SRC, "Session.groovy", "test-repo");
    let setter = find_method_in_class(&entities, "setBaseDir", "Session");
    assert!(
        setter.is_some(),
        "setBaseDir must be present with enclosing_class=Session"
    );
}

#[test]
fn braces_inside_block_comment_do_not_corrupt_scope() {
    let source = r#"
class MyService {
    /**
     * Example: if (x) { doSomething() }
     */
    String getName() { "svc" }
}
"#;
    let entities = extract_entities_groovy(source, "MyService.groovy", "test-repo");
    let method = entities
        .iter()
        .find(|e| e.name == "getName" && e.kind == EntityKind::GroovyMethod)
        .expect("getName not found");
    assert_eq!(
        method.enclosing_class.as_deref(),
        Some("MyService"),
        "method's enclosing_class must be the class, not None"
    );
}

#[test]
fn single_line_block_comment_does_not_leak() {
    let source = "class Foo { /* note */ void run() {} }";
    let entities = extract_entities_groovy(source, "test.groovy", "test-repo");
    assert!(
        entities
            .iter()
            .any(|e| e.name == "Foo" && e.kind == EntityKind::GroovyClass),
        "Foo should be extracted"
    );
}

#[test]
fn trailing_line_comment_is_ignored() {
    let source = "class Foo {\n    Path baseDir // the base dir\n}";
    let entities = extract_entities_groovy(source, "test.groovy", "test-repo");
    assert!(
        entities
            .iter()
            .any(|e| e.name == "baseDir" && e.kind == EntityKind::GroovyProperty),
        "baseDir with trailing line comment should be extracted"
    );
}

#[test]
fn unterminated_block_comment_swallows_rest_of_file() {
    let source = "class Foo {\n/**\nPath baseDir\nString name\n}";
    let entities = extract_entities_groovy(source, "test.groovy", "test-repo");
    assert!(
        !entities.iter().any(|e| e.name == "baseDir"),
        "entities after unterminated /** should not exist"
    );
    assert!(
        !entities.iter().any(|e| e.name == "name"),
        "entities after unterminated /** should not exist"
    );
}

#[test]
fn strip_comments_line_unit() {
    // Table-driven tests for the comment-stripping helper.
    let cases: &[(&str, bool, &str, bool)] = &[
        // (input, in_block_before, expected_output, in_block_after)
        ("code", false, "code", false),
        ("code // comment", false, "code", false),
        ("/* block */ code", false, "code", false),
        ("/* start", false, "", true),
        ("* mid", true, "", true),
        ("*/ after", true, "after", false),
        ("/** doc */", false, "", false),
        ("  ", false, "", false),
        (
            "x = \"// not a comment\"",
            false,
            "x = \"// not a comment\"",
            false,
        ),
        (
            "x = \"a\\\"b\" // escaped quote",
            false,
            "x = \"a\\\"b\"",
            false,
        ),
    ];
    for (i, (input, in_before, expected, in_after)) in cases.iter().enumerate() {
        let mut in_block = *in_before;
        let result = strip_comments_line(input, &mut in_block);
        assert_eq!(
            result.trim(),
            *expected,
            "case {i}: strip_comments_line({input:?}, {in_before})"
        );
        assert_eq!(
            in_block, *in_after,
            "case {i}: in_block after strip_comments_line"
        );
    }
}

// --- Phase 2: Bare property declarations ---

#[test]
fn bare_typed_property_is_extracted() {
    let source = "class Session {\n    Path baseDir\n}";
    let entities = extract_entities_groovy(source, "Session.groovy", "test-repo");
    let prop = pick_entity(&entities, "baseDir", EntityKind::GroovyProperty);
    assert_eq!(prop.enclosing_class.as_deref(), Some("Session"));
    assert_eq!(prop.fqn, "Session.baseDir");
}

#[test]
fn generic_typed_property_is_extracted() {
    let source = "class Session {\n    Map<String,Object> config\n}";
    let entities = extract_entities_groovy(source, "Session.groovy", "test-repo");
    assert!(
        entities
            .iter()
            .any(|e| e.name == "config" && e.kind == EntityKind::GroovyProperty),
        "generic property 'config' not found"
    );
}

#[test]
fn def_property_is_extracted() {
    let source = "class Session {\n    def anything\n}";
    let entities = extract_entities_groovy(source, "Session.groovy", "test-repo");
    assert!(
        entities
            .iter()
            .any(|e| e.name == "anything" && e.kind == EntityKind::GroovyProperty),
        "def property 'anything' not found"
    );
}

#[test]
fn modifier_prefixed_property_is_extracted() {
    let source = "class Session {\n    private static final Path ROOT\n}";
    let entities = extract_entities_groovy(source, "Session.groovy", "test-repo");
    assert!(
        entities
            .iter()
            .any(|e| e.name == "ROOT" && e.kind == EntityKind::GroovyProperty),
        "modifier-prefixed property 'ROOT' not found"
    );
}

#[test]
fn java_style_semicolon_field_is_extracted() {
    let source = "class Session {\n    private int count;\n}";
    let entities = extract_entities_groovy(source, "Session.groovy", "test-repo");
    assert!(
        entities
            .iter()
            .any(|e| e.name == "count" && e.kind == EntityKind::GroovyProperty),
        "semicolon field 'count' not found"
    );
}

#[test]
fn initialized_property_still_extracted() {
    let source = "class Foo { String name = 'test' }";
    let entities = extract_entities_groovy(source, "test.groovy", "test-repo");
    assert!(
        entities
            .iter()
            .any(|e| e.name == "name" && e.kind == EntityKind::GroovyProperty),
        "initialized property 'name' not found"
    );
}

#[test]
fn local_variable_inside_method_is_not_a_property() {
    let source = "class Foo { void m() { Path tmp\n } }";
    let entities = extract_entities_groovy(source, "test.groovy", "test-repo");
    assert!(
        !entities.iter().any(|e| e.name == "tmp"),
        "local variable 'tmp' inside method must NOT be a property"
    );
}

#[test]
fn return_statement_is_not_a_property() {
    let source = "class Foo { void m() { return baseDir } }";
    let entities = extract_entities_groovy(source, "test.groovy", "test-repo");
    assert!(
        !entities.iter().any(|e| e.name == "return"),
        "'return' must not be a property"
    );
}

#[test]
fn import_and_package_lines_are_not_properties() {
    let source = "package com.foo\nimport java.nio.Path\nclass Foo { }";
    let entities = extract_entities_groovy(source, "test.groovy", "test-repo");
    // import/java/nio/Path should not be properties
    assert!(
        !entities
            .iter()
            .any(|e| e.name == "Path" && e.kind == EntityKind::GroovyProperty),
        "'Path' from import must not be a property"
    );
}

#[test]
fn type_declaration_line_is_not_a_property() {
    let source = "class Session implements ISession { String name }";
    let entities = extract_entities_groovy(source, "test.groovy", "test-repo");
    // class/Session/implements/ISession should not be properties
    assert!(
        !entities
            .iter()
            .any(|e| e.name == "Session" && e.kind == EntityKind::GroovyProperty),
        "class name must not be misclassified as property"
    );
}

#[test]
fn script_level_bare_identifier_is_not_a_property() {
    let source = "println";
    let entities = extract_entities_groovy(source, "test.groovy", "test-repo");
    assert!(
        !entities
            .iter()
            .any(|e| e.name == "println" && e.kind == EntityKind::GroovyProperty),
        "single token 'println' must not be a property"
    );
}

// --- Phase 3: Synthetic accessor entities ---

#[test]
fn property_generates_getter_and_setter() {
    let source = "class Session {\n    Path baseDir\n}";
    let entities = extract_entities_groovy(source, "Session.groovy", "test-repo");
    let getter = find_method_in_class(&entities, "getBaseDir", "Session");
    assert!(getter.is_some(), "getter 'getBaseDir' must be synthesised");
    let setter = find_method_in_class(&entities, "setBaseDir", "Session");
    assert!(setter.is_some(), "setter 'setBaseDir' must be synthesised");
}

#[test]
fn boolean_property_generates_is_and_get() {
    let source = "class Session {\n    boolean cacheable\n}";
    let entities = extract_entities_groovy(source, "Session.groovy", "test-repo");
    assert!(
        find_method_in_class(&entities, "isCacheable", "Session").is_some(),
        "boolean is-accessor not synthesised"
    );
    assert!(
        find_method_in_class(&entities, "getCacheable", "Session").is_some(),
        "boolean getter not synthesised"
    );
}

#[test]
fn boxed_boolean_property_generates_is_and_get() {
    let source = "class Session {\n    Boolean resumeMode\n}";
    let entities = extract_entities_groovy(source, "Session.groovy", "test-repo");
    assert!(
        entities.iter().any(|e| e.name == "isResumeMode"),
        "Boolean is-accessor not synthesised"
    );
    assert!(
        entities.iter().any(|e| e.name == "getResumeMode"),
        "Boolean getter not synthesised"
    );
}

#[test]
fn final_property_generates_getter_only() {
    let source = "class Session {\n    final Path root\n}";
    let entities = extract_entities_groovy(source, "Session.groovy", "test-repo");
    assert!(
        entities
            .iter()
            .any(|e| e.name == "getRoot" && e.kind == EntityKind::GroovyMethod),
        "final property must have getter"
    );
    assert!(
        !entities
            .iter()
            .any(|e| e.name == "setRoot" && e.kind == EntityKind::GroovyMethod),
        "final property must NOT have setter"
    );
}

#[test]
fn explicit_setter_suppresses_synthetic_setter() {
    let entities = extract_entities_groovy(SESSION_SRC, "Session.groovy", "test-repo");
    let setters: Vec<_> = entities
        .iter()
        .filter(|e| e.name == "setBaseDir" && e.kind == EntityKind::GroovyMethod)
        .collect();
    assert_eq!(
        setters.len(),
        1,
        "must be exactly one setBaseDir, got {}",
        setters.len()
    );
    let s = setters[0];
    assert!(
        !s.signature
            .as_deref()
            .is_some_and(|sig| sig.contains("synthetic")),
        "the setBaseDir must be the real one, not synthetic"
    );
}

#[test]
fn explicit_getter_suppresses_synthetic_getter() {
    let source = r#"
class Session {
    Path baseDir
    Path getBaseDir() { baseDir }
}
"#;
    let entities = extract_entities_groovy(source, "Session.groovy", "test-repo");
    let getters: Vec<_> = entities
        .iter()
        .filter(|e| e.name == "getBaseDir" && e.kind == EntityKind::GroovyMethod)
        .collect();
    assert_eq!(getters.len(), 1, "exactly one getBaseDir expected");
}

#[test]
fn interface_constant_generates_no_accessor() {
    let source = "interface I { String NAME }";
    let entities = extract_entities_groovy(source, "I.groovy", "test-repo");
    assert!(
        !entities.iter().any(|e| e.name == "getNAME"),
        "interface constants must not generate accessors"
    );
}

#[test]
fn script_level_variable_generates_no_accessor() {
    let source = "def globalConfig = [:]";
    let entities = extract_entities_groovy(source, "script.groovy", "test-repo");
    assert!(
        !entities.iter().any(|e| e.name == "getGlobalConfig"),
        "script-level variable must not generate accessor"
    );
}

#[test]
fn synthetic_accessor_metadata() {
    let entities = extract_entities_groovy(SESSION_SRC, "Session.groovy", "test-repo");
    let getter =
        find_synthetic_accessor(&entities, "getBaseDir").expect("synthetic getter not found");
    let prop = entities
        .iter()
        .find(|e| e.name == "baseDir" && e.kind == EntityKind::GroovyProperty)
        .expect("baseDir property not found");
    assert_eq!(
        getter.enclosing_class.as_deref(),
        Some("Session"),
        "synthetic getter must have enclosing class"
    );
    assert_eq!(
        getter.start_line, prop.start_line,
        "synthetic getter must share property's start_line"
    );
}

#[test]
fn synthetic_accessor_uuid_is_distinct() {
    let entities = extract_entities_groovy(SESSION_SRC, "Session.groovy", "test-repo");
    let getter =
        find_synthetic_accessor(&entities, "getBaseDir").expect("synthetic getter not found");
    let prop = entities
        .iter()
        .find(|e| e.name == "baseDir" && e.kind == EntityKind::GroovyProperty)
        .expect("baseDir property not found");
    assert_ne!(
        getter.uuid, prop.uuid,
        "synthetic getter UUID must be distinct from property UUID"
    );
}

#[test]
fn synthetic_accessors_have_no_reference_intents() {
    let entities = extract_entities_groovy(SESSION_SRC, "Session.groovy", "test-repo");
    for e in entities.iter().filter(|e| {
        e.kind == EntityKind::GroovyMethod
            && e.signature
                .as_deref()
                .is_some_and(|s| s.contains("synthetic"))
    }) {
        assert!(
            e.reference_intents.is_empty(),
            "synthetic accessor '{}' must have no reference intents",
            e.name
        );
    }
}

#[test]
fn url_string_with_double_slash_is_tolerated() {
    // Pinning test: current comment stripping may handle this imperfectly.
    // This test records behavior — it must not panic.
    let source = "class Foo {\n    String url = \"https://example.com/path\"\n}";
    let entities = extract_entities_groovy(source, "test.groovy", "test-repo");
    assert!(
        entities
            .iter()
            .any(|e| e.name == "url" && e.kind == EntityKind::GroovyProperty),
        "url property should still be extracted"
    );
}

#[test]
fn test_all_typed_methods_no_duplication() {
    // Both methods typed → tree-sitter finds both, ad-hoc must NOT duplicate (Fix 3: known_lines)
    let source = r#"
class HttpUtil {
    private static void restartHttpServer() {
        println "hello"
    }
    void loadIntoHttpServer(String html) {
        restartHttpServer()
    }
}
"#;
    let entities = extract_entities_groovy(source, "HttpUtil.groovy", "test-repo");
    let r_count = entities
        .iter()
        .filter(|e| e.name == "restartHttpServer")
        .count();
    let l_count = entities
        .iter()
        .filter(|e| e.name == "loadIntoHttpServer")
        .count();
    assert_eq!(r_count, 1, "restartHttpServer duplicated");
    assert_eq!(l_count, 1, "loadIntoHttpServer duplicated");
}

#[test]
fn test_def_methods_call_typed_private_method() {
    // Simulates LLM scenario: def method calling private typed method
    let source = r#"
class HttpUtil {
    private static void restartHttpServer() {
        println "hello"
    }
    def loadIntoHttpServer(String html) {
        restartHttpServer()
    }
}
"#;
    let entities = extract_entities_groovy(source, "HttpUtil.groovy", "test-repo");
    let load = entities.iter().find(|e| e.name == "loadIntoHttpServer");
    assert!(load.is_some(), "loadIntoHttpServer not found");
    let load = load.unwrap();
    let calls_to_restart = load
        .reference_intents
        .iter()
        .filter(
            |r| matches!(r, ReferenceIntent::Call { method, .. } if method == "restartHttpServer"),
        )
        .count();
    assert!(
        calls_to_restart > 0,
        "Expected def method to have CALL to restartHttpServer"
    );
}

#[test]
fn test_no_paren_call_detection() {
    // Fix 2: Groovy no-paren call style: runAnalyzer "abc", 123 and doSomething arg1
    let source = r#"
class Worker {
    void process() {
        runAnalyzer "abc", 123
        doSomething result
        println "hello"
    }
}
"#;
    let entities = extract_entities_groovy(source, "Worker.groovy", "test-repo");
    let process = entities
        .iter()
        .find(|e| e.name == "process")
        .expect("process not found");
    let refs: Vec<String> = process
        .reference_intents
        .iter()
        .map(|r| match r {
            ReferenceIntent::Call {
                method,
                receiver,
                line,
                arg_count: _,
            } => format!(
                "Call({}{}, line {})",
                receiver
                    .as_ref()
                    .map(|r| format!("{}.", r))
                    .unwrap_or_default(),
                method,
                line
            ),
            _ => format!("{:?}", r),
        })
        .collect();
    eprintln!("process reference_intents: {:?}", refs);

    // runAnalyzer "abc", 123 — no-paren call with string arg
    let has_run = process
        .reference_intents
        .iter()
        .any(|r| matches!(r, ReferenceIntent::Call { method, .. } if method == "runAnalyzer"));
    assert!(has_run);

    // doSomething result — no-paren call with identifier arg
    let has_do = process
        .reference_intents
        .iter()
        .any(|r| matches!(r, ReferenceIntent::Call { method, .. } if method == "doSomething"));
    assert!(has_do);

    // println — must NOT be captured (it's a keyword)
    let has_println = process
        .reference_intents
        .iter()
        .any(|r| matches!(r, ReferenceIntent::Call { method, .. } if method == "println"));
    assert!(!has_println);
}

#[test]
fn test_receiver_call_detection() {
    let source = r#"
class Service {
    void run() {
        obj.compute()
    }
}
"#;
    let entities = extract_entities_groovy(source, "Service.groovy", "test-repo");
    let run = entities
        .iter()
        .find(|e| e.name == "run")
        .expect("run not found");
    let has_recv_call = run.reference_intents.iter().any(|r| {
        matches!(r, ReferenceIntent::Call { method, receiver: Some(rec), .. } if method == "compute" && rec == "obj")
    });
    assert!(
        has_recv_call,
        "Expected Call intent with receiver 'obj' for method 'compute'"
    );
}

#[test]
fn test_non_ascii_trailing_comment_does_not_panic() {
    let source = r#"
class Test {
    void run() {
        def x = 1 // esto está mal
    }
}
"#;
    let entities = extract_entities_groovy(source, "Test.groovy", "test-repo");
    assert!(!entities.is_empty());
}

#[test]
fn test_private_method_with_closure_args_is_callable() {
    // Replicates exact pattern from HttpUtil.groovy in code-history-mining:
    // private static method with closure args, called from a public static method
    let source = r#"
package test
import com.example.SimpleHttpServer
class HttpUtil {
    static String loadIntoHttpServer(String html) {
        def server = restartHttpServer("web", "/tmp", {null}, {log?.errorOnHttpRequest(it.toString())})
        "http://localhost"
    }

    private static SimpleHttpServer restartHttpServer(String id, String webRootPath,
                                                       Closure handler = {null},
                                                       Closure errorListener = {}) {
        def server = new SimpleHttpServer()
        server
    }
}
"#;
    let entities = extract_entities_groovy(source, "HttpUtil.groovy", "test-repo");

    let load = entities.iter().find(|e| e.name == "loadIntoHttpServer");
    assert!(load.is_some(), "loadIntoHttpServer not found");
    let load = load.unwrap();
    assert_eq!(
        load.enclosing_class.as_deref(),
        Some("HttpUtil"),
        "loadIntoHttpServer should have enclosing_class HttpUtil"
    );
    assert!(
        !load.fqn.is_empty(),
        "loadIntoHttpServer should have non-empty FQN, got: '{}'",
        load.fqn
    );

    let calls_restart = load
        .reference_intents
        .iter()
        .filter(
            |r| matches!(r, ReferenceIntent::Call { method, .. } if method == "restartHttpServer"),
        )
        .count();
    assert!(
        calls_restart > 0,
        "Expected loadIntoHttpServer to have CALL to restartHttpServer, but found {} call(s). refs: {:?}",
        calls_restart,
        load.reference_intents
            .iter()
            .filter_map(|r| match r {
                ReferenceIntent::Call { method, line, .. } => Some(format!("{}@L{}", method, line)),
                _ => None,
            })
            .collect::<Vec<_>>()
    );

    let restart = entities.iter().find(|e| e.name == "restartHttpServer");
    assert!(restart.is_some(), "restartHttpServer not found in entities");
    let restart = restart.unwrap();
    assert_eq!(
        restart.enclosing_class.as_deref(),
        Some("HttpUtil"),
        "restartHttpServer should have enclosing_class HttpUtil"
    );
    assert!(
        !restart.fqn.is_empty(),
        "restartHttpServer should have non-empty FQN, got: '{}'",
        restart.fqn
    );
    assert!(
        restart.enclosing_class.is_some(),
        "restartHttpServer should have enclosing_class set"
    );
}

#[test]
fn test_new_constructor_not_method_declaration() {
    // `new File(...).write(...)` and `new SimpleHttpServer()` are constructor calls,
    // NOT method declarations. They should not create spurious method entities.
    let source = r#"
class HttpUtil {
    static String loadIntoHttpServer(String html) {
        def tempDir = FileUtil.createTempDirectory("proj", "")
        new File("path").write(html)
        def server = restartHttpServer("web", "/tmp", {null}, {log?.errorOnHttpRequest(it.toString())})
        "http://localhost"
    }
    private static SimpleHttpServer restartHttpServer(String id, String webRootPath,
                                                       Closure handler = {null},
                                                       Closure errorListener = {}) {
        def server = new SimpleHttpServer()
        server
    }
}
"#;
    let entities = extract_entities_groovy(source, "HttpUtil.groovy", "test-repo");

    // `File` must NOT appear as a method entity
    assert!(
        !entities
            .iter()
            .any(|e| e.kind == EntityKind::GroovyMethod && e.name == "File"),
        "new File(...) was incorrectly extracted as a method declaration"
    );

    // `SimpleHttpServer` constructor call inside method body must NOT appear as method
    // (allow the one at the return type position in the private method signature via multi-line though)
    let ssh_methods: Vec<_> = entities
        .iter()
        .filter(|e| e.kind == EntityKind::GroovyMethod && e.name == "SimpleHttpServer")
        .collect();
    assert!(
        ssh_methods.len() <= 1,
        "new SimpleHttpServer() constructor should not create method entities, found {}: {:?}",
        ssh_methods.len(),
        ssh_methods.iter().map(|e| e.start_line).collect::<Vec<_>>()
    );

    // restartHttpServer should be callable from loadIntoHttpServer
    let load = entities
        .iter()
        .find(|e| e.name == "loadIntoHttpServer")
        .expect("loadIntoHttpServer not found");
    let calls_restart = load
        .reference_intents
        .iter()
        .filter(
            |r| matches!(r, ReferenceIntent::Call { method, .. } if method == "restartHttpServer"),
        )
        .count();
    assert!(
        calls_restart > 0,
        "loadIntoHttpServer should call restartHttpServer"
    );
}
