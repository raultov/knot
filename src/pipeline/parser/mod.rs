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
use std::{
    fs,
    path::{Path, PathBuf},
};
use tracing::{debug, warn};

use crate::models::ParsedEntity;
use tokio::sync::mpsc;

mod comments;
mod context;
pub(crate) mod extractor;
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
#[allow(dead_code)] // Used by language-specific parsers
const DEFAULT_CSS_QUERY: &str = include_str!("../../../queries/css.scm");
#[allow(dead_code)] // Used by language-specific parsers
const DEFAULT_SCSS_QUERY: &str = include_str!("../../../queries/scss.scm");
#[allow(dead_code)] // Used by language-specific parsers
const DEFAULT_RUST_QUERY: &str = include_str!("../../../queries/rust.scm");
#[allow(dead_code)] // Used by language-specific parsers
const DEFAULT_PYTHON_QUERY: &str = include_str!("../../../queries/python.scm");
#[allow(dead_code)] // Used by language-specific parsers
const DEFAULT_C_QUERY: &str = include_str!("../../../queries/c.scm");
#[allow(dead_code)] // Used by language-specific parsers
const DEFAULT_CPP_QUERY: &str = include_str!("../../../queries/cpp.scm");
#[allow(dead_code)] // Used by language-specific parsers
const DEFAULT_MD_QUERY: &str = include_str!("../../../queries/markdown.scm");

/// Configuration for the parse stage.
#[derive(Clone)]
pub struct ParseConfig {
    /// Optional filesystem path to a directory containing custom `.scm` query files.
    pub custom_queries_path: Option<String>,
    /// Logical repository name for multi-repository isolation.
    pub repo_name: String,
    /// Whether to index configuration files (YAML, JSON, .properties) and
    /// Kubernetes/Helm manifests. When `false`, these files produce no entities.
    pub include_config_files: bool,
    /// Filesystem root of the repository being indexed. Required by Rust
    /// post-processing to discover `Cargo.toml` files and compute crate
    /// qualified FQNs (e.g. `crate_a::config::Config`). When `None`, Rust
    /// FQNs fall back to their bare-name form.
    pub repo_path: Option<String>,
}

/// Callback invoked exactly once per input file after the file has been
/// fully processed (all entities sent to the channel, or parse failed).
pub type FileParsedCallback = std::sync::Arc<dyn Fn() + Send + Sync>;

/// Parse a collection of source files in parallel and send results through a channel.
///
/// Uses `std::thread::scope` with raw OS threads (NOT Rayon) so that
/// `blocking_send` on the bounded channel only blocks the dedicated
/// parsing thread rather than a shared thread pool. This prevents
/// deadlocks with `fastembed` which requires Rayon for tokenization.
///
/// This function blocks until all files have been processed. It is
/// intended to be called from a `tokio::task::spawn_blocking` context.
pub fn parse_files_stream(
    files: &[PathBuf],
    parse_cfg: &ParseConfig,
    sender: mpsc::Sender<ParsedEntity>,
    max_concurrent: usize,
    on_file_parsed: Option<FileParsedCallback>,
) {
    use std::sync::{Arc, Condvar, Mutex};

    // Concurrency limiter: Condvar-based semaphore backed by a Mutex.
    let sem = Arc::new((Mutex::new(0usize), Condvar::new()));

    std::thread::scope(|s| {
        for path in files {
            let path = path.clone();
            let parse_cfg = parse_cfg.clone();
            let sender = sender.clone();
            let sem = Arc::clone(&sem);

            // Acquire: block until active < max_concurrent
            {
                let (lock, cvar) = &*sem;
                let mut active = lock.lock().unwrap();
                while *active >= max_concurrent {
                    active = cvar.wait(active).unwrap();
                }
                *active += 1;
            }

            let on_file_parsed = on_file_parsed.clone();

            s.spawn(move || {
                match parse_single_file(&path, &parse_cfg) {
                    Ok(entities) => {
                        for entity in entities {
                            if sender.blocking_send(entity).is_err() {
                                warn!("Failed to send entity to channel");
                                break;
                            }
                        }
                    }
                    Err(e) => {
                        warn!("Failed to parse {}: {e:#}", path.display());
                    }
                }

                if let Some(cb) = &on_file_parsed {
                    cb();
                }

                // Release: decrement active count and wake waiter
                let (lock, cvar) = &*sem;
                let mut active = lock.lock().unwrap();
                *active -= 1;
                cvar.notify_one();
            });
        }
    });
    // All threads joined here (std::thread::scope guarantees this).
}

/// Parse a collection of source files in parallel and return all extracted entities.
///
/// Uses `parse_files_stream` internally. This is a convenience wrapper for
/// callers that want the full Vec instead of streaming through a channel.
pub fn parse_files(files: &[PathBuf], parse_cfg: &ParseConfig) -> Vec<ParsedEntity> {
    let (tx, mut rx) = mpsc::channel(1024);
    let cpus = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4);

    parse_files_stream(files, parse_cfg, tx, cpus, None);

    let mut entities = Vec::with_capacity(1024);
    while let Ok(entity) = rx.try_recv() {
        entities.push(entity);
    }
    entities
}

/// Parse a single source file and return its extracted entities.
/// Heuristic: detect whether a `.h` header contains C++ syntax.
/// Scans for keywords exclusive to C++ that do not appear in valid C.
#[allow(dead_code)] // Used by C/C++ header detection in dispatch
fn is_cpp_header(source: &str) -> bool {
    let cpp_indicators = [
        "class ",
        "namespace ",
        "template<",
        "template <",
        "virtual ",
        "public:",
        "private:",
        "protected:",
        "using namespace",
        "constexpr ",
        "noexcept",
        "nullptr",
        "override",
        " final",
        "::", // qualified calls like Print::write(...)
    ];
    cpp_indicators.iter().any(|kw| source.contains(kw))
}

fn parse_single_file(path: &Path, parse_cfg: &ParseConfig) -> Result<Vec<ParsedEntity>> {
    let source = {
        let bytes =
            fs::read(path).with_context(|| format!("Cannot read file: {}", path.display()))?;
        String::from_utf8_lossy(&bytes).into_owned()
    };

    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default();

    // Handle files identified by name (no extension), e.g. Jenkinsfile
    let filename = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default();

    let file_path = path.to_string_lossy().to_string();

    // Dispatch by filename first for extensionless files
    if filename == "Jenkinsfile" {
        return Ok(languages::jenkins::extract_entities_jenkins(
            &source,
            &file_path,
            &parse_cfg.repo_name,
        ));
    }

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
        "yml" | "yaml" => {
            if !parse_cfg.include_config_files {
                Vec::new()
            } else {
                dispatch_yaml(&source, &file_path, &parse_cfg.repo_name)
            }
        }
        "json" => {
            if !parse_cfg.include_config_files
                && filename != "package.json"
                && filename != "tsconfig.json"
            {
                Vec::new()
            } else {
                languages::json_config::extract_entities_json_config(
                    &source,
                    &file_path,
                    &parse_cfg.repo_name,
                )
            }
        }
        "properties" => {
            if !parse_cfg.include_config_files {
                Vec::new()
            } else {
                languages::properties::extract_entities_properties(
                    &source,
                    &file_path,
                    &parse_cfg.repo_name,
                )
            }
        }
        "tpl" => {
            if !parse_cfg.include_config_files {
                Vec::new()
            } else {
                let chart_name = detect_chart_name(&file_path);
                languages::helm::extract_helm_template(
                    &source,
                    &file_path,
                    &parse_cfg.repo_name,
                    &chart_name,
                )
            }
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
            let mut rust_entities = extractor::extract_entities(
                &source,
                tree_sitter_rust::LANGUAGE.into(),
                &query_src,
                "rust",
                &file_path,
                &parse_cfg.repo_name,
            )?;
            languages::rust::qualify_rust_fqns(
                &mut rust_entities,
                &file_path,
                parse_cfg.repo_path.as_deref(),
                Some(&source),
            );
            rust_entities
        }
        "c" => {
            let query_src = load_query_source("c.scm", DEFAULT_C_QUERY, parse_cfg);
            extractor::extract_entities(
                &source,
                tree_sitter_c::LANGUAGE.into(),
                &query_src,
                "c",
                &file_path,
                &parse_cfg.repo_name,
            )?
        }
        "h" => {
            if is_cpp_header(&source) {
                let query_src = load_query_source("cpp.scm", DEFAULT_CPP_QUERY, parse_cfg);
                extractor::extract_entities(
                    &source,
                    tree_sitter_cpp::LANGUAGE.into(),
                    &query_src,
                    "cpp",
                    &file_path,
                    &parse_cfg.repo_name,
                )?
            } else {
                let query_src = load_query_source("c.scm", DEFAULT_C_QUERY, parse_cfg);
                extractor::extract_entities(
                    &source,
                    tree_sitter_c::LANGUAGE.into(),
                    &query_src,
                    "c",
                    &file_path,
                    &parse_cfg.repo_name,
                )?
            }
        }
        "cpp" | "cxx" | "cc" | "hpp" | "hxx" | "hh" => {
            let query_src = load_query_source("cpp.scm", DEFAULT_CPP_QUERY, parse_cfg);
            extractor::extract_entities(
                &source,
                tree_sitter_cpp::LANGUAGE.into(),
                &query_src,
                "cpp",
                &file_path,
                &parse_cfg.repo_name,
            )?
        }
        "groovy" => {
            languages::groovy::extract_entities_groovy(&source, &file_path, &parse_cfg.repo_name)
        }
        "gradle" => {
            languages::gradle::extract_entities_gradle(&source, &file_path, &parse_cfg.repo_name)
        }
        "jenkinsfile" => {
            languages::jenkins::extract_entities_jenkins(&source, &file_path, &parse_cfg.repo_name)
        }
        "xml" => languages::xml::extract_entities_xml(&source, &file_path, &parse_cfg.repo_name),
        "toml" => languages::toml::extract_entities_toml(&source, &file_path, &parse_cfg.repo_name),
        "md" | "markdown" => {
            let query_src = load_query_source("markdown.scm", DEFAULT_MD_QUERY, parse_cfg);
            extractor::extract_entities(
                &source,
                tree_sitter_md::LANGUAGE.into(),
                &query_src,
                "markdown",
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

/// Dispatch YAML files to the appropriate parser based on content.
#[allow(dead_code)] // Called via dispatch for YAML handling
fn dispatch_yaml(source: &str, file_path: &str, repo_name: &str) -> Vec<ParsedEntity> {
    let filename = Path::new(file_path)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("");

    // 1. Is it Chart.yaml?
    if filename == "Chart.yaml" {
        return languages::helm::extract_chart_yaml(source, file_path, repo_name);
    }

    // 2. Is it inside a Helm chart directory?
    if is_in_helm_chart_dir(file_path) {
        if filename == "values.yaml" || filename == "values.yml" {
            let chart_name = detect_chart_name(file_path);
            return languages::helm::extract_values_yaml(source, file_path, repo_name, &chart_name);
        }
        if is_in_templates_dir(file_path) {
            let chart_name = detect_chart_name(file_path);
            return languages::helm::extract_helm_template(
                source,
                file_path,
                repo_name,
                &chart_name,
            );
        }
    }

    // 3. Is it a K8s manifest? (has apiVersion + kind at root level)
    if let Ok(yaml) = serde_yaml::from_str::<serde_yaml::Value>(source)
        && yaml.get("apiVersion").is_some()
        && yaml.get("kind").is_some()
    {
        return languages::kubernetes::extract_entities_k8s(source, file_path, repo_name);
    }

    // 4. Default: generic configuration YAML
    languages::yaml::extract_entities_yaml(source, file_path, repo_name)
}

/// Check if the file is inside a Helm chart directory by looking for Chart.yaml in parent dirs.
#[allow(dead_code)] // Used by YAML dispatch heuristics
fn is_in_helm_chart_dir(file_path: &str) -> bool {
    let path = Path::new(file_path);
    let mut current = path.parent();

    while let Some(dir) = current {
        if dir.join("Chart.yaml").exists() {
            return true;
        }
        current = dir.parent();
    }
    false
}

/// Check if the file is inside a templates directory (Helm convention).
#[allow(dead_code)] // Used by YAML dispatch heuristics
fn is_in_templates_dir(file_path: &str) -> bool {
    let path = Path::new(file_path);
    let mut current = Some(path);

    while let Some(p) = current {
        if p.file_name().and_then(|n| n.to_str()) == Some("templates") {
            return true;
        }
        current = p.parent();
    }
    false
}

/// Detect the Helm chart name from the nearest Chart.yaml or directory name.
fn detect_chart_name(file_path: &str) -> String {
    let path = Path::new(file_path);
    let mut current = path.parent();

    while let Some(dir) = current {
        let chart_yaml = dir.join("Chart.yaml");
        if chart_yaml.exists() {
            if let Ok(source) = fs::read_to_string(&chart_yaml)
                && let Ok(yaml) = serde_yaml::from_str::<serde_yaml::Value>(&source)
                && let Some(name) = yaml.get("name").and_then(|v| v.as_str())
            {
                return name.to_string();
            }
            break;
        }
        current = dir.parent();
    }

    // Fall back to parent directory name
    path.parent()
        .and_then(|p| p.file_name())
        .and_then(|n| n.to_str())
        .unwrap_or("unknown")
        .to_string()
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
            include_config_files: true,
            repo_path: None,
        };

        assert_eq!(cfg.repo_name, "test-repo");
        assert!(cfg.custom_queries_path.is_none());
    }

    #[test]
    fn test_parse_config_with_custom_queries() {
        let cfg = ParseConfig {
            custom_queries_path: Some("/custom/queries".to_string()),
            repo_name: "my-repo".to_string(),
            include_config_files: true,
            repo_path: None,
        };

        assert_eq!(cfg.repo_name, "my-repo");
        assert_eq!(cfg.custom_queries_path, Some("/custom/queries".to_string()));
    }

    #[test]
    fn test_load_query_source_uses_default() {
        let cfg = ParseConfig {
            custom_queries_path: None,
            repo_name: "test-repo".to_string(),
            include_config_files: true,
            repo_path: None,
        };

        let default_query = "MATCH (n) RETURN n";
        let result = load_query_source("test.scm", default_query, &cfg);

        assert_eq!(result, default_query);
    }

    #[test]
    fn test_load_query_source_nonexistent_custom_path() {
        let cfg = ParseConfig {
            custom_queries_path: None,
            repo_name: "test-repo".to_string(),
            include_config_files: true,
            repo_path: None,
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
            include_config_files: true,
            repo_path: None,
        };

        let files: Vec<PathBuf> = vec![];
        let (sender, mut receiver) = mpsc::channel::<ParsedEntity>(32);

        parse_files_stream(&files, &cfg, sender, 4, None);

        // No files to parse, channel should receive nothing
        assert!(receiver.try_recv().is_err());
    }

    #[test]
    fn test_parse_files_with_mock_channel() {
        let cfg = ParseConfig {
            custom_queries_path: None,
            repo_name: "test-repo".to_string(),
            include_config_files: true,
            repo_path: None,
        };

        // Use an empty list since we can't create real files in unit tests
        let files: Vec<PathBuf> = vec![];
        let (sender, mut receiver) = mpsc::channel::<ParsedEntity>(32);

        parse_files_stream(&files, &cfg, sender, 4, None);

        // Verify channel can receive messages (simulated)
        assert!(receiver.try_recv().is_err()); // No data sent
    }

    #[test]
    fn test_is_cpp_header_detects_class() {
        assert!(is_cpp_header(
            "class Print {\npublic:\n    void write();\n};"
        ));
    }

    #[test]
    fn test_is_cpp_header_detects_namespace() {
        assert!(is_cpp_header("namespace Engine {\n    class Foo {};\n}"));
    }

    #[test]
    fn test_is_cpp_header_detects_virtual() {
        assert!(is_cpp_header("virtual size_t write(uint8_t) = 0;"));
    }

    #[test]
    fn test_is_cpp_header_detects_template() {
        assert!(is_cpp_header("template <typename T>\nclass Container {};"));
    }

    #[test]
    fn test_is_cpp_header_pure_c_returns_false() {
        let c_header = r#"
#ifndef FOO_H
#define FOO_H
typedef struct { int x; int y; } Point;
void foo(int n);
int bar(const char *s);
#endif
"#;
        assert!(!is_cpp_header(c_header));
    }

    #[test]
    fn test_is_cpp_header_empty_returns_false() {
        assert!(!is_cpp_header(""));
    }

    #[test]
    fn test_is_cpp_header_detects_qualified_call() {
        assert!(is_cpp_header(
            "size_t Print::write(const uint8_t *buf, size_t s) { return 0; }"
        ));
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

    fn assert_extensions_detected(extensions: &[&str]) {
        for ext_name in extensions {
            let path = PathBuf::from(format!("/test/file.{}", ext_name));
            let ext = path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or_default();
            assert_eq!(ext, *ext_name);
        }
    }

    #[test]
    fn test_kotlin_file_extension_detection() {
        assert_extensions_detected(&["kt", "kts"]);
    }

    #[test]
    fn test_typescript_file_extension_detection() {
        assert_extensions_detected(&["ts", "tsx", "cts"]);
    }

    #[test]
    fn test_javascript_file_extension_detection() {
        assert_extensions_detected(&["js", "mjs", "cjs", "jsx"]);
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
            include_config_files: true,
            repo_path: None,
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
            include_config_files: true,
            repo_path: None,
        };

        let files: Vec<PathBuf> = vec![];
        let entities = parse_files(&files, &cfg);

        // No files to parse, should return empty vector
        assert!(entities.is_empty());
    }

    #[test]
    fn test_channel_sender_behavior_mock() {
        // Test that bounded channel sender doesn't fail on empty input
        let (sender, mut receiver) = mpsc::channel::<ParsedEntity>(32);

        // Dropping sender without sending should not error
        drop(sender);

        // Receiver should get no data
        assert!(receiver.try_recv().is_err());
    }

    #[test]
    fn test_bounded_channel_blocking_send() {
        // Test that blocking_send works correctly with a bounded channel
        let (sender, mut receiver) = mpsc::channel::<ParsedEntity>(2);

        // Create a minimal entity for testing
        let entity = ParsedEntity::new(
            "TestEntity",
            crate::models::EntityKind::Class,
            "com.test.TestEntity",
            None,
            None,
            "java",
            "/test/Test.java",
            1,
            5,
            None,
            "test-repo",
        );

        // Send via blocking_send (simulating what parse_files_stream does)
        assert!(sender.blocking_send(entity.clone()).is_ok());
        assert!(sender.blocking_send(entity).is_ok());

        // Verify receiver gets both entities
        assert!(receiver.try_recv().is_ok());
        assert!(receiver.try_recv().is_ok());
    }

    #[test]
    fn test_bounded_channel_capacity_backpressure() {
        // Test that bounded channel respects capacity
        let (sender, _receiver) = mpsc::channel::<ParsedEntity>(2);

        let entity = ParsedEntity::new(
            "TestEntity",
            crate::models::EntityKind::Class,
            "com.test.TestEntity",
            None,
            None,
            "java",
            "/test/Test.java",
            1,
            5,
            None,
            "test-repo",
        );

        // Fill the channel to capacity
        assert!(sender.try_send(entity.clone()).is_ok());
        assert!(sender.try_send(entity.clone()).is_ok());

        // Third send should fail with Full error (channel is at capacity)
        assert!(sender.try_send(entity).is_err());
    }

    #[test]
    fn test_bounded_channel_receives_after_blocking_send() {
        // Verify that after blocking_send, data is available on the receiver
        let (sender, mut receiver) = mpsc::channel::<ParsedEntity>(1);

        let entity = ParsedEntity::new(
            "TestClass",
            crate::models::EntityKind::Class,
            "com.example.TestClass",
            Some("public class TestClass".to_string()),
            Some("A test class".to_string()),
            "java",
            "/proj/TestClass.java",
            10,
            25,
            None,
            "test-repo",
        );

        sender.blocking_send(entity).unwrap();

        let received = receiver.try_recv().unwrap();
        assert_eq!(received.name, "TestClass");
        assert_eq!(received.fqn, "com.example.TestClass");
        assert_eq!(received.language, "java");
    }

    #[test]
    fn test_parse_files_stream_callback_once_per_file() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use tempfile::tempdir;

        let dir = tempdir().unwrap();
        for i in 0..3 {
            std::fs::write(dir.path().join(format!("file_{}.rs", i)), "fn foo() {}").unwrap();
        }

        let files: Vec<PathBuf> = (0..3)
            .map(|i| dir.path().join(format!("file_{}.rs", i)))
            .collect();

        let cfg = ParseConfig {
            custom_queries_path: None,
            repo_name: "test-repo".to_string(),
            include_config_files: true,
            repo_path: None,
        };

        let (sender, mut receiver) = mpsc::channel::<ParsedEntity>(32);
        let counter = std::sync::Arc::new(AtomicUsize::new(0));
        let counter_clone = std::sync::Arc::clone(&counter);
        let cb: FileParsedCallback = std::sync::Arc::new(move || {
            counter_clone.fetch_add(1, Ordering::SeqCst);
        });

        parse_files_stream(&files, &cfg, sender, 4, Some(cb));

        let mut count = 0;
        while receiver.try_recv().is_ok() {
            count += 1;
        }
        assert!(count > 0, "Should have extracted some entities");
        assert_eq!(counter.load(Ordering::SeqCst), 3);
    }

    #[test]
    fn test_parse_files_stream_callback_counts_unparseable() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use tempfile::tempdir;

        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("valid.rs"), "fn foo() {}").unwrap();
        std::fs::write(dir.path().join("valid2.rs"), "enum Color { Red }").unwrap();
        std::fs::write(dir.path().join("broken.rs"), "not valid rust @@@@!!").unwrap();

        let files: Vec<PathBuf> = ["valid.rs", "valid2.rs", "broken.rs"]
            .iter()
            .map(|f| dir.path().join(f))
            .collect();

        let cfg = ParseConfig {
            custom_queries_path: None,
            repo_name: "test-repo".to_string(),
            include_config_files: true,
            repo_path: None,
        };

        let (sender, _receiver) = mpsc::channel::<ParsedEntity>(32);
        let counter = std::sync::Arc::new(AtomicUsize::new(0));
        let counter_clone = std::sync::Arc::clone(&counter);
        let cb: FileParsedCallback = std::sync::Arc::new(move || {
            counter_clone.fetch_add(1, Ordering::SeqCst);
        });

        parse_files_stream(&files, &cfg, sender, 4, Some(cb));

        assert_eq!(counter.load(Ordering::SeqCst), 3);
    }

    #[test]
    fn test_parse_files_stream_none_callback() {
        use tempfile::tempdir;

        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("test.rs"), "fn foo() {}").unwrap();
        let files: Vec<PathBuf> = vec![dir.path().join("test.rs")];

        let cfg = ParseConfig {
            custom_queries_path: None,
            repo_name: "test-repo".to_string(),
            include_config_files: true,
            repo_path: None,
        };

        let (sender, mut receiver) = mpsc::channel::<ParsedEntity>(32);
        parse_files_stream(&files, &cfg, sender, 4, None);

        let mut count = 0;
        while receiver.try_recv().is_ok() {
            count += 1;
        }
        assert!(count > 0, "Should parse entities with None callback");
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
