# E2E Test Suite Architecture: Per-Suite Fixture Directories

## 1. Problem Statement

In the fast multi-suite E2E runner (`run_all_e2e_fast.sh`), a single shared instance of Neo4j and Qdrant is used across all test suites, and the indexer is invoked without the `--clean` flag to preserve data from previously run suites.

When multiple single-language test suites (e.g., Kotlin, Python) point to the same physical directory (`tests/testing_files/`) but use different logical repository names (`REPO_NAME`), an indexing conflict occurs:
1. `run_e2e.sh` runs first, indexing all files and creating `tests/testing_files/.knot/index_state.json`.
2. `run_kotlin_e2e.sh` runs next, pointing to the same directory but with a different `REPO_NAME`.
3. The indexer reads the existing `index_state.json`, sees that file hashes haven't changed, and exits early without indexing the files under the new `REPO_NAME`.
4. The Kotlin suite's MCP queries fail because no data was ingested for its `REPO_NAME`.

Previously, this was worked around by having scripts dynamically copy specific files into temporary directories (`$TMP_REPO_DIR`) at runtime. While this bypassed the shared state issue, it resulted in messy script logic that obscured the actual structure being tested.

## 2. Architectural Decision

**One Project = One Directory = One `index_state.json`**

To mirror real-world production usage and clean up test scripts, each E2E test suite that relies on static files will have its own dedicated permanent fixture subdirectory inside `tests/testing_files/`.

By pointing each test script to its own dedicated subdirectory:
- The indexer will naturally create an isolated `.knot/index_state.json` inside each fixture directory.
- There will be zero collision of index states between different test suites.
- Test scripts will no longer need complex runtime `mkdir` and `cp` logic to construct isolated environments.

## 3. Implementation Details

### 3.1. Directory Restructuring

We will create dedicated directories and copy (not move) the relevant sample files into them. We must copy rather than move because the main `run_e2e.sh` acts as a multi-language integration test and requires all files to remain in the root `testing_files/` directory.

New structure to create:
```
tests/testing_files/
├── kotlin/
│   └── sample.kt                               (copied from root)
├── python/
│   └── sample.py                               (copied from root)
├── rust/
│   ├── sample.rs                               (copied from root)
│   └── rust_use_imports/                       (copied from root)
├── build_systems/
│   ├── pom.xml                                 (copied from sample_pom.xml)
│   ├── build.gradle                            (copied from sample_build.gradle)
│   ├── Jenkinsfile                             (copied from sample.jenkinsfile)
│   └── Cargo.toml                              (copied from sample_Cargo.toml)
├── config/
│   ├── application.yml                         (copied from sample_application.yml)
│   ├── config.json                             (copied from sample_config.json)
│   ├── app.properties                          (copied from sample_app.properties)
│   └── package.json                            (copied from sample_package.json)
└── k8s_helm/
    ├── k8s/
    │   ├── deployment.yaml                     (copied from k8s/deployment.yaml)
    │   ├── service.yaml                        (copied from k8s/service.yaml)
    │   └── configmap.yaml                      (copied from k8s/configmap.yaml)
    └── helm/
        └── charts/
            └── sample-app/
                ├── Chart.yaml                  (copied from helm/Chart.yaml)
                ├── values.yaml                 (copied from helm/values.yaml)
                └── templates/
                    └── deployment.yaml         (copied from helm/templates/deployment.yaml)
```

*(Note: `rust_reference_resolution`, `rust_test_module`, and `rust_contains_collision` already exist in the correct format).*

### 3.2. Script Modifications

For the following scripts:
- `run_kotlin_e2e.sh`
- `run_python_e2e.sh`
- `run_rust_e2e.sh`
- `run_build_systems_e2e.sh`
- `run_config_e2e.sh`
- `run_k8s_helm_e2e.sh`
- `run_rust_reference_resolution_e2e.sh`
- `run_rust_test_module_e2e.sh`

The following refactoring will be applied:
1. Point `TEST_FILES_DIR` to the new dedicated subdirectory.
2. Remove all `TMP_REPO_DIR` variable declarations.
3. Remove runtime file copying logic (`rm -rf $TMP_REPO_DIR`, `mkdir -p`, `cp ...`).
4. Set `export KNOT_REPO_PATH="$TEST_FILES_DIR"`.
5. Update specific file path variables (e.g., `KT_FILE="$TEST_FILES_DIR/sample.kt"` instead of pointing to the temp dir or root dir).
6. Remove `TMP_REPO_DIR` from the cleanup trap functions (`rm -rf $TMP_REPO_DIR`).

### 3.3. Gitignore Update

Because running tests locally will now generate `.knot/` directories inside the permanent fixture directories, we must prevent these state files from being committed to the repository.

Add the following rule to the root `.gitignore`:
```gitignore
# Ignore knot index state in test fixture directories
tests/testing_files/**/.knot/
```

### 3.4. Dynamic Suites (Exceptions)

The following test suites generate source code dynamically (using `cat > file <<EOF`) across multiple phases to test incremental indexing and cross-repository linking. They cannot be easily migrated to static fixture directories and **will continue to use `TMP_REPO_DIR`**:

- `run_groovy_e2e.sh`
- `run_cpp_e2e.sh`
- `run_cross_lang_ref_e2e.sh`
- `run_cross_repo_dep_e2e.sh`

## 4. Expected Outcome

After this refactoring, the codebase will cleanly separate the concept of "files used for the multi-language test" from "files used for isolated single-language tests." The E2E suites will be highly robust in shared-DB CI environments, and the test scripts themselves will be significantly shorter and easier to read.
