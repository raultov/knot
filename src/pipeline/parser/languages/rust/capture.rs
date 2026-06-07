//! Tree-sitter capture handler for Rust entities.
//!
//! `handle_rust_capture` is invoked by the generic extractor for every Rust
//! capture name produced by `queries/rust.scm`. It maps the capture name to
//! the corresponding [`EntityKind`] and returns the captured identifier text
//! along with the entity's start line.

use crate::models::EntityKind;
use tree_sitter::Node;

/// Handle Rust-specific entity captures from tree-sitter queries.
///
/// Returns `(name, kind, start_line)` for the captured entity, or `None`
/// when the capture name is not an entity declaration we track (for example
/// `rust.generics`, `rust.signature`, or any unknown capture).
pub(crate) fn handle_rust_capture(
    cap_name: &str,
    text: &str,
    node: Node<'_>,
) -> Option<(String, EntityKind, usize)> {
    let start_line = node.start_position().row + 1;

    match cap_name {
        "rust.struct.name" => Some((text.to_string(), EntityKind::RustStruct, start_line)),
        "rust.enum.name" => Some((text.to_string(), EntityKind::RustEnum, start_line)),
        "rust.union.name" => Some((text.to_string(), EntityKind::RustUnion, start_line)),
        "rust.trait.name" => Some((text.to_string(), EntityKind::RustTrait, start_line)),
        "rust.impl.target" => Some((text.to_string(), EntityKind::RustImpl, start_line)),
        "rust.impl.trait" => Some((text.to_string(), EntityKind::RustImpl, start_line)),
        "rust.function.name" => Some((text.to_string(), EntityKind::RustFunction, start_line)),
        "rust.macro_def.name" => Some((text.to_string(), EntityKind::RustMacroDef, start_line)),
        "rust.macro_inv.name" => Some((text.to_string(), EntityKind::RustMacroInvoke, start_line)),
        "rust.type_alias.name" => Some((text.to_string(), EntityKind::RustTypeAlias, start_line)),
        "rust.constant.name" => Some((text.to_string(), EntityKind::RustConstant, start_line)),
        "rust.static.name" => Some((text.to_string(), EntityKind::RustStatic, start_line)),
        "rust.module.name" => Some((text.to_string(), EntityKind::RustModule, start_line)),
        "rust.method.name" => Some((text.to_string(), EntityKind::RustMethod, start_line)),
        "rust.call.name"
        | "rust.generics"
        | "rust.signature"
        | "rust.return_type"
        | "rust.lifetime"
        | "rust.attribute.name" => None,
        _ => None,
    }
}
