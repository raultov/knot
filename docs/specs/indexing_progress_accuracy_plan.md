# Indexing Progress Accuracy — Implementation Plan (v1.6.2)

**Status:** Planned — not yet implemented
**Target version:** knot `1.6.2` (downstream: knot-server `0.3.2`)
**Methodology:** TDD/BDD — every behaviour starts as a failing test

---

## 1. Problem Statement

During a full index of a 3,713-file repository, `/api/repos/{id}/progress` and the
`[Progress]` log line both reached **100 % within ~6 seconds** and then stayed
frozen at 100 % for several minutes while 652 batches were still being embedded
and ingested. The run only actually finished long after the bar claimed it was
done.

### Observed evidence

```
06:58:01.954  [Progress] [opencode] 3713/3713 files (100.0%) — batch #639 ingested (128 entities)
...
06:58:08.391  Stage 2: Parallel parsing complete.
...
06:58:16.236  All 652 batches dispatched — waiting for ingestion workers to finish...
```

Note that `Stage 2: Parallel parsing complete.` is logged **7 seconds after** the
progress counter already read 100 %, and batches keep flowing for a long time
afterwards.

---

## 2. Root Cause

Three independent facts combine into the bug.

### 2.1 The percentage only measures file reading

`ProgressTracker::snapshot()` (`src/pipeline/progress.rs:61-68`) computes:

```rust
let pct = if stage == IndexingStage::Completed {
    100.0
} else if total == 0 {
    0.0
} else {
    let raw = (parsed as f32 / total as f32) * 100.0;
    raw.clamp(0.0, 100.0)
};
```

`parsed_files / total_files` covers **Stage 2 only**. Stages 3–7 (embedding,
Qdrant/Neo4j ingestion, reference resolution) — which dominate wall-clock time —
contribute nothing to the number.

### 2.2 Parsing is orders of magnitude faster than the rest of the pipeline

`parse_files_stream` (`src/pipeline/parser/mod.rs:92-145`) fans every file out
across OS threads inside a `std::thread::scope`, and fires `on_file_parsed`
inside each worker (`mod.rs:134-136`). Tree-sitter parsing is CPU-local and
extremely fast, so `parsed_files` saturates at `total_files` in ~1–2 seconds.

### 2.3 The counter saturates *before* the slow work even starts

After the scope joins, all entities live in a single `Vec`, are post-processed
by `aggregate_varnish_builtin_subs`, and only *then* pushed one by one into the
bounded channel (`mod.rs:148-158`):

```rust
for entity in entities {
    if sender.blocking_send(entity).is_err() { ... }
}
```

The channel capacity is `batch_size * 4` (`src/pipeline/runner.rs:154`). It
fills immediately, so this loop blocks for the entire duration of embedding and
ingestion. That is precisely why `Stage 2: Parallel parsing complete.` appears 7
seconds late in the logs — and why the percentage sits at 100 % the whole time.

**Summary:** 100 % currently means "the repository has been *read*", not "the
repository has been *indexed*".

---

## 3. Design

### 3.1 Weighted bands

The percentage becomes a piecewise function over weighted bands. Weights are
chosen to approximate **real wall-clock share**, measured on the reference run
(3,713 files: ~6 s parsing vs ~4 min embedding + ingestion, i.e. parsing is
under 5 % of total time):

| Phase | Band | Driver |
|---|---|---|
| `Idle` / `Discovering` / `Classifying` / `CleaningStaleData` | `0 %` | — |
| Parsing | `0 → 10 %` | `parsed_files / total_files` |
| Embedding + Ingestion | `10 → 90 %` | `entities_ingested / total_entities` |
| `ResolvingReferences` | `95 %` | fixed (no sub-counters available) |
| `Completed` | `100 %` | forced |
| `Failed` | last computed value | — |

Constants (named, no magic numbers):

```rust
const PARSE_BAND_END: f32 = 10.0;
const INGEST_BAND_END: f32 = 90.0;
const RESOLVING_PERCENT: f32 = 95.0;
```

### 3.2 The percentage is counter-driven, not stage-driven

`Parsing` and `Ingesting` genuinely overlap — the runner acknowledges this in a
comment at `src/pipeline/runner.rs:194-198`, where the stage flips to
`Ingesting` only after the blocking send loop drains, long after ingestion
actually began. Therefore the ingest band must activate as soon as
`total_entities` becomes known, regardless of the current `stage` value.

Resolution order inside `snapshot()`:

1. `stage == Completed` → `100.0`
2. `stage == ResolvingReferences` → `RESOLVING_PERCENT`
3. `total_entities` known → `PARSE_BAND_END + ratio * (INGEST_BAND_END - PARSE_BAND_END)`
4. `total_files > 0` → `(parsed_files / total_files) * PARSE_BAND_END`
5. otherwise → `0.0`

### 3.3 Where `total_entities` comes from

The exact entity count is known at one specific instant: after the
`thread::scope` joins and after `aggregate_varnish_builtin_subs` runs, but
**before** the blocking send loop begins (`src/pipeline/parser/mod.rs:148-152`).

That instant is exactly the moment the current progress freezes. Publishing the
total there makes the handoff seamless: the parse band tops out at 10 % and the
ingest band takes over in the same tick.

### 3.4 `total_entities` needs an explicit "known" flag

`0` cannot be used as the "unknown" sentinel: a repository can legitimately parse
to zero entities (empty repo, unsupported languages only). Use a separate
`AtomicBool`:

```rust
total_entities: AtomicU64,
total_entities_known: AtomicBool,
```

When known with a value of `0`, the ingest band is trivially complete and the
percentage jumps straight to `INGEST_BAND_END`.

### 3.5 `ParseCallbacks` struct (replaces the bare callback parameter)

`parse_files_stream` already takes 5 arguments and `clippy.toml` sets
`too-many-arguments-threshold = 5`. Adding a 6th parameter would trip
`clippy::too_many_arguments`, and per project policy the fix must be a refactor,
never an `#[allow]`/`#[expect]`.

Therefore the existing 5th parameter is **replaced in place**, keeping arity at 5:

```rust
/// Callbacks surfacing parser progress to an external observer.
/// All fields are optional; `ParseCallbacks::default()` observes nothing.
#[derive(Clone, Default)]
pub struct ParseCallbacks {
    /// Invoked exactly once per input file after it has been fully processed
    /// (successful parse or parse error alike).
    pub on_file_parsed: Option<FileParsedCallback>,
    /// Invoked exactly once, with the final entity count, after post-parse
    /// aggregation and *before* any entity is pushed to the bounded channel.
    pub on_entities_extracted: Option<EntitiesExtractedCallback>,
}

pub type EntitiesExtractedCallback = std::sync::Arc<dyn Fn(usize) + Send + Sync>;

pub fn parse_files_stream(
    files: &[PathBuf],
    parse_cfg: &ParseConfig,
    sender: mpsc::Sender<ParsedEntity>,
    max_concurrent: usize,
    callbacks: Option<ParseCallbacks>,   // was: Option<FileParsedCallback>
) { ... }
```

This keeps every existing `None` call site compiling untouched (see §7).

---

## 4. Blast Radius

### Call sites of `parse_files_stream`

| Location | Current arg | Action |
|---|---|---|
| `src/pipeline/runner.rs:186` | `Some(on_file_parsed)` | **update** — pass `ParseCallbacks` with both hooks |
| `src/pipeline/parser/mod.rs:172` (`parse_files`) | `None` | unchanged |
| `src/pipeline/parser/mod.rs:720` | `None` | unchanged |
| `src/pipeline/parser/mod.rs:740` | `None` | unchanged |
| `src/pipeline/parser/mod.rs:1101` | `Some(cb)` | **update** |
| `src/pipeline/parser/mod.rs:1141` | `Some(cb)` | **update** |
| `src/pipeline/parser/mod.rs:1163` | `None` | unchanged |
| `benches/pipeline_bench.rs:55` | `None` | unchanged |
| `benches/pipeline_bench.rs:163` | `None` | unchanged |
| `benches/channel_backpressure_bench.rs:65` | `None` | unchanged |
| `benches/channel_backpressure_bench.rs:112` | `None` | unchanged |

Only **3 call sites** need editing.

### Existing tests whose expectations change

| Test | File | Why |
|---|---|---|
| `test_percent_basic` | `progress.rs:144` | asserted `20.0` under the old linear formula; becomes `2.0` under the parse band |
| `test_percent_clamped` | `progress.rs:249` | asserted `100.0` on over-count; becomes `PARSE_BAND_END` |
| `test_begin_run_resets` | `progress.rs:185` | must additionally assert the new fields reset |
| `test_serialize_snapshot_json` | `progress.rs:234` | must assert the new JSON field |

### Downstream (knot-server)

`IndexingProgress` is built with struct literals in knot-server tests at
`src/handlers/progress.rs:190`, `:215`, `:235` — adding a public field breaks
compilation there. See §8.

---

## 5. Semver Note (requires acknowledgement)

The target version is **1.6.2**, a patch release. Two of these changes are
technically **breaking** for downstream crates:

1. Adding `pub total_entities: u64` to `IndexingProgress` breaks any downstream
   struct-literal construction (knot-server's tests do exactly this).
2. Changing the type of `parse_files_stream`'s 5th parameter breaks any caller
   passing `Some(callback)`.

Both are mitigated in practice — knot-server is the only known consumer and is
updated in lockstep (§8) — but a strict reading of semver would call for
`1.7.0`. Shipping as `1.6.2` is a deliberate, documented decision. If a third
party pins `knot = "1.6"` they will get a compile error on upgrade.

---

## 6. TDD/BDD Plan

Every step is **Red → Green → Refactor**. No production line is written before a
failing test justifies it. Unit test names follow Given/When/Then.

---

### Step 1 — Tracker contract: recording the entity total

**File:** `src/pipeline/progress.rs` (`mod tests`, line 126)

#### Red

```
given_new_tracker_when_snapshot_then_total_entities_is_zero_and_unknown
given_parse_finished_when_total_entities_recorded_then_snapshot_exposes_it
given_a_populated_tracker_when_begin_run_then_total_entities_is_reset   (extend test_begin_run_resets:185)
```

These fail to compile — `IndexingProgress` has no `total_entities`, and
`ProgressTracker` has no `set_total_entities`. Compile failure is a valid Red.

#### Green

- `ProgressTracker`: add `total_entities: AtomicU64` and
  `total_entities_known: AtomicBool`.
- `pub(crate) fn set_total_entities(&self, n: u64)` — stores value then sets the
  flag (`Ordering::Release` on the flag, `Acquire` on read, so a reader never
  observes `known == true` with a stale value).
- `IndexingProgress`: add `pub total_entities: u64`.
- `begin_run` (`progress.rs:82-90`): reset both.
- `new()` / `Default`: initialise both.

---

### Step 2 — The banded formula

**File:** `src/pipeline/progress.rs`

#### Red

New tests:

```
given_half_the_files_parsed_when_snapshot_then_percent_is_half_the_parse_band
    → total_files=5000, parsed=2500  ⇒ 5.0

given_all_files_parsed_but_nothing_ingested_when_snapshot_then_percent_stays_in_parse_band
    → total_files=3713, parsed=3713, no total_entities  ⇒ 10.0   ← REGRESSION TEST FOR THIS BUG

given_half_the_entities_ingested_when_snapshot_then_percent_is_mid_ingest_band
    → total_entities=1000, ingested=500  ⇒ 50.0

given_all_entities_ingested_when_snapshot_then_percent_is_ingest_band_ceiling
    → ⇒ 90.0

given_zero_entities_parsed_when_snapshot_then_percent_jumps_to_ingest_band_ceiling
    → known=true, total_entities=0  ⇒ 90.0

given_more_entities_ingested_than_expected_when_snapshot_then_percent_is_clamped
    → total_entities=100, ingested=150  ⇒ 90.0

given_resolving_references_stage_when_snapshot_then_percent_is_resolving_constant
    → ⇒ 95.0

given_failed_stage_when_snapshot_then_percent_reflects_work_done_so_far
    → not forced to 0 or 100
```

Updated existing tests: `test_percent_basic:144` (`20.0` → `2.0`),
`test_percent_clamped:249` (`100.0` → `10.0`).

Unchanged and still green: `test_percent_zero_total:156`,
`test_completed_forces_100:164`, `test_fail_records_error:173`,
`test_concurrent_increments:204`, `test_record_batch_accumulates:223`.

#### Green

Rewrite the `let pct = ...` block at `progress.rs:61-68` following the
resolution order in §3.2.

#### Refactor

The branch cascade will push `snapshot()` toward
`clippy::cognitive_complexity` (threshold 15). Extract a free, pure function —
testable without a tracker and trivially readable:

```rust
struct PercentInputs {
    stage: IndexingStage,
    total_files: u64,
    parsed_files: u64,
    total_entities: Option<u64>,
    entities_ingested: u64,
}

fn compute_percent(inputs: &PercentInputs) -> f32 { ... }
```

Grouping into a struct also keeps the helper under
`too-many-arguments-threshold = 5`. **No `#[allow]` / `#[expect]` may be
introduced by this work** — if a lint fires, the code gets restructured.

---

### Step 3 — Monotonicity property

#### Red

```
given_a_full_pipeline_sequence_when_progressing_then_percent_never_decreases
```

Drive the tracker through the real sequence and snapshot at every transition:

```
begin_run → Discovering → set_total_files(3713) → Parsing
  → 3713 × incr_parsed_files          (snapshot every 500)
  → set_total_entities(83_456)
  → 652 × record_batch_ingested(128)  (snapshot every batch)
  → set_stage(ResolvingReferences)
  → complete()
```

Assert `pct[i] >= pct[i-1]` for every consecutive pair, and `pct.last() == 100.0`.

This is the user-facing guarantee: **the bar never goes backwards.** It is also
the test that catches badly chosen band boundaries.

#### Green

Adjust band constants / clamping until the property holds.

---

### Step 4 — The parser publishes the entity total

**File:** `src/pipeline/parser/mod.rs` (`mod tests`, line 643)

Model the new tests on the existing callback tests at `:1073`, `:1112`, `:1147`.

#### Red

```
given_files_to_parse_when_stream_completes_then_entities_extracted_receives_the_total
    → assert the reported count equals the number of entities read off the channel

given_a_saturated_channel_when_parsing_completes_then_total_is_published_before_blocking
    → channel capacity 1, no consumer, run parse_files_stream on a background
      thread; assert the callback has fired while the sender is still blocked.
      THIS IS THE HEART OF THE FIX — it encodes the exact failure scenario.

given_default_parse_callbacks_when_parsing_then_no_observer_is_invoked
    → ParseCallbacks::default() must be a no-op (regression guard, mirrors
      test_parse_files_stream_none_callback:1147)

given_only_on_file_parsed_set_when_parsing_then_it_still_fires_once_per_file
    → back-compat of the migrated behaviour
```

#### Green

- Add `EntitiesExtractedCallback` and `ParseCallbacks` (§3.5).
- Change the `parse_files_stream` signature (`mod.rs:92-98`).
- Inside the scope loop (`mod.rs:126`, `:134-136`), clone/invoke
  `callbacks.on_file_parsed`.
- After `aggregate_varnish_builtin_subs` (`mod.rs:152`) and **before** the
  `for entity in entities` send loop (`mod.rs:154`), invoke
  `on_entities_extracted(entities.len())`.
- Update the two `Some(cb)` test call sites (`:1101`, `:1141`).
- Update the doc comment on `FileParsedCallback` (`mod.rs:79-81`).

---

### Step 5 — Wire it into the runner

**File:** `src/pipeline/runner.rs` — no new unit tests; covered by Steps 1–4 and
the E2E run in Step 7.

- `:27` — import `ParseCallbacks` / `EntitiesExtractedCallback`.
- `:178-192` — build a `ParseCallbacks` carrying both hooks; the new one calls
  `progress.set_total_entities(n as u64)`.
- `:312-322` — rewrite the `[Progress]` log so it is actually informative during
  the long ingestion phase:

  ```
  [Progress] [opencode] 62.3% — files 3713/3713, entities 51200/83456, batch #400 (128 entities)
  ```

- `:366-373` — update the final log line the same way.

---

### Step 6 — Documentation and release metadata

- `Cargo.toml:3` — `version = "1.6.2"`.
- `CHANGELOG.md` — new `## v1.6.2 — Accurate Indexing Progress` section at the
  top, matching the existing prefix style (`Fix(progress)`, `Feat(parser)`,
  `Test(progress)`, `Docs`). Must call out the semver caveat from §5.
- `README.md` — update any documented progress semantics; state explicitly that
  `percent_complete` now spans the whole pipeline, and document the band table
  and the new `total_entities` field.

---

### Step 7 — Verification gates

Run via the `validator` subagent, in this order:

1. `cargo fmt`
2. `cargo clippy --all-targets --all-features -- -D warnings`
3. `cargo test`
4. `./tests/run_all_e2e_fast.sh` (plus `./tests/run_rust_e2e.sh` for a full run)

**Policy reminder:** any warning from `fmt`/`clippy` must be resolved by
refactoring. Introducing `#[allow(...)]` is prohibited; `#[expect(..., reason)]`
only as a genuine last resort and must be flagged to the maintainer. This work
is not expected to need either.

**Manual acceptance check:** index a large repository and poll
`/api/repos/{id}/progress` every 2 s. Record the curve. Acceptance requires:

- the value climbs gradually over the whole run,
- it does **not** exceed 10 % before `Stage 2: Parallel parsing complete.`,
- it reaches 100 % only once the run genuinely terminates.

---

## 7. Acceptance Criteria

- [ ] `percent_complete` reflects the whole pipeline, not just file parsing.
- [ ] With every file parsed and nothing ingested, the value is `10.0`, not `100.0`.
- [ ] `100.0` is reported **only** for `IndexingStage::Completed`.
- [ ] The percentage is monotonically non-decreasing across a full run.
- [ ] `total_entities` is exposed in `IndexingProgress` and its JSON form.
- [ ] `parse_files_stream` keeps arity 5; all `None` call sites compile untouched.
- [ ] `cargo clippy --all-targets --all-features -- -D warnings` is clean, with
      no new `#[allow]` or `#[expect]` anywhere.
- [ ] E2E suite passes.

---

## 8. Downstream Follow-up — knot-server 0.3.2

Tracked here for coordination; executed in the `knot-server` repository.

| File | Change |
|---|---|
| `Cargo.toml` | `knot = "1.6.2"`; bump `version` to `0.3.2` |
| `src/handlers/progress.rs:14-24` | add `total_entities: u64` to `ProgressResponse` |
| `src/handlers/progress.rs:26-52` | map the field in `build_progress_response` |
| `src/handlers/progress.rs:190,215,235` | update `IndexingProgress` literals (compile-breaking) |
| `src/progress_store.rs` | add the field to `PersistedProgress` + `from_tracker`; round-trip test |
| `src/metrics.rs` | expose it from `set_indexing_progress` |
| `README.md` | update the `/api/progress` payload documentation |

New downstream tests:

```
given_a_snapshot_with_total_entities_when_building_response_then_the_field_is_exposed
given_a_persisted_snapshot_when_round_tripped_through_json_then_total_entities_survives
```

---

## 9. Rejected Alternatives

**Patch only in knot-server.** Rescale `percent_complete` at the API layer using
`stage` plus an asymptotic curve. Rejected: without `total_entities` no accurate
figure is possible, only a plausible-looking fake; and the knot CLI would keep
reporting the wrong number.

**Derive the total from `files_to_parse.len()`.** Entities per file vary by one
to two orders of magnitude across languages, so the estimate would be worthless.

**Emit entities from the worker threads instead of buffering.** Would let the
counter advance naturally, but `aggregate_varnish_builtin_subs`
(`mod.rs:152`) requires the complete global entity set before dispatch. Removing
the buffer is a much larger change and is out of scope here.

**Unbounded channel.** Would unblock the parser and let `parsed_files` be
meaningful, but at the cost of holding the entire entity set in memory — the
bounded channel exists deliberately for backpressure (`runner.rs:152-153`).
