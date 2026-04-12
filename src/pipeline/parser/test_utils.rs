//! Test utilities for parsing snippets of code in memory.

use tree_sitter::{Parser, Tree};

/// Parse a Java code snippet and return the syntax tree.
#[cfg(test)]
pub(crate) fn parse_java_snippet(code: &str) -> Result<Tree, String> {
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_java::LANGUAGE.into())
        .map_err(|e| format!("Failed to set Java language: {e}"))?;

    parser
        .parse(code, None)
        .ok_or_else(|| "Failed to parse Java code snippet".to_string())
}

/// Parse a TypeScript code snippet and return the syntax tree.
#[cfg(test)]
pub(crate) fn parse_typescript_snippet(code: &str) -> Result<Tree, String> {
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into())
        .map_err(|e| format!("Failed to set TypeScript language: {e}"))?;

    parser
        .parse(code, None)
        .ok_or_else(|| "Failed to parse TypeScript code snippet".to_string())
}

/// Parse a TSX code snippet and return the syntax tree.
#[cfg(test)]
pub(crate) fn parse_tsx_snippet(code: &str) -> Result<Tree, String> {
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_typescript::LANGUAGE_TSX.into())
        .map_err(|e| format!("Failed to set TSX language: {e}"))?;

    parser
        .parse(code, None)
        .ok_or_else(|| "Failed to parse TSX code snippet".to_string())
}

/// Parse a JavaScript code snippet and return the syntax tree.
#[cfg(test)]
pub(crate) fn parse_javascript_snippet(code: &str) -> Result<Tree, String> {
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_javascript::LANGUAGE.into())
        .map_err(|e| format!("Failed to set JavaScript language: {e}"))?;

    parser
        .parse(code, None)
        .ok_or_else(|| "Failed to parse JavaScript code snippet".to_string())
}

/// Parse a JSX code snippet and return the syntax tree.
#[cfg(test)]
#[allow(dead_code)]
pub(crate) fn parse_jsx_snippet(code: &str) -> Result<Tree, String> {
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_javascript::LANGUAGE.into())
        .map_err(|e| format!("Failed to set JSX language: {e}"))?;

    parser
        .parse(code, None)
        .ok_or_else(|| "Failed to parse JSX code snippet".to_string())
}
