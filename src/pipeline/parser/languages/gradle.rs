use crate::models::{EntityKind, ParsedEntity};
pub(crate) use crate::pipeline::parser::utils::extract_single_quoted;

pub(crate) fn extract_entities_gradle(
    source: &str,
    file_path: &str,
    repo_name: &str,
) -> Vec<ParsedEntity> {
    let mut entities = Vec::new();

    let group_id = extract_gradle_group(source).unwrap_or_else(|| "unknown".to_string());
    let artifact_id =
        extract_gradle_artifact_name(file_path).unwrap_or_else(|| "unknown".to_string());
    let version = extract_gradle_version(source).unwrap_or_else(|| "unknown".to_string());

    let fqn = format!("gradle:{}:{}", group_id, artifact_id);
    entities.push(ParsedEntity::new(
        format!("{}:{}", group_id, artifact_id),
        EntityKind::ProjectIdentity,
        &fqn,
        Some(format!("version: {}, build_system: gradle", version)),
        Some(format!(
            "Gradle project identity: {}:{}",
            group_id, artifact_id
        )),
        "gradle",
        file_path,
        1,
        1,
        None,
        repo_name,
    ));

    for (line_idx, line) in source.lines().enumerate() {
        let line_num = line_idx + 1;
        let trimmed = line.trim();

        if trimmed.is_empty() || trimmed.starts_with("//") || trimmed.starts_with("/*") {
            continue;
        }

        if let Some(dep) = try_extract_gradle_dependency(trimmed) {
            entities.push(ParsedEntity::new(
                &dep,
                EntityKind::BuildDependency,
                &dep,
                None,
                Some(trimmed.to_string()),
                "gradle",
                file_path,
                line_num,
                line_num,
                None,
                repo_name,
            ));
            continue;
        }

        if let Some(plugin) = try_extract_gradle_plugin(trimmed) {
            entities.push(ParsedEntity::new(
                &plugin,
                EntityKind::BuildPlugin,
                &plugin,
                None,
                Some(trimmed.to_string()),
                "gradle",
                file_path,
                line_num,
                line_num,
                None,
                repo_name,
            ));
            continue;
        }

        if let Some(task) = try_extract_gradle_task(trimmed) {
            entities.push(ParsedEntity::new(
                &task,
                EntityKind::BuildTask,
                &task,
                None,
                Some(trimmed.to_string()),
                "gradle",
                file_path,
                line_num,
                line_num,
                None,
                repo_name,
            ));
            continue;
        }
    }

    entities
}

fn extract_gradle_group(source: &str) -> Option<String> {
    for line in source.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("group")
            && let Some(after_keyword) = trimmed.strip_prefix("group")
        {
            let rest = after_keyword.trim_start_matches([' ', '=']).trim();
            if let Some(quoted) = extract_single_quoted(rest) {
                return Some(quoted);
            }
        }
    }
    None
}

fn extract_gradle_version(source: &str) -> Option<String> {
    for line in source.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("version")
            && let Some(after_keyword) = trimmed.strip_prefix("version")
        {
            let rest = after_keyword.trim_start_matches([' ', '=']).trim();
            if let Some(quoted) = extract_single_quoted(rest) {
                return Some(quoted);
            }
        }
    }
    None
}

fn extract_gradle_artifact_name(file_path: &str) -> Option<String> {
    use std::path::Path;
    let path = Path::new(file_path);
    path.parent()
        .and_then(|p| p.file_name())
        .and_then(|n| n.to_str())
        .map(|s| s.to_string())
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
        let entities = extract_entities_gradle(source, "build.gradle", "test-repo");

        assert_eq!(entities.len(), 5);

        // Project identity
        let proj_id = entities
            .iter()
            .find(|e| e.kind == EntityKind::ProjectIdentity);
        assert!(proj_id.is_some());

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
    fn test_gradle_project_identity_extraction() {
        let source = r#"group = 'com.example'
version = '1.0.0'
plugins {
    id 'java'
}
dependencies {
    implementation 'org.springframework.boot:spring-boot-starter-web:2.7.14'
}"#;
        let entities = extract_entities_gradle(source, "/tmp/my-app/build.gradle", "test-repo");

        let proj_ids: Vec<_> = entities
            .iter()
            .filter(|e| e.kind == EntityKind::ProjectIdentity)
            .collect();
        assert_eq!(proj_ids.len(), 1);
        assert_eq!(proj_ids[0].name, "com.example:my-app");
        assert_eq!(proj_ids[0].fqn, "gradle:com.example:my-app");
        assert!(
            proj_ids[0]
                .signature
                .as_ref()
                .unwrap()
                .contains("build_system: gradle")
        );
        assert!(
            proj_ids[0]
                .signature
                .as_ref()
                .unwrap()
                .contains("version: 1.0.0")
        );
    }

    #[test]
    fn test_gradle_project_identity_group_with_equals() {
        let source = r#"group='com.acme'
version='2.5.0'
dependencies {
    implementation 'com.example:dep:1.0'
}"#;
        let entities = extract_entities_gradle(source, "/tmp/acme-lib/build.gradle", "test-repo");

        let proj_ids: Vec<_> = entities
            .iter()
            .filter(|e| e.kind == EntityKind::ProjectIdentity)
            .collect();
        assert_eq!(proj_ids.len(), 1);
        assert_eq!(proj_ids[0].name, "com.acme:acme-lib");
        assert_eq!(proj_ids[0].fqn, "gradle:com.acme:acme-lib");
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
        let entities = extract_entities_gradle(source, "build.gradle", "test-repo");

        // Only the real dependency should be extracted (not commented ones)
        let deps: Vec<_> = entities
            .iter()
            .filter(|e| e.kind == EntityKind::BuildDependency)
            .collect();
        assert_eq!(deps.len(), 1);
        assert_eq!(deps[0].name, "implementation:org.example:real-dep:1.0.0");
    }
}
