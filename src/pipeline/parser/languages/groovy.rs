use crate::models::{EntityKind, ParsedEntity};

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
}
