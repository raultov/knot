mod dialect;
mod lexer;
mod vcc;
mod vcl;
mod vtc;

pub(crate) use vcc::extract_entities_vcc;
pub(crate) use vcl::extract_entities_vcl;
pub(crate) use vtc::extract_entities_vtc;

use crate::models::{EntityKind, ParsedEntity};

/// Post-process: emit one synthetic aggregator entity per built-in sub name
/// across all parsed VCL/VTC files in a repo.
///
/// Each aggregator gets:
/// - `name` = `<sub_name>_aggregator`
/// - `kind` = `VclBuiltinSub`
/// - `fqn` = `vcl:<repo>:<sub_name>`
/// - `file_path` = the lexicographically first file containing the sub
///   (matches `discover_files` sort order at `src/pipeline/input.rs:184`)
///
/// Must be called once after all VCL/VTC files in the repo have been parsed.
pub(crate) fn aggregate_varnish_builtin_subs(entities: &mut Vec<ParsedEntity>, repo_name: &str) {
    use std::collections::BTreeMap;

    // Group indices by sub name, sorted by file_path for deterministic first-file selection.
    let mut groups: BTreeMap<String, Vec<(String, usize)>> = BTreeMap::new();
    for (i, e) in entities.iter().enumerate() {
        if e.kind == EntityKind::VclBuiltinSub && !e.name.ends_with("_aggregator") {
            groups
                .entry(e.name.clone())
                .or_default()
                .push((e.file_path.clone(), i));
        }
    }

    for (sub_name, mut entries) in groups {
        entries.sort_by(|a, b| a.0.cmp(&b.0));
        let first_file = entries[0].0.clone();

        let aggregator = ParsedEntity::new(
            format!("{}_aggregator", sub_name),
            EntityKind::VclBuiltinSub,
            format!("vcl:{}:{}", repo_name, sub_name),
            Some(format!("sub {}", sub_name)),
            Some("Aggregator for multi-part built-in sub".to_string()),
            "vcl",
            first_file,
            0,
            0,
            None,
            repo_name,
        );
        entities.push(aggregator);
    }
}
