//! File state tracking for incremental indexing.
//!
//! Manages a persistent index state file (.knot/index_state.json) that tracks
//! SHA-256 hashes of indexed source files to enable incremental re-indexing.
//!
//! File hashes are keyed by **canonical repo-relative paths** (see
//! `docs/specs/relative_file_paths.md` §3.2). Callers pass absolute
//! `PathBuf`s in (discovery is unchanged); the relative key is computed
//! against `repo_root` on every classification.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use tracing::info;

use crate::pipeline::files::to_repo_relative;

/// State directory name within the repository.
const STATE_DIR: &str = ".knot";

/// State file name containing file hashes.
const STATE_FILE: &str = "index_state.json";

/// Current on-disk version of the index state file.
///
/// Bumping this number forces a clean re-index because earlier versions
/// produced FQNs that are incompatible with the current schema (e.g. Rust
/// entities now carry crate-qualified FQNs introduced in v2, and
/// `__fixture::`/`__loose::` prefixed FQNs for non-src files in v3).
const CURRENT_STATE_VERSION: u32 = 4;

/// Returns the cache directory for fastembed models.
/// Prioritizes the `KNOT_FASTEMBED_CACHE_DIR` environment variable.
/// If not set, defaults to `<repo_path>/.knot/fastembed_cache/`.
pub fn fastembed_cache_dir(repo_path: &str) -> PathBuf {
    if let Ok(custom_dir) = std::env::var("KNOT_FASTEMBED_CACHE_DIR") {
        return PathBuf::from(custom_dir);
    }
    Path::new(repo_path).join(STATE_DIR).join("fastembed_cache")
}

/// Classification of a file based on state comparison.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileStatus {
    /// File exists in both old and new state with identical hash.
    Unchanged,
    /// File exists in both states but hash differs.
    Modified,
    /// File exists in new state but not in old state.
    Added,
    /// File exists in old state but not in new state.
    Deleted,
}

/// Type alias for file classification result: (unchanged, modified, added, deleted)
pub type FileClassification = (Vec<PathBuf>, Vec<PathBuf>, Vec<PathBuf>, Vec<String>);

/// Persistent index state tracking file hashes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexState {
    /// Schema version of the on-disk state file. Versions older than
    /// [`CURRENT_STATE_VERSION`] are treated as stale and force a full
    /// re-index on load.
    #[serde(default)]
    pub version: u32,
    /// Map of file_path -> SHA-256 hash (hex string).
    pub file_hashes: HashMap<String, String>,
}

impl Default for IndexState {
    fn default() -> Self {
        Self {
            version: CURRENT_STATE_VERSION,
            file_hashes: HashMap::new(),
        }
    }
}

impl IndexState {
    /// Load the index state from disk, or return empty state if not found.
    ///
    /// Returns an error if the on-disk state has an older version than
    /// [`CURRENT_STATE_VERSION`], because the FQN schema has changed and
    /// the old index is incompatible. The caller should print instructions
    /// and exit with code 1.
    pub fn load(repo_path: &str) -> Result<Self> {
        let state_path = Self::state_file_path(repo_path);

        if !state_path.exists() {
            info!("No existing index state found — will perform full indexing");
            return Ok(Self::default());
        }

        let content = fs::read_to_string(&state_path)
            .with_context(|| format!("Failed to read state file: {}", state_path.display()))?;

        let state: IndexState =
            serde_json::from_str(&content).context("Failed to deserialize index state JSON")?;

        if state.version < CURRENT_STATE_VERSION {
            anyhow::bail!(
                "Detected index_state v{}; current version is v{}. \
                 The on-disk index is incompatible.\n\
                 Run `knot-indexer --clean` to rebuild from scratch.",
                state.version,
                CURRENT_STATE_VERSION
            );
        }

        info!(
            "Loaded index state v{} with {} tracked files",
            state.version,
            state.file_hashes.len()
        );

        Ok(state)
    }

    /// Save the index state to disk.
    pub fn save(&self, repo_path: &str) -> Result<()> {
        let state_dir = Self::state_dir_path(repo_path);
        let state_path = Self::state_file_path(repo_path);

        // Ensure .knot directory exists
        fs::create_dir_all(&state_dir).with_context(|| {
            format!("Failed to create state directory: {}", state_dir.display())
        })?;

        let to_persist = Self {
            version: CURRENT_STATE_VERSION,
            file_hashes: self.file_hashes.clone(),
        };

        let content = serde_json::to_string_pretty(&to_persist)
            .context("Failed to serialize index state to JSON")?;

        fs::write(&state_path, content)
            .with_context(|| format!("Failed to write state file: {}", state_path.display()))?;

        info!(
            "Saved index state v{} with {} tracked files",
            CURRENT_STATE_VERSION,
            self.file_hashes.len()
        );

        Ok(())
    }

    /// Compute the SHA-256 hash of a file.
    pub fn compute_file_hash(file_path: &Path) -> Result<String> {
        let content = fs::read(file_path)
            .with_context(|| format!("Failed to read file for hashing: {}", file_path.display()))?;

        let mut hasher = Sha256::new();
        hasher.update(&content);
        let hash = hasher.finalize();

        Ok(format!("{:x}", hash))
    }

    /// Classify files based on state comparison.
    ///
    /// `current_files` are absolute `PathBuf`s from `discover_files`.
    /// `repo_root` is the canonicalized repo root used to compute relative
    /// keys (see `docs/specs/relative_file_paths.md` §3.2). The returned
    /// `unchanged`/`modified`/`added` vectors carry absolute paths (callers
    /// need them for I/O); the `deleted` list carries relative keys (to
    /// match what is persisted and to plug into `delete_by_file_paths`).
    ///
    /// Returns four vectors:
    /// - unchanged: files with identical hashes
    /// - modified: files with different hashes
    /// - added: new files not in old state
    /// - deleted: files in old state but not on disk (relative keys)
    pub fn classify_files(
        &self,
        current_files: &[PathBuf],
        repo_root: &Path,
    ) -> Result<FileClassification> {
        let mut unchanged = Vec::new();
        let mut modified = Vec::new();
        let mut added = Vec::new();

        // Build a set of current *relative* file paths for deletion detection
        let current_rel_paths: std::collections::HashSet<String> = current_files
            .iter()
            .map(|p| to_repo_relative(p, repo_root))
            .collect();

        // Classify current files
        for file_path in current_files {
            let key = to_repo_relative(file_path, repo_root);
            let current_hash = Self::compute_file_hash(file_path)?;

            match self.file_hashes.get(&key) {
                Some(old_hash) if old_hash == &current_hash => {
                    unchanged.push(file_path.clone());
                }
                Some(_old_hash) => {
                    modified.push(file_path.clone());
                }
                None => {
                    added.push(file_path.clone());
                }
            }
        }

        // Detect deleted files (in old state but not in current) — relative keys.
        let deleted: Vec<String> = self
            .file_hashes
            .keys()
            .filter(|old_path| !current_rel_paths.contains(*old_path))
            .cloned()
            .collect();

        info!(
            "File classification: {} unchanged, {} modified, {} added, {} deleted",
            unchanged.len(),
            modified.len(),
            added.len(),
            deleted.len()
        );

        Ok((unchanged, modified, added, deleted))
    }

    /// Update the state with new file hashes.
    ///
    /// `files` are absolute paths (from discovery / parsing). Keys stored
    /// in `file_hashes` are canonical relative paths against `repo_root`
    /// (see `docs/specs/relative_file_paths.md` §3.2).
    pub fn update_files(&mut self, files: &[PathBuf], repo_root: &Path) -> Result<()> {
        for file_path in files {
            let key = to_repo_relative(file_path, repo_root);
            let hash = Self::compute_file_hash(file_path)?;
            self.file_hashes.insert(key, hash);
        }

        Ok(())
    }

    /// Remove files from the state.
    pub fn remove_files(&mut self, file_paths: &[String]) {
        for path in file_paths {
            self.file_hashes.remove(path);
        }
    }

    /// Get the path to the .knot directory.
    fn state_dir_path(repo_path: &str) -> PathBuf {
        Path::new(repo_path).join(STATE_DIR)
    }

    /// Get the path to the index_state.json file.
    fn state_file_path(repo_path: &str) -> PathBuf {
        Self::state_dir_path(repo_path).join(STATE_FILE)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn test_compute_file_hash() {
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("test.txt");
        fs::write(&file_path, "test content").unwrap();

        let hash = IndexState::compute_file_hash(&file_path).unwrap();
        // SHA-256 for "test content" is 6ae8a75555209fd6c44157c0aed8016e763ff435a19cf186f76863140143ff72
        assert_eq!(
            hash,
            "6ae8a75555209fd6c44157c0aed8016e763ff435a19cf186f76863140143ff72"
        );

        // Hash should change if content changes
        fs::write(&file_path, "updated content").unwrap();
        let updated_hash = IndexState::compute_file_hash(&file_path).unwrap();
        assert_ne!(hash, updated_hash);
    }

    #[test]
    fn test_state_save_and_load() {
        let dir = tempdir().unwrap();
        let repo_path = dir.path().to_str().unwrap();

        let mut state = IndexState::default();
        state
            .file_hashes
            .insert("file1.ts".to_string(), "hash1".to_string());
        state
            .file_hashes
            .insert("file2.java".to_string(), "hash2".to_string());

        // Save state
        state.save(repo_path).unwrap();

        // Verify file exists
        let state_file = dir.path().join(".knot").join("index_state.json");
        assert!(state_file.exists());

        // Load state
        let loaded_state = IndexState::load(repo_path).unwrap();

        // Check if loaded state matches original
        assert_eq!(loaded_state.file_hashes.len(), 2);
        assert_eq!(loaded_state.file_hashes.get("file1.ts").unwrap(), "hash1");
        assert_eq!(loaded_state.file_hashes.get("file2.java").unwrap(), "hash2");
    }

    #[test]
    fn test_classify_files() {
        let dir = tempdir().unwrap();
        let repo_root = dir.path();
        let unchanged_file = repo_root.join("unchanged.ts");
        let modified_file = repo_root.join("modified.java");
        let added_file = repo_root.join("added.tsx");

        fs::write(&unchanged_file, "unchanged").unwrap();
        fs::write(&modified_file, "original content").unwrap();
        fs::write(&added_file, "new file").unwrap();

        let mut state = IndexState::default();
        // Keys are relative to repo_root (canonicalized).
        state.file_hashes.insert(
            to_repo_relative(&unchanged_file, repo_root),
            IndexState::compute_file_hash(&unchanged_file).unwrap(),
        );
        state.file_hashes.insert(
            to_repo_relative(&modified_file, repo_root),
            "fake_old_hash".to_string(),
        );
        state
            .file_hashes
            .insert("deleted.java".to_string(), "deleted_hash".to_string());

        // Files currently on disk (absolute paths from discover_files).
        let current_files = vec![
            unchanged_file.clone(),
            modified_file.clone(),
            added_file.clone(),
        ];

        let (unchanged, modified, added, deleted) =
            state.classify_files(&current_files, repo_root).unwrap();

        assert_eq!(unchanged.len(), 1);
        assert_eq!(unchanged[0], unchanged_file);

        assert_eq!(modified.len(), 1);
        assert_eq!(modified[0], modified_file);

        assert_eq!(added.len(), 1);
        assert_eq!(added[0], added_file);

        assert_eq!(deleted.len(), 1);
        assert_eq!(deleted[0], "deleted.java");
    }

    #[test]
    fn test_update_and_remove_files() {
        let dir = tempdir().unwrap();
        let repo_root = dir.path();
        let file1 = repo_root.join("file1.ts");
        let file2 = repo_root.join("file2.java");
        fs::write(&file1, "content1").unwrap();
        fs::write(&file2, "content2").unwrap();

        let mut state = IndexState::default();

        // Update files — keys are relative paths.
        state
            .update_files(&[file1.clone(), file2.clone()], repo_root)
            .unwrap();
        assert_eq!(state.file_hashes.len(), 2);

        let key1 = to_repo_relative(&file1, repo_root);
        let key2 = to_repo_relative(&file2, repo_root);
        assert!(state.file_hashes.contains_key(&key1));
        assert!(state.file_hashes.contains_key(&key2));

        // Remove a file by relative key.
        state.remove_files(std::slice::from_ref(&key1));
        assert_eq!(state.file_hashes.len(), 1);
        assert!(!state.file_hashes.contains_key(&key1));
        assert!(state.file_hashes.contains_key(&key2));
    }

    #[test]
    fn test_default_state_uses_current_version() {
        let state = IndexState::default();
        assert_eq!(state.version, CURRENT_STATE_VERSION);
        assert!(state.file_hashes.is_empty());
    }

    #[test]
    fn test_save_writes_current_version() {
        let dir = tempdir().unwrap();
        let repo_path = dir.path().to_str().unwrap();

        let mut state = IndexState {
            version: 0,
            file_hashes: HashMap::new(),
        };
        state
            .file_hashes
            .insert("file1.rs".to_string(), "hash1".to_string());

        state.save(repo_path).unwrap();

        let state_file = dir.path().join(".knot").join("index_state.json");
        let raw = fs::read_to_string(&state_file).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(
            parsed.get("version").and_then(|v| v.as_u64()),
            Some(CURRENT_STATE_VERSION as u64)
        );
    }

    #[test]
    fn test_load_older_version_returns_error_with_instructions() {
        let dir = tempdir().unwrap();
        let repo_path = dir.path().to_str().unwrap();

        let state_dir = dir.path().join(".knot");
        fs::create_dir_all(&state_dir).unwrap();
        let state_file = state_dir.join("index_state.json");

        let raw = r#"{
            "version": 1,
            "file_hashes": {
                "/tmp/stale.rs": "abc123"
            }
        }"#;
        fs::write(&state_file, raw).unwrap();

        let err = IndexState::load(repo_path).unwrap_err();
        let msg = format!("{}", err);
        assert!(
            msg.contains("incompatible"),
            "error should mention incompatibility: {msg}"
        );
        assert!(
            msg.contains("--clean"),
            "error should suggest --clean flag: {msg}"
        );
    }

    #[test]
    fn test_load_missing_version_treated_as_incompatible() {
        let dir = tempdir().unwrap();
        let repo_path = dir.path().to_str().unwrap();

        let state_dir = dir.path().join(".knot");
        fs::create_dir_all(&state_dir).unwrap();
        let state_file = state_dir.join("index_state.json");

        let raw = r#"{
            "file_hashes": {
                "/tmp/legacy.rs": "deadbeef"
            }
        }"#;
        fs::write(&state_file, raw).unwrap();

        let err = IndexState::load(repo_path).unwrap_err();
        let msg = format!("{}", err);
        assert!(
            msg.contains("incompatible"),
            "missing version should be treated as incompatible: {msg}"
        );
    }

    #[test]
    fn test_load_current_version_preserves_state() {
        let dir = tempdir().unwrap();
        let repo_path = dir.path().to_str().unwrap();

        let mut state = IndexState::default();
        state
            .file_hashes
            .insert("file1.rs".to_string(), "hash1".to_string());
        state.save(repo_path).unwrap();

        let loaded = IndexState::load(repo_path).unwrap();
        assert_eq!(loaded.version, CURRENT_STATE_VERSION);
        assert_eq!(loaded.file_hashes.len(), 1);
        assert_eq!(loaded.file_hashes.get("file1.rs").unwrap(), "hash1");
    }

    // ---- §10.1 unit tests for relative file path keys ----

    #[test]
    fn test_classify_files_uses_relative_keys() {
        // State populated with RELATIVE keys must classify a re-discovered
        // absolute path as `unchanged`.
        let dir = tempdir().unwrap();
        let repo_root = dir.path();
        let file = repo_root.join("src/lib.rs");
        fs::create_dir_all(repo_root.join("src")).unwrap();
        fs::write(&file, "fn foo() {}").unwrap();

        let mut state = IndexState::default();
        let key = to_repo_relative(&file, repo_root);
        state
            .file_hashes
            .insert(key.clone(), IndexState::compute_file_hash(&file).unwrap());

        let current = vec![file.clone()];
        let (unchanged, modified, added, deleted) =
            state.classify_files(&current, repo_root).unwrap();

        assert_eq!(unchanged, vec![file]);
        assert!(modified.is_empty());
        assert!(added.is_empty());
        assert!(deleted.is_empty());
    }

    #[test]
    fn test_update_files_stores_relative_keys() {
        let dir = tempdir().unwrap();
        let repo_root = dir.path();
        let f = repo_root.join("a/b.rs");
        fs::create_dir_all(repo_root.join("a")).unwrap();
        fs::write(&f, "fn x() {}").unwrap();

        let mut state = IndexState::default();
        state
            .update_files(std::slice::from_ref(&f), repo_root)
            .unwrap();

        let key = to_repo_relative(&f, repo_root);
        assert_eq!(key, "a/b.rs");
        assert!(state.file_hashes.contains_key(&key));
        assert!(
            !state.file_hashes.contains_key(f.to_str().unwrap()),
            "absolute path must NOT be used as a key"
        );
    }

    #[test]
    fn test_deleted_files_reported_relative() {
        let dir = tempdir().unwrap();
        let repo_root = dir.path();
        fs::create_dir_all(repo_root.join("src")).unwrap();
        let present = repo_root.join("src/present.rs");
        fs::write(&present, "x").unwrap();

        let mut state = IndexState::default();
        state
            .file_hashes
            .insert("src/present.rs".to_string(), "h".to_string());
        state
            .file_hashes
            .insert("src/gone.rs".to_string(), "h2".to_string());

        let current = vec![present.clone()];
        let (_, _, _, deleted) = state.classify_files(&current, repo_root).unwrap();

        assert_eq!(deleted, vec!["src/gone.rs".to_string()]);
    }

    #[test]
    fn test_load_rejects_v3_state() {
        // Mirror of the existing v1/v2 rejection tests: a state file
        // claiming version 3 (one below CURRENT_STATE_VERSION) must be
        // rejected so the caller rebuilds from scratch.
        let dir = tempdir().unwrap();
        let repo_path = dir.path().to_str().unwrap();

        let state_dir = dir.path().join(".knot");
        fs::create_dir_all(&state_dir).unwrap();
        let state_file = state_dir.join("index_state.json");

        let raw = r#"{
            "version": 3,
            "file_hashes": {
                "/tmp/stale.rs": "abc123"
            }
        }"#;
        fs::write(&state_file, raw).unwrap();

        let err = IndexState::load(repo_path).unwrap_err();
        let msg = format!("{}", err);
        assert!(
            msg.contains("incompatible"),
            "error should mention incompatibility: {msg}"
        );
        assert!(
            msg.contains("--clean"),
            "error should suggest --clean flag: {msg}"
        );
    }
}
