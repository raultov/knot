use super::languages::{java, javascript, python, typescript};
use crate::models::{EntityKind, ParsedEntity, ReferenceIntent};
use tree_sitter::Node;

/// Third pass: find call_expression / new_expression / jsx nodes that fall outside
/// the byte ranges of extracted entities, and assign them to the nearest entity.
/// If no entities exist, create a synthetic <module> entity.
pub(crate) fn collect_orphaned_references(
    root: Node<'_>,
    source: &[u8],
    lang_name: &str,
    entities: &mut Vec<ParsedEntity>,
    covered_ranges: &[(usize, usize)],
    file_path: &str,
    repo_name: &str,
) {
    // Collect all reference intents from the entire file
    let mut all_intents: Vec<(ReferenceIntent, usize)> = Vec::new();
    collect_all_reference_intents_with_byte_pos(root, source, lang_name, &mut all_intents);

    // Filter to orphaned intents (not covered by any entity)
    let orphaned_intents: Vec<ReferenceIntent> = all_intents
        .into_iter()
        .filter(|(_, byte_pos)| {
            !covered_ranges
                .iter()
                .any(|(start, end)| byte_pos >= start && byte_pos < end)
        })
        .map(|(intent, _)| intent)
        .collect();

    if orphaned_intents.is_empty() {
        return;
    }

    // Create a synthetic <module> entity that spans the entire file (fallback for
    // module-level orphans not contained by any real entity). Always present so
    // find_nearest_entity_by_line containment pass can match it when needed.
    let module_kind = match lang_name {
        "python" => EntityKind::PythonModule,
        "rust" => EntityKind::RustModule,
        _ => EntityKind::Function,
    };
    let mut module_entity = ParsedEntity::new(
        "<module>",
        module_kind,
        "<module>",
        None,
        None,
        lang_name,
        file_path,
        1,
        usize::MAX,
        None,
        repo_name,
    );

    if entities.is_empty() {
        module_entity.reference_intents = orphaned_intents;
        entities.push(module_entity);
        return;
    }

    // Push module entity at the end so containment check prefers real entities
    // (smaller range) over the module (usize::MAX range)
    entities.push(module_entity);

    // Assign each orphan individually to its nearest entity by line
    for intent in orphaned_intents {
        let orphan_line = match &intent {
            ReferenceIntent::Call { line, .. } => *line,
            ReferenceIntent::Extends { line, .. } => *line,
            ReferenceIntent::Implements { line, .. } => *line,
            ReferenceIntent::TypeReference { line, .. } => *line,
            ReferenceIntent::ValueReference { line, .. } => *line,
            ReferenceIntent::DomElementReference { line, .. } => *line,
            ReferenceIntent::CssClassUsage { line, .. } => *line,
            ReferenceIntent::HtmlFileImport { line, .. } => *line,
            ReferenceIntent::CssFileImport { line, .. } => *line,
            ReferenceIntent::RustMacroCall { line, .. } => *line,
        };
        let target_idx = find_nearest_entity_by_line(entities, orphan_line);
        entities[target_idx].reference_intents.push(intent);
    }
}

/// Collect ALL call/new/jsx intents from the entire AST, paired with byte position.
pub(crate) fn collect_all_reference_intents_with_byte_pos(
    node: Node<'_>,
    source: &[u8],
    lang_name: &str,
    intents: &mut Vec<(ReferenceIntent, usize)>,
) {
    if lang_name == "typescript" {
        typescript::collect_all_reference_intents_typescript(node, source, intents);
    } else if lang_name == "java" {
        java::collect_all_reference_intents_java(node, source, intents);
    } else if lang_name == "javascript" {
        javascript::collect_all_reference_intents_javascript(node, source, intents);
    } else if lang_name == "python" {
        let mut python_call_intents = Vec::new();
        python::extract_call_intents_python(node, source, &mut python_call_intents);
        for call in python_call_intents {
            intents.push((
                ReferenceIntent::Call {
                    method: call.method,
                    receiver: call.receiver,
                    line: call.line,
                },
                node.start_byte(),
            ));
        }
        let mut python_import_intents = Vec::new();
        python::extract_import_intents_python(node, source, &mut python_import_intents);
        for intent in python_import_intents {
            intents.push((intent, node.start_byte()));
        }
        let mut python_value_intents = Vec::new();
        python::extract_value_references_python(node, source, &mut python_value_intents);
        for intent in python_value_intents {
            intents.push((intent, node.start_byte()));
        }
    }
}

/// Find the entity index nearest to the given line number.
/// First pass: tries to find an entity that CONTAINS the target line (start_line <= target_line <= end_line).
/// Second pass: falls back to closest entity at or before target_line.
/// Third pass: falls back to closest entity overall.
pub(crate) fn find_nearest_entity_by_line(entities: &[ParsedEntity], target_line: usize) -> usize {
    let mut best_idx = 0;
    let mut best_distance = usize::MAX;

    // First pass: find entity that contains the target line (start_line <= target_line <= end_line)
    for (idx, entity) in entities.iter().enumerate() {
        if target_line >= entity.start_line && target_line <= entity.end_line {
            // Prefer the entity with smallest range that contains the target
            let range_size = entity.end_line - entity.start_line;
            if range_size < best_distance {
                best_distance = range_size;
                best_idx = idx;
            }
        }
    }

    // If no entity contains the target, second pass: find closest entity at or before target_line
    if best_distance == usize::MAX {
        for (idx, entity) in entities.iter().enumerate() {
            let entity_line = entity.start_line;
            if entity_line <= target_line {
                let distance = target_line - entity_line;
                if distance < best_distance {
                    best_distance = distance;
                    best_idx = idx;
                }
            }
        }
    }

    // If still no match, third pass: fall back to closest entity overall
    if best_distance == usize::MAX {
        for (idx, entity) in entities.iter().enumerate() {
            let distance = entity.start_line.abs_diff(target_line);
            if distance < best_distance {
                best_distance = distance;
                best_idx = idx;
            }
        }
    }

    best_idx
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::ParsedEntity;

    #[test]
    fn test_find_nearest_entity_by_line_before() {
        // Create mock entities at specific lines
        let entity1 = ParsedEntity::new(
            "Entity1",
            crate::models::EntityKind::Class,
            "Entity1",
            None,
            None,
            "java",
            "/test.java",
            10,
            20,
            None,
            "test-repo",
        );
        let entity2 = ParsedEntity::new(
            "Entity2",
            crate::models::EntityKind::Method,
            "Entity2",
            None,
            None,
            "java",
            "/test.java",
            30,
            40,
            None,
            "test-repo",
        );
        let entity3 = ParsedEntity::new(
            "Entity3",
            crate::models::EntityKind::Function,
            "Entity3",
            None,
            None,
            "java",
            "/test.java",
            50,
            60,
            None,
            "test-repo",
        );

        let entities = vec![entity1, entity2, entity3];

        // Target line 25 should be assigned to entity1 (at line 10, since 25 > 10 and closest entity before it)
        let idx = find_nearest_entity_by_line(&entities, 25);
        assert_eq!(idx, 0, "Line 25 should be assigned to entity1 (line 10)");
    }

    #[test]
    fn test_find_nearest_entity_by_line_exact() {
        let entity1 = ParsedEntity::new(
            "Entity1",
            crate::models::EntityKind::Class,
            "Entity1",
            None,
            None,
            "java",
            "/test.java",
            10,
            20,
            None,
            "test-repo",
        );

        let entities = vec![entity1];

        let idx = find_nearest_entity_by_line(&entities, 10);
        assert_eq!(idx, 0);
    }

    #[test]
    fn test_find_nearest_entity_by_line_after() {
        let entity1 = ParsedEntity::new(
            "Entity1",
            crate::models::EntityKind::Class,
            "Entity1",
            None,
            None,
            "java",
            "/test.java",
            10,
            20,
            None,
            "test-repo",
        );
        let entity2 = ParsedEntity::new(
            "Entity2",
            crate::models::EntityKind::Method,
            "Entity2",
            None,
            None,
            "java",
            "/test.java",
            30,
            40,
            None,
            "test-repo",
        );

        let entities = vec![entity1, entity2];

        // Target line 50 is after both entities, should be assigned to closest (entity2)
        let idx = find_nearest_entity_by_line(&entities, 50);
        assert_eq!(idx, 1);
    }

    #[test]
    fn test_find_nearest_entity_by_line_before_all() {
        let entity1 = ParsedEntity::new(
            "Entity1",
            crate::models::EntityKind::Class,
            "Entity1",
            None,
            None,
            "java",
            "/test.java",
            10,
            20,
            None,
            "test-repo",
        );
        let entity2 = ParsedEntity::new(
            "Entity2",
            crate::models::EntityKind::Method,
            "Entity2",
            None,
            None,
            "java",
            "/test.java",
            30,
            40,
            None,
            "test-repo",
        );

        let entities = vec![entity1, entity2];

        // Target line 5 is before all entities, should be assigned to closest (entity1)
        let idx = find_nearest_entity_by_line(&entities, 5);
        assert_eq!(idx, 0);
    }

    #[test]
    fn test_find_nearest_entity_containment_preferred() {
        // Entity1 spans lines 10-20, Entity2 spans lines 30-40
        let entity1 = ParsedEntity::new(
            "Entity1",
            crate::models::EntityKind::Class,
            "Entity1",
            None,
            None,
            "java",
            "/test.java",
            10,
            20,
            None,
            "test-repo",
        );
        let entity2 = ParsedEntity::new(
            "Entity2",
            crate::models::EntityKind::Method,
            "Entity2",
            None,
            None,
            "java",
            "/test.java",
            30,
            40,
            None,
            "test-repo",
        );

        let entities = vec![entity1, entity2];

        // Target line 15 is CONTAINED in entity1 (10-20), should return index 0
        // even though it's closer to entity2's start (30)
        let idx = find_nearest_entity_by_line(&entities, 15);
        assert_eq!(
            idx, 0,
            "Line 15 should be assigned to entity1 (contained in 10-20)"
        );
    }

    #[test]
    fn test_find_nearest_entity_containment_chooses_smallest_range() {
        // Entity1 spans lines 5-50 (large range), Entity2 spans lines 20-30 (small range)
        let entity1 = ParsedEntity::new(
            "Entity1",
            crate::models::EntityKind::Class,
            "Entity1",
            None,
            None,
            "java",
            "/test.java",
            5,
            50,
            None,
            "test-repo",
        );
        let entity2 = ParsedEntity::new(
            "Entity2",
            crate::models::EntityKind::Method,
            "Entity2",
            None,
            None,
            "java",
            "/test.java",
            20,
            30,
            None,
            "test-repo",
        );

        let entities = vec![entity1, entity2];

        // Target line 25 is contained in both entities, should prefer entity2 (smallest range)
        let idx = find_nearest_entity_by_line(&entities, 25);
        assert_eq!(
            idx, 1,
            "Line 25 should be assigned to entity2 (smallest containing range)"
        );
    }
}
