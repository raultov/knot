//! Compile-time string constants of one Java/Kotlin file, so routes built
//! from them (`@Path(ITEMS)`, `@GetMapping(Api.BASE + "/{id}")`) resolve.

use tree_sitter::Node;

use super::annotation::{StringConstants, eval_concatenation};
use crate::pipeline::parser::utils::{find_parent_by_kind_any, node_text};

/// A declared constant before evaluation: its owner type (if any), its
/// name and its initializer text.
struct RawConstant {
    owner: Option<String>,
    name: String,
    initializer: String,
}

/// Rounds of evaluation, so constants built from other constants of the
/// same file resolve whatever their declaration order.
const MAX_ROUNDS: usize = 4;

/// Every string constant of the file, keyed by `NAME` and `Owner.NAME`:
/// Java `static final` fields and interface constants, Kotlin `const val`.
/// Constants whose value depends on something outside the file are left
/// out.
pub(crate) fn extract_string_constants(
    root: Node<'_>,
    source: &[u8],
    lang_name: &str,
) -> StringConstants {
    let mut raw = Vec::new();
    collect(root, source, lang_name, &mut raw);
    let mut constants = StringConstants::new();
    for _ in 0..MAX_ROUNDS {
        let before = constants.len();
        for constant in &raw {
            let value = eval_concatenation(&constant.initializer, &constants);
            if value.contains('<') {
                continue;
            }
            if let Some(owner) = &constant.owner {
                constants.insert(format!("{owner}.{}", constant.name), value.clone());
            }
            constants.insert(constant.name.clone(), value);
        }
        if constants.len() == before {
            break;
        }
    }
    constants
}

fn collect(node: Node<'_>, source: &[u8], lang_name: &str, out: &mut Vec<RawConstant>) {
    match (lang_name, node.kind()) {
        ("java", "field_declaration" | "constant_declaration") => {
            java_constants(node, source, out);
        }
        ("kotlin", "property_declaration") => kotlin_constant(node, source, out),
        _ => {}
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        collect(child, source, lang_name, out);
    }
}

/// `static final String X = ...;` and interface constants (implicitly
/// static final).
fn java_constants(node: Node<'_>, source: &[u8], out: &mut Vec<RawConstant>) {
    let is_type_string = node
        .child_by_field_name("type")
        .is_some_and(|t| matches!(node_text(t, source).as_str(), "String" | "java.lang.String"));
    let modifiers = first_child_text(node, "modifiers", source).unwrap_or_default();
    let in_interface = node.kind() == "constant_declaration";
    let is_constant =
        in_interface || (has_word(&modifiers, "static") && has_word(&modifiers, "final"));
    if !is_type_string || !is_constant {
        return;
    }
    let owner = java_owner(node, source);
    let mut cursor = node.walk();
    for declarator in node.children_by_field_name("declarator", &mut cursor) {
        if let (Some(name), Some(value)) = (
            declarator.child_by_field_name("name"),
            declarator.child_by_field_name("value"),
        ) {
            out.push(RawConstant {
                owner: owner.clone(),
                name: node_text(name, source),
                initializer: node_text(value, source),
            });
        }
    }
}

fn java_owner(node: Node<'_>, source: &[u8]) -> Option<String> {
    owner_name(
        node,
        source,
        &[
            "class_declaration",
            "interface_declaration",
            "enum_declaration",
            "record_declaration",
        ],
    )
}

/// Name of the nearest enclosing declaration whose kind is in `kinds`.
fn owner_name(node: Node<'_>, source: &[u8], kinds: &[&str]) -> Option<String> {
    find_parent_by_kind_any(node, kinds)
        .and_then(|owner| owner.child_by_field_name("name"))
        .map(|name| node_text(name, source))
}

/// `const val X = ...` at top level, in an object or in a companion object
/// (owned by its class).
fn kotlin_constant(node: Node<'_>, source: &[u8], out: &mut Vec<RawConstant>) {
    let modifiers = first_child_text(node, "modifiers", source).unwrap_or_default();
    if !has_word(&modifiers, "const") {
        return;
    }
    let mut cursor = node.walk();
    let children: Vec<Node<'_>> = node.named_children(&mut cursor).collect();
    let Some(name) = children
        .iter()
        .find(|c| c.kind() == "variable_declaration")
        .and_then(|v| v.named_child(0))
    else {
        return;
    };
    let Some(value) = children
        .iter()
        .rev()
        .find(|c| !matches!(c.kind(), "modifiers" | "variable_declaration"))
    else {
        return;
    };
    out.push(RawConstant {
        owner: kotlin_owner(node, source),
        name: node_text(name, source),
        initializer: node_text(*value, source),
    });
}

fn kotlin_owner(node: Node<'_>, source: &[u8]) -> Option<String> {
    owner_name(node, source, &["class_declaration", "object_declaration"])
}

fn first_child_text(node: Node<'_>, kind: &str, source: &[u8]) -> Option<String> {
    let mut cursor = node.walk();
    node.children(&mut cursor)
        .find(|c| c.kind() == kind)
        .map(|c| node_text(c, source))
}

fn has_word(text: &str, word: &str) -> bool {
    text.split(|c: char| !c.is_alphanumeric())
        .any(|token| token == word)
}
