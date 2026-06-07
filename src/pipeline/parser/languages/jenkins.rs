use crate::models::{EntityKind, ParsedEntity};
pub(crate) use crate::pipeline::parser::utils::extract_single_quoted;

pub(crate) fn extract_entities_jenkins(
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

        if let Some(stage) = try_extract_jenkins_stage(trimmed) {
            entities.push(ParsedEntity::new(
                &stage,
                EntityKind::PipelineStage,
                &stage,
                None,
                Some(trimmed.to_string()),
                "jenkinsfile",
                file_path,
                line_num,
                line_num,
                None,
                repo_name,
            ));
            continue;
        }

        if let Some(step) = try_extract_jenkins_step(trimmed) {
            entities.push(ParsedEntity::new(
                &step,
                EntityKind::PipelineStep,
                &step,
                None,
                Some(trimmed.to_string()),
                "jenkinsfile",
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

#[cfg(test)]
mod tests {
    use super::*;

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
        let entities = extract_entities_jenkins(source, "Jenkinsfile", "test-repo");

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
        // A Jenkinsfile should ideally only have pipeline entities
        let source = r#"pipeline {
    stages {
        stage('Test') {
            steps {
                sh 'run tests'
            }
        }
    }
}"#;
        let entities = extract_entities_jenkins(source, "Jenkinsfile", "test-repo");
        assert_eq!(entities.len(), 2); // One stage, one step

        assert_eq!(entities[0].kind, EntityKind::PipelineStage);
        assert_eq!(entities[0].name, "stage: Test");
        assert_eq!(entities[1].kind, EntityKind::PipelineStep);
        assert_eq!(entities[1].name, "sh: run tests");
    }
}
