//! Stage 1 — Input: source file discovery.
//!
//! Uses the `ignore` crate to walk a directory tree while respecting
//! `.gitignore`, `.ignore`, and other standard ignore files.

use anyhow::Result;
use ignore::WalkBuilder;
use std::path::PathBuf;
use tracing::{debug, info};

/// Source-code file extensions — always indexed.
const CORE_EXTENSIONS: &[&str] = &[
    "java", "ts", "tsx", "cts", "js", "mjs", "cjs", "jsx", "kt", "kts", "py", "pyi", "pyw", "html",
    "htm", "css", "scss", "sass", "rs", "groovy", "gradle", "c", "h", "cpp", "hpp", "cc", "cxx",
    "hh", "hxx", "md",
];

/// Configuration / Kubernetes / Helm file extensions — indexed only when
/// `--include-config-files` is set.
const CONFIG_EXTENSIONS: &[&str] = &["yml", "yaml", "json", "properties", "tpl"];

/// Union of [`CORE_EXTENSIONS`] and [`CONFIG_EXTENSIONS`] — the single source
/// of truth for all supported languages across the indexing pipeline.
pub const SUPPORTED_EXTENSIONS: &[&str] = &[
    // Core
    "java",
    "ts",
    "tsx",
    "cts",
    "js",
    "mjs",
    "cjs",
    "jsx",
    "kt",
    "kts",
    "py",
    "pyi",
    "pyw",
    "html",
    "htm",
    "css",
    "scss",
    "sass",
    "rs",
    "groovy",
    "gradle",
    "c",
    "h",
    "cpp",
    "hpp",
    "cc",
    "cxx",
    "hh",
    "hxx",
    "md",
    // Config
    "yml",
    "yaml",
    "json",
    "properties",
    "tpl",
];

/// Lock/config file names to exclude from indexing (prevent indexing large generated files).
const EXCLUDED_NAMES: &[&str] = &[
    "package-lock.json",
    "yarn.lock",
    "pnpm-lock.yaml",
    "Cargo.lock",
    "composer.lock",
    "Gemfile.lock",
    "poetry.lock",
    "Pipfile.lock",
];

/// Maximum file size in bytes for indexing (500 KB).
/// Files larger than this are skipped to prevent indexing data dumps or generated files.
const MAX_FILE_SIZE: u64 = 500 * 1024;

/// Filenames that are always discovered, even when config files are excluded.
/// These are build-system / project-identity files, not generic configuration.
pub(crate) const BUILD_SYSTEM_NAMES: &[&str] = &[
    "Jenkinsfile",
    "pom.xml",
    "Cargo.toml",
    "package.json",
    "tsconfig.json",
];

/// Check whether a filename (with `.json` extension) is a build-system file
/// that should always be indexed.
pub(crate) fn is_build_system_json(filename: &str) -> bool {
    filename == "package.json" || filename == "tsconfig.json"
}

/// Check whether an extension requires `--include-config-files` to be indexed.
pub(crate) fn is_config_extension(ext: &str) -> bool {
    CONFIG_EXTENSIONS.contains(&ext)
}

/// Recursively discover all supported source files under `repo_path`.
///
/// Respects `.gitignore` and other ignore files found during traversal.
/// When `include_config_files` is `false`, configuration and K8s/Helm files
/// (`.yml`, `.yaml`, `.json`, `.properties`, `.tpl`) are excluded, except for
/// build-system files like `package.json` and `tsconfig.json`.
///
/// Returns absolute [`PathBuf`]s sorted for deterministic processing order.
pub fn discover_files(repo_path: &str, include_config_files: bool) -> Result<Vec<PathBuf>> {
    use std::collections::HashSet;

    let mut files: Vec<PathBuf> = WalkBuilder::new(repo_path)
        .hidden(false)
        .git_ignore(true)
        .git_global(true)
        .git_exclude(true)
        .build()
        .filter_map(|entry| {
            let entry = entry.ok()?;
            let path = entry.path().to_path_buf();

            if !path.is_file() {
                return None;
            }

            // Exclude lock files and generated files by name
            if let Some(name) = path.file_name().and_then(|n| n.to_str())
                && EXCLUDED_NAMES.contains(&name)
            {
                debug!("Skipping excluded file: {}", name);
                return None;
            }

            // File size check — skip files > 500KB
            if let Ok(metadata) = std::fs::metadata(&path)
                && metadata.len() > MAX_FILE_SIZE
            {
                debug!(
                    "Skipping file over size limit ({} bytes): {}",
                    metadata.len(),
                    path.display()
                );
                return None;
            }

            // Match by extension first
            if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                if CORE_EXTENSIONS.contains(&ext) {
                    return Some(path);
                }

                if is_config_extension(ext) {
                    // .json files that are build-system files are always included
                    if ext == "json" && !include_config_files {
                        if let Some(name) = path.file_name().and_then(|n| n.to_str())
                            && is_build_system_json(name)
                        {
                            return Some(path);
                        }
                        // Skip other .json files when config is disabled
                        return None;
                    }

                    // Other config extensions: only include when flag is set
                    if include_config_files {
                        return Some(path);
                    }
                    return None;
                }
            }

            // Match by filename for extensionless or non-standard files
            if let Some(name) = path.file_name().and_then(|n| n.to_str())
                && BUILD_SYSTEM_NAMES.contains(&name)
            {
                return Some(path);
            }

            None
        })
        .collect();

    // Deduplicate (some files match both extension and filename)
    let mut seen = HashSet::new();
    files.retain(|p| seen.insert(p.clone()));

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
        fs::write(dir.path().join("readme.md"), "# Readme").unwrap();

        // Create unsupported files
        fs::write(dir.path().join("data.xml"), "<root/>").unwrap();

        // Create nested supported file
        let src_dir = dir.path().join("src");
        fs::create_dir(&src_dir).unwrap();
        fs::write(src_dir.join("utils.ts"), "export {}").unwrap();

        let files = discover_files(repo_path, true).unwrap();

        assert_eq!(files.len(), 12);

        // Verify extensions are in supported list
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

        let files = discover_files(repo_path, true).unwrap();

        // Should only find tracked.java
        assert_eq!(files.len(), 1);
        assert!(files[0].to_str().unwrap().contains("tracked.java"));
    }

    #[test]
    fn test_discover_files_empty() {
        let dir = tempdir().unwrap();
        let repo_path = dir.path().to_str().unwrap();

        let files = discover_files(repo_path, true).unwrap();
        assert!(files.is_empty());
    }

    #[test]
    fn test_discover_files_config_excluded_by_default() {
        let dir = tempdir().unwrap();
        let repo_path = dir.path().to_str().unwrap();

        // Create config extension files
        fs::write(dir.path().join("config.yaml"), "key: value").unwrap();
        fs::write(dir.path().join("settings.json"), r#"{"key":"value"}"#).unwrap();
        fs::write(dir.path().join("app.properties"), "key=value").unwrap();
        fs::write(dir.path().join("template.tpl"), "{{ .Values.x }}").unwrap();

        // Create core extension file
        fs::write(dir.path().join("Main.java"), "class Main {}").unwrap();

        // With include_config_files=false, only Main.java should be found
        let files = discover_files(repo_path, false).unwrap();
        assert_eq!(files.len(), 1, "Only core files should be discovered");
        assert!(files[0].to_str().unwrap().ends_with("Main.java"));

        // With include_config_files=true, all 5 should be found
        let files = discover_files(repo_path, true).unwrap();
        assert_eq!(
            files.len(),
            5,
            "All files should be discovered when flag is on"
        );
    }

    #[test]
    fn test_discover_files_build_system_json_always_included() {
        let dir = tempdir().unwrap();
        let repo_path = dir.path().to_str().unwrap();

        // Create build-system files
        fs::write(dir.path().join("package.json"), r#"{"name":"test"}"#).unwrap();
        fs::write(dir.path().join("tsconfig.json"), "{}").unwrap();

        // Create generic JSON
        fs::write(dir.path().join("random.json"), r#"{"a":1}"#).unwrap();

        // Create core file
        fs::write(dir.path().join("Main.java"), "class Main {}").unwrap();

        // With include_config_files=false, package.json and tsconfig.json still included
        let files = discover_files(repo_path, false).unwrap();
        let filenames: Vec<&str> = files
            .iter()
            .map(|p| p.file_name().unwrap().to_str().unwrap())
            .collect();
        assert!(
            filenames.contains(&"package.json"),
            "package.json should always be included"
        );
        assert!(
            filenames.contains(&"tsconfig.json"),
            "tsconfig.json should always be included"
        );
        assert!(
            !filenames.contains(&"random.json"),
            "generic JSON should be excluded"
        );
        assert!(
            filenames.contains(&"Main.java"),
            "core file should be included"
        );

        // With include_config_files=true, all 4 should be found
        let files = discover_files(repo_path, true).unwrap();
        assert_eq!(files.len(), 4);
    }
}
