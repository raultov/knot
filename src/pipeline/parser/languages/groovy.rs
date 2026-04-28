use crate::models::{EntityKind, ParsedEntity};

pub(crate) fn handle_groovy_capture(
    capture_name: &str,
    text: &str,
    _node: tree_sitter::Node,
) -> Option<(String, EntityKind, usize)> {
    let line = _node.start_position().row + 1;
    match capture_name {
        "groovy.class.name" => Some((text.to_string(), EntityKind::GroovyClass, line)),
        "groovy.interface.name" => Some((text.to_string(), EntityKind::GroovyInterface, line)),
        "groovy.enum.name" => Some((text.to_string(), EntityKind::GroovyEnum, line)),
        "groovy.method.name" => Some((text.to_string(), EntityKind::GroovyMethod, line)),
        "groovy.field.name" => Some((text.to_string(), EntityKind::GroovyProperty, line)),
        _ => None,
    }
}

pub(crate) fn extract_entities_groovy_standard(
    source: &str,
    file_path: &str,
    repo_name: &str,
) -> Vec<ParsedEntity> {
    let mut entities = match super::super::extractor::extract_entities(
        source,
        tree_sitter_groovy::LANGUAGE.into(),
        include_str!("../../../../queries/groovy.scm"),
        "groovy",
        file_path,
        repo_name,
    ) {
        Ok(entities) => entities,
        Err(e) => {
            tracing::warn!("Failed to parse Groovy file {}: {}", file_path, e);
            vec![]
        }
    };

    // Extract package declaration
    let package = extract_package(source);

    // Keep track of lines where entities were found to avoid duplicates
    let mut known_lines = std::collections::HashSet::new();

    // Post-process entities from Tree-sitter
    for entity in entities.iter_mut() {
        known_lines.insert(entity.start_line);

        // Fix: tree-sitter-groovy parses `trait` as `class_declaration`.
        if entity.kind == EntityKind::GroovyClass
            && let Some(line_content) = source.lines().nth(entity.start_line.saturating_sub(1))
        {
            let trimmed = line_content.trim();
            if trimmed.starts_with("trait ") || trimmed.contains(" trait ") {
                entity.kind = EntityKind::GroovyTrait;
            }
        }

        // Set FQN for tree-sitter entities
        if let Some(pkg) = &package {
            entity.fqn = match entity.kind {
                EntityKind::GroovyClass
                | EntityKind::GroovyInterface
                | EntityKind::GroovyTrait
                | EntityKind::GroovyEnum => format!("{}.{}", pkg, entity.name),
                _ => continue,
            };
        }
    }

    // Ad-hoc extraction with scope tracking for enclosing_class
    // Scope stack: (name, brace_count_when_entered)
    let mut scope_stack: Vec<(String, usize)> = Vec::new();
    let mut brace_count = 0usize;

    for (line_idx, line) in source.lines().enumerate() {
        let line_num = line_idx + 1;
        let trimmed = line.trim();

        // Track braces for scope
        brace_count += trimmed.matches('{').count();
        brace_count = brace_count.saturating_sub(trimmed.matches('}').count());

        // Pop scopes whose braces have closed
        while let Some((_, entry_brace)) = scope_stack.last() {
            if brace_count < *entry_brace {
                scope_stack.pop();
            } else {
                break;
            }
        }

        if trimmed.is_empty() || trimmed.starts_with("//") || trimmed.starts_with("/*") {
            continue;
        }

        // Try to extract class/interface/enum/trait if tree-sitter missed it
        if let Some((name, kind)) = try_extract_type_declaration(trimmed) {
            // Push to scope stack BEFORE brace_count is updated for the current line's `{`
            let fqn = if let Some(pkg) = &package {
                format!("{}.{}", pkg, name)
            } else {
                name.clone()
            };
            let current_brace = brace_count;
            entities.push(ParsedEntity::new(
                &name, kind, &fqn, None, None, "groovy", file_path, line_num, line_num, None,
                repo_name,
            ));
            scope_stack.push((name, current_brace));
        }

        // Re-read enclosing after potential scope push
        let enclosing = scope_stack.last().map(|(n, _)| n.clone());

        // Try to find a `def` method declaration
        if let Some((method_name, signature)) = try_extract_def_method(trimmed) {
            let fqn = build_fqn(&package, &enclosing, &method_name);
            entities.push(ParsedEntity::new(
                &method_name,
                EntityKind::GroovyMethod,
                &fqn,
                Some(signature),
                None,
                "groovy",
                file_path,
                line_num,
                line_num,
                enclosing,
                repo_name,
            ));
            continue;
        }

        // Try to find typed methods or script-level methods missed by tree-sitter
        if let Some((method_name, signature)) = try_extract_typed_method(trimmed) {
            // Filter false positives: method names that contain dots or look like object.method()
            if method_name.contains('.')
                || method_name.chars().all(|c| c.is_uppercase() || c == '_')
            {
                continue;
            }
            let fqn = build_fqn(&package, &enclosing, &method_name);
            entities.push(ParsedEntity::new(
                &method_name,
                EntityKind::GroovyMethod,
                &fqn,
                Some(signature),
                None,
                "groovy",
                file_path,
                line_num,
                line_num,
                enclosing,
                repo_name,
            ));
            continue;
        }

        // Try to extract properties or script-level variables
        if let Some(prop_name) = try_extract_property(trimmed) {
            let fqn = build_fqn(&package, &enclosing, &prop_name);
            entities.push(ParsedEntity::new(
                &prop_name,
                EntityKind::GroovyProperty,
                &fqn,
                None,
                None,
                "groovy",
                file_path,
                line_num,
                line_num,
                enclosing,
                repo_name,
            ));
        }
    }

    entities
}

/// Extract package name from source (e.g., `package com.example.service`)
fn extract_package(source: &str) -> Option<String> {
    for line in source.lines().take(20) {
        let trimmed = line.trim();
        if let Some(pkg) = trimmed.strip_prefix("package ") {
            let name = pkg.trim().trim_end_matches(';').trim();
            if !name.is_empty() {
                return Some(name.to_string());
            }
        }
    }
    None
}

/// Build a fully-qualified name: package.parent.method or package.method
fn build_fqn(package: &Option<String>, parent: &Option<String>, name: &str) -> String {
    match (package, parent) {
        (Some(pkg), Some(enclosing_class)) => format!("{}.{}.{}", pkg, enclosing_class, name),
        (Some(pkg), None) => format!("{}.{}", pkg, name),
        (None, Some(enclosing_class)) => format!("{}.{}", enclosing_class, name),
        (None, None) => name.to_string(),
    }
}

/// Tries to extract class, interface, enum, or trait declarations
fn try_extract_type_declaration(line: &str) -> Option<(String, EntityKind)> {
    let tokens: Vec<&str> = line.split_whitespace().collect();

    for (i, token) in tokens.iter().enumerate() {
        let kind = match *token {
            "class" => EntityKind::GroovyClass,
            "interface" => EntityKind::GroovyInterface,
            "trait" => EntityKind::GroovyTrait,
            "enum" => EntityKind::GroovyEnum,
            _ => continue,
        };

        if i + 1 < tokens.len() {
            // The next token should be the name
            let name_raw = tokens[i + 1];
            // Remove generic types, extends, implements, curly braces
            let name = name_raw
                .split('<')
                .next()
                .unwrap_or(name_raw)
                .split('{')
                .next()
                .unwrap_or(name_raw)
                .trim();

            if !name.is_empty() && name.chars().next().unwrap().is_alphabetic() {
                return Some((name.to_string(), kind));
            }
        }
    }
    None
}

/// Tries to extract properties (fields, script variables)
fn try_extract_property(line: &str) -> Option<String> {
    // A very basic heuristic for `Type name = ...` or `def name = ...`
    if let Some(eq_idx) = line.find('=') {
        let left_side = line[..eq_idx].trim();
        // Discard things like `a == b` or assignments in methods
        if left_side.is_empty() || line.chars().nth(eq_idx + 1) == Some('=') {
            return None;
        }

        let tokens: Vec<&str> = left_side.split_whitespace().collect();
        if tokens.len() >= 2 {
            let name = tokens.last().unwrap();
            let first_char = name.chars().next().unwrap();
            // Must start with letter/underscore and not contain weird chars
            if (first_char.is_alphabetic() || first_char == '_')
                && name.chars().all(|c| c.is_alphanumeric() || c == '_')
            {
                // Ignore if it looks like a method signature or control structure
                if !line.contains("if ") && !line.contains("while ") && !line.contains("for ") {
                    return Some(name.to_string());
                }
            }
        }
    }
    None
}

/// Tries to extract a typed method name and signature
fn try_extract_typed_method(line: &str) -> Option<(String, String)> {
    // Quick heuristic: contains `(` and `)` and `{`, doesn't start with `if`/`while`/`for`/`catch`
    if line.contains('(') && line.contains(')') && (line.contains('{') || line.ends_with(')')) {
        if line.starts_with("if ")
            || line.starts_with("while ")
            || line.starts_with("for ")
            || line.starts_with("catch ")
            || line.starts_with("switch ")
        {
            return None;
        }

        let paren_idx = line.find('(').unwrap();
        let before_paren = line[..paren_idx].trim();

        // Handle quoted method names (Spock feature methods)
        if let Some(quote_idx) = before_paren.find('\"') {
            // Find the closing quote
            if let Some(close_idx) = before_paren[quote_idx + 1..].find('\"') {
                let inner_name = &before_paren[quote_idx + 1..quote_idx + 1 + close_idx];
                let sig_end = line.find('{').unwrap_or(line.len());
                let signature = line[..sig_end].trim().to_string();
                return Some((inner_name.to_string(), signature));
            }
        }

        let tokens: Vec<&str> = before_paren.split_whitespace().collect();
        if tokens.len() >= 2 {
            let name = tokens.last().unwrap();
            let first_char = name.chars().next().unwrap();
            if first_char.is_alphabetic() || first_char == '_' {
                let sig_end = line.find('{').unwrap_or(line.len());
                let signature = line[..sig_end].trim().to_string();
                return Some((name.to_string(), signature));
            }
        }
    }
    None
}

/// Tries to extract a method name and signature from a line containing `def`
fn try_extract_def_method(line: &str) -> Option<(String, String)> {
    // Look for `def `
    if let Some(def_idx) = line.find("def ") {
        // Ensure `def` is a word by checking the preceding character (if any)
        if def_idx > 0 {
            let prev_char = line.as_bytes()[def_idx - 1] as char;
            if prev_char.is_alphanumeric() || prev_char == '_' {
                return None;
            }
        }

        let after_def = &line[def_idx + 4..].trim_start();

        // Find the opening parenthesis for the method arguments
        if let Some(paren_idx) = after_def.find('(') {
            let potential_name = &after_def[..paren_idx].trim();

            // Validate the name: must be a valid identifier and not contain spaces
            if !potential_name.is_empty() && !potential_name.contains(|c: char| c.is_whitespace()) {
                // Must start with letter or underscore
                let first_char = potential_name.chars().next().unwrap();
                if first_char.is_alphabetic() || first_char == '_' {
                    // Extract signature from 'def' to the start of the block '{' or end of line
                    let sig_end = line[def_idx..]
                        .find('{')
                        .map(|i| i + def_idx)
                        .unwrap_or(line.len());
                    let signature = line[def_idx..sig_end].trim().to_string();

                    return Some((potential_name.to_string(), signature));
                }
            }
        }
    }
    None
}

pub(crate) fn extract_entities_groovy(
    source: &str,
    file_path: &str,
    repo_name: &str,
) -> Vec<ParsedEntity> {
    let mut entities = Vec::new();

    for (line_idx, line) in source.lines().enumerate() {
        let line_num = line_idx + 1;
        let trimmed = line.trim();

        if trimmed.is_empty() || trimmed.starts_with("//") || trimmed.starts_with("/*") {
            continue;
        }

        // Gradle: implementation 'group:artifact:version'
        if let Some(dep) = try_extract_gradle_dependency(trimmed) {
            entities.push(crate::models::ParsedEntity::new(
                &dep,
                EntityKind::BuildDependency,
                &dep,
                None,
                Some(trimmed.to_string()),
                "groovy",
                file_path,
                line_num,
                line_num,
                None,
                repo_name,
            ));
            continue;
        }

        // Gradle: id 'plugin-name'
        if let Some(plugin) = try_extract_gradle_plugin(trimmed) {
            entities.push(crate::models::ParsedEntity::new(
                &plugin,
                EntityKind::BuildPlugin,
                &plugin,
                None,
                Some(trimmed.to_string()),
                "groovy",
                file_path,
                line_num,
                line_num,
                None,
                repo_name,
            ));
            continue;
        }

        // Gradle: task myTask { ... }
        if let Some(task) = try_extract_gradle_task(trimmed) {
            entities.push(crate::models::ParsedEntity::new(
                &task,
                EntityKind::BuildTask,
                &task,
                None,
                Some(trimmed.to_string()),
                "groovy",
                file_path,
                line_num,
                line_num,
                None,
                repo_name,
            ));
            continue;
        }

        // Jenkins: stage('Name')
        if let Some(stage) = try_extract_jenkins_stage(trimmed) {
            entities.push(crate::models::ParsedEntity::new(
                &stage,
                EntityKind::PipelineStage,
                &stage,
                None,
                Some(trimmed.to_string()),
                "groovy",
                file_path,
                line_num,
                line_num,
                None,
                repo_name,
            ));
            continue;
        }

        // Jenkins: sh 'command'
        if let Some(step) = try_extract_jenkins_step(trimmed) {
            entities.push(crate::models::ParsedEntity::new(
                &step,
                EntityKind::PipelineStep,
                &step,
                None,
                Some(trimmed.to_string()),
                "groovy",
                file_path,
                line_num,
                line_num,
                None,
                repo_name,
            ));
        }
    }

    entities
}

fn try_extract_gradle_dependency(line: &str) -> Option<String> {
    let line = line.trim();
    if !line.contains('\'') {
        return None;
    }

    let configs = [
        "implementation",
        "api",
        "compileOnly",
        "runtimeOnly",
        "testImplementation",
        "testCompileOnly",
        "testRuntimeOnly",
        "annotationProcessor",
        "kapt",
    ];

    for config in &configs {
        if let Some(rest) = line.strip_prefix(config) {
            let rest = rest.trim();
            if let Some(lib) = extract_single_quoted(rest) {
                return Some(format!("{}:{}", config, lib));
            }
        }
    }
    None
}

fn try_extract_gradle_plugin(line: &str) -> Option<String> {
    let line = line.trim();
    if !line.starts_with("id ") {
        return None;
    }
    let rest = &line["id ".len()..].trim();
    extract_single_quoted(rest)
}

fn try_extract_gradle_task(line: &str) -> Option<String> {
    let name = line.trim().strip_prefix("task ")?;
    let task_name: String = name
        .chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_')
        .collect();
    if task_name.is_empty() {
        None
    } else {
        Some(task_name)
    }
}

fn try_extract_jenkins_stage(line: &str) -> Option<String> {
    let line = line.trim();
    if let Some(rest) = line.strip_prefix("stage(")
        && let Some(name) = extract_single_quoted(rest)
    {
        return Some(format!("stage: {}", name));
    }
    None
}

fn try_extract_jenkins_step(line: &str) -> Option<String> {
    let line = line.trim();
    let commands = ["sh", "bat", "echo", "dir", "checkout", "withCredentials"];

    for cmd in &commands {
        // Match `cmd 'arg'` pattern
        let prefix = format!("{} ", cmd);
        if let Some(rest) = line.strip_prefix(&prefix) {
            let rest = rest.trim();
            // Try single-quoted argument first
            if let Some(quoted) = extract_single_quoted(rest) {
                return Some(format!("{}: {}", cmd, quoted));
            }
            // Fallback: take first word as argument (e.g., checkout scm)
            let arg: String = rest
                .chars()
                .take_while(|c| !c.is_whitespace() && *c != '{' && *c != '(')
                .collect();
            if !arg.is_empty() {
                return Some(format!("{}: {}", cmd, arg));
            }
        }

        // Match `cmd(...)` pattern (e.g., dir('reports'), withCredentials([...]))
        let prefix_paren = format!("{}(", cmd);
        if let Some(rest) = line.strip_prefix(&prefix_paren) {
            let truncated: String = rest.chars().take(60).collect();
            return Some(format!("{}: {}", cmd, truncated));
        }
    }
    None
}

fn extract_single_quoted(s: &str) -> Option<String> {
    let s = s.trim();
    if let Some(inner) = s.strip_prefix('\'')
        && let Some(end) = inner.find('\'')
    {
        return Some(inner[..end].to_string());
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_gradle_implementation_dependency() {
        let line = "implementation 'org.springframework.boot:spring-boot-starter-web:2.7.14'";
        let result = try_extract_gradle_dependency(line);
        assert!(result.is_some());
        assert_eq!(
            result.unwrap(),
            "implementation:org.springframework.boot:spring-boot-starter-web:2.7.14"
        );
    }

    #[test]
    fn test_extract_gradle_api_dependency() {
        let line = "api 'org.apache.commons:commons-lang3:3.13.0'";
        let result = try_extract_gradle_dependency(line);
        assert!(result.is_some());
        assert_eq!(
            result.unwrap(),
            "api:org.apache.commons:commons-lang3:3.13.0"
        );
    }

    #[test]
    fn test_extract_gradle_test_implementation() {
        let line = "testImplementation 'junit:junit:4.13.2'";
        let result = try_extract_gradle_dependency(line);
        assert!(result.is_some());
        assert_eq!(result.unwrap(), "testImplementation:junit:junit:4.13.2");
    }

    #[test]
    fn test_extract_gradle_compile_only() {
        let line = "compileOnly 'org.projectlombok:lombok:1.18.30'";
        let result = try_extract_gradle_dependency(line);
        assert!(result.is_some());
        assert_eq!(
            result.unwrap(),
            "compileOnly:org.projectlombok:lombok:1.18.30"
        );
    }

    #[test]
    fn test_extract_gradle_annotation_processor() {
        let line = "annotationProcessor 'org.projectlombok:lombok:1.18.30'";
        let result = try_extract_gradle_dependency(line);
        assert!(result.is_some());
        assert_eq!(
            result.unwrap(),
            "annotationProcessor:org.projectlombok:lombok:1.18.30"
        );
    }

    #[test]
    fn test_extract_gradle_runtime_only() {
        let line = "testRuntimeOnly 'org.junit.vintage:junit-vintage-engine:5.10.0'";
        let result = try_extract_gradle_dependency(line);
        assert!(result.is_some());
        assert_eq!(
            result.unwrap(),
            "testRuntimeOnly:org.junit.vintage:junit-vintage-engine:5.10.0"
        );
    }

    #[test]
    fn test_not_a_dependency_line() {
        assert!(try_extract_gradle_dependency("group = 'com.example'").is_none());
        assert!(try_extract_gradle_dependency("version = '1.0.0'").is_none());
        assert!(try_extract_gradle_dependency("repositories {").is_none());
        assert!(try_extract_gradle_dependency("mavenCentral()").is_none());
    }

    #[test]
    fn test_extract_gradle_plugin() {
        let result = try_extract_gradle_plugin("id 'java'");
        assert_eq!(result, Some("java".to_string()));

        let result = try_extract_gradle_plugin("id 'org.springframework.boot' version '2.7.14'");
        assert_eq!(result, Some("org.springframework.boot".to_string()));
    }

    #[test]
    fn test_extract_gradle_task() {
        let result = try_extract_gradle_task("task buildDocs(type: Copy) {");
        assert_eq!(result, Some("buildDocs".to_string()));

        let result = try_extract_gradle_task("task deployToServer {");
        assert_eq!(result, Some("deployToServer".to_string()));
    }

    #[test]
    fn test_extract_single_quoted_helper() {
        assert_eq!(
            extract_single_quoted("'hello world' rest"),
            Some("hello world".to_string())
        );
        assert_eq!(
            extract_single_quoted("'single'"),
            Some("single".to_string())
        );
        assert_eq!(extract_single_quoted("no quotes"), None);
        assert_eq!(extract_single_quoted(""), None);
    }

    #[test]
    fn test_full_gradle_extraction() {
        let source = r#"plugins {
    id 'java'
}
dependencies {
    implementation 'org.springframework.boot:spring-boot-starter-web:2.7.14'
    testImplementation 'junit:junit:4.13.2'
}
task myTask {
    println 'hello'
}"#;
        let entities = extract_entities_groovy(source, "build.gradle", "test-repo");

        assert_eq!(entities.len(), 4);

        // Plugin
        let plugin = entities.iter().find(|e| e.kind == EntityKind::BuildPlugin);
        assert!(plugin.is_some());
        assert_eq!(plugin.unwrap().name, "java");

        // Dependencies
        let deps: Vec<_> = entities
            .iter()
            .filter(|e| e.kind == EntityKind::BuildDependency)
            .collect();
        assert_eq!(deps.len(), 2);

        // Task
        let task = entities.iter().find(|e| e.kind == EntityKind::BuildTask);
        assert!(task.is_some());
        assert_eq!(task.unwrap().name, "myTask");

        // Repo name preserved
        assert!(entities.iter().all(|e| e.repo_name == "test-repo"));
    }

    #[test]
    fn test_groovy_comments_are_skipped() {
        let source = r#"// This is a comment
dependencies {
    // implementation 'commented:out:1.0.0'
    implementation 'org.example:real-dep:1.0.0'
}
/*
 * Block comment
 * implementation 'org.example:ignored:2.0.0'
 */"#;
        let entities = extract_entities_groovy(source, "build.gradle", "test-repo");

        // Only the real dependency should be extracted (not commented ones)
        let deps: Vec<_> = entities
            .iter()
            .filter(|e| e.kind == EntityKind::BuildDependency)
            .collect();
        assert_eq!(deps.len(), 1);
        assert_eq!(deps[0].name, "implementation:org.example:real-dep:1.0.0");
    }

    // ── Jenkins Pipeline Tests ──

    #[test]
    fn test_extract_jenkins_stage_simple() {
        let line = "stage('Build') {";
        let result = try_extract_jenkins_stage(line);
        assert_eq!(result, Some("stage: Build".to_string()));
    }

    #[test]
    fn test_extract_jenkins_stage_with_spaces() {
        let line = "stage('Deploy to Production') {";
        let result = try_extract_jenkins_stage(line);
        assert_eq!(result, Some("stage: Deploy to Production".to_string()));
    }

    #[test]
    fn test_extract_jenkins_step_sh() {
        let line = "sh 'mvn clean compile'";
        let result = try_extract_jenkins_step(line);
        assert_eq!(result, Some("sh: mvn clean compile".to_string()));
    }

    #[test]
    fn test_extract_jenkins_step_echo() {
        let line = "echo 'Build completed'";
        let result = try_extract_jenkins_step(line);
        assert_eq!(result, Some("echo: Build completed".to_string()));
    }

    #[test]
    fn test_extract_jenkins_step_dir() {
        let line = "dir('reports') {";
        let result = try_extract_jenkins_step(line);
        assert!(result.is_some());
        assert!(result.unwrap().starts_with("dir:"));
    }

    #[test]
    fn test_extract_jenkins_step_checkout() {
        let line = "checkout scm";
        let result = try_extract_jenkins_step(line);
        assert_eq!(result, Some("checkout: scm".to_string()));
    }

    #[test]
    fn test_extract_jenkins_step_with_credentials() {
        let line = "withCredentials([string(credentialsId: 'prod-token', variable: 'TOKEN')]) {";
        let result = try_extract_jenkins_step(line);
        assert!(result.is_some());
        assert!(result.unwrap().starts_with("withCredentials:"));
    }

    #[test]
    fn test_not_a_jenkins_step() {
        assert!(try_extract_jenkins_step("node {").is_none());
        assert!(try_extract_jenkins_step("pipeline {").is_none());
        assert!(try_extract_jenkins_step("stages {").is_none());
        assert!(try_extract_jenkins_step("steps {").is_none());
    }

    #[test]
    fn test_full_jenkinsfile_extraction() {
        let source = r#"pipeline {
    stages {
        stage('Checkout') {
            steps {
                checkout scm
                echo 'Repository cloned'
            }
        }
        stage('Build') {
            steps {
                sh 'mvn compile'
                echo 'Build done'
            }
        }
        stage('Deploy') {
            steps {
                sh 'deploy.sh'
            }
        }
    }
}"#;
        let entities = extract_entities_groovy(source, "Jenkinsfile", "test-repo");

        // All entities: 3 stages + 5 steps = 8
        let stages: Vec<_> = entities
            .iter()
            .filter(|e| e.kind == EntityKind::PipelineStage)
            .collect();
        let steps: Vec<_> = entities
            .iter()
            .filter(|e| e.kind == EntityKind::PipelineStep)
            .collect();

        assert_eq!(stages.len(), 3);
        assert!(stages.iter().any(|s| s.name == "stage: Checkout"));
        assert!(stages.iter().any(|s| s.name == "stage: Build"));
        assert!(stages.iter().any(|s| s.name == "stage: Deploy"));

        assert_eq!(steps.len(), 5);
        assert!(steps.iter().any(|s| s.name == "sh: mvn compile"));
        assert!(steps.iter().any(|s| s.name == "sh: deploy.sh"));
        assert!(steps.iter().any(|s| s.name.starts_with("echo:")));

        // Repo name preserved
        assert!(entities.iter().all(|e| e.repo_name == "test-repo"));
        // File path preserved
        assert!(entities.iter().all(|e| e.file_path == "Jenkinsfile"));
    }

    #[test]
    fn test_jenkinsfile_mixed_with_gradle_entities() {
        // Groovy files might contain both Gradle and Jenkins patterns
        // but a Jenkinsfile should ideally only have pipeline entities
        let source = r#"pipeline {
    stages {
        stage('Test') {
            steps {
                sh 'run tests'
            }
        }
    }
}"#;
        let entities = extract_entities_groovy(source, "Jenkinsfile", "test-repo");
        assert_eq!(entities.len(), 2); // One stage, one step

        assert_eq!(entities[0].kind, EntityKind::PipelineStage);
        assert_eq!(entities[0].name, "stage: Test");
        assert_eq!(entities[1].kind, EntityKind::PipelineStep);
        assert_eq!(entities[1].name, "sh: run tests");
    }

    // ---- Groovy Standard (tree-sitter) extraction tests ----

    #[test]
    fn test_groovy_class_extraction() {
        let source = "class MyGroovyClass { def method() {} }";
        let entities = extract_entities_groovy_standard(source, "test.groovy", "test-repo");
        assert!(
            entities
                .iter()
                .any(|e| e.name == "MyGroovyClass" && e.kind == EntityKind::GroovyClass)
        );
    }

    #[test]
    fn test_groovy_interface_extraction() {
        let source = "interface MyGroovyInterface { void doIt() }";
        let entities = extract_entities_groovy_standard(source, "test.groovy", "test-repo");
        assert!(
            entities
                .iter()
                .any(|e| e.name == "MyGroovyInterface" && e.kind == EntityKind::GroovyInterface)
        );
    }

    #[test]
    fn test_groovy_enum_extraction() {
        let source = "enum Color { RED, GREEN, BLUE }";
        let entities = extract_entities_groovy_standard(source, "test.groovy", "test-repo");
        assert!(
            entities
                .iter()
                .any(|e| e.name == "Color" && e.kind == EntityKind::GroovyEnum)
        );
    }

    #[test]
    fn test_groovy_method_extraction() {
        let source = "class Foo { String greet(String name) { return name } }";
        let entities = extract_entities_groovy_standard(source, "test.groovy", "test-repo");
        let method = entities.iter().find(|e| e.name == "greet");
        assert!(method.is_some(), "Expected method 'greet' to be extracted");
        assert_eq!(method.unwrap().kind, EntityKind::GroovyMethod);
    }

    #[test]
    fn test_groovy_trait_extraction() {
        let source = "trait MyTrait { void doSomething() {} }";
        let entities = extract_entities_groovy_standard(source, "test.groovy", "test-repo");
        assert!(
            entities
                .iter()
                .any(|e| e.name == "MyTrait" && e.kind == EntityKind::GroovyTrait)
        );
    }

    #[test]
    fn test_groovy_property_extraction() {
        let source = "class Foo { String name = 'test' }";
        let entities = extract_entities_groovy_standard(source, "test.groovy", "test-repo");
        assert!(
            entities
                .iter()
                .any(|e| e.name == "name" && e.kind == EntityKind::GroovyProperty)
        );
    }

    #[test]
    fn test_groovy_multiple_classes() {
        let source = "package com.example\nclass First {}\nclass Second {}\nclass Third {}";
        let entities = extract_entities_groovy_standard(source, "test.groovy", "test-repo");
        let class_names: Vec<_> = entities
            .iter()
            .filter(|e| e.kind == EntityKind::GroovyClass)
            .map(|e| e.name.clone())
            .collect();
        assert!(class_names.contains(&"First".to_string()));
        assert!(class_names.contains(&"Second".to_string()));
        assert!(class_names.contains(&"Third".to_string()));
    }

    #[test]
    fn test_groovy_constructor_extraction() {
        let source = "class User { User(String name) { this.name = name } }";
        let entities = extract_entities_groovy_standard(source, "test.groovy", "test-repo");
        assert!(
            entities
                .iter()
                .any(|e| e.name == "User" && e.kind == EntityKind::GroovyMethod)
        );
    }

    #[test]
    fn test_groovy_empty_body_class() {
        let source = "class EmptyClass {}";
        let entities = extract_entities_groovy_standard(source, "test.groovy", "test-repo");
        assert!(
            entities
                .iter()
                .any(|e| e.name == "EmptyClass" && e.kind == EntityKind::GroovyClass)
        );
    }

    #[test]
    fn test_groovy_method_in_class_extracts_correctly() {
        let source = "class Calculator {\n  int add(int a, int b) { return a + b }\n  int subtract(int a, int b) { return a - b }\n}";
        let entities = extract_entities_groovy_standard(source, "test.groovy", "test-repo");
        assert!(
            entities
                .iter()
                .any(|e| e.name == "add" && e.kind == EntityKind::GroovyMethod)
        );
        assert!(
            entities
                .iter()
                .any(|e| e.name == "subtract" && e.kind == EntityKind::GroovyMethod)
        );
    }

    #[test]
    fn test_groovy_parse_sample_full_file() {
        let source = include_str!("../../../../tests/testing_files/sample_full.groovy");
        let entities = extract_entities_groovy_standard(source, "sample_full.groovy", "test-repo");

        println!("--- Extracted Entities ---");
        for e in &entities {
            println!("{:?} - {}", e.kind, e.name);
        }
        println!("--------------------------");

        assert!(
            entities
                .iter()
                .any(|e| e.name == "UserService" && e.kind == EntityKind::GroovyClass)
        );
        assert!(
            entities
                .iter()
                .any(|e| e.name == "BaseService" && e.kind == EntityKind::GroovyClass)
        );
        assert!(
            entities
                .iter()
                .any(|e| e.name == "DatabaseConfig" && e.kind == EntityKind::GroovyClass)
        );
        assert!(
            entities
                .iter()
                .any(|e| e.name == "Repository" && e.kind == EntityKind::GroovyInterface)
        );
        assert!(
            entities
                .iter()
                .any(|e| e.name == "Auditable" && e.kind == EntityKind::GroovyTrait)
        );
        assert!(
            entities
                .iter()
                .any(|e| e.name == "Status" && e.kind == EntityKind::GroovyEnum)
        );
        assert!(
            entities
                .iter()
                .any(|e| e.name == "scriptMethod" && e.kind == EntityKind::GroovyMethod)
        );
        assert!(
            entities
                .iter()
                .any(|e| e.name == "anotherScriptMethod" && e.kind == EntityKind::GroovyMethod)
        );
        assert!(
            entities
                .iter()
                .any(|e| e.name == "globalConfig" && e.kind == EntityKind::GroovyProperty)
        );
        assert!(
            entities
                .iter()
                .any(|e| e.name == "processDataClosure" && e.kind == EntityKind::GroovyProperty)
        );
        assert!(
            entities
                .iter()
                .any(|e| e.name == "initialize" && e.kind == EntityKind::GroovyMethod)
        );
        assert!(
            entities
                .iter()
                .any(|e| e.name == "calculateTotal" && e.kind == EntityKind::GroovyMethod)
        );
        assert!(
            entities
                .iter()
                .any(|e| e.name == "logAction" && e.kind == EntityKind::GroovyMethod)
        );
        assert!(entities.iter().any(|e| e.name
            == "addition of #num1 and #num2 should be #expected"
            && e.kind == EntityKind::GroovyMethod));
        assert!(
            entities
                .iter()
                .any(|e| e.name == "DEFAULT_ROLE" && e.kind == EntityKind::GroovyProperty)
        );
        assert!(
            entities
                .iter()
                .any(|e| e.name == "maxLoginAttempts" && e.kind == EntityKind::GroovyProperty)
        );

        assert!(
            entities.len() >= 20,
            "Expected at least 20 entities, got {}",
            entities.len()
        );
    }

    #[test]
    fn test_groovy_fqn_with_package() {
        let source = "package com.acme.app\nclass MyService { String greet(String name) { name } }";
        let entities = extract_entities_groovy_standard(source, "test.groovy", "test-repo");

        let class_entity = entities
            .iter()
            .find(|e| e.name == "MyService")
            .expect("MyService class not extracted");
        assert_eq!(class_entity.fqn, "com.acme.app.MyService");

        let method_entity = entities
            .iter()
            .find(|e| e.name == "greet")
            .expect("greet method not extracted");
        assert_eq!(method_entity.fqn, "com.acme.app.MyService.greet");
        assert_eq!(method_entity.enclosing_class.as_deref(), Some("MyService"));
    }

    #[test]
    fn test_groovy_method_parent_class() {
        let source = "class Calculator {\n  int add(int a, int b) { a + b }\n  def multiply(int x, int y) { x * y }\n}";
        let entities = extract_entities_groovy_standard(source, "test.groovy", "test-repo");

        let add_method = entities
            .iter()
            .find(|e| e.name == "add")
            .expect("add method not extracted");
        assert_eq!(add_method.enclosing_class.as_deref(), Some("Calculator"));
        assert_eq!(add_method.fqn, "Calculator.add");

        let multiply_method = entities
            .iter()
            .find(|e| e.name == "multiply")
            .expect("multiply method not extracted");
        assert_eq!(
            multiply_method.enclosing_class.as_deref(),
            Some("Calculator")
        );
        assert_eq!(multiply_method.fqn, "Calculator.multiply");
    }

    #[test]
    fn test_groovy_interface_method_has_parent() {
        let source = "interface Repository {\n  String findById(String id)\n}";
        let entities = extract_entities_groovy_standard(source, "test.groovy", "test-repo");

        let method = entities
            .iter()
            .find(|e| e.name == "findById")
            .expect("findById not extracted");
        assert_eq!(method.enclosing_class.as_deref(), Some("Repository"));
    }

    #[test]
    fn test_groovy_nested_scope_tracking() {
        let source = "class Outer {\n  class Inner {\n    String getValue() { 'val' }\n  }\n}";
        let entities = extract_entities_groovy_standard(source, "test.groovy", "test-repo");

        let outer = entities
            .iter()
            .find(|e| e.name == "Outer")
            .expect("Outer class not extracted");
        assert_eq!(outer.kind, EntityKind::GroovyClass);

        let inner = entities
            .iter()
            .find(|e| e.name == "Inner")
            .expect("Inner class not extracted");
        assert_eq!(inner.kind, EntityKind::GroovyClass);

        let method = entities
            .iter()
            .find(|e| e.name == "getValue")
            .expect("getValue method not extracted");
        assert_eq!(method.enclosing_class.as_deref(), Some("Inner"));
        assert_eq!(method.fqn, "Inner.getValue");
    }

    #[test]
    fn test_groovy_trait_method_has_parent() {
        let source = "trait Auditable {\n  def logAction(String msg) { println msg }\n}";
        let entities = extract_entities_groovy_standard(source, "test.groovy", "test-repo");

        let method = entities
            .iter()
            .find(|e| e.name == "logAction")
            .expect("logAction not extracted");
        assert_eq!(method.enclosing_class.as_deref(), Some("Auditable"));
        assert_eq!(method.fqn, "Auditable.logAction");
    }

    #[test]
    fn test_groovy_resilience_empty_file() {
        let entities = extract_entities_groovy_standard("", "test.groovy", "test-repo");
        assert!(entities.is_empty());
    }

    #[test]
    fn test_groovy_resilience_malformed() {
        let source = "garbage {{{ // not valid groovy\nclass ";
        let entities = extract_entities_groovy_standard(source, "test.groovy", "test-repo");
        // Should not panic, just return what it can (likely empty)
        assert!(
            entities.len() <= 2,
            "Expected at most 2 entities from malformed code"
        );
    }

    #[test]
    fn test_groovy_resilience_missing_braces() {
        let source =
            "class Broken {\n  def method1() { }\n  def method2() { }\n// no closing brace";
        let entities = extract_entities_groovy_standard(source, "test.groovy", "test-repo");
        // Should extract what it can without panicking
        assert!(entities.iter().any(|e| e.name == "Broken"));
        assert!(entities.iter().any(|e| e.name == "method1"));
        assert!(entities.iter().any(|e| e.name == "method2"));
    }
}
