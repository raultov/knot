//! File state tracking for incremental indexing.
//!
//! Manages a persistent index state file (.knot/index_state.json) that tracks
//! SHA-256 hashes of indexed source files to enable incremental re-indexing.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use tracing::info;

/// State directory name within the repository.
const STATE_DIR: &str = ".knot";

/// State file name containing file hashes.
const STATE_FILE: &str = "index_state.json";

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
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct IndexState {
    /// Map of file_path -> SHA-256 hash (hex string).
    pub file_hashes: HashMap<String, String>,
}

impl IndexState {
    /// Load the index state from disk, or return empty state if not found.
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

        info!(
            "Loaded index state with {} tracked files",
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

        let content = serde_json::to_string_pretty(self)
            .context("Failed to serialize index state to JSON")?;

        fs::write(&state_path, content)
            .with_context(|| format!("Failed to write state file: {}", state_path.display()))?;

        info!(
            "Saved index state with {} tracked files",
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
    /// Returns four vectors:
    /// - unchanged: files with identical hashes
    /// - modified: files with different hashes
    /// - added: new files not in old state
    /// - deleted: files in old state but not on disk
    pub fn classify_files(&self, current_files: &[PathBuf]) -> Result<FileClassification> {
        let mut unchanged = Vec::new();
        let mut modified = Vec::new();
        let mut added = Vec::new();

        // Build a set of current file paths for deletion detection
        let current_paths: std::collections::HashSet<String> = current_files
            .iter()
            .filter_map(|p| p.to_str().map(|s| s.to_string()))
            .collect();

        // Classify current files
        for file_path in current_files {
            let path_str = file_path
                .to_str()
                .context("File path contains invalid UTF-8")?;

            let current_hash = Self::compute_file_hash(file_path)?;

            match self.file_hashes.get(path_str) {
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

        // Detect deleted files (in old state but not in current)
        let deleted: Vec<String> = self
            .file_hashes
            .keys()
            .filter(|old_path| !current_paths.contains(*old_path))
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
    pub fn update_files(&mut self, files: &[PathBuf]) -> Result<()> {
        for file_path in files {
            let path_str = file_path
                .to_str()
                .context("File path contains invalid UTF-8")?
                .to_string();

            let hash = Self::compute_file_hash(file_path)?;
            self.file_hashes.insert(path_str, hash);
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
        let unchanged_file = dir.path().join("unchanged.ts");
        let modified_file = dir.path().join("modified.java");
        let added_file = dir.path().join("added.tsx");

        fs::write(&unchanged_file, "unchanged").unwrap();
        fs::write(&modified_file, "original content").unwrap();
        fs::write(&added_file, "new file").unwrap();

        let mut state = IndexState::default();
        state.file_hashes.insert(
            unchanged_file.to_str().unwrap().to_string(),
            IndexState::compute_file_hash(&unchanged_file).unwrap(),
        );
        state.file_hashes.insert(
            modified_file.to_str().unwrap().to_string(),
            "fake_old_hash".to_string(),
        );
        state
            .file_hashes
            .insert("deleted.java".to_string(), "deleted_hash".to_string());

        // Files currently on disk
        let current_files = vec![
            unchanged_file.clone(),
            modified_file.clone(),
            added_file.clone(),
        ];

        let (unchanged, modified, added, deleted) = state.classify_files(&current_files).unwrap();

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
        let file1 = dir.path().join("file1.ts");
        let file2 = dir.path().join("file2.java");
        fs::write(&file1, "content1").unwrap();
        fs::write(&file2, "content2").unwrap();

        let mut state = IndexState::default();

        // Update files
        state.update_files(&[file1.clone(), file2.clone()]).unwrap();
        assert_eq!(state.file_hashes.len(), 2);

        let path1 = file1.to_str().unwrap().to_string();
        let path2 = file2.to_str().unwrap().to_string();
        assert!(state.file_hashes.contains_key(&path1));
        assert!(state.file_hashes.contains_key(&path2));

        // Remove a file
        state.remove_files(std::slice::from_ref(&path1));
        assert_eq!(state.file_hashes.len(), 1);
        assert!(!state.file_hashes.contains_key(&path1));
        assert!(state.file_hashes.contains_key(&path2));
    }
}
