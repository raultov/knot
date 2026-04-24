use super::languages::{java, javascript, typescript};
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

    // Assign each orphaned intent to its nearest entity by line number
    if entities.is_empty() {
        // No entities exist: create synthetic <module> entity for all orphans
        let mut module_entity = ParsedEntity::new(
            "<module>",
            EntityKind::Function,
            "<module>",
            None,
            None,
            lang_name,
            file_path,
            1,
            None,
            repo_name,
        );
        module_entity.reference_intents = orphaned_intents;
        entities.push(module_entity);
    } else {
        // Assign each orphan individually to its nearest entity by line
        for intent in orphaned_intents {
            let orphan_line = match &intent {
                ReferenceIntent::Call { line, .. } => *line,
                ReferenceIntent::Extends { line, .. } => *line,
                ReferenceIntent::Implements { line, .. } => *line,
                ReferenceIntent::TypeReference { line, .. } => *line,
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
    }
}

/// Find the entity index nearest to the given line number.
/// Prefers the entity immediately preceding the orphan (same or earlier line).
/// Falls back to the closest entity after the orphan if nothing precedes it.
pub(crate) fn find_nearest_entity_by_line(entities: &[ParsedEntity], target_line: usize) -> usize {
    let mut best_idx = 0;
    let mut best_distance = usize::MAX;

    // First pass: find closest entity at or before target_line
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

    // If no entity precedes the orphan, fall back to closest entity overall (second pass)
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
            None,
            "test-repo",
        );

        let entities = vec![entity1, entity2];

        // Target line 5 is before all entities, should be assigned to closest (entity1)
        let idx = find_nearest_entity_by_line(&entities, 5);
        assert_eq!(idx, 0);
    }
}
