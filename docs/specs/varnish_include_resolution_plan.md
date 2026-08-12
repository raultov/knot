# Varnish VCL Include Resolution: TDD/BDD Implementation Plan

## 1. Context and Problem Statement
In recent versions, `knot` introduced support for indexing Varnish Cache configurations. However, `INCLUDES` relationships are missing for `include` directives that use absolute paths (e.g., `include "/etc/varnish/language.vcl";`).

**Root Cause:**
1. The parser (`src/pipeline/parser/languages/varnish/vcl.rs`) prematurely formats the included path into a Fully Qualified Name (FQN) by prepending `vcl:{repo_name}:` to the exact string found in the `include` directive. Thus, `include "/etc/varnish/language.vcl";` becomes `vcl:repo_name:/etc/varnish/language.vcl`.
2. The indexer discovers files relative to the repository root. A file physically located at `etc/varnish/language.vcl` within the repository will be indexed with the FQN `vcl:repo_name:etc/varnish/language.vcl` (no leading slash).
3. The resolver (`src/pipeline/ingest/resolve/mod.rs`) attempts an exact FQN match, which fails because the paths do not match exactly.

**Goal:**
Refactor the parsing and resolution logic so that `include` directives are correctly mapped to their target files, supporting absolute paths (mapped to repo root), relative paths, and fuzzy filename fallbacks. The implementation will follow a strict TDD/BDD methodology (Red-Green-Refactor).

---

## 2. Phase 1: BDD / End-to-End Tests (The "Red" Phase)
Before changing any code, we will define the expected behavior through end-to-end integration tests.

### 2.1. E2E Test Fixtures
We will add new fixture files in `tests/testing_files/varnish/` to represent the failing scenarios.

1.  **Create `tests/testing_files/varnish/etc/varnish/language.vcl`**
    ```vcl
    vcl 4.1;
    sub vcl_recv {
        set req.http.X-Language = "en";
    }
    ```
2.  **Modify `tests/testing_files/varnish/default.vcl`** (or create a new entrypoint) to include the new scenarios:
    ```vcl
    // Absolute include scenario
    include "/etc/varnish/language.vcl";
    ```

### 2.2. E2E Assertions
Update the bash test script `tests/run_varnish_e2e.sh` to assert the presence of the new `INCLUDES` edge.

```bash
# Append to tests/run_varnish_e2e.sh
assert_cypher_exists "X. Absolute INCLUDES edge" \
    "MATCH (a:Entity)-[r:INCLUDES]->(b:Entity) WHERE b.file_path = 'etc/varnish/language.vcl' AND a.repo_name = '$REPO_NAME' RETURN count(r)"
```

*Executing `./tests/run_varnish_e2e.sh` at this point must **FAIL** (Red).*

---

## 3. Phase 2: Unit Tests (The "Red" Phase)
We will update the unit tests for the VCL parser to expect the new, raw path emission.

### 3.1. Parser Unit Tests
Modify the unit test in `src/pipeline/parser/languages/varnish/vcl.rs`:

```rust
#[test]
fn test_extract_include() {
    let entities = extract_entities_vcl("include \"/etc/varnish/foo.vcl\";\n", "test.vcl", "test-repo");
    assert!(entities.iter().any(|e| {
        e.reference_intents
            .iter()
            .any(|r| matches!(r, ReferenceIntent::VclInclude { path, .. } if path == "/etc/varnish/foo.vcl"))
    }));
}
```
*Note: The path should now assert against the raw string `"/etc/varnish/foo.vcl"` instead of the prefixed FQN `vcl:test-repo:/etc/varnish/foo.vcl`.*

*Executing `cargo test` at this point must **FAIL** (Red).*

---

## 4. Phase 3: Implementation (The "Green" Phase)
With tests in place and failing, we will implement the actual fix to make the tests pass.

### 4.1. Step 1: Update the Parser
In `src/pipeline/parser/languages/varnish/vcl.rs` inside the `parse_include` method:

**Change:**
```rust
entity.reference_intents.push(ReferenceIntent::VclInclude {
    path: format!("vcl:{}:{}", self.repo_name, p),
    line,
});
```
**To:**
```rust
entity.reference_intents.push(ReferenceIntent::VclInclude {
    path: p.clone(), // Store the raw path exactly as written in the VCL
    line,
});
```

### 4.2. Step 2: Update the Resolver
In `src/pipeline/ingest/resolve/mod.rs`, locate the `ReferenceIntent::VclInclude` match arm inside `resolve_reference_intents_with_context`. Implement a 3-step resolution strategy.

```rust
ReferenceIntent::VclInclude { path, .. } => {
    let mut resolved_uuid = None;

    // Remove any leading slash for absolute path evaluation against repo root
    let stripped_path = path.strip_prefix('/').unwrap_or(path);

    // Strategy 1: Treat as absolute from repo root
    let root_fqn = format!("vcl:{}:{}", entity.repo_name, stripped_path);
    if let Some(&uuid) = ctx.fqn_to_uuid.get(&root_fqn) {
        resolved_uuid = Some(uuid);
    } 
    
    // Strategy 2: Treat as relative to current file's directory
    if resolved_uuid.is_none() {
        let parent_dir = std::path::Path::new(&entity.file_path)
            .parent()
            .and_then(|p| p.to_str())
            .unwrap_or("");
            
        let relative_path = if parent_dir.is_empty() {
            path.to_string()
        } else {
            format!("{}/{}", parent_dir, stripped_path)
        };
        // Normalize paths (e.g. resolve ./ or ../ if necessary, or just rely on FQN match)
        let relative_fqn = format!("vcl:{}:{}", entity.repo_name, relative_path);
        if let Some(&uuid) = ctx.fqn_to_uuid.get(&relative_fqn) {
            resolved_uuid = Some(uuid);
        }
    }

    // Strategy 3: Fuzzy fallback by filename
    if resolved_uuid.is_none() {
        let file_name = std::path::Path::new(path)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(path);
            
        for (fqn, &uuid) in ctx.fqn_to_uuid.iter() {
            if fqn.starts_with(&format!("vcl:{}", entity.repo_name)) && fqn.ends_with(file_name) {
                resolved_uuid = Some(uuid);
                break;
            }
        }
    }

    (resolved_uuid, RelationshipType::Includes)
}
```

---

## 5. Phase 4: Refactor and Verification
1. Run `cargo test` to ensure the unit tests pass (Green).
2. Run `./tests/run_varnish_e2e.sh` to ensure the end-to-end integration tests pass and correctly emit the `INCLUDES` relationship for absolute paths (Green).
3. Run `cargo fmt -- --check` and `cargo clippy --all-targets -- -D warnings` to verify code quality standards.
4. Review the resolver logic to ensure it doesn't introduce severe performance regressions (the fuzzy loop is bounded by `ctx.fqn_to_uuid` size, which is acceptable but could be optimized using `uuid_to_file` if needed).
