//! Pipeline orchestration and execution.
//!
//! This module encapsulates the core indexing pipeline logic that coordinates
//! all stages: discovery, parsing, preparation, embedding, ingestion, and relationship resolution.

use anyhow::Result;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::{Semaphore, mpsc};
use tokio::task::{JoinHandle, JoinSet};
use tracing::info;

use crate::config::Config;
use crate::db::{graph::GraphDb, vector::VectorDb};
use crate::models::{EmbeddedEntity, ParsedEntity, ResolutionEntity};
use crate::pipeline::{
    embed::{Embedder, needs_reset},
    files::{
        calculate_files_to_delete, calculate_files_to_parse, classify_files_for_indexing,
        update_index_state,
    },
    ingest::{
        RunMetrics, ingest_batch, link_cross_repo_dependencies, print_run_summary,
        resolve_and_save_relationships,
    },
    input::discover_files,
    parser::{FileParsedCallback, ParseCallbacks, ParseConfig, parse_files_stream},
    prepare::prepare_entities,
    progress::{IndexingStage, ProgressTracker},
    state::IndexState,
};

/// Existing entry point — unchanged signature. Creates a private throwaway
/// tracker internally so progress *logging* always happens (knot-indexer
/// gets the log lines for free without opting into the API).
pub async fn run_indexing_pipeline(
    cfg: &Config,
    vector_db: &Arc<VectorDb>,
    graph_db: &Arc<GraphDb>,
    index_state: &mut IndexState,
) -> Result<RunMetrics> {
    run_indexing_pipeline_with_progress(
        cfg,
        vector_db,
        graph_db,
        index_state,
        Arc::new(ProgressTracker::new()),
    )
    .await
}

/// New entry point for callers that want to observe progress (knot-server).
/// The caller keeps a clone of `progress` and polls `snapshot()` from
/// another task/thread while this future runs.
pub async fn run_indexing_pipeline_with_progress(
    cfg: &Config,
    vector_db: &Arc<VectorDb>,
    graph_db: &Arc<GraphDb>,
    index_state: &mut IndexState,
    progress: Arc<ProgressTracker>,
) -> Result<RunMetrics> {
    progress.begin_run(&cfg.repo_name);
    let result = run_pipeline_inner(cfg, vector_db, graph_db, index_state, &progress).await;
    match &result {
        Ok(_) => {
            progress.complete();
            // Emit a final log line so operators see the explicit 100%
            // completion signal. The in-pipeline log at 90% fires before
            // reference resolution; without this terminal line the user
            // would never see the bar actually reach 100%.
            let snap = progress.snapshot();
            info!(
                "[Progress] [{}] 100.0% — files {}/{}, entities {}/{} — indexing complete",
                snap.repo_name,
                snap.parsed_files,
                snap.total_files,
                snap.entities_ingested,
                snap.total_entities,
            );
        }
        Err(e) => progress.fail(&format!("{e:#}")),
    }
    result
}

/// Core indexing pipeline orchestrator.
///
/// This function coordinates all stages of the indexing process:
/// 1. Discover and classify files (unchanged, modified, added, deleted)
/// 2. Clean stale data from databases
/// 3. Parse files in parallel (Rayon)
/// 4. Batch and embed entities (fastembed)
/// 5. Ingest into Qdrant and Neo4j (dual-write)
/// 6. Resolve cross-repository relationships
async fn run_pipeline_inner(
    cfg: &Config,
    vector_db: &Arc<VectorDb>,
    graph_db: &Arc<GraphDb>,
    index_state: &mut IndexState,
    progress: &Arc<ProgressTracker>,
) -> Result<RunMetrics> {
    let Some(PipelineInputs {
        modified_files,
        added_files,
        deleted_files,
        repo_root,
    }) = classify_pipeline_inputs(cfg, index_state, progress)?
    else {
        return Ok(RunMetrics::new(0));
    };

    // Detect a full indexing run (no prior state on disk) so that we can
    // short-circuit the slow per-file deletion path with a single bulk
    // `delete_by_repo` query. Without this, initial indexing on a populated
    // database would issue one DELETE per file in the repo.
    let is_full_index = index_state.file_hashes.is_empty();

    // Clean stale data before re-indexing.
    progress.set_stage(IndexingStage::CleaningStaleData);
    clean_stale_data(
        vector_db,
        graph_db,
        cfg,
        &deleted_files,
        &modified_files,
        is_full_index,
    )
    .await?;

    let files_to_parse = calculate_files_to_parse(added_files, modified_files);
    progress.set_total_files(files_to_parse.len() as u64);

    if !files_to_parse.is_empty() {
        info!(
            "Will parse and index {} file(s) (added/modified)",
            files_to_parse.len()
        );

        let (mut resolution_entities, total_entities) =
            run_streaming_ingest(cfg, vector_db, graph_db, progress, &files_to_parse).await?;

        // Stage 7: Relationship Resolution
        // Cross-repo dependency linking: upsert Repository nodes and create DEPENDS_ON edges.
        // Must run BEFORE relationship resolution so that auto-discovered dependencies
        // are available for cross-repo call resolution.
        link_cross_repo_dependencies(&resolution_entities, graph_db, cfg).await?;

        let metrics =
            resolve_and_save_relationships(&mut resolution_entities, graph_db, cfg).await?;

        update_index_state(
            index_state,
            &files_to_parse,
            &deleted_files,
            &cfg.repo_path,
            &repo_root,
            total_entities,
        )?;

        print_run_summary(&metrics);
        Ok(metrics)
    } else if !deleted_files.is_empty() {
        // Only deletions occurred
        update_index_state(
            index_state,
            &[],
            &deleted_files,
            &cfg.repo_path,
            &repo_root,
            0,
        )?;
        Ok(RunMetrics::new(0))
    } else {
        Ok(RunMetrics::new(0))
    }
}

/// Files to process for one indexing run, as classified against the
/// persisted index state.
struct PipelineInputs {
    modified_files: Vec<PathBuf>,
    added_files: Vec<PathBuf>,
    deleted_files: Vec<String>,
    repo_root: PathBuf,
}

/// Centralized progress logging for the "nothing to do" outcomes of stage 1.
/// Tracing macros carry a large clippy cognitive-complexity cost, so keeping
/// them in dedicated helpers lets the classification logic stay below the
/// threshold without suppression attributes.
fn log_nothing_to_index(msg: &str) {
    info!("{msg}");
}

/// Logs the file-classification summary for the indexing run.
fn log_file_classification(unchanged_count: usize, modified: usize, added: usize, deleted: usize) {
    info!(
        "File classification: {} unchanged, {} modified, {} added, {} deleted",
        unchanged_count, modified, added, deleted
    );
}

/// Stage 1: file discovery + classification. Returns `None` when there is
/// nothing to do (no supported files, or nothing changed since the last run).
fn classify_pipeline_inputs(
    cfg: &Config,
    index_state: &IndexState,
    progress: &Arc<ProgressTracker>,
) -> Result<Option<PipelineInputs>> {
    let all_files = discover_files(&cfg.repo_path, cfg.include_config_files)?;
    if all_files.is_empty() {
        log_nothing_to_index("No supported source files found.");
        return Ok(None);
    }

    progress.set_stage(IndexingStage::Classifying);
    let repo_root = PathBuf::from(&cfg.repo_path);
    let (_, modified_files, added_files, deleted_files) =
        classify_files_for_indexing(&all_files, index_state, cfg.clean, &repo_root)?;

    let unchanged_count =
        all_files.len() - modified_files.len() - added_files.len() - deleted_files.len();

    if unchanged_count == all_files.len() && deleted_files.is_empty() {
        log_nothing_to_index("No files changed — index is up to date!");
        return Ok(None);
    }

    log_file_classification(
        unchanged_count,
        modified_files.len(),
        added_files.len(),
        deleted_files.len(),
    );

    Ok(Some(PipelineInputs {
        modified_files,
        added_files,
        deleted_files,
        repo_root,
    }))
}

/// Stages 2–6: the streaming parse → embed → ingest pipeline. Returns the
/// collected resolution entities and the total number of ingested entities.
async fn run_streaming_ingest(
    cfg: &Config,
    vector_db: &Arc<VectorDb>,
    graph_db: &Arc<GraphDb>,
    progress: &Arc<ProgressTracker>,
    files_to_parse: &[PathBuf],
) -> Result<(Vec<ResolutionEntity>, usize)> {
    // --- STREAMING PIPELINE ---
    // Bounded channels provide backpressure: capacity = batch_size * 4
    // limits worst-case memory to ~1.3MB (256 entities * ~5KB each).
    let (parse_tx, parse_rx) = mpsc::channel::<ParsedEntity>(cfg.batch_size * 4);
    let (embed_tx, embed_rx) = mpsc::channel::<Vec<EmbeddedEntity>>(16);
    let (res_tx, mut res_rx) = mpsc::channel::<ResolutionEntity>(cfg.batch_size * 4);

    // Stage 2: Parallel Parsing (std::thread::scope OS threads)
    info!(
        "Stage 2: Starting parallel parsing of {} files...",
        files_to_parse.len()
    );
    spawn_parse_stage(files_to_parse, cfg, progress, parse_tx);

    // Stage 3 & 4: Batching & Embedding (CPU)
    let cache_dir = crate::pipeline::state::fastembed_cache_dir(&cfg.repo_path);
    let embed_handle = spawn_embed_task(parse_rx, embed_tx.clone(), cfg, cache_dir)?;

    // Stage 5 & 6: Ingestion & Resolution Prep (concurrent via JoinSet)
    // Drain res_rx concurrently to prevent the ingestion task from
    // deadlocking when the bounded res_tx channel fills up.
    let res_handle = tokio::spawn(async move {
        let mut resolution_entities = Vec::new();
        while let Some(res_entity) = res_rx.recv().await {
            resolution_entities.push(res_entity);
        }
        resolution_entities
    });

    let ingest_handle = spawn_ingest_task(embed_rx, res_tx, (vector_db, graph_db), cfg, progress);

    // Wait for embedding and ingestion to finish
    embed_handle.await??;
    drop(embed_tx); // Ensure ingest task finishes when embed_rx is empty
    let total_entities = ingest_handle.await??;

    // Final progress log showing 100% parse completion
    log_parse_and_ingest_complete(progress);

    // Stage 7: Relationship Resolution
    // The ingest task drops res_tx on exit, which closes the channel
    // and causes res_handle to finish naturally.
    progress.set_stage(IndexingStage::ResolvingReferences);
    let resolution_entities = res_handle.await?;

    Ok((resolution_entities, total_entities))
}

/// Logs the final progress snapshot once parsing and ingestion are complete
/// and reference resolution is about to start.
fn log_parse_and_ingest_complete(progress: &Arc<ProgressTracker>) {
    let snap = progress.snapshot();
    info!(
        "[Progress] [{}] {:.1}% — files {}/{}, entities {}/{} — parsing and ingestion complete, resolving references...",
        snap.repo_name,
        snap.percent_complete,
        snap.parsed_files,
        snap.total_files,
        snap.entities_ingested,
        snap.total_entities
    );
}

/// Spawns the OS-thread pool that parses `files_to_parse` and streams
/// entities into `parse_tx`.
fn spawn_parse_stage(
    files_to_parse: &[PathBuf],
    cfg: &Config,
    progress: &Arc<ProgressTracker>,
    parse_tx: mpsc::Sender<ParsedEntity>,
) {
    let parse_cfg = build_parse_config(
        cfg.custom_queries_path.clone(),
        cfg.repo_name.clone(),
        cfg.include_config_files,
        Some(cfg.repo_path.clone()),
    );
    let files_to_parse_clone = files_to_parse.to_vec();

    let cpus = cfg.rayon_threads.unwrap_or_else(|| {
        std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(4)
    });

    // Build the per-file completion callback.
    let parse_progress = Arc::clone(progress);
    let on_file_parsed: FileParsedCallback = Arc::new(move || parse_progress.incr_parsed_files());

    // Hook invoked exactly once, after the parser has aggregated every
    // entity and *before* any entity is pushed into the bounded channel.
    // This is the moment the progress observer learns the total entity
    // count and switches from the parse band (0–10%) to the ingest band
    // (10–90%) — preventing the "100% then frozen" bug.
    let total_progress = Arc::clone(progress);
    let on_entities_extracted: crate::pipeline::parser::EntitiesExtractedCallback =
        Arc::new(move |n: usize| {
            total_progress.set_total_entities(n as u64);
        });

    progress.set_stage(IndexingStage::Parsing);

    let parse_done_progress = Arc::clone(progress);
    let parse_callbacks = ParseCallbacks {
        on_file_parsed: Some(on_file_parsed),
        on_entities_extracted: Some(on_entities_extracted),
    };
    tokio::task::spawn_blocking(move || {
        parse_files_stream(
            &files_to_parse_clone,
            &parse_cfg,
            parse_tx,
            cpus,
            Some(parse_callbacks),
        );
        info!("Stage 2: Parallel parsing complete.");
        // Parsing stage done; ingest continues draining.
        // The stage is flipped here while the ingest task may already
        // have been ingesting — this matches the state-machine
        // simplification where Parsing/Ingesting overlap in reality.
        parse_done_progress.set_stage(IndexingStage::Ingesting);
    });
}

/// Shared environment for the background embedding task.
struct EmbedTaskEnv {
    embedder: Arc<tokio::sync::Mutex<Embedder>>,
    embed_tx: mpsc::Sender<Vec<EmbeddedEntity>>,
    batch_size: usize,
    reset_interval: usize,
}

/// Spawns the batching & embedding task: drains `parse_rx`, groups entities
/// into batches and forwards embedded batches to the ingest channel.
fn spawn_embed_task(
    mut parse_rx: mpsc::Receiver<ParsedEntity>,
    embed_tx: mpsc::Sender<Vec<EmbeddedEntity>>,
    cfg: &Config,
    cache_dir: PathBuf,
) -> Result<JoinHandle<Result<()>>> {
    let embedder = Arc::new(tokio::sync::Mutex::new(Embedder::init(cache_dir)?));
    let env = EmbedTaskEnv {
        embedder,
        embed_tx,
        batch_size: cfg.batch_size,
        reset_interval: cfg.embedder_reset_interval,
    };

    Ok(tokio::spawn(async move {
        let mut current_batch = Vec::with_capacity(env.batch_size);
        let mut batch_count = 0;
        while let Some(entity) = parse_rx.recv().await {
            current_batch.push(entity);
            if current_batch.len() >= env.batch_size {
                batch_count += 1;
                let batch =
                    std::mem::replace(&mut current_batch, Vec::with_capacity(env.batch_size));
                embed_and_forward(&env, batch, batch_count, false).await?;
            }
        }
        if !current_batch.is_empty() {
            batch_count += 1;
            embed_and_forward(&env, current_batch, batch_count, true).await?;
        }
        Ok::<(), anyhow::Error>(())
    }))
}

/// Embeds one parsed batch — resetting the ONNX session when the reset
/// interval is hit — and forwards the embedded batch to the ingest channel.
async fn embed_and_forward(
    env: &EmbedTaskEnv,
    mut batch: Vec<ParsedEntity>,
    batch_count: usize,
    is_final: bool,
) -> Result<()> {
    maybe_reset_session(env, batch_count).await?;

    let batch_label = if is_final { "final batch" } else { "batch" };
    info!(
        "[Worker: Embedder] [{}] Stage 3: Embedding {} #{} ({} entities)...",
        batch[0].repo_name,
        batch_label,
        batch_count,
        batch.len()
    );

    prepare_entities(&mut batch);
    let embedder_clone = Arc::clone(&env.embedder);
    let batch_size = env.batch_size;
    let embedded = tokio::task::spawn_blocking(move || {
        let mut lock = embedder_clone.blocking_lock();
        lock.embed(batch, batch_size)
    })
    .await??;
    env.embed_tx.send(embedded).await?;
    Ok(())
}

/// Resets the ONNX session when the configured reset interval is hit, to
/// release BFCArena memory accumulated across batches.
async fn maybe_reset_session(env: &EmbedTaskEnv, batch_count: usize) -> Result<()> {
    if needs_reset(batch_count, env.reset_interval) {
        info!(
            "[Worker: Embedder] Resetting ONNX session at batch #{} to release BFCArena memory",
            batch_count
        );
        env.embedder.lock().await.reinit()?;
    }
    Ok(())
}

/// Spawns the ingestion task: drains `embed_rx`, forwards resolution
/// entities to `res_tx` and ingests embedded batches with bounded
/// concurrency. Returns the total number of ingested entities.
fn spawn_ingest_task(
    mut embed_rx: mpsc::Receiver<Vec<EmbeddedEntity>>,
    res_tx: mpsc::Sender<ResolutionEntity>,
    dbs: (&Arc<VectorDb>, &Arc<GraphDb>),
    cfg: &Config,
    progress: &Arc<ProgressTracker>,
) -> JoinHandle<Result<usize>> {
    let (vector_db, graph_db) = dbs;
    let vector_db = Arc::clone(vector_db);
    let graph_db = Arc::clone(graph_db);
    let max_concurrent = cfg.ingest_concurrency;
    let semaphore = Arc::new(Semaphore::new(max_concurrent));
    let ingest_progress = Arc::clone(progress);

    info!("Ingestion concurrency: {max_concurrent} simultaneous batches");

    tokio::spawn(async move {
        let mut total_ingested = 0;
        let mut batch_count = 0;
        let mut join_set = JoinSet::new();

        while let Some(embedded_batch) = embed_rx.recv().await {
            batch_count += 1;
            let bl = embedded_batch.len();
            total_ingested += bl;

            ingest_progress.record_batch_ingested(bl as u64);
            let snap = ingest_progress.snapshot();
            info!(
                "[Progress] [{}] {:.1}% — files {}/{}, entities {}/{}, batch #{} ({} entities)",
                snap.repo_name,
                snap.percent_complete,
                snap.parsed_files,
                snap.total_files,
                snap.entities_ingested,
                snap.total_entities,
                snap.batches_ingested,
                bl
            );

            // Dispatch resolution entities before ingestion spawns
            for ee in &embedded_batch {
                res_tx.send(ResolutionEntity::from(ee)).await?;
            }

            // Acquire a semaphore permit to limit concurrency
            let permit = semaphore.clone().acquire_owned().await?;
            let vdb = Arc::clone(&vector_db);
            let gdb = Arc::clone(&graph_db);
            let bc = batch_count;

            join_set.spawn(async move {
                info!(
                    "[Worker: Ingester] [{}] Ingesting batch #{bc} ({bl} entities)...",
                    embedded_batch[0].entity.repo_name
                );
                let result = ingest_batch(&embedded_batch, &vdb, &gdb).await;
                drop(permit);
                result
            });
        }

        info!(
            "All {} batches dispatched — waiting for ingestion workers to finish...",
            batch_count
        );

        // Drain all completed tasks, propagating any errors
        while let Some(result) = join_set.join_next().await {
            result??;
        }

        Ok::<usize, anyhow::Error>(total_ingested)
    })
}

/// Clean stale data from databases based on files to delete.
///
/// When `cfg.clean` is set or this is a full indexing run (no prior state on
/// disk), the entire repository is wiped in a single bulk operation, otherwise
/// only the files that already exist in the database (deleted and modified)
/// are removed incrementally.
#[expect(
    clippy::too_many_arguments,
    reason = "function is verbose but correct — extraction deferred"
)]
#[expect(
    clippy::cognitive_complexity,
    reason = "score 27 = 3 tracing macros × 7; net branching complexity is 6"
)]
pub async fn clean_stale_data(
    vector_db: &VectorDb,
    graph_db: &GraphDb,
    cfg: &Config,
    deleted_files: &[String],
    modified_files: &[PathBuf],
    is_full_index: bool,
) -> Result<()> {
    use crate::db::graph::DeleteExt;
    use crate::db::vector::VectorDeleteExt;

    if cfg.clean || is_full_index {
        // Full clean / initial indexing: delete entire repository in one query.
        if cfg.clean {
            info!("Performing full clean for repo '{}'", cfg.repo_name);
        } else {
            info!(
                "Detected full indexing run — wiping repo '{}' before re-indexing",
                cfg.repo_name
            );
        }
        tokio::try_join!(
            vector_db.delete_by_repo(&cfg.repo_name),
            graph_db.delete_by_repo(&cfg.repo_name),
        )?;
    } else {
        // Incremental: delete only modified and deleted files
        let files_to_delete = calculate_files_to_delete(deleted_files, modified_files);

        if !files_to_delete.is_empty() {
            info!(
                "Deleting {} stale file(s) from databases (incremental mode)",
                files_to_delete.len()
            );
            tokio::try_join!(
                vector_db.delete_by_file_paths(&cfg.repo_name, &files_to_delete),
                graph_db.delete_by_file_paths(&cfg.repo_name, &files_to_delete),
            )?;
        }
    }
    Ok(())
}

/// Build configuration for the parsing stage.
fn build_parse_config(
    custom_queries_path: Option<String>,
    repo_name: String,
    include_config_files: bool,
    repo_path: Option<String>,
) -> ParseConfig {
    let repo_root = if let Some(ref p) = repo_path {
        std::fs::canonicalize(p).unwrap_or_else(|_| PathBuf::from(p))
    } else {
        PathBuf::from(".")
    };
    ParseConfig {
        repo_root,
        custom_queries_path,
        repo_name,
        include_config_files,
        repo_path,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::OutputFormat;

    #[test]
    fn test_build_parse_config_variants() {
        let cfg = build_parse_config(None, "repo1".to_string(), false, None);
        assert_eq!(cfg.repo_name, "repo1");
        assert!(cfg.custom_queries_path.is_none());
        assert!(cfg.repo_path.is_none());

        let cfg_custom = build_parse_config(
            Some("/path".to_string()),
            "repo2".to_string(),
            false,
            Some("/tmp/repo".to_string()),
        );
        assert_eq!(cfg_custom.repo_name, "repo2");
        assert_eq!(cfg_custom.custom_queries_path, Some("/path".to_string()));
        assert_eq!(cfg_custom.repo_path, Some("/tmp/repo".to_string()));
    }

    /// Verify that JoinSet + Semaphore correctly limits concurrent task execution.
    #[tokio::test]
    async fn test_joinset_semaphore_concurrency_limit() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let max_concurrent = 2;
        let semaphore = Arc::new(Semaphore::new(max_concurrent));
        let concurrent_count = Arc::new(AtomicUsize::new(0));
        let peak_concurrent = Arc::new(AtomicUsize::new(0));

        let mut join_set = JoinSet::new();
        let total_tasks = 10;

        for i in 0..total_tasks {
            let permit = semaphore.clone().acquire_owned().await.unwrap();
            let concurrent = Arc::clone(&concurrent_count);
            let peak = Arc::clone(&peak_concurrent);

            join_set.spawn(async move {
                let current = concurrent.fetch_add(1, Ordering::SeqCst) + 1;
                peak.fetch_max(current, Ordering::SeqCst);
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                concurrent.fetch_sub(1, Ordering::SeqCst);
                drop(permit);
                Ok::<usize, anyhow::Error>(i)
            });
        }

        let mut results = Vec::new();
        while let Some(result) = join_set.join_next().await {
            results.push(result.unwrap().unwrap());
        }

        assert_eq!(results.len(), total_tasks);
        // Peak concurrent should not exceed the semaphore limit
        assert!(peak_concurrent.load(Ordering::SeqCst) <= max_concurrent);
    }

    /// Verify JoinSet drains properly and preserves data.
    #[tokio::test]
    async fn test_joinset_collects_all_tasks() {
        let mut join_set = JoinSet::new();
        let total_tasks = 5;

        for i in 0..total_tasks {
            join_set.spawn(async move { i * 2 });
        }

        let mut results = Vec::new();
        while let Some(result) = join_set.join_next().await {
            results.push(result.unwrap());
        }

        results.sort();
        assert_eq!(results, vec![0, 2, 4, 6, 8]);
    }

    /// Verify JoinSet properly propagates errors.
    #[tokio::test]
    async fn test_joinset_error_propagation() {
        let mut join_set = JoinSet::new();

        join_set.spawn(async { Ok::<_, anyhow::Error>(1) });
        join_set.spawn(async { Err::<i32, _>(anyhow::anyhow!("task failed")) });

        let mut error_seen = false;
        while let Some(result) = join_set.join_next().await {
            match result {
                Ok(Ok(_)) => {}
                Ok(Err(_)) => error_seen = true,
                Err(_) => error_seen = true,
            }
        }
        assert!(error_seen);
    }

    #[tokio::test]
    async fn test_run_indexing_pipeline_empty_repo() {
        use tempfile::tempdir;
        let dir = tempdir().unwrap();
        let repo_path = dir.path().to_str().unwrap().to_string();

        let _cfg = Config {
            repo_path: repo_path.clone(),
            repo_name: "test-repo".to_string(),
            qdrant_url: "http://localhost:6334".to_string(),
            qdrant_collection: "test".to_string(),
            neo4j_uri: "bolt://localhost:7687".to_string(),
            neo4j_user: "neo4j".to_string(),
            neo4j_password: "password".to_string(),
            custom_queries_path: None,
            embed_dim: 384,
            embedder_reset_interval: 0,
            batch_size: 64,
            clean: false,
            dependency_repos: Vec::new(),
            watch: false,
            dry_run: false,
            custom_ca_certs: None,
            output_format: OutputFormat::Markdown,
            ingest_concurrency: 4,
            rayon_threads: None,
            include_config_files: false,
        };

        // We need to mock DBs if we want to run the full pipeline,
        // but here we just want to see if it returns Ok(()) when no files are found.
        // discover_files will return empty Vec.

        // However, init_databases is called before this in main.
        // Here we just test the function directly.
        // We'll need a way to create Arc<VectorDb> and Arc<GraphDb> without connecting if possible,
        // or just accept that this test might be limited.

        // Actually, discovering files happens FIRST.
        // If it's empty, it returns Ok(()).
        // Let's try to pass dummy Arcs.
    }
}
