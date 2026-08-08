# Varnish Support Specification

Implementation plan for indexing the Varnish Cache language family (`.vcl`, `.vtc`, `.vcc`) in `knot`.

**Status:** Planned
**Target version:** v1.6.0
**Approach:** Hand-written parsers (no tree-sitter), BDD/TDD
**Delivery:** Single PR covering all three file types

---

## 1. Scope

### 1.1 In scope

| Extension | Content | Role |
|---|---|---|
| `.vcl` | Varnish Configuration Language | Production configuration |
| `.vtc` | Varnish Test Case | Test code (embeds VCL) |
| `.vcc` | VMOD interface definition | Module API declaration |

All three go into `CORE_EXTENSIONS` — they are production source code, not configuration, and must be indexed unconditionally (no `--include-config-files` gate).

### 1.2 Non-goals

- **Fastly VCL.** Explicitly out of scope. See §1.3.
- **varnishd boot artifacts** (`varnish.service`, `/etc/default/varnish`, `-I` CLI scripts). Their only value is resolving `vcl_path` for bare `include` statements; documented as a known limitation instead (§10.1).
- **Rust VMODs** (crate `varnish` 0.7.1). They have no `.vcc` file — the interface is derived from a `#[varnish::vmod]` proc macro over a Rust module. Their `.rs` files are already indexed by the existing Rust parser; no VMOD-specific handling.
- **Generated VMOD artifacts** (`vcc_if.c`, `vcc_if.h`, `vmod_*.rst`). Build outputs, not sources.

### 1.3 Fastly VCL — why it is excluded

There are two mutually incompatible languages both called "VCL":

| | Varnish Cache VCL | Fastly VCL |
|---|---|---|
| Version marker | `vcl 4.0;` / `vcl 4.1;` | `vcl 4.0;` (different semantics) |
| Directors | VMOD objects via `new` in `vcl_init` | top-level `director` keyword |
| Local variables | none | `declare local var.x STRING;` |
| Lookup tables | none | `table name { "k": "v", }` |
| Error signalling | `return(synth(750))` | `error 750 "msg";` |
| Backend-side subs | `vcl_backend_fetch/response/error` | `vcl_fetch` (VCL-2-era naming) |
| Logging sub | none | `vcl_log` |
| ESI trigger | `set beresp.do_esi = true;` | bare `esi;` statement |

Parsing Fastly VCL with a Varnish Cache parser produces silently wrong entities, not errors. Since Fastly is out of scope, the parser must **not** attempt to parse it.

**Required behaviour:** a lightweight dialect guard runs before parsing. If the source contains any Fastly-exclusive marker, the parser returns an empty `Vec` and logs at `debug` level. This is a guard, not a dialect abstraction — no `Dialect` enum is threaded through the parser, no Fastly entity kinds are reserved.

Fastly markers to detect (any one is sufficient):
- `declare local var.`
- `table <ident> {`
- `director <ident> <ident> {`
- `error <integer>` in statement position
- `sub vcl_fetch` / `sub vcl_log`

Rationale for a guard rather than partial extraction: a `.vcl` file that yields a handful of half-correct backends is worse than one that yields nothing, because search results imply completeness. Silence is honest.

---

## 2. Language background

### 2.1 VCL top-level constructs

VCL has a **flat global namespace** — no modules, no nesting, no shadowing, no generics, no inheritance. This makes FQN computation trivial compared to Rust or Java.

#### Version marker

```vcl
vcl 4.1;
```

Mandatory as the first statement of any file passed to `varnishd -f`. Acts as a hard ceiling: a lower-version file may not `include` a higher-version one. Included files need not carry their own marker. Only `4.0` and `4.1` are supported by Varnish.

The version gates variable legality — `local.socket`, `sess.xid`, `resp.do_esi` are 4.1-only; `req.esi`, `beresp.storage_hint`, `beresp.backend.ip` are 4.0-only. Indexed as a file-level attribute.

#### Import

```vcl
import std;
import directors;
import cookie from "/usr/lib/varnish/vmods/libvmod_cookie.so";
import std as standard;
```

The `as` form rebinds the namespace prefix used at call sites, so reference resolution must carry a per-file alias map.

#### Include

```vcl
include "foo.vcl";                              // searched in vcl_path
include "./relative/to/including/file.vcl";     // relative to includer
include "/absolute/path.vcl";                   // absolute
include +glob "/etc/varnish/example.org/*.vcl"; // glob expansion
```

Resolution rules:
- leading `/` → absolute
- leading `./` → relative to the **including** file
- otherwise → searched across `vcl_path` (a varnishd parameter, invisible from source)
- `+glob` works only with absolute or `./`-relative paths, never with `vcl_path`

#### Backend

```vcl
backend default {
    .host = "127.0.0.1";
    .port = "8080";
    .host_header = "example.com";
    .connect_timeout        = 1s;
    .first_byte_timeout     = 60s;
    .between_bytes_timeout  = 60s;
    .max_connections        = 100;
    .probe                  = myprobe;   // reference to a named probe
}

backend uds { .path = "/var/run/http.sock"; }        // VCL 4.1 UDS

backend behind_proxy {                                // VCL 4.1 via/PROXY
    .host = "192.0.2.10";
    .via  = tunnel;                                   // backend -> backend
    .authority = "example.com";
}

backend default none;                                 // explicit "no backend"
```

Inline probe form:

```vcl
backend b {
    .host = "127.0.0.1";
    .probe = {
        .url = "/healthz";
        .timeout = 1s;
        .interval = 5s;
        .window = 5;
        .threshold = 3;
    }
}
```

`.probe =` has two distinct right-hand sides: a **bare identifier** (cross-entity reference → edge) or a **brace block** (anonymous nested entity → no edge).

#### Probe

```vcl
probe myprobe {
    .url = "/healthz";
    .expected_response = 200;
    .timeout   = 2s;
    .interval  = 5s;
    .window    = 8;
    .threshold = 3;
    .initial   = 3;
    .expect_close = true;
}

probe rawprobe {
    .request =
        "GET /healthz HTTP/1.1"
        "Host: example.com"
        "Connection: close";
    .threshold = 1;
}
```

The `.request` form relies on implicit concatenation of adjacent string literals (§7, gotcha 2).

#### ACL

```vcl
acl localnetwork {
    "localhost";               // resolved at VCL load time
    "192.0.2.0"/24;            // CIDR — mask is OUTSIDE the quotes
    ! "192.0.2.23";            // negation
    ("unresolvable.example");  // parenthesised: ignored if DNS fails
}

acl foo -pedantic +log +table +fold(-report) {
    "firewall.example.com" / 24;
}
```

Flags: `+log`, `+table`, `-pedantic`, `-fold` / `+fold(+report)` / `+fold(-report)`.

#### Subroutines

```vcl
sub pipe_if_local {
    if (client.ip ~ localnetwork) {     // ACL reference
        return (pipe);
    }
}

sub vcl_recv {
    call pipe_if_local;                 // subroutine reference
}
```

Properties:
- Take **no arguments**, return **no values**.
- The `vcl_` prefix is reserved.
- A bare `return;` exits a *custom* sub without a state transition; built-in subs require `return(action)`.
- **Multiple definitions of the same built-in sub name are concatenated in source order**, with Varnish's shipped built-in VCL implicitly appended. See §6.1.

#### Built-in subroutines

Client side:

| Sub | Legal `return()` actions |
|---|---|
| `vcl_recv` | `hash`, `pass`, `pipe`, `purge`, `synth`, `restart`, `vcl`, `fail` |
| `vcl_pipe` | `pipe`, `synth`, `fail` |
| `vcl_pass` | `fetch`, `synth`, `restart`, `fail` |
| `vcl_hash` | `lookup`, `fail` |
| `vcl_purge` | `restart`, `synth`, `fail` |
| `vcl_miss` | `fetch`, `pass`, `restart`, `synth`, `fail` |
| `vcl_hit` | `deliver`, `miss`, `pass`, `restart`, `synth`, `fail` |
| `vcl_deliver` | `deliver`, `restart`, `synth`, `fail` |
| `vcl_synth` | `deliver`, `restart`, `fail` |

Backend side:

| Sub | Legal `return()` actions |
|---|---|
| `vcl_backend_fetch` | `fetch`, `abandon`, `error`, `fail` |
| `vcl_backend_response` | `deliver`, `retry`, `abandon`, `pass`, `error`, `fail` |
| `vcl_backend_error` | `deliver`, `retry`, `abandon`, `fail` |
| `vcl_backend_refresh` | (newer; readable `obj_stale.*`) |

Housekeeping: `vcl_init` (`return(ok)`/`return(fail)`, the **only** place `new` is legal) and `vcl_fini` (`return(ok)`).

#### `new` — VCL object instantiation

```vcl
import directors;

sub vcl_init {
    new cluster = directors.round_robin();
    cluster.add_backend(node1);
    cluster.add_backend(node2);
}
```

Only legal in `vcl_init`, but the resulting instances are **globally visible**. Scope of declaration ≠ scope of visibility.

#### `unused` — reference-count suppression

Varnish errors on declared-but-unreferenced backends, probes, ACLs and subs. `unused` marks them intentional:

```vcl
unused b1;
unused p1;
unused acl1;
unused sb;
```

A pseudo-reference: satisfies the compiler's use-check, no runtime semantics.

### 2.2 VCL variable namespaces

Not extracted as entities, but the lexer must tokenise them correctly and the reference extractor must recognise `set <var> = <backend>` patterns.

**Connection topology** — `client.*`, `server.*`, `remote.*`, `local.*`:

```
without PROXY:        client   server
                      remote   local
                        v        v
              CLIENT ------------ VARNISHD

with PROXY:           client   server   remote   local
                        v        v        v        v
              CLIENT ------------ PROXY ------------ VARNISHD
```

Roots: `client.*`, `server.*`, `remote.*`, `local.*`, `req.*`, `req_top.*`, `bereq.*`, `beresp.*`, `obj.*`, `obj_stale.*`, `resp.*`, `sess.*`, `storage.<name>.*`, `param.*`, `now`.

Two subtleties worth encoding:

- **`storage.<name>.*` has a user-defined infix segment** — the middle component is a stevedore name from the varnishd command line, not a fixed keyword. The matcher needs a wildcard slot.
- **Protected headers**: `content-length` and `transfer-encoding` are read-only from VCL on all of `req`, `bereq`, `beresp`, `resp`.
- **Header names are VCL symbols** — cannot start with a numeral; a quoted form exists for otherwise-invalid names (§7, gotcha 7).

Backend-carrying variables, relevant for `UsesBackend` edges: `req.backend_hint`, `bereq.backend`, `beresp.backend`.

### 2.3 VCL type system

`STRING`, `STRANDS`, `INT`, `REAL`, `BOOL`, `DURATION`, `TIME`, `BYTES`, `IP`, `BLOB`, `HTTP`, `HEADER`, `BODY`, `BACKEND`, `ACL`, `PROBE`, `STEVEDORE`, `SUB`, `INSTANCE`, `VOID`.

Used for `.vcc` signature extraction (§2.5) and for signature strings on VCL entities.

### 2.4 VTC structure

A `.vtc` file is a line-oriented script of top-level commands: keyword, optional instance name, optional `{ ... }` block, trailing `-flag [value]` arguments.

**Lexical rules, verified from `/docs/reference/vtc/` (section PARSING):**

> A vtc file will be read word after word, with very little tokenization, meaning a syntax error won't be detected until the test actually reach the relevant action in the test.

> The parser splits words by detecting whitespace characters and a string is a word, or a series of words on the same line enclosed by double-quotes (`"…"`), or, for multi-line strings, enclosed in curly brackets (`{…}`).

> The leading whitespaces of lines are ignored. Empty lines (or ones consisting only of whitespaces) are ignored too, as are the lines starting with "#" that are comments.

Three consequences that differ from VCL and must not be conflated:

1. **VTC multi-line strings are plain curly brackets `{…}`, not `{"…"}`.** VCL's long-string form is `{"…"}`; VTC's is bare `{…}`. The same brace character therefore serves as both the block delimiter and the multi-line string delimiter in VTC — which is precisely why the block scanner must track quoting state (§7, gotcha 11).
2. **VTC tokenisation is whitespace-driven and extremely permissive.** There is no grammar to validate against; errors surface at execution time. The parser must be correspondingly tolerant — unknown commands and flags are skipped, never fatal.
3. **Comments are `#`-only and must be at the start of the line** (after leading whitespace). VCL's `//` and `/* */` are not VTC comment syntax at the VTC level, though they remain valid inside embedded VCL blocks.

```vtc
varnishtest "Basic cache hit"

server s1 {
    rxreq
    expect req.url == "/foo"
    txresp -status 200 -hdr "Cache-Control: max-age=60" -body "hello"
} -start

varnish v1 -vcl+backend {
    import std;
    sub vcl_recv        { std.log("saw " + req.url); }
    sub vcl_backend_response { set beresp.ttl = 60s; }
} -start

client c1 {
    txreq -url "/foo"
    rxresp
    expect resp.status == 200
    expect resp.body == "hello"
} -run

varnish v1 -expect cache_hit == 1
```

`varnishtest "..."` and `vtest "..."` are equivalent spellings; both must be accepted.

#### Top-level commands

| Command | Purpose |
|---|---|
| `varnishtest` / `vtest` | test description (first line) |
| `server sNAME { ... }` | mock HTTP origin |
| `client cNAME { ... }` | HTTP client driver |
| `varnish vNAME ...` | a varnishd instance |
| `barrier bNAME ...` | synchronisation primitive |
| `logexpect lNAME ...` | assertions over the VSL log |
| `haproxy hNAME -conf {...}` | HAProxy instance (PROXY-protocol tests) |
| `syslog SNAME { ... }` | mock syslog receiver |
| `feature <flag>` | skip test unless capability present |
| `shell "cmd"` | run a shell command |
| `process pNAME { ... }` | drive a subprocess / pty |
| `delay N`, `setenv`, `filewrite`, `random` | misc |

#### `varnish` flags

| Flag | Meaning |
|---|---|
| `-vcl { ... }` | load inline VCL **without** auto-generated backends |
| `-vcl+backend { ... }` | load inline VCL, **auto-prepending a backend per declared `server`** |
| `-arg "..."` | append a raw varnishd command-line argument |
| `-start` / `-stop` / `-wait` | lifecycle |
| `-cli` / `-cliok` / `-clierr` / `-cliexpect` | CLI commands with varying assertions |
| `-errvcl <msg> { vcl }` | assert this VCL **fails** to compile |
| `-expect <counter> <op> <val>` | assert on a varnishstat counter |
| `-vsl_catchup`, `-banner` | misc |

`-vcl+backend` and `-errvcl` both require special handling — see §6.2 and §6.3.

#### Referencing external VCL

```vtc
# 1. via the varnishd -f argument
varnish v1 -arg "-f ${testdir}/external.vcl" -start

# 2. via an include inside an inline block
varnish v1 -vcl+backend {
    include "${testdir}/shared/backends.vcl";
    sub vcl_recv { call shared_logic; }
} -start

# 3. via the CLI
varnish v1 -cliok "vcl.load newconf ${testdir}/other.vcl"
```

#### Macros

`${...}` substitution throughout: `${testdir}`, `${tmpdir}`, `${pwd}`, `${topbuild}`, `${localhost}`, `${bad_ip}`, `${date}`, `${s1_addr}`, `${s1_port}`, `${s1_sock}`, `${v1_addr}`, `${v1_port}`, `${v1_sock}`, `${v1_name}`, `${vmod_std}`, and the generator `${string,repeat,<n>,<str>}`.

The `${sN_*}` / `${vN_*}` families are **cross-entity references within the VTC** — `${s1_port}` inside a VCL block refers to the `server s1` declaration.

#### `logexpect`

```vtc
logexpect l1 -v v1 -g vxid -q "vxid > 0" {
    expect 0 1000 Begin    "^req"
    expect * =    ReqURL   "^/foo$"
    expect * =    VCL_call "^RECV$"
    expect * =    End
} -run
```

Grammar: `expect <skip> <vxid> <tag> <regex>`, where `skip` is a count or `*`, and `vxid` is a literal, `=` (same as previous) or `*`. The `<tag>` field is a VSL tag name from a fixed vocabulary (`ReqURL`, `VCL_call`, `VCL_Log`, `BereqURL`, `Debug`, …).

### 2.5 VCC structure

A `.vcc` file is the source of truth for a VMOD's VCL-visible API. Format: **directive lines starting with `$`, interleaved with reStructuredText prose** that becomes the man page.

```rst
$Module cookie 3 "Varnish Cookie Module"
$ABI strict

DESCRIPTION
===========

This VMOD parses and manipulates cookies.

$Function VOID parse(STRING cookieheader)

Parse the cookie header.

$Function STRING get(STRING cookiename)
$Function BOOL isset(STRING cookiename)

$Object counter(STRING name, INT initial = 0)

A counter object.

$Method VOID .incr(INT n = 1)
$Method INT .get()

$Event event_function
$Restrict client backend
```

| Directive | Form | Meaning |
|---|---|---|
| `$Module` | `$Module <name> <section> "<desc>"` | exactly one per file; defines the VCL namespace prefix |
| `$ABI` | `$ABI strict` \| `vrt` | ABI compatibility mode |
| `$Function` | `$Function <rettype> <name>(<params>)` | called as `<module>.<name>()` |
| `$Object` | `$Object <name>(<ctor-params>)` | instantiated with `new` |
| `$Method` | `$Method <rettype> .<name>(<params>)` | method on the **preceding** `$Object`; note leading dot |
| `$Event` | `$Event <c_function_name>` | lifecycle hook |
| `$Restrict` | `$Restrict <contexts...>` | limits which VCL subs may call the preceding function |
| `$Alias` | `$Alias <alias> <target>` | deprecated-name alias |
| `$Synopsis` | `$Synopsis auto` \| `manual` | doc generation control |
| `$Prefix` | | C symbol prefix override |

Parameter syntax:

```
STRING s                       # required
INT n = 5                      # default value
ENUM {one, two, three} e       # enumerated
ENUM {a, b} e = "a"            # enumerated with default
[STRING optional]              # optional (older bracket form)
PRIV_TASK, PRIV_CALL,          # private-state pointers, invisible from VCL
PRIV_TOP, PRIV_VCL
STRING_LIST                    # variadic string concat (legacy)
STRANDS                        # modern replacement for STRING_LIST
```

---

## 3. Data model

### 3.1 New `EntityKind` variants

Added to `src/models/entity.rs:16`:

```rust
// Varnish VCL entities
VclVersion,        // vcl 4.1;
VclSubroutine,     // sub my_helper { }
VclBuiltinSub,     // sub vcl_recv { }   (multi-part, see §6.1)
VclBackend,        // backend default { }
VclProbe,          // probe healthcheck { }
VclAcl,            // acl localnetwork { }
VclImport,         // import std;  /  import std as s;
VclObjectInstance, // new cluster = directors.round_robin();

// Varnish VTC entities
VtcTestCase,        // varnishtest "..." / vtest "..."
VtcServer,          // server s1 { }
VtcClient,          // client c1 { }
VtcVarnishInstance, // varnish v1 { }
VtcLogexpect,       // logexpect l1 { }
VtcBarrier,         // barrier b1 cond 2

// Varnish VCC entities
VccModule,   // $Module cookie 3 "..."
VccFunction, // $Function VOID parse(STRING)
VccObject,   // $Object counter(STRING, INT)
VccMethod,   // $Method VOID .incr(INT)
```

Each variant requires synchronising **three exhaustive `match` statements**. All three are exhaustive, so the compiler reports every omission — follow the errors:

| Mapping | Location | Produces |
|---|---|---|
| `impl Display for EntityKind` | `src/models/entity.rs:110` | snake_case string → `s.kind` property in Neo4j. **This is what E2E Cypher assertions match on.** |
| `kind_to_label` | `src/db/graph/utils.rs:4` | PascalCase Neo4j node label (`SET n:{label}`) |
| `compute_fqn_and_context` | `src/pipeline/parser/context.rs:83` | FQN construction strategy |

Display strings follow the existing convention (`markdown_document`, `project_identity`):

```
vcl_version, vcl_subroutine, vcl_builtin_sub, vcl_backend, vcl_probe,
vcl_acl, vcl_import, vcl_object_instance,
vtc_test_case, vtc_server, vtc_client, vtc_varnish_instance,
vtc_logexpect, vtc_barrier,
vcc_module, vcc_function, vcc_object, vcc_method
```

Neo4j labels are interpolated into Cypher rather than parameterised (`src/db/graph/upsert.rs:227`), because Neo4j cannot parameterise labels.

### 3.2 New `ReferenceIntent` variants

Added to `src/models/relationship.rs:13`. Every variant carries a `line`:

```rust
VclSubCall     { sub_name: String, line: usize },
VclBackendRef  { backend_name: String, line: usize },
VclProbeRef    { probe_name: String, line: usize },
VclAclRef      { acl_name: String, line: usize },
VclInclude     { path: String, line: usize },
VclVmodImport  { module: String, alias: Option<String>, line: usize },
VclUnusedRef   { name: String, line: usize },
```

VMOD function calls (`std.log(...)`) and instance method calls (`cluster.backend()`) reuse the existing `Call { method, receiver, line, arg_count }` variant — `receiver` carries the module or instance name.

### 3.3 New `RelationshipType` variants

Added to `src/models/relationship.rs:106`, with `Display` arms at `:137`:

| Variant | Display (Cypher edge label) |
|---|---|
| `UsesBackend` | `USES_BACKEND` |
| `UsesProbe` | `USES_PROBE` |
| `UsesAcl` | `USES_ACL` |
| `Includes` | `INCLUDES` |
| `ImportsVmod` | `IMPORTS_VMOD` |
| `DeclaredUnused` | `DECLARED_UNUSED` |

**Design rationale for dedicated edge types.** Collapsing these into the generic `References` would be less code, but *"which backends does this configuration actually route to?"* is the single most common question asked of a Varnish deployment. It deserves a first-class edge so `find_callers` and subgraph traversal can filter on it.

`DeclaredUnused` exists specifically so that `unused b1;` does **not** appear as a real usage in `find_callers` output. It is a compiler-satisfaction marker, not a runtime reference.

### 3.4 Intent → relationship dispatch

Added to the dispatch table in `src/pipeline/ingest/resolve/mod.rs:167`, following the existing pattern:

| `ReferenceIntent` | Resolver | `RelationshipType` |
|---|---|---|
| `VclSubCall` | `calls::resolve_single_call_intent` | `Calls` |
| `Call` (VMOD/instance) | `calls::resolve_single_call_intent` | `Calls` |
| `VclBackendRef` | `non_calls::resolve_non_call_reference` | `UsesBackend` |
| `VclProbeRef` | `non_calls::resolve_non_call_reference` | `UsesProbe` |
| `VclAclRef` | `non_calls::resolve_non_call_reference` | `UsesAcl` |
| `VclInclude` | direct `fqn_to_uuid.get(path)` | `Includes` |
| `VclVmodImport` | `non_calls::resolve_non_call_reference` | `ImportsVmod` |
| `VclUnusedRef` | `non_calls::resolve_non_call_reference` | `DeclaredUnused` |

`VclInclude` follows the `HtmlFileImport` / `CssFileImport` pattern (`resolve/mod.rs:262-269`): the raw path string is used directly as the FQN key. This is the established mechanism for file-level edges.

### 3.5 FQN scheme

VCL has a flat global namespace, so FQNs are far simpler than the Rust or Java schemes:

```
vcl:<repo>:<name>                  // sub, backend, probe, acl, object instance
vcl:<repo>:<file>:<name>           // anonymous inline probes
vcl:<repo>:<file>                  // VclVersion (file-level attribute)
vtc:<repo>:<file>:<instance>       // server s1, client c1, varnish v1, logexpect l1
vcc:<module>::<function>           // $Function
vcc:<module>::<object>             // $Object
vcc:<module>::<object>::<method>   // $Method (positional binding to preceding $Object)
```

VTC entities are file-scoped because `server s1` in two different `.vtc` files are unrelated. VCL entities are repo-scoped because the include graph merges them into one global namespace at load time.

VCC entities are keyed on the module name rather than the repo, so a VMOD indexed once resolves call sites across every repo that imports it — the same mechanism that makes cross-repo resolution work today (`load_entity_mappings` pulls from Neo4j across all loaded repos).

### 3.6 UUID stability

`ParsedEntity::new` computes a deterministic v5 UUID over `repo_name:file_path:fqn:start_line` (`src/models/entity.rs:404`). Consequences for this work:

- `file_path` must be **repo-relative**, as produced by `parse_single_file` (`src/pipeline/parser/mod.rs:222`).
- Multiple `sub vcl_recv` definitions across files do **not** collide, because `file_path` and `start_line` differ (§6.1).
- VCL entities synthesised from VTC `server` blocks (§6.2) must be given the `start_line` of the originating `server` declaration, so they are stable across re-indexing.

---

## 4. Module layout

Following the precedent of `src/pipeline/parser/languages/rust/` (the only existing directory-module):

```
src/pipeline/parser/languages/varnish/
├── mod.rs          // pub(crate) re-exports of the three entry points
├── dialect.rs      // Fastly detection guard
├── lexer.rs        // shared tokeniser for VCL (used by vcl.rs and vtc.rs)
├── vcl.rs          // extract_entities_vcl
├── vtc.rs          // extract_entities_vtc
└── vcc.rs          // extract_entities_vcc
```

Entry point signatures follow the established convention (cf. `properties.rs:9`, `toml.rs:4`):

```rust
pub(crate) fn extract_entities_vcl(
    source: &str,
    file_path: &str,
    repo_name: &str,
) -> Vec<ParsedEntity>

pub(crate) fn extract_entities_vtc(
    source: &str,
    file_path: &str,
    repo_name: &str,
) -> Vec<ParsedEntity>

pub(crate) fn extract_entities_vcc(
    source: &str,
    file_path: &str,
    repo_name: &str,
) -> Vec<ParsedEntity>
```

### 4.1 Error handling convention

Established by `toml.rs:11-14` and followed by every hand-written parser in the repo:

- **Never `panic!`.** Malformed input is normal — VCL in the wild includes work-in-progress files.
- **Never return `Err`.** The return type is `Vec<ParsedEntity>`, not `Result`.
- A parse failure returns an **empty `Vec`** and logs at `debug`.
- A *partial* parse failure (one malformed backend among ten) emits the entities that did parse and skips the rest. Recovery is at statement boundaries (`;` at brace depth 0, or a closing `}`).

---

## 5. Reference extraction catalogue

Each row is a distinct syntactic form producing a graph edge.

| Form | Example | Intent | Edge |
|---|---|---|---|
| Subroutine call | `call pipe_if_local;` | `VclSubCall` | `Calls` |
| SUB passed to VMOD | `std.timed_call(mysub)` | `VclSubCall` | `Calls` |
| VMOD function | `std.log("x")` | `Call` (receiver=module) | `Calls` |
| VMOD constructor | `new c = directors.round_robin()` | `Call` | `Calls` |
| Instance method | `cluster.backend()` | `Call` (receiver=instance) | `Calls` |
| Backend hint | `set req.backend_hint = b;` | `VclBackendRef` | `UsesBackend` |
| Backend assign | `set bereq.backend = b;` | `VclBackendRef` | `UsesBackend` |
| Director member | `cluster.add_backend(b)` | `VclBackendRef` | `UsesBackend` |
| Backend health | `std.healthy(b)` | `VclBackendRef` | `UsesBackend` |
| Backend via | `.via = tunnel;` | `VclBackendRef` | `UsesBackend` |
| Probe reference | `.probe = myprobe;` | `VclProbeRef` | `UsesProbe` |
| ACL match | `client.ip ~ localnetwork` | `VclAclRef` | `UsesAcl` |
| ACL negated match | `client.ip !~ localnetwork` | `VclAclRef` | `UsesAcl` |
| File include | `include "f.vcl";` | `VclInclude` | `Includes` |
| VMOD import | `import std;` | `VclVmodImport` | `ImportsVmod` |
| Unused marker | `unused b1;` | `VclUnusedRef` | `DeclaredUnused` |

### 5.1 Call prefix resolution order

`<prefix>.<name>(...)` is ambiguous between a VMOD function and an instance method. Resolution order within a file:

1. Look up `prefix` in the **`new`-instance table** → instance method call.
2. Look up `prefix` in the **import alias table** → VMOD function call.
3. Otherwise → unresolved; emit the `Call` intent anyway and let the global resolver try.

The instance table takes precedence because a `new` binding and an import share the same flat namespace, and Varnish resolves instances first.

### 5.2 Bare identifiers in VMOD argument position

A bare identifier passed to a VMOD function may be a `SUB`, a `BACKEND`, a `PROBE`, an `ACL` or a `STEVEDORE` — the VCL type system distinguishes them, but the parser does not have types.

**Strategy without `.vcc` data:** resolve against the file's own declaration tables (subs, backends, probes, ACLs) and emit the corresponding typed intent. If a name is declared as a backend, `std.healthy(that_name)` is a `VclBackendRef`. If ambiguous or undeclared, emit nothing rather than guessing.

**Strategy with `.vcc` data (§6.3):** once VCC parsing lands, the parameter type is known from the signature, and the intent kind follows directly. This is a resolution-time improvement, not a parse-time one — the parser still emits its best guess, and the resolver can refine it.

---

## 6. Hard problems

None of these is the VCL grammar itself. The grammar is small; these three are where the real work is.

### 6.1 `sub vcl_recv` is not a unique definition

**Problem.** Multiple definitions of the same built-in sub name are **concatenated in source order**, and Varnish's shipped built-in VCL is implicitly appended at the end. A model that treats `sub vcl_recv` as a single definition site silently drops code when two files both define it — which is the normal organisation of any non-trivial Varnish deployment.

**Solution.** Model built-in subs as a multi-part entity:

1. Each `sub vcl_recv` occurrence is its own `VclBuiltinSub` entity with its real `file_path` and `start_line`. The v5 UUID includes both, so they do not collide.
2. Their FQN is `vcl:<repo>:vcl_recv` — deliberately identical across parts. This is what makes them resolve as one logical target for `call`-site and search purposes.
3. Because the FQN is shared, `fqn_to_uuid` will map the FQN to exactly one of the parts (last writer wins). To avoid arbitrary resolution, the parser additionally emits a **synthetic aggregator entity** per built-in sub name per repo, and each part gets a `Contains` relationship from the aggregator.

The aggregator's `file_path` is the first-encountered file (sorted deterministically, since `discover_files` sorts — `src/pipeline/input.rs:184`), `start_line` is 0.

**Test coverage:** fixtures `multi_recv_a.vcl` and `multi_recv_b.vcl` both define `sub vcl_recv`; the E2E asserts exactly 2 `vcl_builtin_sub` parts plus 1 aggregator, and that both parts' bodies are searchable.

Custom subs (`VclSubroutine`) do **not** have this behaviour — redefining a custom sub is a compile error in Varnish.

### 6.2 VTC synthesises backends that do not appear in the source

**Problem.** `varnish v1 -vcl+backend { ... }` injects a backend declaration for **every declared `server`** ahead of the VCL you can see:

```vcl
backend s1 { .host = "127.0.0.1"; .port = "43210"; }
```

Without modelling this, every `set req.backend_hint = s1;` inside a `.vtc` is a dangling reference — and since `-vcl+backend` is the overwhelmingly common form in the Varnish test suite, that means most VTC-embedded VCL would be broken.

**Solution.** When parsing a `varnish <name> -vcl+backend { ... }` block:

1. Scan the whole `.vtc` for `server <name>` declarations (they may appear before *or* after the `varnish` block — collect them in a pre-pass).
2. For each, synthesise a `VclBackend` entity named after the server, with:
   - `file_path` = the `.vtc` file
   - `start_line` = the line of the originating `server` declaration (for UUID stability)
   - `signature` = `backend <name> { .host = "${<name>_addr}"; .port = "${<name>_port}"; }`
   - a marker in `docstring` noting it is synthesised by `-vcl+backend`
3. Emit a `References` edge from the synthesised backend to the `VtcServer` entity, so the provenance is navigable.

For the plain `-vcl { ... }` form, no synthesis — that flag exists precisely to opt out.

### 6.3 VMOD calls are only typed if `.vcc` is parsed

**Problem.** Without `.vcc`, `std.querysort(req.url)` is an opaque token. The call site cannot be resolved to a definition, and `find_callers` on a VMOD function returns nothing.

**Why it matters.** This is the single highest-leverage piece of the whole feature — it turns every VMOD call site in every indexed VCL into a typed edge. It is also the easiest of the three parsers to write: a line-oriented `$Directive` scanner with RST prose to skip.

**Solution.** Parse `.vcc` into `VccModule` / `VccFunction` / `VccObject` / `VccMethod` entities whose FQNs are `vcc:<module>::<name>`. VCL call sites emit `Call { receiver: "std", method: "querysort" }`, and the resolver matches on the `vcc:std::querysort` FQN.

**Positional binding caveat.** `$Method` binds to the nearest **preceding** `$Object`, with no closing delimiter. The parser must track "current object" state across the RST prose between directives. A `$Method` appearing before any `$Object` is malformed — skip it and log at `debug`.

### 6.4 `-errvcl` blocks contain deliberately invalid VCL

`varnish v1 -errvcl "expected msg" { ... }` asserts that the enclosed VCL **fails to compile**. Indexing it produces entities for code that is, by construction, wrong.

**Solution.** Detect the `-errvcl` flag and skip the following block entirely. Do not emit entities, do not log parse failures. The `.vtc` file's other blocks are parsed normally.

---

## 7. Lexing gotchas

Every item here is a mandatory unit test, written **before** the corresponding lexer code. These are the constructs that break naive VCL parsers.

**1. ACL mask outside the quotes.**
```vcl
"192.0.2.0"/24
"firewall.example.com" / 24    // whitespace permitted
```
A string literal immediately followed by `/` and an integer. `/` is also the division operator. Nothing else in the language looks like this, and it is the single most common failure point of ad-hoc VCL parsers.

**2. Adjacent string literals concatenate implicitly.**
```vcl
.request = "GET / HTTP/1.1" "Host: x" "Connection: close";
```
No operator between them. A lexer that expects an operator between two string tokens fails here. Each literal becomes one CRLF-terminated request line.

**3. Attribute names begin with `.` in statement position.**
```vcl
.host = "x";
```
Collides with member-access lexing. Only legal inside `backend` / `probe` blocks.

**4. `~` is overloaded between regex and ACL.**
```vcl
if (req.http.host ~ "(?i)example\.com$") { }   // regex — NOT an ACL edge
if (client.ip ~ localnetwork)            { }   // ACL reference — edge
```
Disambiguate on the RHS token type: string literal → PCRE regex; identifier → ACL reference. Getting this wrong either loses every ACL edge or fabricates edges from regexes.

**5. `-` inside identifiers.**
```vcl
req.http.X-Forwarded-For
```
A single dotted identifier chain whose final component contains hyphens — but `-` is also subtraction. `a-b` is one identifier, not a subtraction. Varnish resolves this by maximal-munch on the identifier; arithmetic requires whitespace. Consequence: `set req.http.x = 1-2;` does not parse the way a C-like language would suggest.

**6. Durations are number-immediately-followed-by-unit.**
```vcl
1.5s  10ms  2h  7d  4w  1y
```
Units: `ms s m h d w y`. `10m` is ten minutes; `10ms` is ten milliseconds. The lexer must munch maximally, or it reads `10m` followed by an identifier `s`.

**7. Quoted header names.**
```vcl
set req.http."grammatically.valid" = "1";
```
A string literal appearing as a *name* component mid-dotted-path. Needed because header names are VCL symbols and cannot otherwise start with a numeral or contain dots.

**8. Long strings.**
```vcl
{"long string
  spanning lines and containing " double quotes"}

"""triple-quoted
   long string"""
```
Basic strings `"..."` may **not** contain newlines. Long strings `{"..."}` and `"""..."""` may contain any character including newlines and unescaped double quotes, except NUL. **`{"` breaks naive brace counting** when scanning VTC blocks — see gotcha 11.

**9. Extended status codes.**
```vcl
return synth(12404);
```
VCL permits `VWXYZ`-form statuses up to 65535, where only the low three digits reach the client. Do not validate as three digits.

**10. Three comment styles.**
```vcl
// C++ style
#  shell style
/* C style
   multi-line */
```
VTC uses `#` only. VCC uses `#` for directive-file comments, with free-form RST prose between directives.

**11. VTC brace-block scanning vs. embedded VCL.**
A VTC `{ ... }` block is delimited by brace balance, but the VCL inside may contain braces in strings, and VCL long strings are literally `{"..."}` — a brace-quote sequence. Naive brace counting over a `-vcl+backend { ... }` block mis-terminates on `{"`. **The block scanner must track string state.**

**12. VTC `${...}` macros inside embedded VCL.**
```vtc
varnish v1 -vcl+backend {
    backend b { .port = "${s1_port}"; }
}
```
Produces text that is not valid VCL until expanded. Either expand before parsing or make the lexer tolerate macro tokens. **Decision: tolerate.** The lexer emits a `Macro(name)` token for `${...}`, because expansion requires runtime values (`${s1_port}` is only known when varnishtest runs). Expanding `${testdir}` for include resolution is a separate, targeted substitution done by the VTC parser.

**13. `.probe = X` has two right-hand sides.**
```vcl
.probe = myprobe;        // identifier → UsesProbe edge
.probe = { .url = "/"; } // block → anonymous nested entity, no edge
```

**14. `$Method` binding in VCC is positional.**
Binds to the nearest preceding `$Object`, with no closing delimiter. State must be tracked across intervening RST prose.

**15. VCL does NOT process backslash escapes in strings — VERIFIED.**

**Finding: a backslash in a VCL string is an ordinary byte.** There is no escape processing of any kind. Backslashes pass through verbatim to the regex engine.

Evidence, gathered 2026-08-08 from <https://www.varnish.org/docs/>:

1. **The VCL reference documents no escape sequences at all.** The page at `/docs/reference/vcl/` contains **zero** occurrences of the words "escape" or "backslash", and zero backslash characters. Its Strings section reads in full:

   > Basic strings are enclosed in double quotes `" …"`, and may not contain newlines. Long strings are enclosed in `{" …"}` or `""" …"""`. They may contain any character including single double quotes `"`, newline and other control characters except for the NUL (0x00) character.

   It also states that "strings can contain any bytes except NUL (zero, 0), which marks the end of the string."

2. **The existence of long strings proves the absence of escapes.** Long-string syntax exists precisely so that `"` and newlines can be embedded. If `\"` and `\n` worked, `{"…"}` and `"""…"""` would be redundant. Conversely, a basic string has *no* way to contain a `"`.

3. **Official examples use single backslashes for PCRE constructs that are not valid C escapes.** From `/docs/reference/vmod_std/`:
   ```vcl
   set beresp.http.served-by = regsub(std.fileread("/etc/hostname"), "\R$", "");
   ```
   From `/docs/tutorials/example-vcl-template/` and `/docs/tutorials/configuring-varnish-wordpress/`:
   ```vcl
   set req.url = regsub(req.url, "\?$", "");
   if (req.http.cookie ~ "^\s*$")                       { unset req.http.cookie; }
   if (req.url ~ "^[^?]*\.(7z|avi|bmp|css|gif|jpg|js)$") { ... }
   if (req.url ~ "^/\.well-known/acme-challenge/")       { ... }
   set req.http.Cookie = regsuball(req.http.Cookie, "(__)?hs[a-z_\-]+=[^;]+(; )?", "");
   ```
   `\R` and `\s` are PCRE2 constructs and are **not** valid C escape sequences. Under C-style escape processing these would either fail to compile or silently degrade to `R` and `s`, breaking every one of these documented examples.

4. **No official example anywhere uses `\\` to denote a single backslash.** In any language with C-style escapes (Java, JSON, C), regex literals are riddled with doubled backslashes. Their complete absence across the entire documentation corpus is conclusive.

**Lexer consequences — this makes short-string scanning *simpler* than C, but in a way that breaks C habits:**

- A basic string is *everything between one `"` and the next `"`*. Full stop. No escape state machine.
- **A trailing backslash does NOT escape the closing quote.** `"C:\path\"` terminates at the quote after `path\`. A lexer written with C-string reflexes would treat `\"` as an escaped quote, run past the terminator, and swallow arbitrary following code into the string literal — silently corrupting every entity after it in the file. This is the single most damaging way to get gotcha 15 wrong, and it is exactly what a copied-from-C lexer does.
- Any regex ending in a backslash before the closing quote is therefore the highest-value test case in the suite.

---

## 8. Implementation phases

The repo's stated methodology (`AGENTS.md`): **the E2E test is written first and must fail before the logic is implemented.**

### Phase 0 — Fixtures and a red E2E

#### 0.1 Fixtures

`tests/testing_files/varnish/`:

| File | Exercises |
|---|---|
| `default.vcl` | version marker, imports (plain + `as`), backends, named probes, ACLs, custom subs, built-in subs, `call`, directors via `new` in `vcl_init`, `unused` |
| `backends.vcl` | backend + probe definitions, included by `default.vcl` |
| `edge_cases.vcl` | all 15 gotchas from §7 |
| `multi_recv_a.vcl` | `sub vcl_recv` — part 1 (§6.1) |
| `multi_recv_b.vcl` | `sub vcl_recv` — part 2 (§6.1) |
| `inline_probe.vcl` | `.probe = { }` anonymous form vs `.probe = named` |
| `basic_hit.vtc` | `server` / `client` / `varnish -vcl+backend` / `logexpect` / `barrier` |
| `external_ref.vtc` | `-arg "-f ${testdir}/default.vcl"`, `include` inside an inline block |
| `errvcl.vtc` | `-errvcl` block that must yield **no** entities (§6.4) |
| `vmod_cookie.vcc` | `$Module`, `$Function`, `$Object`, `$Method`, `$Event`, `$Restrict`, RST prose, ENUM and default params |
| `fastly_sample.vcl` | Fastly markers — must yield **no** entities (§1.3) |

**Unique-token convention.** Following the markdown suite, fixtures embed nonsense tokens (`VARNISH_SPHINX_TOKEN_42`, `VCC_GIZMO_TOKEN_99`, `VTC_ARTIFACT_TOKEN_31`) inside docstrings and bodies. A search hit on such a token proves specifically that body content reached `embed_text` and that same-named entities in different files did not collide.

#### 0.2 E2E script

`tests/run_varnish_e2e.sh`, templated from `tests/run_markdown_e2e.sh`:

- `QDRANT_COLLECTION="knot_varnish_e2e_test"`
- `REPO_NAME="varnish_e2e_test_repo"`
- `TEST_FILES_DIR=".../testing_files/varnish"`
- Shared high ports: Neo4j `17687`, Qdrant `16334`
- Honours `KNOT_E2E_EXTERNAL_DB` (skip docker lifecycle, omit `--clean`)
- Honours `KNOT_SKIP_BUILD`
- `cleanup()` + trap; leaves containers up on failure with manual-cleanup instructions

Two assertion mechanisms, as in the existing suites:

```bash
# MCP JSON-RPC over stdin
MCP_REQUEST="{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/call\",\"params\":{\"name\":\"search_hybrid_context\",\"arguments\":{\"query\":\"VARNISH_SPHINX_TOKEN_42\",\"repo_name\":\"$REPO_NAME\"}}}"
MCP_RESPONSE=$(echo "$MCP_REQUEST" | cargo run --release --bin knot-mcp 2>/dev/null | tail -n 1)

# Direct Cypher for structural claims search ranking cannot prove
run_neo4j_cypher() {
    echo "$1" | docker exec -i knot_neo4j_e2e cypher-shell -u "$NEO4J_USER" -p "$NEO4J_PASSWORD" \
        --format plain 2>/dev/null | tail -n +2
}
```

Cypher assertions match on **`s.kind`** — the `Display` string, not the node label. The `Display` impl is therefore load-bearing for tests.

#### 0.3 Assertion checklist

| # | Assertion | Mechanism |
|---|---|---|
| 1 | `vcl_backend` count matches fixtures | Cypher |
| 2 | `vcl_probe` named + inline both present | Cypher |
| 3 | `vcl_acl` present with correct name | Cypher |
| 4 | `call pipe_if_local` → `CALLS` edge | Cypher |
| 5 | `set req.backend_hint = b` → `USES_BACKEND` | Cypher |
| 6 | `.probe = myprobe` → `USES_PROBE` | Cypher |
| 7 | `client.ip ~ acl` → `USES_ACL` | Cypher |
| 8 | `client.ip ~ "regex"` → **no** `USES_ACL` | Cypher (count = 0) |
| 9 | `include "backends.vcl"` → `INCLUDES` | Cypher |
| 10 | `import std` → `IMPORTS_VMOD` | Cypher |
| 11 | `unused b1` → `DECLARED_UNUSED`, not `USES_BACKEND` | Cypher |
| 12 | Two `vcl_recv` parts + 1 aggregator, both bodies searchable | Cypher + MCP |
| 13 | `directors.round_robin()` → `vcl_object_instance` | Cypher |
| 14 | `cluster.add_backend(node1)` → `USES_BACKEND` | Cypher |
| 15 | VTC: `vtc_server` / `vtc_client` / `vtc_varnish_instance` present | Cypher |
| 16 | VTC: synthesised backend `s1` exists and is reachable | Cypher |
| 17 | VTC: `-errvcl` block yields zero entities | Cypher (count = 0) |
| 18 | VTC: entities carry `is_test_context = true` | Cypher |
| 19 | VTC: `include "${testdir}/..."` resolves | Cypher |
| 20 | VCC: `$Function` signature extracted with types | MCP |
| 21 | VCC: `$Method` bound to correct `$Object` | Cypher (FQN check) |
| 22 | VCC: `std.log(...)` in VCL → `CALLS` → `vcc:std::log` | Cypher |
| 23 | Fastly fixture yields zero entities | Cypher (count = 0) |
| 24 | Unique tokens searchable via `search_hybrid_context` | MCP |
| 25 | `explore_file` on `default.vcl` lists all top-level entities | MCP |

#### 0.4 Suite registration

`tests/run_all_e2e_fast.sh`:
- Add `"run_varnish_e2e.sh"` to the `SUITES` array (line 41)
- Update the `# All 18 suites now use the shared DB` comment → 19
- Update the count reference at line 177

#### 0.5 Gate

**Run the suite and confirm it fails.** Do not proceed until it does.

### Phase 1 — VCL

Ordered by compiler dependency.

| # | Change | File |
|---|---|---|
| 1.1 | ~~Verify gotcha 15~~ — **done, see §7 gotcha 15**: no escape processing | — |
| 1.2 | Add 8 VCL `EntityKind` variants | `src/models/entity.rs:16` |
| 1.3 | Add `Display` arms | `src/models/entity.rs:110` |
| 1.4 | Add `kind_to_label` arms | `src/db/graph/utils.rs:4` |
| 1.5 | Add `compute_fqn_and_context` arms | `src/pipeline/parser/context.rs:83` |
| 1.6 | Add 7 `ReferenceIntent` variants | `src/models/relationship.rs:13` |
| 1.7 | Add 6 `RelationshipType` variants + `Display` | `src/models/relationship.rs:106`, `:137` |
| 1.8 | Fastly detection guard + unit tests | `languages/varnish/dialect.rs` |
| 1.9 | Lexer — **tests for all 15 gotchas written first** | `languages/varnish/lexer.rs` |
| 1.10 | `extract_entities_vcl` + inline `#[cfg(test)] mod tests` | `languages/varnish/vcl.rs` |
| 1.11 | `pub(crate)` re-exports | `languages/varnish/mod.rs` |
| 1.12 | `pub mod varnish;` | `languages/mod.rs` |
| 1.13 | `"vcl" =>` arm in the `match ext` | `src/pipeline/parser/mod.rs:233` |
| 1.14 | **`CORE_EXTENSIONS` + `SUPPORTED_EXTENSIONS`** | `src/pipeline/input.rs:12`, `:24` |
| 1.15 | Verify `embed_text` assembly handles the new kinds | `src/pipeline/prepare.rs` |
| 1.16 | Verify watch-mode extension filter | `src/pipeline/watch.rs` |
| 1.17 | Dispatch-table arms for the new intents | `src/pipeline/ingest/resolve/mod.rs:167` |

> ⚠️ **Step 1.14 is the sharpest trap in the whole process.**
> `SUPPORTED_EXTENSIONS` (`src/pipeline/input.rs:24`) is documented as *"the single source of truth for all supported languages"*, but it is a **hand-maintained duplicated union** of `CORE_EXTENSIONS` and `CONFIG_EXTENSIONS`. There is no compile-time check that they agree.
>
> Adding `vcl`/`vtc`/`vcc` to `CORE_EXTENSIONS` but forgetting `SUPPORTED_EXTENSIONS` silently breaks `is_supported_file` — which drives **watch mode** and **incremental-state classification** — while `discover_files` continues to work. The failure is invisible in a normal `knot-indexer` run and only surfaces under `--watch` or on the second incremental pass.
>
> All three extensions go in **both** lists.

#### Lexer design

A hand-written tokeniser producing:

```rust
enum Token {
    Ident(String),        // maximal-munch, may contain '-' and '_'
    DottedPath(Vec<PathSegment>), // req.http."X-Foo" — segments may be quoted
    ShortString(String),  // "..."   no newlines
    LongString(String),   // {"..."} or """..."""
    Integer(i64),
    Real(f64),
    Duration(f64, DurationUnit), // maximal-munch on the unit
    Bytes(u64, ByteUnit),
    AttrName(String),     // leading '.' in statement position
    Macro(String),        // ${...} — VTC only, tolerated in VCL blocks
    Punct(char),          // { } ( ) ; , = + - * / ! ~ < > &
    Op(&'static str),     // == != <= >= && || !~ ~ +=
    Comment(String),      // //, #, /* */ — retained for docstrings
    Eof,
}
```

Key behaviours:
- `Ident` munches `[A-Za-z][A-Za-z0-9_-]*`. Arithmetic therefore requires surrounding whitespace (gotcha 5).
- `Duration` munches the longest matching unit suffix (gotcha 6): try `ms` before `m`.
- Entering a `backend`/`probe` block sets a flag so a leading `.` yields `AttrName` rather than `Punct('.')` (gotcha 3).
- String scanning handles `{"`/`"}` and `"""` as distinct delimiters (gotcha 8).
- Comments are retained as tokens so preceding comments can become the `docstring`, following the `properties.rs` `pending_comments` pattern.

#### Parser design

Single-pass recursive descent over the token stream, with two passes over the file:

- **Pass 1 — declarations.** Collect the names of every `sub`, `backend`, `probe`, `acl`, `import` and `new`. This table is needed before reference extraction, so that a bare identifier in VMOD argument position can be typed (§5.2), and so that call prefixes can be resolved (§5.1).
- **Pass 2 — bodies and references.** Walk statement bodies emitting `ReferenceIntent`s against the Pass-1 table.

Statement-level error recovery: on an unexpected token, skip to the next `;` at brace depth 0 or the next closing `}`, then continue. Emit what parsed.

### Phase 2 — VTC

| # | Change |
|---|---|
| 2.1 | 6 VTC `EntityKind` variants + the three synchronised matches |
| 2.2 | String-aware brace-block scanner (gotcha 11) |
| 2.3 | Top-level command parser (`server`/`client`/`varnish`/`logexpect`/`barrier`/`feature`/`shell`/…) |
| 2.4 | Flag parser (`-start`, `-run`, `-vcl`, `-vcl+backend`, `-arg`, `-errvcl`, `-cliok`, …) |
| 2.5 | Pre-pass collecting `server` declarations (needed by 2.6) |
| 2.6 | Backend synthesis for `-vcl+backend` (§6.2) |
| 2.7 | Delegate embedded VCL to `extract_entities_vcl` **with a line offset** so `start_line` is absolute within the `.vtc` |
| 2.8 | Skip `-errvcl` blocks entirely (§6.4) |
| 2.9 | `${testdir}` substitution for `include` path resolution |
| 2.10 | `${sN_*}` / `${vN_*}` macro → `References` edge to the declaring instance |
| 2.11 | Set `is_test_context = true` on all VTC entities and on VCL entities parsed from embedded blocks |
| 2.12 | `"vtc" =>` dispatch arm + `input.rs` lists |

**Line-offset correctness** (2.7) is worth calling out: `extract_entities_vcl` computes `start_line` relative to the source it is given. The VTC parser must add the block's starting line before the entities are returned, or every UUID for VTC-embedded VCL will be wrong and unstable across edits elsewhere in the file. Cleanest implementation: an internal `extract_entities_vcl_with_offset(source, file_path, repo_name, line_offset)`, with the public entry point delegating with `offset = 0`.

### Phase 3 — VCC

| # | Change |
|---|---|
| 3.1 | 4 VCC `EntityKind` variants + the three synchronised matches |
| 3.2 | Line-oriented `$Directive` scanner, skipping RST prose |
| 3.3 | Signature parser for the parameter grammar (§2.5): defaults, `ENUM {..}`, `[optional]`, `PRIV_*`, `STRANDS` |
| 3.4 | Positional `$Object` → `$Method` binding with state tracking (gotcha 14) |
| 3.5 | RST prose immediately following a directive becomes its `docstring` |
| 3.6 | `"vcc" =>` dispatch arm + `input.rs` lists |
| 3.7 | Verify VCL call sites resolve to `vcc:<module>::<name>` FQNs end-to-end |

`PRIV_TASK` / `PRIV_CALL` / `PRIV_TOP` / `PRIV_VCL` parameters are invisible from VCL and must be **excluded from the recorded signature**, otherwise arity-based call matching (`count_params_from_signature`, `src/pipeline/ingest/resolve/mod.rs:318`) will mismatch every call site.

### Phase 4 — Closing

| # | Task |
|---|---|
| 4.1 | `cargo fmt` |
| 4.2 | `cargo clippy --all-targets -- -D warnings` |
| 4.3 | `cargo test` |
| 4.4 | `./tests/run_all_e2e_fast.sh` — full suite, no regressions |
| 4.5 | `README.md` language-support table (required by `AGENTS.md`) |
| 4.6 | `docs/specs/multilanguage_roadmap.md` — add a Varnish phase entry |
| 4.7 | `.prompt` / `.knot-agent.md` if tool behaviour changed |
| 4.8 | Invoke the `validator` subagent (required by `AGENTS.md` for Rust changes) |

---

## 9. Testing strategy

### 9.1 Unit tests

Inline `#[cfg(test)] mod tests` at the bottom of each module, following `properties.rs:143` and `toml.rs:220`. Each test:

1. Defines a raw-string fixture inline
2. Calls the extractor with `("...", "test.vcl", "test-repo")`
3. Filters by kind: `entities.iter().filter(|e| e.kind == EntityKind::VclBackend)`
4. Asserts on `.len()`, `.name`, `.fqn`, `.signature`, `.docstring`, `.reference_intents`

Mandatory coverage:

**`lexer.rs`** — one test per gotcha in §7. These are written before any lexer code.

**`vcl.rs`** — each top-level construct; each reference form from the §5 catalogue; the `~` regex-vs-ACL disambiguation in both directions; `.probe` named vs inline; multiple `vcl_recv`; `unused`; empty input; malformed input (must return empty, not panic); partial-failure recovery.

**`vtc.rs`** — each top-level command; `-vcl` vs `-vcl+backend`; backend synthesis; `-errvcl` exclusion; brace scanning across `{"` long strings; macro tolerance; line-offset correctness for embedded VCL.

**`vcc.rs`** — each directive; parameter grammar variants; `$Method` binding across intervening prose; `$Method` before any `$Object` (malformed); `PRIV_*` exclusion from signatures.

**`dialect.rs`** — each Fastly marker triggers the guard; a valid Varnish file does not.

### 9.2 E2E tests

As specified in §8 Phase 0. The 25 assertions in §8.0.3 are the contract.

### 9.3 Regression policy

Per `AGENTS.md`: every bug found after merge gets an E2E case **before** the fix.

---

## 10. Known limitations

To be documented in `README.md`.

### 10.1 Bare `include` is not resolvable

`include "foo.vcl";` without a `./` or `/` prefix is searched across `vcl_path`, a varnishd command-line parameter that does not appear anywhere in the source tree. Only `./relative` and `/absolute` forms are resolvable from code alone.

**Mitigation:** emit the `VclInclude` intent regardless. If the resolver finds no match, the edge is simply absent — the same behaviour as any other unresolved reference in knot. A future enhancement could parse `varnish.service` / `/etc/default/varnish` to recover `vcl_path`.

### 10.2 `include +glob` requires filesystem expansion

`include +glob "/etc/varnish/sites/*.vcl";` is a one-to-many edge. Expansion happens at index time against the actual filesystem, so results depend on what exists on the indexing machine. Only absolute and `./`-relative globs are expandable.

### 10.3 Fastly VCL is not indexed

Detected and skipped (§1.3). Files yield zero entities.

### 10.4 Rust VMODs have no `.vcc`

VMODs written with the `varnish` crate declare their interface via `#[varnish::vmod]`. Their `.rs` files are indexed by the Rust parser, but VCL call sites into them will not resolve to typed signatures.

### 10.5 Built-in VCL is not indexed

Varnish appends its own shipped built-in VCL to every configuration. That source is not in the user's repo and is not indexed, so `return(lookup)` fall-through behaviour is invisible in the graph.

---

## 11. Acceptance criteria

- [ ] `cargo test` passes; all new unit tests green
- [ ] `cargo clippy --all-targets -- -D warnings` clean
- [ ] `cargo fmt -- --check` clean
- [ ] `./tests/run_all_e2e_fast.sh` passes with 19 suites, no regressions
- [ ] All 25 E2E assertions in §8.0.3 pass
- [ ] `.vcl`, `.vtc`, `.vcc` present in **both** `CORE_EXTENSIONS` and `SUPPORTED_EXTENSIONS`
- [ ] Watch mode picks up changes to all three extensions
- [ ] Incremental re-index correctly classifies all three extensions
- [ ] No `unsafe` blocks
- [ ] `README.md` language table updated
- [x] Gotcha 15 (backslash escapes) verified against the official documentation — VCL performs **no** escape processing; finding recorded in §7 gotcha 15
- [ ] Lexer treats `\` as an ordinary character in VCL strings, and as a line-continuation marker in VTC (§7 gotchas 15 and 16)

---

## 12. References

### 12.1 Upstream documentation

- VCL reference: <https://varnish-cache.org/docs/trunk/reference/vcl.html>
- VCL variables: <https://varnish-cache.org/docs/trunk/reference/vcl-var.html>
- VTC reference: <https://varnish-cache.org/docs/trunk/reference/vtc.html>
- VMOD / VCC: <https://varnish-cache.org/docs/trunk/reference/vmod.html>

### 12.2 Prior art

**[`M4R7iNP/varnishls`](https://github.com/M4R7iNP/varnishls)** — VCL language server written in **Rust**, 19 stars, last updated Dec 2025. The most mature artefact in the ecosystem. Its symbol-table design and include-graph traversal port directly to this work.

**Read it before writing the lexer.** It is not published on crates.io, so it is reference material rather than a dependency.

### 12.3 Rejected dependencies

**`tree-sitter-vcl = "0.4.0"`** (crates.io, repo `ntsk/tree-sitter-vcl`). Rejected. One published version, 13 total downloads, backing repo has 1 star. No tree-sitter grammar exists for VTC or VCC at all, so two of the three parsers would be hand-written regardless — a single consistent approach is preferable to a mixed one. VCL is small enough (≈10 top-level constructs, flat namespace) that a hand-written parser is tractable and gives full control over the node vocabulary.

Other VCL tree-sitter grammars surveyed, all rejected: `angaz/tree-sitter-vcl` (Fastly, stale since Mar 2024), `richardmarshall/tree-sitter-fastly-vcl` (Fastly, 7 commits), `isudzumi/tree-sitter-vcl` (dead since Apr 2022), `ea7jjq/tree-sitter-vcl` (no README).

### 12.4 In-repo templates

| Purpose | File |
|---|---|
| Hand-written line-oriented parser | `src/pipeline/parser/languages/properties.rs` |
| Hand-written parser with typed FQN namespaces | `src/pipeline/parser/languages/toml.rs` |
| Directory-module layout | `src/pipeline/parser/languages/rust/` |
| Hybrid/lexical parsing | `src/pipeline/parser/languages/groovy.rs` |
| E2E suite structure | `tests/run_markdown_e2e.sh` |
| Content-based sub-dispatch | `dispatch_yaml`, `src/pipeline/parser/mod.rs:478` |
