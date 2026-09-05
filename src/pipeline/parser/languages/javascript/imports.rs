use crate::models::ReferenceIntent;
use crate::pipeline::parser::utils::{is_capitalized, node_text};
use tree_sitter::Node;

#[expect(
    clippy::too_many_arguments,
    reason = "function is verbose but correct — extraction deferred"
)]
pub(crate) fn collect_import_intents_javascript(
    node: Node<'_>,
    source: &[u8],
    intents: &mut Vec<(ReferenceIntent, usize)>,
    byte_pos: usize,
    line: usize,
    is_type_import: bool,
) {
    let mut cursor = node.walk();
    let import_clause = node
        .children(&mut cursor)
        .find(|c| c.kind() == "import_clause");
    let Some(clause) = import_clause else {
        return;
    };

    let mut clause_child = clause.child(0);
    while let Some(c) = clause_child {
        match c.kind() {
            "named_imports" => {
                let mut spec_child = c.child(0);
                while let Some(spec) = spec_child {
                    if spec.kind() == "import_specifier" {
                        let name_node = spec.child_by_field_name("name");
                        if let Some(nn) = name_node {
                            let name = node_text(nn, source);
                            push_import_ref_if_capitalized(
                                name,
                                is_type_import,
                                intents,
                                byte_pos,
                                line,
                            );
                        }
                    }
                    spec_child = spec.next_sibling();
                }
            }
            "identifier" => {
                let name = node_text(c, source);
                push_import_ref_if_capitalized(name, is_type_import, intents, byte_pos, line);
            }
            _ => {}
        }
        clause_child = c.next_sibling();
    }
}

fn push_import_ref_if_capitalized(
    name: String,
    is_type_import: bool,
    intents: &mut Vec<(ReferenceIntent, usize)>,
    byte_pos: usize,
    line: usize,
) {
    if is_capitalized(&name) {
        if is_type_import {
            intents.push((
                ReferenceIntent::TypeReference {
                    type_name: name,
                    line,
                },
                byte_pos,
            ));
        } else {
            intents.push((
                ReferenceIntent::ValueReference {
                    value_name: name,
                    line,
                },
                byte_pos,
            ));
        }
    }
}

pub(crate) fn collect_require_destructure_intents(
    node: Node<'_>,
    source: &[u8],
    intents: &mut Vec<(ReferenceIntent, usize)>,
    byte_pos: usize,
    line: usize,
) {
    let mut declarator = None;
    let mut child = node.child(0);
    while let Some(c) = child {
        if c.kind() == "variable_declarator" {
            declarator = Some(c);
            break;
        }
        child = c.next_sibling();
    }
    let Some(decl) = declarator else {
        return;
    };

    let Some(value) = decl.child_by_field_name("value") else {
        return;
    };
    if value.kind() != "call_expression" {
        return;
    }
    let Some(func) = value.child_by_field_name("function") else {
        return;
    };
    if func.kind() != "identifier" || node_text(func, source) != "require" {
        return;
    }

    let Some(name_node) = decl.child_by_field_name("name") else {
        return;
    };
    if name_node.kind() != "object_pattern" {
        return;
    }

    let mut pattern_child = name_node.child(0);
    while let Some(pc) = pattern_child {
        match pc.kind() {
            "shorthand_property_identifier_pattern" => {
                let name = node_text(pc, source);
                if is_capitalized(&name) {
                    intents.push((
                        ReferenceIntent::ValueReference {
                            value_name: name,
                            line,
                        },
                        byte_pos,
                    ));
                }
            }
            "pair_pattern" => {
                if let Some(key) = pc.child_by_field_name("key") {
                    let name = node_text(key, source);
                    if is_capitalized(&name) {
                        intents.push((
                            ReferenceIntent::ValueReference {
                                value_name: name,
                                line,
                            },
                            byte_pos,
                        ));
                    }
                }
            }
            _ => {}
        }
        pattern_child = pc.next_sibling();
    }
}

// ── Alias extraction (require / module.exports) ──────────────────

pub(crate) fn extract_require_module_path(node: Node<'_>, source: &[u8]) -> Option<String> {
    let mut declarator = node;
    if node.kind() == "lexical_declaration" || node.kind() == "variable_declaration" {
        let mut child = node.child(0);
        while let Some(c) = child {
            if c.kind() == "variable_declarator" {
                declarator = c;
                break;
            }
            child = c.next_sibling();
        }
    }

    let value = declarator.child_by_field_name("value")?;
    if value.kind() == "new_expression" {
        if let Some(constructor) = value.child_by_field_name("constructor") {
            extract_require_string(constructor, source)
        } else {
            None
        }
    } else if value.kind() == "call_expression" {
        extract_require_string(value, source)
    } else {
        None
    }
}

fn extract_require_string(call_node: Node<'_>, source: &[u8]) -> Option<String> {
    let func = call_node.child_by_field_name("function")?;
    if func.kind() != "identifier" || node_text(func, source) != "require" {
        return None;
    }
    let args = call_node.child_by_field_name("arguments")?;
    let first_arg = args.child(1)?;
    if first_arg.kind() != "string" {
        return None;
    }
    let raw = node_text(first_arg, source);
    Some(
        raw.trim_matches(|c| c == '\'' || c == '"' || c == '`')
            .to_string(),
    )
}

pub(crate) fn scan_module_exports_target(root: Node<'_>, source: &[u8]) -> Option<String> {
    fn walk(node: Node<'_>, source: &[u8]) -> Option<String> {
        if node.kind() == "assignment_expression" {
            let left = node.child_by_field_name("left")?;
            if left.kind() == "member_expression"
                && let Some(obj) = left.child_by_field_name("object")
                && node_text(obj, source) == "module"
                && let Some(prop) = left.child_by_field_name("property")
                && node_text(prop, source) == "exports"
            {
                let right = node.child_by_field_name("right")?;
                if right.kind() == "identifier" {
                    return Some(node_text(right, source));
                }
            }
        }
        let mut child = node.child(0);
        while let Some(c) = child {
            if let Some(result) = walk(c, source) {
                return Some(result);
            }
            child = c.next_sibling();
        }
        None
    }
    walk(root, source)
}
