# Five-Model Cosine-Window & Ranking Comparison

This document summarizes the empirical evaluation of five candidate embedding models across the entry-point recall benchmark suite.

## Summary Table (Cosine rank | Final rank)

| Row | must | Baseline v5 | Workstream A (MiniLM) | BGE-small | BGE-base | Jina-code | E5-small | Nomic v1.5 |
|---|---|---|---|---|---|---|---|---|
| knot `run_search_hybrid_context` | — | absent | absent | 423→ABSENT | absent | absent | **50→91** | absent |
| HikariCP `acquire a client` | — | 7→9 | 3→13 | 5→5 | 5→8 | **1**→15 | 7→23 | 3→4 |
| csharp `get the callers` | ≤5 | 8→9 | 6→**5** | **1**→3 | **1**→2 | 5→2 | 28→33 | **1**→6 |
| csharp `find who invokes` | — | 250→ABSENT | 221→ABSENT | 168→ABSENT | **66**→ABSENT | absent | 175→ABSENT | 225→ABSENT |
| job-watch-ui `log a user in` | — | 14→20 | 13→19 | 15→31 | 4→29 | **2→3** | 25→83 | 27→81 |
| `borrow a connection` | #1 | ✅1 | ✅1 | ❌12 | ✅1 | ❌4 | ❌2 | ❌2 |
| `authenticate user` | #1 | ✅1 | ✅1 | ❌12 | ❌7 | ✅1 | ❌8 | ✅1 |
| `take screenshot` | #1 | ✅1 | ✅1 | ✅1 | ✅1 | ✅1 | ✅1 | ✅1 |
| `capture the current view` | #1 | ✅1 | ✅1 | ❌2 | ✅1 | ✅1 | ✅1 | ❌2 |
| **`must` rows failing** | | **0** | **0** | **3** | **1** | **1** | **3** | **2** |

> **Superseded (v1.11.0).** The table below measured the **raw-cosine**
> ranker, whose additive boosts were calibrated to MiniLM's band — the
> blocker this document identified. That blocker is fixed in v1.11.0
> (pool-normalized, scale-invariant scoring) and the default is now
> `BGEBaseENV15`. The v1.11.0 measurement is
> `entrypoint_cosine_v1_11_bgebase.md`; the table below is kept as the
> evidence that motivated the work.

## Findings & Key Takeaways

1. **Workstream A (`embed_text` role sentence) is a pure gain:**
   - All `must` rows remain 100% PASS with zero regressions.
   - `get the callers` improves from position 9 to 5 (meeting the ratcheted ≤5 bound).
   - 4 out of 5 target queries improve in raw cosine rank under MiniLM.

2. **Model Swap Trade-off:**
   - Models like BGE-small, BGE-base, Jina-code, and Nomic v1.5 significantly boost raw semantic cosines.
   - However, because `rank::final_score` uses additive boosts calibrated to MiniLM's scale (0.24-0.53), models with compressed high-cosine ranges (0.50-0.87) cause baseline regressions.
   - For instance, Jina-code achieves rank 1 on query embedding for `borrow a connection` (cosine 0.6620), but `HikariDataSource.getConnection` outranks `HikariPool.getConnection` due to wrapper call structure.

3. **Conclusion (as of v1.10.0):**
   - Default stayed `AllMiniLML6V2` to ensure 100% baseline compatibility out-of-the-box.
   - Full multi-model support shipped as an opt-in via `KNOT_EMBED_MODEL` (with automatic prefix management and dimension validation).
   - **Superseded in v1.11.0:** the scale-invariant ranker removes the boost-calibration blocker, and the default moved to `BGEBaseENV15`. See `entrypoint_cosine_v1_11_bgebase.md`.
