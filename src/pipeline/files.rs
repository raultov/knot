//! File classification and path utilities for the indexing pipeline.
//!
//! This module handles the logic for determining which files need to be indexed,
//! deleted, or remain unchanged based on the persistent index state.

use anyhow::Result;
use std::path::{Path, PathBuf};
use tracing::info;

use crate::pipeline::input::{
    BUILD_SYSTEM_NAMES, SUPPORTED_EXTENSIONS, is_build_system_json, is_config_extension,
};
use crate::pipeline::state::IndexState;

/// Type alias for file classification result: (unchanged, modified, added, deleted)
pub type FileClassification = (Vec<PathBuf>, Vec<PathBuf>, Vec<PathBuf>, Vec<String>);

/// Check if a file path refers to a supported source file.
/// When `include_config_files` is `false`, configuration files (`.yml`, `.yaml`,
/// `.json`, `.properties`, `.tpl`) are excluded except for build-system files
/// like `package.json` and `tsconfig.json`.
pub fn is_supported_file(path: &Path, include_config_files: bool) -> bool {
    // Match by extension first
    if let Some(ext) = path.extension().and_then(|e| e.to_str())
        && SUPPORTED_EXTENSIONS.contains(&ext)
    {
        // Config extensions are only supported when the flag is on
        if is_config_extension(ext) && !include_config_files {
            // .json with build-system filenames are always included
            if ext == "json"
                && let Some(name) = path.file_name().and_then(|n| n.to_str())
                && is_build_system_json(name)
            {
                return true;
            }
            return false;
        }
        return true;
    }
    // Match by filename for extensionless or non-standard files
    if let Some(name) = path.file_name().and_then(|n| n.to_str())
        && BUILD_SYSTEM_NAMES.contains(&name)
    {
        return true;
    }
    false
}

/// Classify files into unchanged, modified, added, and deleted categories.
///
/// `repo_root` is the canonicalized repo root used by `to_repo_relative` to
/// compute the keys persisted in `index_state` (see
/// `docs/specs/relative_file_paths.md` §3.2).
pub fn classify_files_for_indexing(
    all_files: &[PathBuf],
    index_state: &IndexState,
    clean_mode: bool,
    repo_root: &Path,
) -> Result<FileClassification> {
    if clean_mode {
        Ok((vec![], vec![], all_files.to_vec(), vec![]))
    } else {
        index_state.classify_files(all_files, repo_root)
    }
}

/// Calculate which files should be deleted from the databases before re-indexing.
///
/// Only files that already exist in the database need to be removed: deleted and
/// modified files. Added files have never been indexed, so including them here
/// would trigger pointless sequential delete operations.
pub fn calculate_files_to_delete(deleted: &[String], modified: &[PathBuf]) -> Vec<String> {
    let mut files_to_delete = deleted.to_vec();
    files_to_delete.extend(modified.iter().filter_map(|p| p.to_str().map(String::from)));
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
///
/// `repo_root` is the canonicalized repo root used to compute relative keys
/// (see `docs/specs/relative_file_paths.md` §3.2). `repo_path` is the raw
/// input path used to locate the state file on disk (its canonical form is
/// what `repo_root` should be).
#[expect(
    clippy::too_many_arguments,
    reason = "function is verbose but correct — extraction deferred"
)]
#[expect(
    clippy::cognitive_complexity,
    reason = "function is verbose but correct — extraction deferred"
)]
pub fn update_index_state(
    index_state: &mut IndexState,
    files_to_parse: &[PathBuf],
    deleted_files: &[String],
    repo_path: &str,
    repo_root: &Path,
    embedded_count: usize,
) -> Result<()> {
    info!("Updating index state...");
    index_state.update_files(files_to_parse, repo_root)?;
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

/// Convert an absolute file path into the canonical repo-relative form
/// (POSIX separators, no leading "./"). `repo_root` must be canonicalized
/// by the caller (once per run). Returns the absolute path unchanged and
/// logs a warning if `path` is not under `repo_root` (rule R5).
///
/// Rules enforced (see `docs/specs/relative_file_paths.md` §1):
/// - R1: relative to canonicalized `repo_root`
/// - R2: POSIX separators always (Windows `\` walks are normalized)
/// - R3: no leading `./`, no trailing `/`
/// - R4: a file at the repo root stores its bare filename
/// - R5: paths outside the repo root fall back to the absolute path with a warn
pub fn to_repo_relative(path: &Path, repo_root: &Path) -> String {
    match path.strip_prefix(repo_root) {
        Ok(rel) => {
            let mut s = rel.to_string_lossy().to_string();
            s = s.replace('\\', "/");
            s = s.trim_start_matches("./").to_string();
            s = s.trim_end_matches('/').to_string();
            if s.is_empty() {
                // Degenerate: path == repo_root itself. Not reachable for
                // discovered files (only files are parsed), but be defensive
                // — return the basename so we never produce an empty string.
                rel.file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| rel.to_string_lossy().into_owned())
            } else {
                s
            }
        }
        Err(_) => {
            tracing::warn!(
                "Path {} is not under repo root {}; storing absolute path (rule R5)",
                path.display(),
                repo_root.display()
            );
            path.to_string_lossy().replace('\\', "/").to_string()
        }
    }
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

        let to_delete = calculate_files_to_delete(&deleted, &modified);

        // Only deleted and modified files should be returned.
        // Added files are skipped — they have never been indexed.
        assert_eq!(to_delete.len(), 2);
        assert!(to_delete.contains(&"deleted.java".to_string()));
        assert!(to_delete.contains(&"modified.ts".to_string()));
        assert!(!to_delete.contains(&"added.tsx".to_string()));
        // Verify the `added` input does not affect the output.
        let _ = added;
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
        let temp_dir = tempfile::tempdir().unwrap();
        let repo_root = temp_dir.path();
        let all_files = vec![repo_root.join("src/main.rs"), repo_root.join("src/lib.rs")];
        let index_state = IndexState::default();

        let (unchanged, modified, added, deleted) =
            classify_files_for_indexing(&all_files, &index_state, true, repo_root).unwrap();

        assert_eq!(unchanged.len(), 0);
        assert_eq!(modified.len(), 0);
        assert_eq!(added.len(), 2);
        assert_eq!(deleted.len(), 0);
    }

    #[test]
    fn test_classify_files_for_indexing_no_clean() {
        let temp_dir = tempfile::tempdir().unwrap();
        let repo_root = temp_dir.path();
        let file1 = repo_root.join("file1.ts");
        let _all_files = [file1.clone()];

        let mut index_state = IndexState::default();
        // Simulate file1 being already indexed (relative key).
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
        let repo_root = temp_dir.path();

        // Create a fake file to hash
        let fake_file = temp_dir.path().join("new.ts");
        fs::write(&fake_file, "content").unwrap();

        let result = update_index_state(
            &mut index_state,
            &[fake_file],
            &deleted_files,
            temp_repo,
            repo_root,
            10,
        );

        assert!(result.is_ok());
        assert_eq!(index_state.file_hashes.len(), 1);
        assert!(!index_state.file_hashes.contains_key("old.java"));
        assert!(index_state.file_hashes.contains_key("new.ts"));
    }

    #[test]
    fn test_calculate_files_to_delete_edge_cases() {
        let to_delete = calculate_files_to_delete(&[], &[]);
        assert!(to_delete.is_empty());

        let to_delete = calculate_files_to_delete(&["a.ts".to_string()], &[]);
        assert_eq!(to_delete, vec!["a.ts".to_string()]);

        let to_delete = calculate_files_to_delete(&[], &[PathBuf::from("m.ts")]);
        assert_eq!(to_delete, vec!["m.ts".to_string()]);

        // Mixed input: both deleted and modified files are included.
        let to_delete =
            calculate_files_to_delete(&["del.ts".to_string()], &[PathBuf::from("m.ts")]);
        assert_eq!(to_delete.len(), 2);
        assert!(to_delete.contains(&"del.ts".to_string()));
        assert!(to_delete.contains(&"m.ts".to_string()));
    }

    #[test]
    fn test_classify_files_for_indexing_no_clean_unchanged() {
        let temp_dir = tempfile::tempdir().unwrap();
        let repo_root = temp_dir.path();
        let file1 = repo_root.join("file1.ts");
        fs::write(&file1, "content").unwrap();

        let all_files = [file1.clone()];
        let mut index_state = IndexState::default();

        let hash = IndexState::compute_file_hash(&file1).unwrap();
        // State keys are relative.
        index_state
            .file_hashes
            .insert(to_repo_relative(&file1, repo_root), hash);

        let (unchanged, modified, added, deleted) =
            classify_files_for_indexing(&all_files, &index_state, false, repo_root).unwrap();

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
        assert!(is_supported_file(Path::new("test.java"), true));
        assert!(is_supported_file(Path::new("test.ts"), true));
        assert!(is_supported_file(Path::new("test.tsx"), true));
        assert!(is_supported_file(Path::new("test.cts"), true));
        assert!(is_supported_file(Path::new("test.js"), true));
        assert!(is_supported_file(Path::new("test.mjs"), true));
        assert!(is_supported_file(Path::new("test.cjs"), true));
        assert!(is_supported_file(Path::new("test.jsx"), true));
        assert!(is_supported_file(Path::new("test.rs"), true));

        assert!(!is_supported_file(Path::new("test.txt"), true));
        assert!(!is_supported_file(Path::new("test"), true));
        assert!(!is_supported_file(Path::new("test.java.bak"), true));
    }

    // ---- §10.1 unit tests for to_repo_relative ----

    #[test]
    fn test_to_repo_relative_nested_file() {
        let root = Path::new("/repo");
        let path = Path::new("/repo/src/a/b.rs");
        assert_eq!(to_repo_relative(path, root), "src/a/b.rs");
    }

    #[test]
    fn test_to_repo_relative_root_file() {
        let root = Path::new("/repo");
        let path = Path::new("/repo/Cargo.toml");
        assert_eq!(to_repo_relative(path, root), "Cargo.toml");
    }

    #[test]
    fn test_to_repo_relative_trailing_slash_root() {
        let root = Path::new("/repo/");
        let path = Path::new("/repo/a/b/c.rs");
        // Path::strip_prefix handles trailing-slash roots transparently.
        assert_eq!(to_repo_relative(path, root), "a/b/c.rs");
    }

    #[test]
    fn test_to_repo_relative_backslash_normalization() {
        // R2: POSIX separators always. `\` from Windows walks is converted.
        let root = Path::new("C:\\repo");
        let path_str = "C:\\repo\\src\\lib.rs";
        let path = Path::new(path_str);
        let result = to_repo_relative(path, root);
        assert!(
            !result.contains('\\'),
            "backslash should be converted to forward slash, got {result}"
        );
        assert!(result.contains("src/lib.rs"), "got {result}");
    }

    #[test]
    fn test_to_repo_relative_outside_root_falls_back_absolute() {
        // R5: paths outside the root fall back to absolute with a warn.
        let root = Path::new("/repo");
        let path = Path::new("/elsewhere/x.rs");
        let result = to_repo_relative(path, root);
        assert!(
            result.contains("/elsewhere/x.rs"),
            "expected absolute passthrough, got {result}"
        );
    }

    #[test]
    fn test_to_repo_relative_no_leading_dot_slash() {
        // R3: no leading "./".
        let root = Path::new("/repo");
        let path = Path::new("/repo/./src/lib.rs");
        let result = to_repo_relative(path, root);
        assert!(
            !result.starts_with("./"),
            "result must not have leading ./: got {result}"
        );
        assert!(result.contains("src/lib.rs"), "got {result}");
    }

    #[test]
    fn test_to_repo_relative_no_trailing_slash() {
        // R3: no trailing "/".
        let root = Path::new("/repo");
        let path = Path::new("/repo/dir/");
        let result = to_repo_relative(path, root);
        assert!(
            !result.ends_with('/'),
            "result must not have trailing /: got {result}"
        );
    }
}
