//! Stage 3 — Prepare: UUID assignment and embedding text construction.
//!
//! Each [`ParsedEntity`] already carries a UUID generated at construction time
//! (see [`ParsedEntity::new`]). This stage's responsibility is to build the
//! `embed_text` field — the string that will be fed to the embedding model.
//!
//! # Embedding text format
//! ```text
//! [<KIND>] <name>
//! Identifier: <tokenized name>        ← omitted for single-token names
//! FQN: <fully qualified name>         ← omitted when it equals the name
//! Decorators: <decorators>            ← omitted when none
//! Signature: <signature>              ← omitted when None
//! <docstring>                         ← omitted when None
//! Implementation context: <comments>  ← omitted when none
//! Calls: <tokenized outgoing names>   ← omitted when none (capped at 20)
//! File: <file_path>:<start_line>
//! ```
//!
//! Keeping the format consistent across runs is important: the same entity
//! should always produce the same embedding so vector updates are idempotent.
//!
//! Recall contract: the embed text must share vocabulary with the
//! natural-language queries that describe the entity's behaviour. Identifiers
//! reach it both raw (name, FQN) and tokenized (`useChangePassword` → `use
//! change password`), and the names of the callees in the entity's body are
//! tokenized in a final `Calls:` section — a definition without a doc
//! comment stays retrievable through what it does, as observable from its
//! own body.

use crate::models::{EntityKind, ParsedEntity, ReferenceIntent};
use crate::utils::identifiers::identifier_token_phrase;

/// Maximum number of outgoing-reference names carried in the `Calls:`
/// section of the embed text. Deliberately no content filter beyond
/// dropping the entity's own name: short callee names (`new`, `use`, `map`)
/// are almost uniformly distributed across entities, so embedding them is
/// noise-neutral, and any length or deny-list heuristic would silently drop
/// legitimate short identifiers (`use` for React hooks, `map`, `post`).
const MAX_CALL_NAMES: usize = 20;

/// Build the `embed_text` field for every entity in-place.
///
/// This function is intentionally synchronous and allocation-cheap; it is
/// called after Rayon parsing and before the async embedding stage.
pub fn prepare_entities(entities: &mut [ParsedEntity]) {
    for entity in entities.iter_mut() {
        if matches!(
            entity.kind,
            EntityKind::MarkdownSection | EntityKind::MarkdownDocument
        ) {
            continue;
        }
        entity.embed_text = build_embed_text(entity);
    }
}

/// Construct the embedding text for a single entity.
fn build_embed_text(entity: &ParsedEntity) -> String {
    let mut parts: Vec<String> = Vec::with_capacity(10);

    // Header: kind + name
    parts.push(format!("[{}] {}", entity.kind, entity.name));

    // Tokenized identifier: natural-language queries share vocabulary with
    // the identifier's word components, not with its raw spelling.
    let name_phrase = identifier_token_phrase(&entity.name);
    if name_phrase != entity.name {
        parts.push(format!("Identifier: {name_phrase}"));
    }

    // Fully qualified name: path/module components ("api auth login") make
    // a definition retrievable through where it lives, too.
    if !entity.fqn.is_empty() && entity.fqn != entity.name {
        parts.push(format!("FQN: {}", entity.fqn));
        let fqn_phrase = identifier_token_phrase(&entity.fqn);
        if fqn_phrase != name_phrase {
            parts.push(format!("Identifier: {fqn_phrase}"));
        }
    }

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

    // Outgoing references: the names called/referenced inside this entity's
    // body, tokenized. For a definition without a doc comment this is the
    // only behavioural vocabulary available.
    let calls_section = calls_section_text(entity);
    if !calls_section.is_empty() {
        parts.push(format!("Calls: {calls_section}"));
    }

    // Source location — helps distinguish identically-named entities
    parts.push(format!("File: {}:{}", entity.file_path, entity.start_line));

    parts.join("\n")
}

/// Collect the deduplicated, tokenized names this entity calls or refers to,
/// in first-appearance order, capped at [`MAX_CALL_NAMES`]. The entity's
/// own name is dropped (self-references add no recall). Empty when there is
/// nothing to say.
fn calls_section_text(entity: &ParsedEntity) -> String {
    let mut seen: Vec<String> = Vec::new();
    for intent in &entity.reference_intents {
        if seen.len() >= MAX_CALL_NAMES {
            break;
        }
        let name = match intent {
            ReferenceIntent::Call { method, .. } => method.as_str(),
            ReferenceIntent::TypeReference { type_name, .. } => type_name.as_str(),
            ReferenceIntent::Extends { parent, .. } => parent.as_str(),
            ReferenceIntent::Implements { interface, .. } => interface.as_str(),
            _ => continue,
        };
        if name == entity.name || name.trim().is_empty() {
            continue;
        }
        if !seen.contains(&name.to_string()) {
            seen.push(name.to_string());
        }
    }
    seen.iter()
        .map(|n| identifier_token_phrase(n))
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::ParsedEntity;

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

    #[test]
    fn test_build_embed_text_groovy_class() {
        let entity = ParsedEntity::new(
            "MyGroovyClass",
            EntityKind::GroovyClass,
            "com.example.MyGroovyClass",
            None,
            Some("A Groovy class".to_string()),
            "groovy",
            "src/MyGroovyClass.groovy",
            1,
            10,
            None,
            "test-repo",
        );
        let embed_text = build_embed_text(&entity);
        assert!(embed_text.contains("[groovy_class] MyGroovyClass"));
        assert!(embed_text.contains("A Groovy class"));
    }

    #[test]
    fn test_build_embed_text_groovy_interface() {
        let entity = ParsedEntity::new(
            "MyGroovyInterface",
            EntityKind::GroovyInterface,
            "com.example.MyGroovyInterface",
            None,
            None,
            "groovy",
            "src/MyGroovyInterface.groovy",
            1,
            5,
            None,
            "test-repo",
        );
        let embed_text = build_embed_text(&entity);
        assert!(embed_text.contains("[groovy_interface] MyGroovyInterface"));
    }

    #[test]
    fn test_build_embed_text_groovy_trait() {
        let entity = ParsedEntity::new(
            "MyGroovyTrait",
            EntityKind::GroovyTrait,
            "com.example.MyGroovyTrait",
            None,
            None,
            "groovy",
            "src/MyGroovyTrait.groovy",
            1,
            8,
            None,
            "test-repo",
        );
        let embed_text = build_embed_text(&entity);
        assert!(embed_text.contains("[groovy_trait] MyGroovyTrait"));
    }

    #[test]
    fn test_build_embed_text_groovy_method() {
        let mut entity = ParsedEntity::new(
            "save",
            EntityKind::GroovyMethod,
            "UserService.save",
            Some("def save(User user)".to_string()),
            Some("Saves the user".to_string()),
            "groovy",
            "UserService.groovy",
            42,
            46,
            Some("UserService".to_string()),
            "test-repo",
        );
        entity.decorators = vec!["@Transactional".to_string()];
        let embed_text = build_embed_text(&entity);
        assert!(embed_text.contains("[groovy_method] save"));
        assert!(embed_text.contains("Signature: def save(User user)"));
        assert!(embed_text.contains("Saves the user"));
        assert!(embed_text.contains("Decorators: @Transactional"));
    }

    #[test]
    fn test_embed_text_groovy_method_includes_docstring() {
        // BDD contract: a Groovy entity's docstring must reach embed_text so
        // semantic search can match its concepts (nextflow `init` regression).
        let entity = ParsedEntity::new(
            "init",
            EntityKind::GroovyMethod,
            "nextflow.plugin.extension.PluginExtensionPoint.init",
            Some("abstract protected void init(Session session)".to_string()),
            Some(
                "Channel factory initialization. This method is invoked one and only once"
                    .to_string(),
            ),
            "groovy",
            "PluginExtensionPoint.groovy",
            12,
            12,
            Some("PluginExtensionPoint".to_string()),
            "test-repo",
        );
        let embed_text = build_embed_text(&entity);
        assert!(embed_text.contains("[groovy_method] init"));
        assert!(embed_text.contains("Channel factory initialization"));
    }

    #[test]
    fn test_build_embed_text_groovy_property() {
        let entity = ParsedEntity::new(
            "config",
            EntityKind::GroovyProperty,
            "com.example.App.config",
            None,
            None,
            "groovy",
            "App.groovy",
            3,
            3,
            None,
            "test-repo",
        );
        let embed_text = build_embed_text(&entity);
        assert!(embed_text.contains("[groovy_property] config"));
    }

    /// BDD contract (recall bug): the embed text must carry the entity's
    /// FQN so a definition is retrievable through the path components of
    /// its fully qualified name, not only its short name.
    #[test]
    fn embed_text_includes_fqn() {
        let entity = ParsedEntity::new(
            "login",
            EntityKind::RustFunction,
            "jobwatch::api::auth::login",
            None,
            None,
            "rust",
            "src/api/auth.rs",
            136,
            160,
            None,
            "job-watch",
        );
        let embed_text = build_embed_text(&entity);
        assert!(embed_text.contains("jobwatch::api::auth::login"));
    }

    /// BDD contract (recall bug): multi-case identifiers must reach the
    /// embed text in token form (`useChangePassword` → `use change
    /// password`) so natural-language queries share vocabulary with the
    /// vector without knowing the identifier's spelling.
    #[test]
    fn embed_text_includes_tokenized_identifier() {
        let entity = ParsedEntity::new(
            "useChangePassword",
            EntityKind::Function,
            "useChangePassword",
            None,
            None,
            "typescript",
            "src/api/authQueries.ts",
            5,
            12,
            None,
            "ui",
        );
        let embed_text = build_embed_text(&entity);
        assert!(embed_text.contains("Identifier: use change password"));
        // snake_case stays transparent too.
        let snake = ParsedEntity::new(
            "verify_credentials_or_fail",
            EntityKind::RustFunction,
            "verify_credentials_or_fail",
            None,
            None,
            "rust",
            "src/api/auth.rs",
            190,
            200,
            None,
            "job-watch",
        );
        let embed_text = build_embed_text(&snake);
        assert!(embed_text.contains("Identifier: verify credentials or fail"));
    }

    /// BDD contract (recall bug): the names of the callees in an entity's
    /// body must reach the embed text tokenized, so a paraphrase describing
    /// the behaviour ("authenticate user with email and password") shares
    /// vocabulary with a definition whose behaviour is only observable
    /// through its body.
    #[test]
    fn embed_text_includes_outgoing_call_names() {
        let mut entity = ParsedEntity::new(
            "login",
            EntityKind::RustFunction,
            "login",
            None,
            None,
            "rust",
            "src/api/auth.rs",
            136,
            160,
            None,
            "job-watch",
        );
        entity.reference_intents.push(ReferenceIntent::Call {
            method: "normalize_email".to_string(),
            receiver: None,
            line: 1,
            arg_count: None,
        });
        entity.reference_intents.push(ReferenceIntent::Call {
            method: "new".to_string(),
            receiver: None,
            line: 2,
            arg_count: None,
        });
        entity.reference_intents.push(ReferenceIntent::Call {
            method: "useChangePassword".to_string(),
            receiver: None,
            line: 3,
            arg_count: None,
        });

        let embed_text = build_embed_text(&entity);
        assert!(embed_text.contains("Calls: normalize email, new, use change password"));
    }

    /// BDD contract: the embed text is a deterministic pure function of the
    /// entity state, and the `Calls:` section keeps a stable size for
    /// entities with many outgoing references.
    #[test]
    fn embed_text_is_deterministic_and_bounded() {
        let build = |calls: usize| {
            let mut entity = ParsedEntity::new(
                "hub",
                EntityKind::RustFunction,
                "hub",
                None,
                None,
                "rust",
                "src/hub.rs",
                1,
                2,
                None,
                "r",
            );
            for i in 0..calls {
                entity.reference_intents.push(ReferenceIntent::Call {
                    method: if i == 0 {
                        "hub".to_string() // self reference must be dropped
                    } else {
                        format!("callee_{i:03}")
                    },
                    receiver: None,
                    line: i + 1,
                    arg_count: None,
                });
            }
            prepare_entities(std::slice::from_mut(&mut entity));
            (entity.embed_text.clone(), entity)
        };

        let (first, _) = build(0);
        let (second, _) = build(0);
        assert_eq!(first, second, "same entity must embed identically");

        let (many, _) = build(60);
        let counted = many
            .lines()
            .find(|l| l.starts_with("Calls:"))
            .expect("Calls section present");
        let names = counted.trim_start_matches("Calls: ").split(", ").count();
        assert_eq!(names, 20, "Calls section capped at 20 names");

        // Self-name never enters the Calls section.
        let (with_self, _) = build(3);
        assert!(!with_self.contains("Calls: hub"));
    }
}
