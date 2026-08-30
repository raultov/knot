//! Capture routing for C# queries (`queries/csharp.scm`).
//!
//! Handles the grammar gaps documented in plan §2.3:
//!
//! - **`csharp.field.name`** — `field_declaration` has no `name` field; the
//!   capture targets `variable_declarator > name:(identifier)` and the entity
//!   node is resolved by walking up to the declaration. A `const` modifier on
//!   the declaration promotes the entity to [`EntityKind::CSharpConstant`].
//! - **`csharp.record.name`** — `record_declaration` covers both
//!   `record class` and `record struct`; both map to [`EntityKind::CSharpRecord`]
//!   (the struct flavour only matters for the `base_list` heuristic, see
//!   `refs::record_is_struct`).
//! - **`csharp.indexer` / `csharp.operator`** — these declarations have no
//!   `name` field, so names are synthesised (`this[]`, `operator +`).

use crate::models::EntityKind;
use crate::pipeline::parser::utils::{find_parent_by_kind, node_text};
use tree_sitter::Node;

/// A resolved C# capture: entity name, kind, 1-based start line, and the AST
/// node representing the full declaration (used for comments, decorators,
/// covered ranges, and FQN ancestor walks).
pub(crate) struct CsharpCapture<'a> {
    pub name: String,
    pub kind: EntityKind,
    pub start_line: usize,
    pub entity_node: Node<'a>,
}

/// Simple captures where the entity is `(text, kind, named declaration
/// ancestor)`: capture name → (kind, declaration node kind).
const SIMPLE_CAPTURES: &[(&str, EntityKind, &str)] = &[
    (
        "csharp.class.name",
        EntityKind::CSharpClass,
        "class_declaration",
    ),
    (
        "csharp.interface.name",
        EntityKind::CSharpInterface,
        "interface_declaration",
    ),
    (
        "csharp.struct.name",
        EntityKind::CSharpStruct,
        "struct_declaration",
    ),
    // `record class` and `record struct` share one node type (plan §2.3,
    // Gap 3); both surface as CSharpRecord.
    (
        "csharp.record.name",
        EntityKind::CSharpRecord,
        "record_declaration",
    ),
    (
        "csharp.enum.name",
        EntityKind::CSharpEnum,
        "enum_declaration",
    ),
    (
        "csharp.method.name",
        EntityKind::CSharpMethod,
        "method_declaration",
    ),
    (
        "csharp.constructor.name",
        EntityKind::CSharpConstructor,
        "constructor_declaration",
    ),
    (
        "csharp.property.name",
        EntityKind::CSharpProperty,
        "property_declaration",
    ),
    (
        "csharp.delegate.name",
        EntityKind::CSharpDelegate,
        "delegate_declaration",
    ),
    (
        "csharp.local_function.name",
        EntityKind::CSharpLocalFunction,
        "local_function_statement",
    ),
];

/// Route a `csharp.*` query capture to an entity candidate.
///
/// Returns `None` for captures that do not introduce entities (metadata
/// captures are handled by the caller) or when the expected declaration
/// ancestor cannot be found.
pub(crate) fn handle_csharp_capture<'a>(
    cap_name: &str,
    node: Node<'a>,
    source: &[u8],
) -> Option<CsharpCapture<'a>> {
    // Table-driven simple captures first.
    if let Some((_, kind, decl_kind)) = SIMPLE_CAPTURES.iter().find(|(cap, _, _)| *cap == cap_name)
    {
        let entity_node = find_parent_by_kind(node, decl_kind)?;
        return Some(capture_from(
            node_text(node, source),
            kind.clone(),
            entity_node,
        ));
    }

    let text = node_text(node, source);

    let (name, kind, entity_node) = match cap_name {
        "csharp.namespace.name" => (
            text,
            EntityKind::CSharpNamespace,
            find_parent_by_kind(node, "namespace_declaration")
                .or_else(|| find_parent_by_kind(node, "file_scoped_namespace_declaration"))?,
        ),
        // Grammar gap (plan §2.3, Gap 2): the name lives two levels down in
        // variable_declaration > variable_declarator; the entity node is the
        // ancestor field_declaration.
        "csharp.field.name" => {
            let decl = find_parent_by_kind(node, "field_declaration")?;
            let kind = if has_modifier(decl, source, "const") {
                EntityKind::CSharpConstant
            } else {
                EntityKind::CSharpField
            };
            (text, kind, decl)
        }
        // Both `event X { add; remove; }` (event_declaration) and the
        // event-field form `event X y;` (event_field_declaration) surface as
        // CSharpEvent.
        "csharp.event.name" => {
            let decl = find_parent_by_kind(node, "event_declaration")
                .or_else(|| find_parent_by_kind(node, "event_field_declaration"))?;
            (text, EntityKind::CSharpEvent, decl)
        }
        // Grammar gap (plan §2.3): indexer declarations have no `name` field.
        // The query captures the whole declaration; the name is synthesised.
        "csharp.indexer" => {
            let decl = declaration_or_self(node, "indexer_declaration");
            ("this[]".to_string(), EntityKind::CSharpIndexer, decl)
        }
        // Grammar gap (plan §2.3): operator declarations have no `name`
        // field. The name is synthesised from the `operator` token.
        "csharp.operator" => {
            let decl = declaration_or_self(node, "operator_declaration");
            let op_token = decl
                .child_by_field_name("operator")
                .map(|t| node_text(t, source))
                .unwrap_or_default();
            let name = if op_token.is_empty() {
                "operator".to_string()
            } else {
                format!("operator {op_token}")
            };
            (name, EntityKind::CSharpOperator, decl)
        }
        _ => return None,
    };

    Some(capture_from(name, kind, entity_node))
}

/// Wrap a resolved capture: the 1-based start line comes from the full
/// declaration node (attributes/modifiers included).
fn capture_from<'a>(name: String, kind: EntityKind, entity_node: Node<'a>) -> CsharpCapture<'a> {
    CsharpCapture {
        name,
        kind,
        start_line: entity_node.start_position().row + 1,
        entity_node,
    }
}

/// The nearest ancestor declaration of `kind`, or `node` itself when the
/// query captured the whole declaration.
fn declaration_or_self<'a>(node: Node<'a>, kind: &str) -> Node<'a> {
    if node.kind() == kind {
        node
    } else {
        find_parent_by_kind(node, kind).unwrap_or(node)
    }
}

/// `true` when the declaration carries the given keyword among its
/// `modifier` children (e.g. `const`, `static`, `virtual`).
pub(crate) fn has_modifier(decl: Node<'_>, source: &[u8], keyword: &str) -> bool {
    let mut child = decl.child(0);
    while let Some(c) = child {
        if c.kind() == "modifier" && node_text(c, source) == keyword {
            return true;
        }
        child = c.next_sibling();
    }
    false
}
