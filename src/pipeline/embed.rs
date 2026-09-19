//! Stage 4 — Embed: vector generation via fastembed.
//!
//! Uses the `fastembed` crate (pure-Rust ONNX inference) to embed the
//! `embed_text` of every [`ParsedEntity`] into a high-dimensional vector.
//!
//! The model is selectable via `KNOT_EMBED_MODEL` (see [`model`]); the
//! default is `BGEBaseENV15` (768-dim, asymmetric — the query carries
//! BAAI's instruction prefix, the passage does not). For instruction-aware
//! models, knot applies the asymmetric retrieval prefixes itself — see the
//! `model` module doc for why (fastembed 6 embeds texts verbatim).
//!
//! All entities are embedded in a single batched call to maximize throughput.

use anyhow::Result;
use tracing::info;

use crate::models::{EmbeddedEntity, ParsedEntity};

use anyhow::Context;

use fastembed::{InitOptions, TextEmbedding};

pub mod model;

pub use model::{DEFAULT_EMBED_MODEL, EmbedModelChoice};

/// Wrapper around the fastembed [`TextEmbedding`] model.
pub struct Embedder {
    model: TextEmbedding,
    cache_dir: std::path::PathBuf,
    choice: EmbedModelChoice,
}

impl Embedder {
    pub fn cache_dir_path(p: &std::path::Path) -> std::path::PathBuf {
        p.to_path_buf()
    }

    pub fn reinit(&mut self) -> Result<()> {
        let fresh = TextEmbedding::try_new(
            InitOptions::new(self.choice.model.clone())
                .with_cache_dir(self.cache_dir.clone())
                .with_show_download_progress(false),
        )
        .context("Failed to reinit fastembed TextEmbedding model")?;
        self.model = fresh;
        Ok(())
    }

    /// Initialize the embedding model resolved from the environment
    /// (`KNOT_EMBED_MODEL`, defaulting to `DEFAULT_EMBED_MODEL`).
    ///
    /// Signature kept binary-compatible for library consumers (knot-server
    /// holds `Embedder::init(cache_dir)` from the published crate).
    ///
    /// On first run this will download the ONNX model weights (~440 MB for
    /// the opt-in BGEBaseENV15, ~23 MB for the default AllMiniLML6V2) and
    /// cache them locally. Subsequent runs load from cache.
    pub fn init(cache_dir: std::path::PathBuf) -> Result<Self> {
        let choice = EmbedModelChoice::from_env()
            .map_err(|e| anyhow::anyhow!("Invalid KNOT_EMBED_MODEL: {e}"))?;
        Self::init_with_model(cache_dir, choice)
    }

    /// Initialize with an explicit model choice (used by tests and callers
    /// that already resolved configuration).
    pub fn init_with_model(
        cache_dir: std::path::PathBuf,
        choice: EmbedModelChoice,
    ) -> Result<Self> {
        info!(
            "Initialising fastembed model ({} dim {}) in {}…",
            choice.model,
            choice.dim,
            cache_dir.display()
        );

        std::fs::create_dir_all(&cache_dir).context("Failed to create fastembed cache dir")?;

        let model = TextEmbedding::try_new(
            InitOptions::new(choice.model.clone())
                .with_cache_dir(cache_dir.clone())
                .with_show_download_progress(true),
        )
        .context("Failed to initialise fastembed TextEmbedding model")?;

        info!("Embedding model ready");
        Ok(Self {
            model,
            cache_dir,
            choice,
        })
    }

    /// The resolved model's native vector dimension (for config validation).
    pub fn dim(&self) -> u64 {
        self.choice.dim
    }

    /// The resolved model's wire name (query-time guard reports).
    pub fn model_name(&self) -> &'static str {
        EmbedModelChoice::supported()
            .iter()
            .find(|(_, c)| c == &self.choice)
            .map(|(name, _)| *name)
            .unwrap_or("unknown")
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

        let repo_name = entities[0].repo_name.clone();

        // Passages carry the model's passage prefix; symmetric models keep
        // the texts byte-identical to the pre-change embed_text so existing
        // vector semantics are untouched.
        let passages: Vec<String>;
        let texts: Vec<&str> = if self.choice.passage_prefix.is_empty() {
            entities.iter().map(|e| e.embed_text.as_str()).collect()
        } else {
            passages = entities
                .iter()
                .map(|e| format!("{}{}", self.choice.passage_prefix, e.embed_text))
                .collect();
            passages.iter().map(String::as_str).collect()
        };

        info!(
            "[{repo_name}] Embedding {} entities (batch_size={})…",
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

        info!(
            "[{repo_name}] Embedding complete — {} vectors produced",
            embedded.len()
        );
        Ok(embedded)
    }

    /// Embed a single text query and return the vector.
    ///
    /// This is used by the MCP server for runtime query embedding.
    ///
    /// For asymmetric models the query carries the model's query prefix
    /// (`query: `, `Represent this sentence…`, `search_query: `). fastembed
    /// 6 has no `query_embed` API — `TextEmbedding::embed` passes texts
    /// through verbatim — so the prefix is applied here.
    pub fn embed_query(&mut self, query: &str) -> Result<Vec<f32>> {
        let text = if self.choice.query_prefix.is_empty() {
            query.to_owned()
        } else {
            format!("{}{}", self.choice.query_prefix, query)
        };

        let vectors = self
            .model
            .embed(vec![&text], Some(1))
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
    use std::str::FromStr;

    #[ignore = "Downloads ONNX model (~23MB) and requires significant memory/CPU"]
    #[test]
    fn test_embedder_init_and_embed_basic() {
        let temp_dir = tempfile::tempdir().unwrap();
        let mut embedder =
            Embedder::init(temp_dir.path().to_path_buf()).expect("Failed to init embedder");

        let entity = ParsedEntity::new(
            "TestClass",
            EntityKind::Class,
            "TestClass",
            None,
            None,
            "java",
            "Test.java",
            1,
            10,
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
        let temp_dir = tempfile::tempdir().unwrap();
        let mut embedder =
            Embedder::init(temp_dir.path().to_path_buf()).expect("Failed to init embedder");
        let vector = embedder
            .embed_query("How to implement a singleton in Java?")
            .expect("Failed to embed query");

        assert_eq!(vector.len(), 384);
    }

    /// Asymmetric-prefix proof: for BGE the query embeds with the
    /// instruction prefix and the passage without it, so identical input
    /// text takes two different vectors. Pinned here (behind the ignore)
    /// because the prefix band is glued into `embed_query`/`embed`.
    #[ignore = "Downloads the BGESmallENV15 ONNX model and requires network"]
    #[test]
    fn bge_query_and_passage_vectors_differ_by_prefix() {
        let temp_dir = tempfile::tempdir().unwrap();
        let choice = EmbedModelChoice::from_str("BGESmallENV15").expect("known model");
        assert_eq!(choice.passage_prefix, "", "BGE recipe: query-side only");
        let mut embedder =
            Embedder::init_with_model(temp_dir.path().to_path_buf(), choice).expect("BGE init");

        let query_a = embedder.embed_query("acquire a connection").unwrap();
        // Same text passed as a "passage" via the indexed-text path: build a
        // minimal entity whose embed_text is the same string.
        let mut entity = ParsedEntity::new(
            "get_connection",
            EntityKind::RustFunction,
            "get_connection",
            None,
            None,
            "rust",
            "src/db.rs",
            1,
            5,
            None,
            "test",
        );
        entity.embed_text = "acquire a connection".to_string();
        let embedded = embedder.embed(vec![entity], 1).expect("passage embed");
        let passage_vec = &embedded[0].vector;

        let query_b = embedder.embed_query("acquire a connection").unwrap();
        assert_eq!(query_a.len(), 384);
        assert_eq!(query_a, query_b, "query prefix must be deterministic");
        assert_ne!(
            query_a, *passage_vec,
            "BGE query vector must differ from the same text embedded as a passage"
        );
    }
}

pub fn needs_reset(batch_count: usize, interval: usize) -> bool {
    interval > 0 && batch_count > 0 && batch_count.is_multiple_of(interval)
}

#[cfg(test)]
mod reset_tests {
    use super::*;
    #[test]
    fn test_needs_reset_disabled_when_interval_zero() {
        assert!(!needs_reset(500, 0));
        assert!(!needs_reset(1000, 0));
    }
    #[test]
    fn test_needs_reset_true_exactly_at_interval() {
        assert!(needs_reset(500, 500));
        assert!(needs_reset(1000, 500));
        assert!(needs_reset(250, 250));
    }
    #[test]
    fn test_needs_reset_false_before_interval() {
        assert!(!needs_reset(499, 500));
        assert!(!needs_reset(1, 500));
    }
    #[test]
    fn test_needs_reset_false_between_intervals() {
        assert!(!needs_reset(501, 500));
        assert!(!needs_reset(999, 500));
    }
    #[test]
    fn test_needs_reset_multiples_of_interval() {
        for multiplier in 1..=10 {
            assert!(needs_reset(500 * multiplier, 500));
        }
    }
    #[test]
    fn test_needs_reset_batch_count_zero_never_resets() {
        assert!(!needs_reset(0, 500));
        assert!(!needs_reset(0, 1));
    }
}
