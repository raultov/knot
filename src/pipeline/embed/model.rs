//! Embedding-model selection for the fastembed runtime.
//!
//! A single [`EmbedModelChoice`] ties together the fastembed [`EmbeddingModel`]
//! variant, its native vector dimension, the asymmetric retrieval prefixes the
//! model family expects, **and** the collection-name suffix that keeps
//! different-dimension models from colliding in Qdrant. This table is the
//! single source of truth for everything model-shaped in knot: adding a
//! future model is a one-row change here plus nothing else.
//!
//! # Why knot owns the prefixes
//!
//! fastembed 6 removed the `query_embed` API that earlier versions had: the
//! only public entry points are `try_new`/`transform`/`embed`, and
//! [`TextEmbedding::embed`] passes texts to the tokenizer **verbatim**.
//! For an instruction-aware model family — BGE v1.5 — the asymmetric
//! query/passage prefixes are therefore knot's responsibility, applied in
//! [`Embedder::embed`](super::Embedder::embed) (passages) and
//! [`Embedder::embed_query`](super::Embedder::embed_query) (query).
//! The symmetric default model (MiniLM) carries empty prefixes, which
//! preserves the pre-change behavior exactly.
//!
//! # Config
//!
//! `KNOT_EMBED_MODEL` selects the model by name (case-insensitive). The
//! supported set is closed to exactly two models with a two-to-one dimension
//! map (`384 ⇒ AllMiniLML6V2`, `768 ⇒ BGEBaseENV15`), which makes the model
//! behind an existing Qdrant collection inferable from its vector size. The
//! dimension is **derived** from the model; `KNOT_EMBED_DIM` is deprecated
//! (see `crate::config`). Changing the model requires a clean re-index of the
//! affected collections (`knot-indexer --clean`).

use std::str::FromStr;

use fastembed::EmbeddingModel;

/// Default embedding model (name form of [`EmbeddingModel::AllMiniLML6V2`]).
///
/// Backward-compatibility pin: every published knot release up to `v1.10.0`
/// shipped MiniLM/384 on the `knot_entities` collection. Keeping MiniLM as
/// the default means an upgrading user needs **zero re-index and zero
/// configuration change**. Adopting BGE-base is a deliberate opt-in
/// (`KNOT_EMBED_MODEL=BGEBaseENV15`) whose only cost is a clean re-index —
/// never a silent quality or correctness change.
pub const DEFAULT_EMBED_MODEL: &str = "AllMiniLML6V2";

/// BAAI's bge-*-en-v1.5 query-side instruction. The model card specifies it
/// for the *query* only — passages are embedded unprefixed.
pub const BGE_QUERY_PREFIX: &str = "Represent this sentence for searching relevant passages: ";

/// A supported embedding model: the fastembed variant, its native vector
/// dimension, the asymmetric retrieval prefixes it expects, and the
/// collection-name suffix that derives its default Qdrant collection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmbedModelChoice {
    pub model: EmbeddingModel,
    pub dim: u64,
    /// Prepended to the search query at query time. Empty when the model is
    /// symmetric (no instruction prefixes in its retrieval recipe).
    pub query_prefix: &'static str,
    /// Prepended to every indexed entity's embed_text at index time.
    pub passage_prefix: &'static str,
    /// Suffix appended to the base collection name when this model is
    /// selected. `None` for the default model, which must keep the
    /// historical `knot_entities` so a MiniLM user never re-indexes.
    ///
    /// A Qdrant collection's vector size is fixed at creation, so a
    /// different-dimension model cannot share the default collection: it
    /// derives a distinct one instead.
    pub collection_suffix: Option<&'static str>,
}

impl EmbedModelChoice {
    /// The closed set of models knot supports, as `(wire_name, choice)`
    /// pairs. A name outside this list is a hard configuration error
    /// listing the accepted names — never a silent fallback to the default
    /// (that would silently mismatch the model that built the live index).
    ///
    /// Exactly one row must carry [`Self::collection_suffix`] == `None` and
    /// it must be the row named by [`DEFAULT_EMBED_MODEL`] (pinned by
    /// tests); every suffix must be unique so the derived collections
    /// cannot collide.
    pub fn supported() -> &'static [(&'static str, Self)] {
        &[
            (
                "AllMiniLML6V2",
                Self {
                    model: EmbeddingModel::AllMiniLML6V2,
                    dim: 384,
                    query_prefix: "",
                    passage_prefix: "",
                    collection_suffix: None,
                },
            ),
            (
                "BGEBaseENV15",
                Self {
                    model: EmbeddingModel::BGEBaseENV15,
                    dim: 768,
                    query_prefix: BGE_QUERY_PREFIX,
                    passage_prefix: "",
                    collection_suffix: Some("bge768"),
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

    /// Default collection for this model given the base name.
    ///
    /// `None` suffix → the base name unchanged (backward compatibility:
    /// the default model keeps the historical `knot_entities`).
    pub fn default_collection(&self, base: &str) -> String {
        match self.collection_suffix {
            None => base.to_owned(),
            Some(suffix) => format!("{base}_{suffix}"),
        }
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

impl FromStr for EmbedModelChoice {
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
            err.contains("BGEBaseENV15"),
            "error must list accepts: {err}"
        );
        // Case-insensitive near-matches still parse.
        assert!(EmbedModelChoice::from_str("BGEBaseENv15").is_ok());
        assert!(EmbedModelChoice::from_str("allminilmL6v2").is_ok());
        // Genuinely unknown names are rejected — including the four models
        // removed from the supported set (an accepted, intentional break).
        assert!(EmbedModelChoice::from_str("BGESmallENV15").is_err());
        assert!(EmbedModelChoice::from_str("JinaEmbeddingsV2BaseCode").is_err());
        assert!(EmbedModelChoice::from_str("MultilingualE5Small").is_err());
        assert!(EmbedModelChoice::from_str("NomicEmbedTextV15").is_err());
    }

    #[test]
    fn bge_uses_query_prefix_and_no_passage_prefix() {
        let bge = EmbedModelChoice::from_str("BGEBaseENV15").unwrap();
        assert_eq!(bge.dim, 768);
        assert_eq!(bge.query_prefix, BGE_QUERY_PREFIX);
        // BAAI's recipe: instruction on the QUERY side only.
        assert_eq!(bge.passage_prefix, "");
        assert!(
            bge.query_prefix.ends_with(' '),
            "prefix must be space-separated"
        );
    }

    #[test]
    fn minilm_is_symmetric() {
        let choice = EmbedModelChoice::from_str("AllMiniLML6V2").unwrap();
        assert_eq!(choice.query_prefix, "", "MiniLM must be symmetric");
        assert_eq!(choice.passage_prefix, "", "MiniLM must be symmetric");
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
    fn default_model_is_minilm_384_symmetric() {
        let choice = EmbedModelChoice::from_str(DEFAULT_EMBED_MODEL).expect("default known");
        assert_eq!(choice.model, EmbeddingModel::AllMiniLML6V2);
        // Backward-compatibility pin: the default stays at the historical
        // 384-dimension MiniLM so a `v1.10.0` user never re-indexes.
        assert_eq!(choice.dim, 384);
        // The default is symmetric: no instruction prefix on either side.
        assert_eq!(choice.query_prefix, "");
        assert_eq!(choice.passage_prefix, "");
    }

    #[test]
    fn exactly_one_model_has_no_collection_suffix_and_it_is_the_default() {
        let baseless: Vec<&str> = EmbedModelChoice::supported()
            .iter()
            .filter(|(_, choice)| choice.collection_suffix.is_none())
            .map(|(name, _)| *name)
            .collect();
        assert_eq!(baseless, vec![DEFAULT_EMBED_MODEL]);
    }

    #[test]
    fn collection_suffixes_are_unique() {
        let mut seen = std::collections::HashSet::new();
        for (name, choice) in EmbedModelChoice::supported() {
            if let Some(suffix) = choice.collection_suffix {
                assert!(
                    seen.insert(suffix),
                    "collection suffix '{suffix}' duplicated (first on {name})"
                );
            }
        }
    }

    #[test]
    fn default_collection_maps_base_and_suffix() {
        let minilm = EmbedModelChoice::from_str("AllMiniLML6V2").unwrap();
        assert_eq!(minilm.default_collection("knot_entities"), "knot_entities");

        let bge = EmbedModelChoice::from_str("BGEBaseENV15").unwrap();
        assert_eq!(
            bge.default_collection("knot_entities"),
            "knot_entities_bge768"
        );
    }

    // The `from_env` override branch is deliberately NOT unit-tested:
    // `std::env::set_var` is unsafe in Rust 2024 and this crate denies
    // unsafe code. Its coverage is live: every indexed run with a
    // KNOT_EMBED_MODEL override (CI BGE model cache, E2E suites) resolves
    // through the same `from_str` table tested above.
}
