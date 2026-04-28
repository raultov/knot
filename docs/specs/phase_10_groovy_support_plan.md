# Phase 10: Full Groovy Language Support (v0.10.5)

## Objective
Enable `knot` to parse and semantically understand standard Groovy language files (beyond build scripts). This will allow the MCP to answer questions about architecture, call flows, and perform hybrid searches on Groovy code, integrating `tree-sitter-groovy` into the Rust pipeline.

---

## Phase 10.1: Tree-sitter Dependencies
Since the official `tree-sitter-groovy` crate on `crates.io` might be outdated regarding tree-sitter `0.26`, we will resolve the dependency appropriately (e.g., using a git dependency or a compatible fork like `tree-sitter-groovy-sqry`).

1. **Cargo.toml:** Add the dependency pointing to a compatible version.
2. **Build System:** Ensure the C/C++ compiler builds the Groovy grammar correctly.

## Phase 10.2: Entity Modeling (`src/models/entity.rs`)
Groovy has Java-like constructs but includes its own features (Traits, loose Scripts). We will expand the `EntityKind` enum.

1. **New `EntityKind` variants:** `GroovyClass`, `GroovyInterface`, `GroovyTrait`, `GroovyMethod`, `GroovyFunction` (for script-level methods), `GroovyEnum`, `GroovyProperty` (fields/variables).
2. **Trait Adaptation:** Update the `Display` trait to format these entities.
3. **Neo4j Mapping:** Update `kind_to_label` in `src/db/graph/utils.rs` so the graph engine assigns correct Labels (e.g., `GroovyClass` -> `["Entity", "Class", "GroovyClass"]`).

## Phase 10.3: Syntax Tree Queries (`queries/groovy.scm`)
Create a new file `src/pipeline/parser/queries/groovy.scm` with S-Expression rules to capture Groovy AST nodes.

1. **Main Declarations:** Classes (`class_declaration`), Interfaces (`interface_declaration`), Traits (`trait_declaration`), Enums (`enum_declaration`).
2. **Methods and Functions:** Methods inside classes (`method_declaration`), top-level methods in scripts, and closures.
3. **Metadata:** Extraction of decorators/annotations and docstrings (`GroovyDoc` or multiline comments).

## Phase 10.4: Extraction Logic (`src/pipeline/parser/languages/groovy.rs`)
Currently, this module processes Gradle and Jenkins line-by-line. We will implement a hybrid strategy.

1. **Dispatcher Refactor (`mod.rs` and `groovy.rs`):**
   - If the file is `build.gradle` or `Jenkinsfile`, use the current line-by-line logic.
   - If it is a generic `.groovy` file, inject the code into the generic `extractor::extract_entities(source, tree_sitter_groovy::language().into(), query_src, ...)`.
2. **Tree-sitter Captures Mapping:** Process captures from the `.scm` file to populate `ParsedEntity` structures with correct start/end lines, signatures, and names.

## Phase 10.5: Context and FQN (`src/pipeline/parser/context.rs`)
Groovy uses packages like Java, but more flexibly.

1. **Package Deduction:** Read the `package com.mydomain.app` instruction.
2. **FQN Construction:** Ensure `compute_fqn_and_context` generates correct FQNs: `com.mydomain.app.MyClass.myMethod`.
3. **Hierarchies (`parent_name`):** Associate methods to their containing class for graph relations `(Method)-[:BELONGS_TO]->(Class)`.

## Phase 10.6: Semantic Enrichment (`src/pipeline/prepare.rs`)
Ensure the embedding text generated is highly optimized for vector search.

1. **Update `build_embed_text()`:** Add match arms for new Groovy kinds. Format: `[groovy_method] myMethod in MyClass. Signature: def myMethod(String arg)`. Include decorators and docstrings.

## Phase 10.7: CLI and MCP Presentation (`explore_file.rs` and `format.rs`)
1. **`src/cli_tools/explore_file.rs`:** Add Markdown sections for results: `## Classes (Groovy)`, `## Traits (Groovy)`, `## Methods (Groovy)`.
2. **`search_hybrid_context/format.rs`:** Ensure semantic results return the entity type formatted correctly to the LLM.

## Phase 10.8: Exhaustive Unit Tests
Create at least 25 unit tests.

1. **Test File:** Create `tests/testing_files/sample_full.groovy` with a mix of Groovy syntax (Traits, Classes, loose Closures, Annotations, Inheritance).
2. **Tests in `groovy.rs` or `extractor.rs`:**
   - Method signature extraction (`def` vs typed).
   - Traits extraction.
   - Static methods extraction.
   - FQN parsing with package declaration.
   - Resilience (ignoring invalid code).

## Phase 10.9: E2E Suite (End-to-End)
Groovy will have its own E2E validation to avoid semantic search noise.

1. **`tests/run_groovy_e2e.sh`:** Deploy isolated Neo4j + Qdrant. Index a `TMP_REPO_DIR` with only `.groovy` files.
2. **E2E Validations:**
   - `search_hybrid_context` for a trait.
   - `find_callers` to validate dependencies/invocations.
   - `explore_file` to return exact class/method structure.

## Phase 10.10: Documentation & Release (v0.10.5)
1. **Docs:** Update `README.md`.
2. **Roadmap:** Update `docs/specs/multilanguage_roadmap.md`.
3. **Release:** Bump to `0.10.5`, commit, tag, and push.