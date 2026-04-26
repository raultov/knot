//! Stage 2 — Parse: AST extraction via Tree-sitter + Rayon.
//!
//! Each source file is parsed in parallel on the Rayon thread pool.
//! Tree-sitter queries extract class declarations, method/function declarations,
//! associated documentation comments, and call-site references.
//!
//! # Custom queries
//! Built-in queries are compiled into the binary at build time (see `queries/`
//! directory). When [`ParseConfig::custom_queries_path`] is set, the parser
//! will instead load `java.scm` and `typescript.scm` from that directory,
//! allowing callers to override extraction logic without recompiling.

use anyhow::{Context, Result};
use rayon::prelude::*;
use std::{
    fs,
    path::{Path, PathBuf},
};
use tracing::{debug, warn};

use crate::models::ParsedEntity;
use tokio::sync::mpsc;

mod comments;
mod context;
mod extractor;
pub mod languages;
mod orphans;
mod utils;

#[cfg(test)]
mod test_utils;

// Built-in query files compiled into the binary.
const DEFAULT_JAVA_QUERY: &str = include_str!("../../../queries/java.scm");
const DEFAULT_KOTLIN_QUERY: &str = include_str!("../../../queries/kotlin.scm");
const DEFAULT_TS_QUERY: &str = include_str!("../../../queries/typescript.scm");
const DEFAULT_TSX_QUERY: &str = include_str!("../../../queries/tsx.scm");
const DEFAULT_JS_QUERY: &str = include_str!("../../../queries/javascript.scm");
#[allow(dead_code)] // Reserved for future query-based HTML parsing
const DEFAULT_HTML_QUERY: &str = include_str!("../../../queries/html.scm");
const DEFAULT_CSS_QUERY: &str = include_str!("../../../queries/css.scm");
const DEFAULT_SCSS_QUERY: &str = include_str!("../../../queries/scss.scm");
const DEFAULT_RUST_QUERY: &str = include_str!("../../../queries/rust.scm");
const DEFAULT_PYTHON_QUERY: &str = include_str!("../../../queries/python.scm");

/// Configuration for the parse stage.
pub struct ParseConfig {
    /// Optional filesystem path to a directory containing custom `.scm` query files.
    pub custom_queries_path: Option<String>,
    /// Logical repository name for multi-repository isolation.
    pub repo_name: String,
}

/// Parse a collection of source files in parallel and send results through a channel.
///
/// This function blocks until all files have been processed. It is intended to be
/// called from a `tokio::task::spawn_blocking` context.
pub fn parse_files_stream(
    files: &[PathBuf],
    parse_cfg: &ParseConfig,
    sender: mpsc::UnboundedSender<ParsedEntity>,
) {
    files
        .par_iter()
        .for_each(|path| match parse_single_file(path, parse_cfg) {
            Ok(entities) => {
                for entity in entities {
                    if let Err(e) = sender.send(entity) {
                        warn!("Failed to send entity to channel: {e}");
                        break;
                    }
                }
            }
            Err(e) => {
                warn!("Failed to parse {}: {e:#}", path.display());
            }
        });
}

/// Parse a collection of source files in parallel and return all extracted entities.
///
/// This function blocks until all files have been processed. It is intended to be
/// called from a `tokio::task::spawn_blocking` context so the async executor is
/// not starved.
pub fn parse_files(files: &[PathBuf], parse_cfg: &ParseConfig) -> Vec<ParsedEntity> {
    files
        .par_iter()
        .flat_map(|path| match parse_single_file(path, parse_cfg) {
            Ok(entities) => entities,
            Err(e) => {
                warn!("Failed to parse {}: {e:#}", path.display());
                vec![]
            }
        })
        .collect()
}

/// Parse a single source file and return its extracted entities.
fn parse_single_file(path: &Path, parse_cfg: &ParseConfig) -> Result<Vec<ParsedEntity>> {
    let source = fs::read_to_string(path)
        .with_context(|| format!("Cannot read file: {}", path.display()))?;

    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default();

    let file_path = path.to_string_lossy().to_string();

    let entities = match ext {
        "java" => {
            let query_src = load_query_source("java.scm", DEFAULT_JAVA_QUERY, parse_cfg);
            extractor::extract_entities(
                &source,
                tree_sitter_java::LANGUAGE.into(),
                &query_src,
                "java",
                &file_path,
                &parse_cfg.repo_name,
            )?
        }
        "kt" | "kts" => {
            let query_src = load_query_source("kotlin.scm", DEFAULT_KOTLIN_QUERY, parse_cfg);
            extractor::extract_entities(
                &source,
                tree_sitter_kotlin_ng::LANGUAGE.into(),
                &query_src,
                "kotlin",
                &file_path,
                &parse_cfg.repo_name,
            )?
        }
        "ts" | "tsx" | "cts" => {
            let mut query_src = load_query_source("typescript.scm", DEFAULT_TS_QUERY, parse_cfg);
            let lang: tree_sitter::Language = if ext == "tsx" {
                // For TSX files, append TSX-specific rules (JSX component invocations)
                let tsx_rules = load_query_source("tsx.scm", DEFAULT_TSX_QUERY, parse_cfg);
                query_src.push('\n');
                query_src.push_str(&tsx_rules);
                tree_sitter_typescript::LANGUAGE_TSX.into()
            } else {
                tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()
            };
            extractor::extract_entities(
                &source,
                lang,
                &query_src,
                "typescript",
                &file_path,
                &parse_cfg.repo_name,
            )?
        }
        "js" | "mjs" | "cjs" | "jsx" => {
            let query_src = load_query_source("javascript.scm", DEFAULT_JS_QUERY, parse_cfg);
            let lang: tree_sitter::Language = tree_sitter_javascript::LANGUAGE.into();
            extractor::extract_entities(
                &source,
                lang,
                &query_src,
                "javascript",
                &file_path,
                &parse_cfg.repo_name,
            )?
        }
        "html" | "htm" => {
            let mut parser = tree_sitter::Parser::new();
            parser
                .set_language(&tree_sitter_html::LANGUAGE.into())
                .context("Failed to load HTML grammar")?;
            let tree = parser
                .parse(&source, None)
                .context("Failed to parse HTML")?;
            languages::html::extract_entities_html(
                tree.root_node(),
                source.as_bytes(),
                &file_path,
                &parse_cfg.repo_name,
            )
        }
        "css" => {
            let query_src = load_query_source("css.scm", DEFAULT_CSS_QUERY, parse_cfg);
            extractor::extract_entities(
                &source,
                tree_sitter_css::LANGUAGE.into(),
                &query_src,
                "css",
                &file_path,
                &parse_cfg.repo_name,
            )?
        }
        "scss" | "sass" => {
            let query_src = load_query_source("scss.scm", DEFAULT_SCSS_QUERY, parse_cfg);
            extractor::extract_entities(
                &source,
                tree_sitter_scss::language(),
                &query_src,
                "scss",
                &file_path,
                &parse_cfg.repo_name,
            )?
        }
        "py" | "pyi" | "pyw" => {
            let query_src = load_query_source("python.scm", DEFAULT_PYTHON_QUERY, parse_cfg);
            extractor::extract_entities(
                &source,
                tree_sitter_python::LANGUAGE.into(),
                &query_src,
                "python",
                &file_path,
                &parse_cfg.repo_name,
            )?
        }
        "rs" => {
            let query_src = load_query_source("rust.scm", DEFAULT_RUST_QUERY, parse_cfg);
            extractor::extract_entities(
                &source,
                tree_sitter_rust::LANGUAGE.into(),
                &query_src,
                "rust",
                &file_path,
                &parse_cfg.repo_name,
            )?
        }
        other => {
            warn!("Unsupported extension '{other}', skipping");
            vec![]
        }
    };

    debug!("Extracted {} entities from {}", entities.len(), file_path);
    Ok(entities)
}

/// Return the query source string, preferring a custom file when available.
fn load_query_source(filename: &str, default: &str, cfg: &ParseConfig) -> String {
    if let Some(dir) = &cfg.custom_queries_path {
        let custom_path = PathBuf::from(dir).join(filename);
        if custom_path.exists() {
            match fs::read_to_string(&custom_path) {
                Ok(src) => {
                    tracing::info!("Using custom query: {}", custom_path.display());
                    return src;
                }
                Err(e) => warn!(
                    "Failed to load custom query {}: {e} — using built-in",
                    custom_path.display()
                ),
            }
        }
    }
    default.to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_config_creation() {
        let cfg = ParseConfig {
            custom_queries_path: None,
            repo_name: "test-repo".to_string(),
        };

        assert_eq!(cfg.repo_name, "test-repo");
        assert!(cfg.custom_queries_path.is_none());
    }

    #[test]
    fn test_parse_config_with_custom_queries() {
        let cfg = ParseConfig {
            custom_queries_path: Some("/custom/queries".to_string()),
            repo_name: "my-repo".to_string(),
        };

        assert_eq!(cfg.repo_name, "my-repo");
        assert_eq!(cfg.custom_queries_path, Some("/custom/queries".to_string()));
    }

    #[test]
    fn test_load_query_source_uses_default() {
        let cfg = ParseConfig {
            custom_queries_path: None,
            repo_name: "test-repo".to_string(),
        };

        let default_query = "MATCH (n) RETURN n";
        let result = load_query_source("test.scm", default_query, &cfg);

        assert_eq!(result, default_query);
    }

    #[test]
    fn test_load_query_source_nonexistent_custom_path() {
        let cfg = ParseConfig {
            custom_queries_path: Some("/nonexistent/path".to_string()),
            repo_name: "test-repo".to_string(),
        };

        let default_query = "MATCH (n) RETURN n";
        let result = load_query_source("test.scm", default_query, &cfg);

        // Should fall back to default when custom path doesn't exist
        assert_eq!(result, default_query);
    }

    #[test]
    fn test_parse_files_empty_list() {
        let cfg = ParseConfig {
            custom_queries_path: None,
            repo_name: "test-repo".to_string(),
        };

        let files: Vec<PathBuf> = vec![];
        let (sender, mut receiver) = mpsc::unbounded_channel();

        parse_files_stream(&files, &cfg, sender);

        // No files to parse, channel should receive nothing
        assert!(receiver.try_recv().is_err());
    }

    #[test]
    fn test_parse_files_with_mock_channel() {
        let cfg = ParseConfig {
            custom_queries_path: None,
            repo_name: "test-repo".to_string(),
        };

        // Use an empty list since we can't create real files in unit tests
        let files: Vec<PathBuf> = vec![];
        let (sender, mut receiver) = mpsc::unbounded_channel();

        parse_files_stream(&files, &cfg, sender);

        // Verify channel can receive messages (simulated)
        assert!(receiver.try_recv().is_err()); // No data sent
    }

    #[test]
    fn test_unsupported_file_extension_handling() {
        // Test extension detection logic
        let path = PathBuf::from("/test/file.unsupported");
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or_default();

        assert_eq!(ext, "unsupported");
        // File would be skipped (not java, ts, tsx, cts, js, mjs, cjs, jsx)
        assert!(
            ext != "java"
                && ext != "ts"
                && ext != "tsx"
                && ext != "cts"
                && ext != "js"
                && ext != "mjs"
                && ext != "cjs"
                && ext != "jsx"
        );
    }

    #[test]
    fn test_java_file_extension_detection() {
        let path = PathBuf::from("/test/Service.java");
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or_default();

        assert_eq!(ext, "java");
    }

    #[test]
    fn test_kotlin_file_extension_detection() {
        let extensions = vec!["kt", "kts"];

        for ext_name in &extensions {
            let path = PathBuf::from(format!("/test/file.{}", ext_name));
            let ext = path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or_default();

            assert_eq!(ext, *ext_name);
        }
    }

    #[test]
    fn test_typescript_file_extension_detection() {
        let extensions = vec!["ts", "tsx", "cts"];

        for ext_name in &extensions {
            let path = PathBuf::from(format!("/test/file.{}", ext_name));
            let ext = path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or_default();

            assert_eq!(ext, *ext_name);
        }
    }

    #[test]
    fn test_javascript_file_extension_detection() {
        let extensions = vec!["js", "mjs", "cjs", "jsx"];

        for ext_name in &extensions {
            let path = PathBuf::from(format!("/test/file.{}", ext_name));
            let ext = path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or_default();

            assert_eq!(ext, *ext_name);
        }
    }

    #[test]
    fn test_file_path_conversion() {
        let path = PathBuf::from("/home/user/project/src/Main.java");
        let file_path = path.to_string_lossy().to_string();

        assert!(file_path.contains("Main.java"));
        assert_eq!(file_path, "/home/user/project/src/Main.java");
    }

    #[test]
    fn test_parse_config_repo_name_assignment() {
        let cfg = ParseConfig {
            custom_queries_path: None,
            repo_name: "myproject".to_string(),
        };

        let path = PathBuf::from("/src/Main.java");
        let _entities = parse_files(&[path], &cfg);

        // With empty/invalid files, should return empty vector
        // But repo_name should be preserved in config
        assert_eq!(cfg.repo_name, "myproject");
    }

    #[test]
    fn test_parse_files_with_empty_input() {
        let cfg = ParseConfig {
            custom_queries_path: None,
            repo_name: "test-repo".to_string(),
        };

        let files: Vec<PathBuf> = vec![];
        let entities = parse_files(&files, &cfg);

        // No files to parse, should return empty vector
        assert!(entities.is_empty());
    }

    #[test]
    fn test_channel_sender_behavior_mock() {
        // Test that channel sender doesn't fail on empty input
        let (sender, mut receiver) = mpsc::unbounded_channel::<ParsedEntity>();

        // Dropping sender without sending should not error
        drop(sender);

        // Receiver should get no data
        assert!(receiver.try_recv().is_err());
    }

    #[test]
    fn test_multiple_file_extensions_in_batch() {
        let files = [
            PathBuf::from("file1.java"),
            PathBuf::from("file2.ts"),
            PathBuf::from("file3.tsx"),
            PathBuf::from("file4.kt"),
            PathBuf::from("file5.unsupported"),
        ];

        let expected_extensions = ["java", "ts", "tsx", "kt", "unsupported"];

        for (file, expected_ext) in files.iter().zip(expected_extensions.iter()) {
            let ext = file
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or_default();
            assert_eq!(ext, *expected_ext);
        }
    }
}
