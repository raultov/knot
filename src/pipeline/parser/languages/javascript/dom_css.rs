//! DOM/CSS reference capture for JavaScript.
//!
//! Two supplies feed this module:
//!
//! 1. `handle_dom_css_capture` — the per-match capture handler for the
//!    `dom.element_id` / `css.class_name` captures of `javascript.scm`.
//!    Those matches fire on the string argument alone (no enclosing entity)
//!    so the captured intent can never attach to an entity — see
//!    [`collect_dom_css_references`] for the fix.
//! 2. [`collect_dom_css_references`] — a post pass that detects **every**
//!    DOM/CSS manipulation call site straight from the AST and attaches the
//!    intent to the nearest entity, mirroring `collect_orphaned_references`.

use crate::models::{EntityKind, ParsedEntity, ReferenceIntent};
use crate::pipeline::parser::orphans::find_nearest_entity_by_line;
use crate::pipeline::parser::utils::node_text;
use tree_sitter::Node;

/// Handle DOM and CSS reference captures in JavaScript
/// (dom.element_id, css.class_name, etc.)
pub(crate) fn handle_dom_css_capture(
    cap_name: &str,
    text: &str,
    line: usize,
) -> Option<ReferenceIntent> {
    match cap_name {
        "dom.element_id" => Some(ReferenceIntent::DomElementReference {
            element_id: strip_quotes(text),
            line,
        }),
        "css.class_name" | "css.class_assignment" => Some(ReferenceIntent::CssClassUsage {
            class_name: strip_quotes(text),
            line,
        }),
        _ => None,
    }
}

/// Strip the surrounding quote characters of a JS string literal.
fn strip_quotes(text: &str) -> String {
    let mut start = 0usize;
    let chars: Vec<char> = text.chars().collect();
    while start < chars.len() && matches!(chars[start], '"' | '\'' | '`') {
        start += 1;
    }
    let mut end = chars.len();
    while end > start && matches!(chars[end - 1], '"' | '\'' | '`') {
        end -= 1;
    }
    chars[start..end].iter().collect()
}

/// Post-pass: collect DOM/CSS manipulation call sites straight from the AST
/// and attach them to the nearest entity.
///
/// The main `javascript.scm` query captures `dom.element_id` /
/// `css.class_name` on the *string argument* alone — no enclosing entity is
/// captured in the same match, so `enrich_and_create_entity` never attaches
/// those per-match intents to anything and Neo4j never received a
/// `REFERENCES_DOM` / `USES_CSS_CLASS` edge anywhere before this pass.
///
/// Call sites inside an entity's line range attach to that entity; a call
/// site in no containing entity attaches to the synthetic `<module>` entity,
/// which is created on the pattern of `collect_orphaned_references`
/// (spanning the file, pushed last so containment resolution keeps
/// preferring real entities with smaller ranges).
pub(crate) fn collect_dom_css_references(
    root: Node<'_>,
    source: &[u8],
    entities: &mut Vec<ParsedEntity>,
) {
    if entities.is_empty() {
        return;
    }

    let mut intents = Vec::new();
    collect_dom_css_intents(root, source, &mut intents);
    if intents.is_empty() {
        return;
    }

    // Defensive de-duplication of identical call sites (nested query
    // patterns could otherwise revisit one call_expression twice).
    let mut seen = std::collections::HashSet::new();
    intents.retain(|intent| match intent {
        ReferenceIntent::DomElementReference { element_id, line } => {
            seen.insert(format!("dom:{line}:{element_id}"))
        }
        ReferenceIntent::CssClassUsage { class_name, line } => {
            seen.insert(format!("css:{line}:{class_name}"))
        }
        _ => true,
    });

    for intent in intents {
        let line = match &intent {
            ReferenceIntent::DomElementReference { line, .. }
            | ReferenceIntent::CssClassUsage { line, .. } => *line,
            _ => continue,
        };

        let contained = entities
            .iter()
            .any(|e| e.start_line <= line && line <= e.end_line);
        if contained {
            // The containment winner of the nearest-entity resolution is the
            // enclosing function/method — a legitimate attach target.
            let idx = find_nearest_entity_by_line(entities, line);
            entities[idx].reference_intents.push(intent);
        } else {
            attach_to_module_or_create(entities, intent);
        }
    }
}

/// Attach an intent to the existing `<module>` entity, or push one spanning
/// the whole file at the end.
fn attach_to_module_or_create(entities: &mut Vec<ParsedEntity>, intent: ReferenceIntent) {
    if let Some(module) = entities.iter_mut().find(|e| e.name == "<module>") {
        module.reference_intents.push(intent);
        return;
    }

    // Synthetic module mirroring `collect_orphaned_references`: a Function
    // kind spanning 1..usize::MAX, appended last so containment checks
    // (first-wins) still prefer real entities.
    let (lang_name, file_path, repo_name) = entities
        .first()
        .map(|e| (e.language.clone(), e.file_path.clone(), e.repo_name.clone()))
        .unwrap_or_default();

    let mut module_entity = ParsedEntity::new(
        "<module>",
        EntityKind::Function,
        file_path.clone(),
        None,
        None,
        lang_name,
        file_path,
        1,
        usize::MAX,
        None,
        repo_name,
    );
    module_entity.reference_intents.push(intent);
    entities.push(module_entity);
}

/// Recursively detect the DOM/CSS call patterns at `node` and below.
fn collect_dom_css_intents(node: Node<'_>, source: &[u8], intents: &mut Vec<ReferenceIntent>) {
    process_dom_css_node(node, source, intents);
    for child in children(node) {
        collect_dom_css_intents(child, source, intents);
    }
}

/// Detect a DOM/CSS pattern rooted at `node` if it is one.
fn process_dom_css_node(node: Node<'_>, source: &[u8], intents: &mut Vec<ReferenceIntent>) {
    let line = node.start_position().row + 1;
    match node.kind() {
        "call_expression" => {
            let Some(function) = node.child_by_field_name("function") else {
                return;
            };
            match final_property_text(function, source).as_deref() {
                // `document.getElementById('app-container')` — element lookup.
                Some("getElementById") => {
                    if let Some(text) = string_argument(node, source)
                        && let Some(intent) = handle_dom_css_capture("dom.element_id", &text, line)
                    {
                        intents.push(intent);
                    }
                }
                // `element.classList.add('cls')` — the final property (`add`)
                // does not identify the pattern; the callee object's
                // `classList` property does.
                Some("add" | "remove" | "toggle" | "contains" | "replace")
                    if function
                        .child_by_field_name("object")
                        .is_some_and(|callee| {
                            final_property_text(callee, source).as_deref() == Some("classList")
                        }) =>
                {
                    if let Some(text) = string_argument(node, source)
                        && let Some(intent) = handle_dom_css_capture("css.class_name", &text, line)
                    {
                        intents.push(intent);
                    }
                }
                _ => {}
            }
        }
        "assignment_expression" => {
            // `element.className = 'cls'`
            if let Some(left) = node.child_by_field_name("left")
                && final_property_text(left, source).as_deref() == Some("className")
                && let Some(right) = node.child_by_field_name("right")
                && right.kind() == "string"
            {
                let value = node_text(right, source);
                if let Some(intent) = handle_dom_css_capture("css.class_assignment", &value, line) {
                    intents.push(intent);
                }
            }
        }
        _ => {}
    }
}

/// Text of the final `property_identifier` of a member chain, if any.
fn final_property_text(node: Node<'_>, source: &[u8]) -> Option<String> {
    match node.kind() {
        "property_identifier" => Some(node_text(node, source)),
        "member_expression" => node
            .child_by_field_name("property")
            .map(|property| node_text(property, source)),
        _ => None,
    }
}

/// Text of the first string argument of a call, quote-stripped.
fn string_argument(call: Node<'_>, source: &[u8]) -> Option<String> {
    let arguments = call.child_by_field_name("arguments")?;
    for arg in children(arguments) {
        if arg.kind() == "string" {
            return Some(strip_quotes(&node_text(arg, source)));
        }
    }
    None
}

/// Children of `node`, materialised so re-walking does not borrow the cursor.
fn children<'a>(node: Node<'a>) -> Vec<Node<'a>> {
    let mut out = Vec::new();
    let mut cursor = node.walk();
    if cursor.goto_first_child() {
        loop {
            out.push(cursor.node());
            if !cursor.goto_next_sibling() {
                break;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pipeline::parser::extractor::extract_entities;

    fn parse_entities(source: &str) -> Vec<ParsedEntity> {
        extract_entities(
            source,
            tree_sitter_javascript::LANGUAGE.into(),
            include_str!("../../../../../queries/javascript.scm"),
            "javascript",
            "/app.js",
            "test-repo",
        )
        .expect("parse")
    }

    #[test]
    fn post_pass_attaches_dom_and_css_intents_to_enclosing_entity() {
        // Single-line fixture: both the `app` constant (its initializer
        // contains the call) and `initApp` contain line 1 — the containment
        // winner (first containing entity) is the valid attach target.
        let source = "function initApp() { const app = document.getElementById('app-container'); element.classList.add('active'); }";

        let entities = parse_entities(source);
        let has_dom = entities.iter().any(|e| {
            e.reference_intents
                .iter()
                .any(|i| matches!(i, ReferenceIntent::DomElementReference { element_id, .. } if element_id == "app-container"))
        });
        let has_css = entities.iter().any(|e| {
            e.reference_intents
                .iter()
                .any(|i| matches!(i, ReferenceIntent::CssClassUsage { class_name, .. } if class_name == "active"))
        });
        assert!(has_dom && has_css, "dom={has_dom} css={has_css}");
    }

    #[test]
    fn post_pass_never_drops_top_level_dom_intents() {
        // Top-level statement: the intent must attach to *some* entity —
        // here the `appContainer` constant whose initializer contains the
        // call — rather than being dropped like the per-match mechanism did.
        let source = "const appContainer = document.getElementById('app-container');";
        let entities = parse_entities(source);
        let attached = entities.iter().any(|e| {
            e.reference_intents
                .iter()
                .any(|i| matches!(i, ReferenceIntent::DomElementReference { element_id, .. } if element_id == "app-container"))
        });
        assert!(attached, "top-level DOM use must reach an entity");
    }
}
