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

/// Coverage grade of a candidate whose root set another pool candidate
/// *calls into* and covers strictly better: such a candidate is an internal
/// step of an outer orchestrator, so its roots are credited at
/// [`ROOT_TRANSITIVE_WEIGHT`] — the same grade as roots reached through a
/// helper — instead of at the direct ladder.
///
/// The entry-point provenance belongs to the caller. Ranking both at full
/// strength let one nested helper (`run`, a closure inside
/// `submitChangePassword` that calls two of the three roots the wrapper
/// also reaches) ride the same signature up past the documentation-light
/// definition. Downgraded, not zeroed: the step still outranks the plain
/// helpers it sits among, it simply cannot claim the direct-coverage
/// ladder or the entry-point [`ROOT_SET_BONUS`] it did not originate.
///
/// Previously expressed as a flat ×0.5 of the assembled boost. That was
/// calibrated against raw cosine: once the semantic term is pool-normalized
/// ([`normalize_pool_cosines`]) a halved entry-point signature still
/// outweighed the definition it displaced in every pool narrower than the
/// old calibration — measured on the TypeScript E2E fixture under both
/// `AllMiniLML6V2` and `BGESmallENV15`. Grading the provenance instead of
/// scaling the total keeps the rule commensurate with the normalized unit
/// and free of any per-model constant.
///
/// Test-path callers are excluded from the check — only a production caller
/// reassigns provenance.
fn superseded_step_boost(direct: usize, transitive: usize) -> f32 {
    (direct + transitive).clamp(0, ROOT_TRANSITIVE_CAP) as f32 / ROOT_TRANSITIVE_CAP as f32
        * ROOT_TRANSITIVE_WEIGHT
}

/// Weight for partial token overlap between the query and the identifier,
/// scaled by the matched-token ratio.
const TOKEN_OVERLAP_WEIGHT: f32 = 0.08;

/// Weight of the pool-normalized semantic term ([`normalize_pool_cosines`])
/// in [`final_score`] — the unit every boost constant above is expressed
/// against.
///
/// Raw cosine cannot carry that role: each embedding model produces its own
/// band and its own *semantic separation*, so absolute boosts of 0.1–0.3
/// tuned on one model overwhelm the similarity signal on another (measured:
/// `MultilingualE5Small` puts the correct hit 0.02 from its runner-up,
/// `BGESmallENV15` 0.25 span vs `AllMiniLML6V2` 0.40). Normalizing the pool
/// to `[0, 1]` and weighting it here fixes the boost-to-semantics ratio for
/// every model.
///
/// The value is the mean min–max span of the candidate pool measured over
/// the seven benchmark queries of `tests/measure_entrypoint_cosine.sh` on a
/// live `AllMiniLML6V2` index (0.299–0.464, mean 0.40) — i.e. the scale the
/// constants above were historically calibrated against. Keeping it here
/// makes the re-rank model-agnostic *without* re-tuning every boost.
const SEMANTIC_WEIGHT: f32 = 0.40;

/// Pool spread below which the normalization is degenerate (every candidate
/// carries the same cosine): the semantic term collapses to zero instead of
/// dividing by ~0.
const SEM_EPSILON: f32 = 1e-6;

/// Normalize a candidate pool's raw cosines into the `[0, 1]` semantic unit
/// [`final_score`] scores in.
///
/// Min–max over the pool: an affine transform, so two pools identical in
/// ordering *and relative gaps* normalize to identical values whatever band
/// the model emits. Relative gaps are kept (rather than replaced by rank
/// positions) because the boost constants encode "this evidence is worth X
/// cosine"; flattening the gaps would turn the ranker into "boosts first,
/// similarity as tie-break".
///
/// Degenerate inputs are defined, never NaN:
/// - fewer than two candidates, or no usable cosine at all → all zero
///   (nothing to order semantically; the boosts and the `(file_path,
///   start_line, uuid)` tie-break decide);
/// - zero spread (`max - min <= SEM_EPSILON`) → all zero, boosts still
///   applied, so a definition never ties with prose just because their
///   cosines matched;
/// - a missing or non-finite cosine → the pool minimum, so a row that
///   entered through a non-cosine recall channel cannot be pushed below a
///   pool whose true minimum is well above zero.
pub(crate) fn normalize_pool_cosines(cosines: &[Option<f32>]) -> Vec<f32> {
    let usable = |c: &Option<f32>| c.filter(|v| v.is_finite());
    let mut present = cosines.iter().filter_map(usable);
    let Some(first) = present.next() else {
        return vec![0.0; cosines.len()];
    };
    let (min, max) = present.fold((first, first), |(lo, hi), c| (lo.min(c), hi.max(c)));

    let span = max - min;
    if cosines.len() < 2 || span <= SEM_EPSILON {
        return vec![0.0; cosines.len()];
    }
    cosines
        .iter()
        .map(|c| ((usable(c).unwrap_or(min) - min) / span).clamp(0.0, 1.0))
        .collect()
}

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
    if superseded_by_caller {
        return superseded_step_boost(direct, transitive);
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
///
/// `sem` is the **pool-normalized** semantic term produced by
/// [`normalize_pool_cosines`], not a raw cosine: every boost below is
/// expressed against [`SEMANTIC_WEIGHT`] × `[0, 1]`, which is what makes
/// the ranking independent of the embedding model's cosine scale.
pub fn final_score(sem: f32, query: &str, candidate: Candidate<'_>) -> f32 {
    let Candidate {
        kind,
        file_path,
        name,
        context,
        out_degree,
    } = candidate;
    let mut score =
        SEMANTIC_WEIGHT * sem + kind_boost(kind) + lexical_boost_in_context(query, name, context);
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
    // One parse pass: the pool normalization and the scoring pass must read
    // exactly the same view of every row.
    let facts: Vec<RowFacts> = entities.iter().map(row_facts).collect();
    let cosines: Vec<Option<f32>> = facts.iter().map(|f| f.cosine).collect();
    let sems = normalize_pool_cosines(&cosines);

    let mut scored: Vec<(f32, String, i64, String, serde_json::Value)> = entities
        .into_iter()
        .zip(facts)
        .zip(sems)
        .map(|((entity, facts), sem)| {
            let score = score_row(&facts, sem, query);
            trace_ranked(&facts, sem, score);
            (score, facts.file_path, facts.start_line, facts.uuid, entity)
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

/// Everything [`rerank`] reads off one pool row, parsed once.
struct RowFacts {
    name: String,
    kind: String,
    file_path: String,
    fqn: String,
    start_line: i64,
    uuid: String,
    /// Raw cosine, `None` when the row carries no `score` field (a
    /// graph-side hit merged before ranking). Normalized to the pool
    /// minimum rather than to zero — see [`normalize_pool_cosines`].
    cosine: Option<f32>,
    direct_roots: usize,
    transitive_roots: usize,
    total_roots: usize,
    superseded_by_caller: bool,
    out_degree: usize,
    /// Diagnostic provenance: which recall channel surfaced the row
    /// (`cosine` / `definition` / `probe` / `bridge` / `prefix`, attached by
    /// the pool's `merge_hits`). Absent → plain cosine.
    channel: String,
}

/// Read one pool row's ranking inputs out of its JSON payload.
fn row_facts(entity: &serde_json::Value) -> RowFacts {
    let string_field = |key: &str| {
        entity
            .get(key)
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string()
    };
    let count_field =
        |key: &str| entity.get(key).and_then(|v| v.as_i64()).unwrap_or(0).max(0) as usize;
    RowFacts {
        name: string_field("name"),
        kind: string_field("kind"),
        file_path: string_field("file_path"),
        fqn: string_field("fqn"),
        start_line: entity
            .get("start_line")
            .and_then(|v| v.as_i64())
            .unwrap_or(0),
        uuid: string_field("uuid"),
        cosine: entity
            .get("score")
            .and_then(|v| v.as_f64())
            .map(|c| c as f32),
        direct_roots: count_field("caller_roots"),
        transitive_roots: count_field("caller_roots_transitive"),
        total_roots: count_field("caller_root_total"),
        superseded_by_caller: entity
            .get("caller_superseded")
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
        out_degree: count_field("caller_out_degree"),
        channel: entity
            .get(super::CHANNEL_FIELD)
            .and_then(|v| v.as_str())
            .unwrap_or("cosine")
            .to_string(),
    }
}

/// Score one row from its normalized semantic term and its annotations.
fn score_row(facts: &RowFacts, sem: f32, query: &str) -> f32 {
    final_score(
        sem,
        query,
        Candidate {
            kind: &facts.kind,
            file_path: &facts.file_path,
            name: &facts.name,
            context: Some(&facts.fqn),
            out_degree: facts.out_degree,
        },
    ) + root_coverage_boost(
        &facts.kind,
        &facts.file_path,
        Coverage {
            direct: facts.direct_roots,
            transitive: facts.transitive_roots,
            total_roots: facts.total_roots,
            superseded_by_caller: facts.superseded_by_caller,
        },
    )
}

/// Rank trace for one row. `cosine` stays the raw model output (the
/// cosine-window harnesses read it) and `cosine_norm` exposes the
/// pool-normalized term the score is actually built from, so a lost row can
/// be attributed to semantic recall or to scoring.
fn trace_ranked(facts: &RowFacts, sem: f32, score: f32) {
    tracing::debug!(
        target: "search_hybrid_context::rank",
        score = score,
        cosine = facts.cosine.unwrap_or(0.0),
        cosine_norm = sem,
        kind = facts.kind, name = facts.name, fqn = facts.fqn,
        channel = facts.channel,
        out_degree = facts.out_degree,
        direct = facts.direct_roots, transitive = facts.transitive_roots,
        total_roots = facts.total_roots,
        superseded = facts.superseded_by_caller,
        "ranked"
    );
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

    /// Extend a fixture pool so its cosine spread matches a live candidate
    /// pool's, and return it ready for [`rerank`].
    ///
    /// `rerank` always scores a *pool*: [`candidate_limit`] fetches 24–400
    /// rows, so the window reaches far below the handful of rows a
    /// regression fixture names (measured with
    /// `tests/measure_entrypoint_cosine.sh` on a live `AllMiniLML6V2` index,
    /// seven benchmark queries: the pool's min–max span is 0.299–0.464,
    /// mean 0.40 — the value of [`SEMANTIC_WEIGHT`]). A fixture holding only
    /// the head of that window would hand [`normalize_pool_cosines`] a
    /// several-fold stretched span and score its rows in a unit no live
    /// search produces.
    ///
    /// The appended row is inert — neutral kind, no annotation, no token in
    /// common with any query — and sits at the pool floor, so it cannot
    /// displace anything. With the span pinned to [`SEMANTIC_WEIGHT`] the
    /// normalized score reduces to `cosine + boosts` shifted by a constant,
    /// i.e. these fixtures keep testing the *ranking rule* exactly as they
    /// did before the pool normalization existed, and the scale invariance
    /// itself is pinned by the dedicated band tests below.
    fn ranked_pool(rows: Vec<serde_json::Value>, query: &str) -> Vec<serde_json::Value> {
        rerank(with_pool_tail(rows), query)
    }

    fn with_pool_tail(mut rows: Vec<serde_json::Value>) -> Vec<serde_json::Value> {
        let cosine_of =
            |row: &serde_json::Value| row.get("score").and_then(|v| v.as_f64()).map(|c| c as f32);
        let cosines: Vec<f32> = rows.iter().filter_map(cosine_of).collect();
        let Some(max) = cosines.iter().copied().reduce(f32::max) else {
            return rows;
        };
        let min = cosines.iter().copied().fold(max, f32::min);
        let floor = max - SEMANTIC_WEIGHT;
        if floor < min {
            rows.push(json!({
                "uuid": "pool-tail", "name": "zzz_inert_tail", "kind": "rust_impl",
                "fqn": "inert::zzz_inert_tail",
                "file_path": "src/zzz_inert_tail.rs", "start_line": 1,
                "score": floor
            }));
        }
        rows
    }

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
        // must not ride the entry-point signature: its roots are credited at
        // transitive grade ([`superseded_step_boost`]), so it stays below
        // the doc-light definition it displaced while remaining above the
        // plain helpers it sits among (measured on the TypeScript E2E
        // fixture: `run` inside `submitChangePassword` vs the doc-less
        // `useChangePassword` hook).
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
        assert!((step - superseded_step_boost(2, 0)).abs() < 1e-5);
        assert!(step < plain, "the downgrade must lower the boost");
        assert!(step > 0.0, "downgraded, not zeroed");
        assert!(outer > plain, "outer orchestrator keeps the full boost");
        // The step never earns the entry-point set bonus, whatever it
        // reaches: two directly-covered roots grade the same as two reached
        // through a helper.
        assert!(
            (superseded_step_boost(2, 0) - superseded_step_boost(0, 2)).abs() < 1e-6,
            "superseded provenance must ignore the direct/transitive split"
        );
        assert!(
            superseded_step_boost(3, 3) <= ROOT_TRANSITIVE_WEIGHT,
            "the downgraded grade saturates at the transitive weight"
        );
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
        let ranked = ranked_pool(candidates, "authenticate user with email and password");
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
        let ranked = ranked_pool(candidates, "authenticate user with email and password");
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
        // The bug report scenario: Markdown and a test file out-cosine the
        // definition, and two production helpers sit right below it; the
        // re-rank must put `login` first anyway.
        //
        // `normalize_email` sits at 0.57, not 0.60: at 0.60 its cosine
        // deficit against `login` (0.02) exactly cancelled its own lexical
        // boost (`email` matches 1 of the 4 query tokens, 0.08 × 1/4 =
        // 0.02), leaving the two rows tied to the last bit and the "first"
        // slot decided by floating-point noise rather than by the ranking
        // rule. The scenario is unchanged — the helper still ranks right
        // behind the definition — but the assertion now rests on a real
        // margin.
        let candidates = vec![
            json!({"uuid": "1", "name": "setup", "kind": "markdown_section",
                   "file_path": "docs/AUTH.md", "start_line": 1, "score": 0.82}),
            json!({"uuid": "2", "name": "test_login_success", "kind": "rust_function",
                   "file_path": "tests/login_test.rs", "start_line": 10, "score": 0.75}),
            json!({"uuid": "3", "name": "handle_login", "kind": "rust_function",
                   "file_path": "src/routes.rs", "start_line": 20, "score": 0.58}),
            json!({"uuid": "4", "name": "normalize_email", "kind": "rust_function",
                   "file_path": "src/util.rs", "start_line": 5, "score": 0.57}),
            json!({"uuid": "5", "name": "login", "kind": "rust_function",
                   "file_path": "src/auth.rs", "start_line": 30, "score": 0.62}),
        ];
        let ranked = ranked_pool(candidates, "authenticate user with email and password");
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
        let ranked = ranked_pool(candidates, "authenticate user with email and password");
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
        let ranked = ranked_pool(candidates, "build lookup maps for reference resolution");
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
        let ranked = ranked_pool(candidates, "evict a connection from the pool");
        assert_eq!(ranked[0]["name"], "evictConnection");
        assert_eq!(ranked[1]["name"], "shutdown");
    }

    #[test]
    fn rerank_is_deterministic_on_full_ties() {
        let a = json!({"uuid": "aaa", "name": "foo", "kind": "function",
                       "file_path": "src/a.rs", "start_line": 1, "score": 0.5});
        let b = json!({"uuid": "bbb", "name": "foo", "kind": "function",
                       "file_path": "src/a.rs", "start_line": 1, "score": 0.5});
        let ranked = ranked_pool(vec![b.clone(), a.clone()], "foo");
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
        let ranked = ranked_pool(
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
        let ranked = ranked_pool(
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
        let ranked = ranked_pool(
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
        let ranked = ranked_pool(
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
        let ranked = ranked_pool(
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
        let ranked = ranked_pool(
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
        let ranked = ranked_pool(
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
        let ranked = ranked_pool(
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
        let ranked = ranked_pool(
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
        let ranked = ranked_pool(
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
        let ranked = ranked_pool(
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
        let ranked = ranked_pool(
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
            SEMANTIC_WEIGHT * 0.30 + CALLABLE_BOOST + TEST_PATH_PENALTY
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
            SEMANTIC_WEIGHT * 0.40 + CALLABLE_BOOST,
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

    // --- pool normalization (scale-invariant scoring) ---

    #[test]
    fn normalizer_maps_pool_to_unit_range() {
        let sems = normalize_pool_cosines(&[Some(0.1), Some(0.2), Some(0.3)]);
        assert!((sems[0] - 0.0).abs() < 1e-6, "{sems:?}");
        assert!((sems[1] - 0.5).abs() < 1e-6, "{sems:?}");
        assert!((sems[2] - 1.0).abs() < 1e-6, "{sems:?}");
    }

    #[test]
    fn normalizer_flat_pool_is_zero_without_nan() {
        let sems = normalize_pool_cosines(&[Some(0.5), Some(0.5), Some(0.5)]);
        assert_eq!(sems, vec![0.0, 0.0, 0.0]);
        assert!(sems.iter().all(|s| s.is_finite()), "no NaN on a flat pool");
    }

    #[test]
    fn normalizer_single_candidate_is_zero() {
        assert_eq!(normalize_pool_cosines(&[Some(0.7)]), vec![0.0]);
    }

    #[test]
    fn normalizer_empty_pool_is_empty() {
        assert!(normalize_pool_cosines(&[]).is_empty());
    }

    #[test]
    fn normalizer_missing_cosine_is_pool_minimum() {
        // A row without a `score` field must not be treated as cosine 0 in a
        // pool whose true minimum is 0.2 — it takes the pool minimum.
        let sems = normalize_pool_cosines(&[Some(0.4), None, Some(0.2)]);
        assert!((sems[0] - 1.0).abs() < 1e-6, "{sems:?}");
        assert!((sems[1] - 0.0).abs() < 1e-6, "{sems:?}");
        assert!((sems[2] - 0.0).abs() < 1e-6, "{sems:?}");
        // Non-finite values are treated the same way.
        let with_inf = normalize_pool_cosines(&[Some(0.4), Some(f32::INFINITY), Some(0.2)]);
        assert!(with_inf.iter().all(|s| s.is_finite()), "{with_inf:?}");
        assert!((with_inf[1] - 0.0).abs() < 1e-6, "{with_inf:?}");
    }

    #[test]
    fn normalizer_is_affine_invariant() {
        // The scale-invariance mechanism: two pools identical in ordering and
        // relative gaps normalize to the same values whatever their band.
        let base = [Some(0.24), Some(0.31), Some(0.40), Some(0.53)];
        for (scale, offset) in [(0.25_f32, 0.55_f32), (3.0, -0.10), (0.1, 0.81)] {
            let shifted: Vec<Option<f32>> = base
                .iter()
                .map(|c| c.map(|v| (v - 0.24) / 0.29 * scale + offset))
                .collect();
            let a = normalize_pool_cosines(&base);
            let b = normalize_pool_cosines(&shifted);
            for (x, y) in a.iter().zip(&b) {
                assert!((x - y).abs() < 1e-5, "affine drift: {a:?} vs {b:?}");
            }
        }
    }

    #[test]
    fn normalizer_clamps_outliers_into_unit_range() {
        let sems =
            normalize_pool_cosines(&[Some(0.99), Some(0.31), Some(0.30), Some(0.29), Some(-0.42)]);
        assert!(
            sems.iter().all(|s| (0.0..=1.0).contains(s)),
            "out of range: {sems:?}"
        );
        assert!((sems[0] - 1.0).abs() < 1e-6, "{sems:?}");
        assert!((sems[4] - 0.0).abs() < 1e-6, "{sems:?}");
    }

    #[test]
    fn normalizer_is_deterministic() {
        let pool = [Some(0.31), None, Some(0.52), Some(0.41), Some(0.52)];
        assert_eq!(normalize_pool_cosines(&pool), normalize_pool_cosines(&pool));
    }

    // --- scale invariance of the whole re-rank ---

    /// The Workstream-B contract (PLAN §7 gherkin): two candidate pools
    /// identical in ordering and relative gaps must rank identically,
    /// however different their cosine bands.
    #[test]
    fn rerank_order_is_invariant_to_cosine_scale() {
        let rows = |cosines: [f32; 6]| {
            vec![
                json!({"uuid": "1", "name": "setup", "kind": "markdown_section",
                       "file_path": "docs/AUTH.md", "start_line": 1, "score": cosines[0]}),
                json!({"uuid": "2", "name": "test_login_success", "kind": "rust_function",
                       "file_path": "tests/login_test.rs", "start_line": 10, "score": cosines[1]}),
                json!({"uuid": "3", "name": "handle_login", "kind": "rust_function",
                       "fqn": "app::routes::handle_login",
                       "file_path": "src/routes.rs", "start_line": 20, "score": cosines[2]}),
                json!({"uuid": "4", "name": "normalize_email", "kind": "rust_function",
                       "fqn": "app::util::normalize_email",
                       "file_path": "src/util.rs", "start_line": 5, "score": cosines[3]}),
                json!({"uuid": "5", "name": "login", "kind": "rust_function",
                       "fqn": "app::auth::login",
                       "file_path": "src/auth.rs", "start_line": 30, "score": cosines[4],
                       "caller_roots": 2, "caller_roots_transitive": 1,
                       "caller_root_total": 8}),
                json!({"uuid": "6", "name": "AuthConfig", "kind": "rust_struct",
                       "fqn": "app::auth::AuthConfig",
                       "file_path": "src/auth.rs", "start_line": 12, "score": cosines[5]}),
            ]
        };
        let order = |ranked: Vec<serde_json::Value>| -> Vec<String> {
            ranked
                .iter()
                .filter_map(|e| e.get("name").and_then(|v| v.as_str()))
                .map(String::from)
                .collect()
        };
        let query = "authenticate user with email and password";
        // MiniLM band …
        let wide = order(rerank(rows([0.53, 0.47, 0.38, 0.41, 0.43, 0.24]), query));
        // … and the same pool squeezed into E5's ~0.07-wide band.
        let narrow = order(rerank(
            rows([0.53, 0.47, 0.38, 0.41, 0.43, 0.24].map(|c| (c - 0.24) / 0.29 * 0.07 + 0.81)),
            query,
        ));
        assert_eq!(wide, narrow, "ranking must not depend on the cosine band");
    }

    /// Unit-level stand-in for the live model matrix: the measured cosine
    /// band of every supported embedding model
    /// ([`crate::pipeline::embed::EmbedModelChoice::supported`], bands from
    /// `docs/measurements/entrypoint_cosine_comparison.md`) is applied to
    /// the two `must` baseline shapes. A live matrix would need one full
    /// re-index per model (and a Qdrant collection recreation for the
    /// 768-dim ones), so the *mechanism* is proven here and the live
    /// harness runs on the adopted model only.
    #[test]
    fn rerank_baselines_hold_across_every_supported_model_band() {
        // (model, low, high) — the measured raw-cosine band of each model.
        const BANDS: &[(&str, f32, f32)] = &[
            ("AllMiniLML6V2", 0.24, 0.53),
            ("BGESmallENV15", 0.54, 0.80),
            ("BGEBaseENV15", 0.44, 0.77),
            ("MultilingualE5Small", 0.81, 0.88),
            ("JinaEmbeddingsV2BaseCode", 0.15, 0.69),
            ("NomicEmbedTextV15", 0.55, 0.74),
        ];
        // Relative positions inside the band (1.0 = top of the band), taken
        // from the live traces: the entry point never holds the top cosine.
        for (model, low, high) in BANDS {
            let at = |fraction: f32| low + (high - low) * fraction;

            // Baseline 1 — HikariCP "borrow a connection from the pool".
            // Deliberately NOT `ranked_pool`: these fixtures *are* the pool
            // geometry under test (each row placed inside the model's own
            // measured band), so no synthetic tail may reshape them.
            let ranked = rerank(
                vec![
                    json!({"uuid": "1", "name": "borrow", "kind": "method",
                           "fqn": "com.zaxxer.hikari.util.ConcurrentBag.borrow",
                           "file_path": "src/main/java/com/zaxxer/hikari/util/ConcurrentBag.java",
                           "start_line": 312, "score": at(1.0)}),
                    json!({"uuid": "2", "name": "isConnectionAlive", "kind": "method",
                           "fqn": "com.zaxxer.hikari.pool.PoolBase.isConnectionAlive",
                           "file_path": "src/main/java/com/zaxxer/hikari/pool/PoolBase.java",
                           "start_line": 288, "score": at(0.93)}),
                    json!({"uuid": "3", "name": "releaseConnection", "kind": "method",
                           "fqn": "com.zaxxer.hikari.pool.HikariPool.releaseConnection",
                           "file_path": "src/main/java/com/zaxxer/hikari/pool/HikariPool.java",
                           "start_line": 292, "score": at(0.90)}),
                    json!({"uuid": "4", "name": "getConnection", "kind": "method",
                           "fqn": "com.zaxxer.hikari.pool.HikariPool.getConnection",
                           "file_path": "src/main/java/com/zaxxer/hikari/pool/HikariPool.java",
                           "start_line": 239, "score": at(0.76),
                           "caller_roots": 3, "caller_roots_transitive": 0,
                           "caller_root_total": 8}),
                    json!({"uuid": "5", "name": "connection pooling", "kind": "markdown_section",
                           "fqn": "README.md::connection pooling",
                           "file_path": "README.md", "start_line": 4, "score": at(0.83)}),
                    json!({"uuid": "6", "name": "testConnectionBorrow", "kind": "method",
                           "fqn": "com.zaxxer.hikari.pool.TestConnections.testConnectionBorrow",
                           "file_path": "src/test/java/com/zaxxer/hikari/pool/TestConnections.java",
                           "start_line": 61, "score": at(0.88)}),
                ],
                "borrow a connection from the pool",
            );
            let names: Vec<&str> = ranked
                .iter()
                .filter_map(|e| e.get("name").and_then(|v| v.as_str()))
                .collect();
            assert_eq!(
                names.first(),
                Some(&"getConnection"),
                "{model} band ({low}..{high}) broke the HikariCP baseline: {names:?}"
            );

            // Baseline 2 — job-watch "authenticate user with email and password".
            let ranked = rerank(
                vec![
                    json!({"uuid": "1", "name": "normalize_email", "kind": "rust_function",
                           "fqn": "jobwatch::auth::credentials::normalize_email",
                           "file_path": "src/auth/credentials.rs", "start_line": 71,
                           "score": at(1.0)}),
                    json!({"uuid": "2", "name": "verify_password", "kind": "rust_function",
                           "fqn": "jobwatch::auth::credentials::verify_password",
                           "file_path": "src/auth/credentials.rs", "start_line": 41,
                           "score": at(0.94)}),
                    json!({"uuid": "3", "name": "test_login_success", "kind": "rust_function",
                           "fqn": "tests::login::test_login_success",
                           "file_path": "tests/login_test.rs", "start_line": 10,
                           "score": at(0.90)}),
                    json!({"uuid": "4", "name": "login", "kind": "rust_function",
                           "fqn": "jobwatch::api::auth::login",
                           "file_path": "src/api/auth.rs", "start_line": 136,
                           "score": at(0.72),
                           "caller_roots": 3, "caller_roots_transitive": 1,
                           "caller_root_total": 8}),
                    json!({"uuid": "5", "name": "create_user", "kind": "rust_function",
                           "fqn": "jobwatch::api::admin::create_user",
                           "file_path": "src/api/admin.rs", "start_line": 44,
                           "score": at(0.86), "caller_roots": 1,
                           "caller_root_total": 8}),
                ],
                "authenticate user with email and password",
            );
            let names: Vec<&str> = ranked
                .iter()
                .filter_map(|e| e.get("name").and_then(|v| v.as_str()))
                .collect();
            assert_eq!(
                names.first(),
                Some(&"login"),
                "{model} band ({low}..{high}) broke the job-watch baseline: {names:?}"
            );
        }
    }

    /// The measured failure mode a model swap produces (§ "The blocker"):
    /// when a model's band compresses, a fixed boost outweighs the whole
    /// semantic span and the kind taxonomy decides the ranking on its own.
    /// Shape from chrome-devtools-mcp "capture the current view as an
    /// image": the tool is a neutral-kind `constant` holding the top cosine
    /// of the window, the competitor a callable helper at the bottom of it.
    /// Raw cosine ranks the tool first only while the band is wider than
    /// [`CALLABLE_BOOST`] — under `MultilingualE5Small` (~0.07) it is not.
    #[test]
    fn rerank_compressed_band_does_not_let_boosts_outrank_semantics() {
        const BANDS: &[(&str, f32, f32)] = &[
            ("AllMiniLML6V2", 0.24, 0.53),
            ("BGESmallENV15", 0.54, 0.80),
            ("BGEBaseENV15", 0.44, 0.77),
            ("MultilingualE5Small", 0.81, 0.88),
            ("JinaEmbeddingsV2BaseCode", 0.15, 0.69),
            ("NomicEmbedTextV15", 0.55, 0.74),
        ];
        for (model, low, high) in BANDS {
            // The band IS the fixture here — no synthetic pool tail.
            let ranked = rerank(
                vec![
                    json!({"uuid": "1", "name": "getScreenRecorder", "kind": "method",
                           "fqn": "McpContext.getScreenRecorder",
                           "file_path": "src/McpContext.ts", "start_line": 411,
                           "score": low}),
                    json!({"uuid": "2", "name": "screenshot", "kind": "constant",
                           "fqn": "screenshot",
                           "file_path": "src/tools/screenshot.ts", "start_line": 21,
                           "score": high}),
                ],
                "capture the current view as an image",
            );
            assert_eq!(
                ranked[0]["name"], "screenshot",
                "{model} band ({low}..{high}): the kind boost swallowed the semantic span"
            );
        }
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
