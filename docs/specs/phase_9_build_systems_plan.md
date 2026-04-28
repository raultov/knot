# Phase 9: Build Systems & CI/CD Support (Groovy, Gradle, Maven, Jenkins)

## Objective
Enable `knot` to index project infrastructure and build configurations. By understanding `build.gradle`, `pom.xml`, and `Jenkinsfile`, the MCP server will be able to answer semantic questions about project dependencies, custom build tasks, and deployment pipeline stages.

## Target Files
- **Gradle**: `build.gradle`, `settings.gradle` (Groovy DSL)
- **Maven**: `pom.xml` (XML)
- **Jenkins**: `Jenkinsfile` (Groovy Pipeline DSL)

---

## Phase 9.1: Base Integration & Parsers (Tree-sitter)
**Goal:** Wire up the required tree-sitter parsers and register the new file extensions.

1. **Cargo Dependencies:**
   - Add `tree-sitter-groovy` to `Cargo.toml`.
   - Add `tree-sitter-xml` to `Cargo.toml`.
2. **Language Modules:**
   - Create `src/pipeline/parser/languages/groovy.rs` implementing the `LanguageParser` trait.
   - Create `src/pipeline/parser/languages/xml.rs` implementing the `LanguageParser` trait.
3. **Pipeline Wiring (`src/pipeline/parser/languages/mod.rs`):**
   - Route `*.groovy`, `build.gradle`, `settings.gradle`, and `Jenkinsfile` to the Groovy parser.
   - Route `*.xml` and `pom.xml` to the XML parser.
4. **Entity Kinds (`src/models/entity.rs`):**
   - Add new `EntityKind` variants: `BuildDependency`, `BuildPlugin`, `BuildTask`, `PipelineStage`, `PipelineStep`.

## Phase 9.2: Maven (`pom.xml`) Dependency Extraction
**Goal:** Extract dependencies and plugins from Maven XML files.

1. **XML S-Expression Queries (`xml.rs`):**
   - Write queries to target `<dependency>` nodes inside `<dependencies>`.
   - Extract child nodes text: `<groupId>`, `<artifactId>`, and `<version>`.
2. **Entity Construction:**
   - Create `BuildDependency` entities.
   - Format the entity `name` as `groupId:artifactId:version` (e.g., `org.springframework:spring-core:5.3.9`).
   - Store the XML block in the docstring/signature for context.
3. **Plugin Extraction:**
   - Write queries to target `<plugin>` nodes inside `<build><plugins>` and map them to `BuildPlugin`.
4. **Unit Tests:**
   - Add a sample `pom.xml` in `tests/testing_files/` and write extraction unit tests in `src/pipeline/parser/extractor.rs`.

## Phase 9.3: Gradle (`build.gradle`) DSL Extraction
**Goal:** Extract dependencies, plugins, and custom tasks from Gradle Groovy scripts.

1. **Groovy S-Expression Queries (`groovy.rs`):**
   - **Dependencies:** Target method calls inside the `dependencies { ... }` closure (e.g., `implementation 'com.google.code.gson:gson:2.8.6'`). Extract the configuration type (`implementation`, `api`, `testImplementation`) and the library string. Map to `BuildDependency`.
   - **Plugins:** Target method calls inside the `plugins { ... }` closure (e.g., `id 'java'`). Map to `BuildPlugin`.
   - **Tasks:** Target task definitions like `task myCustomTask { ... }` or `tasks.register(...)`. Map to `BuildTask`.
2. **Unit Tests:**
   - Add a sample `build.gradle` in `tests/testing_files/` and write extraction unit tests.

## Phase 9.4: Jenkins (`Jenkinsfile`) Pipeline Extraction
**Goal:** Understand CI/CD infrastructure by extracting pipeline stages and execution steps.

1. **Pipeline Queries (`groovy.rs`):**
   - **Stages:** Target method calls named `stage('Stage Name') { ... }` inside the `stages` block. Map to `PipelineStage`.
   - **Steps:** Target method calls inside a `steps { ... }` block (e.g., `sh 'make build'`, `echo 'Deploying'`). Map to `PipelineStep`.
2. **Context Linking:**
   - Use the `enclosing_class` or parent-tracking logic (similar to how methods are linked to classes) to link `PipelineStep` entities to their parent `PipelineStage`.
3. **Unit Tests:**
   - Add a sample `Jenkinsfile` and write extraction unit tests.

## Phase 9.5: Semantic Text Adjustments (`prepare.rs`)
**Goal:** Ensure the embedding text generated for these new entities is highly optimized for vector search.

1. **Format `embed_text`:**
   - Update `build_embed_text()` in `src/pipeline/prepare.rs` to handle the new `EntityKind` variants.
   - For `BuildDependency`, ensure the text explicitly states: `[build_dependency] library: org.springframework:spring-core version: 5.3.9`.
   - Include relevant context so semantic searches like *"what version of spring are we using?"* or *"deployment pipeline stages"* hit the right vectors.

## Phase 9.6: E2E Integration Testing
**Goal:** Validate the full pipeline (Parsing -> Neo4j/Qdrant -> MCP Search).

1. **Create E2E Script:**
   - Create `tests/run_build_systems_e2e.sh`.
   - Isolate the test in a temporary directory containing ONLY the `pom.xml`, `build.gradle`, and `Jenkinsfile` to avoid semantic noise (following the best practice from Phase 8).
2. **Test Cases:**
   - Assert `BuildDependency` entities are found via MCP `search_hybrid_context`.
   - Assert `PipelineStage` entities are found via MCP `search_hybrid_context`.
   - Assert `explore_file` on `pom.xml` returns a structured list of dependencies.
3. **CI Integration:**
   - Add `./tests/run_build_systems_e2e.sh` to `.github/workflows/ci.yml`.

## Phase 9.7: Documentation & Release
1. **Update Documentation:**
   - Update `README.md` to highlight Build Systems & CI/CD support (Maven, Gradle, Jenkins).
   - Mark Phase 9 as `✅ Completed` in `docs/specs/multilanguage_roadmap.md`.
2. **Release:**
   - Bump version to `v0.10.0` in `Cargo.toml`.
   - Commit, tag, and push.