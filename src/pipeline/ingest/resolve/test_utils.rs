//! Shared test helpers for the `resolve` pipeline.
//!
//! Used by the `#[cfg(test)]` modules in `mod.rs`, `calls.rs`, and
//! `non_calls.rs` to avoid duplicating mock-factory functions across
//! every file.

use crate::models::{EntityKind, ResolutionEntity};
use uuid::Uuid;

pub(crate) fn mock_resolution_entity_with_kind(
    name: &str,
    fqn: &str,
    enclosing: Option<&str>,
    file_path: &str,
    kind: EntityKind,
) -> ResolutionEntity {
    ResolutionEntity {
        uuid: Uuid::new_v4(),
        name: name.to_string(),
        fqn: fqn.to_string(),
        file_path: file_path.to_string(),
        kind,
        enclosing_class: enclosing.map(|s| s.to_string()),
        enclosing_class_fqn: None,
        signature: None,
        reference_intents: Vec::new(),
        relationships: Vec::new(),
        alias_module_path: None,
        original_export_name: None,
        default_export: None,
        is_test_context: false,
    }
}

pub(crate) fn mock_resolution_entity_at(
    name: &str,
    fqn: &str,
    enclosing: Option<&str>,
    file_path: &str,
) -> ResolutionEntity {
    mock_resolution_entity_with_kind(name, fqn, enclosing, file_path, EntityKind::Method)
}

pub(crate) fn mock_resolution_entity(
    name: &str,
    fqn: &str,
    enclosing: Option<&str>,
) -> ResolutionEntity {
    mock_resolution_entity_at(name, fqn, enclosing, "test/file.java")
}
