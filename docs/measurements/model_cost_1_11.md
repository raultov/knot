# Embedding-model cost measurement — knot 1.11.0

Per-model footprint and runtime cost for the two supported models
(`AllMiniLML6V2`, 384-dim, default; `BGEBaseENV15`, 768-dim, opt-in). This is
the evidence behind keeping the default on MiniLM for large estates.

**Provenance of every number is marked:**
`[measured]` = observed on the dev machine in this session,
`[derived]` = arithmetic from measured values, `[model card]` = published
architecture, `[not measured]` = requires an instrumented run that has not
happened.

## 1. Model footprint

| Model | Dims | Layers / hidden | Params | fastembed cache on disk |
|---|---|---|---|---|
| `AllMiniLML6V2` | 384 | 6 / 384 | ~22.7 M `[model card]` | **87 MB** `[measured]` |
| `BGEBaseENV15` | 768 | 12 / 768 | ~109 M `[model card]` | **417 MB** `[measured]` |

`[derived]` BGE-base carries ~4.8× the parameters and therefore ~5× the
compute per token, and produces 2× the vector bytes per entity.

For reference, the other models that were dropped in 1.11.0 are larger still
and no longer part of the supported set: E5-small 465 MB, Nomic v1.5 523 MB,
Jina-code 615 MB `[measured]`.

## 2. Collection footprint

| Model | Collection | Points | Dims | On-disk |
|---|---|---|---|---|
| `AllMiniLML6V2` | `knot_entities` | 61,638 | 384 | **162 MB** `[measured]` |

`[derived]` Raw vector bytes for that corpus: `61,638 × 384 × 4 ≈ 94.7 MB`
(MiniLM) versus `61,638 × 768 × 4 ≈ 189.4 MB` (BGE-base), i.e. **+94.7 MB** for
the same entities, before payload and index overhead. A BGE collection over the
same corpus is therefore expected around **~257 MB**, roughly 1.6× the MiniLM
one. `[not measured]` — no live BGE collection exists right now (it was wiped
in the environment reset of 2026-09-19).

## 3. Query latency

| Model | p50 | p95 | n | Build |
|---|---|---|---|---|
| `AllMiniLML6V2` | **206 ms** `[measured]` | **221 ms** `[measured]` | 20 | debug |
| `BGEBaseENV15` | `[not measured]` | `[not measured]` | — | — |

Measured through `GET /api/search` end to end (query embedding + Qdrant +
Neo4j enrichment), not vector search alone. **The server was a debug build**,
so these are an upper bound; a release build is expected to be substantially
faster. The model-to-model delta is what matters for the default choice, and
BGE-base's ~5× compute per query is not offset by anything at query time.

## 4. Indexing cost

| Metric | MiniLM | BGE-base |
|---|---|---|
| Wall-clock (full re-index) | `[not measured]` | `[not measured]` |
| Throughput (entities/s) | `[not measured]` | `[not measured]` |
| Peak RSS of the indexer | `[not measured]` | `[not measured]` |

This is the one gap. The earlier full BGE re-index (83,746 points / 30 repos,
recorded in `entrypoint_cosine_v1_11_bgebase.md`) was not instrumented for
wall-clock or RSS, and the environment was subsequently reset to a MiniLM
index, so neither side can be back-filled.

**Method to fill it** (bounded, one repository is enough for a per-model
ratio): run `knot-indexer --clean` under `/usr/bin/time -v` on the same
repository twice, once per model, into the two coexisting collections
(`knot_entities` and `knot_entities_bge768`), and record
`Elapsed (wall clock)` and `Maximum resident set size`. Throughput is the
pipeline's own entity count over the wall-clock.

## 5. Conclusion

Even with indexing wall-clock unmeasured, the footprint and query-compute
asymmetry is decisive for the default: MiniLM is **4.8× smaller on disk**
(87 MB vs 417 MB), needs **half the vector bytes** per entity, and costs ~5×
less compute per token and per query. BGE-base stays opt-in, where an operator
trades a one-off re-index and a larger collection for the recall gains
measured in `model_matrix_1_11.md`.
