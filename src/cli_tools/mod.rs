//! Core CLI Tools Module
//!
//! Shared logic for CLI and MCP tools. These functions encapsulate
//! the business logic for searching, finding callers, and exploring files.
//! Both the CLI and MCP interfaces use these functions to avoid duplication.

pub mod explore_file;
pub mod find_callers;
pub mod formatters;
pub mod search_hybrid_context;

pub use explore_file::{format_file_entities, run_explore_file};
pub use find_callers::{format_reference_entry, format_references_result, run_find_callers};
pub use search_hybrid_context::run_search_hybrid_context;
