//! Utility helpers: logging initialisation and miscellaneous functions.

use anyhow::{Context, Result};
use std::io::{IsTerminal, Write};
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

    fmt()
        .with_env_filter(filter)
        .with_target(false)
        .with_writer(std::io::stderr)
        .init();

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

/// Injects custom CA certificates into the process environment for TLS connections.
///
/// This is required for `fastembed`/`hf-hub` to work through corporate SSL-inspecting proxies.
/// Must be called before any async runtime threads are spawned (i.e., early in `main()`).
///
/// # Safety
/// `std::env::set_var` is marked unsafe in Rust 2024 because concurrent modification
/// from multiple threads is a data race. This function is safe because:
/// - It is called exactly once, early in main(), before any Tokio threads exist.
/// - The tokio runtime is not yet running at this point.
#[inline(always)]
pub fn inject_custom_ca_certs(cert_path: &Option<String>) {
    if let Some(path) = cert_path {
        // SAFETY: This is safe because:
        // 1. Called before any threads exist (single-threaded main context)
        // 2. No other code can concurrently modify env vars at this point
        // 3. Tokio runtime hasn't been entered yet
        #[expect(
            unsafe_code,
            reason = "std::env::set_var is unsafe in Rust 2024. fastembed 5.13 and \
                      hf-hub expose no API to supply a CA bundle (InitOptions has no \
                      TLS options; ApiBuilder is constructed internally), so \
                      SSL_CERT_FILE is the only mechanism. Called once from main() \
                      before the Tokio runtime starts, so no other thread can observe \
                      the mutation."
        )]
        unsafe {
            std::env::set_var("SSL_CERT_FILE", path);
        }
        tracing::info!("Injected custom CA certificate path: {}", path);
    }
}

/// Print content through a pager (`less -R -e`) when stdout is a terminal,
/// otherwise print directly to stdout.
pub fn print_with_pager(content: &str) {
    use std::process::{Command, Stdio};

    if std::io::stdout().is_terminal()
        && let Ok(mut child) = Command::new("less")
            .arg("-R")
            .arg("-e")
            .stdin(Stdio::piped())
            .spawn()
    {
        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(content.as_bytes());
        }
        let _ = child.wait();
        return;
    }
    println!("{}", content);
}

/// Format search results according to the requested output format.
pub fn format_output(
    json_value: serde_json::Value,
    output_format: crate::config::OutputFormat,
) -> String {
    match output_format {
        crate::config::OutputFormat::Table => {
            if json_value.is_null() {
                return "No matching code found for your query.".to_string();
            }
            crate::cli_tools::formatters::format_search_table(&json_value)
        }
        crate::config::OutputFormat::Json => {
            serde_json::to_string_pretty(&json_value).unwrap_or_default()
        }
        crate::config::OutputFormat::Markdown => {
            crate::cli_tools::formatters::format_search_results(&json_value)
        }
    }
}

/// Format callers (reverse dependency) results according to the requested output format.
pub fn format_callers_output(
    entity_name: &str,
    json_value: serde_json::Value,
    output_format: crate::config::OutputFormat,
) -> String {
    match output_format {
        crate::config::OutputFormat::Table => {
            crate::cli_tools::formatters::format_callers_table(entity_name, &json_value)
        }
        crate::config::OutputFormat::Json => {
            serde_json::to_string_pretty(&json_value).unwrap_or_default()
        }
        crate::config::OutputFormat::Markdown => {
            crate::cli_tools::format_references_result(entity_name, &json_value)
        }
    }
}

/// Format file exploration results according to the requested output format.
pub fn format_explore_output(
    file_path: &str,
    json_value: serde_json::Value,
    output_format: crate::config::OutputFormat,
) -> String {
    match output_format {
        crate::config::OutputFormat::Table => {
            crate::cli_tools::formatters::format_explore_table(file_path, &json_value)
        }
        crate::config::OutputFormat::Json => {
            serde_json::to_string_pretty(&json_value).unwrap_or_default()
        }
        crate::config::OutputFormat::Markdown => {
            crate::cli_tools::format_file_entities(file_path, &json_value)
        }
    }
}

/// Compute the number of Rayon threads to use based on available CPUs.
///
/// Default formula: `logical_cpus - 1` (leaves 1 core for tokio runtime + OS).
/// Minimum: 2 threads. The formula can be overridden by an explicit `threads` parameter.
pub fn calculate_rayon_threads(threads: Option<usize>) -> usize {
    let cpus = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4);

    threads.unwrap_or(cpus.saturating_sub(1).max(2))
}

/// Configure the Rayon thread pool size using the global builder.
///
/// Must be called before any parallel parsing occurs and before the tokio runtime
/// spawns blocking tasks. Respects the `KNOT_RAYON_THREADS` env var override.
///
/// # Errors
/// Returns an error if the global thread pool has already been initialized.
pub fn configure_rayon(threads: Option<usize>) -> Result<usize> {
    let cpus = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4);

    let thread_count = calculate_rayon_threads(threads);

    rayon::ThreadPoolBuilder::new()
        .num_threads(thread_count)
        .build_global()
        .context("Failed to initialize Rayon thread pool")?;

    tracing::info!(
        "Rayon thread pool initialized with {thread_count} threads ({cpus} logical CPUs)"
    );
    Ok(thread_count)
}

/// Print startup banner with configuration details for the indexer.
#[expect(
    clippy::cognitive_complexity,
    reason = "Banner printing is sequential formatting logic"
)]
pub fn print_startup_banner(cfg: &crate::config::Config, rayon_threads: usize) {
    let cpus = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4);

    tracing::info!(
        "knot indexer starting (v{} - parallel streaming + watch mode)",
        env!("CARGO_PKG_VERSION")
    );
    tracing::info!("Repository path : {}", cfg.repo_path);
    tracing::info!("Repository name : {}", cfg.repo_name);
    tracing::info!("Logical CPUs    : {cpus}");
    tracing::info!("Rayon threads   : {rayon_threads}");
    tracing::info!("Batch size      : {}", cfg.batch_size);
    tracing::info!("Ingest workers  : {}", cfg.ingest_concurrency);
    tracing::info!("Clean mode      : {}", cfg.clean);
    tracing::info!("Watch mode      : {}", cfg.watch);
    tracing::info!(
        "Qdrant          : {} / {}",
        cfg.qdrant_url,
        cfg.qdrant_collection
    );
    tracing::info!("Neo4j           : {}", cfg.neo4j_uri);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    static ENV_MUTEX: Mutex<()> = Mutex::new(());

    #[test]
    fn test_inject_custom_ca_certs_none() {
        let _lock = ENV_MUTEX.lock().unwrap();
        temp_env::with_var("SSL_CERT_FILE", None::<&str>, || {
            let original = std::env::var("SSL_CERT_FILE").ok();
            inject_custom_ca_certs(&None);
            assert_eq!(std::env::var("SSL_CERT_FILE").ok(), original);
        });
    }

    #[test]
    fn test_inject_custom_ca_certs_some() {
        let _lock = ENV_MUTEX.lock().unwrap();
        temp_env::with_var("SSL_CERT_FILE", None::<&str>, || {
            let test_path = "/path/to/test/ca-bundle.crt".to_string();
            inject_custom_ca_certs(&Some(test_path.clone()));
            assert_eq!(std::env::var("SSL_CERT_FILE").ok(), Some(test_path));
        });
    }

    #[test]
    fn test_inject_custom_ca_certs_overwrites_previous() {
        let _lock = ENV_MUTEX.lock().unwrap();
        temp_env::with_var("SSL_CERT_FILE", None::<&str>, || {
            let first = "/first/path.pem".to_string();
            let second = "/second/path.pem".to_string();
            inject_custom_ca_certs(&Some(first));
            inject_custom_ca_certs(&Some(second.clone()));
            assert_eq!(std::env::var("SSL_CERT_FILE").ok(), Some(second));
        });
    }

    // --- format_output tests ---

    #[test]
    fn test_format_output_null_returns_no_match_message() {
        let result = format_output(serde_json::Value::Null, crate::config::OutputFormat::Table);
        assert_eq!(result, "No matching code found for your query.");
    }

    #[test]
    fn test_format_output_json_pretty_print() {
        let json = serde_json::json!({"name": "Test", "kind": "class"});
        let result = format_output(json.clone(), crate::config::OutputFormat::Json);
        assert!(result.contains("Test"));
        assert!(result.contains("class"));
    }

    // --- format_callers_output tests ---

    #[test]
    fn test_format_callers_output_json() {
        let json = serde_json::json!({
            "calls": [{"name": "caller1"}],
            "extends": [],
            "implements": [],
            "references": []
        });
        let result = format_callers_output("MyEntity", json, crate::config::OutputFormat::Json);
        assert!(result.contains("caller1"));
    }

    // --- format_explore_output tests ---

    #[test]
    fn test_format_explore_output_json() {
        let json = serde_json::json!([{"name": "MyClass", "kind": "class"}]);
        let result = format_explore_output("test.java", json, crate::config::OutputFormat::Json);
        assert!(result.contains("MyClass"));
    }

    // --- calculate_rayon_threads tests ---

    #[test]
    fn test_calculate_rayon_threads_default_formula() {
        let cpus = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(4);
        let thread_count = calculate_rayon_threads(None);
        assert_eq!(thread_count, cpus.saturating_sub(1).max(2));
    }

    #[test]
    fn test_calculate_rayon_threads_explicit_override() {
        let thread_count = calculate_rayon_threads(Some(8));
        assert_eq!(thread_count, 8);
    }

    #[test]
    fn test_calculate_rayon_threads_explicit_zero() {
        let thread_count = calculate_rayon_threads(Some(0));
        assert_eq!(thread_count, 0);
    }
}
