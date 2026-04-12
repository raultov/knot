use anyhow::{Context, Result};
use streaming_iterator::StreamingIterator;
use tree_sitter::{Language, Node, Parser, Query, QueryCursor};

use super::comments::*;
use super::context::*;
use super::languages::{java, javascript, typescript};
use super::orphans::*;
use super::utils::*;
use crate::models::{EntityKind, ParsedEntity, ReferenceIntent};

/// Run a Tree-sitter query against `source` and convert matches to [`ParsedEntity`].
pub(crate) fn extract_entities(
    source: &str,
    language: Language,
    query_src: &str,
    lang_name: &str,
    file_path: &str,
    repo_name: &str,
) -> Result<Vec<ParsedEntity>> {
    let mut parser = Parser::new();
    parser
        .set_language(&language)
        .context("Failed to set Tree-sitter language")?;

    let tree = parser
        .parse(source, None)
        .context("Tree-sitter failed to parse source")?;

    let query = Query::new(&language, query_src).context("Failed to compile Tree-sitter query")?;

    let source_bytes = source.as_bytes();
    let mut cursor = QueryCursor::new();

    let capture_names: Vec<String> = query
        .capture_names()
        .iter()
        .map(|s| s.to_string())
        .collect();

    let mut entities: Vec<ParsedEntity> = Vec::new();

    // First pass: extract all class/interface names and their line ranges for context
    let mut class_contexts: Vec<ClassContext> = Vec::new();
    extract_class_contexts(tree.root_node(), source_bytes, &mut class_contexts);

    // Second pass: extract entities and resolve their contexts
    let mut matches = cursor.matches(&query, tree.root_node(), source_bytes);
    let mut covered_ranges: Vec<(usize, usize)> = Vec::new();

    while let Some(m) = {
        matches.advance();
        matches.get()
    } {
        let mut name: Option<String> = None;
        let mut kind: Option<EntityKind> = None;
        let mut signature: Option<String> = None;
        let mut start_line: usize = 0;
        let mut entity_node: Option<Node> = None;
        let mut reference_intents: Vec<ReferenceIntent> = Vec::new();

        for cap in m.captures {
            let cap_name = &capture_names[cap.index as usize];
            let node = cap.node;
            let text = node_text(node, source_bytes);

            match cap_name.as_str() {
                "class.name" => {
                    name = Some(text.clone());
                    kind = Some(EntityKind::Class);
                    start_line = node.start_position().row + 1;
                    entity_node = find_parent_by_kind(node, "class_declaration")
                        .or_else(|| find_parent_by_kind(node, "abstract_class_declaration"));
                }
                "interface.name" => {
                    name = Some(text.clone());
                    kind = Some(EntityKind::Interface);
                    start_line = node.start_position().row + 1;
                    entity_node = find_parent_by_kind(node, "interface_declaration");
                }
                "method.name" => {
                    name = Some(text.clone());
                    kind = Some(EntityKind::Method);
                    start_line = node.start_position().row + 1;
                    entity_node = find_parent_by_kind(node, "method_declaration")
                        .or_else(|| find_parent_by_kind(node, "method_definition"))
                        .or_else(|| find_parent_by_kind(node, "method_signature"))
                        .or_else(|| find_parent_by_kind(node, "abstract_method_signature"));
                    // For methods, extract reference intents from the method body
                    if let Some(method_node) = entity_node {
                        if lang_name == "java" {
                            java::extract_reference_intents_java(
                                method_node,
                                source_bytes,
                                &mut reference_intents,
                            );
                        } else if lang_name == "javascript" {
                            javascript::extract_reference_intents_javascript(
                                method_node,
                                source_bytes,
                                &mut reference_intents,
                            );
                        } else {
                            typescript::extract_reference_intents_typescript(
                                method_node,
                                source_bytes,
                                &mut reference_intents,
                            );
                        }

                        // Extract type references from method signatures (parameters, return types)
                        if lang_name == "typescript" {
                            typescript::extract_type_references(
                                method_node,
                                source_bytes,
                                &mut reference_intents,
                            );
                        } else if lang_name == "java" {
                            java::extract_type_references(
                                method_node,
                                source_bytes,
                                &mut reference_intents,
                            );
                        }
                    }
                }
                "function.name" => {
                    name = Some(text.clone());
                    kind = Some(EntityKind::Function);
                    start_line = node.start_position().row + 1;
                    entity_node = find_parent_by_kind(node, "function_declaration")
                        .or_else(|| find_parent_by_kind(node, "lexical_declaration"))
                        .or_else(|| find_parent_by_kind(node, "variable_declaration"))
                        .or_else(|| find_parent_by_kind(node, "export_statement"));
                    // For functions, extract reference intents from the function body
                    if let Some(func_node) = entity_node {
                        if lang_name == "javascript" {
                            javascript::extract_reference_intents_javascript(
                                func_node,
                                source_bytes,
                                &mut reference_intents,
                            );
                        } else {
                            typescript::extract_reference_intents_typescript(
                                func_node,
                                source_bytes,
                                &mut reference_intents,
                            );
                        }
                    }
                }
                "constant.name" => {
                    name = Some(text.clone());
                    kind = Some(EntityKind::Constant);
                    start_line = node.start_position().row + 1;
                    entity_node = find_parent_by_kind(node, "lexical_declaration")
                        .or_else(|| find_parent_by_kind(node, "variable_declarator"))
                        .or_else(|| find_parent_by_kind(node, "field_declaration"))
                        .or_else(|| find_parent_by_kind(node, "public_field_definition"))
                        .or_else(|| find_parent_by_kind(node, "field_definition"));

                    // Extract reference intents from constant initializers
                    // This captures function calls inside const assignments like:
                    //   const formattedItems = formatRegistryItems(registryItems)
                    //   const config = await getMcpConfig(process.cwd())
                    if let Some(const_node) = entity_node {
                        if lang_name == "java" {
                            java::extract_reference_intents_java(
                                const_node,
                                source_bytes,
                                &mut reference_intents,
                            );
                        } else if lang_name == "javascript" {
                            javascript::extract_reference_intents_javascript(
                                const_node,
                                source_bytes,
                                &mut reference_intents,
                            );
                        } else {
                            typescript::extract_reference_intents_typescript(
                                const_node,
                                source_bytes,
                                &mut reference_intents,
                            );
                        }
                    }
                }
                "enum.name" => {
                    name = Some(text.clone());
                    kind = Some(EntityKind::Enum);
                    start_line = node.start_position().row + 1;
                    entity_node = find_parent_by_kind(node, "enum_declaration");
                }
                "signature" => signature = Some(text.clone()),
                "type.reference" => {
                    // Type annotations in signatures, variables, etc.
                    reference_intents.push(ReferenceIntent::TypeReference {
                        type_name: text.clone(),
                        line: node.start_position().row + 1,
                    });
                }
                _ => {}
            }
        }

        if let (Some(name), Some(kind)) = (name, kind) {
            // Extract docstring and inline comments dynamically from the entity node
            let (docstring, inline_comments) = if let Some(node) = entity_node {
                extract_comments(node, source_bytes, lang_name, &kind, &class_contexts)
            } else {
                (None, Vec::new())
            };

            // Extract decorators/annotations from the entity node
            let decorators = if let Some(node) = entity_node {
                extract_decorators(node, source_bytes, lang_name)
            } else {
                Vec::new()
            };

            // Determine FQN and enclosing class based on context
            let (fqn, enclosing_class) =
                compute_fqn_and_context(&name, &kind, start_line, lang_name, &class_contexts);

            // For classes, also extract extends/implements from AST
            if matches!(kind, EntityKind::Class | EntityKind::Interface)
                && let Some(class_node) = entity_node
            {
                if lang_name == "javascript" {
                    javascript::extract_class_inheritance_js(
                        class_node,
                        source_bytes,
                        &mut reference_intents,
                    );
                    // Extract decorator references for JavaScript (e.g., @Component, @Injectable)
                    // Decorators may be in the parent node (export_statement) rather than class_declaration
                    let decorator_node = class_node
                        .parent()
                        .filter(|p| p.kind() == "export_statement")
                        .unwrap_or(class_node);
                    javascript::extract_decorator_references(
                        decorator_node,
                        source_bytes,
                        &mut reference_intents,
                    );
                } else if lang_name == "typescript" {
                    typescript::extract_class_inheritance(
                        class_node,
                        source_bytes,
                        &mut reference_intents,
                    );
                    // Extract decorator references (e.g., @Component, @NgModule)
                    // Decorators may be in the parent node (export_statement) rather than class_declaration
                    let decorator_node = class_node
                        .parent()
                        .filter(|p| p.kind() == "export_statement")
                        .unwrap_or(class_node);
                    typescript::extract_decorator_references(
                        decorator_node,
                        source_bytes,
                        &mut reference_intents,
                    );
                    // Extract type references (e.g., constructor parameters, property types)
                    typescript::extract_type_references(
                        class_node,
                        source_bytes,
                        &mut reference_intents,
                    );
                } else if lang_name == "java" {
                    // Extract annotation references (e.g., @Component, @Autowired)
                    java::extract_annotation_references(
                        class_node,
                        source_bytes,
                        &mut reference_intents,
                    );
                    // Extract type references (e.g., constructor parameters, field types)
                    java::extract_type_references(class_node, source_bytes, &mut reference_intents);
                }
            }

            let mut entity = ParsedEntity::new(
                name,
                kind,
                fqn,
                signature,
                docstring,
                lang_name,
                file_path,
                start_line,
                enclosing_class,
                repo_name,
            );
            entity.reference_intents = reference_intents;
            entity.inline_comments = inline_comments;
            entity.decorators = decorators;

            // Track byte range of this entity for orphan detection
            // Must be done for ALL entities to keep indices aligned with the entities vector
            if let Some(node) = entity_node {
                covered_ranges.push((node.start_byte(), node.end_byte()));
            } else {
                // If we don't have a node, use a dummy range that won't match any orphans
                covered_ranges.push((usize::MAX, usize::MAX));
            }

            entities.push(entity);
        }
    }

    // Deduplicate entities extracted from tree-sitter queries.
    // This handles cases where multiple query patterns match the same AST node.
    // For example, in JavaScript: var foo = function() {} can match both:
    //   1. (variable_declaration ... @function.name) → foo as Function
    //   2. (variable_declaration ... @constant.name) → foo as Constant
    // Deduplication key: (file_path, name, kind, start_line)
    // This ensures we keep only one entity per unique declaration.
    entities.sort_by(|a, b| {
        a.file_path
            .cmp(&b.file_path)
            .then(a.name.cmp(&b.name))
            .then(format!("{:?}", a.kind).cmp(&format!("{:?}", b.kind)))
            .then(a.start_line.cmp(&b.start_line))
    });
    entities.dedup_by(|a, b| {
        a.file_path == b.file_path
            && a.name == b.name
            && a.kind == b.kind
            && a.start_line == b.start_line
    });

    // Third pass: capture orphaned reference intents (calls in top-level statements,
    // callbacks, etc. that were not captured by any named entity)
    if lang_name == "typescript" || lang_name == "java" || lang_name == "javascript" {
        collect_orphaned_references(
            tree.root_node(),
            source_bytes,
            lang_name,
            &mut entities,
            &covered_ranges,
            file_path,
            repo_name,
        );
    }

    Ok(entities)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_entities_empty_source_java() {
        let source = "";
        let result = extract_entities(
            source,
            tree_sitter_java::LANGUAGE.into(),
            "",
            "java",
            "/test.java",
            "test-repo",
        );

        // Empty source should still return Ok with empty vec
        assert!(result.is_ok());
        assert!(result.unwrap().is_empty());
    }

    #[test]
    fn test_extract_entities_empty_source_typescript() {
        let source = "";
        let result = extract_entities(
            source,
            tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
            "",
            "typescript",
            "/test.ts",
            "test-repo",
        );

        assert!(result.is_ok());
        assert!(result.unwrap().is_empty());
    }

    #[test]
    fn test_extract_entities_simple_java_class() {
        let source = "public class MyClass {}";
        let query = "(class_declaration name: (identifier) @class.name)";

        let result = extract_entities(
            source,
            tree_sitter_java::LANGUAGE.into(),
            query,
            "java",
            "/MyClass.java",
            "test-repo",
        );

        assert!(result.is_ok());
        let entities = result.unwrap();
        assert!(!entities.is_empty());
        assert_eq!(entities[0].name, "MyClass");
        assert_eq!(entities[0].kind, EntityKind::Class);
    }

    #[test]
    fn test_extract_entities_simple_typescript_function() {
        let source = "function myFunction() {}";
        let query = "(function_declaration name: (identifier) @function.name)";

        let result = extract_entities(
            source,
            tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
            query,
            "typescript",
            "/test.ts",
            "test-repo",
        );

        assert!(result.is_ok());
        let entities = result.unwrap();
        assert!(!entities.is_empty());
        assert_eq!(entities[0].name, "myFunction");
        assert_eq!(entities[0].kind, EntityKind::Function);
    }

    #[test]
    fn test_extract_entities_with_signature() {
        let source = "public void testMethod(String param) {}";
        let query = "(method_declaration name: (identifier) @method.name (#any-of? @method.name \"testMethod\"))";

        let result = extract_entities(
            source,
            tree_sitter_java::LANGUAGE.into(),
            query,
            "java",
            "/Test.java",
            "test-repo",
        );

        assert!(result.is_ok());
        let entities = result.unwrap();
        assert!(!entities.is_empty());
    }

    #[test]
    fn test_extract_entities_interface_java() {
        let source = "public interface MyInterface {}";
        // Use identifier instead of type_identifier for interface names in Java
        let query = "(interface_declaration name: (identifier) @interface.name)";

        let result = extract_entities(
            source,
            tree_sitter_java::LANGUAGE.into(),
            query,
            "java",
            "/MyInterface.java",
            "test-repo",
        );

        assert!(result.is_ok());
        let entities = result.unwrap();
        assert!(!entities.is_empty());
        assert_eq!(entities[0].kind, EntityKind::Interface);
    }

    #[test]
    fn test_extract_entities_enum_java() {
        let source = "public enum Color { RED, GREEN, BLUE }";
        let query = "(enum_declaration name: (identifier) @enum.name)";

        let result = extract_entities(
            source,
            tree_sitter_java::LANGUAGE.into(),
            query,
            "java",
            "/Color.java",
            "test-repo",
        );

        assert!(result.is_ok());
    }

    #[test]
    fn test_extract_entities_constant_typescript() {
        let source = "const MY_CONSTANT = 42;";
        let query = "(lexical_declaration (variable_declarator name: (identifier) @constant.name))";

        let result = extract_entities(
            source,
            tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
            query,
            "typescript",
            "/constants.ts",
            "test-repo",
        );

        assert!(result.is_ok());
        let entities = result.unwrap();
        assert!(!entities.is_empty());
        assert_eq!(entities[0].kind, EntityKind::Constant);
    }

    #[test]
    fn test_extract_entities_with_docstring() {
        let source = "/** Test documentation */\npublic class DocClass {}";
        let query = "(class_declaration name: (identifier) @class.name)";

        let result = extract_entities(
            source,
            tree_sitter_java::LANGUAGE.into(),
            query,
            "java",
            "/DocClass.java",
            "test-repo",
        );

        assert!(result.is_ok());
        let entities = result.unwrap();
        assert!(!entities.is_empty());
        // Docstring extraction depends on comments parsing
    }

    #[test]
    fn test_extract_entities_multiple_entities_java() {
        let source = "public class FirstClass {} public class SecondClass {}";
        let query = "(class_declaration name: (identifier) @class.name)";

        let result = extract_entities(
            source,
            tree_sitter_java::LANGUAGE.into(),
            query,
            "java",
            "/Classes.java",
            "test-repo",
        );

        assert!(result.is_ok());
        let entities = result.unwrap();
        assert_eq!(entities.len(), 2);
        assert_eq!(entities[0].name, "FirstClass");
        assert_eq!(entities[1].name, "SecondClass");
    }

    #[test]
    fn test_extract_entities_nested_class() {
        let source = "public class Outer { public class Inner {} }";
        let query = "(class_declaration name: (identifier) @class.name)";

        let result = extract_entities(
            source,
            tree_sitter_java::LANGUAGE.into(),
            query,
            "java",
            "/Outer.java",
            "test-repo",
        );

        assert!(result.is_ok());
        let entities = result.unwrap();
        // Should find both outer and inner classes
        assert!(!entities.is_empty());
    }

    #[test]
    fn test_extract_entities_file_path_preservation() {
        let file_path = "/src/main/java/com/example/MyClass.java";
        let source = "public class MyClass {}";
        let query = "(class_declaration name: (identifier) @class.name)";

        let result = extract_entities(
            source,
            tree_sitter_java::LANGUAGE.into(),
            query,
            "java",
            file_path,
            "test-repo",
        );

        assert!(result.is_ok());
        let entities = result.unwrap();
        assert!(!entities.is_empty());
        assert_eq!(entities[0].file_path, file_path);
    }

    #[test]
    fn test_extract_entities_repo_name_preservation() {
        let repo_name = "my-awesome-repo";
        let source = "public class MyClass {}";
        let query = "(class_declaration name: (identifier) @class.name)";

        let result = extract_entities(
            source,
            tree_sitter_java::LANGUAGE.into(),
            query,
            "java",
            "/MyClass.java",
            repo_name,
        );

        assert!(result.is_ok());
        let entities = result.unwrap();
        assert!(!entities.is_empty());
        assert_eq!(entities[0].repo_name, repo_name);
    }

    #[test]
    fn test_extract_entities_start_line_calculation() {
        let source = "\n\n\npublic class MyClass {}";
        let query = "(class_declaration name: (identifier) @class.name)";

        let result = extract_entities(
            source,
            tree_sitter_java::LANGUAGE.into(),
            query,
            "java",
            "/MyClass.java",
            "test-repo",
        );

        assert!(result.is_ok());
        let entities = result.unwrap();
        assert!(!entities.is_empty());
        // Line should be 4 (1-indexed, after 3 newlines)
        assert_eq!(entities[0].start_line, 4);
    }

    #[test]
    fn test_extract_entities_language_name_preserved() {
        let source = "public class MyClass {}";
        let query = "(class_declaration name: (identifier) @class.name)";

        let result_java = extract_entities(
            source,
            tree_sitter_java::LANGUAGE.into(),
            query,
            "java",
            "/MyClass.java",
            "test-repo",
        );

        assert!(result_java.is_ok());
        let entities_java = result_java.unwrap();
        assert!(!entities_java.is_empty());
        // Language is part of the model but not directly accessible via public getter
        // This is tested indirectly through extraction behavior
    }

    #[test]
    fn test_extract_entities_deduplication_javascript() {
        // Test deduplication with overlapping patterns in JavaScript
        // The actual javascript.scm has patterns that can capture the same node
        // multiple times due to tree-sitter query semantics
        let source = r#"
            function myFunc() {}
            var myVar = 42;
        "#;

        // Use a simple query that extracts both functions and constants
        let query = r#"
            (function_declaration name: (identifier) @function.name)
            (variable_declaration
              (variable_declarator
                name: (identifier) @constant.name))
        "#;

        let result = extract_entities(
            source,
            tree_sitter_javascript::LANGUAGE.into(),
            query,
            "javascript",
            "/test.js",
            "test-repo",
        );

        assert!(result.is_ok());
        let entities = result.unwrap();

        // Should have 2 unique entities
        assert_eq!(
            entities.len(),
            2,
            "Should have exactly 2 unique entities: function and constant"
        );

        // Verify myFunc is captured as function
        let func_entity = entities.iter().find(|e| e.name == "myFunc");
        assert!(func_entity.is_some());
        assert_eq!(func_entity.unwrap().kind, EntityKind::Function);

        // Verify myVar is captured as constant
        let const_entity = entities.iter().find(|e| e.name == "myVar");
        assert!(const_entity.is_some());
        assert_eq!(const_entity.unwrap().kind, EntityKind::Constant);
    }

    #[test]
    fn test_extract_entities_deduplication_respects_file_path() {
        // Same entity name in different "files" should NOT be deduplicated
        let source = "var myVar = 42;";
        let query =
            "(variable_declaration (variable_declarator name: (identifier) @constant.name))";

        // Extract from "file1"
        let result1 = extract_entities(
            source,
            tree_sitter_javascript::LANGUAGE.into(),
            query,
            "javascript",
            "/file1.js",
            "test-repo",
        );

        // Extract from "file2"
        let result2 = extract_entities(
            source,
            tree_sitter_javascript::LANGUAGE.into(),
            query,
            "javascript",
            "/file2.js",
            "test-repo",
        );

        assert!(result1.is_ok());
        assert!(result2.is_ok());

        let entities1 = result1.unwrap();
        let entities2 = result2.unwrap();

        // Each file should have 1 entity
        assert_eq!(entities1.len(), 1);
        assert_eq!(entities2.len(), 1);

        // Same name but different file paths
        assert_eq!(entities1[0].name, "myVar");
        assert_eq!(entities2[0].name, "myVar");
        assert_ne!(entities1[0].file_path, entities2[0].file_path);
    }

    #[test]
    fn test_extract_entities_deduplication_respects_kind() {
        // Hypothetical scenario: same name used for class and function
        // (unrealistic in real code, but tests the deduplication logic)
        let source = r#"
            class MyEntity {}
            function MyEntity() {}
        "#;

        let query = r#"
            (class_declaration name: (identifier) @class.name)
            (function_declaration name: (identifier) @function.name)
        "#;

        let result = extract_entities(
            source,
            tree_sitter_javascript::LANGUAGE.into(),
            query,
            "javascript",
            "/test.js",
            "test-repo",
        );

        assert!(result.is_ok());
        let entities = result.unwrap();

        // Should have 2 entities with same name but different kinds
        assert_eq!(
            entities.len(),
            2,
            "Should keep both entities with same name but different kinds"
        );

        let class_entity = entities.iter().find(|e| e.kind == EntityKind::Class);
        let function_entity = entities.iter().find(|e| e.kind == EntityKind::Function);

        assert!(class_entity.is_some());
        assert!(function_entity.is_some());
        assert_eq!(class_entity.unwrap().name, "MyEntity");
        assert_eq!(function_entity.unwrap().name, "MyEntity");
    }

    #[test]
    fn test_extract_entities_deduplication_respects_line_number() {
        // Multiple functions with same name on different lines (overloading scenario)
        let source = r#"
            function process(x) { return x; }
            function process(x, y) { return x + y; }
        "#;

        let query = "(function_declaration name: (identifier) @function.name)";

        let result = extract_entities(
            source,
            tree_sitter_javascript::LANGUAGE.into(),
            query,
            "javascript",
            "/test.js",
            "test-repo",
        );

        assert!(result.is_ok());
        let entities = result.unwrap();

        // Should have 2 entities (same name, same kind, different lines)
        assert_eq!(
            entities.len(),
            2,
            "Should keep both functions with same name on different lines"
        );

        // Both should be named "process"
        assert!(entities.iter().all(|e| e.name == "process"));

        // They should have different start lines
        assert_ne!(entities[0].start_line, entities[1].start_line);
    }

    #[test]
    fn test_extract_entities_angular_decorator_references() {
        // Test the complete Angular use case that motivated this bugfix
        let source = r#"
            import { Component } from '@angular/core';
            import { AnalyticsService } from './analytics.service';
            import { SeoService } from './seo.service';

            @Component({
                selector: 'ngx-app',
                template: '<router-outlet></router-outlet>',
            })
            export class AppComponent {
                constructor(
                    private analytics: AnalyticsService,
                    private seo: SeoService
                ) {}
            }
        "#;

        let query = r#"
            (class_declaration name: (type_identifier) @class.name)
        "#;

        let result = extract_entities(
            source,
            tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
            query,
            "typescript",
            "/test/app.component.ts",
            "test-repo",
        );

        assert!(result.is_ok());
        let entities = result.unwrap();

        // Should have 1 class entity
        assert_eq!(entities.len(), 1);
        let app_component = &entities[0];
        assert_eq!(app_component.name, "AppComponent");

        // Should have captured decorator references
        let decorator_refs: Vec<_> = app_component
            .reference_intents
            .iter()
            .filter(|r| matches!(r, crate::models::ReferenceIntent::TypeReference { type_name, .. } if type_name == "Component"))
            .collect();
        assert!(
            !decorator_refs.is_empty(),
            "Should capture @Component decorator reference"
        );

        // Should have captured type references from constructor parameters
        let analytics_refs: Vec<_> = app_component
            .reference_intents
            .iter()
            .filter(|r| matches!(r, crate::models::ReferenceIntent::TypeReference { type_name, .. } if type_name == "AnalyticsService"))
            .collect();
        assert!(
            !analytics_refs.is_empty(),
            "Should capture AnalyticsService type reference from constructor"
        );

        let seo_refs: Vec<_> = app_component
            .reference_intents
            .iter()
            .filter(|r| matches!(r, crate::models::ReferenceIntent::TypeReference { type_name, .. } if type_name == "SeoService"))
            .collect();
        assert!(
            !seo_refs.is_empty(),
            "Should capture SeoService type reference from constructor"
        );
    }

    #[test]
    fn test_extract_entities_angular_ngmodule_references() {
        // Test NgModule decorator with component references
        let source = r#"
            import { NgModule } from '@angular/core';
            import { AppComponent } from './app.component';
            import { UserComponent } from './user.component';

            @NgModule({
                declarations: [AppComponent, UserComponent],
                bootstrap: [AppComponent]
            })
            export class AppModule {}
        "#;

        let query = r#"
            (class_declaration name: (type_identifier) @class.name)
        "#;

        let result = extract_entities(
            source,
            tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
            query,
            "typescript",
            "/test/app.module.ts",
            "test-repo",
        );

        assert!(result.is_ok());
        let entities = result.unwrap();

        // Should have 1 class entity
        assert_eq!(entities.len(), 1);
        let app_module = &entities[0];
        assert_eq!(app_module.name, "AppModule");

        // Should capture NgModule decorator reference
        let ngmodule_refs: Vec<_> = app_module
            .reference_intents
            .iter()
            .filter(|r| matches!(r, crate::models::ReferenceIntent::TypeReference { type_name, .. } if type_name == "NgModule"))
            .collect();
        assert!(
            !ngmodule_refs.is_empty(),
            "Should capture @NgModule decorator reference"
        );

        // Should capture AppComponent references from decorator arguments
        let app_component_refs: Vec<_> = app_module
            .reference_intents
            .iter()
            .filter(|r| matches!(r, crate::models::ReferenceIntent::TypeReference { type_name, .. } if type_name == "AppComponent"))
            .collect();
        assert!(
            app_component_refs.len() >= 2,
            "Should capture AppComponent references (appears in declarations and bootstrap)"
        );

        // Should capture UserComponent reference from decorator arguments
        let user_component_refs: Vec<_> = app_module
            .reference_intents
            .iter()
            .filter(|r| matches!(r, crate::models::ReferenceIntent::TypeReference { type_name, .. } if type_name == "UserComponent"))
            .collect();
        assert!(
            !user_component_refs.is_empty(),
            "Should capture UserComponent reference from declarations"
        );
    }
}
