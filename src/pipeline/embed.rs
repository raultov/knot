//! Stage 4 — Embed: vector generation via fastembed.
//!
//! Uses the `fastembed` crate (pure-Rust ONNX inference) to embed the
//! `embed_text` of every [`ParsedEntity`] into a high-dimensional vector.
//!
//! The default model is `AllMiniLML6V2` (384-dim, fast, good quality).
//! All entities are embedded in a single batched call to maximise throughput.

use anyhow::Result;
use tracing::info;

use crate::models::{EmbeddedEntity, ParsedEntity};

#[cfg(feature = "indexer")]
use anyhow::Context;

#[cfg(feature = "indexer")]
use fastembed::{EmbeddingModel, InitOptions, TextEmbedding};

/// Embedding model used when no override is configured.
#[cfg(feature = "indexer")]
const DEFAULT_MODEL: EmbeddingModel = EmbeddingModel::AllMiniLML6V2;

/// Wrapper around the fastembed [`TextEmbedding`] model.
#[cfg(feature = "indexer")]
pub struct Embedder {
    model: TextEmbedding,
}

/// Stub implementation of Embedder when compiled without the indexer feature.
///
/// This allows `knot` and `knot-mcp` to compile and run in a lightweight
/// "only-clients" mode that doesn't require ONNX Runtime or fastembed.
/// Structural search tools (find_callers, explore_file) work fine with this stub.
/// Semantic search will gracefully fail with an informative error message.
#[cfg(not(feature = "indexer"))]
pub struct Embedder;

#[cfg(feature = "indexer")]
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

#[cfg(not(feature = "indexer"))]
impl Embedder {
    /// Stub initialiser for the lightweight "only-clients" mode.
    pub fn init() -> Result<Self> {
        info!("Running in lightweight 'only-clients' mode: embedding feature disabled");
        Ok(Self)
    }

    /// Stub embedding for batch entities.
    ///
    /// This returns zero-filled vectors to satisfy the type system and allow
    /// the code to compile. In practice, knot-indexer won't be built in this mode.
    pub fn embed(
        &mut self,
        entities: Vec<ParsedEntity>,
        _batch_size: usize,
    ) -> Result<Vec<EmbeddedEntity>> {
        let embedded: Vec<EmbeddedEntity> = entities
            .into_iter()
            .map(|entity| EmbeddedEntity {
                entity,
                vector: vec![0.0; 384], // Match default AllMiniLML6V2 dimension
            })
            .collect();

        Ok(embedded)
    }

    /// Stub query embedding that returns an informative error.
    ///
    /// When users try to use semantic search in the lightweight build,
    /// they get a clear message about what tools are available.
    pub fn embed_query(&mut self, _query: &str) -> Result<Vec<f32>> {
        Err(anyhow::anyhow!(
            "Semantic search is disabled in the lightweight 'only-clients' build. \
             Please use 'find_callers' or 'explore_file' for structural analysis instead. \
             To enable semantic search, build with: cargo build --release"
        ))
    }
}

#[cfg(all(test, feature = "indexer"))]
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
