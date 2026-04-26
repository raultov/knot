//! Stage 1 — Input: source file discovery.
//!
//! Uses the `ignore` crate to walk a directory tree while respecting
//! `.gitignore`, `.ignore`, and other standard ignore files.
//! Supported extensions: `.java`, `.ts`, `.tsx`, `.cts`, `.js`, `.mjs`, `.cjs`, `.jsx`, `.kt`, `.kts`, `.py`, `.pyi`, `.pyw`, `.html`, `.htm`, `.css`, `.scss`, `.sass`.

use anyhow::Result;
use ignore::WalkBuilder;
use std::path::PathBuf;
use tracing::info;

/// Supported source file extensions.
/// This is the single source of truth for all supported languages across the indexing pipeline.
pub const SUPPORTED_EXTENSIONS: &[&str] = &[
    "java", "ts", "tsx", "cts", "js", "mjs", "cjs", "jsx", "kt", "kts", "py", "pyi", "pyw", "html",
    "htm", "css", "scss", "sass", "rs",
];

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

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn test_discover_files_basic() {
        let dir = tempdir().unwrap();
        let repo_path = dir.path().to_str().unwrap();

        // Create supported files
        fs::write(dir.path().join("test.java"), "public class Test {}").unwrap();
        fs::write(dir.path().join("app.ts"), "export class App {}").unwrap();
        fs::write(
            dir.path().join("component.tsx"),
            "export const Comp = () => {}",
        )
        .unwrap();
        fs::write(dir.path().join("legacy.cts"), "module.exports = {}").unwrap();
        fs::write(dir.path().join("vanilla.js"), "console.log('test')").unwrap();
        fs::write(dir.path().join("module.mjs"), "export {}").unwrap();
        fs::write(dir.path().join("service.kt"), "class Service {}").unwrap();
        fs::write(dir.path().join("main.py"), "def main(): pass").unwrap();
        fs::write(dir.path().join("stub.pyi"), "def foo() -> None: ...").unwrap();
        fs::write(dir.path().join("gui.pyw"), "import tkinter").unwrap();

        // Create unsupported files
        fs::write(dir.path().join("readme.md"), "# Readme").unwrap();
        fs::write(dir.path().join("config.json"), "{}").unwrap();

        // Create nested supported file
        let src_dir = dir.path().join("src");
        fs::create_dir(&src_dir).unwrap();
        fs::write(src_dir.join("utils.ts"), "export {}").unwrap();

        let files = discover_files(repo_path).unwrap();

        assert_eq!(files.len(), 11);

        // Verify extensions
        for path in files {
            let ext = path.extension().unwrap().to_str().unwrap();
            assert!(SUPPORTED_EXTENSIONS.contains(&ext));
        }
    }

    #[test]
    fn test_discover_files_with_gitignore() {
        let dir = tempdir().unwrap();
        let repo_path = dir.path().to_str().unwrap();

        // Create .git directory to make WalkBuilder treat it as a repo
        fs::create_dir(dir.path().join(".git")).unwrap();

        // Create supported files
        fs::write(dir.path().join("tracked.java"), "public class Tracked {}").unwrap();
        fs::write(dir.path().join("ignored.java"), "public class Ignored {}").unwrap();

        // Create .gitignore
        fs::write(dir.path().join(".gitignore"), "ignored.java").unwrap();

        let files = discover_files(repo_path).unwrap();

        // Should only find tracked.java
        assert_eq!(files.len(), 1);
        assert!(files[0].to_str().unwrap().contains("tracked.java"));
    }

    #[test]
    fn test_discover_files_empty() {
        let dir = tempdir().unwrap();
        let repo_path = dir.path().to_str().unwrap();

        let files = discover_files(repo_path).unwrap();
        assert!(files.is_empty());
    }
}
