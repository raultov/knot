//! Groovy language support (`.groovy`) — hybrid lexical parser.
//!
//! tree-sitter-groovy (v0.1.2) query compilation is unreliable on CI runners
//! with tree-sitter 0.26 (disabled since v1.2.0), so entity extraction is
//! driven by an ad-hoc lexical parser that provides equivalent entity coverage
//! (classes, interfaces, enums, traits, methods, properties) plus scope
//! tracking and reference-intent extraction not available via tree-sitter
//! alone. [`capture`] stays wired for the query-driven path and the `.scm`
//! contract tests.
//!
//! Submodules group the pipeline by responsibility:
//!
//! | Submodule       | Responsibility |
//! |-----------------|----------------|
//! | [`capture`]     | Tree-sitter capture name → entity mapping |
//! | [`inheritance`] | Type declarations (`class`/`interface`/`enum`/`trait`) and `extends` / `implements` intents |
//! | [`methods`]     | `def` / typed / multi-line method detection and body-extent fixup |
//! | [`properties`]  | Property, field and script-variable detection |
//! | [`accessors`]   | Synthetic getter/setter emission for properties |
//! | [`refs`]        | Method-call reference-intent scanning |
//! | [`utils`]       | Comment stripping, FQN construction, GroovyDoc extraction, validators |

mod accessors;
mod capture;
mod inheritance;
mod methods;
mod properties;
mod refs;
mod utils;

#[cfg(test)]
mod tests;

pub(crate) use capture::handle_groovy_capture;

use crate::models::{EntityKind, ParsedEntity, ReferenceIntent};

use accessors::synthesize_property_accessors;
use inheritance::{
    build_type_declaration, extract_inheritance_intents, try_extract_type_declaration,
};
use methods::{
    find_method_body_end, try_extract_def_method, try_extract_typed_method,
    try_extract_typed_method_multiline,
};
use properties::{GroovyPropertyDecl, try_extract_property};
use refs::extract_method_calls;
use utils::{build_fqn, extract_package, extract_preceding_docstring, strip_comments_line};

/// Extracts entities from a Groovy source file with the ad-hoc lexical parser.
#[expect(
    clippy::too_many_lines,
    reason = "function is verbose but correct — extraction deferred"
)]
#[expect(
    clippy::cognitive_complexity,
    reason = "function is verbose but correct — extraction deferred"
)]
pub(crate) fn extract_entities_groovy(
    source: &str,
    file_path: &str,
    repo_name: &str,
) -> Vec<ParsedEntity> {
    // tree-sitter-groovy (v0.1.2) query compilation fails intermittently on CI
    // runners with tree-sitter 0.26. The ad-hoc lexical parser provides equivalent
    // entity coverage (classes, interfaces, enums, traits, methods, properties) plus
    // scope tracking and reference intent extraction not available via tree-sitter alone.
    let mut entities: Vec<ParsedEntity> = vec![];

    // Extract package declaration
    let package = extract_package(source);

    // Keep track of lines where entities were found to avoid duplicates
    let mut known_lines = std::collections::HashSet::new();

    // Post-process entities from Tree-sitter
    for entity in entities.iter_mut() {
        known_lines.insert(entity.start_line);

        // Fix: tree-sitter-groovy parses `trait` as `class_declaration`.
        if entity.kind == EntityKind::GroovyClass
            && let Some(line_content) = source.lines().nth(entity.start_line.saturating_sub(1))
        {
            let trimmed = line_content.trim();
            if trimmed.starts_with("trait ") || trimmed.contains(" trait ") {
                entity.kind = EntityKind::GroovyTrait;
            }
        }

        // Set FQN for tree-sitter entities
        if let Some(pkg) = &package {
            entity.fqn = match entity.kind {
                EntityKind::GroovyClass
                | EntityKind::GroovyInterface
                | EntityKind::GroovyTrait
                | EntityKind::GroovyEnum => format!("{}.{}", pkg, entity.name),
                _ => continue,
            };
        }
    }

    // Ad-hoc extraction with scope tracking for enclosing_class
    // Scope stack: (name, brace_count_when_entered)
    let mut scope_stack: Vec<(String, usize)> = Vec::new();
    let mut brace_count = 0usize;
    let mut in_block_comment = false;

    // Side-car: property metadata needed for accessor synthesis (Phase 3).
    let mut prop_decls: std::collections::HashMap<(String, String), GroovyPropertyDecl> =
        std::collections::HashMap::new();

    // Materialize lines once: docstring extraction walks backwards from each
    // declaration and must not pay O(n²) re-scanning the source per entity.
    let lines: Vec<&str> = source.lines().collect();

    for (line_idx, line) in source.lines().enumerate() {
        let line_num = line_idx + 1;

        let effective = strip_comments_line(line, &mut in_block_comment);

        // Track braces on the effective (code-bearing) line only.
        let opened = effective.matches('{').count();
        let closed = effective.matches('}').count();
        let prev_brace_count = brace_count;
        brace_count += opened;

        let mut early_pop = false;
        if closed > opened {
            let temp_brace = brace_count.saturating_sub(closed);
            while let Some((_, entry_brace)) = scope_stack.last() {
                if temp_brace < *entry_brace {
                    scope_stack.pop();
                    early_pop = true;
                } else {
                    break;
                }
            }
        }

        if effective.is_empty() {
            continue;
        }

        // Try to extract class/interface/enum/trait if tree-sitter missed it
        if !known_lines.contains(&line_num)
            && let Some((name, kind)) = try_extract_type_declaration(effective.as_ref())
        {
            // Push to scope stack BEFORE brace_count is updated for the current line's `{`
            let fqn = if let Some(pkg) = &package {
                format!("{}.{}", pkg, name)
            } else {
                name.clone()
            };
            let current_brace = brace_count;
            // Build a multi-line declaration so that `extends X` / `implements Y`
            // on a following line still feed `extract_inheritance_intents`.
            // Falls back to the single trimmed line when no `{` is in the
            // lookahead window.
            let decl_text =
                build_type_declaration(source, line_idx).unwrap_or_else(|| effective.to_string());
            let inheritance_intents = extract_inheritance_intents(&decl_text, &kind, line_num);
            let docstring = extract_preceding_docstring(&lines, line_idx);
            let mut new_entity = ParsedEntity::new(
                &name, kind, &fqn, None, docstring, "groovy", file_path, line_num, line_num, None,
                repo_name,
            );
            new_entity.reference_intents.extend(inheritance_intents);
            entities.push(new_entity);
            scope_stack.push((name, current_brace));
        }

        // Re-read enclosing after potential scope push
        let enclosing = scope_stack.last().map(|(n, _)| n.clone());

        // Ad-hoc method/field/closure extraction only if tree-sitter didn't already find an entity at this line
        if !known_lines.contains(&line_num) {
            // Try to find a `def` method declaration
            if let Some((method_name, signature)) = try_extract_def_method(effective.as_ref()) {
                let fqn = build_fqn(&package, &enclosing, &method_name);
                let docstring = extract_preceding_docstring(&lines, line_idx);
                entities.push(ParsedEntity::new(
                    &method_name,
                    EntityKind::GroovyMethod,
                    &fqn,
                    Some(signature),
                    docstring,
                    "groovy",
                    file_path,
                    line_num,
                    line_num,
                    enclosing,
                    repo_name,
                ));
                continue;
            }

            // Try to find typed methods or script-level methods missed by tree-sitter
            // First, try single-line detection
            if let Some((method_name, _signature)) = try_extract_typed_method(effective.as_ref()) {
                // Filter false positives: method names that contain dots or look like object.method()
                if method_name.contains('.')
                    || method_name.chars().all(|c| c.is_uppercase() || c == '_')
                {
                    continue;
                }
                let sig_end = effective.find('{').unwrap_or(effective.len());
                let signature_full = effective[..sig_end].trim().to_string();
                let fqn = build_fqn(&package, &enclosing, &method_name);
                let docstring = extract_preceding_docstring(&lines, line_idx);
                entities.push(ParsedEntity::new(
                    &method_name,
                    EntityKind::GroovyMethod,
                    &fqn,
                    Some(signature_full),
                    docstring,
                    "groovy",
                    file_path,
                    line_num,
                    line_num,
                    enclosing,
                    repo_name,
                ));
                continue;
            }

            // Multi-line method detection: method signature with `(` but no `)` on this line,
            // spanning multiple lines (e.g., closure default parameter values)
            if let Some((method_name, method_start_line)) =
                try_extract_typed_method_multiline(source, line_idx)
                && !method_name.contains('.')
            {
                let fqn = build_fqn(&package, &enclosing, &method_name);
                // The docstring sits above the first line of the signature
                // (method_start_line is 1-based → subtract 1 for the 0-based index).
                let docstring = extract_preceding_docstring(&lines, method_start_line - 1);
                entities.push(ParsedEntity::new(
                    &method_name,
                    EntityKind::GroovyMethod,
                    &fqn,
                    None,
                    docstring,
                    "groovy",
                    file_path,
                    method_start_line,
                    line_num,
                    enclosing,
                    repo_name,
                ));
                continue;
            }
        }

        // Try to extract properties or script-level variables.
        // Gated at type-body depth, or at script level when no enclosing type is in scope.
        // `at_type_body_depth` uses the post-increment brace_count so that a single-line
        // class declaration (`class Foo { String name }`) matches: the type's `{` opens
        // before the body's `at_type_body_depth` check happens on the same line.
        // `at_script_level` uses `prev_brace_count == 0` so that a script-level property
        // declaring a closure literal on the same line (`def foo = { ... }`) also matches.
        let at_type_body_depth = scope_stack
            .last()
            .is_some_and(|(_, entry_brace)| brace_count == *entry_brace);
        let at_script_level = scope_stack.is_empty() && prev_brace_count == 0;

        if (at_type_body_depth || at_script_level)
            && !known_lines.contains(&line_num)
            && let Some(prop_decl) = try_extract_property(effective.as_ref())
        {
            let fqn = build_fqn(&package, &enclosing, &prop_decl.name);
            let docstring = extract_preceding_docstring(&lines, line_idx);
            let enclosing_for_prop = enclosing.clone();
            let name_clone = prop_decl.name.clone();
            let enc_clone = enclosing_for_prop.clone();
            entities.push(ParsedEntity::new(
                &prop_decl.name,
                EntityKind::GroovyProperty,
                &fqn,
                None,
                docstring,
                "groovy",
                file_path,
                line_num,
                line_num,
                enclosing_for_prop,
                repo_name,
            ));
            if let Some(enc) = enc_clone {
                prop_decls.insert((enc, name_clone), prop_decl);
            }
        }

        brace_count = brace_count.saturating_sub(closed);
        if !early_pop {
            while let Some((_, entry_brace)) = scope_stack.last() {
                if brace_count < *entry_brace {
                    scope_stack.pop();
                } else {
                    break;
                }
            }
        }
    }

    // Fix end_line for all methods (both tree-sitter and ad-hoc) that
    // couldn't determine their body closing line.
    for entity in entities.iter_mut() {
        if entity.kind == EntityKind::GroovyMethod
            && entity.end_line == entity.start_line
            && let Some(end_line) = find_method_body_end(source, entity.start_line)
            && end_line > entity.start_line
        {
            entity.end_line = end_line;
        }
    }

    // Phase 3: emit synthetic accessor entities for Groovy properties
    // so OVERRIDES linking can match against interface getters/setters.
    synthesize_property_accessors(&mut entities, &package, file_path, repo_name, &prop_decls);

    // Extract reference intents: for each method, scan source lines after its signature
    let mut method_spans: Vec<(usize, usize, usize)> = entities
        .iter()
        .enumerate()
        .filter(|(_, e)| {
            matches!(
                e.kind,
                EntityKind::GroovyMethod | EntityKind::GroovyFunction
            )
        })
        .map(|(i, e)| (e.start_line, e.end_line, i))
        .collect();
    method_spans.sort_by_key(|(s, _, _)| *s);

    let refs = extract_method_calls(source, &entities);

    // Assign each reference intent to the innermost containing method.
    // When methods are nested (e.g., hyperlinkUpdate inside showGrabbingFinishedMessage),
    // we assign the call to the deepest method, not the outer container.
    for method_ref in refs.iter() {
        if let ReferenceIntent::Call { line, .. } = method_ref {
            // Find all methods that contain this line
            let mut candidates: Vec<(usize, usize, usize)> = method_spans
                .iter()
                .filter(|(m_start, m_end, _)| {
                    let actual_end = if *m_end != *m_start { *m_end } else { *m_start };
                    *line > *m_start && *line <= actual_end
                })
                .copied()
                .collect();
            // Pick the innermost: smallest (end - start) wins
            candidates.sort_by_key(|(s, e, _)| e.saturating_sub(*s));
            if let Some(&(_, _, m_eidx)) = candidates.first() {
                entities[m_eidx].reference_intents.push(method_ref.clone());
            }
        }
    }

    entities
}
