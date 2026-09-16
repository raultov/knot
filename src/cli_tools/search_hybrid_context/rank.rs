//! Kind-aware re-ranking for `search_hybrid_context`.
//!
//! Vector similarity alone lets long natural-language payloads (Markdown
//! sections, test files, config keys, build dependencies) outrank the code
//! definitions a natural-language query is actually looking for: prose
//! embeds are long and lexical while a function's embed is a short
//! `[kind] name + signature` header.
//!
//! This module applies a deterministic, language-agnostic re-rank on top of
//! the cosine score returned by Qdrant:
//!
//! ```text
//! final = cosine + kind_boost + lexical_boost + root_coverage_boost
//!         + test_path_penalty
//! ```
//!
//! `root_coverage_boost` reads the pool annotation attached by the search
//! pipeline (how many of the top semantic roots a candidate calls directly
//! / through one helper) — the shared entry point of the highest-ranked
//! helpers outranks them, per the module's recall contract.
//!
//! Everything is query-time: no re-indexing is required — the annotation
//! comes from one bounded Neo4j query at search time
//! ([`crate::db::graph::QueryExt::fetch_root_coverage`]), and every other
//! input is already present in the Qdrant payload (`kind`, `file_path`,
//! `name`) or is the query itself. Ties break on
//! `(file_path, start_line, uuid)` so the order is total and reproducible.
//!
//! Penalties are deliberately moderate: a documentation-only topic (no
//! competing definition) must still surface its best Markdown section, so
//! prose/test penalties offset the systematic cosine advantage of
//! natural-language payloads without burying them.

use std::collections::HashSet;

/// Boost for definition kinds that describe behavior (functions, methods,
/// constructors and their per-language twins). Slightly above [`TYPE_BOOST`]
/// so `Type::method` can outrank its own container when a query describes
/// what the method does (`LookupMaps::build` vs `LookupMaps`).
const CALLABLE_BOOST: f32 = 0.15;

/// Boost for type-declaration kinds (classes, interfaces, structs, enums,
/// traits, records). Below [`CALLABLE_BOOST`] but far above prose.
const TYPE_BOOST: f32 = 0.10;

/// Penalty for prose kinds (Markdown documents/sections). Their embeds are
/// long natural-language payloads that dominate cosine similarity for
/// natural-language queries. Kept moderate so documentation-only topics
/// (no competing definition) still surface their best section.
const PROSE_PENALTY: f32 = -0.20;

/// Penalty for configuration and build-system entities. They are rarely what
/// a behavioral query is looking for, but must stay reachable (config/build
/// E2E suites search for them directly).
const CONFIG_BUILD_PENALTY: f32 = -0.20;

/// Penalty for entities living under test paths. Test names read like
/// natural language ("login with bad password fails") and win cosine
/// against the production definition for the same behavior. The penalty
/// must exceed that systematic cosine advantage for realistic gaps.
const TEST_PATH_PENALTY: f32 = -0.20;

/// Boost when the entity's whole name equals a query token
/// (`build` in "build lookup maps for reference resolution"). Strong enough
/// to flip a method above its own container even on a sizable cosine gap,
/// and to promote a name-probe hit whose cosine is deep but whose
/// identifier the query literally named.
const EXACT_NAME_BOOST: f32 = 0.30;

/// Attenuated exact-name boost for a *generic* token:
/// [`GENERIC_NAME_TOKENS`] verbs and nouns so frequent in code that an
/// entity merely *named* after one carries no evidence it is what the query
/// describes. `ToolRegistry.Find` ("find who invokes a…"),
/// `SuspendResumeLock.acquire` ("acquire a client…"),
/// `ConcurrentBag.borrow` ("borrow a connection…") used to win #1 on the
/// bare verb and displace the actual entry point. The full
/// [`EXACT_NAME_BOOST`] is paid only when a *second* token of the entity's
/// container context (its FQN, e.g. `ChatClient.create` ⊃ chat+client for
/// "create a client to chat…") corroborates the query — which is exactly
/// the discriminator that keeps the legitimate `LookupMaps::build` and
/// `ChatClient.create` cases at full strength.
const GENERIC_EXACT_NAME_BOOST: f32 = 0.08;

/// Boost for neutral kinds whose graph node orchestrates >=
/// [`NEUTRAL_BEHAVIORAL_OUT_DEGREE`] outgoing CALLS edges. Language-agnostic
/// by construction — it reads kind neutrality plus call-graph degree,
/// nothing else. Reason it must exist (measured, chrome-devtools-mcp):
/// a TypeScript MCP tool is `export const screenshot = defineTool({...})`, so
/// its kind (`constant`) takes [`kind_boost`] == 0 despite being the
/// behaviour a natural-language query names; a same-repo *method* helper
/// then outranks it purely on the callables' kind advantage while the tool
/// holds the highest cosine of all code rows.
const NEUTRAL_BEHAVIORAL_BOOST: f32 = 0.10;

/// Outgoing CALLS degree a neutral kind needs to count as behavioral.
/// Two calls minimum: a bare constant referencing one thing is not
/// behavior, but a definition directing two calls is street-level
/// orchestration no matter the wire kind.
const NEUTRAL_BEHAVIORAL_OUT_DEGREE: usize = 2;

/// Query words that do identify an entity when the entity's whole name
/// equals them. Deliberately set to code-generic verbs/nouns: anything
/// specific (`authenticate`, `screenshot`, `login`) stays out, and a
/// suffix-probe hit on it is the strongest name evidence the ranker has.
const GENERIC_NAME_TOKENS: &[&str] = &[
    "acquire", "add", "apply", "borrow", "build", "call", "check", "clear", "close", "create",
    "current", "data", "delete", "execute", "find", "get", "handle", "init", "insert", "list",
    "load", "make", "name", "new", "next", "open", "parse", "pop", "process", "push", "read",
    "release", "remove", "render", "reset", "result", "run", "save", "send", "set", "start",
    "stop", "update", "value", "write",
];

/// Root-set coverage weight per counted root of [`ROOT_COVERAGE_CAP`]. The
/// primary entry-point signal: a natural-language query ranks the helpers
/// of the behaviour it names, and the production entry point is usually
/// their shared caller (`login` calls `normalize_email`,
/// `verify_credentials_or_fail`, `generate_token` in job-watch; the pool
/// annotation measures exactly that reach). Absolute count, NOT a fraction
/// of the root set: a saturating count survives the widened seed
/// (`TOP_ROOTS = 8`) — a pure `covered/total` fraction would shrink every
/// candidate's score as the seed grows, diluting the very signal the
/// wider seed exists to amplify. The fraction appears only as a
/// tie-break ([`ROOT_FRACTION_WEIGHT`]).
const ROOT_COVERAGE_WEIGHT: f32 = 0.36;

/// Coverage count the weight saturates at. Covering ≥ 3 roots fills the
/// primary term; beyond that the bonuses below fire, not more weight.
const ROOT_COVERAGE_CAP: usize = 3;

/// Entry-point signature bonus: a candidate calling ≥ 2 distinct top roots
/// must beat the sum of two single-root boosts (2 × 0.126) — that is what
/// distinguishes the shared caller of the behaviour from an incidental
/// one-caller helper.
const ROOT_SET_BONUS: f32 = 0.14;

/// Weight for transitive coverage: the candidate calls a helper which calls
/// a top root (one hop beyond the candidate). Catches entry points that
/// orchestrate exactly one helper wrapping several roots. Counted for
/// roots the candidate does NOT already call directly — never double
/// counted ([`crate::db::graph::root_coverage_query`]).
const ROOT_TRANSITIVE_WEIGHT: f32 = 0.10;

/// Transitive count the weight saturates at.
const ROOT_TRANSITIVE_CAP: usize = 3;

/// Tie-break term: the covered fraction of the seed set
/// (`direct / total`). "2 of 3" beats "1 of 5" when the absolute counts
/// agree (both capped) — keeps the priority on genuine root-set coverage
/// over raw counts without scaling the whole boost down as the seed
/// widens.
const ROOT_FRACTION_WEIGHT: f32 = 0.05;

/// Upper bound of the assembled root-set boost, kept moderate so a
/// coverage-driven promotion still needs the cosine to be in the same
/// band as the candidates it displaces.
const ROOT_BOOST_MAX: f32 = 0.55;

/// Attenuation applied to the coverage boost when another pool candidate
/// CALLS this entity and covers a *strictly greater* root set
/// Such a candidate is an internal step of an outer orchestrator — the
/// entry-point provenance belongs to the caller, and ranking both at full
/// strength would let one nested helper (`run`, a closure inside
/// `submitChangePassword` that calls two of the three roots the wrapper
/// also reaches) ride the same signature up past the documentation-light
/// definition. Halved, not zeroed: the step still outranked the plain
/// helpers it sits among. Test-path callers are excluded from the check —
/// only a production caller reassigns provenance.
const ORCHESTRATOR_STEP_ATTENUATION: f32 = 0.5;

/// Weight for partial token overlap between the query and the identifier,
/// scaled by the matched-token ratio.
const TOKEN_OVERLAP_WEIGHT: f32 = 0.08;

use crate::cli_tools::kinds::{CALLABLE_KINDS, CONFIG_BUILD_KINDS, TYPE_KINDS};
/// Prose kinds whose embeds are long natural-language bodies.
/// Shared taxonomy lives in [`crate::cli_tools::kinds`]; imported here (plus
/// `rank::parse_kinds` / `rank::kinds_allow` re-exports used by `mod.rs`) so
/// the ranker and the kind filters can never drift apart.
pub(crate) use crate::cli_tools::kinds::{PROSE_KINDS, kinds_allow, parse_kinds};

/// Query words that carry no identifier meaning. Deliberately small: words
/// like `get` or `user` are meaningful identifier fragments.
const STOPWORDS: &[&str] = &[
    "a", "an", "the", "for", "with", "from", "to", "of", "in", "on", "and", "or", "is", "are",
    "be", "by", "at", "as", "it", "its", "this", "that", "how", "do", "does", "what", "when",
    "where", "which", "my", "me", "i", "you", "your", "we", "our", "their", "his", "her",
];

/// Kind boost for a wire-format kind string. Unknown kinds (HTML, CSS ids,
/// VTC fixtures, …) score neutral so the cosine order between them is kept.
pub fn kind_boost(kind: &str) -> f32 {
    if CALLABLE_KINDS.contains(&kind) {
        CALLABLE_BOOST
    } else if TYPE_KINDS.contains(&kind) {
        TYPE_BOOST
    } else if PROSE_KINDS.contains(&kind) {
        PROSE_PENALTY
    } else if CONFIG_BUILD_KINDS.contains(&kind) {
        CONFIG_BUILD_PENALTY
    } else {
        0.0
    }
}

/// Whether `file_path` points into test code.
///
/// Matched by path segments and file-name conventions only — deliberately
/// conservative so words merely *containing* "test" (`latest.rs`,
/// `contest.rs`) never match.
pub fn is_test_path(file_path: &str) -> bool {
    let normalized = file_path.replace('\\', "/");
    let lower = normalized.to_lowercase();

    // Directory conventions (leading slash added so bare `tests/…` matches).
    let padded = format!("/{lower}");
    for marker in ["/test/", "/tests/", "/__tests__/", "/spec/"] {
        if padded.contains(marker) {
            return true;
        }
    }

    let file_name = normalized.rsplit('/').next().unwrap_or(&normalized);
    let lower_name = file_name.to_lowercase();
    // Suffix conventions: `foo_test.rs`, `foo.test.ts`, `foo.spec.ts`.
    if lower_name.contains("_test.")
        || lower_name.contains(".test.")
        || lower_name.contains(".spec.")
    {
        return true;
    }
    // `test_foo.rs` prefix convention.
    if lower_name.starts_with("test_") {
        return true;
    }
    // Java/TS class-per-file convention: `FooTest.java`, `UserTests.ts`.
    // Case-sensitive on purpose: a lowercase stem ending in "test"
    // (`latest.rs`) is not a test file.
    let stem = file_name.split('.').next().unwrap_or(file_name);
    stem.ends_with("Test") || stem.ends_with("Tests")
}

/// Split an identifier into lowercase tokens at snake_case, kebab-case and
/// camelCase boundaries. Delegates to the shared tokenizer
/// ([`crate::utils::identifiers::identifier_tokens`]) so the ranker and the
/// embed-text builder always agree on token boundaries.
fn identifier_tokens(name: &str) -> Vec<String> {
    crate::utils::identifiers::identifier_tokens(name)
}

/// Lowercase alphanumeric query tokens, minus stopwords and single chars.
pub(crate) fn query_tokens(query: &str) -> Vec<String> {
    query
        .split(|c: char| !c.is_alphanumeric())
        .map(str::to_lowercase)
        .filter(|t| t.len() >= 2 && !STOPWORDS.contains(&t.as_str()))
        .collect()
}

/// Significant query tokens for the name-exact probe: identifier-like
/// tokens (≥ 3 chars) that may literally name an entity. Deduplicated in
/// query order and capped to keep the probe to a single Qdrant round-trip.
pub fn significant_query_tokens(query: &str) -> Vec<String> {
    let mut seen: HashSet<String> = HashSet::new();
    query_tokens(query)
        .into_iter()
        .filter(|t| t.len() >= 3)
        .filter(|t| seen.insert(t.clone()))
        .take(4)
        .collect()
}

/// Case variants for the name-exact probe: each token in lowercase plus its
/// PascalCase form, covering snake_case functions and class-style names
/// (`build` → build, Build; `similaritysearch` → similaritysearch,
/// Similaritysearch).
pub fn probe_name_variants(tokens: &[String]) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    for token in tokens {
        let mut pascal = token.clone();
        if let Some(first) = pascal.get_mut(0..1) {
            first.make_ascii_uppercase();
        }
        for candidate in [token.clone(), pascal] {
            if !names.contains(&candidate) {
                names.push(candidate);
            }
        }
    }
    names
}

/// Root-set coverage inputs the boost reads, bundled to stay within
/// clippy's arity threshold ([`root_coverage_boost`]).
pub(crate) struct Coverage {
    /// Top semantic roots the candidate calls directly.
    pub direct: usize,
    /// Top roots reached through exactly one helper (excluding direct).
    pub transitive: usize,
    /// Size of the seeded root set (the annotation's denominator).
    pub total_roots: usize,
    /// Whether another pool candidate covering ⊇ this root set calls this
    /// entity (the boost is halved; see [`ORCHESTRATOR_STEP_ATTENUATION`]).
    pub superseded_by_caller: bool,
}

/// Root-set coverage boost — assembled entry-point signature from the pool
/// annotation. Production callables only: a candidate under a test path
/// must not reclaim the test-path penalty through call provenance, and
/// prose/config kinds have no boost at all.
fn root_coverage_boost(kind: &str, file_path: &str, coverage: Coverage) -> f32 {
    let Coverage {
        direct,
        transitive,
        total_roots,
        superseded_by_caller,
    } = coverage;
    if kind_boost(kind) < 0.0 || is_test_path(file_path) {
        return 0.0;
    }
    let covered = direct.clamp(0, ROOT_COVERAGE_CAP) as f32 / ROOT_COVERAGE_CAP as f32;
    let mut boost = covered * ROOT_COVERAGE_WEIGHT;
    if direct >= 2 {
        boost += ROOT_SET_BONUS;
    }
    boost += transitive.clamp(0, ROOT_TRANSITIVE_CAP) as f32 / ROOT_TRANSITIVE_CAP as f32
        * ROOT_TRANSITIVE_WEIGHT;
    // Fraction tie-break, saturated at the same coverage cap so a huge
    // direct count cannot keep nudging the score past the bound.
    boost += direct.clamp(0, ROOT_COVERAGE_CAP) as f32 / total_roots.max(1) as f32
        * ROOT_FRACTION_WEIGHT;
    if superseded_by_caller {
        boost *= ORCHESTRATOR_STEP_ATTENUATION;
    }
    boost.min(ROOT_BOOST_MAX)
}

/// Lexical agreement between the query and the entity name — no container
/// context available (see [`lexical_boost_in_context`] for the guarded
/// variant).
pub fn lexical_boost(query: &str, name: &str) -> f32 {
    lexical_boost_in_context(query, name, None)
}

/// Lexical agreement between the query and the entity name, guarded by the
/// entity's container context.
///
/// An exact whole-name hit against a query token earns [`EXACT_NAME_BOOST`]
/// — unless the token is a generic verb/noun ([`GENERIC_NAME_TOKENS`]),
/// when the full boost is paid only if a *second* token of `context` (the
/// FQN: `ChatClient` in `ChatClient.create`) also appears in the query;
/// otherwise [`GENERIC_EXACT_NAME_BOOST`]. Partial token overlap earns
/// `TOKEN_OVERLAP_WEIGHT * matched/query_tokens` either way.
pub fn lexical_boost_in_context(query: &str, name: &str, context: Option<&str>) -> f32 {
    let q_tokens = query_tokens(query);
    if q_tokens.is_empty() {
        return 0.0;
    }
    if q_tokens.iter().any(|q| *q == name.to_lowercase()) {
        let matched_generic = GENERIC_NAME_TOKENS.contains(&name.to_lowercase().as_str());
        if !matched_generic {
            return EXACT_NAME_BOOST;
        }
        // Generic token: demand corroboration from the container context
        // (any *other* token of the FQN that the query also names). The
        // shared tokenizer splits snake/kebab/camel plus namespace
        // delimiters, so `LookupMaps` yields `lookup`+`maps`.
        if let Some(ctx) = context {
            let name_lower = name.to_lowercase();
            if identifier_tokens(ctx)
                .iter()
                .any(|part| *part != name_lower && q_tokens.contains(part))
            {
                return EXACT_NAME_BOOST;
            }
        }
        return GENERIC_EXACT_NAME_BOOST;
    }
    let n_tokens = identifier_tokens(name);
    let matched = n_tokens.iter().filter(|t| q_tokens.contains(t)).count();
    if matched == 0 {
        return 0.0;
    }
    TOKEN_OVERLAP_WEIGHT * (matched as f32 / q_tokens.len() as f32)
}

/// Row fields the scorer reads, bundled to stay within clippy's arity
/// threshold ([`final_score`]). `context` is the entity's FQN when the pool
/// annotation supplied one — the exact-name guard's corroboration input.
/// `out_degree` is the graph's outgoing CALLS count for the entity, the
/// input of the neutral-kind behavioral boost; `0` when un-annotated.
pub struct Candidate<'a> {
    /// Entity name (the lexical-boost input).
    pub name: &'a str,
    /// Wire-format kind (kind boost).
    pub kind: &'a str,
    /// Repo-relative path (test-path penalty input).
    pub file_path: &'a str,
    /// Container context (FQN) or `None` when unavailable.
    pub context: Option<&'a str>,
    /// Outgoing CALLS degree measured by the pool annotation.
    pub out_degree: usize,
}

/// Final ranking score for one candidate.
pub fn final_score(cosine: f32, query: &str, candidate: Candidate<'_>) -> f32 {
    let Candidate {
        kind,
        file_path,
        name,
        context,
        out_degree,
    } = candidate;
    let mut score = cosine + kind_boost(kind) + lexical_boost_in_context(query, name, context);
    // Neutral kinds that orchestrate calls are behavior the taxonomy did
    // not name (TS MCP tools, top-level TS modules); prosa/config/test
    // kinds pay penalties and can never take the boost, and a sub-degree
    // node stays cosmetic.
    if kind_boost(kind) == 0.0
        && out_degree >= NEUTRAL_BEHAVIORAL_OUT_DEGREE
        && !is_test_path(file_path)
    {
        score += NEUTRAL_BEHAVIORAL_BOOST;
    }
    if is_test_path(file_path) {
        score += TEST_PATH_PENALTY;
    }
    score
}

/// Re-rank candidate entities by their final score, ties broken
/// deterministically on `(file_path, start_line, uuid)`.
///
/// Rows carry the graph annotation the boost reads: `fqn` (the exact-name
/// guard's context), and — when the pool was root-coverage annotated —
/// `caller_roots`, `caller_roots_transitive` and `caller_root_total`.
/// An absent annotation contributes nothing; the score stays
/// cosine + kind + lexical + penalties.
///
/// Candidates missing a `score` field (e.g. graph-side prefix hits that were
/// merged before ranking) are not expected here; they score cosine `0.0` and
/// sink below scored hits, which keeps the name-match contract intact —
/// callers prepend those separately.
pub fn rerank(entities: Vec<serde_json::Value>, query: &str) -> Vec<serde_json::Value> {
    let mut scored: Vec<(f32, String, i64, String, serde_json::Value)> = entities
        .into_iter()
        .map(|entity| {
            let kind = entity.get("kind").and_then(|v| v.as_str()).unwrap_or("");
            let file_path = entity
                .get("file_path")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let name = entity.get("name").and_then(|v| v.as_str()).unwrap_or("");
            let fqn = entity.get("fqn").and_then(|v| v.as_str()).unwrap_or("");
            let start_line = entity
                .get("start_line")
                .and_then(|v| v.as_i64())
                .unwrap_or(0);
            let uuid = entity
                .get("uuid")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let cosine = entity.get("score").and_then(|v| v.as_f64()).unwrap_or(0.0) as f32;
            let direct_roots = entity
                .get("caller_roots")
                .and_then(|v| v.as_i64())
                .unwrap_or(0)
                .max(0) as usize;
            let transitive_roots = entity
                .get("caller_roots_transitive")
                .and_then(|v| v.as_i64())
                .unwrap_or(0)
                .max(0) as usize;
            let total_roots = entity
                .get("caller_root_total")
                .and_then(|v| v.as_i64())
                .unwrap_or(0)
                .max(0) as usize;
            let superseded_by_caller = entity
                .get("caller_superseded")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            // Diagnostic provenance: which recall channel surfaced the row
            // (`cosine` / `definition` / `probe` / `bridge` / `prefix`,
            // attached by the pool's `merge_hits`). Absent → plain cosine.
            let channel = entity
                .get(super::CHANNEL_FIELD)
                .and_then(|v| v.as_str())
                .unwrap_or("cosine");
            let out_degree = entity
                .get("caller_out_degree")
                .and_then(|v| v.as_i64())
                .unwrap_or(0)
                .max(0) as usize;
            let score = final_score(
                cosine,
                query,
                Candidate {
                    kind,
                    file_path: &file_path,
                    name,
                    context: Some(fqn),
                    out_degree,
                },
            ) + root_coverage_boost(
                kind,
                &file_path,
                Coverage {
                    direct: direct_roots,
                    transitive: transitive_roots,
                    total_roots,
                    superseded_by_caller,
                },
            );
            tracing::debug!(
                target: "search_hybrid_context::rank",
                score = score,
                cosine = cosine,
                kind = kind, name = name, fqn = fqn,
                channel = channel,
                out_degree = out_degree,
                direct = direct_roots, transitive = transitive_roots,
                total_roots = total_roots,
                superseded = superseded_by_caller,
                "ranked"
            );
            (score, file_path, start_line, uuid, entity)
        })
        .collect();

    scored.sort_by(|a, b| {
        b.0.partial_cmp(&a.0)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.1.cmp(&b.1))
            .then_with(|| a.2.cmp(&b.2))
            .then_with(|| a.3.cmp(&b.3))
    });

    scored
        .into_iter()
        .map(|(_, _, _, _, entity)| entity)
        .collect()
}

/// Whether an entity passes the search's optional path filter.
///
/// Delegates to the shared matcher ([`crate::cli_tools::list_files`]):
/// empty/`.` keeps everything, a directory prefix must match on a `/`
/// boundary (`src/api` never matches `src/api-notes.md`), and `*`/`?`/`**`
/// switch to glob matching. Applies to name-prefix hits and vector hits
/// alike so CLI and MCP behave identically.
pub fn path_allows(pattern: Option<&str>, file_path: &str) -> bool {
    match pattern {
        None => true,
        Some(p) => crate::cli_tools::list_files::path_matches(p, file_path),
    }
}

/// Over-fetch window for the vector search: wide enough that a definition
/// sitting below prose in pure cosine order still enters the candidate set
/// and can be promoted by the re-rank.
///
/// The upper clamp scales with the module's result ceiling
/// ([`super::MAX_RESULTS_CEILING`]) so a request at the ceiling keeps a
/// coherent cosine pool (4× the result count) instead of being padded from
/// the recall channels. Unchanged for every `max_results <= 20`, where the
/// legacy `80` ceiling never bound.
pub fn candidate_limit(max_results: usize) -> usize {
    (max_results * 4).clamp(24, super::MAX_RESULTS_CEILING * 4)
}

/// Deduplicate enriched entities by identity.
///
/// Primary key is the entity UUID (the Qdrant↔Neo4j bridge); as a defensive
/// second pass, rows sharing `(repo_name, fqn, start_line)` collapse to the
/// first occurrence — the UUID identity is exactly
/// `repo:file:fqn:start_line`, so this can only merge the same entity.
/// Rows without a usable FQN keep their (already unique) UUID as key.
pub fn dedup_by_identity(entities: Vec<serde_json::Value>) -> Vec<serde_json::Value> {
    let mut seen: HashSet<String> = HashSet::new();
    entities
        .into_iter()
        .filter(|entity| {
            let uuid = entity
                .get("uuid")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let fqn = entity
                .get("fqn")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let repo = entity
                .get("repo_name")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let line = entity
                .get("start_line")
                .and_then(|v| v.as_i64())
                .unwrap_or(0);
            let key = if fqn.is_empty() {
                uuid
            } else {
                format!("{repo}\u{0}{fqn}\u{0}{line}")
            };
            seen.insert(key)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // --- root_coverage_boost ---

    #[test]
    fn root_coverage_ladder_rewards_multi_root_callers() {
        // Coverage 0 contributes nothing.
        assert_eq!(
            root_coverage_boost(
                "function",
                "src/a.rs",
                Coverage {
                    direct: 0,
                    transitive: 0,
                    total_roots: 8,
                    superseded_by_caller: false
                }
            ),
            0.0
        );
        // One root ≈ the legacy single-root boost (0.126 vs 0.12): subtle.
        let one = root_coverage_boost(
            "function",
            "src/a.rs",
            Coverage {
                direct: 1,
                transitive: 0,
                total_roots: 8,
                superseded_by_caller: false,
            },
        );
        assert!(
            (one - 1.0 / 3.0 * ROOT_COVERAGE_WEIGHT - ROOT_FRACTION_WEIGHT * (1.0 / 8.0)).abs()
                < 1e-5
        );
        // Two roots must beat the sum of two single-root candidates — the
        // entry-point signature.
        let two = root_coverage_boost(
            "function",
            "src/a.rs",
            Coverage {
                direct: 2,
                transitive: 0,
                total_roots: 8,
                superseded_by_caller: false,
            },
        );
        assert!(two >= 2.0 * one, "set bonus missing: {two} vs 2×{one}");
        // Weight saturates at the cap; the cap-bound total holds.
        let three = root_coverage_boost(
            "function",
            "src/a.rs",
            Coverage {
                direct: 3,
                transitive: 0,
                total_roots: 8,
                superseded_by_caller: false,
            },
        );
        let saturated = root_coverage_boost(
            "function",
            "src/a.rs",
            Coverage {
                direct: 7,
                transitive: 0,
                total_roots: 8,
                superseded_by_caller: false,
            },
        );
        assert!(
            (three - saturated).abs() < 1e-6,
            "beyond the cap coverage must not change the score"
        );
        assert!(three <= ROOT_BOOST_MAX);
    }

    #[test]
    fn root_coverage_two_of_three_beats_helper_with_higher_cosine() {
        // Coverage 2 of 3 must beat a helper covering only 1 of 5 that
        // outranks it in pure cosine (+0.10): coverage 2.0/3.0-derived
        // score difference ≈ 0.393 - 0.126 > 0.10.
        let a_covered = final_score(
            0.40,
            "unshared query",
            Candidate {
                kind: "function",
                file_path: "src/a.rs",
                name: "entryPoint",
                context: Some("app::A::entryPoint"),
                out_degree: 0,
            },
        ) + root_coverage_boost(
            "function",
            "src/a.rs",
            Coverage {
                direct: 2,
                transitive: 0,
                total_roots: 3,
                superseded_by_caller: false,
            },
        );
        let b_higher_cosine = final_score(
            0.5,
            "unshared query",
            Candidate {
                kind: "function",
                file_path: "src/b.rs",
                name: "helperOne",
                context: Some("app::B::helperOne"),
                out_degree: 0,
            },
        ) + root_coverage_boost(
            "function",
            "src/b.rs",
            Coverage {
                direct: 1,
                transitive: 0,
                total_roots: 5,
                superseded_by_caller: false,
            },
        );
        assert!(
            a_covered > b_higher_cosine,
            "coverage 2/3 must beat cosine +0.10 at 1/5"
        );
    }

    #[test]
    fn root_coverage_transitive_lifts_entry_point_calling_one_helper() {
        // The orchestration-follows-one-helper shape: the entry point calls
        // exactly one helper which wraps several roots. Direct 0, three
        // transitive roots must land between zero coverage and direct 2.
        let none = root_coverage_boost(
            "function",
            "src/a.rs",
            Coverage {
                direct: 0,
                transitive: 0,
                total_roots: 8,
                superseded_by_caller: false,
            },
        );
        let transitive3 = root_coverage_boost(
            "function",
            "src/a.rs",
            Coverage {
                direct: 0,
                transitive: 3,
                total_roots: 8,
                superseded_by_caller: false,
            },
        );
        let direct2 = root_coverage_boost(
            "function",
            "src/a.rs",
            Coverage {
                direct: 2,
                transitive: 0,
                total_roots: 8,
                superseded_by_caller: false,
            },
        );
        assert!(
            transitive3 > none,
            "transitive coverage must lift above zero"
        );
        assert!(
            transitive3 + none <= direct2,
            "one-helper orchestration must not outrank a real 2-root caller"
        );
        // Transitive coverage saturates at its cap.
        assert_eq!(
            root_coverage_boost(
                "function",
                "src/a.rs",
                Coverage {
                    direct: 0,
                    transitive: 5,
                    total_roots: 8,
                    superseded_by_caller: false
                }
            ),
            transitive3
        );
    }

    #[test]
    fn root_coverage_orchestration_step_attenuated_when_caller_covers_superset() {
        // A nested step whose pool caller covers at least the same root set
        // must not ride the entry-point signature at full strength: the
        // halved score stays below the doc-light definition it displaced
        // (measured: run 0.316-cosine × full boost 0.39 > use 0.519-cosine
        // + 0; halved it falls back under).
        let step = root_coverage_boost(
            "function",
            "src/hooks.ts",
            Coverage {
                direct: 2,
                transitive: 0,
                total_roots: 8,
                superseded_by_caller: true,
            },
        );
        let outer = root_coverage_boost(
            "function",
            "src/hooks.ts",
            Coverage {
                direct: 3,
                transitive: 0,
                total_roots: 8,
                superseded_by_caller: false,
            },
        );
        let plain = root_coverage_boost(
            "function",
            "src/hooks.ts",
            Coverage {
                direct: 2,
                transitive: 0,
                total_roots: 8,
                superseded_by_caller: false,
            },
        );
        assert!((step - plain * ORCHESTRATOR_STEP_ATTENUATION).abs() < 1e-5);
        assert!(step < plain, "attenuation must lower the boost");
        assert!(outer > plain, "outer orchestrator keeps the full boost");
    }

    #[test]
    fn root_coverage_never_rescues_tests_prose_or_config() {
        // Test-path candidates must not reclaim the test-path penalty
        // through call provenance.
        assert_eq!(
            root_coverage_boost(
                "rust_function",
                "tests/login_test.rs",
                Coverage {
                    direct: 3,
                    transitive: 2,
                    total_roots: 8,
                    superseded_by_caller: false
                }
            ),
            0.0
        );
        // Prose roots have no callers, but guard the kind gate anyway.
        assert_eq!(
            root_coverage_boost(
                "markdown_section",
                "docs/a.md",
                Coverage {
                    direct: 3,
                    transitive: 2,
                    total_roots: 8,
                    superseded_by_caller: false
                }
            ),
            0.0
        );
        // Config/build entities likewise.
        assert_eq!(
            root_coverage_boost(
                "config_property",
                "config/app.yml",
                Coverage {
                    direct: 3,
                    transitive: 2,
                    total_roots: 8,
                    superseded_by_caller: false
                }
            ),
            0.0
        );
        assert_eq!(
            root_coverage_boost(
                "build_dependency",
                "Cargo.toml",
                Coverage {
                    direct: 3,
                    transitive: 2,
                    total_roots: 8,
                    superseded_by_caller: false
                }
            ),
            0.0
        );
    }

    #[test]
    fn rerank_test_calling_same_helpers_stays_below_entry_point() {
        // A test file calling exactly the same helpers gets the identical
        // annotation; only the test-path penalty keeps it below the
        // production entry point.
        let candidates = vec![
            json!({"uuid": "1", "name": "test_login_success", "kind": "rust_function",
                   "file_path": "tests/login_test.rs", "start_line": 10, "score": 0.62,
                   "caller_roots": 2, "caller_roots_transitive": 1, "caller_root_total": 8}),
            json!({"uuid": "2", "name": "login", "kind": "rust_function",
                   "file_path": "src/api/auth.rs", "start_line": 30, "score": 0.62,
                   "caller_roots": 2, "caller_roots_transitive": 1, "caller_root_total": 8}),
        ];
        let ranked = rerank(candidates, "authenticate user with email and password");
        assert_eq!(ranked[0]["name"], "login", "test path must stay below");
        assert_eq!(ranked[1]["name"], "test_login_success");
    }

    #[test]
    fn rerank_promotes_shared_caller_of_top_root() {
        // Bug scenario: the doc-less definition shares the cosine band with
        // the helpers a paraphrase ranks first, but it is their caller. The
        // caller-root boost must lift it into the leading slots.
        let candidates = vec![
            json!({"uuid": "1", "name": "normalize_email", "kind": "rust_function",
                   "file_path": "src/auth/credentials.rs", "start_line": 71, "score": 0.319}),
            json!({"uuid": "2", "name": "verify_password", "kind": "rust_function",
                   "file_path": "src/auth/credentials.rs", "start_line": 41, "score": 0.302}),
            json!({"uuid": "3", "name": "login", "kind": "rust_function",
                   "file_path": "src/api/auth.rs", "start_line": 136, "score": 0.241,
                   "caller_roots": 2}),
            json!({"uuid": "4", "name": "test_login_success", "kind": "rust_function",
                   "file_path": "tests/login_test.rs", "start_line": 10, "score": 0.24,
                   "caller_roots": 2}),
            json!({"uuid": "5", "name": "create_user", "kind": "rust_function",
                   "file_path": "src/api/admin.rs", "start_line": 12, "score": 0.26,
                   "caller_roots": 1}),
        ];
        let ranked = rerank(candidates, "authenticate user with email and password");
        let names: Vec<&str> = ranked
            .iter()
            .filter_map(|e| e.get("name").and_then(|v| v.as_str()))
            .collect();
        let login = names.iter().position(|n| *n == "login").unwrap();
        assert!(
            login <= 2,
            "login must enter the leading slots, got order {names:?}"
        );
        // Tests never climb via call provenance.
        let test = names
            .iter()
            .position(|n| *n == "test_login_success")
            .unwrap();
        assert!(test > login, "test files stay below production callables");
    }

    // --- kind_boost ---

    #[test]
    fn kind_boost_callables_outrank_types() {
        assert!(kind_boost("function") > kind_boost("class"));
        assert!(kind_boost("rust_method") > kind_boost("rust_struct"));
        assert_eq!(kind_boost("java_method_placeholder_unused"), 0.0);
    }

    #[test]
    fn kind_boost_covers_one_kind_per_language() {
        // One callable and one type per supported language family.
        for callable in [
            "method",
            "kotlin_function",
            "rust_function",
            "python_method",
            "c_function",
            "cpp_method",
            "csharp_method",
            "groovy_method",
        ] {
            assert_eq!(kind_boost(callable), CALLABLE_BOOST, "callable {callable}");
        }
        for ty in [
            "class",
            "kotlin_class",
            "rust_struct",
            "python_class",
            "cpp_class",
            "groovy_class",
            "csharp_class",
        ] {
            assert_eq!(kind_boost(ty), TYPE_BOOST, "type {ty}");
        }
    }

    #[test]
    fn kind_boost_penalizes_prose_and_config() {
        assert_eq!(kind_boost("markdown_section"), PROSE_PENALTY);
        assert_eq!(kind_boost("markdown_document"), PROSE_PENALTY);
        assert_eq!(kind_boost("config_property"), CONFIG_BUILD_PENALTY);
        assert_eq!(kind_boost("build_dependency"), CONFIG_BUILD_PENALTY);
        assert_eq!(kind_boost("cargo_package"), CONFIG_BUILD_PENALTY);
        assert_eq!(kind_boost("project_identity"), CONFIG_BUILD_PENALTY);
        // Neutral kinds keep cosine order.
        assert_eq!(kind_boost("constant"), 0.0);
        assert_eq!(kind_boost("css_class"), 0.0);
        assert_eq!(kind_boost("rust_impl"), 0.0);
        assert_eq!(kind_boost("something_new"), 0.0);
    }

    // --- is_test_path ---

    #[test]
    fn test_path_positives() {
        for path in [
            "tests/login_test.rs",
            "src/tests/login.rs",
            "src/test/foo.java",
            "src/__tests__/login.ts",
            "spec/login_spec.rb",
            "tests/foo.rs",
            "login_test.rs",
            "login.test.ts",
            "login.spec.ts",
            "test_login.rs",
            "LoginTest.java",
            "UserServiceTests.java",
        ] {
            assert!(is_test_path(path), "expected test path: {path}");
        }
    }

    #[test]
    fn test_path_negatives_do_not_substring_match() {
        for path in [
            "src/latest.rs",
            "src/contest.rs",
            "src/testing_support.rs",
            "src/login.rs",
            "attestations.rs",
        ] {
            assert!(!is_test_path(path), "expected non-test path: {path}");
        }
    }

    // --- lexical_boost ---

    #[test]
    fn lexical_exact_name_hit() {
        // Non-generic token: the full boost stands without context.
        assert_eq!(
            lexical_boost_in_context(
                "authenticate user with email and password",
                "authenticate",
                None
            ),
            EXACT_NAME_BOOST
        );
    }

    #[test]
    fn lexical_partial_overlap_ratio() {
        // similarity + search match 2 of 5 query tokens.
        let boost = lexical_boost(
            "vector store similarity search implementation",
            "similaritySearch",
        );
        assert!((boost - TOKEN_OVERLAP_WEIGHT * (2.0 / 5.0)).abs() < 1e-6);
    }

    #[test]
    fn lexical_snake_case_splits() {
        let boost = lexical_boost(
            "link cross repository build dependencies",
            "link_cross_repo_dependencies",
        );
        // link, cross, dependencies match 3 of 5 tokens (repo != repository).
        assert!((boost - TOKEN_OVERLAP_WEIGHT * (3.0 / 5.0)).abs() < 1e-6);
    }

    #[test]
    fn lexical_no_overlap_is_zero() {
        assert_eq!(
            lexical_boost("authenticate user with email and password", "parse_config"),
            0.0
        );
    }

    #[test]
    fn lexical_stopwords_and_single_chars_ignored() {
        // Only "build" survives as a meaningful token; as a *generic* name
        // token (no context), it earns the attenuated boost.
        assert_eq!(
            lexical_boost("a build of the thing", "build"),
            GENERIC_EXACT_NAME_BOOST
        );
        assert_eq!(lexical_boost("a to i", "a"), 0.0);
    }

    // --- final_score / rerank ---

    #[test]
    fn rerank_promotes_definition_over_prose_test_and_helper() {
        // The bug report scenario: Markdown, a test file, a helper and a
        // caller all out-cosine the definition; the re-rank must put `login`
        // first anyway.
        let candidates = vec![
            json!({"uuid": "1", "name": "setup", "kind": "markdown_section",
                   "file_path": "docs/AUTH.md", "start_line": 1, "score": 0.82}),
            json!({"uuid": "2", "name": "test_login_success", "kind": "rust_function",
                   "file_path": "tests/login_test.rs", "start_line": 10, "score": 0.75}),
            json!({"uuid": "3", "name": "handle_login", "kind": "rust_function",
                   "file_path": "src/routes.rs", "start_line": 20, "score": 0.58}),
            json!({"uuid": "4", "name": "normalize_email", "kind": "rust_function",
                   "file_path": "src/util.rs", "start_line": 5, "score": 0.60}),
            json!({"uuid": "5", "name": "login", "kind": "rust_function",
                   "file_path": "src/auth.rs", "start_line": 30, "score": 0.62}),
        ];
        let ranked = rerank(candidates, "authenticate user with email and password");
        assert_eq!(ranked[0]["name"], "login", "definition must rank first");
        // Prose and the test file must fall below every production callable.
        let names: Vec<&str> = ranked
            .iter()
            .filter_map(|e| e.get("name").and_then(|v| v.as_str()))
            .collect();
        let login_pos = names.iter().position(|n| *n == "login").unwrap();
        let setup_pos = names.iter().position(|n| *n == "setup").unwrap();
        let test_pos = names
            .iter()
            .position(|n| *n == "test_login_success")
            .unwrap();
        assert!(login_pos < setup_pos && login_pos < test_pos);
    }

    #[test]
    fn rerank_recovers_definition_buried_far_below_prose() {
        // Extreme case from the report: the definition's cosine is so low it
        // was absent from the top-8; the kind/test/prose adjustments alone
        // must still lift it above Markdown and the test file.
        let candidates = vec![
            json!({"uuid": "1", "name": "setup", "kind": "markdown_section",
                   "file_path": "docs/AUTH.md", "start_line": 1, "score": 0.82}),
            json!({"uuid": "2", "name": "test_login_success", "kind": "rust_function",
                   "file_path": "tests/login_test.rs", "start_line": 10, "score": 0.70}),
            json!({"uuid": "3", "name": "login", "kind": "rust_function",
                   "file_path": "src/auth.rs", "start_line": 30, "score": 0.52}),
        ];
        let ranked = rerank(candidates, "authenticate user with email and password");
        assert_eq!(ranked[0]["name"], "login");
        assert_eq!(ranked[1]["name"], "test_login_success");
        assert_eq!(ranked[2]["name"], "setup");
    }

    #[test]
    fn rerank_lets_method_beat_its_own_container() {
        // `LookupMaps::build` (callable, lower cosine) vs the `LookupMaps`
        // struct (type, higher cosine): exact name token + callable boost
        // must flip the order. The rows carry their real FQNs, as the pool
        // annotation does — `build` is a generic token and only the
        // corroborating container (`lookup`, `maps` both in the query) pays
        // the full exact-name boost.
        let candidates = vec![
            json!({"uuid": "1", "name": "LookupMaps", "kind": "rust_struct",
                   "fqn": "knot::pipeline::ingest::resolve::LookupMaps",
                   "file_path": "src/resolve/mod.rs", "start_line": 131, "score": 0.80}),
            json!({"uuid": "2", "name": "build", "kind": "rust_method",
                   "fqn": "knot::pipeline::ingest::resolve::LookupMaps::build",
                   "file_path": "src/resolve/mod.rs", "start_line": 144, "score": 0.58}),
        ];
        let ranked = rerank(candidates, "build lookup maps for reference resolution");
        assert_eq!(ranked[0]["name"], "build");
    }

    #[test]
    fn rerank_preserves_cosine_order_within_same_bucket() {
        // HikariCP guardrail: two same-kind hits keep their cosine order.
        let candidates = vec![
            json!({"uuid": "1", "name": "evictConnection", "kind": "method",
                   "file_path": "src/Pool.java", "start_line": 10, "score": 0.91}),
            json!({"uuid": "2", "name": "shutdown", "kind": "method",
                   "file_path": "src/Pool.java", "start_line": 40, "score": 0.87}),
        ];
        let ranked = rerank(candidates, "evict a connection from the pool");
        assert_eq!(ranked[0]["name"], "evictConnection");
        assert_eq!(ranked[1]["name"], "shutdown");
    }

    #[test]
    fn rerank_is_deterministic_on_full_ties() {
        let a = json!({"uuid": "aaa", "name": "foo", "kind": "function",
                       "file_path": "src/a.rs", "start_line": 1, "score": 0.5});
        let b = json!({"uuid": "bbb", "name": "foo", "kind": "function",
                       "file_path": "src/a.rs", "start_line": 1, "score": 0.5});
        let ranked = rerank(vec![b.clone(), a.clone()], "foo");
        assert_eq!(ranked[0]["uuid"], "aaa");
        assert_eq!(ranked[1]["uuid"], "bbb");
    }

    #[test]
    fn rerank_handles_missing_fields_without_panic() {
        let ranked = rerank(
            vec![json!({"name": "mystery"}), json!({"uuid": "u"})],
            "query",
        );
        assert_eq!(ranked.len(), 2);
    }

    // --- candidate_limit ---

    #[test]
    fn candidate_limit_clamped_window() {
        assert_eq!(candidate_limit(1), 24);
        assert_eq!(candidate_limit(5), 24);
        assert_eq!(candidate_limit(10), 40);
        assert_eq!(candidate_limit(20), 80);
    }

    #[test]
    fn candidate_limit_is_unchanged_below_legacy_ceiling() {
        // Backward-compat pin: for every `max_results <= 20` the widened
        // scale never binds, so pre-existing search behavior is untouched.
        for n in 1..=20 {
            assert_eq!(
                candidate_limit(n),
                (n * 4).clamp(24, 80),
                "candidate_limit changed for max_results = {n}"
            );
        }
    }

    #[test]
    fn candidate_limit_scales_to_result_ceiling() {
        // At the advertised maximum the re-rank keeps a coherent cosine
        // pool (4x the result count) instead of padding from recall.
        assert_eq!(candidate_limit(crate::cli_tools::MAX_RESULTS_CEILING), 400);
    }

    // --- name-exact probe ---

    #[test]
    fn significant_query_tokens_filters_and_caps() {
        let tokens = significant_query_tokens("build lookup maps for reference resolution");
        assert_eq!(tokens, vec!["build", "lookup", "maps", "reference"]);
        // Stopwords and short tokens dropped; duplicates collapsed.
        assert_eq!(
            significant_query_tokens("a to do the the build"),
            vec!["build"]
        );
        assert!(significant_query_tokens("of to in").is_empty());
    }

    #[test]
    fn probe_name_variants_cover_snake_and_pascal() {
        let variants = probe_name_variants(&["build".to_string(), "user".to_string()]);
        assert_eq!(
            variants,
            vec![
                "build".to_string(),
                "Build".to_string(),
                "user".to_string(),
                "User".to_string()
            ]
        );
        // Empty input → empty variants.
        assert!(probe_name_variants(&[]).is_empty());
    }

    // --- generic-name guard ---

    #[test]
    fn generic_name_boost_attenuated_without_context() {
        // `find` in "find who invokes a given symbol" is a generic verb;
        // `ToolRegistry.Find`'s context carries no corroborating token,
        // so only the attenuated boost applies.
        assert_eq!(
            lexical_boost_in_context(
                "find who invokes a given symbol",
                "Find",
                Some("CodeMap.Mcp.ToolRegistry.Find")
            ),
            GENERIC_EXACT_NAME_BOOST
        );
        // Same verdict without any context at all.
        assert_eq!(
            lexical_boost("acquire a client for talking to the database", "acquire"),
            GENERIC_EXACT_NAME_BOOST
        );
    }

    #[test]
    fn generic_name_boost_full_when_context_corroborates() {
        // `ChatClient.create`: `chat` and `client` (both in the query)
        // corroborate the container — the legitimate spring-ai case keeps
        // the full boost.
        assert_eq!(
            lexical_boost_in_context(
                "create a client to chat with a language model",
                "create",
                Some("org.springframework.ai.chat.client.ChatClient.create")
            ),
            EXACT_NAME_BOOST
        );
        // `LookupMaps::build`: the container corroborates too.
        assert_eq!(
            lexical_boost_in_context(
                "build lookup maps for reference resolution",
                "build",
                Some("knot::pipeline::ingest::resolve::LookupMaps::build")
            ),
            EXACT_NAME_BOOST
        );
    }

    #[test]
    fn non_generic_exact_name_keeps_full_boost() {
        for (query, name) in [
            ("authenticate user with email and password", "authenticate"),
            ("find screenshot capture in tool reference", "screenshot"),
            ("encode the payload as utf8", "encode"),
        ] {
            assert_eq!(
                lexical_boost_in_context(query, name, None),
                EXACT_NAME_BOOST,
                "non-generic token must keep the full boost: {name}"
            );
        }
    }

    // --- cross-language entry-point recall regressions ---
    //
    // One test per evidence-table row of the reported bug. Each fixture
    // reproduces the real query-time shape (kind, file path, FQN, plausible
    // cosine band and root coverage as the live index showed it); the
    // assertion pins the expected entry point at position 1. These were
    // failing before the root-set coverage + generic-name guard fix.
    //
    // Scope note (v1.9.7): the fixtures *feed the ranker directly*, i.e.
    // each row supplies the graph annotation (`caller_roots`, …) as an
    // input. Whether the live pipeline actually supplies those inputs —
    // the pool/seed/bridge side — is validated by the two complementary
    // layers: the pipeline unit tests of `super`/`mod.rs` (definition
    // channel, seed union, depth-2 bridge, prefix demotion) and the
    // opt-in live harness `tests/run_rank_recall_live.sh`, which measures
    // the full pipeline against real indexed repositories. A unit test
    // here passing does not imply the live pipeline produces the same
    // annotation; that separation is deliberate and documented.

    /// Rust / job-watch. Paraphrase shares no tokens with `login`
    /// (`caller_roots` previously never reached cosine-pool rows).
    #[test]
    fn regression_rust_job_watch_login() {
        let ranked = rerank(
            vec![
                json!({"uuid": "ensure_account_eligible", "name": "ensure_account_eligible", "kind": "rust_function",
                       "fqn": "app::ensure_account_eligible",
                       "file_path": "src/api/auth.rs", "start_line": 9, "score": 0.62,
                       "caller_roots": 1, "caller_roots_transitive": 0,
                       "caller_root_total": 8}),
                json!({"uuid": "2", "name": "create_user", "kind": "rust_function",
                       "fqn": "jobwatch::api::admin::create_user",
                       "file_path": "src/api/admin.rs", "start_line": 44, "score": 0.60}),
                json!({"uuid": "3", "name": "bootstrap_email", "kind": "rust_function",
                       "fqn": "jobwatch::bootstrap_email",
                       "file_path": "src/main.rs", "start_line": 210, "score": 0.58}),
                json!({"uuid": "3", "name": "login", "kind": "rust_function",
                       "fqn": "jobwatch::api::auth::login",
                       "file_path": "src/api/auth.rs", "start_line": 136, "score": 0.57,
                       "caller_roots": 3, "caller_roots_transitive": 1,
                       "caller_root_total": 8}),
            ],
            "authenticate user with email and password",
        );
        assert_regression_first(&ranked, "login");
    }

    /// Rust / knot. `find` in the query must not hand #1 to the dep-graph
    /// helpers, and the entry point must surface from deep cosine at all.
    #[test]
    fn regression_rust_knot_run_search_hybrid_context() {
        let ranked = rerank(
            vec![
                json!({"uuid": "find_repo_dependents", "name": "find_repo_dependents", "kind": "rust_function",
                       "fqn": "app::find_repo_dependents",
                       "file_path": "src/db/graph/query_repo.rs", "start_line": 9, "score": 0.64,
                       "caller_roots": 0, "caller_roots_transitive": 0,
                       "caller_root_total": 8}),
                json!({"uuid": "find_repo_dependencies", "name": "find_repo_dependencies", "kind": "rust_function",
                       "fqn": "app::find_repo_dependencies",
                       "file_path": "src/db/graph/query_repo.rs", "start_line": 9, "score": 0.62,
                       "caller_roots": 0, "caller_roots_transitive": 0,
                       "caller_root_total": 8}),
                json!({"uuid": "3", "name": "run_search_hybrid_context",
                       "kind": "rust_function",
                       "fqn": "knot::cli_tools::search_hybrid_context::run_search_hybrid_context",
                       "file_path": "src/cli_tools/search_hybrid_context/mod.rs",
                       "start_line": 112, "score": 0.40,
                       "caller_roots": 2, "caller_roots_transitive": 2,
                       "caller_root_total": 8}),
            ],
            "find relevant code by meaning across the repository",
        );
        assert_regression_first(&ranked, "run_search_hybrid_context");
    }

    /// Java / HikariCP "borrow a connection from the pool": `borrow` (a
    /// generic verb, no corroboration) must not win; the entry point calls
    /// the top roots directly.
    #[test]
    fn regression_java_hikari_get_connection_borrow() {
        let ranked = rerank(
            vec![
                json!({"uuid": "1", "name": "borrow", "kind": "method",
                       "fqn": "com.zaxxer.hikari.util.ConcurrentBag.borrow",
                       "file_path": "src/main/java/com/zaxxer/hikari/util/ConcurrentBag.java",
                       "start_line": 312, "score": 0.62}),
                json!({"uuid": "2", "name": "isConnectionAlive", "kind": "method",
                       "fqn": "com.zaxxer.hikari.pool.PoolBase.isConnectionAlive",
                       "file_path": "src/main/java/com/zaxxer/hikari/pool/PoolBase.java",
                       "start_line": 288, "score": 0.60}),
                json!({"uuid": "3", "name": "releaseConnection", "kind": "method",
                       "fqn": "com.zaxxer.hikari.pool.HikariPool.releaseConnection",
                       "file_path": "src/main/java/com/zaxxer/hikari/pool/HikariPool.java",
                       "start_line": 292, "score": 0.59}),
                json!({"uuid": "4", "name": "getConnection", "kind": "method",
                       "fqn": "com.zaxxer.hikari.pool.HikariPool.getConnection",
                       "file_path": "src/main/java/com/zaxxer/hikari/pool/HikariPool.java",
                       "start_line": 239, "score": 0.55,
                       "caller_roots": 3, "caller_roots_transitive": 0,
                       "caller_root_total": 8}),
            ],
            "borrow a connection from the pool",
        );
        assert_regression_first(&ranked, "getConnection");
    }

    /// Java / HikariCP "acquire a client for talking to the database":
    /// `acquire` (a lock helper) must not displace the pool's entry point.
    #[test]
    fn regression_java_hikari_get_connection_acquire() {
        let ranked = rerank(
            vec![
                json!({"uuid": "1", "name": "acquire", "kind": "method",
                       "fqn": "com.zaxxer.hikari.util.SuspendResumeLock.acquire",
                       "file_path": "src/main/java/com/zaxxer/hikari/util/SuspendResumeLock.java",
                       "start_line": 62, "score": 0.63}),
                json!({"uuid": "2", "name": "getConnectionTimeout", "kind": "method",
                       "fqn": "com.zaxxer.hikari.HikariConfigMXBean.getConnectionTimeout",
                       "file_path": "src/main/java/com/zaxxer/hikari/HikariConfigMXBean.java",
                       "start_line": 34, "score": 0.60}),
                json!({"uuid": "3", "name": "getConnection", "kind": "method",
                       "fqn": "com.zaxxer.hikari.pool.HikariPool.getConnection",
                       "file_path": "src/main/java/com/zaxxer/hikari/pool/HikariPool.java",
                       "start_line": 239, "score": 0.54,
                       "caller_roots": 2, "caller_roots_transitive": 2,
                       "caller_root_total": 8}),
            ],
            "acquire a client for talking to the database",
        );
        assert_regression_first(&ranked, "getConnection");
    }

    /// Java / spring-ai. Control row: the fix must NOT unseat this one —
    /// `ChatClient.create` corroborates the container via chat+client.
    #[test]
    fn regression_java_spring_ai_chat_client_create() {
        let ranked = rerank(
            vec![
                json!({"uuid": "1", "name": "create", "kind": "method",
                       "fqn": "org.springframework.ai.chat.client.ChatClient.create",
                       "file_path": "spring-ai-client-chat/src/main/java/org/springframework/ai/chat/client/ChatClient.java",
                       "start_line": 98, "score": 0.56}),
                json!({"uuid": "2", "name": "model", "kind": "method",
                       "fqn": "org.springframework.ai.chat.prompt.ChatOptions.model",
                       "file_path": "spring-ai-model/src/main/java/org/springframework/ai/chat/prompt/ChatOptions.java",
                       "start_line": 87, "score": 0.50}),
            ],
            "create a client to chat with a language model",
        );
        assert_regression_first(&ranked, "create");
    }

    /// C# / csharp-code-map. `find` in the query used to hand #1 to the
    /// unrelated `ToolRegistry.Find`; the entry point must win on coverage.
    #[test]
    fn regression_csharp_get_callers_async_find() {
        let ranked = rerank(
            vec![
                json!({"uuid": "1", "name": "Find", "kind": "csharp_method",
                       "fqn": "CodeMap.Mcp.ToolRegistry.Find",
                       "file_path": "src/CodeMap.Mcp/ToolRegistry.cs",
                       "start_line": 45, "score": 0.62}),
                json!({"uuid": "2", "name": "GetFactsForSymbolAsync", "kind": "csharp_method",
                       "fqn": "CodeMap.Core.Interfaces.ISymbolStore.GetFactsForSymbolAsync",
                       "file_path": "src/CodeMap.Core/Interfaces/ISymbolStore.cs",
                       "start_line": 120, "score": 0.59}),
                json!({"uuid": "3", "name": "GetCallersAsync", "kind": "csharp_method",
                       "fqn": "CodeMap.Query.QueryEngine.GetCallersAsync",
                       "file_path": "src/CodeMap.Query/QueryEngine.cs",
                       "start_line": 380, "score": 0.53,
                       "caller_roots": 3, "caller_roots_transitive": 1,
                       "caller_root_total": 8}),
            ],
            "find who invokes a given symbol",
        );
        assert_regression_first(&ranked, "GetCallersAsync");
    }

    /// C# / csharp-code-map, second row: `get` is generic too.
    #[test]
    fn regression_csharp_get_callers_async_get() {
        let ranked = rerank(
            vec![
                json!({"uuid": "1", "name": "GetSymbolAsync", "kind": "csharp_method",
                       "fqn": "CodeMap.Core.Interfaces.ISymbolStore.GetSymbolAsync",
                       "file_path": "src/CodeMap.Core/Interfaces/ISymbolStore.cs",
                       "start_line": 96, "score": 0.60}),
                json!({"uuid": "2", "name": "GetCallersAsync", "kind": "csharp_method",
                       "fqn": "CodeMap.Query.QueryEngine.GetCallersAsync",
                       "file_path": "src/CodeMap.Query/QueryEngine.cs",
                       "start_line": 380, "score": 0.54,
                       "caller_roots": 2, "caller_roots_transitive": 2,
                       "caller_root_total": 8}),
            ],
            "get the callers of a symbol",
        );
        assert_regression_first(&ranked, "GetCallersAsync");
    }

    /// TypeScript / chrome-devtools-mcp: "capture the current view as an
    /// image" — a constant named after a query word (`current`) must not
    /// bury the tool definition.
    #[test]
    fn regression_ts_screenshot_capture_view() {
        let ranked = rerank(
            vec![
                json!({"uuid": "1", "name": "current", "kind": "constant",
                       "file_path": "src/TextSnapshot.ts", "start_line": 32, "score": 0.61}),
                json!({"uuid": "2", "name": "getScreenRecorder", "kind": "method",
                       "fqn": "McpContext.getScreenRecorder",
                       "file_path": "src/McpContext.ts", "start_line": 411, "score": 0.59}),
                json!({"uuid": "3", "name": "attachImage", "kind": "method",
                       "fqn": "McpResponse.attachImage",
                       "file_path": "src/McpResponse.ts", "start_line": 168, "score": 0.58}),
                json!({"uuid": "4", "name": "screenshot", "kind": "constant",
                       "file_path": "src/tools/screenshot.ts", "start_line": 21, "score": 0.52,
                       "caller_roots": 3, "caller_roots_transitive": 1,
                       "caller_root_total": 8}),
            ],
            "capture the current view as an image",
        );
        assert_regression_first(&ranked, "screenshot");
    }

    /// TypeScript / chrome-devtools-mcp: "take screenshot" must surface the
    /// tool, not the Markdown section that shares its title verbatim.
    #[test]
    fn regression_ts_screenshot_take_screenshot() {
        let ranked = rerank(
            vec![
                json!({"uuid": "1", "name": "take screenshot",
                       "kind": "markdown_section",
                       "fqn": "docs/tool-reference.md::Chrome DevTools MCP Tool Reference > Debugging > take screenshot",
                       "file_path": "docs/tool-reference.md", "start_line": 12, "score": 0.66}),
                json!({"uuid": "2", "name": "screenshot", "kind": "constant",
                       "file_path": "src/tools/screenshot.ts", "start_line": 21, "score": 0.55,
                       "caller_roots": 3, "caller_roots_transitive": 1,
                       "caller_root_total": 8}),
            ],
            "take screenshot",
        );
        assert_regression_first(&ranked, "screenshot");
    }

    /// JavaScript / job-watch-ui: the session/auth entry point must beat
    /// the `Session` machinery it operates on.
    #[test]
    fn regression_js_job_watch_ui_login() {
        let ranked = rerank(
            vec![
                json!({"uuid": "1", "name": "Session", "kind": "class",
                       "file_path": "src/lib/session.js", "start_line": 18, "score": 0.60}),
                json!({"uuid": "2", "name": "readSession", "kind": "function",
                       "fqn": "readSession",
                       "file_path": "src/lib/token.js", "start_line": 45, "score": 0.58}),
                json!({"uuid": "3", "name": "login", "kind": "function",
                       "fqn": "src::auth::login",
                       "file_path": "src/auth.js", "start_line": 88, "score": 0.53,
                       "caller_roots": 3, "caller_roots_transitive": 0,
                       "caller_root_total": 8}),
            ],
            "log a user in and issue a session token",
        );
        assert_regression_first(&ranked, "login");
    }

    /// Report task 3: five roots where the entry point calls only one and
    /// two helpers share query tokens (one of them named after a generic
    /// verb) — the entry point still ranks first. Cosines stay in a
    /// realistic band (helpers slightly above the entry point, as measured
    /// on the live index).
    #[test]
    fn rerank_entry_point_beats_generic_verb_leaf() {
        let ranked = rerank(
            vec![
                json!({"uuid": "1", "name": "find_matching_nodes", "kind": "rust_function",
                       "fqn": "app::find_matching_nodes",
                       "file_path": "src/match.rs", "start_line": 9, "score": 0.55}),
                json!({"uuid": "2", "name": "lookup_nodes", "kind": "rust_function",
                       "fqn": "app::lookup_tables",
                       "file_path": "src/tables.rs", "start_line": 21, "score": 0.54}),
                json!({"uuid": "3", "name": "resolve", "kind": "rust_function",
                       "fqn": "app::resolve",
                       "file_path": "src/resolve.rs", "start_line": 31, "score": 0.52,
                       "caller_roots": 1, "caller_roots_transitive": 2,
                       "caller_root_total": 5}),
            ],
            "find matching nodes by lookup",
        );
        assert_regression_first(&ranked, "resolve");
    }

    // --- neutral-kind behavioral boost (C1) ---

    /// Measured regression (chrome-devtools-mcp, `capture the current view
    /// as an image`): the `screenshot` tool is `export const … = defineTool`
    /// so its kind scores neutral, but it holds the highest code cosine of
    /// the window (0.376 vs the method helper's 0.295) and orchestrates 16
    /// outgoing CALLS. Once the neutral-kind boost is in, it must top the
    /// callable helper.
    #[test]
    fn neutral_kind_with_call_edges_outranks_lower_cosine_callable() {
        let ranked = rerank(
            vec![
                json!({"uuid": "1", "name": "getScreenRecorder", "kind": "method",
                       "fqn": "McpContext.getScreenRecorder",
                       "file_path": "src/McpContext.ts", "start_line": 411, "score": 0.295}),
                json!({"uuid": "2", "name": "screenshot", "kind": "constant",
                       "fqn": "screenshot",
                       "file_path": "src/tools/screenshot.ts", "start_line": 21, "score": 0.376,
                       "caller_out_degree": 16}),
            ],
            "capture the current view as an image",
        );
        assert_eq!(ranked[0]["name"], "screenshot", "order got {ranked:?}");
    }

    /// Prose/config/test candidates must never take the boost, whatever
    /// their call-edge degree (they should not gain outgoing CALLS edges in
    /// the graph at all, but the boost's own gate stays closed for
    /// defense-in-depth).
    #[test]
    fn neutral_boost_never_rescues_prose_config_or_tests() {
        let query = "capture anything";
        let boosted = |kind: &str, path: &str| {
            final_score(
                0.30,
                query,
                Candidate {
                    name: "x",
                    kind,
                    file_path: path,
                    context: None,
                    out_degree: 20,
                },
            )
        };
        let bare = final_score(
            0.30,
            query,
            Candidate {
                name: "x",
                kind: "markdown_section",
                file_path: "docs/a.md",
                context: None,
                out_degree: 0,
            },
        );
        // Prose: penalty, no boost.
        assert_eq!(boosted("markdown_section", "docs/a.md"), bare);
        // Config: penalty, no boost.
        let config_bare = final_score(
            0.30,
            query,
            Candidate {
                name: "x",
                kind: "config_property",
                file_path: "config/a.yml",
                context: None,
                out_degree: 0,
            },
        );
        assert_eq!(boosted("config_property", "config/a.yml"), config_bare);
        // Test path: the boost gate is closed (the kind is a callable, so
        // the neutral branch never opened) — cosine + callable + penalty.
        assert_eq!(
            boosted("rust_function", "tests/foo_test.rs"),
            0.30 + CALLABLE_BOOST + TEST_PATH_PENALTY
        );
    }

    #[test]
    fn neutral_boost_requires_minimum_out_degree() {
        // Out-degree 1 stays cosmetic: one call is not orchestration.
        let one = final_score(
            0.40,
            "anything at all",
            Candidate {
                kind: "constant",
                file_path: "src/a.ts",
                name: "k",
                context: None,
                out_degree: 1,
            },
        );
        let zero = final_score(
            0.40,
            "anything at all",
            Candidate {
                kind: "constant",
                file_path: "src/a.ts",
                name: "k",
                context: None,
                out_degree: 0,
            },
        );
        assert_eq!(one, zero);
        // At the threshold the boost lands exactly once.
        let two = final_score(
            0.40,
            "anything at all",
            Candidate {
                kind: "constant",
                file_path: "src/a.ts",
                name: "k",
                context: None,
                out_degree: 2,
            },
        );
        assert!((two - (one + NEUTRAL_BEHAVIORAL_BOOST)).abs() < 1e-6);
    }

    /// C# regression-scope sanity: an interface declaration (kind boost via
    /// taxonomy, but zero out-degree from the graph) must not ride the
    /// neutral-kind boost — an interface cannot CALL anything.
    #[test]
    fn neutral_boost_applies_only_to_neutral_kinds() {
        let callable = final_score(
            0.40,
            "q",
            Candidate {
                kind: "csharp_method",
                file_path: "src/A.cs",
                name: "M",
                context: None,
                out_degree: 10,
            },
        );
        assert_eq!(
            callable,
            0.40 + CALLABLE_BOOST,
            "callable kinds keep their own boost and never overlay the neutral one"
        );
    }

    /// Sanity helper: position of a name in a ranked list, verified at #1.
    fn assert_regression_first(ranked: &[serde_json::Value], expected: &str) {
        let names: Vec<&str> = ranked
            .iter()
            .filter_map(|e| e.get("name").and_then(|v| v.as_str()))
            .collect();
        assert_eq!(
            names.first(),
            Some(&expected),
            "expected {expected} at position 1, got order {names:?}"
        );
    }

    // --- dedup_by_identity ---

    #[test]
    fn dedup_keeps_first_of_duplicate_identity() {
        let entities = vec![
            json!({"uuid": "1", "name": "user", "fqn": "app::User", "repo_name": "r", "start_line": 3}),
            json!({"uuid": "2", "name": "user", "fqn": "app::User", "repo_name": "r", "start_line": 3}),
            json!({"uuid": "3", "name": "user", "fqn": "other::User", "repo_name": "r", "start_line": 9}),
        ];
        let deduped = dedup_by_identity(entities);
        assert_eq!(deduped.len(), 2);
        assert_eq!(deduped[0]["uuid"], "1");
        assert_eq!(deduped[1]["uuid"], "3");
    }

    #[test]
    fn dedup_rows_without_fqn_survive_via_uuid_key() {
        let entities = vec![
            json!({"uuid": "1", "name": "a", "repo_name": "r", "start_line": 1}),
            json!({"uuid": "2", "name": "b", "repo_name": "r", "start_line": 1}),
        ];
        assert_eq!(dedup_by_identity(entities).len(), 2);
    }
}
