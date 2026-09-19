//! Startup model-consistency guard ("the guard ladder").
//!
//! Every verdict the guard emits is a **pure** function of the configured
//! [`EmbedModelChoice`], the Qdrant collection's real vector dimension and
//! the persisted `:Repository` embed markers — fully unit-testable without
//! databases. The binaries wire the inputs in and act on the result:
//! `Abort` exits non-zero, `WarnPartial` logs at `warn!`.
//!
//! # Why a marker at all
//!
//! A Qdrant collection's vector size is fixed at creation, but a
//! same-dimension model switch would produce **no error at all** — only
//! silently degraded recall (new-model query vectors searching old-model
//! passages). The marker makes that class of failure explicit:
//!
//! - a repository indexed with another model is visible in the graph but
//!   unreachable by semantic search (it lives in another collection);
//! - a missing marker means a legacy index, and with the supported set
//!   closed to two models the collection's dimension infers the model
//!   unambiguously (`384 ⇒ AllMiniLML6V2`, `768 ⇒ BGEBaseENV15`), so a
//!   legacy index is never fail-closed.

use std::str::FromStr as _;

use crate::pipeline::embed::EmbedModelChoice;

/// Wire-up helper used by the binaries (knot, knot-mcp): resolve the
/// configured model, probe the collection's real vector dimension, read the
/// persisted markers, and **act** on the verdict. `WarnPartial` logs at
/// `warn!`; `Abort` surfaces as a hard `Err` (the binaries then exit
/// non-zero). Failures reading the marker data are degraded to a warning —
/// the guard protects against silent model mixing, not against reachable
/// databases going away (the normal connection path reports those).
pub async fn verify_startup(cfg: &crate::config::Config) -> anyhow::Result<()> {
    use crate::db::graph::{ConnectExt as _, GraphDb, RepoQueryExt as _};
    use crate::db::vector::probe_collection_dim;

    let choice =
        EmbedModelChoice::from_str(&cfg.embed_model).map_err(|e| anyhow::anyhow!("{e}"))?;

    let collection_dim = match probe_collection_dim(&cfg.qdrant_url, &cfg.qdrant_collection).await {
        Ok(dim) => dim,
        Err(e) => {
            tracing::warn!(
                "Embed-model guard skipped: could not probe collection '{}' ({e})",
                cfg.qdrant_collection
            );
            return Ok(());
        }
    };

    let markers = match GraphDb::connect(&cfg.neo4j_uri, &cfg.neo4j_user, &cfg.neo4j_password).await
    {
        Ok(db) => db.repo_embed_markers(&[]).await.unwrap_or_else(|e| {
            tracing::warn!("Embed markers unreadable ({e}) — model check skipped");
            Vec::new()
        }),
        Err(e) => {
            tracing::warn!("Neo4j unreachable for the embed-marker check ({e}) — skipped");
            Vec::new()
        }
    };

    let verdict = classify_startup(
        &choice,
        &cfg.embed_model,
        &cfg.qdrant_collection,
        collection_dim,
        &markers,
    );
    match verdict {
        StartupVerdict::Ok => Ok(()),
        StartupVerdict::WarnPartial {
            invisible,
            their_model,
        } => {
            tracing::warn!(
                "Repositories indexed with embedding model '{their_model}' exist in the graph \
                 but are invisible to semantic search from '{}': {}. \
                 Re-index them (`knot-indexer --clean`) with '{}' to include them.",
                cfg.qdrant_collection,
                invisible.join(", "),
                cfg.embed_model
            );
            Ok(())
        }
        StartupVerdict::Abort(msg) => Err(anyhow::anyhow!(msg)),
    }
}

/// The embedding-model marker persisted on a `:Repository` node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoEmbedMarker {
    pub repo_name: String,
    /// `None` for a repository indexed before the marker existed.
    pub embed_model: Option<String>,
    pub embed_dim: Option<u64>,
    pub qdrant_collection: Option<String>,
}

impl RepoEmbedMarker {
    /// Build a marker straight from the Neo4j `:Repository` row parsed with
    /// `coalesce` fallbacks (`''` / `-1` map to `None`).
    pub fn from_row(
        repo_name: String,
        embed_model: String,
        embed_dim: i64,
        qdrant_collection: String,
    ) -> Self {
        Self {
            repo_name,
            embed_model: if embed_model.is_empty() {
                None
            } else {
                Some(embed_model)
            },
            embed_dim: if embed_dim < 0 {
                None
            } else {
                Some(embed_dim as u64)
            },
            qdrant_collection: if qdrant_collection.is_empty() {
                None
            } else {
                Some(qdrant_collection)
            },
        }
    }
}

/// What the guard ladder decided.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StartupVerdict {
    /// Proceed silently.
    Ok,
    /// Some repositories were indexed with another model: they exist in the
    /// graph but cannot be reached by semantic search.
    WarnPartial {
        invisible: Vec<String>,
        their_model: String,
    },
    /// Hard configuration error; the candidate exits non-zero with the
    /// message.
    Abort(String),
}

/// Closed-set inference for a legacy index that carries no marker.
///
/// Returns `None` for a dimension outside the supported set (the marker
/// stays unknown; rule 2 of [`classify_startup`] still catches a genuine
/// contradiction at the collection level).
pub fn infer_model_from_dim(dim: u64) -> Option<&'static str> {
    match dim {
        384 => Some("AllMiniLML6V2"),
        768 => Some("BGEBaseENV15"),
        _ => None,
    }
}

/// Operator-facing wording for the guard's remediation messages.
///
/// The ladder itself is audience-agnostic, but its *advice* is not: knot's
/// binaries read `KNOT_QDRANT_COLLECTION` and re-index with
/// `knot-indexer --clean`, while a library consumer such as knot-server reads
/// `KNOT_SERVER_QDRANT_COLLECTION` and re-indexes through its REST API.
/// Passing the labels in keeps a single implementation of the ladder while the
/// remediation stays actionable for each audience.
#[derive(Debug, Clone, Copy)]
pub struct GuardHints<'a> {
    /// Environment variable that selects the Qdrant collection.
    pub collection_var: &'a str,
    /// Environment variable that selects the embedding model.
    pub embed_model_var: &'a str,
    /// Operator-facing phrase describing how to force a full re-index.
    pub reindex_cmd: &'a str,
}

impl Default for GuardHints<'_> {
    fn default() -> Self {
        Self {
            collection_var: "KNOT_QDRANT_COLLECTION",
            embed_model_var: "KNOT_EMBED_MODEL",
            reindex_cmd: "`knot-indexer --clean`",
        }
    }
}

/// The guard ladder (rules in order — see the plan, §F4.3):
///
/// 1. absent collection → fresh deployment, never blocked;
/// 2. collection dimension mismatch → abort (vector size is fixed at
///    creation, so the configured model cannot ever write valid vectors);
/// 3. markers all without `embed_model` → legacy index, dimension already
///    agreed via rule 2, self-heals on the next index run;
/// 4. every marked repo on another model → abort;
/// 5. some marked repos differ → warn (they are invisible to search);
/// 6. otherwise → ok.
pub fn classify_startup(
    configured: &EmbedModelChoice,
    configured_name: &str,
    collection: &str,
    collection_dim: Option<u64>,
    markers: &[RepoEmbedMarker],
) -> StartupVerdict {
    classify_startup_with_hints(
        GuardContext {
            configured,
            configured_name,
            collection,
            collection_dim,
        },
        markers,
        &GuardHints::default(),
    )
}

/// Target/environment context passed into [`classify_startup_with_hints`].
#[derive(Debug, Clone, Copy)]
pub struct GuardContext<'a> {
    pub configured: &'a EmbedModelChoice,
    pub configured_name: &'a str,
    pub collection: &'a str,
    pub collection_dim: Option<u64>,
}

/// [`classify_startup`] with caller-supplied remediation wording.
pub fn classify_startup_with_hints(
    ctx: GuardContext<'_>,
    markers: &[RepoEmbedMarker],
    hints: &GuardHints<'_>,
) -> StartupVerdict {
    // Rule 1: fresh deployment.
    let Some(stored_dim) = ctx.collection_dim else {
        return StartupVerdict::Ok;
    };

    // Rule 2: the collection is a fixed-size voucher for exactly one model.
    if stored_dim != ctx.configured.dim {
        return StartupVerdict::Abort(format!(
            "Qdrant collection '{collection}' holds {stored_dim}-dimensional vectors but the \
                 configured embedding model '{configured_name}' produces {configured_dim}-dimensional ones. \
                 A collection's vector size is fixed at creation: point {collection_var} at \
                 a different collection (the default for '{configured_name}' is '{default_collection}') or \
                 switch {embed_model_var} back to a model matching {stored_dim}, then \
                 {reindex_cmd} the affected repositories.",
            collection = ctx.collection,
            configured_dim = ctx.configured.dim,
            configured_name = ctx.configured_name,
            default_collection = ctx.configured.default_collection("knot_entities"),
            collection_var = hints.collection_var,
            embed_model_var = hints.embed_model_var,
            reindex_cmd = hints.reindex_cmd
        ));
    }

    // Rule 3: legacy index (no marker) — proceed and self-heal on the next
    // run; the dimension already agreed via rule 2.
    let marked: Vec<&RepoEmbedMarker> =
        markers.iter().filter(|m| m.embed_model.is_some()).collect();
    if marked.is_empty() {
        return StartupVerdict::Ok;
    }

    // Rules 4/5: compare markers' models against the configured one.
    let mismatched: Vec<&RepoEmbedMarker> = marked
        .iter()
        .copied()
        .filter(|m| m.embed_model.as_deref() != Some(ctx.configured_name))
        .collect();

    if mismatched.is_empty() {
        return StartupVerdict::Ok;
    }

    if mismatched.len() == marked.len() {
        return StartupVerdict::Abort(format!(
            "All {} marked repositories were indexed with another embedding model \
             ({}): the configured model is '{configured_name}'. Set {} to the \
             model already in use, or re-index every repository \
             ({}) with '{configured_name}'.",
            marked.len(),
            mismatched[0].embed_model.as_deref().unwrap_or("?"),
            hints.embed_model_var,
            hints.reindex_cmd,
            configured_name = ctx.configured_name,
        ));
    }

    StartupVerdict::WarnPartial {
        invisible: mismatched.iter().map(|m| m.repo_name.clone()).collect(),
        their_model: mismatched[0].embed_model.clone().unwrap_or_default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn marker(repo: &str, model: Option<&str>) -> RepoEmbedMarker {
        RepoEmbedMarker {
            repo_name: repo.to_string(),
            embed_model: model.map(String::from),
            embed_dim: model.map(|_| 384),
            qdrant_collection: model.map(|_| "knot_entities".to_string()),
        }
    }

    fn minilm() -> (EmbedModelChoice, &'static str) {
        let choice = EmbedModelChoice::from_str("AllMiniLML6V2").expect("default model known");
        (choice, "AllMiniLML6V2")
    }

    #[test]
    fn infer_model_from_dim_covers_the_closed_set() {
        assert_eq!(infer_model_from_dim(384), Some("AllMiniLML6V2"));
        assert_eq!(infer_model_from_dim(768), Some("BGEBaseENV15"));
        assert_eq!(infer_model_from_dim(1024), None);
        assert_eq!(infer_model_from_dim(0), None);
    }

    #[test]
    fn rule_1_absent_collection_is_ok() {
        let (choice, name) = minilm();
        assert_eq!(
            classify_startup(&choice, name, "knot_entities", None, &[]),
            StartupVerdict::Ok
        );
    }

    #[test]
    fn rule_2_dimension_mismatch_aborts_with_counters() {
        let bge = EmbedModelChoice::from_str("BGEBaseENV15").expect("opt-in model known");
        let verdict = classify_startup(&bge, "BGEBaseENV15", "knot_entities", Some(384), &[]);
        match &verdict {
            StartupVerdict::Abort(msg) => {
                assert!(msg.contains("knot_entities"), "{msg}");
                assert!(msg.contains("384"), "{msg}");
                assert!(msg.contains("768"), "{msg}");
                assert!(msg.contains("BGEBaseENV15"), "{msg}");
                assert!(msg.contains("--clean"), "{msg}");
            }
            other => panic!("expected Abort, got {other:?}"),
        }
    }

    #[test]
    fn rule_3_legacy_markers_with_no_model_are_ok() {
        let (choice, name) = minilm();
        assert_eq!(
            classify_startup(
                &choice,
                name,
                "knot_entities",
                Some(384),
                &[marker("legacy-a", None), marker("legacy-b", None)],
            ),
            StartupVerdict::Ok
        );
    }

    #[test]
    fn rule_4_every_marked_repo_on_another_model_aborts() {
        let (choice, name) = minilm();
        let verdict = classify_startup(
            &choice,
            name,
            "knot_entities",
            Some(384),
            &[
                marker("a", Some("BGEBaseENV15")),
                marker("b", Some("BGEBaseENV15")),
            ],
        );
        match &verdict {
            StartupVerdict::Abort(msg) => {
                assert!(msg.contains("BGEBaseENV15"), "{msg}");
                assert!(msg.contains("AllMiniLML6V2"), "{msg}");
            }
            other => panic!("expected Abort, got {other:?}"),
        }
    }

    #[test]
    fn rule_5_some_marked_repos_differ_warn_and_name_them() {
        let (choice, name) = minilm();
        let verdict = classify_startup(
            &choice,
            name,
            "knot_entities",
            Some(384),
            &[
                marker("ok-repo", Some("AllMiniLML6V2")),
                marker("other-repo", Some("BGEBaseENV15")),
                marker("legacy-repo", None),
            ],
        );
        match verdict {
            StartupVerdict::WarnPartial {
                invisible,
                their_model,
            } => {
                assert_eq!(invisible, vec!["other-repo".to_string()]);
                assert_eq!(their_model, "BGEBaseENV15");
            }
            other => panic!("expected WarnPartial, got {other:?}"),
        }
    }

    #[test]
    fn rule_6_all_marked_repos_agree_is_ok() {
        let (choice, name) = minilm();
        assert_eq!(
            classify_startup(
                &choice,
                name,
                "knot_entities",
                Some(384),
                &[marker("a", Some("AllMiniLML6V2")), marker("b", None)],
            ),
            StartupVerdict::Ok
        );
    }

    #[test]
    fn custom_hints_replace_the_default_remediation_wording() {
        // H1: a library consumer (knot-server) must not be told to set knot's
        // variable or run knot-indexer. The ladder is identical; only the
        // advice changes.
        let hints = GuardHints {
            collection_var: "KNOT_SERVER_QDRANT_COLLECTION",
            embed_model_var: "KNOT_EMBED_MODEL",
            reindex_cmd: "`POST /api/repos/{id}/sync`",
        };
        let bge = EmbedModelChoice::from_str("BGEBaseENV15").expect("opt-in model known");
        match classify_startup_with_hints(
            GuardContext {
                configured: &bge,
                configured_name: "BGEBaseENV15",
                collection: "knot_entities",
                collection_dim: Some(384),
            },
            &[],
            &hints,
        ) {
            StartupVerdict::Abort(msg) => {
                assert!(msg.contains("KNOT_SERVER_QDRANT_COLLECTION"), "{msg}");
                assert!(msg.contains("POST /api/repos/{id}/sync"), "{msg}");
                assert!(!msg.contains("knot-indexer"), "{msg}");
            }
            other => panic!("expected Abort, got {other:?}"),
        }

        let (minilm, name) = minilm();
        let markers = [marker("a", Some("BGEBaseENV15"))];
        match classify_startup_with_hints(
            GuardContext {
                configured: &minilm,
                configured_name: name,
                collection: "knot_entities",
                collection_dim: Some(384),
            },
            &markers,
            &hints,
        ) {
            StartupVerdict::Abort(msg) => {
                assert!(msg.contains("KNOT_EMBED_MODEL"), "{msg}");
                assert!(msg.contains("POST /api/repos/{id}/sync"), "{msg}");
                assert!(!msg.contains("knot-indexer"), "{msg}");
            }
            other => panic!("expected Abort, got {other:?}"),
        }
    }
}
