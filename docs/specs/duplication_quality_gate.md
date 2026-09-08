# Code Duplication Quality Gate (`cargo-dupes`)

**Status:** Approved, not yet implemented
**Target version:** v1.9.1 (or next minor after approval)
**Author:** Raul Tovar
**Date:** 2026-09-08
**Scope:** Phases 1, 3 and 4 (Phase 2 — clearing the backlog — is deferred, see §11)

---

## 1. Objective

Add a fifth quality gate to `knot` that blocks **new** duplicated Rust code, using
[`cargo-dupes`](https://crates.io/crates/cargo-dupes) as the detector.

The gate must behave like the existing gates in this repo:

- thresholds live in a dedicated, **commented** config file with the measured curve
  (same convention as `clippy.toml`);
- suppressions are **per-item, justified, and self-cleaning** (same spirit as the
  `#[expect(..., reason = "…")]` policy, which forbids bare `#[allow]`);
- the gate is green on the current tree from the first commit — it is a *ratchet*
  against regressions, not a mass-refactor mandate.

### In scope

| Phase | Deliverable |
|-------|-------------|
| 1 | `dupes.toml` at repo root, calibrated so the gate passes on today's tree |
| 3 | `cargo dupes check` wired into `.github/workflows/ci.yml` and `.github/workflows/release.yml`; `/ship` command extended |
| 4 | `AGENTS.md`, `CONTRIBUTING.md`, `README.md`, `CHANGELOG.md` updated |

### Out of scope

- Phase 2 (refactoring the 19 pre-existing exact-duplicate groups down to zero) —
  tracked in §11 as a follow-up backlog.
- `--sub-function` mode (see §4.4, rejected).
- Adding the gate to the global `validator` subagent (`~/.config/opencode/opencode.json`).
  Decision: CI + `/ship` only.

---

## 2. Decisions and rationale

| # | Decision | Rationale |
|---|----------|-----------|
| D1 | Adopt `cargo-dupes`, pinned to `=0.2.1` | Only maintained AST-normalizing duplicate detector for Rust on crates.io; MIT; dev-only tool, never enters knot's dependency tree |
| D2 | Config in `dupes.toml`, **not** `[package.metadata.dupes]` | Symmetry with `clippy.toml`; allows documenting the measured curve inline; keeps `Cargo.toml` focused on the package |
| D3 | Ratchet baseline (`max_exact_duplicates = 8`, `max_near_duplicates = 9`), not `0` | Green from day one; blocks new duplicate groups. Lowering to 0 is Phase 2 work with real refactor risk over parsers and `db/graph` |
| D4 | Gate runs in CI (`test-unit` of both workflows) and in `/ship` Step 7 | Same two enforcement points as `fmt`/`clippy`/`test`. The `validator` subagent stays untouched (user's global config) |
| D5 | `/ship` runs the gate **conditionally**, only if `dupes.toml` exists | `/ship` is a generic Rust command used across repos; it must not fail on repos that never adopted the tool |
| D6 | `--sub-function` disabled | 275 extra groups of noise and `check` exposes no thresholds for them (see §4.4) |
| D7 | Pin the version in CI (`--version 0.2.1 --locked`) | Fingerprints are a function of the normalization algorithm; an upgrade could invalidate every ignore entry and break CI without any code change |

---

## 3. How the tool works (contract we depend on)

`cargo-dupes` parses each `.rs` file with `syn` and normalizes every code unit into a
canonical AST:

- identifiers → positional placeholders (`foo(x)` ≡ `bar(y)`);
- **literal values erased, types preserved** (`42` ≡ `99`, `"a"` ≡ `"bbb"`);
- control flow preserved exactly;
- macro invocations become opaque nodes.

The normalized tree is hashed into a 16-hex-char **fingerprint** (exact duplicates) and
compared tree-by-tree with the Dice coefficient (near duplicates).

**Code units analyzed:** top-level `fn`, methods in `impl` blocks, `fn` in
`impl Trait for Type` blocks, and closures above the node threshold.

**Automatic skips:** `target/`, hidden directories, plus configured `exclude` patterns.
Unparseable files are skipped with a warning.

### 3.1 Subcommands

| Command | Behaviour |
|---------|-----------|
| `stats` | Statistics only (`--format json` available) |
| `report` | Full report with fingerprints, members, file:line ranges |
| `check` | Prints stats + report, exits **1** when any threshold is exceeded |
| `ignore <fp> --reason "…"` | Appends an entry to `.dupes-ignore.toml` |
| `ignored` | Lists ignored fingerprints with reasons |
| `cleanup [--dry-run]` | Removes/lists ignore entries whose fingerprint no longer exists |

**Exit codes:** `0` pass · `1` thresholds exceeded · `2` error (no sources, bad path).

### 3.2 Config precedence

1. CLI flags
2. `dupes.toml` in the project root
3. `[package.metadata.dupes]` in `Cargo.toml`

Verified: `dupes.toml` is resolved relative to the **current working directory**, so CI
must invoke the tool from the repo root (it does).

### 3.3 Verified limitations

- `cleanup --dry-run` **exits 0** even when stale entries exist — it is informational
  only and cannot be a hard gate by itself (§7.3 handles this).
- Group *ordering* in `report` output is not stable across runs; **fingerprints are**.
  Never assert on group numbers, only on fingerprints.
- `exclude` is a plain substring match on the path. `"tests/"` is precise;
  `"tests"` would also swallow `src/**/tests.rs` (sometimes desirable — see §6.2).
- `exclude_tests` only sees the file it parses. Helper functions inside
  `src/**/tests.rs` are **not** recognized as test code because the `#[cfg(test)]`
  attribute lives on the parent's `mod tests;` declaration
  (`src/pipeline/parser/extractor/mod.rs:13`). This is why `"tests.rs"` is an explicit
  exclude pattern in §6.1.

---

## 4. Measurements on knot (2026-09-08)

Environment: `cargo-dupes 0.2.1`, knot @ `c0a7d07` (v1.9.0), 136 `.rs` files,
54 235 lines under `src/`.

Reproduce with (tool installed out-of-tree, see §12):

```bash
cargo-dupes dupes --path . --exclude-tests \
  --exclude "tests/" --exclude "benches/" --exclude "tests.rs" \
  --min-lines <N> --format json stats
```

### 4.1 Runtime

**0,27 s** for the whole repository. Negligible next to `cargo clippy`; the gate adds
no meaningful CI time beyond installing the tool (§7.2).

### 4.2 The `min_lines` curve (with the §6.1 exclusions)

| `min_lines` | units | exact groups | near groups | exact lines | near lines | exact % |
|------------:|------:|-------------:|------------:|------------:|-----------:|--------:|
| 0 | 861 | 38 | 5 | 965 | 131 | 4,43 % |
| 5 | 760 | 23 | 3 | 891 | 120 | 4,13 % |
| **10** | **631** | **19** | **2** | **822** | **91** | **3,98 %** |
| 15 | 521 | 13 | 2 | 670 | 91 | 3,46 % |
| 20 | 405 | 5 | 1 | 360 | 56 | 2,07 % |

`min_lines = 10` is the chosen point: 19 groups is a triageable backlog, and below
10 lines a "duplicate" is rarely worth extracting into a helper.

### 4.3 Why tests must be excluded

| Configuration | exact groups | exact % |
|---------------|-------------:|--------:|
| No exclusions at all | 246 | 16,71 % |
| `exclude_tests` only | 53 | 5,55 % |
| `exclude_tests` + `tests/` + `benches/` | 20 | 4,22 % |
| `exclude_tests` + `tests/` + `benches/` + `tests.rs` (**chosen**) | 19 | 3,98 % |

Test code is structurally repetitive by design (arrange/act/assert). Gating on it would
produce a permanent stream of false positives. The 20th group that the `tests.rs`
pattern removes is a 6-member group of test accessor helpers
(`type_ref_names`, `call_method_names`, `type_reference_names`, `value_reference_names`,
`extends_parent_names`, `implements_interface_names`) in
`src/pipeline/parser/extractor/tests.rs` and `src/pipeline/parser/languages/rust/tests.rs`.

### 4.4 `--sub-function` mode: rejected

With `-s`, knot reports **275** sub-exact groups and 28 sub-near groups on top of the
function-level results. `cargo dupes check` exposes no `--max-sub-*` thresholds, so those
numbers cannot gate anything — they would only add noise to every CI log.

### 4.5 Near-duplicate threshold

Sweeping `--threshold` from 0.8 to 0.9 leaves the result unchanged (2 groups at 99 % and
92 % similarity). The default `0.8` is kept; no evidence justifies moving it.

---

## 5. Duplication policy (to be added to `AGENTS.md`)

Mirrors the existing `#[expect(..., reason)]` policy:

> When `cargo dupes check` fails, the conventional fix is **always** to remove the
> duplication by extracting a shared, parameterized helper.
>
> Adding a fingerprint to `.dupes-ignore.toml` is tolerated **only** when unifying the
> code units would make the code worse, and every entry MUST carry a `reason`
> explaining why. Two legitimate categories exist in knot today:
>
> 1. **Literal-differentiated units** — functions whose entire semantic content lives in
>    string literals that AST normalization erases (Cypher query builders in
>    `src/db/graph/query.rs`). Merging them would obscure the queries.
> 2. **Externally-imposed boilerplate** — trait implementations whose shape is dictated
>    by a third-party crate (`Tool::tool` declarations required by `rust-mcp-sdk`).
>
> Raising `max_exact_duplicates` / `max_near_duplicates` to accommodate new code is
> **prohibited**. The thresholds may only move downwards.
>
> Run `cargo dupes cleanup --dry-run` after any refactor; stale ignore entries must be
> removed in the same PR.

---

## 6. Phase 1 — `dupes.toml`

### 6.1 File to create: `dupes.toml` (repo root)

```toml
# Duplication thresholds for knot (cargo-dupes 0.2.1).
#
# Counts below were measured on 2026-09-08 against v1.9.0 with the
# exclusions configured in this same file:
#
#   cargo dupes stats
#
# NOTE: cargo-dupes normalizes the AST (identifiers become positional
# placeholders and literal VALUES are erased, though their types are kept).
# Two functions that differ only in the string literals they embed therefore
# collapse into one fingerprint. That is a known false-positive class in this
# repo (Cypher builders in src/db/graph/query.rs) and is handled through
# .dupes-ignore.toml, never by raising the thresholds.

# --- Analysis scope ---

# Minimum source lines for a code unit to be considered.
# Measured curve (exact duplicate groups): 0 -> 38 | 5 -> 23 | 10 -> 19
#                                          15 -> 13 | 20 -> 5
# Below 10 lines an extracted helper is rarely an improvement, and the group
# count explodes. 10 is the useful point. Later target: keep at 10 and drive
# the group count to 0 instead.
min_lines = 10

# Minimum AST node count. Left at the tool default; min_lines is the axis that
# actually discriminates useful findings in this codebase.
min_nodes = 10

# Similarity threshold for near duplicates. Sweeping 0.8 -> 0.9 does not change
# the result on this tree (both hits sit at 92% and 99%), so the default stands.
similarity_threshold = 0.8

# --- Exclusions ---

# Test code is structurally repetitive by design (arrange/act/assert) and
# gating on it yields permanent false positives: without exclusions the repo
# reports 246 groups / 16.7%, with them 19 groups / 4.0%.
#
# exclude_tests drops #[test] functions and #[cfg(test)] modules, but it only
# sees the file being parsed: helpers in src/**/tests.rs are invisible to it
# because the #[cfg(test)] attribute lives on the parent's `mod tests;`
# declaration. Hence the explicit "tests.rs" pattern.
#
# Patterns are plain substring matches on the path; the trailing slash on
# "tests/" and "benches/" keeps them from matching unrelated files.
exclude_tests = true
exclude = ["tests/", "benches/", "tests.rs"]

# --- CI thresholds (consumed by `cargo dupes check`) ---
#
# RATCHET: these are the counts of the pre-existing backlog documented in
# docs/specs/duplication_quality_gate.md. They exist to block the 20th group,
# not to bless the first 19. They may only ever be lowered.
max_exact_duplicates = 8
max_near_duplicates = 9
```

`max_exact_percent` / `max_near_percent` are deliberately left unset: group count is the
actionable metric, and a percentage threshold would silently loosen as the codebase grows.

### 6.2 Acceptance criteria for Phase 1

```bash
cargo dupes check            # exit 0
cargo dupes stats            # exact_duplicate_groups = 19, near_duplicate_groups = 2
```

A deliberate regression test (temporarily copy-pasting a 12-line function) must make
`cargo dupes check` exit 1.

---

## 7. Phase 3 — CI and `/ship` integration

### 7.1 `.github/workflows/ci.yml` — job `test-unit`

Append to the existing job (after `Run Unit Tests`, line 88); **do not create a new job**
(a separate job would pay the checkout + toolchain cost again for a 0,27 s check):

```yaml
      - name: Cache cargo-dupes
        uses: actions/cache@v4
        with:
          path: ~/.cargo/bin/cargo-dupes
          key: ${{ runner.os }}-cargo-dupes-0.2.1

      - name: Install cargo-dupes
        run: test -x ~/.cargo/bin/cargo-dupes || cargo install cargo-dupes --version 0.2.1 --locked

      - name: Duplication Check
        run: cargo dupes check

      - name: Stale duplication suppressions (informational)
        run: cargo dupes cleanup --dry-run
```

Notes:

- The `test -x` guard is required: with only `~/.cargo/bin/cargo-dupes` restored from
  cache, `cargo install` would still rebuild because `~/.cargo/.crates2.json` is absent.
- The cache key embeds the version, so bumping `0.2.1` automatically invalidates it.
- `test-unit` is skipped on `release:` commits by its own `if:` guard; that is correct —
  `release.yml` carries its own gate (§7.2).

### 7.2 `.github/workflows/release.yml` — job `test-unit`

Same three steps appended after `Run unit tests` (line 106). This is the job that blocks
tag → GitHub Release.

**Maintenance warning:** `release.yml` is autogenerated by `cargo-dist` and already
hand-edited to add this `test-unit` gate; `allow-dirty = ["ci"]` in `dist-workspace.toml`
protects the hand-edits. Add the steps **inside the existing job** and leave the
`HAND-WRITTEN ADDITION` banner intact.

### 7.3 Cost

`cargo install cargo-dupes --version 0.2.1 --locked` from a cold registry: **11 s wall**
on a 5-core dev machine (54 s CPU); expect 40–60 s on a 2-core GitHub runner, and ~0 s on
every subsequent run thanks to the cache. The check itself is 0,27 s.

Because `cleanup --dry-run` exits 0 even when stale entries exist (§3.3), it is a
**reporting** step, not a gate. If it later needs teeth, the fix is:

```yaml
        run: cargo dupes cleanup --dry-run | tee /tmp/dupes-cleanup.txt
             ! grep -q "stale entries would be removed" /tmp/dupes-cleanup.txt
```

Not adopted now: stale entries are a hygiene issue, not a correctness one, and a false
CI failure on a fingerprint drift would be worse than a log line.

### 7.4 `/ship` command (`~/.config/opencode/commands/ship.md`)

Rewrite **Step 7 — Quality gate (Rust only)** so the duplication check runs whenever the
repo has opted in:

````markdown
## Step 7 — Quality gate (Rust only)

Before pushing, run the full quality suite sequentially:

```bash
cargo fmt -- --check
cargo clippy --all-targets -- -D warnings
cargo test
```

If the repository contains a `dupes.toml` at its root, the project has opted into the
duplication gate. In that case also run:

```bash
test -x "$(command -v cargo-dupes)" || cargo install cargo-dupes --locked
cargo dupes check
cargo dupes cleanup --dry-run
```

If `dupes.toml` is absent, skip this check entirely and do not install anything —
the project has not adopted the gate.

If any step fails:
1. Fix the issue (auto-fix with `cargo fmt` or `cargo clippy --fix` where safe).
2. For `cargo dupes check` failures, the fix is to extract a shared helper. Adding the
   fingerprint to `.dupes-ignore.toml` (with `--reason`) is only acceptable when
   unifying the code units would make the code worse; raising the thresholds in
   `dupes.toml` is never acceptable.
3. Stage the fixes as a new `chore: apply fmt/clippy fixes` commit (do NOT amend).
4. Re-run the full suite until green.
````

Also update **Step 11 — Summary** to add a `Duplication : pass / skipped (no dupes.toml)`
row.

Note: `/ship` uses an unpinned `cargo install cargo-dupes --locked` on purpose — it is a
developer convenience across many repos, while CI pins the version for reproducibility.
If a future version shifts fingerprints, CI is the authority and `/ship` is corrected to
match.

### 7.5 Acceptance criteria for Phase 3

- CI on a scratch branch is green and the `Duplication Check` step reports 19/2.
- A branch with an intentional duplicate fails CI at `Duplication Check`, not later.
- `/ship` on a repo *without* `dupes.toml` runs the three classic gates and prints
  `Duplication : skipped (no dupes.toml)`.

---

## 8. Phase 4 — Documentation

| File | Change |
|------|--------|
| `AGENTS.md` | Add `cargo dupes check` as gate #4 in **Code Quality Requirements**; add the §5 policy block next to the `#[expect]` policy; mention `dupes.toml` in **Repo-Specific Constraints** |
| `AGENTS.md` (Quick Commands) | Add `cargo dupes check` / `cargo dupes report` to the code-quality block |
| `CONTRIBUTING.md` | Add a bullet after the clippy one: "No new duplicated code is introduced (`cargo dupes check`); suppressions require a documented `reason`" + a one-liner on installing the tool |
| `README.md` | Document the gate in the development/quality section, including the `cargo install cargo-dupes --version 0.2.1 --locked` prerequisite |
| `CHANGELOG.md` | `### Added — Code duplication quality gate (cargo-dupes) with ratcheted thresholds` under the next version |
| `docs/specs/duplication_quality_gate.md` | This document; update §11 as backlog items are cleared |

`.dupes-ignore.toml` is **not** created in Phase 1 (there is nothing to ignore yet while
the thresholds carry the whole backlog). It appears in Phase 2.

---

## 9. Execution plan

| Step | Action | Verification |
|------|--------|--------------|
| 1 | Install the tool locally: `cargo install cargo-dupes --version 0.2.1 --locked` | `cargo dupes --version` → `0.2.1` |
| 2 | Create `dupes.toml` (§6.1) | `cargo dupes check` exits 0; `stats` reports 19/2 |
| 3 | Regression-test the gate: duplicate a ≥10-line function on a scratch commit | `cargo dupes check` exits 1 naming the new group; revert |
| 4 | Patch `.github/workflows/ci.yml` (§7.1) | `actionlint` clean or YAML parses |
| 5 | Patch `.github/workflows/release.yml` (§7.2), banner intact | `git diff` touches only the `test-unit` job |
| 6 | Patch `~/.config/opencode/commands/ship.md` (§7.4) | Manual read-through |
| 7 | Docs (§8) | — |
| 8 | Full local gate: `cargo fmt -- --check && cargo clippy --all-targets -- -D warnings && cargo test && cargo dupes check` | all green |
| 9 | Push to a branch, confirm CI green, then merge | CI `test-unit` shows the new steps |

Commit split (for `/ship`):

1. `chore(quality): add cargo-dupes duplication thresholds in dupes.toml`
2. `chore(ci): gate CI and release on cargo dupes check`
3. `docs: document the duplication quality gate`

`~/.config/opencode/commands/ship.md` lives outside the repo and is not part of any commit.

---

## 10. Risks and mitigations

| # | Risk | Severity | Mitigation |
|---|------|----------|------------|
| R1 | Crate is young (published 2026-02, 0.2.x, single maintainer, ~7k downloads) | Medium | Dev-only tool, never linked into knot; MIT; version pinned; the gate can be dropped by deleting one file and three CI steps |
| R2 | A version bump changes the normalization → all fingerprints go stale, CI reddens with no code change | High | Pin `--version 0.2.1` in both workflows; upgrades are an explicit PR that re-runs `stats` and refreshes `.dupes-ignore.toml`; `cleanup --dry-run` surfaces the drift |
| R3 | Literal-erasure false positives (Cypher builders, MCP tool boilerplate) push contributors toward harmful "de-duplication" | Medium | §5 policy documents both false-positive classes explicitly and requires a `reason` on every ignore |
| R4 | `exclude` substring matching silently drops a future path containing `tests`/`benches` | Low | Patterns use trailing slashes where possible; the unit count (631) is recorded here as a canary — a sudden drop means an over-broad exclude |
| R5 | CI time regression | Negligible | 0,27 s check + cached install |
| R6 | The ratchet ossifies: 19 groups stay forever | Medium | §11 tracks the backlog with owners per area; thresholds may only be lowered (§5) |
| R7 | `cargo install` from crates.io fails in a restricted CI network | Low | Same failure mode as the existing `cargo` steps; the registry cache already covers the common case |

---

## 11. Phase 2 backlog (deferred)

The 19 exact groups and 2 near groups present at v1.9.0. Classification is
**preliminary** except where marked *verified*; each item must be re-judged when it is
picked up. Fingerprints are stable identifiers for `cargo dupes ignore`.

### Genuine duplication — extract a shared helper

| Fingerprint | Members | Location |
|-------------|---------|----------|
| `eddfd4cb1bd6fe67` | `extract_reference_intents_java` / `_kotlin` / `_cpp` (3) | `java.rs:201`, `kotlin.rs:417`, `cpp.rs:69` |
| `7d773be4fffe7141` | `extract_call_intents_java` / `_kotlin` | `java.rs:219`, `kotlin.rs:435` |
| `e96a5decaa13a7de` (near, 99 %) | `extract_identifiers_from_annotation` ×2 | `java.rs:171`, `kotlin.rs:158` |
| `28725ac517d7ada7` | `extract_decorator_references` / `extract_annotation_references` | `parser/utils.rs:29`, `kotlin.rs:137` |
| `abe3d163ebc5e2f3` | `find_child_by_kind` / `first_child_of_kind` | `java.rs:399`, `csharp/refs.rs:689` |
| `4fdd8c5dec8898bf` *verified* | `extract_gradle_group` / `extract_gradle_version` — differ only in the keyword | `gradle.rs:97`, `gradle.rs:112` |
| `5095104136fb7dd4` | `scan_default_export_target` / `scan_module_exports_target` | `typescript.rs:510`, `javascript/imports.rs:206` |
| `a0ae22ce20ad8579` | `handle_css_scss_capture` / `handle_html_capture` | `extractor/captures.rs:449`, `:467` |
| `02145df5bd2b6d7b` | `Lexer::scan_long_string` / `Lexer::scan_block_comment` | `varnish/lexer.rs:181`, `:217` |
| `6abbc1604522cec6` | `VtcContext::handle_client` / `handle_logexpect` | `varnish/vtc.rs:96`, `:138` |
| `6f4aa7e2d5f0bac0` | `init_logging` / `init_logging_for_cli` | `utils/mod.rs:16`, `:43` |
| `900ee0de0aad52df` | `format_callers_output` / `format_explore_output` | `utils/mod.rs:131`, `:150` |
| `a1c0a5b9a46615ad` | `lookup_fqn` / `lookup_fqn_by_fqn` | `ingest/resolve/calls.rs:114`, `:245` |
| `ba06b1ae45fab715` | `delete_by_repo` / `delete_repository` | `db/graph/delete.rs:21`, `:74` |
| `e1d3941739b6186e` | `run_indexing_pipeline` / `setup_watch_mode` | `pipeline/runner.rs:36`, `pipeline/watch.rs:30` |
| `5b95c89175259497` (near, 92 %) | `resolve_plain` / `resolve_typed` | `ingest/resolve/mod.rs:406`, `:425` |

### Probable false positives — candidates for `.dupes-ignore.toml`

| Fingerprint | Members | Why |
|-------------|---------|-----|
| `063774d2011429cc` *verified* | `relationship_query` / `reference_target_query` (`query.rs:195`, `:368`) | Identical shape, entirely different Cypher; literals erased by normalization |
| `a5b9b106a6a71e41` | `overridden_by_query` / `overrides_query` (`query.rs:217`, `:243`) | Same class |
| `e77dd72a96fb81a1` | `get_file_entities_query` / `get_file_outgoing_references_query` (`query.rs:285`, `:305`) | Same class |
| `72fa93e52fbd2227` | `ListRepoDependenciesTool::tool` / `SearchHybridContextTool::tool` | Schema boilerplate imposed by `rust-mcp-sdk` |
| `9c4eef28574686ee` | `FindCallersTool::tool` / `ExploreFileTool::tool` | Same class |

**Phase 2 exit criteria:** every group above either refactored away or present in
`.dupes-ignore.toml` with a `reason`, then `max_exact_duplicates = 0` and
`max_near_duplicates = 0` in `dupes.toml`, with this section updated to reflect it.

Suggested PR split, to keep review surface small and E2E blast radius contained:
(a) parser language helpers, (b) `utils` + `pipeline` runners, (c) `db/graph`,
(d) `mcp_tools` ignores + threshold drop to 0.

---

## 12. Appendix — reproducing the measurements without installing globally

```bash
cargo install cargo-dupes --version 0.2.1 --locked --root /tmp/dupes-prefix
alias cd0='/tmp/dupes-prefix/bin/cargo-dupes dupes'

cd /path/to/knot
cd0 --path . --exclude-tests --exclude "tests/" --exclude "benches/" --exclude "tests.rs" \
    --min-lines 10 --format json stats
cd0 --path . --exclude-tests --exclude "tests/" --exclude "benches/" --exclude "tests.rs" \
    --min-lines 10 report
```

Once `dupes.toml` exists, all of the above collapses to `cargo dupes stats` /
`cargo dupes report` run from the repo root.
