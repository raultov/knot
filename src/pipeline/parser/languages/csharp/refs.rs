//! Reference-intent extraction for C#.
//!
//! AST-to-intent mapping (plan §9.3):
//!
//! | AST node | Emitted intent |
//! |---|---|
//! | `invocation_expression` with `function: (identifier)` | `Call { receiver: None }` |
//! | `invocation_expression` with `function: (member_access_expression)` | `Call { method: name, receiver: expression }` |
//! | `object_creation_expression type:` | `Call` (redirected to the constructor at resolution) |
//! | `base_list` on `interface_declaration` | `Extends` for every entry |
//! | `base_list` on `struct_declaration` / record-struct | `Implements` for every entry |
//! | `base_list` on `class_declaration` / record-class | first entry `Extends` unless `^I[A-Z]`, rest `Implements` |
//! | `attribute name:` | `Call` (mirrors the Java annotation pass) |
//! | parameter / returns / property / variable types | `TypeReference` |
//! | `using_directive` | `TypeReference` (last segment) |
//! | outermost `member_access_expression` / `qualified_name` chain of plain identifiers with a capitalized penultimate segment, not an invocation callee | `TypeReference { type_name: "<dotted path>" }` |
//!
//! Two C#-specific refinements improve resolution:
//!
//! - **Receiver-type substitution** — a member call on a field
//!   (`_repository.FindByIdAsync()`) rewrites the receiver to the field's
//!   declared type (`UserRepository`), letting the resolver find the exact
//!   implementation method.
//! - **`base` → `super`** — `base.Process()` is emitted with the receiver
//!   `super`, which the shared resolver already routes through the extends
//!   map (C# `base` ≡ Java `super`).
//!
//! All type names are stripped of type arguments before emission
//! (`IRepository<User>` → `IRepository`).

use std::collections::HashMap;

use crate::models::{CallIntent, ReferenceIntent};
use crate::pipeline::parser::utils::{is_capitalized, node_text};
use tree_sitter::Node;

// ---------------------------------------------------------------------------
// Entry points
// ---------------------------------------------------------------------------

/// Extract call + type-reference intents from a method-like entity
/// (method, constructor, local function). Used by the capture router.
pub(crate) fn extract_reference_intents_csharp(
    node: Node<'_>,
    source: &[u8],
    out: &mut Vec<ReferenceIntent>,
) {
    let field_types = collect_field_types(node, source);
    let mut calls = Vec::new();
    extract_call_intents_csharp(node, source, &field_types, &mut calls);
    for call in calls {
        out.push(ReferenceIntent::Call {
            method: call.method,
            receiver: call.receiver,
            line: call.line,
            arg_count: call.arg_count,
        });
    }
    extract_type_references_csharp(node, source, out);
}

/// Extract the EXTENDS/IMPLEMENTS heuristic from a type declaration's
/// `base_list` (plan §3.3).
pub(crate) fn extract_class_inheritance_csharp(
    class_node: Node<'_>,
    source: &[u8],
    out: &mut Vec<ReferenceIntent>,
) {
    let line = class_node.start_position().row + 1;

    let mut child = class_node.child(0);
    while let Some(c) = child {
        if c.kind() == "base_list" {
            let entries: Vec<String> = base_entries(c, source);
            let is_record_struct =
                class_node.kind() == "record_declaration" && record_is_struct(class_node);
            for (idx, name) in entries.iter().enumerate() {
                let intent = match class_node.kind() {
                    // An interface can only extend.
                    "interface_declaration" => ReferenceIntent::Extends {
                        parent: name.clone(),
                        line,
                    },
                    // A struct cannot inherit; record-struct likewise.
                    "struct_declaration" => ReferenceIntent::Implements {
                        interface: name.clone(),
                        line,
                    },
                    "record_declaration" if is_record_struct => ReferenceIntent::Implements {
                        interface: name.clone(),
                        line,
                    },
                    // Classes and record-classes: the base class is listed
                    // first and interface names use the near-universal
                    // `IPascalCase` prefix (plan §3.3).
                    "class_declaration" | "record_declaration" => {
                        if idx == 0 && !is_interface_prefix(name) {
                            ReferenceIntent::Extends {
                                parent: name.clone(),
                                line,
                            }
                        } else {
                            ReferenceIntent::Implements {
                                interface: name.clone(),
                                line,
                            }
                        }
                    }
                    // Enum base lists declare the underlying integral type;
                    // struct-like handling is the least-wrong mapping.
                    _ => ReferenceIntent::Implements {
                        interface: name.clone(),
                        line,
                    },
                };
                out.push(intent);
            }
        }
        child = c.next_sibling();
    }
}

/// Extract `Call` intents from C# attribute names (`[Obsolete]`,
/// `[Authorize(Roles = "…")]`). C# attributes live in `attribute_list`
/// children of the declaration, **not** inside a `modifiers` node as in
/// Java/Kotlin, so this cannot reuse either existing branch.
pub(crate) fn extract_attribute_references(
    node: Node<'_>,
    source: &[u8],
    out: &mut Vec<ReferenceIntent>,
) {
    if node.kind() == "attribute_list" {
        let line = node.start_position().row + 1;
        let mut child = node.child(0);
        while let Some(c) = child {
            if c.kind() == "attribute"
                && let Some(name_node) = c.child_by_field_name("name")
                && let Some(name) = base_name_from_type_node(name_node, source)
            {
                out.push(ReferenceIntent::Call {
                    method: name,
                    receiver: None,
                    line,
                    arg_count: None,
                });
            }
            child = c.next_sibling();
        }
        return;
    }

    let mut child = node.child(0);
    while let Some(c) = child {
        extract_attribute_references(c, source, out);
        child = c.next_sibling();
    }
}

/// Extract capitalized type references from a node subtree (method
/// signatures, class bodies). Skips identifiers that name a member access
/// (`user.Name`, `Task.FromResult`) so property/method names are not
/// mistaken for types.
pub(crate) fn extract_type_references_csharp(
    node: Node<'_>,
    source: &[u8],
    out: &mut Vec<ReferenceIntent>,
) {
    if (node.kind() == "member_access_expression" || node.kind() == "qualified_name")
        && is_outermost_member_access(node)
        && !is_invocation_function(node)
        && let Some(path) = dotted_path(node, source)
    {
        let segments: Vec<&str> = path.split('.').collect();
        // The penultimate segment is the type; a lowercase one means the
        // chain starts at a value (`user.Name`), not at a type.
        if segments.len() >= 2 && is_capitalized(segments[segments.len() - 2]) {
            out.push(ReferenceIntent::TypeReference {
                type_name: path,
                line: node.start_position().row + 1,
            });
        }
    }

    if node.kind() == "identifier" && !is_member_access_name(node) && !is_declaration_name(node) {
        let type_name = node_text(node, source);
        if is_capitalized(&type_name) {
            out.push(ReferenceIntent::TypeReference {
                type_name,
                line: node.start_position().row + 1,
            });
        }
        // Identifiers are leaves — no need to descend.
        return;
    }

    let mut child = node.child(0);
    while let Some(c) = child {
        extract_type_references_csharp(c, source, out);
        child = c.next_sibling();
    }
}

/// Collect ALL reference intents from the entire AST, paired with byte
/// position (the Kotlin approach): intents inside covered entity ranges are
/// discarded by the orphan pass, so calls inside method bodies fall within
/// covered ranges and are NOT orphaned.
pub(crate) fn collect_all_reference_intents_csharp(
    node: Node<'_>,
    source: &[u8],
    out: &mut Vec<(ReferenceIntent, usize)>,
) {
    let byte_pos = node.start_byte();
    let line = node.start_position().row + 1;

    match node.kind() {
        "member_access_expression" | "qualified_name"
            if is_outermost_member_access(node) && !is_invocation_function(node) =>
        {
            if let Some(path) = dotted_path(node, source) {
                let segments: Vec<&str> = path.split('.').collect();
                if segments.len() >= 2 {
                    let penultimate = segments[segments.len() - 2];
                    if is_capitalized(penultimate) {
                        out.push((
                            ReferenceIntent::TypeReference {
                                type_name: path,
                                line,
                            },
                            byte_pos,
                        ));
                    }
                }
            }
        }
        "invocation_expression" | "object_creation_expression" => {
            // Non-recursive single-node extraction: this function already
            // handles recursion via the child loop below.
            let field_types = HashMap::new();
            let calls = single_call_intents(node, source, &field_types);
            for call in calls {
                out.push((
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
        "attribute" => {
            if let Some(name_node) = node.child_by_field_name("name")
                && let Some(name) = base_name_from_type_node(name_node, source)
            {
                out.push((
                    ReferenceIntent::Call {
                        method: name,
                        receiver: None,
                        line,
                        arg_count: None,
                    },
                    byte_pos,
                ));
            }
        }
        "using_directive" => {
            if let Some(type_name) = using_directive_type_name(node, source) {
                out.push((ReferenceIntent::TypeReference { type_name, line }, byte_pos));
            }
        }
        "identifier" if !is_member_access_name(node) && !is_declaration_name(node) => {
            let type_name = node_text(node, source);
            if is_capitalized(&type_name) {
                out.push((ReferenceIntent::TypeReference { type_name, line }, byte_pos));
            }
        }
        _ => {}
    }

    let mut child = node.child(0);
    while let Some(c) = child {
        collect_all_reference_intents_csharp(c, source, out);
        child = c.next_sibling();
    }
}

// ---------------------------------------------------------------------------
// Call intents
// ---------------------------------------------------------------------------

/// Recursively extract call intents, substituting field receivers with their
/// declared types.
fn extract_call_intents_csharp(
    node: Node<'_>,
    source: &[u8],
    field_types: &HashMap<String, String>,
    out: &mut Vec<CallIntent>,
) {
    out.extend(single_call_intents(node, source, field_types));

    let mut child = node.child(0);
    while let Some(c) = child {
        extract_call_intents_csharp(c, source, field_types, out);
        child = c.next_sibling();
    }
}

/// Extract call intents from a SINGLE node without recursive descent.
fn single_call_intents(
    node: Node<'_>,
    source: &[u8],
    field_types: &HashMap<String, String>,
) -> Vec<CallIntent> {
    let mut intents = Vec::new();
    let line = node.start_position().row + 1;

    if node.kind() == "invocation_expression" {
        if let Some(function) = node.child_by_field_name("function") {
            match function.kind() {
                "identifier" => intents.push(CallIntent {
                    method: node_text(function, source),
                    receiver: None,
                    line,
                    arg_count: argument_count(node),
                }),
                "generic_name" => {
                    if let Some(id) = first_identifier(function) {
                        intents.push(CallIntent {
                            method: node_text(id, source),
                            receiver: None,
                            line,
                            arg_count: argument_count(node),
                        });
                    }
                }
                "member_access_expression" => {
                    if let Some(name_node) = function.child_by_field_name("name")
                        && let Some(method) = base_name_from_type_node(name_node, source)
                    {
                        let receiver = function
                            .child_by_field_name("expression")
                            .map(|expr| receiver_text(expr, source, field_types));
                        intents.push(CallIntent {
                            method,
                            receiver,
                            line,
                            arg_count: argument_count(node),
                        });
                    }
                }
                _ => {}
            }
        }
    } else if node.kind() == "object_creation_expression"
        && let Some(type_node) = node.child_by_field_name("type")
        && let Some(name) = base_name_from_type_node(type_node, source)
    {
        intents.push(CallIntent {
            method: name,
            receiver: None,
            line,
            arg_count: None,
        });
    }

    intents
}

/// Number of named `argument` nodes in the invocation's argument list.
fn argument_count(invocation: Node<'_>) -> Option<usize> {
    let args = invocation.child_by_field_name("arguments")?;
    let mut count = 0;
    let mut child = args.child(0);
    while let Some(c) = child {
        if c.kind() == "argument" {
            count += 1;
        }
        child = c.next_sibling();
    }
    Some(count)
}

/// Resolve the receiver text for a member access, applying the two C#
/// refinements: `base` → `super`, and field receivers → declared type.
fn receiver_text(expr: Node<'_>, source: &[u8], field_types: &HashMap<String, String>) -> String {
    let raw = node_text(expr, source);

    // `base.X()` is C#'s `super.X()`; the shared resolver routes `super`
    // receivers through the extends map.
    if expr.kind() == "base_expression" || raw == "base" {
        return "super".to_string();
    }

    // Field receiver: `_repository.FindByIdAsync()` — substitute the
    // receiver with the field's declared type when known.
    if expr.kind() == "identifier"
        && let Some(declared) = field_types.get(raw.as_str())
    {
        return declared.clone();
    }

    raw
}

/// Build a map of field name → base type name for the type that encloses
/// `node` (method/constructor/local function). Only `field_declaration`
/// members are collected; the map drives receiver-type substitution.
fn collect_field_types(node: Node<'_>, source: &[u8]) -> HashMap<String, String> {
    let mut map = HashMap::new();

    let enclosing = find_parent_by_kind_any(
        node,
        &[
            "class_declaration",
            "interface_declaration",
            "struct_declaration",
            "record_declaration",
        ],
    );

    let Some(type_node) = enclosing else {
        return map;
    };

    // Members live inside the type's `declaration_list` body child, not
    // directly under the declaration node.
    let body = first_child_of_kind(type_node, "declaration_list").unwrap_or(type_node);
    let mut child = body.child(0);
    while let Some(c) = child {
        if c.kind() == "field_declaration"
            && let Some(var_decl) = first_child_of_kind(c, "variable_declaration")
            && let Some(type_node) = var_decl.child_by_field_name("type")
        {
            let base_type = base_type_text(type_node, source);
            let mut declarator = var_decl.child(0);
            while let Some(d) = declarator {
                if d.kind() == "variable_declarator"
                    && let Some(name_node) = d.child_by_field_name("name")
                {
                    map.insert(node_text(name_node, source), base_type.clone());
                }
                declarator = d.next_sibling();
            }
        }
        child = c.next_sibling();
    }

    map
}

/// Strip type arguments / array suffixes / nullable markers from a declared
/// type text: `List<User>` → `List`, `string[]` → `string`, `Notifier?` →
/// `Notifier`.
fn base_type_text(type_node: Node<'_>, source: &[u8]) -> String {
    let raw = node_text(type_node, source);
    raw.split(['<', '[', '?'])
        .next()
        .unwrap_or(&raw)
        .trim()
        .to_string()
}

// ---------------------------------------------------------------------------
// base_list heuristic (plan §3.3)
// ---------------------------------------------------------------------------

/// `true` when the name follows the `IPascalCase` interface prefix
/// (`^I[A-Z]`), the near-universal .NET convention.
fn is_interface_prefix(name: &str) -> bool {
    let bytes = name.as_bytes();
    bytes.len() >= 2 && bytes[0] == b'I' && bytes[1].is_ascii_uppercase()
}

/// `true` for `record struct …` declarations (the anonymous `struct` token
/// among the `record_declaration` children, plan §2.3, Gap 3).
pub(crate) fn record_is_struct(record_node: Node<'_>) -> bool {
    let mut child = record_node.child(0);
    while let Some(c) = child {
        if c.kind() == "struct" && !c.is_named() {
            return true;
        }
        child = c.next_sibling();
    }
    false
}

/// Extract base names from a `base_list` node, stripping type arguments.
fn base_entries(base_list: Node<'_>, source: &[u8]) -> Vec<String> {
    let mut entries = Vec::new();
    let mut child = base_list.child(0);
    while let Some(c) = child {
        if c.is_named()
            && let Some(name) = base_name_from_type_node(c, source)
        {
            entries.push(name);
        }
        child = c.next_sibling();
    }
    entries
}

/// Reduce a type-shaped node to its base name, dropping generics
/// (`IRepository<User>` → `IRepository`) and qualifiers
/// (`System.IEquatable` → `IEquatable`).
fn base_name_from_type_node(node: Node<'_>, source: &[u8]) -> Option<String> {
    match node.kind() {
        "identifier" => Some(node_text(node, source)),
        "generic_name" => first_identifier(node).map(|id| node_text(id, source)),
        "qualified_name" | "alias_qualified_name" => node
            .child_by_field_name("name")
            .and_then(|n| base_name_from_type_node(n, source)),
        "nullable_type" => node
            .child_by_field_name("type")
            .and_then(|n| base_name_from_type_node(n, source)),
        // `record Foo(...)` primary-constructor base entry
        // (`record Derived : Base(args)` — aliased to base_list).
        "primary_constructor_base_type" => node
            .child_by_field_name("type")
            .and_then(|n| base_name_from_type_node(n, source)),
        _ => None,
    }
}

/// `true` when `node` is the `name` field of a `member_access_expression`
/// or `qualified_name` (property/method/qualified names must not be mistaken for type references).
fn is_member_access_name(node: Node<'_>) -> bool {
    node.parent().is_some_and(|p| {
        (p.kind() == "member_access_expression" || p.kind() == "qualified_name")
            && p.child_by_field_name("name")
                .is_some_and(|n| n.id() == node.id())
    })
}

/// Reduce a `member_access_expression` / `qualified_name` chain to its dotted
/// text when **every** segment is a plain identifier.
fn dotted_path(node: Node<'_>, source: &[u8]) -> Option<String> {
    let mut current = node;
    let mut segments = Vec::new();
    loop {
        // `qualified_name` keeps its left-hand side under `qualifier`,
        // `member_access_expression` under `expression`.
        let parent_field = match current.kind() {
            "member_access_expression" => "expression",
            "qualified_name" => "qualifier",
            "identifier" => {
                segments.push(node_text(current, source));
                break;
            }
            _ => return None,
        };

        let name_node = current.child_by_field_name("name")?;
        if name_node.kind() != "identifier" {
            return None;
        }
        segments.push(node_text(name_node, source));
        current = current.child_by_field_name(parent_field)?;
    }
    segments.reverse();
    Some(segments.join("."))
}

fn is_outermost_member_access(node: Node<'_>) -> bool {
    if let Some(parent) = node.parent() {
        match parent.kind() {
            "member_access_expression" => {
                parent.child_by_field_name("expression").map(|e| e.id()) != Some(node.id())
            }
            "qualified_name" => {
                parent.child_by_field_name("qualifier").map(|q| q.id()) != Some(node.id())
            }
            _ => true,
        }
    } else {
        true
    }
}

fn is_invocation_function(node: Node<'_>) -> bool {
    if let Some(parent) = node.parent() {
        parent.kind() == "invocation_expression"
            && parent.child_by_field_name("function").map(|f| f.id()) == Some(node.id())
    } else {
        false
    }
}

/// `true` when `node` is the `name:` field of a declaration — the identifier
/// *introduces* a symbol rather than referring to one.
fn is_declaration_name(node: Node<'_>) -> bool {
    if let Some(parent) = node.parent() {
        let parent_kind = parent.kind();
        let is_decl_kind = matches!(
            parent_kind,
            "enum_member_declaration"
                | "class_declaration"
                | "interface_declaration"
                | "struct_declaration"
                | "record_declaration"
                | "enum_declaration"
                | "method_declaration"
                | "constructor_declaration"
                | "destructor_declaration"
                | "property_declaration"
                | "event_declaration"
                | "delegate_declaration"
                | "namespace_declaration"
                | "file_scoped_namespace_declaration"
                | "local_function_statement"
                | "variable_declarator"
                | "parameter"
                | "type_parameter"
                | "catch_declaration"
                | "declaration_expression"
                | "declaration_pattern"
        );
        if is_decl_kind
            && let Some(name_node) = parent.child_by_field_name("name")
            && name_node.id() == node.id()
        {
            return true;
        }
    }
    false
}

/// Resolve a plain `using` directive to its last identifier segment
/// (`using System.Text;` → `Text`). The alias form (`using Foo = Bar.Baz;`)
/// contributes the target's last segment. Returns `None` when the directive
/// has no resolvable identifier or the segment is not capitalized.
fn using_directive_type_name(directive: Node<'_>, source: &[u8]) -> Option<String> {
    let mut last: Option<String> = None;
    let mut stack = vec![directive];
    while let Some(n) = stack.pop() {
        match n.kind() {
            "identifier" => last = Some(node_text(n, source)),
            "generic_name" | "qualified_name" | "alias_qualified_name" => {
                if let Some(name_node) = n.child_by_field_name("name") {
                    stack.push(name_node);
                } else {
                    let mut child = n.child(0);
                    while let Some(c) = child {
                        if c.is_named() {
                            stack.push(c);
                        }
                        child = c.next_sibling();
                    }
                }
            }
            _ => {
                let mut child = n.child(0);
                while let Some(c) = child {
                    if c.is_named() {
                        stack.push(c);
                    }
                    child = c.next_sibling();
                }
            }
        }
    }

    let name = last?;
    if is_capitalized(&name) {
        Some(name)
    } else {
        None
    }
}

// ---------------------------------------------------------------------------
// Small AST helpers
// ---------------------------------------------------------------------------

fn first_identifier(node: Node<'_>) -> Option<Node<'_>> {
    if node.kind() == "identifier" {
        return Some(node);
    }
    let mut child = node.child(0);
    while let Some(c) = child {
        if let Some(found) = first_identifier(c) {
            return Some(found);
        }
        child = c.next_sibling();
    }
    None
}

fn first_child_of_kind<'a>(node: Node<'a>, kind: &str) -> Option<Node<'a>> {
    let mut child = node.child(0);
    while let Some(c) = child {
        if c.kind() == kind {
            return Some(c);
        }
        child = c.next_sibling();
    }
    None
}

fn find_parent_by_kind_any<'a>(node: Node<'a>, kinds: &[&str]) -> Option<Node<'a>> {
    let mut current = node.parent();
    while let Some(parent) = current {
        if kinds.contains(&parent.kind()) {
            return Some(parent);
        }
        current = parent.parent();
    }
    None
}
