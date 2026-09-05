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
    postprocess_tree_sitter_entities(&mut entities, source, &package, &mut known_lines);

    let mut scan = GroovyScan {
        source,
        file_path,
        repo_name,
        package: &package,
        lines: source.lines().collect(),
        known_lines,
        scope_stack: Vec::new(),
        brace_count: 0,
        in_block_comment: false,
        prop_decls: std::collections::HashMap::new(),
        entities,
    };

    for (line_idx, line) in source.lines().enumerate() {
        scan.process_line(line_idx, line);
    }

    let GroovyScan {
        mut entities,
        prop_decls,
        ..
    } = scan;

    // Fix end_line for all methods (both tree-sitter and ad-hoc) that
    // couldn't determine their body closing line.
    fix_method_end_lines(&mut entities, source);

    // Phase 3: emit synthetic accessor entities for Groovy properties
    // so OVERRIDES linking can match against interface getters/setters.
    synthesize_property_accessors(&mut entities, &package, file_path, repo_name, &prop_decls);

    // Extract reference intents: for each method, scan source lines after its signature
    assign_method_call_references(source, &mut entities);

    entities
}

/// Marks lines claimed by tree-sitter entities, fixes the `trait` misparse
/// and sets package-qualified FQNs for type entities.
fn postprocess_tree_sitter_entities(
    entities: &mut [ParsedEntity],
    source: &str,
    package: &Option<String>,
    known_lines: &mut std::collections::HashSet<usize>,
) {
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
        if let Some(pkg) = package {
            entity.fqn = match entity.kind {
                EntityKind::GroovyClass
                | EntityKind::GroovyInterface
                | EntityKind::GroovyTrait
                | EntityKind::GroovyEnum => format!("{}.{}", pkg, entity.name),
                _ => continue,
            };
        }
    }
}

/// State threaded through the ad-hoc line scan: scope tracking for
/// `enclosing_class`, the line registry claimed by tree-sitter, and the
/// property side-car needed for accessor synthesis.
struct GroovyScan<'a> {
    source: &'a str,
    file_path: &'a str,
    repo_name: &'a str,
    package: &'a Option<String>,
    /// Materialized once: docstring extraction walks backwards from each
    /// declaration and must not pay O(n²) re-scanning the source per entity.
    lines: Vec<&'a str>,
    known_lines: std::collections::HashSet<usize>,
    /// Scope stack: (name, brace_count_when_entered)
    scope_stack: Vec<(String, usize)>,
    brace_count: usize,
    in_block_comment: bool,
    /// Side-car: property metadata needed for accessor synthesis (Phase 3).
    prop_decls: std::collections::HashMap<(String, String), GroovyPropertyDecl>,
    entities: Vec<ParsedEntity>,
}

impl<'a> GroovyScan<'a> {
    /// Ad-hoc extraction with scope tracking for `enclosing_class`.
    fn process_line(&mut self, line_idx: usize, line: &'a str) {
        let line_num = line_idx + 1;

        let effective = strip_comments_line(line, &mut self.in_block_comment);

        // Track braces on the effective (code-bearing) line only.
        let opened = effective.matches('{').count();
        let closed = effective.matches('}').count();
        let prev_brace_count = self.brace_count;
        self.brace_count += opened;

        let mut early_pop = false;
        if closed > opened {
            let temp_brace = self.brace_count.saturating_sub(closed);
            while let Some((_, entry_brace)) = self.scope_stack.last() {
                if temp_brace < *entry_brace {
                    self.scope_stack.pop();
                    early_pop = true;
                } else {
                    break;
                }
            }
        }

        if effective.is_empty() {
            return;
        }

        // Try to extract class/interface/enum/trait if tree-sitter missed it
        self.try_type_declaration(effective.as_ref(), line_num);

        // Re-read enclosing after potential scope push
        let enclosing = self.scope_stack.last().map(|(n, _)| n.clone());

        // Ad-hoc method/field/closure extraction only if tree-sitter didn't
        // already find an entity at this line. A consumed line skips property
        // extraction and the trailing brace/scope update (original `continue`).
        if self.try_methods(effective.as_ref(), line_num, &enclosing) {
            return;
        }

        self.try_property(effective.as_ref(), line_num, &enclosing, prev_brace_count);

        self.brace_count = self.brace_count.saturating_sub(closed);
        if !early_pop {
            while let Some((_, entry_brace)) = self.scope_stack.last() {
                if self.brace_count < *entry_brace {
                    self.scope_stack.pop();
                } else {
                    break;
                }
            }
        }
    }

    /// Extracts class/interface/enum/trait declarations tree-sitter missed.
    fn try_type_declaration(&mut self, effective: &str, line_num: usize) {
        if !self.known_lines.contains(&line_num)
            && let Some((name, kind)) = try_extract_type_declaration(effective)
        {
            // Push to scope stack BEFORE brace_count is updated for the current line's `{`
            let fqn = if let Some(pkg) = self.package {
                format!("{}.{}", pkg, name)
            } else {
                name.clone()
            };
            let current_brace = self.brace_count;
            // Build a multi-line declaration so that `extends X` / `implements Y`
            // on a following line still feed `extract_inheritance_intents`.
            // Falls back to the single trimmed line when no `{` is in the
            // lookahead window.
            let decl_text = build_type_declaration(self.source, line_num - 1)
                .unwrap_or_else(|| effective.to_string());
            let inheritance_intents = extract_inheritance_intents(&decl_text, &kind, line_num);
            let docstring = extract_preceding_docstring(&self.lines, line_num - 1);
            let mut new_entity = ParsedEntity::new(
                &name,
                kind,
                &fqn,
                None,
                docstring,
                "groovy",
                self.file_path,
                line_num,
                line_num,
                None,
                self.repo_name,
            );
            new_entity.reference_intents.extend(inheritance_intents);
            self.entities.push(new_entity);
            self.scope_stack.push((name, current_brace));
        }
    }

    /// Tries the three method-extraction heuristics (def, typed single-line,
    /// typed multi-line). Returns `true` when the line was consumed — the
    /// caller then skips property extraction and the trailing brace/scope
    /// update, mirroring the original `continue` paths.
    fn try_methods(
        &mut self,
        effective: &str,
        line_num: usize,
        enclosing: &Option<String>,
    ) -> bool {
        // Ad-hoc method/field/closure extraction only if tree-sitter didn't already find an entity at this line
        if self.known_lines.contains(&line_num) {
            return false;
        }
        let line_idx = line_num - 1;

        // Try to find a `def` method declaration
        if let Some((method_name, signature)) = try_extract_def_method(effective) {
            let fqn = build_fqn(self.package, enclosing, &method_name);
            let docstring = extract_preceding_docstring(&self.lines, line_idx);
            self.entities.push(ParsedEntity::new(
                &method_name,
                EntityKind::GroovyMethod,
                &fqn,
                Some(signature),
                docstring,
                "groovy",
                self.file_path,
                line_num,
                line_num,
                enclosing.clone(),
                self.repo_name,
            ));
            return true;
        }

        // Try to find typed methods or script-level methods missed by tree-sitter
        // First, try single-line detection
        if let Some((method_name, _signature)) = try_extract_typed_method(effective) {
            // Filter false positives: method names that contain dots or look like object.method()
            if method_name.contains('.')
                || method_name.chars().all(|c| c.is_uppercase() || c == '_')
            {
                // False positive — still consume the line (original `continue`)
                return true;
            }
            let sig_end = effective.find('{').unwrap_or(effective.len());
            let signature_full = effective[..sig_end].trim().to_string();
            let fqn = build_fqn(self.package, enclosing, &method_name);
            let docstring = extract_preceding_docstring(&self.lines, line_idx);
            self.entities.push(ParsedEntity::new(
                &method_name,
                EntityKind::GroovyMethod,
                &fqn,
                Some(signature_full),
                docstring,
                "groovy",
                self.file_path,
                line_num,
                line_num,
                enclosing.clone(),
                self.repo_name,
            ));
            return true;
        }

        // Multi-line method detection: method signature with `(` but no `)` on this line,
        // spanning multiple lines (e.g., closure default parameter values)
        if let Some((method_name, method_start_line)) =
            try_extract_typed_method_multiline(self.source, line_idx)
            && !method_name.contains('.')
        {
            let fqn = build_fqn(self.package, enclosing, &method_name);
            // The docstring sits above the first line of the signature
            // (method_start_line is 1-based → subtract 1 for the 0-based index).
            let docstring = extract_preceding_docstring(&self.lines, method_start_line - 1);
            self.entities.push(ParsedEntity::new(
                &method_name,
                EntityKind::GroovyMethod,
                &fqn,
                None,
                docstring,
                "groovy",
                self.file_path,
                method_start_line,
                line_num,
                enclosing.clone(),
                self.repo_name,
            ));
            return true;
        }

        false
    }

    /// Extracts properties or script-level variables. Gated at type-body
    /// depth, or at script level when no enclosing type is in scope.
    /// `at_type_body_depth` uses the post-increment brace_count so that a
    /// single-line class declaration (`class Foo { String name }`) matches:
    /// the type's `{` opens before the body's `at_type_body_depth` check
    /// happens on the same line. `at_script_level` uses `prev_brace_count ==
    /// 0` so that a script-level property declaring a closure literal on the
    /// same line (`def foo = { ... }`) also matches.
    fn try_property(
        &mut self,
        effective: &str,
        line_num: usize,
        enclosing: &Option<String>,
        prev_brace_count: usize,
    ) {
        let at_type_body_depth = self
            .scope_stack
            .last()
            .is_some_and(|(_, entry_brace)| self.brace_count == *entry_brace);
        let at_script_level = self.scope_stack.is_empty() && prev_brace_count == 0;

        if !(at_type_body_depth || at_script_level) || self.known_lines.contains(&line_num) {
            return;
        }
        let Some(prop_decl) = try_extract_property(effective) else {
            return;
        };
        self.push_property(prop_decl, line_num, enclosing);
    }

    /// Pushes one property entity and registers it in the accessor-synthesis
    /// side-car keyed by (enclosing type, property name).
    fn push_property(
        &mut self,
        prop_decl: GroovyPropertyDecl,
        line_num: usize,
        enclosing: &Option<String>,
    ) {
        let fqn = build_fqn(self.package, enclosing, &prop_decl.name);
        let docstring = extract_preceding_docstring(&self.lines, line_num - 1);
        let enclosing_for_prop = enclosing.clone();
        let name_clone = prop_decl.name.clone();
        let enc_clone = enclosing_for_prop.clone();
        self.entities.push(ParsedEntity::new(
            &prop_decl.name,
            EntityKind::GroovyProperty,
            &fqn,
            None,
            docstring,
            "groovy",
            self.file_path,
            line_num,
            line_num,
            enclosing_for_prop,
            self.repo_name,
        ));
        if let Some(enc) = enc_clone {
            self.prop_decls.insert((enc, name_clone), prop_decl);
        }
    }
}

/// Fix end_line for all methods (both tree-sitter and ad-hoc) that couldn't
/// determine their body closing line.
fn fix_method_end_lines(entities: &mut [ParsedEntity], source: &str) {
    for entity in entities.iter_mut() {
        if entity.kind == EntityKind::GroovyMethod
            && entity.end_line == entity.start_line
            && let Some(end_line) = find_method_body_end(source, entity.start_line)
            && end_line > entity.start_line
        {
            entity.end_line = end_line;
        }
    }
}

/// Extracts call reference intents and assigns each to the innermost
/// containing method. When methods are nested (e.g., hyperlinkUpdate inside
/// showGrabbingFinishedMessage), the call is assigned to the deepest method,
/// not the outer container.
fn assign_method_call_references(source: &str, entities: &mut [ParsedEntity]) {
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

    let refs = extract_method_calls(source, entities);

    // Assign each reference intent to the innermost containing method.
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
}
