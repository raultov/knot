//! Shared helpers used across the Rust language pipeline.

use crate::models::ParsedEntity;

/// Find the entity index nearest to the given line number.
///
/// Returns the entity whose `start_line` is the largest value still `<=` the
/// requested line. When no entity starts at or before `line` (typical for
/// file-level references like `use` statements that precede every entity),
/// the entity with the smallest `start_line` is returned instead so the
/// reference is attributed to the first entity in the file.
pub(crate) fn find_nearest_entity_by_line(entities: &[ParsedEntity], line: usize) -> usize {
    let mut nearest_idx = 0;
    let mut nearest_line = 0;
    let mut found_any = false;

    // Find the entity with the largest start_line that is still <= line.
    // The array is NOT assumed to be sorted by start_line.
    for (idx, entity) in entities.iter().enumerate() {
        if entity.start_line <= line && entity.start_line >= nearest_line {
            nearest_line = entity.start_line;
            nearest_idx = idx;
            found_any = true;
        }
    }

    if !found_any
        && let Some((idx, _)) = entities
            .iter()
            .enumerate()
            .min_by_key(|(_, e)| e.start_line)
    {
        return idx;
    }

    nearest_idx
}
