//! Unit tests for the C# parser (plan §7–§10, BDD: tests define the contract).
//!
//! Coverage:
//! - Entity extraction per kind (§7) — the grammar-gap cases (fields, const,
//!   records, indexer, operator, local functions) included.
//! - FQN construction across namespace forms (§8.1).
//! - References: inheritance heuristic, attributes, type refs, call intents
//!   with receiver substitution (§9).
//! - OVERRIDES end-to-end via `resolve_reference_intents` (§10.1).
//! - Orphan handling for top-level statements (§13).

use super::refs::{
    extract_attribute_references, extract_class_inheritance_csharp,
    extract_reference_intents_csharp,
};
use super::{build_csharp_fqn_prefix, extract_file_scoped_namespace};
use crate::models::{EntityKind, ReferenceIntent, RelationshipType, ResolutionEntity};
use crate::pipeline::parser::DEFAULT_CSHARP_QUERY;
use crate::pipeline::parser::context::{ClassContext, extract_class_contexts};
use crate::pipeline::parser::extractor::extract_entities;
use crate::pipeline::parser::test_utils::{
    collect_extends, collect_implements, find_first_node, parse_csharp_snippet,
};
use tree_sitter::Node;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Run the full query-driven extraction on a snippet, as `parse_single_file`
/// would for a `.cs` file.
fn extract(code: &str) -> Vec<crate::models::ParsedEntity> {
    extract_entities(
        code,
        tree_sitter_c_sharp::LANGUAGE.into(),
        DEFAULT_CSHARP_QUERY,
        "csharp",
        "Test.cs",
        "test-repo",
    )
    .expect("C# extraction failed")
}

fn find_entity<'a>(
    entities: &'a [crate::models::ParsedEntity],
    name: &str,
    kind: &EntityKind,
) -> &'a crate::models::ParsedEntity {
    entities
        .iter()
        .find(|e| e.name == name && e.kind == *kind)
        .unwrap_or_else(|| panic!("entity {name} of kind {kind} not found"))
}

fn entities_of_kind<'a>(
    entities: &'a [crate::models::ParsedEntity],
    kind: &EntityKind,
) -> Vec<&'a str> {
    entities
        .iter()
        .filter(|e| e.kind == *kind)
        .map(|e| e.name.as_str())
        .collect()
}

fn find_node<'a>(root: Node<'a>, kind: &str, name: &str, source: &[u8]) -> Node<'a> {
    let mut stack = vec![root];
    while let Some(n) = stack.pop() {
        if n.kind() == kind
            && let Some(name_node) = n.child_by_field_name("name")
            && name_node.utf8_text(source).unwrap_or("") == name
        {
            return n;
        }
        let mut child = n.child(0);
        while let Some(c) = child {
            stack.push(c);
            child = c.next_sibling();
        }
    }
    panic!("node {kind} named {name} not found");
}

// ---------------------------------------------------------------------------
// Phase 1 — discovery / snippet parsing
// ---------------------------------------------------------------------------

#[test]
fn test_parse_csharp_snippet_parses_class() {
    let tree = parse_csharp_snippet("class Foo {}").expect("parse failed");
    assert!(!tree.root_node().has_error());
}

// ---------------------------------------------------------------------------
// Phase 2 — entity extraction
// ---------------------------------------------------------------------------

#[test]
fn test_extract_class_interface_struct_enum() {
    let code = r#"
namespace App;
public class Foo {}
public interface IBar {}
public struct Baz {}
public enum Qux { A, B }
"#;
    let entities = extract(code);
    assert_eq!(
        entities_of_kind(&entities, &EntityKind::CSharpClass),
        ["Foo"]
    );
    assert_eq!(
        entities_of_kind(&entities, &EntityKind::CSharpInterface),
        ["IBar"]
    );
    assert_eq!(
        entities_of_kind(&entities, &EntityKind::CSharpStruct),
        ["Baz"]
    );
    assert_eq!(
        entities_of_kind(&entities, &EntityKind::CSharpEnum),
        ["Qux"]
    );
}

#[test]
fn test_extract_record_class_and_record_struct_are_records() {
    // Grammar gap (§2.3, Gap 3): both flavors share `record_declaration`
    // and surface as CSharpRecord.
    let code = r#"
namespace App;
public record Point(int X, int Y);
public record struct Coord(double Lat, double Lon);
"#;
    let entities = extract(code);
    let records = entities_of_kind(&entities, &EntityKind::CSharpRecord);
    assert_eq!(records.len(), 2, "both record flavours are records");
    assert!(records.contains(&"Point"));
    assert!(records.contains(&"Coord"));
    assert!(
        !entities_of_kind(&entities, &EntityKind::CSharpStruct).contains(&"Coord"),
        "record struct must not double-extract as struct"
    );
}

#[test]
fn test_extract_method_property_constructor() {
    let code = r#"
namespace App;
public class Foo
{
    public int Bar { get; set; }
    public Foo() {}
    public void DoWork() {}
}
"#;
    let entities = extract(code);
    assert_eq!(
        entities_of_kind(&entities, &EntityKind::CSharpProperty),
        ["Bar"]
    );
    assert_eq!(
        entities_of_kind(&entities, &EntityKind::CSharpConstructor),
        ["Foo"]
    );
    assert_eq!(
        entities_of_kind(&entities, &EntityKind::CSharpMethod),
        ["DoWork"]
    );
}

#[test]
fn test_extract_field_and_const_detection() {
    // Grammar gap (§2.3, Gap 2): field names live in variable_declarator;
    // `const` promotes the kind.
    let code = r#"
namespace App;
public class Foo
{
    public const int Max = 3;
    private readonly string _name;
}
"#;
    let entities = extract(code);
    assert_eq!(
        entities_of_kind(&entities, &EntityKind::CSharpConstant),
        ["Max"],
        "const field must be a CSharpConstant"
    );
    assert_eq!(
        entities_of_kind(&entities, &EntityKind::CSharpField),
        ["_name"],
        "readonly field must be a CSharpField"
    );
}

#[test]
fn test_extract_delegate_event_indexer_operator() {
    let code = r#"
namespace App;
public class Foo
{
    public delegate void Handler(string msg);
    public event Handler? OnEvent;
    public string this[int i] { get => ""; set { } }
    public static Foo operator +(Foo a, Foo b) => a;
}
"#;
    let entities = extract(code);
    assert_eq!(
        entities_of_kind(&entities, &EntityKind::CSharpDelegate),
        ["Handler"]
    );
    assert_eq!(
        entities_of_kind(&entities, &EntityKind::CSharpEvent),
        ["OnEvent"],
        "event-field form must extract the declarator name"
    );
    assert_eq!(
        entities_of_kind(&entities, &EntityKind::CSharpIndexer),
        ["this[]"],
        "indexer name is synthesised (no grammar name field)"
    );
    assert_eq!(
        entities_of_kind(&entities, &EntityKind::CSharpOperator),
        ["operator +"],
        "operator name is synthesised from the operator token"
    );
}

#[test]
fn test_extract_local_function() {
    let code = r#"
namespace App;
public static class Extensions
{
    public static string Slugify(this string value)
    {
        string Normalize(string raw) => raw.Trim();
        return Normalize(value);
    }
}
"#;
    let entities = extract(code);
    assert_eq!(
        entities_of_kind(&entities, &EntityKind::CSharpMethod),
        ["Slugify"]
    );
    assert_eq!(
        entities_of_kind(&entities, &EntityKind::CSharpLocalFunction),
        ["Normalize"]
    );
}

#[test]
fn test_extract_block_and_file_scoped_namespaces() {
    let code = r#"
namespace App.Deep
{
    public class Foo {}
}
namespace App.Top;
public class Bar {}
"#;
    let entities = extract(code);
    let namespaces = entities_of_kind(&entities, &EntityKind::CSharpNamespace);
    assert_eq!(namespaces.len(), 2);
    assert!(namespaces.contains(&"App.Deep"));
    assert!(namespaces.contains(&"App.Top"));
}

#[test]
fn test_extract_class_contexts_includes_struct_record_enum() {
    // context.rs wiring: struct/record/enum declarations establish class
    // contexts so members inside them get qualified FQNs.
    let code = r#"
public struct S { public int X { get; set; } }
public record R { public int Y { get; set; } }
public enum E { A }
"#;
    let tree = parse_csharp_snippet(code).expect("parse failed");
    let mut contexts: Vec<ClassContext> = Vec::new();
    extract_class_contexts(tree.root_node(), code.as_bytes(), &mut contexts);
    assert!(
        contexts.iter().any(|c| c.name == "S"),
        "struct must be a class context"
    );
    assert!(
        contexts.iter().any(|c| c.name == "R"),
        "record must be a class context"
    );
    assert!(
        contexts.iter().any(|c| c.name == "E"),
        "enum must be a class context"
    );
}

// ---------------------------------------------------------------------------
// Phase 3 — namespaces and FQN (§8.1)
// ---------------------------------------------------------------------------

#[test]
fn test_fqn_file_scoped_namespace() {
    let code = "namespace MyApp.Services;\npublic class UserService {}";
    let entities = extract(code);
    let e = find_entity(&entities, "UserService", &EntityKind::CSharpClass);
    assert_eq!(e.fqn, "MyApp.Services.UserService");
}

#[test]
fn test_fqn_block_namespace() {
    let code = "namespace MyApp\n{\n    public class Foo {}\n}";
    let entities = extract(code);
    let e = find_entity(&entities, "Foo", &EntityKind::CSharpClass);
    assert_eq!(e.fqn, "MyApp.Foo");
}

#[test]
fn test_fqn_nested_block_namespace() {
    let code = r#"
namespace MyApp.Legacy
{
    namespace Deep
    {
        public class OldStyle {}
    }
}
"#;
    let entities = extract(code);
    let e = find_entity(&entities, "OldStyle", &EntityKind::CSharpClass);
    assert_eq!(e.fqn, "MyApp.Legacy.Deep.OldStyle");
}

#[test]
fn test_fqn_no_namespace() {
    let code = "public class Free {}";
    let entities = extract(code);
    let e = find_entity(&entities, "Free", &EntityKind::CSharpClass);
    assert_eq!(e.fqn, "Free");
}

#[test]
fn test_fqn_nested_type_in_type() {
    let code = r#"
namespace MyApp.Domain;
public class Container
{
    public class Nested {}
}
"#;
    let entities = extract(code);
    let e = find_entity(&entities, "Nested", &EntityKind::CSharpClass);
    assert_eq!(e.fqn, "MyApp.Domain.Container.Nested");
}

#[test]
fn test_fqn_nested_type_in_nested_namespace() {
    let code = r#"
namespace MyApp.Legacy
{
    namespace Deep
    {
        public class OldStyle
        {
            public class Inner {}
        }
    }
}
"#;
    let entities = extract(code);
    let e = find_entity(&entities, "Inner", &EntityKind::CSharpClass);
    assert_eq!(e.fqn, "MyApp.Legacy.Deep.OldStyle.Inner");
}

#[test]
fn test_fqn_method_under_file_scoped_namespace() {
    let code = r#"
namespace MyApp.Services;
public class UserService
{
    public Task<Dto> GetUserAsync(int id) { return null; }
}
"#;
    let entities = extract(code);
    let e = find_entity(&entities, "GetUserAsync", &EntityKind::CSharpMethod);
    assert_eq!(e.fqn, "MyApp.Services.UserService.GetUserAsync");
    assert_eq!(e.enclosing_class.as_deref(), Some("UserService"));
    assert_eq!(
        e.enclosing_class_fqn.as_deref(),
        Some("MyApp.Services.UserService"),
        "enclosing_class_fqn enables the exact CONTAINS auto-link"
    );
}

#[test]
fn test_fqn_member_of_nested_type() {
    let code = r#"
namespace App;
public class Outer
{
    public class Inner
    {
        public void Run() {}
    }
}
"#;
    let entities = extract(code);
    let e = find_entity(&entities, "Run", &EntityKind::CSharpMethod);
    assert_eq!(e.fqn, "App.Outer.Inner.Run");
}

// ---------------------------------------------------------------------------
// Phase 4 — references (§9)
// ---------------------------------------------------------------------------

#[test]
fn test_inheritance_class_extends_and_implements() {
    let code = "class UserService : BaseService, IUserService {}";
    let tree = parse_csharp_snippet(code).unwrap();
    let class_node = find_first_node(tree.root_node(), &["class_declaration"]).unwrap();
    let mut intents = Vec::new();
    extract_class_inheritance_csharp(class_node, code.as_bytes(), &mut intents);
    assert_eq!(collect_extends(&intents), ["BaseService"]);
    assert_eq!(collect_implements(&intents), ["IUserService"]);
}

#[test]
fn test_inheritance_class_leading_interface_goes_to_implements() {
    // The §3.3 heuristic: a first entry matching `^I[A-Z]` is an interface.
    let code = "class UserRepository : IRepository {}";
    let tree = parse_csharp_snippet(code).unwrap();
    let class_node = find_first_node(tree.root_node(), &["class_declaration"]).unwrap();
    let mut intents = Vec::new();
    extract_class_inheritance_csharp(class_node, code.as_bytes(), &mut intents);
    assert!(collect_extends(&intents).is_empty());
    assert_eq!(collect_implements(&intents), ["IRepository"]);
}

#[test]
fn test_inheritance_interface_extends_everything() {
    let code = "interface IAdminRepository : IRepository<User>, IDisposable {}";
    let tree = parse_csharp_snippet(code).unwrap();
    let iface = find_first_node(tree.root_node(), &["interface_declaration"]).unwrap();
    let mut intents = Vec::new();
    extract_class_inheritance_csharp(iface, code.as_bytes(), &mut intents);
    assert_eq!(collect_extends(&intents), ["IRepository", "IDisposable"]);
    assert!(collect_implements(&intents).is_empty());
}

#[test]
fn test_inheritance_struct_only_implements() {
    let code = "struct Point : IEquatable<Point> {}";
    let tree = parse_csharp_snippet(code).unwrap();
    let struct_node = find_first_node(tree.root_node(), &["struct_declaration"]).unwrap();
    let mut intents = Vec::new();
    extract_class_inheritance_csharp(struct_node, code.as_bytes(), &mut intents);
    assert_eq!(collect_implements(&intents), ["IEquatable"]);
    assert!(
        collect_extends(&intents).is_empty(),
        "a struct never extends"
    );
}

#[test]
fn test_inheritance_record_class_first_extends() {
    let code = "record Employee : Person, IComparable<Employee> {}";
    let tree = parse_csharp_snippet(code).unwrap();
    let record = find_first_node(tree.root_node(), &["record_declaration"]).unwrap();
    assert!(!super::refs::record_is_struct(record));
    let mut intents = Vec::new();
    extract_class_inheritance_csharp(record, code.as_bytes(), &mut intents);
    assert_eq!(collect_extends(&intents), ["Person"]);
    assert_eq!(collect_implements(&intents), ["IComparable"]);
}

#[test]
fn test_inheritance_record_struct_only_implements() {
    let code = "record struct Coord : IEquatable<Coord> {}";
    let tree = parse_csharp_snippet(code).unwrap();
    let record = find_first_node(tree.root_node(), &["record_declaration"]).unwrap();
    assert!(super::refs::record_is_struct(record));
    let mut intents = Vec::new();
    extract_class_inheritance_csharp(record, code.as_bytes(), &mut intents);
    assert_eq!(collect_implements(&intents), ["IEquatable"]);
    assert!(collect_extends(&intents).is_empty());
}

#[test]
fn test_inheritance_generic_arguments_stripped() {
    // v1.3.6-parity: base entries are stripped of type arguments.
    let code = "class UserRepository : IRepository<User> {}";
    let tree = parse_csharp_snippet(code).unwrap();
    let class_node = find_first_node(tree.root_node(), &["class_declaration"]).unwrap();
    let mut intents = Vec::new();
    extract_class_inheritance_csharp(class_node, code.as_bytes(), &mut intents);
    assert_eq!(collect_implements(&intents), ["IRepository"]);
}

#[test]
fn test_attribute_references_emitted_as_calls() {
    let code = r#"
[Obsolete("Use V2 instead")]
class UserService {}
"#;
    let tree = parse_csharp_snippet(code).unwrap();
    let class_node = find_first_node(tree.root_node(), &["class_declaration"]).unwrap();
    let mut intents = Vec::new();
    extract_attribute_references(class_node, code.as_bytes(), &mut intents);
    assert!(
        intents
            .iter()
            .any(|i| matches!(i, ReferenceIntent::Call { method, .. } if method == "Obsolete")),
        "attribute names must be emitted, got {intents:?}"
    );
}

#[test]
fn test_type_references_from_method_signature_and_body() {
    let code = r#"
class UserService
{
    Task<UserDto> GetUserAsync(int id)
    {
        var dto = new UserDto(id);
        return null;
    }
}
"#;
    let tree = parse_csharp_snippet(code).unwrap();
    let method = find_first_node(tree.root_node(), &["method_declaration"]).unwrap();
    let mut intents = Vec::new();
    extract_reference_intents_csharp(method, code.as_bytes(), &mut intents);
    let type_refs: Vec<&str> = intents
        .iter()
        .filter_map(|i| match i {
            ReferenceIntent::TypeReference { type_name, .. } => Some(type_name.as_str()),
            _ => None,
        })
        .collect();
    assert!(type_refs.contains(&"UserDto"), "got {type_refs:?}");
    assert!(
        intents
            .iter()
            .any(|i| matches!(i, ReferenceIntent::Call { method, .. } if method == "UserDto")),
        "object_creation_expression must emit a Call intent"
    );
}

#[test]
fn test_call_receiver_substituted_with_field_type() {
    let code = r#"
class UserService
{
    private readonly UserRepository _repository;
    void Load(int id)
    {
        var u = _repository.FindByIdAsync(id);
    }
}
"#;
    let tree = parse_csharp_snippet(code).unwrap();
    let method = find_first_node(tree.root_node(), &["method_declaration"]).unwrap();
    let mut intents = Vec::new();
    extract_reference_intents_csharp(method, code.as_bytes(), &mut intents);
    let call = intents.iter().find_map(|i| match i {
        ReferenceIntent::Call {
            method, receiver, ..
        } if method == "FindByIdAsync" => Some(receiver.clone()),
        _ => None,
    });
    assert_eq!(
        call,
        Some(Some("UserRepository".to_string())),
        "field receiver must be substituted with its declared type"
    );
}

#[test]
fn test_call_base_receiver_maps_to_super() {
    let code = r#"
class Derived : Base
{
    override void Process(string input)
    {
        base.Process(input);
    }
}
"#;
    let tree = parse_csharp_snippet(code).unwrap();
    let method = find_first_node(tree.root_node(), &["method_declaration"]).unwrap();
    let mut intents = Vec::new();
    extract_reference_intents_csharp(method, code.as_bytes(), &mut intents);
    let call = intents.iter().find_map(|i| match i {
        ReferenceIntent::Call {
            method, receiver, ..
        } if method == "Process" => Some(receiver.clone()),
        _ => None,
    });
    assert_eq!(
        call,
        Some(Some("super".to_string())),
        "C# `base.` must map to the resolver's `super` receiver"
    );
}

#[test]
fn test_orphan_pass_captures_top_level_statements() {
    // C# 9 top-level statements (§13): no enclosing entity, so the orphan
    // pass must collect the calls into a synthetic <module> entity.
    let code = r#"
using System;
Console.WriteLine("hello");
Foo();
"#;
    let entities = extract(code);
    let module = entities
        .iter()
        .find(|e| e.name == "<module>")
        .expect("orphan pass must create a synthetic <module> entity");
    assert_eq!(module.kind, EntityKind::CSharpNamespace);
    assert!(
        module
            .reference_intents
            .iter()
            .any(|i| matches!(i, ReferenceIntent::Call { method, .. } if method == "Foo")),
        "top-level call must be orphaned onto <module>, got {:?}",
        module.reference_intents
    );
}

#[test]
fn test_xml_doc_comments_captured_as_docstring() {
    let code = r#"
namespace App;
/// <summary>
/// Generic persistence abstraction.
/// </summary>
public interface IRepository
{
    /// <summary>Finds an entity by its identifier.</summary>
    Task<object> FindByIdAsync(int id);
}
"#;
    let entities = extract(code);
    let iface = find_entity(&entities, "IRepository", &EntityKind::CSharpInterface);
    assert_eq!(
        iface.docstring.as_deref(),
        Some("Generic persistence abstraction."),
        "XML doc comment must be captured with tags stripped"
    );
    let method = find_entity(&entities, "FindByIdAsync", &EntityKind::CSharpMethod);
    assert_eq!(
        method.docstring.as_deref(),
        Some("Finds an entity by its identifier.")
    );
}

#[test]
fn test_attribute_captured_in_decorators() {
    let code = r#"
[Obsolete("Use UserServiceV2 instead")]
public class UserService {}
"#;
    let entities = extract(code);
    let e = find_entity(&entities, "UserService", &EntityKind::CSharpClass);
    assert!(
        e.decorators.iter().any(|d| d.contains("Obsolete")),
        "attribute must appear in decorators, got {:?}",
        e.decorators
    );
}

// ---------------------------------------------------------------------------
// Phase 5 — OVERRIDES (§10.1), end-to-end through resolution
// ---------------------------------------------------------------------------

/// Parse two snippets as separate files and resolve their relationships.
fn resolve_together(repo_files: &[(&str, &str)]) -> Vec<ResolutionEntity> {
    let mut entities = Vec::new();
    for (path, code) in repo_files {
        let parsed = extract_entities(
            code,
            tree_sitter_c_sharp::LANGUAGE.into(),
            DEFAULT_CSHARP_QUERY,
            "csharp",
            path,
            "test-repo",
        )
        .expect("extraction failed");
        entities.extend(parsed.into_iter().map(|e| {
            let mut r = ResolutionEntity::from(&e);
            r.file_path = path.to_string();
            r
        }));
    }
    crate::pipeline::ingest::resolve_reference_intents(&mut entities);
    entities
}

fn has_rel(e: &ResolutionEntity, target: uuid::Uuid, rel: RelationshipType) -> bool {
    e.relationships.contains(&(target, rel))
}

#[test]
fn test_override_csharp_interface_impl() {
    let entities = resolve_together(&[
        (
            "IRepository.cs",
            "namespace App;\npublic interface IRepository\n{\n    Task<object> FindByIdAsync(int id);\n}",
        ),
        (
            "UserRepository.cs",
            "namespace App;\npublic class UserRepository : IRepository\n{\n    public Task<object> FindByIdAsync(int id) { return null; }\n}",
        ),
    ]);
    let iface_method = entities
        .iter()
        .find(|e| e.fqn == "App.IRepository.FindByIdAsync")
        .expect("interface method");
    let impl_method = entities
        .iter()
        .find(|e| e.fqn == "App.UserRepository.FindByIdAsync")
        .expect("impl method");
    assert!(
        has_rel(impl_method, iface_method.uuid, RelationshipType::Overrides),
        "interface implementation must produce OVERRIDES"
    );
}

#[test]
fn test_override_csharp_class_virtual() {
    let entities = resolve_together(&[
        (
            "BaseService.cs",
            "namespace App;\npublic abstract class BaseService\n{\n    public virtual string Process(string input) { return input; }\n}",
        ),
        (
            "UserService.cs",
            "namespace App;\npublic class UserService : BaseService\n{\n    public override string Process(string input) { return input; }\n}",
        ),
    ]);
    let base_method = entities
        .iter()
        .find(|e| e.fqn == "App.BaseService.Process")
        .expect("virtual method");
    let derived_method = entities
        .iter()
        .find(|e| e.fqn == "App.UserService.Process")
        .expect("override method");
    assert!(has_rel(
        derived_method,
        base_method.uuid,
        RelationshipType::Overrides
    ));
}

#[test]
fn test_override_csharp_constructor_excluded() {
    let entities = resolve_together(&[
        (
            "Base.cs",
            "namespace App;\npublic class Base\n{\n    public Base() {}\n}",
        ),
        (
            "Derived.cs",
            "namespace App;\npublic class Derived : Base\n{\n    public Derived() {}\n}",
        ),
    ]);
    let derived_ctor = entities
        .iter()
        .find(|e| e.fqn == "App.Derived.Derived")
        .expect("derived constructor");
    assert!(
        derived_ctor
            .relationships
            .iter()
            .all(|(_, r)| *r != RelationshipType::Overrides),
        "constructors must never get OVERRIDES edges"
    );
}

#[test]
fn test_override_csharp_calls_resolve_across_files() {
    // C22/G31 contract: a field-typed receiver call resolves to the exact
    // implementation method (not the interface declaration).
    let entities = resolve_together(&[
        (
            "IRepository.cs",
            "namespace App;\npublic interface IRepository\n{\n    Task<object> FindByIdAsync(int id);\n}",
        ),
        (
            "UserRepository.cs",
            "namespace App;\npublic class UserRepository : IRepository\n{\n    public Task<object> FindByIdAsync(int id) { return null; }\n}",
        ),
        (
            "UserService.cs",
            "namespace App;\npublic class UserService\n{\n    private readonly UserRepository _repository;\n    public Task<object> GetUserAsync(int id) { return _repository.FindByIdAsync(id); }\n}",
        ),
    ]);
    let impl_method = entities
        .iter()
        .find(|e| e.fqn == "App.UserRepository.FindByIdAsync")
        .expect("impl method");
    let caller = entities
        .iter()
        .find(|e| e.fqn == "App.UserService.GetUserAsync")
        .expect("caller");
    assert!(
        has_rel(caller, impl_method.uuid, RelationshipType::Calls),
        "field receiver call must resolve to the implementation method, got {:?}",
        caller.relationships
    );
}

// ---------------------------------------------------------------------------
// FQN prefix helpers (direct unit coverage)
// ---------------------------------------------------------------------------

#[test]
fn test_file_scoped_namespace_extraction_helpers() {
    let code = "namespace A.B;\nclass C {}";
    let tree = parse_csharp_snippet(code).unwrap();
    assert_eq!(
        extract_file_scoped_namespace(tree.root_node(), code.as_bytes()),
        Some("A.B".to_string())
    );
    let class_node = find_node(tree.root_node(), "class_declaration", "C", code.as_bytes());
    assert_eq!(
        build_csharp_fqn_prefix(class_node, code.as_bytes()),
        None,
        "file-scoped namespaces are not ancestors; the pre-pass covers them"
    );
}

#[test]
fn test_is_pattern_emits_qualified_nested_type_reference() {
    let code = "namespace App;\npublic class G { public bool Enabled(Device d) => !(d.Owner is GestureOwner.Off); }";
    let entities = extract(code);
    let method = find_entity(&entities, "Enabled", &EntityKind::CSharpMethod);
    assert!(
        method.reference_intents.iter().any(|i| matches!(i, ReferenceIntent::TypeReference { type_name, .. } if type_name == "GestureOwner.Off")),
        "should emit qualified type reference to GestureOwner.Off, got {:?}",
        method.reference_intents
    );
    assert!(
        method.reference_intents.iter().any(|i| matches!(i, ReferenceIntent::TypeReference { type_name, .. } if type_name == "GestureOwner")),
        "should also emit base identifier GestureOwner, got {:?}",
        method.reference_intents
    );
}

#[test]
fn test_switch_arm_constant_pattern_emits_qualified_reference() {
    let code = "namespace App;\npublic class G { public int? OwnerOf(Device d) => d.Owner switch { MyApp.Gestures.GestureOwner.Off => null, _ => null }; }";
    let entities = extract(code);
    let method = find_entity(&entities, "OwnerOf", &EntityKind::CSharpMethod);
    assert!(
        method.reference_intents.iter().any(|i| matches!(i, ReferenceIntent::TypeReference { type_name, .. } if type_name == "MyApp.Gestures.GestureOwner.Off")),
        "should emit qualified type reference, got {:?}",
        method.reference_intents
    );
}

#[test]
fn test_static_member_access_emits_qualified_reference() {
    let code = "namespace App;\npublic class G { public void Disable(Device d) => d.Owner = MyApp.Gestures.GestureOwner.OffValue; }";
    let entities = extract(code);
    let method = find_entity(&entities, "Disable", &EntityKind::CSharpMethod);
    assert!(
        method.reference_intents.iter().any(|i| matches!(i, ReferenceIntent::TypeReference { type_name, .. } if type_name == "MyApp.Gestures.GestureOwner.OffValue")),
        "should emit qualified type reference to OffValue, got {:?}",
        method.reference_intents
    );
}

#[test]
fn test_declaration_pattern_emits_qualified_not_bare() {
    let code = "namespace App;\npublic class G { public int? OwnerOf(Device d) => d.Owner switch { GestureOwner.Button b => b.Id, _ => null }; }";
    let entities = extract(code);
    let method = find_entity(&entities, "OwnerOf", &EntityKind::CSharpMethod);
    assert!(
        method.reference_intents.iter().any(|i| matches!(i, ReferenceIntent::TypeReference { type_name, .. } if type_name == "GestureOwner.Button")),
        "should emit qualified type reference to Button, got {:?}",
        method.reference_intents
    );
    assert!(
        !method.reference_intents.iter().any(|i| matches!(i, ReferenceIntent::TypeReference { type_name, .. } if type_name == "Button")),
        "should not emit bare Button type reference, got {:?}",
        method.reference_intents
    );
}

#[test]
fn test_enum_member_name_is_not_a_type_reference() {
    let code = "namespace App;\npublic enum LightingEffect { Solid, Breathing, Cycle, Off }";
    let entities = extract(code);
    let enum_ent = find_entity(&entities, "LightingEffect", &EntityKind::CSharpEnum);
    assert!(
        !enum_ent.reference_intents.iter().any(|i| matches!(i, ReferenceIntent::TypeReference { type_name, .. } if type_name == "Off" || type_name == "Solid")),
        "enum members should not yield type references, got {:?}",
        enum_ent.reference_intents
    );
}

#[test]
fn test_declaration_names_are_not_type_references() {
    let code = "namespace App;\npublic record GestureOwner { public record Off : GestureOwner; }";
    let entities = extract(code);
    let owner_rec = find_entity(&entities, "GestureOwner", &EntityKind::CSharpRecord);
    assert!(
        !owner_rec.reference_intents.iter().any(
            |i| matches!(i, ReferenceIntent::TypeReference { type_name, .. } if type_name == "Off")
        ),
        "nested declaration name should not be type reference, got {:?}",
        owner_rec.reference_intents
    );
}

#[test]
fn test_invocation_receiver_path_does_not_duplicate_call_intent() {
    let code =
        "namespace App;\npublic class G { public void Write() { Console.WriteLine(\"hello\"); } }";
    let entities = extract(code);
    let method = find_entity(&entities, "Write", &EntityKind::CSharpMethod);
    assert!(
        !method.reference_intents.iter().any(|i| matches!(i, ReferenceIntent::TypeReference { type_name, .. } if type_name == "Console.WriteLine")),
        "should not emit type reference for Console.WriteLine, got {:?}",
        method.reference_intents
    );
}

/// Fixture for the `GestureOwner.Off` regression: a nested record named `Off`
/// that collides with an unrelated enum member `LightingEffect.Off`.
fn gesture_owner_fixture() -> Vec<ResolutionEntity> {
    resolve_together(&[
        (
            "GestureOwner.cs",
            "namespace MyApp.Gestures;\npublic abstract record GestureOwner {\n  public sealed record Off : GestureOwner;\n  public sealed record Button(int Id) : GestureOwner;\n  public static readonly Off OffValue = new();\n}",
        ),
        (
            "LightingEffect.cs",
            "namespace MyApp.ViewModels;\npublic enum LightingEffect { Solid, Breathing, Cycle, Off }",
        ),
        (
            "GestureConfig.cs",
            "namespace MyApp.Gestures;\npublic class DeviceEntry { public GestureOwner? Owner { get; set; } }\npublic class GestureConfig {\n  public bool GesturesEnabled(DeviceEntry d) => !(d.Owner is GestureOwner.Off);\n  public int? OwnerOf(DeviceEntry d) => d.Owner switch {\n    MyApp.Gestures.GestureOwner.Off => null,\n    GestureOwner.Button b => b.Id,\n    _ => null,\n  };\n  public void Disable(DeviceEntry d) => d.Owner = MyApp.Gestures.GestureOwner.OffValue;\n}",
        ),
    ])
}

fn by_fqn<'a>(entities: &'a [ResolutionEntity], fqn: &str) -> &'a ResolutionEntity {
    entities
        .iter()
        .find(|e| e.fqn == fqn)
        .unwrap_or_else(|| panic!("entity `{fqn}` not found"))
}

#[test]
fn test_gesture_owner_off_no_spurious_edge_from_homonym_enum() {
    let entities = gesture_owner_fixture();
    let lighting_effect = by_fqn(&entities, "MyApp.ViewModels.LightingEffect");
    let off_record = by_fqn(&entities, "MyApp.Gestures.GestureOwner.Off");

    assert!(
        !has_rel(
            lighting_effect,
            off_record.uuid,
            RelationshipType::References
        ),
        "spurious edge LightingEffect -> Off must not exist"
    );
}

#[test]
fn test_gesture_owner_off_referenced_by_consumers() {
    let entities = gesture_owner_fixture();
    let off_record = by_fqn(&entities, "MyApp.Gestures.GestureOwner.Off");
    let off_value = by_fqn(&entities, "MyApp.Gestures.GestureOwner.OffValue");
    let enabled_method = by_fqn(&entities, "MyApp.Gestures.GestureConfig.GesturesEnabled");
    let owner_of_method = by_fqn(&entities, "MyApp.Gestures.GestureConfig.OwnerOf");
    let disable_method = by_fqn(&entities, "MyApp.Gestures.GestureConfig.Disable");

    assert!(
        has_rel(
            enabled_method,
            off_record.uuid,
            RelationshipType::References
        ),
        "GesturesEnabled -> Off REFERENCES missing"
    );
    assert!(
        has_rel(
            owner_of_method,
            off_record.uuid,
            RelationshipType::References
        ),
        "OwnerOf -> Off REFERENCES missing"
    );
    assert!(
        has_rel(disable_method, off_value.uuid, RelationshipType::References),
        "Disable -> OffValue REFERENCES missing"
    );
}

#[test]
fn test_gesture_owner_nested_records_extend_parent_without_back_edges() {
    let entities = gesture_owner_fixture();
    let gesture_owner = by_fqn(&entities, "MyApp.Gestures.GestureOwner");
    let off_record = by_fqn(&entities, "MyApp.Gestures.GestureOwner.Off");
    let button_record = by_fqn(&entities, "MyApp.Gestures.GestureOwner.Button");

    assert!(
        has_rel(off_record, gesture_owner.uuid, RelationshipType::Extends),
        "Off -> GestureOwner EXTENDS missing"
    );
    assert!(
        has_rel(button_record, gesture_owner.uuid, RelationshipType::Extends),
        "Button -> GestureOwner EXTENDS missing"
    );

    assert!(
        !has_rel(gesture_owner, off_record.uuid, RelationshipType::References),
        "GestureOwner -> Off REFERENCES must not exist"
    );
    assert!(
        !has_rel(
            gesture_owner,
            button_record.uuid,
            RelationshipType::References
        ),
        "GestureOwner -> Button REFERENCES must not exist"
    );
}
