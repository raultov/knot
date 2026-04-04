//! Stage 3 — Prepare: UUID assignment and embedding text construction.
//!
//! Each [`ParsedEntity`] already carries a UUID generated at construction time
//! (see [`ParsedEntity::new`]). This stage's responsibility is to build the
//! `embed_text` field — the string that will be fed to the embedding model.
//!
//! # Embedding text format
//! ```text
//! [<KIND>] <name>
//! Signature: <signature>   ← omitted when None
//! <docstring>              ← omitted when None
//! File: <file_path>:<start_line>
//! ```
//!
//! Keeping the format consistent across runs is important: the same entity
//! should always produce the same embedding so vector updates are idempotent.

use crate::models::ParsedEntity;

/// Build the `embed_text` field for every entity in-place.
///
/// This function is intentionally synchronous and allocation-cheap; it is
/// called after Rayon parsing and before the async embedding stage.
pub fn prepare_entities(entities: &mut [ParsedEntity]) {
    for entity in entities.iter_mut() {
        entity.embed_text = build_embed_text(entity);
    }
}

/// Construct the embedding text for a single entity.
fn build_embed_text(entity: &ParsedEntity) -> String {
    let mut parts: Vec<String> = Vec::with_capacity(8);

    // Header: kind + name
    parts.push(format!("[{}] {}", entity.kind, entity.name));

    // Optional decorators/annotations (framework metadata)
    if !entity.decorators.is_empty() {
        let decorators_text = entity
            .decorators
            .iter()
            .filter(|d| !d.trim().is_empty())
            .cloned()
            .collect::<Vec<_>>()
            .join(", ");

        if !decorators_text.is_empty() {
            parts.push(format!("Decorators: {}", decorators_text));
        }
    }

    // Optional signature
    if let Some(sig) = &entity.signature {
        parts.push(format!("Signature: {sig}"));
    }

    // Optional docstring (preceding comments)
    if let Some(doc) = &entity.docstring
        && !doc.trim().is_empty()
    {
        parts.push(doc.trim().to_owned());
    }

    // Inline comments found within the entity body
    if !entity.inline_comments.is_empty() {
        let inline_text = entity
            .inline_comments
            .iter()
            .filter(|c| !c.trim().is_empty())
            .cloned()
            .collect::<Vec<_>>()
            .join("\n");

        if !inline_text.trim().is_empty() {
            parts.push(format!("Implementation context:\n{}", inline_text));
        }
    }

    // Source location — helps distinguish identically-named entities
    parts.push(format!("File: {}:{}", entity.file_path, entity.start_line));

    parts.join("\n")
}
