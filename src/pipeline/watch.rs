//! Filesystem watch mode for real-time incremental indexing.
//!
//! Monitors the repository for changes to supported source files and
//! triggers the indexing pipeline automatically.

use anyhow::Result;
use std::path::Path;
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{error, info};

use crate::config::Config;
use crate::db::{graph::GraphDb, vector::VectorDb};
use crate::pipeline::{files::is_supported_file, runner::run_indexing_pipeline, state::IndexState};

/// Setup and run the watch mode event loop.
///
/// This function:
/// 1. Creates a debounced filesystem watcher
/// 2. Listens for changes to supported source files (.java, .ts, .tsx, .cts)
/// 3. Triggers incremental re-indexing on file changes
pub async fn setup_watch_mode(
    cfg: &Config,
    vector_db: &Arc<VectorDb>,
    graph_db: &Arc<GraphDb>,
    index_state: &mut IndexState,
) -> Result<()> {
    let (tx, mut rx) = mpsc::channel(100);

    // Setup debounced filesystem watcher
    let mut debouncer = notify_debouncer_mini::new_debouncer(
        std::time::Duration::from_millis(500),
        move |res: notify_debouncer_mini::DebounceEventResult| {
            if let Ok(events) = res {
                for event in events {
                    let _ = tx.blocking_send(event.path);
                }
            }
        },
    )?;

    debouncer
        .watcher()
        .watch(Path::new(&cfg.repo_path), notify::RecursiveMode::Recursive)?;

    // Event loop for watch mode
    while let Some(path) = rx.recv().await {
        // Check if the changed file is a supported source file
        if is_supported_file(&path) {
            info!("Change detected in: {}", path.display());
            if let Err(e) = run_indexing_pipeline(cfg, vector_db, graph_db, index_state).await {
                error!("Error during incremental update: {e:#}");
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn test_watch_supported_files() {
        assert!(is_supported_file(Path::new("test.java")));
        assert!(is_supported_file(Path::new("test.ts")));
        assert!(is_supported_file(Path::new("test.tsx")));
        assert!(is_supported_file(Path::new("test.cts")));

        assert!(!is_supported_file(Path::new("test.txt")));
        assert!(!is_supported_file(Path::new("test.rs")));
    }
}
