# Integration Plan: Python Support in `knot`

This document details the step-by-step strategy to add native code analysis support for Python (compatible with both Python 2 and Python 3) into the `knot` indexer. It is divided into incremental phases to facilitate a gradual implementation.

## Phase 1: Base Configuration and Language Registration
**Objective:** Integrate the parser dependency and ensure `knot` can process files with `.py` extensions, without extracting complex logic yet.

1. **Dependencies:**
   - Add `tree-sitter-python` to `Cargo.toml`.
2. **Data Models (`src/models/entity.rs`):**
   - Extend the `EntityKind` enum with initial variants: `PythonClass`, `PythonFunction`, `PythonMethod`.
   - Implement visual formatting (ANSI colors for CLI) in the `Display` and `from_str` blocks.
3. **Parsing Pipeline (`src/pipeline/parser/mod.rs`):**
   - Register associated extensions: `py`, `pyi` (type stubs), and `pyw`.
   - Wire the execution to a new module `languages::python` and the query in `queries/python.scm`.

## Phase 2: Basic Structural Extraction and Docstrings
**Objective:** Capture structural definitions (functions, classes, methods) and their associated documentation.

1. **Tree-sitter Queries (`queries/python.scm`):**
   - Capture `class_definition` and extract the name.
   - Capture `function_definition` at the module level.
   - Capture `function_definition` as a child of a class to categorize them as `EntityKind::PythonMethod`.
2. **Docstrings and Signatures:**
   - Extract the first `expression_statement` that contains a string literal right below the definition (Python docstring convention).
   - Capture the parameters of function/method signatures (including Python 3 typing support).

## Phase 3: References and Calls (Call & Reference Graph)
**Objective:** Allow `knot` to understand who calls who within the same file.

1. **Invocation Capture:**
   - Add rules to `queries/python.scm` for the `call` node.
   - Differentiate direct calls (`identifier()`) from method/attribute calls (`object.method()`).
2. **Python 2 Compatibility:**
   - Consider the `print_statement` node (specific to Python 2) in addition to the `print()` function (Python 3).
3. **Relationships:** Emit `CallIntent` entities and connect them in the pipeline to build `CALLS` edges in the dependency graph.

## Phase 4: Imports and Module Graph
**Objective:** Capture dependencies between different files and packages.

1. **New EntityKinds:**
   - Add `EntityKind::PythonModule` and potentially global module constants.
2. **Import Resolution (`import_statement`, `import_from_statement`):**
   - Detect statements like `import os` or `from django.db import models`.
   - Map these imports to `REFERENCES` relationships, so that if the imported module is part of the indexed project, Neo4j can create the inter-file connection edge.

## Phase 5: Advanced Object-Orientation (Inheritance and Decorators)
**Objective:** Extract metaprogramming and class inheritance, essential in frameworks like Django, FastAPI, or Flask.

1. **Inheritance:**
   - Analyze the `argument_list` in class definitions (e.g., `class User(AbstractBaseUser):`).
   - Create an `EXTENDS` relationship between the child class and the parent class.
2. **Decorators:**
   - Identify the `decorator` node (`@property`, `@staticmethod`, `@route(...)`).
   - Map the decorator application as a hidden call (`CALLS` relationship) to the decorator function/class.
3. **Old-Style Classes (Py2):**
   - Ensure in the tests that class definitions without explicit inheritance in Python 2 (`class MyClass:`) are correctly handled and parsed.

## Phase 6: Unit and E2E Testing
**Objective:** Secure the integration against syntax disparities between different versions.

1. **Test Module (`src/pipeline/parser/languages/python.rs`):**
   - Write unit tests verifying:
     - Syntax differences between Py2 and Py3 exceptions.
     - Correct extraction of generic Type Hints (`List[str]`).
     - Correct extraction of variable unpacking (`*args`, `**kwargs`).
2. **Full Integration:** Add test code to the End-to-End suite to verify that all entities and references end up correctly serialized and saved in the Vector DB (Qdrant) and the Graph DB (Neo4j).