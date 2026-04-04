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
