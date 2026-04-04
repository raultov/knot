//! Stage 1 — Input: source file discovery.
//!
//! Uses the `ignore` crate to walk a directory tree while respecting
//! `.gitignore`, `.ignore`, and other standard ignore files.
//! Only files with the extensions `.java`, `.ts`, and `.tsx` are retained.

use anyhow::Result;
use ignore::WalkBuilder;
use std::path::PathBuf;
use tracing::info;

/// Supported source file extensions.
const SUPPORTED_EXTENSIONS: &[&str] = &["java", "ts", "tsx"];

/// Recursively discover all supported source files under `repo_path`.
///
/// Respects `.gitignore` and other ignore files found during traversal.
/// Returns absolute [`PathBuf`]s sorted for deterministic processing order.
pub fn discover_files(repo_path: &str) -> Result<Vec<PathBuf>> {
    let mut files: Vec<PathBuf> = WalkBuilder::new(repo_path)
        .hidden(false) // include dot-files (e.g. .github actions) but not dirs
        .git_ignore(true) // respect .gitignore
        .git_global(true) // respect global gitignore
        .git_exclude(true) // respect .git/info/exclude
        .build()
        .filter_map(|entry| {
            let entry = entry.ok()?;
            let path = entry.path().to_path_buf();

            // Skip directories and unsupported extensions.
            if !path.is_file() {
                return None;
            }

            let ext = path.extension()?.to_str()?;
            if SUPPORTED_EXTENSIONS.contains(&ext) {
                Some(path)
            } else {
                None
            }
        })
        .collect();

    files.sort();

    info!(
        "Discovered {} source files under '{}'",
        files.len(),
        repo_path
    );

    Ok(files)
}
