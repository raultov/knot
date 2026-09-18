//! Embedding-model selection for the fastembed runtime.
//!
//! A single [`EmbedModelChoice`] ties together the fastembed [`EmbeddingModel`]
//! variant, its native vector dimension and the asymmetric retrieval prefixes
//! the model family expects.
//!
//! # Why knot owns the prefixes
//!
//! fastembed 6 removed the `query_embed` API that earlier versions had: the
//! only public entry points are `try_new`/`transform`/`embed`, and
//! [`TextEmbedding::embed`] passes texts to the tokenizer **verbatim**.
//! For an instruction-aware model family — BGE v1.5, E5, Nomic v1.5 — the
//! asymmetric query/passage prefixes are therefore knot's responsibility,
//! applied in [`Embedder::embed`](super::Embedder::embed) (passages) and
//! [`Embedder::embed_query`](super::Embedder::embed_query) (query).
//! Symmetric models (MiniLM, Jina code) carry empty prefixes, which
//! preserves the pre-change behaviour exactly.
//!
//! # Config
//!
//! `KNOT_EMBED_MODEL` selects the model by name (case-insensitive). Changing
//! it invalidates every stored vector: the index must be re-built
//! (`knot-indexer --clean`) AND `KNOT_EMBED_DIM` must match the model's
//! native dimension — see the guard in `crate::config`.

use std::str::FromStr;

use fastembed::EmbeddingModel;

/// Default embedding model (name form of [`EmbeddingModel::BGEBaseENV15`]).
///
/// Flipping this constant is a measured decision, not a code change. It was
/// flipped from `AllMiniLML6V2` once the ranker stopped depending on the
/// model's cosine scale: `search_hybrid_context` normalizes each candidate
/// pool before scoring
/// ([`crate::cli_tools::search_hybrid_context::rank`]), so the boost
/// constants are no longer calibrated to one model's band — the property
/// that had blocked the adoption.
///
/// BGE-base is 768-dimensional, unlike the historical 384: adopting it
/// **requires recreating the Qdrant collection** at 768 and re-indexing
/// every repository, and it changes the `KNOT_EMBED_DIM` default. The
/// index-state bump (`crate::pipeline::state`, v7) forces the re-index on
/// the database of record.
pub const DEFAULT_EMBED_MODEL: &str = "BGEBaseENV15";

/// BAAI's bge-*-en-v1.5 query-side instruction. The model card specifies it
/// for the *query* only — passages are embedded unprefixed.
pub const BGE_QUERY_PREFIX: &str = "Represent this sentence for searching relevant passages: ";

/// E5 family prefixes (the `intfloat/e5` asymmetric retrieval recipe).
pub const E5_QUERY_PREFIX: &str = "query: ";
pub const E5_PASSAGE_PREFIX: &str = "passage: ";

/// Nomic v1.5 prefixes (the model was trained with `task: search |
/// <instruction>` query/passage pairs).
pub const NOMIC_QUERY_PREFIX: &str = "search_query: ";
pub const NOMIC_PASSAGE_PREFIX: &str = "search_document: ";

/// A supported embedding model: the fastembed variant, its native vector
/// dimension, and the asymmetric retrieval prefixes it expects.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmbedModelChoice {
    pub model: EmbeddingModel,
    pub dim: u64,
    /// Prepended to the search query at query time. Empty when the model is
    /// symmetric (no instruction prefixes in its retrieval recipe).
    pub query_prefix: &'static str,
    /// Prepended to every indexed entity's embed_text at index time.
    pub passage_prefix: &'static str,
}

impl EmbedModelChoice {
    /// The closed set of models knot supports, as `(wire_name, choice)`
    /// pairs. A name outside this list is a hard configuration error
    /// listing the accepted names — never a silent fallback to the default
    /// (that would silently mismatch the model that built the live index).
    pub fn supported() -> &'static [(&'static str, Self)] {
        &[
            (
                "AllMiniLML6V2",
                Self {
                    model: EmbeddingModel::AllMiniLML6V2,
                    dim: 384,
                    query_prefix: "",
                    passage_prefix: "",
                },
            ),
            (
                "BGESmallENV15",
                Self {
                    model: EmbeddingModel::BGESmallENV15,
                    dim: 384,
                    query_prefix: BGE_QUERY_PREFIX,
                    passage_prefix: "",
                },
            ),
            (
                "BGEBaseENV15",
                Self {
                    model: EmbeddingModel::BGEBaseENV15,
                    dim: 768,
                    query_prefix: BGE_QUERY_PREFIX,
                    passage_prefix: "",
                },
            ),
            (
                "MultilingualE5Small",
                Self {
                    model: EmbeddingModel::MultilingualE5Small,
                    dim: 384,
                    query_prefix: E5_QUERY_PREFIX,
                    passage_prefix: E5_PASSAGE_PREFIX,
                },
            ),
            (
                "JinaEmbeddingsV2BaseCode",
                Self {
                    model: EmbeddingModel::JinaEmbeddingsV2BaseCode,
                    dim: 768,
                    query_prefix: "",
                    passage_prefix: "",
                },
            ),
            (
                "NomicEmbedTextV15",
                Self {
                    model: EmbeddingModel::NomicEmbedTextV15,
                    dim: 768,
                    query_prefix: NOMIC_QUERY_PREFIX,
                    passage_prefix: NOMIC_PASSAGE_PREFIX,
                },
            ),
        ]
    }

    /// The accepted `KNOT_EMBED_MODEL` names, for error messages and docs.
    pub fn accepted_names() -> String {
        Self::supported()
            .iter()
            .map(|(name, _)| *name)
            .collect::<Vec<_>>()
            .join(", ")
    }

    /// Resolve the model name from `KNOT_EMBED_MODEL`, falling back to
    /// [`DEFAULT_EMBED_MODEL`]. Errors on an unknown name.
    pub fn from_env() -> Result<Self, String> {
        match std::env::var("KNOT_EMBED_MODEL") {
            Ok(name) => Self::from_str(&name),
            Err(_) => Self::from_str(DEFAULT_EMBED_MODEL),
        }
    }
}

impl std::str::FromStr for EmbedModelChoice {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::supported()
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case(s))
            .map(|(_, choice)| choice.clone())
            .ok_or_else(|| {
                format!(
                    "Unknown embedding model '{s}'. Accepted KNOT_EMBED_MODEL values: {}",
                    Self::accepted_names()
                )
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_known_names_case_insensitively() {
        for (name, choice) in EmbedModelChoice::supported() {
            let lower =
                EmbedModelChoice::from_str(&name.to_lowercase()).expect("lowercase name parses");
            assert_eq!(&lower, choice, "case-insensitive parse failed for {name}");
        }
    }

    #[test]
    fn rejects_unknown_name_listing_accepts() {
        let err = EmbedModelChoice::from_str("GPT5").unwrap_err();
        assert!(err.contains("Unknown embedding model 'GPT5'"), "{err}");
        assert!(
            err.contains("AllMiniLML6V2"),
            "error must list accepts: {err}"
        );
        assert!(
            err.contains("BGESmallENV15"),
            "error must list accepts: {err}"
        );
        // Case-insensitive near-matches still parse.
        assert!(EmbedModelChoice::from_str("BGESmallENv15").is_ok());
        assert!(EmbedModelChoice::from_str("BgEsmallenv15").is_ok());
        // Genuinely unknown names are rejected.
        assert!(EmbedModelChoice::from_str("BGESmallEnv18").is_err());
    }

    #[test]
    fn bge_uses_query_prefix_and_no_passage_prefix() {
        let bge = EmbedModelChoice::from_str("BGESmallENV15").unwrap();
        assert_eq!(bge.dim, 384);
        assert_eq!(bge.query_prefix, BGE_QUERY_PREFIX);
        // BAAI's recipe: instruction on the QUERY side only.
        assert_eq!(bge.passage_prefix, "");
        assert!(
            bge.query_prefix.ends_with(' '),
            "prefix must be space-separated"
        );
    }

    #[test]
    fn e5_uses_distinct_query_and_passage_prefixes() {
        let e5 = EmbedModelChoice::from_str("MultilingualE5Small").unwrap();
        assert_eq!(e5.dim, 384);
        assert_ne!(e5.query_prefix, e5.passage_prefix);
        assert_eq!(e5.query_prefix, E5_QUERY_PREFIX);
        assert_eq!(e5.passage_prefix, E5_PASSAGE_PREFIX);
    }

    #[test]
    fn nomic_uses_distinct_query_and_passage_prefixes() {
        let nomic = EmbedModelChoice::from_str("NomicEmbedTextV15").unwrap();
        assert_eq!(nomic.dim, 768);
        assert_ne!(nomic.query_prefix, nomic.passage_prefix);
        assert_eq!(nomic.query_prefix, NOMIC_QUERY_PREFIX);
        assert_eq!(nomic.passage_prefix, NOMIC_PASSAGE_PREFIX);
    }

    #[test]
    fn minilm_and_jina_are_symmetric() {
        for name in ["AllMiniLML6V2", "JinaEmbeddingsV2BaseCode"] {
            let choice = EmbedModelChoice::from_str(name).unwrap();
            assert_eq!(choice.query_prefix, "", "{name} must be symmetric");
            assert_eq!(choice.passage_prefix, "", "{name} must be symmetric");
        }
    }

    #[test]
    fn every_choice_dim_matches_fastembed_model_info() {
        // Drift guard: if a fastembed upgrade changes a model's dimension,
        // this fails BEFORE the mismatch reaches Qdrant. Pure metadata
        // lookup — no model download.
        for (name, choice) in EmbedModelChoice::supported() {
            let info = fastembed::TextEmbedding::get_model_info(&choice.model)
                .unwrap_or_else(|e| panic!("fastembed metadata for {name}: {e}"));
            assert_eq!(
                u64::try_from(info.dim).expect("non-negative dim"),
                choice.dim,
                "declared dim for {name} drifted from fastembed"
            );
        }
    }

    #[test]
    fn default_model_is_bge_base_768_with_query_prefix() {
        let choice = EmbedModelChoice::from_str(DEFAULT_EMBED_MODEL).expect("default known");
        assert_eq!(choice.model, EmbeddingModel::BGEBaseENV15);
        // 768 dims: adopting the default requires a Qdrant collection
        // recreation and a full re-index (pinned against
        // `IndexerCli`/`McpCli` defaults by `config`'s drift guard).
        assert_eq!(choice.dim, 768);
        // The default is asymmetric: the query side MUST carry BAAI's
        // instruction, the passage side must not. A regression here is
        // silent — same dimensions, degraded recall — so it is pinned.
        assert_eq!(choice.query_prefix, BGE_QUERY_PREFIX);
        assert_eq!(choice.passage_prefix, "");
    }

    // The `from_env` override branch is deliberately NOT unit-tested:
    // `std::env::set_var` is unsafe in Rust 2024 and this crate denies
    // unsafe code. Its coverage is live: every indexed run with a
    // KNOT_EMBED_MODEL override (CI BGE model cache, E2E suites) resolves
    // through the same `from_str` table tested above.
}
