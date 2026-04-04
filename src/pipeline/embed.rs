//! Stage 4 — Embed: vector generation via fastembed.
//!
//! Uses the `fastembed` crate (pure-Rust ONNX inference) to embed the
//! `embed_text` of every [`ParsedEntity`] into a high-dimensional vector.
//!
//! The default model is `AllMiniLML6V2` (384-dim, fast, good quality).
//! All entities are embedded in a single batched call to maximise throughput.

use anyhow::{Context, Result};
use fastembed::{EmbeddingModel, InitOptions, TextEmbedding};
use tracing::info;

use crate::models::{EmbeddedEntity, ParsedEntity};

/// Embedding model used when no override is configured.
const DEFAULT_MODEL: EmbeddingModel = EmbeddingModel::AllMiniLML6V2;

/// Wrapper around the fastembed [`TextEmbedding`] model.
pub struct Embedder {
    model: TextEmbedding,
}

impl Embedder {
    /// Initialise the embedding model.
    ///
    /// On first run this will download the ONNX model weights (~23 MB for
    /// AllMiniLML6V2) and cache them locally. Subsequent runs load from cache.
    pub fn init() -> Result<Self> {
        info!("Initialising fastembed model ({DEFAULT_MODEL:?})…");

        let model = TextEmbedding::try_new(
            InitOptions::new(DEFAULT_MODEL).with_show_download_progress(true),
        )
        .context("Failed to initialise fastembed TextEmbedding model")?;

        info!("Embedding model ready");
        Ok(Self { model })
    }

    /// Embed a batch of [`ParsedEntity`] records and return [`EmbeddedEntity`] values.
    ///
    /// `batch_size` controls how many texts are passed to the ONNX runtime at once.
    /// Tuning this trades memory usage against throughput.
    pub fn embed(
        &mut self,
        entities: Vec<ParsedEntity>,
        batch_size: usize,
    ) -> Result<Vec<EmbeddedEntity>> {
        if entities.is_empty() {
            return Ok(vec![]);
        }

        let texts: Vec<&str> = entities.iter().map(|e| e.embed_text.as_str()).collect();

        info!(
            "Embedding {} entities (batch_size={})…",
            texts.len(),
            batch_size
        );

        let vectors = self
            .model
            .embed(texts, Some(batch_size))
            .context("fastembed embedding failed")?;

        debug_assert_eq!(
            vectors.len(),
            entities.len(),
            "Mismatch between entity count and vector count"
        );

        let embedded: Vec<EmbeddedEntity> = entities
            .into_iter()
            .zip(vectors)
            .map(|(entity, vector)| EmbeddedEntity { entity, vector })
            .collect();

        info!("Embedding complete — {} vectors produced", embedded.len());
        Ok(embedded)
    }

    /// Embed a single text query and return the vector.
    ///
    /// This is used by the MCP server for runtime query embedding.
    pub fn embed_query(&mut self, query: &str) -> Result<Vec<f32>> {
        let vectors = self
            .model
            .embed(vec![query], Some(1))
            .context("fastembed query embedding failed")?;

        vectors
            .into_iter()
            .next()
            .ok_or_else(|| anyhow::anyhow!("No vector returned for query"))
    }
}
