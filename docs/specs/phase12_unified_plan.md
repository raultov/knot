# Phase 12 Unified Implementation Plan — v1.2.0

**Synthesized from**: Gemini plan + Opus plan (best of both)
**Date**: 2026-05-03
**Scope**: Cargo.toml parsing, Configuration files (.properties, JSON, YAML), Helm/K8s support, Cross-repo dependency linking

---

## Executive Summary

Release v1.2.0 introduces four major functional areas:

1. **Cargo.toml parsing** — Extends build system support to Rust's package manager with package metadata, features, workspace members, and multi-format dependency parsing.
2. **Configuration files** — Indexing of YAML (.yml/.yaml), JSON (.json), and Java Properties (.properties) with leaf-key granularity. Special handling for package.json (npm dependencies as `BuildDependency`).
3. **Helm charts + Kubernetes manifests** — Infrastructure-as-code indexing with granular K8s entity kinds and Helm template variable tracking.
4. **Cross-repo dependency linking** — Automatic inter-repository call resolution through a `:Repository` graph model with `DEPENDS_ON` edges, retroactive linking, and a new `knot deps` CLI subcommand + MCP tool.

The cross-repo linking feature is the headline capability: when `knot-indexer` discovers build dependencies (Maven, Gradle, Cargo, npm), it automatically checks Neo4j for locally indexed repos that match those dependencies and creates `DEPENDS_ON` relationships. This enables `find_callers` and `search_hybrid_context` to trace call chains across repository boundaries.

---

## Table of Contents

- [Phase A: Cargo.toml Parser](#phase-a--cargotoml-parser)
- [Phase B: Configuration Files (YAML, JSON, .properties)](#phase-b--configuration-files-yaml-json-properties)
- [Phase C: Helm Charts + Kubernetes Manifests](#phase-c--helm-charts--kubernetes-manifests)
- [Phase D: Cross-Repo Dependency Linking](#phase-d--cross-repo-dependency-linking)
- [Phase E: Models, Formatting, and Documentation](#phase-e--models-formatting-and-documentation)
- [Implementation Order](#implementation-order)
- [Risks and Mitigations](#risks-and-mitigations)
- [File Inventory](#file-inventory)
- [Success Criteria](#success-criteria)
- [Key Design Decisions](#key-design-decisions-merged-plan-rationale)

---

## Phase A — Cargo.toml Parser

### A1. Add `toml` dependency to `Cargo.toml`

```toml
# --- TOML Parsing (Cargo.toml) ---
toml = "0.8"
```

### A2. Create parser `src/pipeline/parser/languages/toml.rs`

**Entry point**: `extract_entities_toml(source: &str, file_path: &str, repo_name: &str) -> Vec<ParsedEntity>`

**Entities to extract**:

| TOML Section | EntityKind | Name format | FQN format |
|---|---|---|---|
| `[package]` | `CargoPackage` | `"crate_name"` | `"cargo:crate_name:version"` |
| `[dependencies]` | `BuildDependency` | `"crate_name:version"` | `"cargo:crate_name:version"` |
| `[dev-dependencies]` | `BuildDependency` | `"crate_name:version"` | `"cargo:crate_name:version"` |
| `[build-dependencies]` | `BuildDependency` | `"crate_name:version"` | `"cargo:crate_name:version"` |
| `[features]` | `CargoFeature` | `"feature_name"` | `"cargo:crate:feature:feature_name"` |
| `[workspace.members]` | `WorkspaceMember` | `"member_path"` | `"cargo:workspace:member_path"` |

**Complex version handling**: Cargo dependencies can be specified as:

- **Simple**: `serde = "1.0"` -> version = `"1.0"`
- **Table**: `serde = { version = "1.0", features = ["derive"] }` -> version + features
- **Git**: `my-lib = { git = "https://...", branch = "main" }` -> git URL as version
- **Path (local)**: `my-lib = { path = "../my-lib" }` -> flag for cross-repo linking (Phase D)

**Signature field**: For complex dependencies, include activated features: `"features: [derive, serde_json]"`. For path deps: `"path: ../my-lib"`.

**Docstring field**: `"Cargo dependency: crate_name:version"` (consistent with Maven/Gradle format).

**Scope mapping** (stored in signature, analogous to Maven scope):

| Cargo Section | Scope equivalent |
|---|---|
| `[dependencies]` | `"scope: compile"` |
| `[dev-dependencies]` | `"scope: dev"` |
| `[build-dependencies]` | `"scope: build"` |

**Project identity**: Additionally emit a `ProjectIdentity` entity from `[package]`:
- `name`: `"crate_name"` (the package name)
- `fqn`: `"cargo:crate_name"`
- `signature`: `"version: X, build_system: cargo"`

This marker entity is used by Phase D for cross-repo linking.

### A3. Register in dispatch and input

**In `src/pipeline/parser/mod.rs`**:
- Add match branch for `"toml"` -> `languages::toml::extract_entities_toml()`
- Only `Cargo.toml` files should be indexed, not arbitrary `.toml` files

**In `src/pipeline/input.rs`**:
- Add `"Cargo.toml"` to the `known_names` array (alongside `"Jenkinsfile"` and `"pom.xml"`)
- Do **NOT** add `"toml"` to `SUPPORTED_EXTENSIONS` — we only want `Cargo.toml` by filename match

**In `src/pipeline/parser/languages/mod.rs`**:
- Add `pub mod toml;`

### A4. New `EntityKind` variants

Add to `src/models/entity.rs`:

```rust
// Cargo (Rust build system)
CargoPackage,     // [package] metadata (name, version, edition)
CargoFeature,     // [features] flag definitions
WorkspaceMember,  // [workspace.members] entries
```

**Required updates across the codebase** (all new entity kinds in this plan follow this pattern):
- `EntityKind::Display` impl — add display strings: `"cargo_package"`, `"cargo_feature"`, `"workspace_member"`
- `kind_to_label()` in `src/db/graph/utils.rs` — add Neo4j label mappings
- `format_file_entities()` in `src/cli_tools/explore_file.rs` — add formatting sections

### A5. Unit tests

Minimum 8 tests in `toml.rs`:

1. `test_extract_single_simple_dependency` — `serde = "1.0"`
2. `test_extract_table_dependency_with_features` — `serde = { version = "1.0", features = ["derive"] }`
3. `test_extract_git_dependency` — `my-lib = { git = "https://..." }`
4. `test_extract_path_dependency` — `my-lib = { path = "../my-lib" }`
5. `test_extract_dev_and_build_dependencies` — `[dev-dependencies]` + `[build-dependencies]`
6. `test_extract_package_metadata` — `[package]` name, version, edition
7. `test_extract_features_section` — `[features] default = ["std", "derive"]`
8. `test_extract_workspace_members` — `[workspace] members = ["crate-a", "crate-b"]`

### A6. Test fixture

Create `tests/testing_files/sample_Cargo.toml` — a realistic workspace Cargo.toml with:
- `[package]` with name, version, edition, description
- 5+ `[dependencies]` (mix of simple, table, git, path)
- 2+ `[dev-dependencies]`
- 1+ `[build-dependencies]`
- `[features]` with 3+ features
- `[workspace]` with 2+ members

### A7. E2E tests

Extend `tests/run_build_systems_e2e.sh` with 4-6 new Cargo.toml validation tests:

1. **Search for Cargo dependency**: `knot search "serde"` should find `BuildDependency` entity
2. **Explore Cargo.toml**: `knot explore "Cargo.toml"` should list deps/features/workspace
3. **Search for Cargo feature**: `knot search "derive"` should find `CargoFeature`
4. **Search for workspace member**: `knot search "crate-a"` should find `WorkspaceMember`
5. **Search for package metadata**: `knot search "my-crate"` should find `CargoPackage`
6. **Search by scope**: `knot search "dev"` should find dev-dependencies

---

## Phase B — Configuration Files (YAML, JSON, .properties)

### B1. Add `serde_yaml` dependency

Add to `Cargo.toml`:

```toml
# --- YAML Parsing (Config, Helm, K8s) ---
serde_yaml = "0.9"
```

**Note**: `serde_json` is already a dependency in the project.

### B2. New `EntityKind` variant for configuration

Add to `src/models/entity.rs`:

```rust
// Configuration entities
ConfigProperty,   // key=value leaf in .properties, YAML, or JSON config files
```

A single variant is sufficient. The `language` field on `ParsedEntity` distinguishes the source format (`"yaml"`, `"json"`, `"properties"`).

### B3. Create parser `src/pipeline/parser/languages/yaml.rs`

**Entry point**: `extract_entities_yaml(source: &str, file_path: &str, repo_name: &str) -> Vec<ParsedEntity>`

**Parsing strategy**: Use `serde_yaml::Value` for generic YAML deserialization, then recursively walk the value tree:

```rust
fn walk_yaml(prefix: &str, value: &Value, entities: &mut Vec<ParsedEntity>, ...) {
    match value {
        Value::Mapping(map) => {
            for (key, val) in map {
                let key_str = key.as_str().unwrap_or("?");
                let new_prefix = if prefix.is_empty() { key_str } else { &format!("{prefix}.{key_str}") };
                walk_yaml(new_prefix, val, entities, ...);
            }
        }
        Value::Sequence(seq) => {
            for (i, item) in seq.iter().enumerate() {
                walk_yaml(&format!("{prefix}[{i}]"), item, entities, ...);
            }
        }
        _ => {
            // Leaf value — create ConfigProperty entity
            entities.push(make_config_entity(prefix, value, ...));
        }
    }
}
```

**Entity field mapping for each leaf key**:

| ParsedEntity field | Value |
|---|---|
| `name` | Last segment of the path (e.g., `"url"`) |
| `fqn` | `"<repo_name>:<file_path>:<full_dot_path>"` (prevents collisions between `application-dev.yml` and `application-prod.yml`) |
| `signature` | Literal value truncated to 200 chars (e.g., `"jdbc:mysql://localhost:3306/db"`) |
| `docstring` | `"Config property: spring.datasource.url = jdbc:mysql://..."` |
| `embed_text` | Full dot-path + value for semantic search |
| `kind` | `ConfigProperty` |
| `language` | `"yaml"` |

**Depth limit**: Maximum 10 nesting levels to prevent pathological YAML files.

**Multi-document YAML**: Handle `---` separated documents by splitting on `\n---` and processing each independently.

**Important**: This function must NOT be called for YAML files that are K8s manifests or Helm charts. The dispatch logic in `parser/mod.rs` handles routing (see Phase C3).

### B4. Create parser `src/pipeline/parser/languages/json_config.rs`

**Entry point**: `extract_entities_json_config(source: &str, file_path: &str, repo_name: &str) -> Vec<ParsedEntity>`

Same recursive walking logic as YAML but using `serde_json::Value`.

**Special case — `package.json`**: Dependencies in package.json must generate `BuildDependency` entities (analogous to Maven/Gradle), not `ConfigProperty`:

| package.json field | EntityKind | Name format |
|---|---|---|
| `dependencies.X` | `BuildDependency` | `"npm:X:version"` |
| `devDependencies.X` | `BuildDependency` | `"npm:X:version"` |
| `peerDependencies.X` | `BuildDependency` | `"npm:X:version"` |
| `scripts.X` | `ConfigProperty` | `"scripts.X"` |
| Other fields | `ConfigProperty` | dot-separated path |

**Detection**: Check if the root JSON object has a `"name"` field and any of `"dependencies"`, `"devDependencies"`, `"peerDependencies"` — if so, treat as package.json.

**Project identity for npm**: When a package.json is detected, also emit a `ProjectIdentity` entity:
- `name`: package name (e.g., `"@scope/my-app"` or `"my-app"`)
- `fqn`: `"npm:package_name"`
- `signature`: `"version: X, build_system: npm"`

### B5. Create parser `src/pipeline/parser/languages/properties.rs`

**Entry point**: `extract_entities_properties(source: &str, file_path: &str, repo_name: &str) -> Vec<ParsedEntity>`

Line-by-line parser (no tree-sitter needed). Java `.properties` format:

- `key=value` or `key: value` or `key value` (first delimiter wins)
- Comments: lines starting with `#` or `!`
- Line continuation: trailing `\` joins with next line
- Heuristic: if the line(s) immediately preceding a property are comments, extract them as the `docstring`

Each non-comment, non-empty line produces a `ConfigProperty` entity:

| Field | Value |
|---|---|
| `name` | Last dot-segment of key (e.g., `"url"` from `"spring.datasource.url"`) |
| `fqn` | `"<repo_name>:<file_path>:<full_key>"` |
| `signature` | Value string (truncated to 200 chars) |
| `docstring` | Preceding comment lines, if any |
| `language` | `"properties"` |

### B6. Register in input and dispatch

**In `src/pipeline/input.rs`**:

Add to `SUPPORTED_EXTENSIONS`:
```rust
"yml", "yaml", "json", "properties"
```

Add exclusion logic to `discover_files()`:
```rust
let excluded_names = [
    "package-lock.json", "yarn.lock", "pnpm-lock.yaml",
    "Cargo.lock", "composer.lock", "Gemfile.lock",
    "poetry.lock", "Pipfile.lock",
];
```

Add file size limit check — skip files > 500KB to prevent indexing data dumps or generated files.

Update `known_names`:
```rust
let known_names = ["Jenkinsfile", "pom.xml", "Cargo.toml", "package.json"];
```

**In `src/pipeline/parser/mod.rs`**:

```rust
"yml" | "yaml" => dispatch_yaml(source, file_path, repo_name),  // See Phase C3
"json" => languages::json_config::extract_entities_json_config(source, file_path, repo_name),
"properties" => languages::properties::extract_entities_properties(source, file_path, repo_name),
```

**In `src/pipeline/parser/languages/mod.rs`**:
```rust
pub mod yaml;
pub mod json_config;
pub mod properties;
```

### B7. Unit tests

**YAML** (in `yaml.rs`):
1. `test_parse_flat_keys` — `key: value` at root level
2. `test_parse_nested_keys` — `spring.datasource.url` style nesting
3. `test_parse_array_values` — `servers: [a, b, c]`
4. `test_parse_depth_limit` — deeply nested YAML stops at depth 10
5. `test_parse_multi_document_yaml` — `---` separated documents
6. `test_empty_yaml` — empty or comment-only file

**JSON** (in `json_config.rs`):
1. `test_parse_simple_config` — flat JSON config
2. `test_parse_nested_config` — nested objects
3. `test_parse_package_json_dependencies` — `dependencies` extracted as `BuildDependency`
4. `test_parse_package_json_project_identity` — `ProjectIdentity` emitted for package.json
5. `test_parse_tsconfig` — compiler options as `ConfigProperty`
6. `test_parse_empty_json` — empty object

**Properties** (in `properties.rs`):
1. `test_parse_simple_pairs` — `key=value`
2. `test_parse_colon_delimiter` — `key: value`
3. `test_parse_comments` — `# comment` and `! comment` lines skipped
4. `test_parse_comment_as_docstring` — comments preceding a property become its docstring
5. `test_parse_multiline_values` — trailing `\` continuation
6. `test_parse_empty_file` — no properties

### B8. Test fixtures and E2E

**New test fixtures**:
- `tests/testing_files/sample_application.yml` — Spring Boot style config (20+ properties)
- `tests/testing_files/sample_config.json` — generic config with nested objects
- `tests/testing_files/sample_app.properties` — Java properties file (10+ entries)
- `tests/testing_files/sample_package.json` — Node.js package with dependencies, devDependencies, scripts

**New E2E script**: `tests/run_config_e2e.sh` (6-8 tests):
1. Search for YAML property: `knot search "datasource url"` should find `ConfigProperty`
2. Explore `application.yml`: should list all config properties
3. Search for JSON config property: `knot search "compilerOptions"` in tsconfig
4. Search for npm dependency in package.json: `knot search "express"`
5. Explore `sample_app.properties`: should list all properties
6. Explore `package.json`: should show dependencies + scripts

---

## Phase C — Helm Charts + Kubernetes Manifests

### C1. Create parser `src/pipeline/parser/languages/kubernetes.rs`

**Entry point**: `extract_entities_k8s(source: &str, file_path: &str, repo_name: &str) -> Vec<ParsedEntity>`

Uses `serde_yaml::Value` to parse, then analyzes the K8s-specific structure.

**Detection criteria**: A YAML file is a K8s manifest if it has both `apiVersion` and `kind` fields at the root level.

**New `EntityKind` variants**:

```rust
// Kubernetes entities
K8sDeployment,    // kind: Deployment
K8sService,       // kind: Service
K8sConfigMap,     // kind: ConfigMap
K8sSecret,        // kind: Secret
K8sIngress,       // kind: Ingress
K8sNamespace,     // kind: Namespace
K8sResource,      // Catch-all for other kinds (DaemonSet, StatefulSet, Job, CronJob, etc.)
```

**Kind-to-EntityKind mapping**:

| K8s `kind` value | EntityKind |
|---|---|
| `"Deployment"` | `K8sDeployment` |
| `"Service"` | `K8sService` |
| `"ConfigMap"` | `K8sConfigMap` |
| `"Secret"` | `K8sSecret` |
| `"Ingress"` | `K8sIngress` |
| `"Namespace"` | `K8sNamespace` |
| Everything else | `K8sResource` |

**Entity field mapping**:

| K8s field | ParsedEntity field |
|---|---|
| `kind` | Determines the `EntityKind` variant |
| `metadata.name` | `name` (formatted as `"kind metadata.name"`, e.g., `"Deployment my-backend"`) |
| `metadata.namespace` (default: `"default"`) | Part of `fqn`: `"k8s:namespace/kind/name"` |
| `apiVersion` | Included in `signature`: `"apiVersion: apps/v1"` |
| `metadata.labels` | Appended to `signature` (serialized key=value pairs) |
| `spec` (summary: image, replicas, ports) | `docstring` |
| `metadata.annotations` | `inline_comments` |

**References extracted**:

| Source | ReferenceIntent | Target |
|---|---|---|
| `spec.selector.matchLabels` | `ValueReference` | Pods matched by labels |
| `spec.template.spec.containers[].image` | `ValueReference` | Container image reference |
| `spec.rules[].backend.service.name` (Ingress) | `ValueReference` | Service name reference |
| `spec.template.spec.volumes[].configMap.name` | `ValueReference` | ConfigMap reference |
| `spec.template.spec.volumes[].secret.secretName` | `ValueReference` | Secret reference |

**Multi-resource YAML**: K8s manifests often contain multiple resources separated by `---`. The parser must split on document boundaries and process each independently:

```rust
for doc in source.split("\n---") {
    if let Ok(yaml) = serde_yaml::from_str::<Value>(doc.trim()) {
        if has_api_version_and_kind(&yaml) {
            entities.extend(extract_single_k8s_resource(&yaml, file_path, repo_name));
        }
    }
}
```

### C2. Create parser `src/pipeline/parser/languages/helm.rs`

**Entry point**: Multiple functions for different Helm file types.

**New `EntityKind` variants**:

```rust
// Helm entities
HelmChart,          // Chart.yaml metadata
HelmValue,          // values.yaml key-value pairs
HelmTemplateVar,    // {{ .Values.x }} template variable usage
```

#### C2a. `Chart.yaml` parser

**Function**: `extract_chart_yaml(source: &str, file_path: &str, repo_name: &str) -> Vec<ParsedEntity>`

| Chart.yaml field | Entity |
|---|---|
| Root chart metadata | `HelmChart` entity (name, version, description, appVersion) |
| `dependencies[].name` | `BuildDependency` with name format `"helm:chart_name:version"` |

**FQN for chart**: `"helm:chart_name:version"`
**Signature**: `"appVersion: 1.16.0, type: application"`
**Docstring**: Chart description field.

#### C2b. `values.yaml` parser

**Function**: `extract_values_yaml(source: &str, file_path: &str, repo_name: &str, chart_name: &str) -> Vec<ParsedEntity>`

Same recursive walking strategy as the generic YAML config parser (Phase B3), but:
- Uses `HelmValue` instead of `ConfigProperty`
- FQN includes chart context: `"helm:values.chart_name.path.to.key"`
- Language set to `"helm"`

The `chart_name` parameter is derived from the sibling `Chart.yaml` or from the parent directory name.

#### C2c. Template parser

**Function**: `extract_helm_template(source: &str, file_path: &str, repo_name: &str, chart_name: &str) -> Vec<ParsedEntity>`

Helm templates use Go template syntax (`{{ }}`) mixed with YAML. Since these are not valid YAML, parsing requires regex-based extraction:

1. **Regex patterns** for template variables:
   ```regex
   \{\{\s*\.Values\.([a-zA-Z0-9_.]+)\s*\}\}
   \{\{\s*\.Release\.([a-zA-Z0-9_]+)\s*\}\}
   \{\{\s*include\s+"([^"]+)"\s*
   \{\{\s*\.Chart\.([a-zA-Z0-9_]+)\s*\}\}
   ```

2. Each unique `.Values.X` reference generates a `HelmTemplateVar` entity:
   - **name**: Last segment (e.g., `"replicaCount"`)
   - **fqn**: `"helm:template:chart_name.Values.replicaCount"`
   - **kind**: `HelmTemplateVar`
   - **language**: `"helm"`

3. Template variables generate `ReferenceIntent::ValueReference` pointing to `HelmValue` entities from the chart's `values.yaml`.

4. **No YAML stripping**: Do NOT attempt to parse templates as YAML after removing `{{ }}` blocks — this is fragile. Only extract template variable references.

**Deduplication**: If the same `.Values.X` appears multiple times in a template, only one `HelmTemplateVar` entity is created, but each occurrence generates a reference intent.

### C3. YAML dispatch function in `parser/mod.rs`

The `.yml`/`.yaml` extension handler needs a dispatch function:

```rust
fn dispatch_yaml(source: &str, file_path: &str, repo_name: &str) -> Vec<ParsedEntity> {
    let filename = Path::new(file_path)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("");

    // 1. Is it Chart.yaml?
    if filename == "Chart.yaml" {
        return helm::extract_chart_yaml(source, file_path, repo_name);
    }

    // 2. Is it inside a Helm chart directory?
    if is_in_helm_chart_dir(file_path) {
        if filename == "values.yaml" || filename == "values.yml" {
            let chart_name = detect_chart_name(file_path);
            return helm::extract_values_yaml(source, file_path, repo_name, &chart_name);
        }
        if is_in_templates_dir(file_path) {
            let chart_name = detect_chart_name(file_path);
            return helm::extract_helm_template(source, file_path, repo_name, &chart_name);
        }
    }

    // 3. Is it a K8s manifest? (has apiVersion + kind at root level)
    if let Ok(yaml) = serde_yaml::from_str::<serde_yaml::Value>(source) {
        if yaml.get("apiVersion").is_some() && yaml.get("kind").is_some() {
            return kubernetes::extract_entities_k8s(source, file_path, repo_name);
        }
    }

    // 4. Default: generic configuration YAML
    yaml::extract_entities_yaml(source, file_path, repo_name)
}
```

**Helper functions**:
- `is_in_helm_chart_dir(file_path)`: Walk parent directories looking for a sibling `Chart.yaml`
- `is_in_templates_dir(file_path)`: Check if any parent directory is named `templates`
- `detect_chart_name(file_path)`: Parse the nearest `Chart.yaml` to extract the chart name, or fall back to the chart directory name

### C4. Extensions and known filenames

**In `src/pipeline/input.rs`**:

Extensions (partially added in Phase B6):
- `"tpl"` — Helm template files (add to `SUPPORTED_EXTENSIONS`)

**In `src/pipeline/parser/mod.rs`**:
- `"tpl"` -> `helm::extract_helm_template(source, file_path, repo_name, &detect_chart_name(file_path))`

### C5. Unit tests

**Kubernetes** (in `kubernetes.rs`):
1. `test_parse_deployment` — standard Deployment with replicas, containers, labels
2. `test_parse_service` — ClusterIP Service with ports and selector
3. `test_parse_configmap` — ConfigMap with data entries
4. `test_parse_ingress` — Ingress with rules and backend services
5. `test_parse_multi_resource_yaml` — `---` separated multiple resources
6. `test_parse_unknown_kind` — CronJob -> `K8sResource` catch-all
7. `test_parse_references_between_resources` — Deployment referencing ConfigMap by name

**Helm** (in `helm.rs`):
1. `test_parse_chart_yaml` — chart metadata extraction
2. `test_parse_chart_dependencies` — chart dependencies as `BuildDependency`
3. `test_parse_values_yaml` — leaf keys as `HelmValue` entities
4. `test_parse_template_variables` — `{{ .Values.X }}` extraction
5. `test_parse_release_and_chart_references` — `{{ .Release.Name }}`, `{{ .Chart.Name }}`

**Auto-detection** (in `mod.rs` tests):
1. `test_dispatch_k8s_manifest` — YAML with apiVersion+kind routes to K8s parser
2. `test_dispatch_helm_chart_yaml` — Chart.yaml routes to Helm parser
3. `test_dispatch_generic_config` — plain YAML routes to config parser

### C6. Test fixtures and E2E

**New test fixtures**:
```
tests/testing_files/k8s/deployment.yaml         # Standard Deployment (nginx, 3 replicas)
tests/testing_files/k8s/service.yaml            # ClusterIP Service + NodePort
tests/testing_files/k8s/configmap.yaml          # K8s ConfigMap fixture
tests/testing_files/helm/Chart.yaml             # Chart with 2 dependencies
tests/testing_files/helm/values.yaml            # 15+ values across nested sections
tests/testing_files/helm/templates/deployment.yaml  # Template using {{ .Values.X }}
```

**New E2E script**: `tests/run_k8s_helm_e2e.sh` (8-10 tests):

1. Search for K8s Deployment: `knot search "nginx deployment"` finds `K8sDeployment`
2. Explore `deployment.yaml`: lists Deployment entity with labels and container info
3. Search for K8s Service: `knot search "backend service"` finds `K8sService`
4. Search for ConfigMap: `knot search "app-config"` finds `K8sConfigMap`
5. Explore `Chart.yaml`: lists `HelmChart` + chart `BuildDependency` entities
6. Search for Helm value: `knot search "replicaCount"` finds `HelmValue`
7. Explore `values.yaml`: lists all `HelmValue` leaf keys
8. Search for template variable: `knot search "Values.image.repository"` finds `HelmTemplateVar`
9. Find callers of Helm value: `knot callers "replicaCount"` finds template references
10. Multi-resource YAML: explore a file with 3+ K8s resources

---

## Phase D — Cross-Repo Dependency Linking

This is the headline feature of v1.2.0.

### D1. New graph model: `:Repository` node

Introduce a new node type in Neo4j (separate from `:Entity` nodes):

```cypher
(:Repository {
    name: "my-app",            // matches repo_name used in indexing
    build_system: "maven",     // "maven", "gradle", "cargo", "npm"
    group_id: "com.example",   // Maven groupId / npm scope / Cargo crate name
    artifact_id: "my-app",     // Maven artifactId / npm package name / Cargo crate name
    version: "1.0.0",          // Current version
    indexed_at: datetime()     // Last indexing timestamp
})
```

**New `RelationshipType`** in `src/models/relationship.rs`:

```rust
/// Repository-level dependency relationship.
DependsOn,  // (:Repository)-[:DEPENDS_ON]->(:Repository)
```

Update `Display` impl to map to `"DEPENDS_ON"`.

### D2. New `EntityKind`: `ProjectIdentity`

Add to `src/models/entity.rs`:

```rust
// Cross-repo linking
ProjectIdentity,  // Build file project declaration (Maven GAV, Cargo package, npm name)
```

The `ProjectIdentity` entity is a **marker entity** emitted by build file parsers. It carries:
- `name`: `"groupId:artifactId"` (Maven) or `"crate_name"` (Cargo) or `"package_name"` (npm)
- `fqn`: Full identity string (e.g., `"maven:com.example:my-app"`, `"cargo:my-crate"`, `"npm:@scope/my-app"`)
- `signature`: `"version: X, build_system: maven"`

The ingest stage detects these entities and creates/updates `:Repository` nodes.

### D3. Project identity extraction from build files

Each build file parser must extract the **project identity** in addition to dependencies:

| Build System | Identity source | group_id | artifact_id |
|---|---|---|---|
| Maven | `<project>` in pom.xml | `<groupId>` | `<artifactId>` |
| Gradle | `build.gradle` root | `group = 'com.example'` | `rootProject.name` or project name |
| Cargo | `[package]` in Cargo.toml | crate `name` | crate `name` |
| npm | `package.json` root | `@scope` (if scoped) | `name` |

**Implementation**:

1. **Extend `xml.rs`** (Maven): Add extraction of `<groupId>` + `<artifactId>` + `<version>` from the root `<project>` element. Emit a `ProjectIdentity` entity.
2. **Extend `groovy.rs`** (Gradle): Add extraction of `group = '...'` and project name declarations. Emit a `ProjectIdentity` entity.
3. **In `toml.rs`** (Cargo): Already emitting `ProjectIdentity` from `[package]` (Phase A2).
4. **In `json_config.rs`** (npm): Already emitting `ProjectIdentity` from package.json (Phase B4).

### D4. Automatic discovery in Neo4j

When `knot-indexer` parses a build file and extracts dependencies, the ingest stage performs automatic cross-repo linking:

#### Step 1: Create/Update `:Repository` node

After ingesting a `ProjectIdentity` entity, upsert the `:Repository` node:

```cypher
MERGE (r:Repository {name: $repo_name})
SET r.build_system = $build_system,
    r.group_id = $group_id,
    r.artifact_id = $artifact_id,
    r.version = $version,
    r.indexed_at = datetime()
```

#### Step 2: Match dependencies against indexed repos

For each `BuildDependency` entity discovered during parsing:

```cypher
// Maven/Gradle: match by groupId:artifactId
MATCH (r:Repository)
WHERE r.group_id = $dep_group_id AND r.artifact_id = $dep_artifact_id
RETURN r.name AS matched_repo

// Cargo: match by crate name
MATCH (r:Repository)
WHERE r.artifact_id = $dep_crate_name AND r.build_system = 'cargo'
RETURN r.name AS matched_repo

// npm: match by package name
MATCH (r:Repository)
WHERE r.artifact_id = $dep_package_name AND r.build_system = 'npm'
RETURN r.name AS matched_repo
```

If a match is found:

```cypher
MATCH (from:Repository {name: $current_repo})
MATCH (to:Repository {name: $matched_repo})
MERGE (from)-[:DEPENDS_ON]->(to)
```

Add `$matched_repo` to the resolution repos list.

#### Step 3: Retroactive linking

When a library is indexed **after** its clients were already indexed:

```cypher
// Find all repos that have a BuildDependency entity matching this repo's identity
MATCH (d:Entity)
WHERE d.kind = 'build_dependency'
  AND d.name CONTAINS $this_artifact_id
  AND d.repo_name <> $this_repo_name
RETURN DISTINCT d.repo_name AS client_repo
```

For each `client_repo`, create the `DEPENDS_ON` edge. Log a message suggesting re-indexing the client repo with `--clean` for full cross-repo call resolution.

**v1.2.0 scope**: Retroactive linking creates `DEPENDS_ON` edges and logs suggestions. Full incremental re-resolution deferred to v1.3.0.

### D5. Cross-repo call resolution

**Extend `src/pipeline/ingest/resolve.rs`**:

The existing resolve pipeline already supports cross-repo resolution via `cfg.dependency_repos`. Changes needed:

1. **Auto-populate `dependency_repos`**: At the start of `resolve_and_save_relationships()`, query Neo4j:

```rust
// Query transitive dependencies up to 3 levels deep
let auto_deps = graph_db.find_repo_dependencies(&cfg.repo_name, 3).await?;

let mut repos_to_load = vec![cfg.repo_name.clone()];
repos_to_load.extend(cfg.dependency_repos.iter().cloned());  // manual overrides
repos_to_load.extend(auto_deps);  // auto-discovered
repos_to_load.sort();
repos_to_load.dedup();
```

2. **No changes to resolution logic itself**: The existing `resolve_reference_intents_with_context()` already resolves by name/FQN across all loaded repos.

3. **Logging**: Add `info!` logs when cross-repo calls are resolved.

The `--dependencies` / `KNOT_DEPENDENCIES` flag becomes a **manual supplement** to auto-discovery, still useful for:
- Forcing a dependency before the library is indexed
- Linking repos without matching build file identities
- Testing and debugging

### D6. New `GraphDb` methods

**In `src/db/graph/connection.rs`**:

```rust
/// Create indexes for :Repository nodes
async fn ensure_repository_indexes(&self) -> Result<()> {
    // CREATE CONSTRAINT repo_name_unique FOR (r:Repository) REQUIRE r.name IS UNIQUE
    // CREATE INDEX repo_artifact FOR (r:Repository) ON (r.group_id, r.artifact_id)
}
```

Call from existing `ensure_indexes()`.

**In `src/db/graph/upsert.rs`**:

```rust
/// Create or update a Repository node with project identity metadata
async fn upsert_repository(&self, repo: &RepositoryNode) -> Result<()>

/// Create a DEPENDS_ON relationship between two repositories
async fn upsert_repo_dependency(&self, from_repo: &str, to_repo: &str) -> Result<()>
```

**In `src/db/graph/query.rs`**:

```rust
/// Find all repositories that this repo depends on (transitive, up to max_depth)
async fn find_repo_dependencies(&self, repo_name: &str, max_depth: u32) -> Result<Vec<String>>

/// Find all repositories that depend on this repo (reverse lookup)
async fn find_repo_dependents(&self, repo_name: &str) -> Result<Vec<String>>

/// Find a repository by its Maven/Cargo/npm artifact identity
async fn find_repository_by_artifact(
    &self, group_id: &str, artifact_id: &str
) -> Result<Option<String>>
```

**In `src/db/graph/delete.rs`**:

```rust
/// Delete a Repository node and its DEPENDS_ON edges when a repo is cleaned
async fn delete_repository(&self, repo_name: &str) -> Result<()>
```

### D7. Extend `find_callers` for cross-repo output

**In `src/db/graph/query.rs`**: Update `find_references()` Cypher queries to allow cross-repo traversal. Current queries filter by `repo_name` — relax this when the target entity has known dependency repos.

**In `src/cli_tools/find_callers.rs`**: After formatting results, check if any caller's `repo_name` differs from the target's. If so, group those callers under a "Cross-Repository Callers" section:

```markdown
## Cross-Repository Callers

### From `client-app` (via DEPENDS_ON)
- **`UserService.login`** (line 42)
  - File: `src/main/java/com/example/UserService.java:42`
  - Relationship: CALLS
```

### D8. New CLI subcommand: `knot deps`

Add to `src/bin/knot.rs`:

```rust
/// Show dependency graph for a repository
Deps {
    /// Repository name to show dependencies for
    repo_name: String,
    /// Maximum depth for transitive dependencies (default: 3)
    #[arg(short, long, default_value = "3")]
    depth: u32,
    /// Show reverse dependencies (who depends on this repo)
    #[arg(long)]
    reverse: bool,
    /// Output format
    #[arg(short, long, value_enum, default_value_t = OutputFormat::Table)]
    output: OutputFormat,
}
```

**Output format** (tree view):

```
$ knot deps my-app
my-app
+-- auth-lib (indexed, 142 entities)
+-- common-utils (indexed, 89 entities)
|   +-- logging-core (indexed, 34 entities)
+-- spring-boot-starter-web (not indexed)

$ knot deps --reverse auth-lib
Repositories that depend on auth-lib:
+-- my-app (indexed, 1203 entities)
+-- admin-portal (indexed, 567 entities)
```

**Implementation**: New file `src/cli_tools/deps.rs`.

### D9. New MCP tool: `list_repo_dependencies`

Expose dependency graph info via MCP for AI agent consumption:

```json
{
  "name": "list_repo_dependencies",
  "description": "Show the dependency graph for a repository, including which dependencies are locally indexed",
  "inputSchema": {
    "type": "object",
    "properties": {
      "repo_name": { "type": "string", "description": "Repository name" },
      "max_depth": { "type": "integer", "default": 3 },
      "reverse": { "type": "boolean", "default": false }
    },
    "required": ["repo_name"]
  }
}
```

### D10. Unit tests

1. `test_upsert_repository_node` — create and update `:Repository` node
2. `test_upsert_repo_dependency` — create `DEPENDS_ON` edge
3. `test_find_repo_dependencies_direct` — find direct dependencies
4. `test_find_repo_dependencies_transitive` — find 2-3 level transitive deps
5. `test_find_repo_dependents` — reverse dependency lookup
6. `test_find_repository_by_artifact_maven` — match by groupId+artifactId
7. `test_find_repository_by_artifact_cargo` — match by crate name
8. `test_retroactive_linking` — index library after client, verify DEPENDS_ON created
9. `test_cross_repo_call_resolution` — verify calls resolve across repo boundaries
10. `test_delete_repository` — clean up repo node and edges

### D11. E2E tests

**New script**: `tests/run_cross_repo_dep_e2e.sh` (8+ tests):

**Setup**: Create two temporary repos:
- `lib-repo`: Contains `AuthService.java` with `login()` and `logout()` methods, plus `pom.xml` declaring `com.example:auth-lib:1.0.0`
- `client-repo`: Contains `UserController.java` calling `AuthService.login()`, plus `pom.xml` with dependency on `com.example:auth-lib:1.0.0`

**Tests**:

1. **Index lib first, then client**: Verify `find_callers AuthService.login` returns callers from `client-repo`
2. **Verify DEPENDS_ON edge**: `knot deps client-repo` shows `lib-repo`
3. **Reverse lookup**: `knot deps --reverse lib-repo` shows `client-repo`
4. **Index client first, then lib (retroactive)**: Verify DEPENDS_ON edge created retroactively
5. **Cross-repo search**: `knot search "authentication" --repo client-repo` includes results from `lib-repo`
6. **Cargo cross-repo**: Same pattern with Cargo.toml repos
7. **Transitive deps**: Three repos: `app -> middleware -> core`. Verify 2-level transitive resolution
8. **Manual override**: Use `--dependencies core` to force linking without build file match

---

## Phase E — Models, Formatting, and Documentation

### E1. Complete list of new `EntityKind` variants

```rust
// Cargo (Rust build system) — Phase A
CargoPackage,       // [package] metadata
CargoFeature,       // [features] flag definitions
WorkspaceMember,    // [workspace.members] entries

// Configuration — Phase B
ConfigProperty,     // key=value in .properties / YAML leaf / JSON leaf

// Kubernetes — Phase C
K8sDeployment,      // kind: Deployment
K8sService,         // kind: Service
K8sConfigMap,       // kind: ConfigMap
K8sSecret,          // kind: Secret
K8sIngress,         // kind: Ingress
K8sNamespace,       // kind: Namespace
K8sResource,        // Catch-all for other K8s kinds

// Helm — Phase C
HelmChart,          // Chart.yaml metadata
HelmValue,          // values.yaml key-value pairs
HelmTemplateVar,    // {{ .Values.x }} template variable usage

// Cross-repo — Phase D
ProjectIdentity,    // Build file project declaration
```

**Total new variants**: 16

### E2. New `RelationshipType` variant

```rust
DependsOn,  // Repository -> Repository dependency edge
```

### E3. Update `explore_file.rs` formatting

Add new formatting sections in `format_file_entities()`:

| EntityKind group | Section header |
|---|---|
| `CargoPackage` | `## Cargo Package` |
| `CargoFeature` | `## Cargo Features` |
| `WorkspaceMember` | `## Workspace Members` |
| `ConfigProperty` | `## Configuration Properties` |
| `K8sDeployment`, `K8sService`, `K8sConfigMap`, `K8sSecret`, `K8sIngress`, `K8sNamespace`, `K8sResource` | `## Kubernetes Resources` |
| `HelmChart` | `## Helm Chart` |
| `HelmValue` | `## Helm Values` |
| `HelmTemplateVar` | `## Template Variables` |
| `ProjectIdentity` | (hidden — not shown in explore output) |

### E4. Update `kind_to_label()` in `db/graph/utils.rs`

Add Neo4j label mappings for all 16 new variants. This is an exhaustive match — the compiler enforces completeness.

### E5. Update `README.md`

1. Update description to mention YAML/Helm/K8s/Cargo.toml support
2. Add new sections: Cargo.toml, Configuration Files, Kubernetes, Helm Charts
3. Add "Cross-Repository Dependency Linking" section with examples
4. Document `knot deps` subcommand
5. Move Phase 12 from "Upcoming" to "Current Release (v1.2.0)"
6. Update entity kind count and relationship type count

### E6. Update `AGENTS.md`

1. Add new parsers to language table
2. Document all new modules
3. Add `knot deps` to Quick Commands
4. Update entity kinds section
5. Document cross-repo linking workflow

---

## Implementation Order

| Step | Phase | Description | Dependencies | Estimate |
|---|---|---|---|---|
| 1 | E1-E2 | Add all new EntityKind + RelationshipType variants, Display, kind_to_label | None | 0.5 days |
| 2 | A1-A6 | Cargo.toml parser + unit tests + fixture | Step 1 | 1 day |
| 3 | B1-B5 | YAML/JSON/.properties parsers + unit tests | Step 1 (parallelizable with Step 2) | 1.5 days |
| 4 | C1-C2 | K8s + Helm parsers + unit tests | Step 1 + B1 (needs serde_yaml) | 1.5 days |
| 5 | B6-B8, C3-C4 | Input discovery, dispatch, exclusions, file size limit | Steps 2-4 | 1 day |
| 6 | E3-E4 | explore_file formatting + kind_to_label updates | Step 1 | 0.5 days |
| 7 | A7, C6 | E2E tests for Cargo + K8s/Helm + Config | Steps 2-6 | 1 day |
| 8 | D1-D6 | Cross-repo: Repository nodes, GraphDb methods, auto-discovery, resolve changes | Steps 1-7 completed | 2 days |
| 9 | D7-D9 | Cross-repo: find_callers output, `knot deps` CLI, MCP tool | Step 8 | 1 day |
| 10 | D10-D11 | Cross-repo unit + E2E tests | Step 9 | 1 day |
| 11 | E5-E6 | README + AGENTS.md documentation | All completed | 0.5 days |
| 12 | -- | Validator: cargo fmt + clippy + test + E2E | All completed | 0.5 days |

**Total estimated**: ~11 working days

**Parallelization**: Steps 2 and 3 can be done simultaneously. Step 6 can overlap with steps 2-4.

---

## Risks and Mitigations

| Risk | Impact | Probability | Mitigation |
|---|---|---|---|
| Generic YAML indexing generates noise (thousands of config properties per repo) | High | Medium | 500KB file size limit, max depth 10, exclude lock files and generated files |
| K8s vs Helm vs config YAML auto-detection fails on edge cases | Medium | Medium | Conservative cascade: Helm (directory context) -> K8s (apiVersion+kind) -> config (fallback). Log warnings. |
| Cross-repo linking with ambiguous artifact names | Medium | Low | Match by full `groupId:artifactId` (not just artifactId). Warn in logs if multiple matches. |
| Performance degradation resolving across many dependency repos | Low | Low | Limit transitive depth to 3 levels. Lazy-load entity mappings only for matching repos. |
| Helm Go template parsing with complex syntax (conditionals, loops, pipes) | Medium | Medium | Regex-based extraction only. Complex expressions produce partial results rather than errors. |
| `package-lock.json` and similar large files accidentally indexed | High | High | Explicit exclusion list in `discover_files()` + file size limit check. |
| Retroactive linking requires re-indexing client repos for full call resolution | Medium | High | v1.2.0 creates DEPENDS_ON edges retroactively but logs suggestion to re-index. Full incremental re-resolution deferred to v1.3.0. |
| FQN collisions between config files (dev.yml vs prod.yml same keys) | Medium | Medium | FQN includes `repo_name:file_path` prefix to guarantee uniqueness. |

---

## File Inventory

### New Files to Create

```
src/pipeline/parser/languages/toml.rs              # Cargo.toml parser
src/pipeline/parser/languages/yaml.rs              # Generic YAML config parser
src/pipeline/parser/languages/json_config.rs       # JSON config parser (+ package.json)
src/pipeline/parser/languages/properties.rs        # Java .properties parser
src/pipeline/parser/languages/kubernetes.rs        # Kubernetes manifest parser
src/pipeline/parser/languages/helm.rs              # Helm chart parser (Chart.yaml, values.yaml, templates)
src/cli_tools/deps.rs                              # `knot deps` subcommand implementation

tests/testing_files/sample_Cargo.toml              # Cargo workspace fixture
tests/testing_files/sample_application.yml         # Spring Boot YAML config fixture
tests/testing_files/sample_config.json             # Generic JSON config fixture
tests/testing_files/sample_app.properties          # Java properties fixture
tests/testing_files/sample_package.json            # Node.js package.json fixture
tests/testing_files/k8s/deployment.yaml            # K8s Deployment fixture
tests/testing_files/k8s/service.yaml               # K8s Service fixture
tests/testing_files/k8s/configmap.yaml             # K8s ConfigMap fixture
tests/testing_files/helm/Chart.yaml                # Helm Chart metadata fixture
tests/testing_files/helm/values.yaml               # Helm values fixture
tests/testing_files/helm/templates/deployment.yaml # Helm template fixture

tests/run_config_e2e.sh                            # Config files E2E test suite
tests/run_k8s_helm_e2e.sh                          # K8s + Helm E2E test suite
tests/run_cross_repo_dep_e2e.sh                    # Cross-repo linking E2E test suite
```

### Existing Files to Modify

```
Cargo.toml                                         # New deps: toml = "0.8", serde_yaml = "0.9"

src/models/entity.rs                               # 16 new EntityKind variants + Display impl
src/models/relationship.rs                         # DependsOn RelationshipType + Display impl

src/pipeline/input.rs                              # New extensions, known_names, exclusions, size limit
src/pipeline/parser/mod.rs                         # Dispatch for toml/yaml/json/properties/tpl + dispatch_yaml()
src/pipeline/parser/languages/mod.rs               # pub mod for 6 new modules
src/pipeline/parser/languages/xml.rs               # Extract Maven ProjectIdentity (groupId/artifactId)
src/pipeline/parser/languages/groovy.rs            # Extract Gradle ProjectIdentity (group/name)
src/pipeline/ingest/resolve.rs                     # Auto-discovery of deps + cross-repo resolution

src/db/graph/connection.rs                         # Indexes for :Repository nodes
src/db/graph/upsert.rs                             # upsert_repository(), upsert_repo_dependency()
src/db/graph/query.rs                              # find_repo_dependencies(), find_repo_dependents(), find_repository_by_artifact()
src/db/graph/delete.rs                             # delete_repository()
src/db/graph/utils.rs                              # kind_to_label() for 16 new EntityKind variants

src/cli_tools/mod.rs                               # Re-export deps module
src/cli_tools/explore_file.rs                      # Formatting sections for new entity kinds
src/cli_tools/find_callers.rs                      # Cross-repo caller output formatting

src/bin/knot.rs                                    # New `deps` subcommand
src/mcp_handler.rs                                 # list_repo_dependencies MCP tool (or src/mcp_tools/)

tests/run_build_systems_e2e.sh                     # Extended with Cargo.toml tests
tests/run_all_e2e.sh                               # Include 3 new E2E scripts

README.md                                          # Updated features, roadmap, documentation
AGENTS.md                                          # Updated architecture, commands, entity kinds
```

---

## Key Design Decisions (Merged Plan Rationale)

### Adopted from Gemini plan:
- **FQN with `repo_name:file_path` for config files**: Prevents collisions between environment-specific configs (e.g., `application-dev.yml` vs `application-prod.yml` with identical key paths).
- **Comment-as-docstring heuristic for `.properties`**: Comments immediately preceding a property definition are extracted as docstring.
- **Name format for K8s entities**: `"kind metadata.name"` (e.g., `"Deployment my-backend"`) for natural search.
- **Explicit use of `ReferenceIntent::ValueReference`** for Helm template variable linking (already exists in the codebase).

### Adopted from Opus plan:
- **Granular K8s entity kinds** (7 variants): Enables targeted queries like "show all deployments" or "find all services".
- **`ProjectIdentity` marker entity**: Clean separation of "who am I" from "what do I depend on". Uses the existing `ParsedEntity` pipeline with no return type changes.
- **`:Repository` node type with `DEPENDS_ON` edges**: Proper graph model enables transitive dependency traversal, cleaner than querying Entity nodes.
- **package.json as build file**: npm dependencies become `BuildDependency` entities, enabling cross-repo linking for Node.js projects.
- **`knot deps` CLI + `list_repo_dependencies` MCP tool**: User-facing and AI-facing dependency graph visualization.
- **Retroactive linking**: Concrete implementation for out-of-order indexing (library indexed after client).
- **File exclusions + 500KB size limit**: Prevents noise from lock files and data dumps.
- **Depth limit (10) for YAML**: Prevents pathological files from generating thousands of entities.
- **Separate `HelmValue` vs `ConfigProperty`**: Helm values are semantically distinct from generic config properties.
- **`CargoPackage`, `CargoFeature`, `WorkspaceMember`**: Finer granularity than a single `BuildArtifact`, more searchable.
- **Risk matrix with mitigations**: 8 identified risks with concrete countermeasures.
- **Detailed file inventory**: Complete list of 20+ new files and 25+ modified files.

### Rejected from both plans:
- Opus's suggestion to strip `{{ }}` from Helm templates and parse as YAML — too fragile, Go template syntax mixed with YAML produces invalid YAML even after stripping.
- Gemini's single `K8sManifest` entity kind — insufficient for targeted K8s resource type searches.
- Gemini's `BuildArtifact` entity kind — replaced by the more descriptive `ProjectIdentity` which clearly separates "project identity" from "build dependency".
- Gemini's reference to `src/pipeline/files.rs` — file does not exist; the actual file is `src/pipeline/input.rs`.

---

## Success Criteria

The release is ready when:

1. `cargo fmt -- --check` passes
2. `cargo clippy --all-targets -- -D warnings` passes
3. All existing unit tests pass (`cargo test`)
4. All existing E2E tests pass (`./tests/run_all_e2e.sh`)
5. All new unit tests pass (target: 50+ new tests across 6 parser modules + DB operations)
6. All new E2E tests pass:
   - `./tests/run_config_e2e.sh` (6-8 tests)
   - `./tests/run_k8s_helm_e2e.sh` (8-10 tests)
   - `./tests/run_cross_repo_dep_e2e.sh` (8+ tests)
   - Extended `./tests/run_build_systems_e2e.sh` (+4-6 tests)
7. Cross-repo call resolution works bidirectionally (library indexed before and after client)
8. `knot deps` CLI subcommand works for forward and reverse lookups
9. `list_repo_dependencies` MCP tool responds correctly
10. README.md and AGENTS.md are updated
11. No performance regression on existing language indexing
