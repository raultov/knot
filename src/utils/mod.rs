//! Utility helpers: logging initialisation and miscellaneous functions.

use anyhow::Result;
use tracing_subscriber::{EnvFilter, fmt};

/// Initialise the global `tracing` subscriber.
///
/// Log level is controlled by the `RUST_LOG` environment variable.
/// Falls back to `info` when the variable is not set.
///
/// # Example
/// ```text
/// RUST_LOG=debug knot --repo-path /path/to/repo
/// ```
pub fn init_logging() -> Result<()> {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));

    fmt().with_env_filter(filter).with_target(false).init();

    Ok(())
}

/// Initialise the global `tracing` subscriber for the CLI tool.
///
/// This is a specialized version for the `knot` CLI that:
/// - Defaults to `error` level (not `info`) to minimize noise from dependencies
/// - Can be overridden by the `RUST_LOG` environment variable
/// - Sends logs to stderr to avoid contaminating stdout (which contains query results)
///
/// # Example
/// ```text
/// # Default (only errors shown)
/// knot search "something"
///
/// # Override to show more detail
/// RUST_LOG=debug knot search "something"
/// ```
pub fn init_logging_for_cli() -> Result<()> {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("error"));

    fmt()
        .with_env_filter(filter)
        .with_target(false)
        .with_writer(std::io::stderr) // Ensure logs go to stderr, not stdout
        .init();

    Ok(())
}
