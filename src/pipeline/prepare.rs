//! Stage 3 — Prepare: UUID assignment and embedding text construction.
//!
//! Each [`ParsedEntity`] already carries a UUID generated at construction time
//! (see [`ParsedEntity::new`]). This stage's responsibility is to build the
//! `embed_text` field — the string that will be fed to the embedding model.
//!
//! # Embedding text format
//! ```text
//! [<KIND>] <name>
//! Signature: <signature>   ← omitted when None
//! <docstring>              ← omitted when None
//! File: <file_path>:<start_line>
//! ```
//!
//! Keeping the format consistent across runs is important: the same entity
//! should always produce the same embedding so vector updates are idempotent.

use crate::models::ParsedEntity;

/// Build the `embed_text` field for every entity in-place.
///
/// This function is intentionally synchronous and allocation-cheap; it is
/// called after Rayon parsing and before the async embedding stage.
pub fn prepare_entities(entities: &mut [ParsedEntity]) {
    for entity in entities.iter_mut() {
        entity.embed_text = build_embed_text(entity);
    }
}

/// Construct the embedding text for a single entity.
fn build_embed_text(entity: &ParsedEntity) -> String {
    let mut parts: Vec<String> = Vec::with_capacity(8);

    // Header: kind + name
    parts.push(format!("[{}] {}", entity.kind, entity.name));

    // Optional decorators/annotations (framework metadata)
    if !entity.decorators.is_empty() {
        let decorators_text = entity
            .decorators
            .iter()
            .filter(|d| !d.trim().is_empty())
            .cloned()
            .collect::<Vec<_>>()
            .join(", ");

        if !decorators_text.is_empty() {
            parts.push(format!("Decorators: {}", decorators_text));
        }
    }

    // Optional signature
    if let Some(sig) = &entity.signature {
        parts.push(format!("Signature: {sig}"));
    }

    // Optional docstring (preceding comments)
    if let Some(doc) = &entity.docstring
        && !doc.trim().is_empty()
    {
        parts.push(doc.trim().to_owned());
    }

    // Inline comments found within the entity body
    if !entity.inline_comments.is_empty() {
        let inline_text = entity
            .inline_comments
            .iter()
            .filter(|c| !c.trim().is_empty())
            .cloned()
            .collect::<Vec<_>>()
            .join("\n");

        if !inline_text.trim().is_empty() {
            parts.push(format!("Implementation context:\n{}", inline_text));
        }
    }

    // Source location — helps distinguish identically-named entities
    parts.push(format!("File: {}:{}", entity.file_path, entity.start_line));

    parts.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{EntityKind, ParsedEntity};

    #[test]
    fn test_build_embed_text_minimal() {
        let entity = ParsedEntity::new(
            "MyClass",
            EntityKind::Class,
            "com.example.MyClass",
            None,
            None,
            "java",
            "com/example/MyClass.java",
            10,
            20,
            None,
            "test-repo",
        );

        let embed_text = build_embed_text(&entity);

        assert!(embed_text.contains("[class] MyClass"));
        assert!(embed_text.contains("File: com/example/MyClass.java:10"));
        assert!(!embed_text.contains("Signature:"));
        assert!(!embed_text.contains("Decorators:"));
    }

    #[test]
    fn test_build_embed_text_full() {
        let mut entity = ParsedEntity::new(
            "saveUser",
            EntityKind::Method,
            "UserService.saveUser",
            Some("public void saveUser(User user)".to_string()),
            Some("Saves a new user to the database.".to_string()),
            "java",
            "UserService.java",
            42,
            50,
            Some("UserService".to_string()),
            "test-repo",
        );

        entity.decorators = vec!["@Transactional".to_string(), "@Override".to_string()];
        entity.inline_comments = vec![
            "// Check for duplicates".to_string(),
            "/* Commit transaction */".to_string(),
        ];

        let embed_text = build_embed_text(&entity);

        assert!(embed_text.contains("[method] saveUser"));
        assert!(embed_text.contains("Signature: public void saveUser(User user)"));
        assert!(embed_text.contains("Saves a new user to the database."));
        assert!(embed_text.contains("Decorators: @Transactional, @Override"));
        assert!(embed_text.contains(
            "Implementation context:\n// Check for duplicates\n/* Commit transaction */"
        ));
        assert!(embed_text.contains("File: UserService.java:42"));
    }

    #[test]
    fn test_prepare_entities_batch() {
        let entity1 = ParsedEntity::new(
            "Class1",
            EntityKind::Class,
            "Class1",
            None,
            None,
            "java",
            "file1.java",
            1,
            10,
            None,
            "test-repo",
        );
        let entity2 = ParsedEntity::new(
            "Class2",
            EntityKind::Class,
            "Class2",
            None,
            None,
            "java",
            "file2.java",
            1,
            10,
            None,
            "test-repo",
        );

        let mut entities = vec![entity1, entity2];

        // Before prepare, embed_text is empty
        assert!(entities[0].embed_text.is_empty());
        assert!(entities[1].embed_text.is_empty());

        prepare_entities(&mut entities);

        // After prepare, embed_text is populated
        assert!(!entities[0].embed_text.is_empty());
        assert!(!entities[1].embed_text.is_empty());
        assert!(entities[0].embed_text.contains("Class1"));
        assert!(entities[1].embed_text.contains("Class2"));
    }

    #[test]
    fn test_build_embed_text_build_dependency() {
        let entity = ParsedEntity::new(
            "org.springframework:spring-core:5.3.29",
            EntityKind::BuildDependency,
            "org.springframework:spring-core:5.3.29",
            Some("scope: compile".to_string()),
            Some("Maven dependency: org.springframework:spring-core:5.3.29".to_string()),
            "xml",
            "pom.xml",
            15,
            19,
            None,
            "test-repo",
        );

        let embed_text = build_embed_text(&entity);

        assert!(embed_text.contains("[build_dependency] org.springframework:spring-core:5.3.29"));
        assert!(embed_text.contains("Signature: scope: compile"));
        assert!(embed_text.contains("Maven dependency:"));
        assert!(embed_text.contains("File: pom.xml:15"));
    }

    #[test]
    fn test_build_embed_text_build_plugin() {
        let entity = ParsedEntity::new(
            "org.apache.maven.plugins:maven-compiler-plugin:3.11.0",
            EntityKind::BuildPlugin,
            "org.apache.maven.plugins:maven-compiler-plugin:3.11.0",
            None,
            Some("Maven plugin: org.apache.maven.plugins:maven-compiler-plugin:3.11.0".to_string()),
            "xml",
            "pom.xml",
            30,
            35,
            None,
            "test-repo",
        );

        let embed_text = build_embed_text(&entity);

        assert!(embed_text.contains("[build_plugin]"));
        assert!(embed_text.contains("maven-compiler-plugin"));
        assert!(embed_text.contains("Maven plugin:"));
        assert!(embed_text.contains("File: pom.xml:30"));
    }

    #[test]
    fn test_build_embed_text_build_task() {
        let entity = ParsedEntity::new(
            "buildDocs",
            EntityKind::BuildTask,
            "buildDocs",
            None,
            Some("task buildDocs(type: Copy) {".to_string()),
            "groovy",
            "build.gradle",
            25,
            25,
            None,
            "test-repo",
        );

        let embed_text = build_embed_text(&entity);

        assert!(embed_text.contains("[build_task] buildDocs"));
        assert!(embed_text.contains("task buildDocs(type: Copy) {"));
        assert!(embed_text.contains("File: build.gradle:25"));
    }

    #[test]
    fn test_build_embed_text_pipeline_stage() {
        let entity = ParsedEntity::new(
            "stage: Build",
            EntityKind::PipelineStage,
            "stage: Build",
            None,
            Some("stage('Build') {".to_string()),
            "groovy",
            "Jenkinsfile",
            10,
            10,
            None,
            "test-repo",
        );

        let embed_text = build_embed_text(&entity);

        assert!(embed_text.contains("[pipeline_stage] stage: Build"));
        assert!(embed_text.contains("File: Jenkinsfile:10"));
    }

    #[test]
    fn test_build_embed_text_pipeline_step() {
        let entity = ParsedEntity::new(
            "sh: mvn compile",
            EntityKind::PipelineStep,
            "sh: mvn compile",
            None,
            Some("sh 'mvn compile'".to_string()),
            "groovy",
            "Jenkinsfile",
            12,
            12,
            None,
            "test-repo",
        );

        let embed_text = build_embed_text(&entity);

        assert!(embed_text.contains("[pipeline_step] sh: mvn compile"));
        assert!(embed_text.contains("File: Jenkinsfile:12"));
    }
}
