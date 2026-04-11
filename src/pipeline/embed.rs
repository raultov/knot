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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{EntityKind, ParsedEntity};

    #[ignore = "Downloads ONNX model (~23MB) and requires significant memory/CPU"]
    #[test]
    fn test_embedder_init_and_embed_basic() {
        let mut embedder = Embedder::init().expect("Failed to init embedder");

        let entity = ParsedEntity::new(
            "TestClass",
            EntityKind::Class,
            "TestClass",
            None,
            None,
            "java",
            "Test.java",
            1,
            None,
            "test-repo",
        );

        let mut entities = vec![entity];
        entities[0].embed_text = "[class] TestClass\nFile: Test.java:1".to_string();

        let results = embedder.embed(entities, 1).expect("Failed to embed");

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].vector.len(), 384); // AllMiniLML6V2 produces 384-dim vectors
    }

    #[ignore = "Downloads ONNX model (~23MB) and requires significant memory/CPU"]
    #[test]
    fn test_embedder_embed_query() {
        let mut embedder = Embedder::init().expect("Failed to init embedder");
        let vector = embedder
            .embed_query("How to implement a singleton in Java?")
            .expect("Failed to embed query");

        assert_eq!(vector.len(), 384);
    }
}
