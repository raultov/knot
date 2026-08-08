use crate::models::{EntityKind, ParsedEntity};
use crate::pipeline::parser::languages::{javascript, kotlin, rust, typescript};
use crate::pipeline::parser::orphans::collect_orphaned_references;
use tree_sitter::Node;

/// Find the existing `<module>` entity and set its `default_export`, or create
/// a synthetic one if none exists yet.
fn set_module_default_export(
    entities: &mut Vec<ParsedEntity>,
    target: String,
    lang_name: &str,
    file_path: &str,
    repo_name: &str,
) {
    if let Some(module_entity) = entities.iter_mut().find(|e| e.name == "<module>") {
        module_entity.default_export = Some(target);
    } else {
        // Create a synthetic module entity just to hold the default export
        let mut module_entity = ParsedEntity::new(
            "<module>",
            EntityKind::Function,
            file_path,
            None,
            None,
            lang_name,
            file_path,
            1,
            1,
            None,
            repo_name,
        );
        module_entity.default_export = Some(target);
        entities.push(module_entity);
    }
}

#[expect(
    clippy::too_many_lines,
    reason = "function is verbose but correct — extraction deferred"
)]
#[expect(
    clippy::too_many_arguments,
    reason = "function is verbose but correct — extraction deferred"
)]
pub(crate) fn run_post_passes<'a>(
    tree_root: Node<'a>,
    source_bytes: &[u8],
    lang_name: &str,
    file_path: &str,
    repo_name: &str,
    entities: &mut Vec<ParsedEntity>,
    covered_ranges: &[(usize, usize)],
) {
    // Must run AFTER entity dedup so existing_entities have correct line ranges.
    if lang_name == "kotlin" {
        let mut anon_entities = Vec::new();
        kotlin::extract_anonymous_object_implementations(
            tree_root,
            source_bytes,
            file_path,
            repo_name,
            entities,
            &mut anon_entities,
        );
        entities.extend(anon_entities);
    }

    // Third pass: capture orphaned reference intents (calls in top-level statements,
    // callbacks, etc. that were not captured by any named entity)
    if lang_name == "typescript"
        || lang_name == "java"
        || lang_name == "javascript"
        || lang_name == "kotlin"
        || lang_name == "python"
    {
        collect_orphaned_references(
            tree_root,
            source_bytes,
            lang_name,
            entities,
            covered_ranges,
            file_path,
            repo_name,
        );
    }

    // Rust: collect macro invocations, function calls, type references, and trait implementations
    if lang_name == "rust" {
        rust::collect_rust_macro_references(
            tree_root,
            source_bytes,
            entities,
            file_path,
            repo_name,
        );
        rust::collect_rust_call_references(tree_root, source_bytes, entities, file_path, repo_name);
        rust::collect_rust_type_references(tree_root, source_bytes, entities, file_path, repo_name);
        rust::collect_rust_trait_implementations(
            tree_root,
            source_bytes,
            entities,
            file_path,
            repo_name,
        );
        rust::reclassify_methods_in_impl_blocks(tree_root, source_bytes, entities);
    }

    // Fourth pass: extract HTML attributes from JSX elements (id, className)
    // This enables cross-language CSS/HTML search (e.g., "which components use class 'btn'?")
    if lang_name == "javascript" || lang_name == "typescript" {
        javascript::extract_jsx_html_attributes(
            tree_root,
            source_bytes,
            entities,
            file_path,
            repo_name,
        );
    }

    // Sixth pass: extract alias metadata (require/import → module path)
    if lang_name == "javascript" {
        // Set default_export on <module> entity from module.exports = X
        if let Some(target) = javascript::scan_module_exports_target(tree_root, source_bytes) {
            set_module_default_export(entities, target, lang_name, file_path, repo_name);
        }
    }
    if lang_name == "typescript" {
        // Create entities for renamed/default/namespace imports so that
        // cross-file alias resolution can follow them. Non-renamed named
        // imports (`import { X }`) don't need entities because the name
        // is already captured as a type reference by the main query.
        let aliases = typescript::scan_import_module_aliases(tree_root, source_bytes);
        for (name, module_path, original_name, is_renamed) in aliases {
            if is_renamed && !entities.iter().any(|e| e.name == name) {
                let mut alias_entity = ParsedEntity::new(
                    &name,
                    EntityKind::Constant,
                    file_path,
                    None,
                    None,
                    lang_name,
                    file_path,
                    1,
                    1,
                    None,
                    repo_name,
                );
                alias_entity.alias_module_path = Some(module_path);
                alias_entity.original_export_name = original_name;
                entities.push(alias_entity);
            }
        }
        // Set default_export on <module> from export default X
        if let Some(target) = typescript::scan_default_export_target(tree_root, source_bytes) {
            set_module_default_export(entities, target, lang_name, file_path, repo_name);
        }
    }
}
