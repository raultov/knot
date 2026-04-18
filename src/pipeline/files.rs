//! File classification and path utilities for the indexing pipeline.
//!
//! This module handles the logic for determining which files need to be indexed,
//! deleted, or remain unchanged based on the persistent index state.

use anyhow::Result;
use std::path::{Path, PathBuf};
use tracing::info;

use crate::pipeline::state::IndexState;
use crate::pipeline::input::SUPPORTED_EXTENSIONS;

/// Type alias for file classification result: (unchanged, modified, added, deleted)
pub type FileClassification = (Vec<PathBuf>, Vec<PathBuf>, Vec<PathBuf>, Vec<String>);

/// Check if a file path refers to a supported source file.
/// Uses the centralized SUPPORTED_EXTENSIONS list to ensure consistency
/// across all pipeline stages (discovery, watching, parsing, etc.).
pub fn is_supported_file(path: &Path) -> bool {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default();
    SUPPORTED_EXTENSIONS.contains(&ext)
}

/// Classify files into unchanged, modified, added, and deleted categories.
pub fn classify_files_for_indexing(
    all_files: &[PathBuf],
    index_state: &IndexState,
    clean_mode: bool,
) -> anyhow::Result<FileClassification> {
    if clean_mode {
        Ok((vec![], vec![], all_files.to_vec(), vec![]))
    } else {
        index_state.classify_files(all_files)
    }
}

/// Calculate which files should be deleted from the databases before re-indexing.
pub fn calculate_files_to_delete(
    deleted: &[String],
    modified: &[PathBuf],
    added: &[PathBuf],
) -> Vec<String> {
    let mut files_to_delete = deleted.to_vec();
    files_to_delete.extend(
        modified
            .iter()
            .chain(added.iter())
            .filter_map(|p| p.to_str().map(String::from)),
    );
    files_to_delete
}

/// Calculate which files need to be parsed and indexed.
pub fn calculate_files_to_parse(added: Vec<PathBuf>, modified: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut files_to_parse = Vec::new();
    files_to_parse.extend(added);
    files_to_parse.extend(modified);
    files_to_parse
}

/// Update index state and log completion.
pub fn update_index_state(
    index_state: &mut IndexState,
    files_to_parse: &[PathBuf],
    deleted_files: &[String],
    repo_path: &str,
    embedded_count: usize,
) -> Result<()> {
    info!("Updating index state...");
    index_state.update_files(files_to_parse)?;
    index_state.remove_files(deleted_files);
    index_state.save(repo_path)?;

    if embedded_count > 0 {
        info!(
            "Incremental update complete! Processed {} entities from {} file(s).",
            embedded_count,
            files_to_parse.len()
        );
    } else if !deleted_files.is_empty() {
        info!(
            "Incremental update complete! Removed {} stale file(s).",
            deleted_files.len()
        );
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn test_calculate_files_to_delete() {
        let deleted = vec!["deleted.java".to_string()];
        let modified = vec![PathBuf::from("modified.ts")];
        let added = vec![PathBuf::from("added.tsx")];

        let to_delete = calculate_files_to_delete(&deleted, &modified, &added);

        assert_eq!(to_delete.len(), 3);
        assert!(to_delete.contains(&"deleted.java".to_string()));
        assert!(to_delete.contains(&"modified.ts".to_string()));
        assert!(to_delete.contains(&"added.tsx".to_string()));
    }

    #[test]
    fn test_calculate_files_to_parse() {
        let added = vec![PathBuf::from("added.java")];
        let modified = vec![PathBuf::from("modified.ts")];

        let to_parse = calculate_files_to_parse(added, modified);

        assert_eq!(to_parse.len(), 2);
        assert_eq!(to_parse[0], PathBuf::from("added.java"));
        assert_eq!(to_parse[1], PathBuf::from("modified.ts"));
    }

    #[test]
    fn test_classify_files_for_indexing_clean_mode() {
        let all_files = vec![PathBuf::from("src/main.rs"), PathBuf::from("src/lib.rs")];
        let index_state = IndexState::default();

        let (unchanged, modified, added, deleted) =
            classify_files_for_indexing(&all_files, &index_state, true).unwrap();

        assert_eq!(unchanged.len(), 0);
        assert_eq!(modified.len(), 0);
        assert_eq!(added.len(), 2);
        assert_eq!(deleted.len(), 0);
    }

    #[test]
    fn test_classify_files_for_indexing_no_clean() {
        let file1 = PathBuf::from("file1.ts");
        let _all_files = [file1.clone()];

        let mut index_state = IndexState::default();
        // Simulate file1 being already indexed
        index_state
            .file_hashes
            .insert("file1.ts".to_string(), "old_hash".to_string());

        // Note: we can't easily test the full classification here without mock files on disk
        // because classify_files calls compute_file_hash.
    }

    #[test]
    fn test_update_index_state_logic() {
        let mut index_state = IndexState::default();
        let deleted_files = vec!["old.java".to_string()];

        let temp_dir = tempfile::tempdir().unwrap();
        let temp_repo = temp_dir.path().to_str().unwrap();

        // Create a fake file to hash
        let fake_file = temp_dir.path().join("new.ts");
        fs::write(&fake_file, "content").unwrap();

        let result = update_index_state(
            &mut index_state,
            &[fake_file],
            &deleted_files,
            temp_repo,
            10,
        );

        assert!(result.is_ok());
        assert_eq!(index_state.file_hashes.len(), 1);
        assert!(!index_state.file_hashes.contains_key("old.java"));
    }

    #[test]
    fn test_calculate_files_to_delete_edge_cases() {
        let to_delete = calculate_files_to_delete(&[], &[], &[]);
        assert!(to_delete.is_empty());

        let to_delete = calculate_files_to_delete(&["a.ts".to_string()], &[], &[]);
        assert_eq!(to_delete, vec!["a.ts".to_string()]);

        let to_delete =
            calculate_files_to_delete(&[], &[PathBuf::from("m.ts")], &[PathBuf::from("add.ts")]);
        assert_eq!(to_delete.len(), 2);
        assert!(to_delete.contains(&"m.ts".to_string()));
        assert!(to_delete.contains(&"add.ts".to_string()));
    }

    #[test]
    fn test_classify_files_for_indexing_no_clean_unchanged() {
        let temp_dir = tempfile::tempdir().unwrap();
        let file1 = temp_dir.path().join("file1.ts");
        fs::write(&file1, "content").unwrap();

        let all_files = [file1.clone()];
        let mut index_state = IndexState::default();

        let hash = IndexState::compute_file_hash(&file1).unwrap();
        index_state
            .file_hashes
            .insert(file1.to_str().unwrap().to_string(), hash);

        let (unchanged, modified, added, deleted) =
            classify_files_for_indexing(&all_files, &index_state, false).unwrap();

        assert_eq!(unchanged.len(), 1);
        assert_eq!(modified.len(), 0);
        assert_eq!(added.len(), 0);
        assert_eq!(deleted.len(), 0);
    }

    #[test]
    fn test_calculate_files_to_parse_combinations() {
        let added = vec![PathBuf::from("a.ts")];
        let modified = vec![PathBuf::from("m.ts")];

        let to_parse = calculate_files_to_parse(added, modified);
        assert_eq!(to_parse.len(), 2);
        assert!(to_parse.contains(&PathBuf::from("a.ts")));
        assert!(to_parse.contains(&PathBuf::from("m.ts")));

        let to_parse_empty = calculate_files_to_parse(vec![], vec![]);
        assert!(to_parse_empty.is_empty());
    }

    #[test]
    fn test_is_supported_file() {
        assert!(is_supported_file(Path::new("test.java")));
        assert!(is_supported_file(Path::new("test.ts")));
        assert!(is_supported_file(Path::new("test.tsx")));
        assert!(is_supported_file(Path::new("test.cts")));
        assert!(is_supported_file(Path::new("test.js")));
        assert!(is_supported_file(Path::new("test.mjs")));
        assert!(is_supported_file(Path::new("test.cjs")));
        assert!(is_supported_file(Path::new("test.jsx")));

        assert!(!is_supported_file(Path::new("test.txt")));
        assert!(!is_supported_file(Path::new("test.rs")));
        assert!(!is_supported_file(Path::new("test")));
        assert!(!is_supported_file(Path::new("test.java.bak")));
    }
}
