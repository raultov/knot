//! Output formatters for CLI and MCP tools.
//!
//! Provides table, json, and markdown formatting options
//! for displaying search results, callers, and file entities.

pub mod markdown;
pub mod table;

pub use markdown::format_search_results;
pub use table::{format_callers_table, format_explore_table, format_search_table};
