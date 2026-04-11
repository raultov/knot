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
mod languages;
mod orphans;
mod utils;

#[cfg(test)]
mod test_utils;

// Built-in query files compiled into the binary.
const DEFAULT_JAVA_QUERY: &str = include_str!("../../../queries/java.scm");
const DEFAULT_TS_QUERY: &str = include_str!("../../../queries/typescript.scm");
const DEFAULT_TSX_QUERY: &str = include_str!("../../../queries/tsx.scm");

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
