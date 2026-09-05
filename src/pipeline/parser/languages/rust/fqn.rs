//! FQN qualification for Rust entities and inline `mod` context extraction.
//!
//! Production passes here rewrite every Rust entity's FQN so it is anchored at
//! its owning crate, its module path (derived from the file's location within
//! `src/`), and any **inline** `mod foo { ... }` blocks that contain it. This
//! prevents same-named entities in different crates or files from colliding on
//! the bare name, and lets consumers tell `#[cfg(test)]`-gated code apart from
//! production code.

use crate::models::{EntityKind, ParsedEntity};
use crate::pipeline::parser::utils::node_text;
use crate::pipeline::rust_crate_discovery::CrateDiscovery;
use tree_sitter::Node;

/// An inline Rust `mod foo { ... }` block (NOT external `mod foo;`).
///
/// Tracked so the FQN of every entity nested inside the block is anchored at
/// the module name (e.g. `crate::config::tests::test_x` rather than the
/// colliding `crate::config::test_x`) and so consumers of the index can tell
/// `#[cfg(test)]`-gated code apart from production code.
#[derive(Debug, Clone)]
struct RustModuleContext {
    name: String,
    start_line: usize,
    end_line: usize,
    /// `true` when at least one `#[cfg(test)]` (or `cfg_attr(test, ...)`)
    /// attribute is attached to this mod_item.
    is_cfg_test: bool,
}

/// Rewrite the FQN of every Rust entity in `entities` to be prefixed by its
/// owning crate name, module path, and any **inline** `mod foo { ... }`
/// blocks that contain it.
///
/// Before this pass each Rust entity carries a bare-name FQN (e.g. `Config`
/// for a struct or `Foo::new` for a method). After this pass the FQN is
/// anchored at the crate that owns the file (e.g.
/// `knot::config::Config`, `knot::config::Config::new`), so that two crates
/// defining the same type do not collide on the bare name.
///
/// Inline modules are walked here too: a function `test_x` declared inside
/// `#[cfg(test)] mod tests { ... }` in `src/config.rs` ends up with FQN
/// `knot::config::tests::test_x` AND `is_test_context = true`. Without this
/// the test function would either collide with same-named tests in other
/// files or fail to surface at all in `find_callers` queries.
///
/// `file_path` is the canonical **repo-relative** path produced by the
/// parser (§3.1 of `docs/specs/relative_file_paths.md`). Internally the
/// absolute path is reconstructed against `repo_path` because crate
/// discovery (`CrateDiscovery`) and `compute_rust_file_kind` index the
/// repository by absolute paths — only the **persisted** entity path is
/// relative. Existing callers passing absolute paths continue to work.
///
/// A `None` `repo_path`, a missing `Cargo.toml`, or a file outside any
/// known crate `src/` directory is treated as a no-op for that file.
pub(crate) fn qualify_rust_fqns(
    entities: &mut [ParsedEntity],
    file_path: &str,
    repo_path: Option<&str>,
    source: Option<&str>,
) {
    let Some(repo_path) = repo_path else {
        return;
    };
    if entities.is_empty() {
        return;
    }

    let repo_root = std::path::Path::new(repo_path);
    let relative_path = std::path::Path::new(file_path);
    // §3.1 ordering constraint: crate discovery must use absolute paths.
    let absolute_path: std::path::PathBuf = if relative_path.is_absolute() {
        relative_path.to_path_buf()
    } else {
        repo_root.join(relative_path)
    };

    let discovery = CrateDiscovery::discover(repo_root);
    let crate_root = discovery.crate_for_file(&absolute_path);

    let file_kind = crate::pipeline::parser::context::compute_rust_file_kind(
        &absolute_path.to_string_lossy(),
        crate_root.map(|cr| cr.root_dir.as_path()),
        repo_root,
    );

    let crate_name = crate_root
        .map(|cr| cr.crate_name.as_str())
        .unwrap_or("__loose");

    // Inline `mod foo { ... }` blocks: when available, the source is reparsed
    // to gather their ranges. When `source` is `None` (e.g. legacy unit tests),
    // the inline pass becomes a no-op.
    let module_contexts: Vec<RustModuleContext> =
        source.map(extract_rust_module_contexts).unwrap_or_default();

    for entity in entities.iter_mut() {
        if entity.language != "rust" {
            continue;
        }
        if !is_qualifiable_rust_kind(&entity.kind) {
            continue;
        }
        let (inline_path, is_test) =
            inline_module_path_for_entity(&module_contexts, entity.start_line);

        let new_fqn =
            crate::pipeline::parser::context::compute_rust_qualified_fqn_with_inline_modules(
                &entity.name,
                &entity.kind,
                &file_kind,
                crate_name,
                &inline_path,
                entity.enclosing_class.as_deref(),
            );
        entity.fqn = new_fqn;
        entity.is_test_context = is_test;

        // For methods inside impl blocks, persist the FQN of the enclosing
        // class so the CONTAINS auto-link in Neo4j can match by FQN instead
        // of by bare name (which collides when fixtures define the same struct).
        if entity.kind == EntityKind::RustMethod
            && let Some(enclosing_class) = &entity.enclosing_class
        {
            let class_fqn =
                crate::pipeline::parser::context::compute_rust_qualified_fqn_with_inline_modules(
                    enclosing_class,
                    &EntityKind::RustStruct,
                    &file_kind,
                    crate_name,
                    &inline_path,
                    None,
                );
            entity.enclosing_class_fqn = Some(class_fqn);
        }
    }
}

/// Return the inline module suffix (e.g. `"tests"` or `"outer::inner"`) that
/// contains an entity at `line`, plus whether any enclosing module is
/// `#[cfg(test)]`-gated.
///
/// A module *contains* an entity when the entity sits **strictly inside**
/// the module's line range — `module.start_line < line <= module.end_line`.
/// The strict lower bound prevents a `mod tests` entity from listing itself
/// as its own enclosing module.
fn inline_module_path_for_entity(contexts: &[RustModuleContext], line: usize) -> (String, bool) {
    let mut containing: Vec<&RustModuleContext> = contexts
        .iter()
        .filter(|m| line > m.start_line && line <= m.end_line)
        .collect();

    // Outermost first → predictable `outer::inner` ordering regardless of
    // walk order in `extract_rust_module_contexts`.
    containing.sort_by_key(|m| m.start_line);

    let path = containing
        .iter()
        .map(|m| m.name.as_str())
        .collect::<Vec<_>>()
        .join("::");
    let is_test = containing.iter().any(|m| m.is_cfg_test);
    (path, is_test)
}

/// Reparse `source` and walk the tree to enumerate every inline
/// `mod foo { ... }` block, including its line range and whether it is
/// `#[cfg(test)]`-gated.
///
/// External declarations (`mod foo;` with no body) are intentionally ignored
/// because they don't introduce a textual scope in the current file.
fn extract_rust_module_contexts(source: &str) -> Vec<RustModuleContext> {
    let mut parser = tree_sitter::Parser::new();
    if parser
        .set_language(&tree_sitter_rust::LANGUAGE.into())
        .is_err()
    {
        return Vec::new();
    }
    let Some(tree) = parser.parse(source, None) else {
        return Vec::new();
    };

    let source_bytes = source.as_bytes();
    let mut contexts = Vec::new();
    collect_inline_mod_items(&tree.root_node(), source_bytes, &mut contexts);
    contexts
}

/// Walk the AST recursively, pushing every inline `mod_item` (one whose body
/// is a `declaration_list`) into `out`.
fn collect_inline_mod_items(node: &Node<'_>, source: &[u8], out: &mut Vec<RustModuleContext>) {
    if node.kind() == "mod_item"
        && let Some(ctx) = build_module_context(node, source)
    {
        out.push(ctx);
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_inline_mod_items(&child, source, out);
    }
}

/// Build a [`RustModuleContext`] for a `mod_item` node, returning `None` for
/// external declarations (`mod foo;` without a body).
fn build_module_context(node: &Node<'_>, source: &[u8]) -> Option<RustModuleContext> {
    // Find both the name and the body within this mod_item's direct children.
    let mut name: Option<String> = None;
    let mut has_body = false;
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "identifier" if name.is_none() => {
                name = Some(node_text(child, source).to_string());
            }
            "declaration_list" => {
                has_body = true;
            }
            _ => {}
        }
    }

    let name = name?;
    if !has_body {
        return None;
    }

    let is_cfg_test = node_attribute_marks_cfg_test(node, source);
    Some(RustModuleContext {
        name,
        start_line: node.start_position().row + 1,
        end_line: node.end_position().row + 1,
        is_cfg_test,
    })
}

/// Return `true` when an `attribute_item` sibling preceding `mod_node`
/// contains `cfg(test)` or `cfg_attr(test, ...)`.
///
/// Tree-sitter attaches attribute_items as *previous siblings* of the
/// `mod_item` they decorate, so the search walks back from `mod_node`.
fn node_attribute_marks_cfg_test(mod_node: &Node<'_>, source: &[u8]) -> bool {
    let mut sibling = mod_node.prev_sibling();
    while let Some(s) = sibling {
        match s.kind() {
            "attribute_item" | "inner_attribute_item" => {
                let text = node_text(s, source);
                if attribute_text_marks_cfg_test(&text) {
                    return true;
                }
                sibling = s.prev_sibling();
            }
            // Stop the moment we hit a non-attribute sibling: attributes
            // belonging to other items are not transitively applied.
            "line_comment" | "block_comment" => {
                sibling = s.prev_sibling();
            }
            _ => break,
        }
    }
    false
}

/// Detect whether an attribute string carries a `cfg(test)` guard.
///
/// Accepts both the canonical `#[cfg(test)]` and the indirect
/// `#[cfg_attr(test, ...)]` form because both gate compilation on the
/// `test` profile.
fn attribute_text_marks_cfg_test(text: &str) -> bool {
    let normalised: String = text.chars().filter(|c| !c.is_whitespace()).collect();
    normalised.contains("cfg(test)") || normalised.contains("cfg_attr(test,")
}

fn is_qualifiable_rust_kind(kind: &EntityKind) -> bool {
    matches!(
        kind,
        EntityKind::RustStruct
            | EntityKind::RustEnum
            | EntityKind::RustUnion
            | EntityKind::RustTrait
            | EntityKind::RustImpl
            | EntityKind::RustFunction
            | EntityKind::RustMethod
            | EntityKind::RustMacroDef
            | EntityKind::RustTypeAlias
            | EntityKind::RustConstant
            | EntityKind::RustStatic
            | EntityKind::RustModule
    )
}
