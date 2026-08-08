use anyhow::{Context, Result};
use streaming_iterator::StreamingIterator;
use tree_sitter::{Language, Parser, Query, QueryCursor};

use crate::models::ParsedEntity;
use crate::pipeline::parser::context::{ClassContext, extract_class_contexts};
use crate::pipeline::parser::languages::java;
use crate::pipeline::parser::utils::*;

mod captures;
mod enrich;
mod post_passes;
#[cfg(test)]
mod tests;

#[expect(
    clippy::too_many_arguments,
    reason = "function is verbose but correct — extraction deferred"
)]
pub(crate) fn extract_entities(
    source: &str,
    language: Language,
    query_src: &str,
    lang_name: &str,
    file_path: &str,
    repo_name: &str,
) -> Result<Vec<ParsedEntity>> {
    let mut parser = Parser::new();
    parser
        .set_language(&language)
        .context("Failed to set Tree-sitter language")?;

    let tree = parser
        .parse(source, None)
        .context("Tree-sitter failed to parse source")?;

    let query = Query::new(&language, query_src).context("Failed to compile Tree-sitter query")?;

    let source_bytes = source.as_bytes();
    let mut cursor = QueryCursor::new();

    let capture_names: Vec<String> = query
        .capture_names()
        .iter()
        .map(|s| s.to_string())
        .collect();

    let mut entities: Vec<ParsedEntity> = Vec::new();

    // First pass: extract all class/interface names and their line ranges for context
    let mut class_contexts: Vec<ClassContext> = Vec::new();
    extract_class_contexts(tree.root_node(), source_bytes, &mut class_contexts);

    // Extract Java package name for FQN prefixing
    let java_package = if lang_name == "java" {
        java::extract_package_name(tree.root_node(), source_bytes)
    } else {
        None
    };

    // Second pass: extract entities and resolve their contexts
    let mut matches = cursor.matches(&query, tree.root_node(), source_bytes);
    let mut covered_ranges: Vec<(usize, usize)> = Vec::new();

    while let Some(m) = {
        matches.advance();
        matches.get()
    } {
        let mut state = captures::CaptureState::default();

        for cap in m.captures {
            let cap_name = &capture_names[cap.index as usize];
            let node = cap.node;
            let text = node_text(node, source_bytes);

            captures::process_capture(cap_name, text, node, source_bytes, lang_name, &mut state);
        }

        enrich::enrich_and_create_entity(
            &mut state,
            source_bytes,
            lang_name,
            file_path,
            repo_name,
            &class_contexts,
            &java_package,
            &mut entities,
            &mut covered_ranges,
        );
    }

    // Deduplication
    entities.sort_by(|a, b| {
        a.file_path
            .cmp(&b.file_path)
            .then(a.name.cmp(&b.name))
            .then(format!("{:?}", a.kind).cmp(&format!("{:?}", b.kind)))
            .then(a.start_line.cmp(&b.start_line))
    });
    entities.dedup_by(|a, b| {
        a.file_path == b.file_path
            && a.name == b.name
            && a.kind == b.kind
            && a.start_line == b.start_line
    });

    post_passes::run_post_passes(
        tree.root_node(),
        source_bytes,
        lang_name,
        file_path,
        repo_name,
        &mut entities,
        &covered_ranges,
    );

    Ok(entities)
}
