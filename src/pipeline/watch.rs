//! Filesystem watch mode for real-time incremental indexing.
//!
//! Monitors the repository for changes to supported source files and
//! triggers the indexing pipeline automatically with intelligent batching
//! to avoid redundant re-indexing when IDEs generate multiple events.

use anyhow::Result;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{error, info, warn};

use crate::config::Config;
use crate::db::{graph::GraphDb, vector::VectorDb};
use crate::pipeline::{files::is_supported_file, runner::run_indexing_pipeline, state::IndexState};

/// Setup and run the watch mode event loop.
///
/// This function:
/// 1. Creates a debounced filesystem watcher
/// 2. Groups filesystem events into batches of supported source files
/// 3. Drains accumulated events before indexing to avoid redundant pipeline executions
/// 4. Deduplicates paths within a batch
/// 5. Triggers incremental re-indexing once per batch
/// 6. Applies a post-indexation cooldown to capture stray filesystem events from IDEs/linters
/// 7. Drains residual events after cooldown to prevent ghost re-indexations
pub async fn setup_watch_mode(
    cfg: &Config,
    vector_db: &Arc<VectorDb>,
    graph_db: &Arc<GraphDb>,
    index_state: &mut IndexState,
) -> Result<()> {
    let (tx, mut rx) = mpsc::channel::<Vec<PathBuf>>(100);

    // Setup debounced filesystem watcher
    let mut debouncer = notify_debouncer_mini::new_debouncer(
        std::time::Duration::from_millis(500),
        move |res: notify_debouncer_mini::DebounceEventResult| {
            if let Ok(events) = res {
                // Filter to only supported files and collect into a single batch
                let paths: Vec<PathBuf> = events
                    .into_iter()
                    .map(|e| e.path)
                    .filter(|p| is_supported_file(p))
                    .collect();

                // Only send if we found at least one supported file
                if !paths.is_empty() {
                    let _ = tx.blocking_send(paths);
                }
            }
        },
    )?;

    // Attempt to set up recursive watching
    // If permission denied on a subdirectory (e.g., data/neo4j/import), fall back to non-recursive
    let repo_path = Path::new(&cfg.repo_path);
    match debouncer
        .watcher()
        .watch(repo_path, notify::RecursiveMode::Recursive)
    {
        Ok(()) => {
            info!("Recursive watch mode enabled for {}", cfg.repo_path);
        }
        Err(e) => {
            let err_msg = e.to_string();
            if err_msg.contains("Permission denied") {
                warn!(
                    "Permission denied when watching subdirectory. \
                     This can happen for system directories (e.g., data/neo4j/import). \
                     Falling back to non-recursive watch mode for the repository root."
                );
                debouncer
                    .watcher()
                    .watch(repo_path, notify::RecursiveMode::NonRecursive)?;
                warn!("Watch mode is now monitoring only top-level directories.");
            } else {
                return Err(e.into());
            }
        }
    }

    // Event loop for watch mode
    while let Some(mut paths) = rx.recv().await {
        // [PHASE 1] Drain any additional events that arrived while we were idle or processing
        // This prevents redundant pipeline executions when multiple file changes
        // are detected in quick succession
        while let Ok(mut more_paths) = rx.try_recv() {
            paths.append(&mut more_paths);
        }

        // Deduplicate paths (IDEs may report the same file multiple times)
        paths.sort();
        paths.dedup();

        // Log the detected changes
        if paths.len() == 1 {
            info!("Change detected in: {}", paths[0].display());
        } else {
            info!(
                "Changes detected in {} files, triggering update...",
                paths.len()
            );
        }

        // [PHASE 2] Execute the pipeline once for all accumulated changes
        if let Err(e) = run_indexing_pipeline(cfg, vector_db, graph_db, index_state).await {
            error!("Error during incremental update: {e:#}");
        }

        // [PHASE 3] Post-indexation cooldown: capture stray filesystem events
        // While the indexer was working, IDEs and background tools (linters, formatters)
        // may have generated additional filesystem events. These arrive as "bounces"
        // after the indexer completes. We sleep briefly to let these settle.
        tokio::time::sleep(tokio::time::Duration::from_millis(300)).await;

        // [PHASE 4] Drain residual events from the cooldown period
        // These are discarded because we just indexed the fresh state of the repository.
        // Any NEW user edits arriving after this cooldown will trigger the next cycle.
        let mut residual_count = 0;
        while rx.try_recv().is_ok() {
            residual_count += 1;
        }
        if residual_count > 0 {
            info!(
                "Discarded {} residual filesystem event(s) from post-indexation period",
                residual_count
            );
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
        assert!(is_supported_file(Path::new("test.kt")));
        assert!(is_supported_file(Path::new("test.kts")));
        assert!(is_supported_file(Path::new("test.html")));
        assert!(is_supported_file(Path::new("test.css")));
        assert!(is_supported_file(Path::new("test.scss")));
        assert!(is_supported_file(Path::new("test.rs")));

        assert!(!is_supported_file(Path::new("test.txt")));
        assert!(is_supported_file(Path::new("test.py")));
        assert!(is_supported_file(Path::new("test.pyi")));
        assert!(is_supported_file(Path::new("test.pyw")));
    }

    #[test]
    fn test_path_deduplication() {
        // Test that duplicate paths are correctly removed
        let mut paths = vec![
            PathBuf::from("src/file1.ts"),
            PathBuf::from("src/file2.ts"),
            PathBuf::from("src/file1.ts"),
            PathBuf::from("src/file3.ts"),
            PathBuf::from("src/file2.ts"),
        ];

        paths.sort();
        paths.dedup();

        assert_eq!(paths.len(), 3);
        assert_eq!(paths[0], PathBuf::from("src/file1.ts"));
        assert_eq!(paths[1], PathBuf::from("src/file2.ts"));
        assert_eq!(paths[2], PathBuf::from("src/file3.ts"));
    }

    #[test]
    fn test_path_deduplication_empty() {
        // Test that empty paths remain empty after deduplication
        let mut paths: Vec<PathBuf> = vec![];

        paths.sort();
        paths.dedup();

        assert_eq!(paths.len(), 0);
    }

    #[test]
    fn test_path_deduplication_single_element() {
        // Test that single element paths are preserved
        let mut paths = vec![PathBuf::from("src/only_file.ts")];

        paths.sort();
        paths.dedup();

        assert_eq!(paths.len(), 1);
        assert_eq!(paths[0], PathBuf::from("src/only_file.ts"));
    }

    #[test]
    fn test_path_deduplication_all_duplicates() {
        // Test that all duplicates of the same path are reduced to one
        let mut paths = vec![
            PathBuf::from("src/file.ts"),
            PathBuf::from("src/file.ts"),
            PathBuf::from("src/file.ts"),
            PathBuf::from("src/file.ts"),
        ];

        paths.sort();
        paths.dedup();

        assert_eq!(paths.len(), 1);
        assert_eq!(paths[0], PathBuf::from("src/file.ts"));
    }

    #[test]
    fn test_supported_file_extensions() {
        // Test all supported extensions
        let supported = vec!["file.java", "module.ts", "component.tsx", "script.cts"];

        for filename in supported {
            assert!(
                is_supported_file(Path::new(filename)),
                "File {} should be supported",
                filename
            );
        }
    }

    #[test]
    fn test_unsupported_file_extensions() {
        // Test common unsupported extensions (JS/TS, CSS, HTML, Kotlin, Rust, Python are now supported)
        let unsupported = vec![
            "readme.md",
            "config.json",
            "data.xml",
            "document.txt",
            "image.png",
            "script.pyc",
            "script.pyo",
            "script.py.bak",
            "styles.less",
        ];

        for filename in unsupported {
            assert!(
                !is_supported_file(Path::new(filename)),
                "File {} should not be supported",
                filename
            );
        }
    }

    #[test]
    fn test_supported_file_with_nested_paths() {
        // Test that files with nested paths are correctly identified
        let supported_paths = vec![
            "src/components/Frame.ts",
            "packages/core/src/index.ts",
            "lib/utils/helper.tsx",
            "main/java/com/example/MyClass.java",
        ];

        for path in supported_paths {
            assert!(
                is_supported_file(Path::new(path)),
                "Path {} should be supported",
                path
            );
        }
    }

    #[test]
    fn test_path_batch_accumulation_logic() {
        // Test the logic of accumulating multiple batches into one
        let mut batch1 = vec![PathBuf::from("src/file1.ts"), PathBuf::from("src/file2.ts")];

        let batch2 = vec![
            PathBuf::from("src/file3.ts"),
            PathBuf::from("src/file1.ts"), // Duplicate
        ];

        // Simulate appending batches
        batch1.append(&mut batch2.clone());

        // Simulate deduplication
        batch1.sort();
        batch1.dedup();

        assert_eq!(batch1.len(), 3);
        assert!(batch1.contains(&PathBuf::from("src/file1.ts")));
        assert!(batch1.contains(&PathBuf::from("src/file2.ts")));
        assert!(batch1.contains(&PathBuf::from("src/file3.ts")));
    }

    #[test]
    fn test_supported_files_case_sensitivity() {
        // Test that file extension matching is case-sensitive
        // (as per Rust's default behavior)
        assert!(is_supported_file(Path::new("file.ts")));
        // .TS (uppercase) should not be supported as is_supported_file uses lowercase matching
        assert!(!is_supported_file(Path::new("file.TS")));
    }
}
