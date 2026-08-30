# C# Language Support: TDD/BDD Implementation Plan

**Closes:** [#5 — help wanted: add C# support](https://github.com/raultov/knot/issues/5)
**Target version:** v1.7.0
**Roadmap phase:** Phase 15 (see `docs/specs/multilanguage_roadmap.md`)
**Baseline:** knot v1.6.2

---

## 1. Context and Problem Statement

`knot` currently indexes Java, Kotlin, TypeScript/TSX, JavaScript/JSX, Rust, Python,
Groovy, C/C++, HTML, CSS/SCSS, Markdown, Varnish VCL, plus build-system and
configuration formats. C# is absent, which excludes the entire .NET ecosystem from
semantic search, call-graph traversal, and impact analysis.

Issue #5 has been open since May 2026 with no implementation. This plan defines the
complete, test-driven path to close it.

**Goal:** index C# source files (`.cs`) with the same fidelity knot already provides
for Java and Kotlin — entity extraction, namespace-qualified FQNs, `CALLS`,
`EXTENDS`, `IMPLEMENTS`, `REFERENCES`, `CONTAINS` and `OVERRIDES` edges, XML doc
comments, and attributes — validated end-to-end through both the MCP server and the
CLI.

**Methodology:** strict BDD/TDD. E2E scenarios are authored first and must fail
(*Red*) before any production code is written. Unit tests precede each
implementation slice. Every phase ends with a defined green gate.

---

## 2. Grammar Evaluation

### 2.1 Crate selection

`tree-sitter-c-sharp = "0.23.5"` — published by the `tree-sitter` organisation, the
same maintainers as `tree-sitter-java`, which knot already depends on.

Verified empirically by downloading and inspecting the crate:

| Check | Result |
|---|---|
| Rust binding | `pub const LANGUAGE: LanguageFn` via `tree-sitter-language = "0.1"` — byte-for-byte the same pattern as `tree-sitter-java 0.23.5` |
| Grammar ABI | `LANGUAGE_VERSION 15` (`src/parser.c:17`) |
| ABI accepted by `tree-sitter 0.26.8` (current `Cargo.lock`) | 14 and 15 — `LANGUAGE_VERSION_WITH_PRIMARY_STATES 14`, `LANGUAGE_VERSION_WITH_RESERVED_WORDS 15` (`src/language.h:13-14`) → **compatible** |
| `tree-sitter-language` | `0.1.7` already present in `Cargo.lock` → **zero new transitive dependencies** |
| Named node types | 224 |
| Licence | MIT — same as every other grammar in the tree |
| Upstream reference query | `queries/tags.scm` ships with the crate and can seed `queries/csharp.scm` |

`tree-sitter-java` emits ABI 14 and works today; C# emits ABI 15, which is also
within range. **There is no ABI risk.**

Alternatives considered and rejected: `arborium-c-sharp` (different runtime, not
tree-sitter-compatible with knot's `Language` type), `ast-grep-tree-sitter-c-sharp`
(stale fork at 0.20.0), `treebank-grammar-csharp` (patched redistribution, no
upstream tracking).

### 2.2 Node/field coverage

Extracted from `src/node-types.json` of the crate. Coverage is materially better
than `tree-sitter-groovy` and comparable to `tree-sitter-java`:

```
compilation_unit                       (no fields)
namespace_declaration                  name=(alias_qualified_name|generic_name|identifier|qualified_name)
                                       body=(declaration_list)
file_scoped_namespace_declaration      name=(alias_qualified_name|generic_name|identifier|qualified_name)
using_directive                        name=(identifier)                     ← alias form only
class_declaration                      name=(identifier) body=(declaration_list)
interface_declaration                  name=(identifier) body=(declaration_list)
                                       type_parameters=(type_parameter_list)
struct_declaration                     name=(identifier) body=(declaration_list)
record_declaration                     name=(identifier) body=(declaration_list)
enum_declaration                       name=(identifier) body=(enum_member_declaration_list)
enum_member_declaration                name=(identifier) value=(expression)
method_declaration                     name=(identifier) parameters=(parameter_list)
                                       returns=(type) type_parameters=(type_parameter_list)
                                       body=(block|arrow_expression_clause)
constructor_declaration                name=(identifier) parameters=(parameter_list) body=(...)
destructor_declaration                 name=(identifier) parameters=(parameter_list) body=(...)
property_declaration                   name=(identifier) type=(type) accessors=(accessor_list)
                                       value=(arrow_expression_clause|expression)
field_declaration                      (no fields)                           ← gap
event_declaration                      name=(identifier) type=(type) accessors=(accessor_list)
event_field_declaration                (no fields)
delegate_declaration                   name=(identifier) parameters=(parameter_list)
                                       type=(type) type_parameters=(type_parameter_list)
operator_declaration                   operator=(...) parameters=(parameter_list) type=(type)
indexer_declaration                    parameters=(bracketed_parameter_list) type=(type)
                                       accessors=(accessor_list)
accessor_declaration                   name=(add|get|identifier|init|remove|set) body=(...)
local_function_statement               name=(identifier) parameters=(parameter_list)
                                       type=(type) type_parameters=(...)
invocation_expression                  function=(...) arguments=(argument_list)
member_access_expression               expression=(...) name=(generic_name|identifier)
object_creation_expression             type=(type) arguments=(argument_list)
                                       initializer=(initializer_expression)
base_list                              (no fields)                           ← gap
attribute                              name=(alias_qualified_name|generic_name|identifier|qualified_name)
attribute_list                         (no fields)
variable_declaration                   type=(type)
variable_declarator                    name=(identifier)
qualified_name                         name=(generic_name|identifier) qualifier=(...)
parameter_list                         name=(identifier) type=(type)
```

### 2.3 Grammar gaps requiring custom Rust logic

Four gaps. None are blocking; each maps to a pattern knot already uses in
`java.rs`, `kotlin.rs` or `cpp.rs`.

**Gap 1 — `base_list` does not distinguish inheritance from implementation.**
C# has no separate syntax: `class Foo : Bar, IBaz` is a single `base_list`. Java
exposes `superclass:` and `interfaces:` as distinct fields; C# does not. Requires a
heuristic (§3.3). This is the same class of problem `kotlin.rs` already solves for
`class Foo : Base(), Iface` by checking for a constructor invocation.

**Gap 2 — `field_declaration` has no `name` field.**
The name lives two levels down. Handler must descend
`field_declaration > variable_declaration > variable_declarator > name:(identifier)`.
Same shape applies to `event_field_declaration`.

**Gap 3 — `record_declaration` covers both `record class` and `record struct`.**
`grammar.js:367-381`:
```js
_record_declaration_initializer: $ => seq(
  repeat($._attribute_list),
  repeat($.modifier),
  'record',
  optional(choice('class', 'struct')),   // ← same node type for both
  field('name', $.identifier),
  ...
),
```
The handler must inspect the anonymous `struct` token to pick between
`CSharpRecord` and a struct-flavoured record.

**Gap 4 — `using_directive` only exposes `name` for the alias form.**
`grammar.js:190-204`:
```js
using_directive: $ => seq(
  optional('global'),
  'using',
  choice(
    seq(optional('unsafe'), field('name', $.identifier), '=', $.type),  // alias → has name
    seq(repeat(choice('static', 'unsafe')), $._name),                   // plain → no field
  ),
),
```
Plain `using System.Text;` must be read from the `qualified_name`/`identifier`
child rather than a field.

### 2.4 What works with no changes

XML documentation comments (`/// <summary>…`) require **zero** new code:
`strip_comment_markers` in `src/pipeline/parser/comments.rs:289` already strips the
`///` prefix, and the upward docstring pass at `comments.rs:38-79` already handles
consecutive `comment` nodes.

---

## 3. Design Decisions

### 3.1 Dedicated `CSharp*` EntityKinds

**Decision:** dedicated variants, not reuse of the generic
`Class`/`Interface`/`Method`/`Enum` kinds that Java uses.

**Rationale.** The header comment of `src/pipeline/ingest/resolve/overrides.rs:11-13`
states the constraint explicitly:

> **JVM guard** — a file-extension guard plus a kind allowlist. The generic
> `Class`/`Interface`/`Method` kinds are shared with TypeScript, so the extension
> guard is what keeps non-JVM languages at zero `OVERRIDES` edges.

Reusing the generic kinds would make C# entities indistinguishable from Java and
TypeScript ones in every kind-filtered query (`explore_file` buckets, subgraph
`visible_kinds`, search filters). Kotlin, Groovy, Rust, Python and C/C++ all define
prefixed kinds; C# follows that precedent.

**Cost:** four exhaustive matches must be extended per variant (§7.1).

### 3.2 Canonical FQN

```
<namespace>.<OuterType>.<NestedType>.<member>
```

Examples:

| Source | FQN |
|---|---|
| `namespace MyApp.Services;` + `class UserService` | `MyApp.Services.UserService` |
| … + `Task<UserDto> GetUserAsync(int id)` | `MyApp.Services.UserService.GetUserAsync` |
| `namespace MyApp { namespace Legacy { class Outer { class Inner } } }` | `MyApp.Legacy.Outer.Inner` |
| No namespace, `class Free` | `Free` |

**Computation requires two mechanisms, not one.** This is the single design point
where neither the Java model nor the C++ model suffices alone:

1. **File-level pre-pass** for `file_scoped_namespace_declaration` (C# 10+).
   `grammar.js:260-264` shows this node has **no `body` field** — types declared
   after it are *siblings* under `compilation_unit`, not descendants:
   ```js
   file_scoped_namespace_declaration: $ => seq(
     'namespace',
     field('name', $._name),
     ';',
   ),
   ```
   A parent-walk from an entity node will therefore never reach it. It must be
   discovered by scanning `compilation_unit` children once per file — exactly the
   shape of `java::extract_package_name` called from
   `src/pipeline/parser/extractor/mod.rs:55`.

2. **Ancestor walk** from `entity_node` for block-form `namespace X { … }` (which
   *does* have a `body` and nests) and for containing types. This is the
   `cpp::build_cpp_fqn` model wired at `src/pipeline/parser/extractor/enrich.rs:73-87`.

Both are needed; a file can legally use either namespace form, and nested types
always require the ancestor walk.

**Consequence for `extractor/mod.rs`:** the `java_package: &Option<String>`
parameter threaded through `extract_entities` → `enrich_and_create_entity` should be
renamed to `package_prefix` and made language-generic rather than adding a second
parallel parameter.

### 3.3 `base_list` disambiguation heuristic

| Declaring node | Rule |
|---|---|
| `interface_declaration` | every entry → `EXTENDS` (an interface can only extend) |
| `struct_declaration`, record-struct | every entry → `IMPLEMENTS` (a struct cannot inherit) |
| `class_declaration`, record-class | first entry → `EXTENDS` **unless** it matches `^I[A-Z]`; all remaining entries → `IMPLEMENTS` |

The class rule exploits two facts: C# requires the base class to be listed first,
and the `IPascalCase` interface prefix is near-universal in .NET (enforced by the
official naming guidelines and by default analyzer rules).

**Known failure mode:** non-idiomatic code such as `class Foo : Base, Comparable`
(interface without the `I` prefix) mislabels `Comparable` as… nothing — it is the
second entry, so it is correctly `IMPLEMENTS`. The genuine failure is
`class Foo : IWeird` where `IWeird` is actually an abstract *class*: it would be
labelled `IMPLEMENTS` instead of `EXTENDS`. This is documented as a known
limitation, and struct/interface declarations remain fully deterministic.

**Test contract:** E2E assertions are written against the *semantically correct*
edge, so that if the heuristic is later replaced by resolution-time correction
(cross-referencing the target entity's actual kind — see §14.3), the tests do not
change.

**Generic stripping:** base entries are stripped of type arguments before edge
creation (`IRepository<User>` → `IRepository`), matching the Java behaviour
introduced in v1.3.6.

### 3.4 Scope of the first PR

**Phase A — full language support, no `.csproj`.**

`.csproj`/`.sln`/NuGet cross-repo linking is deferred (§14.1). Rationale: `.csproj`
filenames are project-specific (`MyApp.csproj`), and `BUILD_SYSTEM_NAMES`
(`src/pipeline/input.rs:85-91`) matches filenames exactly. Supporting it requires
either registering `csproj` as an extension or adding suffix-matching to
`discover_files` — an orthogonal change to the discovery layer that should not ride
along with the language work.

Consequence: fixtures carry no project file, so FQNs derive purely from the
`namespace` declared in source. This is the correct behaviour for Phase A and is
asserted as such.

---

## 4. Integration Map

Every touch point, verified against the working tree at v1.6.2.

| # | Location | Change |
|---|---|---|
| 1 | `Cargo.toml` (deps block, after line 70) | `tree-sitter-c-sharp = "0.23"` |
| 2 | `queries/csharp.scm` | **new** — template `queries/java.scm` |
| 3 | `src/pipeline/parser/languages/csharp/` | **new directory** — `mod.rs`, `capture.rs`, `fqn.rs`, `refs.rs`, `tests.rs` |
| 4 | `src/pipeline/parser/languages/mod.rs:1` | `pub mod csharp;` (alphabetical, between `cpp` and `css`) |
| 5 | `src/pipeline/parser/mod.rs:34-47` | `const DEFAULT_CSHARP_QUERY: &str = include_str!("../../../queries/csharp.scm");` |
| 6 | `src/pipeline/parser/mod.rs:289` (`match ext`) | new arm `"cs" => { … }` with `lang_name = "csharp"` |
| 7 | `src/pipeline/input.rs:12-16` | add `"cs"` to `CORE_EXTENSIONS` |
| 8 | `src/pipeline/input.rs:24-65` | add `"cs"` to `SUPPORTED_EXTENSIONS` (the union is hand-duplicated, **not** computed) |
| 9 | `src/models/entity.rs:16-129` | 16 new `CSharp*` variants in a commented block |
| 10 | `src/models/entity.rs:131-232` | `Display` impl — lowercase strings (`csharp_class`, …) |
| 11 | `src/db/graph/utils.rs:8-105` | `kind_to_label` — Neo4j node labels |
| 12 | `src/pipeline/parser/context.rs:87-247` | `compute_fqn_and_context` — FQN shape per kind |
| 13 | `src/pipeline/parser/context.rs:20-27` | `extract_class_contexts` — add `struct_declaration`, `record_declaration`, `enum_declaration` |
| 14 | `src/pipeline/parser/extractor/mod.rs:54-59` | file-scoped namespace pre-pass; rename `java_package` → `package_prefix` |
| 15 | `src/pipeline/parser/extractor/captures.rs:364` | new arm `starts_with("csharp.")` |
| 16 | `src/pipeline/parser/extractor/captures.rs:116-155`, `:251-330` | C# branches for body reference intents |
| 17 | `src/pipeline/parser/extractor/enrich.rs:73-87` | C# FQN prefix branch (parallel to the C++ one) |
| 18 | `src/pipeline/parser/extractor/enrich.rs:110-179` | C# inheritance + attributes + type refs; extend the `matches!` guard with `CSharp*` kinds |
| 19 | `src/pipeline/parser/extractor/post_passes.rs:70` | add `"csharp"` to the orphan-collection allowlist |
| 20 | `src/pipeline/parser/orphans.rs:43-47` | `"csharp" => EntityKind::CSharpNamespace` for the synthetic `<module>` |
| 21 | `src/pipeline/parser/orphans.rs:105-157` | C# branch in `collect_all_reference_intents_with_byte_pos` |
| 22 | `src/pipeline/parser/comments.rs:167-205` | C# attribute extraction (attributes live in `attribute_list`, **not** in `modifiers` as in Java/Kotlin) |
| 23 | `src/pipeline/parser/comments.rs:216-234` | C# child-entity node allowlist |
| 24 | `src/pipeline/parser/test_utils.rs` | `parse_csharp_snippet` |
| 25 | `src/pipeline/ingest/resolve/overrides.rs:34,45,54` | `.cs` extension + `CSharp*` kinds; rename `JVM_*` → `OVERRIDE_CAPABLE_*` |
| 26 | `src/cli_tools/explore_file.rs:170` | `KIND_BUCKETS` entries for C# |
| 27 | `src/mcp_tools/search_hybrid_context/mod.rs:72`, `src/mcp_tools/find_callers.rs:56,60`, `src/mcp_tools/explore_file.rs:54` | tool descriptions |
| 28 | `tests/testing_files/csharp/` | **new** — 7 fixtures |
| 29 | `tests/run_csharp_e2e.sh` | **new** — template `tests/run_varnish_e2e.sh` |
| 30 | `tests/run_all_e2e_fast.sh:42-60` | append `"run_csharp_e2e.sh"`; bump "19 suites" comments at `:40`, `:175`, `:178` |
| 31 | `README.md`, `AGENTS.md`, `CHANGELOG.md`, `docs/specs/multilanguage_roadmap.md` | documentation |

### 4.1 Deliberate non-changes

- **`src/pipeline/files.rs`** — `is_supported_file` (`files.rs:22`) reads
  `SUPPORTED_EXTENSIONS` directly, so adding `"cs"` to `input.rs` propagates
  automatically. No edit needed. *(This corrects an earlier assumption that the
  predicate was duplicated.)*
- **`src/pipeline/state.rs:33` (`CURRENT_STATE_VERSION = 4`)** — must **not** be
  bumped. C# changes no existing FQN shape; bumping would force every current user
  into a full re-index for no benefit.
- **`.github/workflows/ci.yml`** — CI invokes `./tests/run_all_e2e_fast.sh`
  wholesale and never names individual suites.
- **`tests/docker-compose.e2e.yml`** — nothing in it is language-specific.
- **`tests/benchmark_e2e.sh`** — the language `case` at `:354-374` passes the same
  `$TESTING_FILES` root for every label, so no new arm is needed. (But see §13.)
- **`src/db/graph/query_repo.rs::most_common_language`** — derives
  `primary_language` from the stored `language` field; already generic.
- **`src/models/cli_args.rs`, `src/config.rs`** — no per-language flags exist.

---

## 5. Phase 0 — BDD / End-to-End Tests (the "Red" phase)

Written **before** any change under `src/`. Must fail.

### 5.1 Fixtures — `tests/testing_files/csharp/`

Seven files, deliberately compact. `tests/benchmark_e2e.sh` indexes the entire
`tests/testing_files` tree and compares against the committed
`.perf_metrics/baseline.json`, so fixture size directly affects the CI performance
gate.

No `.csproj` (out of scope for Phase A) — FQNs derive from the `namespace` keyword.

#### `Domain/IRepository.cs`
```csharp
namespace MyApp.Domain;

/// <summary>
/// Generic persistence abstraction for aggregate roots.
/// </summary>
public interface IRepository<T>
{
    /// <summary>Finds an entity by its identifier.</summary>
    Task<T?> FindByIdAsync(int id);

    Task SaveAsync(T entity);
}

public interface IAdminRepository : IRepository<User>
{
    Task<int> PurgeAsync();
}

public interface IUserService
{
    Task<UserDto?> GetUserAsync(int id);
}
```
Covers: generic interface, interface-extends-interface, XML doc comments,
file-scoped namespace.

#### `Domain/Models.cs`
```csharp
namespace MyApp.Domain;

public record UserDto(int Id, string Name);

public record struct Coord(double Lat, double Lon);

public struct Point : IEquatable<Point>
{
    public int X { get; init; }
    public int Y { get; init; }

    public bool Equals(Point other) => X == other.X && Y == other.Y;
}

public enum UserStatus
{
    Active,
    Suspended,
    Deleted
}

public class User
{
    public int Id { get; set; }
    public string Name { get; set; } = string.Empty;
}

public class Container
{
    public class Nested
    {
        public int Value { get; set; }
    }
}
```
Covers: `record` class, `record struct`, `struct` implementing an interface, enum,
nested type, auto-properties, expression-bodied method.

#### `Services/BaseService.cs`
```csharp
namespace MyApp.Services;

public abstract class BaseService
{
    protected const int MaxRetries = 3;

    protected readonly string _serviceName;

    protected BaseService(string serviceName)
    {
        _serviceName = serviceName;
    }

    public virtual string Process(string input)
    {
        return input.Trim();
    }
}
```
Covers: abstract class, `const` field, `readonly` field, constructor, `virtual`
method.

#### `Services/UserService.cs`
```csharp
using System;
using System.Threading.Tasks;
using MyApp.Domain;
using MyApp.Infrastructure;

namespace MyApp.Services;

/// <summary>
/// Application service coordinating user retrieval.
/// </summary>
[Obsolete("Use UserServiceV2 instead")]
public class UserService : BaseService, IUserService
{
    private readonly UserRepository _repository;

    public string ServiceLabel { get; private set; }

    public UserService(UserRepository repository) : base("users")
    {
        _repository = repository;
        ServiceLabel = "user-service";
    }

    public async Task<UserDto?> GetUserAsync(int id)
    {
        var user = await _repository.FindByIdAsync(id);
        if (user is null)
        {
            return null;
        }
        return new UserDto(user.Id, user.Name);
    }

    public override string Process(string input)
    {
        return base.Process(input).ToUpperInvariant();
    }
}
```
Covers: file-scoped namespace, `extends` + `implements` in one `base_list`,
attribute, constructor with DI, property with restricted setter, `async Task<T>`,
`override`, method call on a field receiver, `new` expression, `using` directives.

#### `Infrastructure/UserRepository.cs`
```csharp
using System.Linq;
using MyApp.Domain;

namespace MyApp.Infrastructure;

public class UserRepository : IRepository<User>
{
    private readonly List<User> _users = new();

    public Task<User?> FindByIdAsync(int id)
    {
        var match = _users.FirstOrDefault(u => u.Id == id);
        return Task.FromResult(match);
    }

    public Task SaveAsync(User entity)
    {
        _users.Add(entity);
        return Task.CompletedTask;
    }
}
```
Covers: interface implementation with generic argument, LINQ, lambda, collection
initializer.

#### `Extensions/StringExtensions.cs`
```csharp
namespace MyApp.Extensions;

public static class StringExtensions
{
    public static string Slugify(this string value)
    {
        string Normalize(string raw) => raw.Trim().ToLowerInvariant();

        return Normalize(value).Replace(' ', '-');
    }
}
```
Covers: `static class`, extension method (`this string`), `local_function_statement`.

#### `Legacy/OldStyle.cs`
```csharp
namespace MyApp.Legacy
{
    namespace Deep
    {
        public delegate void Notifier(string message);

        public class OldStyle
        {
            public event Notifier? OnNotify;

            private readonly string[] _items = new string[10];

            public string this[int index]
            {
                get => _items[index];
                set => _items[index] = value;
            }

            public static OldStyle operator +(OldStyle a, OldStyle b) => a;

            public class Inner
            {
                public int Depth { get; set; }
            }
        }
    }
}
```
Covers: block-form namespace, **nested** namespace, delegate, event, indexer,
operator overload, type nested inside a type.

### 5.2 `tests/run_csharp_e2e.sh`

**Template: `tests/run_varnish_e2e.sh`** — it is the only per-language suite that
honours `KNOT_SKIP_BUILD`, which is how CI runs (`.github/workflows/ci.yml` sets
`KNOT_SKIP_BUILD: "1"`). `run_kotlin_e2e.sh` and `run_rust_e2e.sh` ignore the flag
and only work in CI by accident.

Mandatory contracts:

- `set -e`, `set -u`; `trap cleanup EXIT INT TERM`
- **Honour `KNOT_E2E_EXTERNAL_DB`** — if unset, start/stop compose; if set, do
  nothing. A suite that tears down the shared DB breaks every subsequent suite.
- `--clean` passed **only** in standalone mode:
  `[[ -z "${KNOT_E2E_EXTERNAL_DB:-}" ]] && INDEXER_FLAGS+=("--clean")`
- `KNOT_SKIP_BUILD=1` → run `./target/release/knot-indexer` directly; otherwise
  `cargo run --release --bin knot-indexer`. Same branch for `knot-mcp` and `knot`.
- Per-suite namespaces:
  ```bash
  TEST_FILES_DIR="$SCRIPT_DIR/testing_files/csharp"
  E2E_DATA_DIR="$SCRIPT_DIR/.e2e_csharp_data"
  QDRANT_COLLECTION="knot_csharp_e2e_test"
  REPO_NAME="csharp_e2e_test_repo"
  ```
  Ports stay at the shared `17687` (Neo4j bolt) / `16334` (Qdrant gRPC).
- **Every** Cypher assertion and **every** search scoped by `repo_name` — the
  orchestrator shares one database across all suites and `--clean` is repo-scoped
  (`run_all_e2e_fast.sh:175-177`).
- Local `assert_cypher_count` / `assert_cypher_exists` helpers plus a `FAILURES`
  counter (varnish style, `run_varnish_e2e.sh:145-198`); final gate exits non-zero
  if `FAILURES > 0`.
- `source "$SCRIPT_DIR/lib/assert_neo4j_relationships.sh"` for FQN-keyed edge
  assertions (`assert_edge_exists`, `assert_no_edge`, `assert_edge_count`). The
  library requires `NEO4J_URI`, `NEO4J_USER`, `NEO4J_PASSWORD` in the environment
  and scopes queries by `KNOT_REPO_NAME` when set.
- Dual **MCP + CLI** validation for every extraction and query assertion.
- Indexer progress-log contract check (copied from `run_rust_e2e.sh:155-173`):
  assert a `[Progress] … n% — files x/y` line and a `100.0%` line unless the run
  reports "No files changed" / "No supported source files found".

MCP invocation shape (stdin JSON-RPC, response is the last stdout line):
```bash
MCP_REQUEST="{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/call\",\"params\":{\"name\":\"explore_file\",\"arguments\":{\"file_path\":\"$CS_FILE\"}}}"
MCP_RESPONSE=$(echo "$MCP_REQUEST" | ./target/release/knot-mcp 2>/dev/null | tail -n 1)
CLI_RESPONSE=$(./target/release/knot explore "$CS_FILE" -r "$REPO_NAME" 2>/dev/null)
```

### 5.3 E2E scenarios — 31 assertions

#### A · Entity extraction (`explore_file` via MCP **and** `knot explore` via CLI)

| # | Assertion |
|---|---|
| A1 | `csharp_class` `UserService` found in `Services/UserService.cs` |
| A2 | `csharp_interface` `IRepository` found in `Domain/IRepository.cs` |
| A3 | `csharp_method` `GetUserAsync` found |
| A4 | `csharp_property` `ServiceLabel` found, and is **not** reported as a field |
| A5 | `csharp_record` `UserDto` found |
| A6 | `csharp_struct` `Point` found |
| A7 | `csharp_enum` `UserStatus` found |
| A8 | `csharp_delegate` `Notifier` and `csharp_event` `OnNotify` found in `Legacy/OldStyle.cs` |
| A9 | `csharp_indexer` and `csharp_operator` found in `OldStyle` |
| A10 | `csharp_class` `StringExtensions` and `csharp_method` `Slugify` found; `csharp_local_function` `Normalize` found |
| A11 | `csharp_constructor` for `UserService` found |
| A12 | Exact per-kind counts via `assert_cypher_count` — guards against double extraction |

Example for A12:
```bash
assert_cypher_count "A12a. csharp_interface count" \
  "MATCH (e:Entity) WHERE e.kind = 'csharp_interface' AND e.repo_name = '$REPO_NAME' RETURN count(e) AS cnt" \
  "3"
```

#### B · FQN and namespaces (Cypher)

| # | Assertion |
|---|---|
| B13 | `fqn = 'MyApp.Services.UserService'` exists (file-scoped namespace) |
| B14 | `fqn = 'MyApp.Services.UserService.GetUserAsync'` exists |
| B15 | `fqn = 'MyApp.Legacy.Deep.OldStyle.Inner'` exists (nested block namespace + nested type) |
| B16 | `fqn = 'MyApp.Domain.Container.Nested'` exists (type nested in type) |

```bash
assert_cypher_exists "B13. file-scoped namespace FQN" \
  "MATCH (e:Entity) WHERE e.fqn = 'MyApp.Services.UserService' AND e.repo_name = '$REPO_NAME' RETURN count(e)"
```

#### C · Relationships (`assert_edge_exists` / `assert_no_edge`, keyed on FQN)

| # | Assertion |
|---|---|
| C17 | `UserService -[:EXTENDS]-> BaseService` |
| C18 | `UserService -[:IMPLEMENTS]-> IUserService` |
| C19 | `UserRepository -[:IMPLEMENTS]-> IRepository` (generic argument stripped) |
| C20 | `IAdminRepository -[:EXTENDS]-> IRepository` (interface extends interface) |
| C21 | `Point -[:IMPLEMENTS]-> IEquatable` **and** `assert_no_edge Point IEquatable EXTENDS` — a struct never extends |
| C22 | `UserService.GetUserAsync -[:CALLS]-> UserRepository.FindByIdAsync` |
| C23 | `new UserDto(...)` produces a `CALLS` edge redirected to the constructor (`redirect_class_call_to_constructor`) |
| C24 | `UserService.GetUserAsync -[:REFERENCES]-> UserDto` (return/parameter type reference) |
| C25 | `UserService -[:CONTAINS]-> UserService.GetUserAsync` (auto-link via `enclosing_class_fqn`, `db/graph/upsert.rs:30-39`) |

#### D · OVERRIDES

| # | Assertion |
|---|---|
| D26 | `UserRepository.FindByIdAsync -[:OVERRIDES]-> IRepository.FindByIdAsync` (interface implementation) |
| D27 | `UserService.Process -[:OVERRIDES]-> BaseService.Process` (`virtual` / `override`) |

#### E · Comments and attributes

| # | Assertion |
|---|---|
| E28 | The XML doc summary of `IRepository` appears in `explore_file` output (MCP + CLI) |
| E29 | `[Obsolete]` present in the `decorators` property of `UserService` |

```bash
assert_cypher_exists "E29. Obsolete attribute captured" \
  "MATCH (e:Entity) WHERE e.fqn = 'MyApp.Services.UserService' AND e.repo_name = '$REPO_NAME' AND any(d IN e.decorators WHERE d CONTAINS 'Obsolete') RETURN count(e)"
```

#### F · Semantic search (MCP + CLI)

| # | Assertion |
|---|---|
| F30 | `search_hybrid_context "user repository data access"` scoped to `$REPO_NAME` returns `UserRepository` |

Wrap in the `retry_match` helper (`run_cpp_e2e.sh:47-60`) to absorb Qdrant's
eventual consistency.

#### G · find_callers (MCP + CLI)

| # | Assertion |
|---|---|
| G31 | `find_callers FindByIdAsync` lists `UserService.GetUserAsync` under *Calls* and `UserRepository.FindByIdAsync` under *Overridden by* |

### 5.4 Registration

- Append `"run_csharp_e2e.sh"` to the `SUITES` array in
  `tests/run_all_e2e_fast.sh:42-60`.
- Update the "19 suites" comments at `:40`, `:175`, `:178` → 20.
- `chmod +x tests/run_csharp_e2e.sh`.
- No CI change: `ci.yml` runs the orchestrator.

### 5.5 Red gate

```bash
./tests/run_csharp_e2e.sh
```
Must fail with an empty index — `.cs` files are not even discovered
(`SUPPORTED_EXTENSIONS` lacks `"cs"`). **This is the exit criterion for Phase 0.**

---

## 6. Phase 1 — Discovery and Dispatch

Goal: `.cs` files reach the parser. E2E stays red, but now fails on *missing
entities* rather than an invisible file.

### 6.1 Unit tests (Red)

- `src/pipeline/input.rs` tests — `"cs"` present in `SUPPORTED_EXTENSIONS`; a
  `.cs` fixture is discovered by `discover_files`.
- `src/pipeline/files.rs` tests — `is_supported_file(Path::new("Foo.cs"), false)`
  is `true`.
- `src/pipeline/parser/mod.rs` — `assert_extensions_detected(&["cs"])` following
  the pattern at `mod.rs:870-893`.
- `src/pipeline/parser/test_utils.rs` — `parse_csharp_snippet("class Foo {}")`
  returns a tree with `!has_error()`.

### 6.2 Implementation (Green)

```toml
# Cargo.toml — after line 70
tree-sitter-c-sharp = "0.23"
```

```rust
// src/pipeline/parser/mod.rs — with the other DEFAULT_*_QUERY consts (:34-47)
const DEFAULT_CSHARP_QUERY: &str = include_str!("../../../queries/csharp.scm");

// src/pipeline/parser/mod.rs — inside `match ext` (:289)
"cs" => {
    let query_src = load_query_source("csharp.scm", DEFAULT_CSHARP_QUERY, parse_cfg);
    extractor::extract_entities(
        &source,
        tree_sitter_c_sharp::LANGUAGE.into(),
        &query_src,
        "csharp",
        &file_path,
        &parse_cfg.repo_name,
    )?
}
```

```rust
// src/pipeline/parser/test_utils.rs
#[cfg(test)]
pub(crate) fn parse_csharp_snippet(code: &str) -> Result<Tree, String> {
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_c_sharp::LANGUAGE.into())
        .map_err(|e| format!("Failed to set C# language: {e}"))?;
    parser
        .parse(code, None)
        .ok_or_else(|| "Failed to parse C# code snippet".to_string())
}
```

Plus `"cs"` in **both** `CORE_EXTENSIONS` (`input.rs:12`) and
`SUPPORTED_EXTENSIONS` (`input.rs:24`), `pub mod csharp;` in
`languages/mod.rs`, and a skeleton `queries/csharp.scm` containing only
`csharp.class.name`.

---

## 7. Phase 2 — Entity Extraction

Unit tests first, in `src/pipeline/parser/languages/csharp/tests.rs`, convention
`test_<subject>_<scenario>`, inline snippets (Kotlin model,
`kotlin.rs:708-1092`).

### 7.1 EntityKinds

Sixteen variants in a new banner-commented block in `src/models/entity.rs:16-129`:

```rust
    // C# entities
    CSharpClass,         // class declarations
    CSharpInterface,     // interface declarations
    CSharpStruct,        // struct declarations
    CSharpRecord,        // record class / record struct declarations
    CSharpEnum,          // enum declarations
    CSharpMethod,        // methods inside types
    CSharpConstructor,   // constructor declarations
    CSharpProperty,      // property declarations (get/set/init)
    CSharpField,         // field declarations
    CSharpConstant,      // const fields
    CSharpDelegate,      // delegate declarations
    CSharpEvent,         // event / event field declarations
    CSharpIndexer,       // this[...] indexer declarations
    CSharpOperator,      // operator overloads
    CSharpNamespace,     // namespace declarations (block + file-scoped)
    CSharpLocalFunction, // local functions inside method bodies
```

Each variant requires **four** exhaustive matches:

1. **`Display`** — `src/models/entity.rs:131-232`. Emits the lowercase snake_case
   string that becomes the `kind` property in Neo4j and Qdrant, and the value the
   E2E Cypher assertions match on: `csharp_class`, `csharp_interface`,
   `csharp_struct`, `csharp_record`, `csharp_enum`, `csharp_method`,
   `csharp_constructor`, `csharp_property`, `csharp_field`, `csharp_constant`,
   `csharp_delegate`, `csharp_event`, `csharp_indexer`, `csharp_operator`,
   `csharp_namespace`, `csharp_local_function`.
2. **`kind_to_label`** — `src/db/graph/utils.rs:8-105`. PascalCase Neo4j labels:
   `CSharpClass`, ….
3. **`compute_fqn_and_context`** — `src/pipeline/parser/context.rs:87-247`.
   Type-like kinds and member kinds both use the `.`-joined enclosing-class shape
   already used by `Method`/`KotlinMethod`.
4. **`KIND_BUCKETS`** — `src/cli_tools/explore_file.rs:170-352`. Unmatched kinds
   silently fall into `OTHERS_HEADER` (`:357`), so omitting this degrades output
   without failing a test.

### 7.2 `queries/csharp.scm`

Prefixed capture names (Rust/Groovy pattern, routed at `captures.rs:364`):

```
csharp.namespace.name       csharp.class.name        csharp.interface.name
csharp.struct.name          csharp.record.name       csharp.enum.name
csharp.method.name          csharp.constructor.name  csharp.property.name
csharp.field.name           csharp.constant.name     csharp.delegate.name
csharp.event.name           csharp.indexer           csharp.operator
csharp.local_function.name  csharp.signature
```

Header comment mirrors `queries/java.scm:1-15`, including the note that the file
can be overridden via `--custom-queries-path` / `KNOT_CUSTOM_QUERIES_PATH`.

### 7.3 `languages/csharp/capture.rs`

```rust
pub(crate) fn handle_csharp_capture(
    cap_name: &str,
    text: &str,
    node: Node<'_>,
) -> Option<(String, EntityKind, usize)>
```

Custom cases (the grammar gaps from §2.3):

- **`csharp.record.name`** — inspect the anonymous `struct` token among the
  `record_declaration` children to distinguish `record class` from `record struct`.
- **`csharp.field.name`** — the capture targets
  `variable_declarator > name:(identifier)`; the entity node is the ancestor
  `field_declaration`, resolved with `find_parent_by_kind`.
- **`const` detection** — walk the `field_declaration` modifiers; a `const` token
  yields `CSharpConstant` instead of `CSharpField`.
- **`csharp.indexer` / `csharp.operator`** — these declarations have no `name`
  field, so the handler synthesises names (`this[]`, `operator +`) from the
  declaration node.

### 7.4 Wiring

- `captures.rs:364` — new arm:
  ```rust
  name_or_intent if name_or_intent.starts_with("csharp.") => { … }
  ```
  resolving `entity_node` via `find_parent_by_kind` against
  `class_declaration` / `interface_declaration` / `struct_declaration` /
  `record_declaration` / `enum_declaration` / `method_declaration` /
  `constructor_declaration` / `property_declaration` / `field_declaration` /
  `delegate_declaration` / `event_declaration` / `indexer_declaration` /
  `operator_declaration` / `local_function_statement` / `namespace_declaration`.
- `context.rs:20-27` — extend the `extract_class_contexts` node allowlist with
  `struct_declaration`, `record_declaration`, `enum_declaration`.
  `class_declaration` and `interface_declaration` already match.

**Green gate:** assertions **A1–A12**.

---

## 8. Phase 3 — Namespaces and FQN

### 8.1 Unit tests (Red)

`test_fqn_file_scoped_namespace`, `test_fqn_block_namespace`,
`test_fqn_nested_block_namespace`, `test_fqn_no_namespace`,
`test_fqn_nested_type_in_type`, `test_fqn_nested_type_in_nested_namespace`.

### 8.2 `languages/csharp/fqn.rs`

```rust
/// File-scoped namespace prefix (C# 10+). Scans direct children of
/// `compilation_unit` for `file_scoped_namespace_declaration`, which has no
/// `body` field — subsequent types are siblings, not descendants.
pub(crate) fn extract_file_scoped_namespace(root: Node<'_>, source: &[u8]) -> Option<String>;

/// Dotted chain of enclosing block namespaces and containing types, built by
/// walking ancestors from the entity node. Returns `None` at file scope.
pub(crate) fn build_csharp_fqn_prefix(node: Node<'_>, source: &[u8]) -> Option<String>;
```

### 8.3 Wiring

- `extractor/mod.rs:54-59` — rename `java_package` → `package_prefix` and extend:
  ```rust
  let package_prefix = match lang_name {
      "java" => java::extract_package_name(tree.root_node(), source_bytes),
      "csharp" => csharp::extract_file_scoped_namespace(tree.root_node(), source_bytes),
      _ => None,
  };
  ```
  The rename propagates to the `enrich_and_create_entity` signature
  (`enrich.rs:30`) and its use at `enrich.rs:68-71`.
- `enrich.rs` — new branch parallel to the C++ one (`:73-87`), gated on the
  `CSharp*` kinds, applying `build_csharp_fqn_prefix` with `.` as separator and
  setting `enclosing_class` to the resulting prefix.

Ordering: the file-scoped prefix applies first (it is the outermost scope), then
the ancestor-walk prefix, then the entity name.

**Green gate:** assertions **B13–B16**.

---

## 9. Phase 4 — References

### 9.1 Unit tests (Red)

One test per function, plus one per row of the AST-to-intent table below.

### 9.2 `languages/csharp/refs.rs`

```rust
pub(crate) fn extract_reference_intents_csharp(
    node: Node<'_>, source: &[u8], out: &mut Vec<ReferenceIntent>);

pub(crate) fn extract_call_intents_csharp(
    node: Node<'_>, source: &[u8]) -> Vec<CallIntent>;

pub(crate) fn extract_class_inheritance_csharp(
    node: Node<'_>, source: &[u8], out: &mut Vec<ReferenceIntent>);

pub(crate) fn extract_attribute_references(
    node: Node<'_>, source: &[u8], out: &mut Vec<ReferenceIntent>);

pub(crate) fn extract_type_references_csharp(
    node: Node<'_>, source: &[u8], out: &mut Vec<ReferenceIntent>);

pub(crate) fn collect_all_reference_intents_csharp(
    node: Node<'_>, source: &[u8], out: &mut Vec<(ReferenceIntent, usize)>);
```

### 9.3 AST-to-intent mapping

| AST node | Emitted `ReferenceIntent` |
|---|---|
| `invocation_expression` with `function: (identifier)` | `Call { method, receiver: None, arg_count }` |
| `invocation_expression` with `function: (member_access_expression)` | `Call { method: name, receiver: expression }` |
| `object_creation_expression type:` | `Call` (redirected to the constructor at resolution) **and** `TypeReference` |
| `base_list` on `class_declaration` / `record_declaration` | `Extends` / `Implements` per the §3.3 heuristic |
| `base_list` on `interface_declaration` | `Extends` for every entry |
| `base_list` on `struct_declaration` / record-struct | `Implements` for every entry |
| `attribute name:` | `Call` (mirrors `java::extract_annotation_references`) |
| `parameter type:`, `method_declaration returns:`, `property_declaration type:`, `variable_declaration type:` | `TypeReference` |
| `using_directive` | `TypeReference` — alias form from the `name` field; plain form from the `qualified_name`/`identifier` child (§2.3, Gap 4) |

All type names are stripped of type arguments before emission
(`IRepository<User>` → `IRepository`).

### 9.4 Wiring

- `captures.rs:116-155` — `csharp` branch for method-body reference intents and
  signature type references.
- `captures.rs:251-330` — `csharp` branch for `function.name` / `constant.name`
  equivalents.
- `enrich.rs:110-179` — `csharp` branch performing inheritance + attribute
  references + type references; the enclosing `matches!` guard at `:110-118` must
  be extended with `CSharpClass | CSharpInterface | CSharpStruct | CSharpRecord |
  CSharpEnum`.
- `post_passes.rs:70` — add `"csharp"` to the orphan-collection allowlist.
- `orphans.rs:43-47` — `"csharp" => EntityKind::CSharpNamespace` for the synthetic
  `<module>` entity.
- `orphans.rs:105-157` — `csharp` branch in
  `collect_all_reference_intents_with_byte_pos`, emitting each intent with its real
  byte position (the Kotlin approach) so that calls inside method bodies fall
  within covered ranges and are **not** orphaned.
- `comments.rs:167-205` — `csharp` branch in `extract_decorators`. **C# attributes
  live in `attribute_list` children of the declaration, not inside a `modifiers`
  node** as in Java and Kotlin, so this cannot reuse either existing branch.
- `comments.rs:216-234` — `csharp` entry in the `extract_child_entity_nodes`
  allowlist: `method_declaration`, `constructor_declaration`, `property_declaration`,
  `class_declaration`, `interface_declaration`, `struct_declaration`,
  `record_declaration`, `enum_declaration`.

XML doc comments need no code: `strip_comment_markers` (`comments.rs:289`) already
strips `///`.

**Green gate:** assertions **C17–C25**, **E28–E29**.

---

## 10. Phase 5 — OVERRIDES

`src/pipeline/ingest/resolve/overrides.rs` is currently scoped to JVM languages by
name and by comment, but the mechanism is language-agnostic. C# fits the model
exactly: `.`-separated FQNs, explicit `virtual`/`override`, interface
implementation, and constructors named after their type
(`is_constructor`, `overrides.rs:99`).

### 10.1 Unit tests (Red)

`test_override_csharp_class_virtual`, `test_override_csharp_interface_impl`,
`test_override_csharp_constructor_excluded`,
`test_override_csharp_static_method_excluded`.

### 10.2 Implementation

```rust
// overrides.rs:34
const OVERRIDE_CAPABLE_EXTENSIONS: &[&str] =
    &[".java", ".kt", ".kts", ".groovy", ".gvy", ".gradle", ".cs"];
```

- `overrides.rs:45` — add `CSharpMethod` (and `CSharpProperty`, since C#
  properties can be `virtual`/`override`) to the method-like allowlist.
- `overrides.rs:54` — add `CSharpClass`, `CSharpInterface`, `CSharpStruct`,
  `CSharpRecord`, `CSharpEnum` to the type-like allowlist.

### 10.3 Required rename

The module vocabulary must be renamed, or the module starts lying about its scope:

| Old | New |
|---|---|
| `JVM_EXTENSIONS` | `OVERRIDE_CAPABLE_EXTENSIONS` |
| `is_jvm_file` | `is_override_capable_file` |
| `is_jvm_method_like` | `is_override_capable_method` |
| `is_jvm_type_like` | `is_override_capable_type` |

Also update the module rustdoc (`overrides.rs:1-21`), the existing unit test name
`is_jvm_file_guard` (`overrides.rs:712`), and
`docs/specs/method_override_relationships.md`.

**Green gate:** assertions **D26–D27**, **G31**.

---

## 11. Phase 6 — Output, Tool Descriptions and Documentation

### 11.1 Code

- `src/cli_tools/explore_file.rs:170` — `KIND_BUCKETS` entries: *Classes (C#)*,
  *Interfaces (C#)*, *Structs (C#)*, *Records (C#)*, *Enums (C#)*, *Methods (C#)*,
  *Properties & Fields (C#)*, *Delegates & Events (C#)*, *Namespaces (C#)*.
- `src/mcp_tools/search_hybrid_context/mod.rs:72`,
  `src/mcp_tools/find_callers.rs:56` and `:60`,
  `src/mcp_tools/explore_file.rs:54` — add C# to the language lists in the tool
  descriptions (these strings are what an LLM sees when choosing a tool).

### 11.2 Documentation

- `README.md`: line 13 (one-line language list), the *Multi-Language Support*
  bullet list at `:54-70`, the E2E invocation list at `:430-443`, the
  `find_callers` examples at `:474-480`, the *Override Discovery (JVM)* note at
  `:488` (now covers C#), the custom-queries note at `:601`, and flip
  `- [ ] C# support` → `- [x]` at `:719`.
- `AGENTS.md`: *Language Parsers* table, E2E suite list, `EntityKind` section.
- `docs/specs/multilanguage_roadmap.md`: **Phase 15: C# Support** section plus a
  row in the *Implementation Priority & Timeline* table and a `v1.7.0` changelog
  entry.
- `CHANGELOG.md`: new entry.
- `Cargo.toml:5` — package `description`.

**Green gate:** assertion **F30**; full E2E suite green.

---

## 12. Phase 7 — Quality Gate

```bash
cargo fmt -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test
./tests/run_csharp_e2e.sh
./tests/run_all_e2e_fast.sh                  # zero regressions across the other 19 suites
./tests/benchmark_e2e.sh --focus java_e2e    # C# fixtures enter the perf baseline
```

Then invoke the `validator` subagent.

Constraints: no `#[allow(...)]` anywhere. `#[expect(...)]` only as a last resort,
always with a `reason`, and flagged explicitly to the human reviewer. No `unsafe`
(`unsafe_code = "deny"` at crate level).

---

## 13. Risks and Mitigations

| Risk | Mitigation |
|---|---|
| The `I`-prefix heuristic mislabels a base *class* named `IFoo` as `IMPLEMENTS` | Documented limitation; struct and interface declarations remain deterministic; follow-up issue for resolution-time correction (§14.3) |
| `benchmark_e2e.sh` indexes the whole `tests/testing_files` tree and compares against the committed `.perf_metrics/baseline.json`, so new fixtures shift every perf label | Keep fixtures compact (7 small files); review and, if needed, refresh `.perf_metrics/baseline.json` in the PR |
| `partial class` (one type split across N files) produces duplicate entities with distinct UUIDs | Out of scope for Phase A. Add a **negative fixture** documenting current behaviour so the regression is visible when the feature is addressed (§14.2) |
| `input.rs` duplicates the extension union by hand — adding `"cs"` to only one list silently half-works | Add to **both** `CORE_EXTENSIONS` and `SUPPORTED_EXTENSIONS`; the Phase 1 discovery unit test covers it |
| `run_kotlin_e2e.sh` / `run_rust_e2e.sh` ignore `KNOT_SKIP_BUILD` and only pass in CI by accident | Do not copy them — base the C# suite on `run_varnish_e2e.sh` |
| The shared E2E database means an unscoped Cypher assertion can match entities from the other 19 languages | Every query filters on `repo_name = '$REPO_NAME'`; enforced by review |
| `top-level statements` (C# 9 `Program.cs` with no class) produce `global_statement` nodes with no enclosing entity | Handled by the existing orphan pass once `"csharp"` is in the `post_passes.rs:70` allowlist; add a unit test |

---

## 14. Out of Scope (follow-up issues)

### 14.1 `.csproj` / `.sln` / NuGet cross-repo linking

> **Superseded by** `docs/specs/subgraph_root_resolution_and_msbuild_support_plan.md`
> (Parts B + A; shipped as v1.7.2). The "suffix-based discovery" blocker
> premise at line 1119 is factually wrong — `Path::extension()` on
> `CodeMap.Storage.Engine.csproj` yields `"csproj"`, so the ordinary
> extension branch in `discover_files` handles discovery. `.csproj` is
> now in `CORE_EXTENSIONS` / `SUPPORTED_EXTENSIONS`, `Directory.Packages.props`
> is in `BUILD_SYSTEM_NAMES`, the MSBuild parser lives at
> `src/pipeline/parser/languages/msbuild.rs`, and the cross-repo wiring
> edits to `cross_repo.rs` are all landed. Remaining `.sln` / `.slnx` work
> (solution-level identity) is tracked as part of the v1.7.2 out-of-scope
> list — it requires its own selection-rule decision.

Original text (retained for archaeology):

Would emit `ProjectIdentity` with FQN `nuget:<AssemblyName>` (from
`<AssemblyName>` / `<RootNamespace>`), `BuildDependency` per
`<PackageReference Include="X" Version="Y"/>`, and cross-repo edges from
`<ProjectReference Include="..\Other\Other.csproj"/>`.

Requires:
- Suffix-based discovery in `src/pipeline/input.rs` — `BUILD_SYSTEM_NAMES`
  (`:85-91`) matches filenames exactly, and `.csproj` names are project-specific.
- `parse_build_system_from_fqn` (`resolve/cross_repo.rs:61-73`) — add `"nuget"`.
- `parse_artifact_identity` (`cross_repo.rs:75-90`) — add a `"nuget"` arm.
- `match_dependency_to_repository` (`cross_repo.rs:115-166`) — add a NuGet branch,
  **and** add `"nuget"` to the exclusion list at `:136-137`; the unguarded Cargo
  fallback at `:134-143` tries any dependency name without a `.` as a crate name,
  which would misroute bare NuGet package ids.

### 14.2 `partial class` / `partial method`

Merging partial declarations into a single logical entity requires cross-file
entity unification, which knot has no mechanism for today.

### 14.3 Resolution-time EXTENDS/IMPLEMENTS correction

Replace the §3.3 naming heuristic by looking up the actual `EntityKind` of the base
entry during relationship resolution and re-typing the edge accordingly. Requires a
two-pass resolution or a post-resolution fix-up pass.

### 14.4 Generic constraints

`where T : IComparable<T>, new()` → `GenericBound` relationships, mirroring the
Rust trait-bound handling.

---

## 15. Estimated Size

| Artefact | Size |
|---|---|
| `src/pipeline/parser/languages/csharp/mod.rs` | ~120 lines |
| `src/pipeline/parser/languages/csharp/capture.rs` | ~350 lines |
| `src/pipeline/parser/languages/csharp/fqn.rs` | ~250 lines |
| `src/pipeline/parser/languages/csharp/refs.rs` | ~550 lines |
| `src/pipeline/parser/languages/csharp/tests.rs` | ~900 lines (~45 unit tests) |
| `queries/csharp.scm` | ~120 lines |
| Wiring edits across ~20 existing files | ~350 lines |
| `tests/run_csharp_e2e.sh` | ~600 lines (31 assertions) |
| `tests/testing_files/csharp/` | 7 files, ~200 lines total |
| Documentation | ~150 lines |
| **Total** | **~3,600 lines** |

Directory layout for the parser follows the precedent of
`src/pipeline/parser/languages/rust/` and `.../varnish/` rather than a single flat
module, given the size.

---

## 16. Phase Summary

| Phase | Deliverable | Green gate |
|---|---|---|
| 0 | Fixtures + E2E suite (Red) | Suite fails on empty index |
| 1 | Discovery + dispatch | `.cs` reaches the parser; extension unit tests pass |
| 2 | Entity extraction | A1–A12 |
| 3 | Namespaces + FQN | B13–B16 |
| 4 | References | C17–C25, E28–E29 |
| 5 | OVERRIDES | D26–D27, G31 |
| 6 | Output + docs | F30; full suite green |
| 7 | Quality gate | fmt + clippy + unit + all E2E + perf |
