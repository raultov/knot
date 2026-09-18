//! Stage 3 — Prepare: UUID assignment and embedding text construction.
//!
//! Each [`ParsedEntity`] already carries a UUID generated at construction time
//! (see [`ParsedEntity::new`]). This stage's responsibility is to build the
//! `embed_text` field — the string that will be fed to the embedding model.
//!
//! # Embedding text format
//! ```text
//! <identifier_phrase>: <first docstring sentence>   ← natural-language role
//!   — or, for a doc-less callable with outgoing calls:
//! <identifier_phrase> — calls <tokenized callee names> (max 6)
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
//! natural-language queries that describe the entity's behaviour. The FIRST
//! line is a plain natural-language role sentence (identifier phrase plus
//! the docstring's first sentence, or the outgoing call names for doc-less
//! callables) so the model's strongest signal is behavioural vocabulary —
//! truncation eats structural fields before it eats the role. Identifiers
//! reach it both raw (name, FQN) and tokenized (`useChangePassword` → `use
//! change password`), and the names of the callees in the entity's body are
//! tokenized in a final `Calls:` section — a definition without a doc
//! comment stays retrievable through what it does, as observable from its
//! own body.

use crate::models::{EntityKind, ParsedEntity, ReferenceIntent};
use crate::utils::identifiers::identifier_token_phrase;

/// Maximum characters carried from the docstring's first sentence into the
/// role line. Bounded so the natural-language lead cannot itself push the
/// structural fields past the model's token budget (all current 384-dim
/// models truncate at 512 tokens; the role line and the kind/name header
/// must survive even when everything after them is clipped).
const ROLE_SENTENCE_MAX_CHARS: usize = 200;

/// Outgoing call names used to describe a doc-less entity's behaviour in
/// the role line. Shorter than the `Calls:` section (`MAX_CALL_NAMES`):
/// the role line is prose-shaped and a comma list of 20 identifiers stops
/// reading like a sentence.
const ROLE_CALL_NAMES: usize = 6;

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
    let mut parts: Vec<String> = Vec::with_capacity(11);

    // Role line FIRST: the natural-language lead is what the model sees
    // before truncation removes anything. All downstream model families
    // clip at their token budget keeping the front of the text, so the
    // behavioural sentence must precede structural noise.
    parts.push(role_sentence(entity));

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
/// in first-appearance order, capped at `cap`. The entity's own name is
/// dropped (self-references add no recall).
fn call_names(entity: &ParsedEntity, cap: usize) -> Vec<String> {
    let mut seen: Vec<String> = Vec::new();
    for intent in &entity.reference_intents {
        if seen.len() >= cap {
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
        .filter(|phrase| !phrase.is_empty())
        .collect()
}

/// The `Calls:` section body: deduplicated tokenized call names, capped at
/// [`MAX_CALL_NAMES`].
fn calls_section_text(entity: &ParsedEntity) -> String {
    call_names(entity, MAX_CALL_NAMES).join(", ")
}

// --- role line (natural-language lead of the embed text) ---------------------

/// Strip the comment markers a docstring collector may have carried verbatim
/// (`///`, `/**`, `*/`, `*`, `//`, `#`, `\"\"\"` …), collapse whitespace and
/// return the prose. Deterministic and language-agnostic: the same line
/// rule set applies to every language's comment syntax.
fn strip_comment_markers(doc: &str) -> String {
    doc.lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            // `trim_start_matches` peels repeated leading markers per line,
            // so `///`, `* *`, `*/` and `/*` all become empty or prose.
            let peeled = line
                .trim()
                .trim_start_matches(['/', '*', '-', '#', '"', '\'', '>', '<']);
            peeled.trim()
        })
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// First sentence of a comment-stripped docstring: text up to (and
/// including) the first `. ` / `.\n`, or the trailing period, or the whole
/// text when there is no sentence boundary. Truncated to
/// [`ROLE_SENTENCE_MAX_CHARS`] on a char boundary (never byte-sliced) with
/// an ellipsis, so multi-byte docstrings cannot panic.
fn first_sentence(doc: &str) -> String {
    let doc = doc.trim();
    let end = doc
        .find(". ")
        .or_else(|| doc.find(".\n"))
        .or_else(|| {
            if doc.ends_with('.') {
                Some(doc.len() - 1)
            } else {
                None
            }
        })
        .map(|i| i + 1)
        .unwrap_or(doc.len());
    let sentence = doc[..end].trim();

    if sentence.chars().count() <= ROLE_SENTENCE_MAX_CHARS {
        return sentence.to_owned();
    }
    let cut: String = sentence
        .char_indices()
        .take(ROLE_SENTENCE_MAX_CHARS)
        .map(|(_, c)| c)
        .collect();
    format!("{cut}…")
}

/// Deterministic natural-language role sentence for an entity.
///
/// Precedence:
/// 1. a non-empty docstring → `identifier phrase: first docstring sentence`;
/// 2. otherwise, for an entity whose body shows behaviour →
///    `identifier phrase — calls <tokenized callee names>`;
/// 3. otherwise the bare identifier phrase alone.
fn role_sentence(entity: &ParsedEntity) -> String {
    let phrase = {
        let p = identifier_token_phrase(&entity.name);
        if p.is_empty() { entity.name.clone() } else { p }
    };

    let docstring = entity
        .docstring
        .as_ref()
        .map(|d| strip_comment_markers(d))
        .filter(|d| !d.is_empty());

    if let Some(doc) = docstring {
        let sentence = first_sentence(&doc);
        if !sentence.is_empty() {
            return format!("{phrase}: {sentence}");
        }
    }

    let calls = call_names(entity, ROLE_CALL_NAMES);
    if !calls.is_empty() {
        return format!("{phrase} — calls {}", calls.join(", "));
    }

    phrase
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

    #[test]
    fn prepare_entities_leaves_markdown_embed_text_untouched() {
        let mut md_doc = ParsedEntity::new(
            "README.md",
            EntityKind::MarkdownDocument,
            "README.md",
            None,
            None,
            "markdown",
            "README.md",
            1,
            50,
            None,
            "test-repo",
        );
        md_doc.embed_text = "Original Markdown Document Body".to_string();

        let mut md_section = ParsedEntity::new(
            "Overview",
            EntityKind::MarkdownSection,
            "README.md::Overview",
            None,
            None,
            "markdown",
            "README.md",
            5,
            20,
            None,
            "test-repo",
        );
        md_section.embed_text = "Original Markdown Section Body".to_string();

        let code_entity = ParsedEntity::new(
            "run",
            EntityKind::Function,
            "run",
            None,
            None,
            "rust",
            "src/main.rs",
            1,
            10,
            None,
            "test-repo",
        );

        let mut batch = vec![md_doc, md_section, code_entity];
        prepare_entities(&mut batch);

        assert_eq!(batch[0].embed_text, "Original Markdown Document Body");
        assert_eq!(batch[1].embed_text, "Original Markdown Section Body");
        assert!(
            batch[2].embed_text.starts_with("run\n"),
            "code entity must be prepared with role line: {:?}",
            batch[2].embed_text
        );
    }

    // --- role line (natural-language lead, Workstream A) --------------------

    #[test]
    fn role_sentence_leads_with_docstring() {
        let entity = ParsedEntity::new(
            "saveUser",
            EntityKind::Method,
            "UserService.saveUser",
            None,
            Some("Saves a new user to the database. Throws on conflict.".to_string()),
            "java",
            "UserService.java",
            42,
            50,
            Some("UserService".to_string()),
            "test-repo",
        );
        let embed_text = build_embed_text(&entity);
        // First line is the role sentence; the kind header follows it.
        assert!(
            embed_text.starts_with("save user: Saves a new user to the database.\n"),
            "role sentence must lead plainly: {embed_text:?}"
        );
        assert!(
            embed_text
                .lines()
                .nth(1)
                .unwrap()
                .starts_with("[method] saveUser")
        );
    }

    #[test]
    fn role_sentence_falls_back_to_call_names() {
        let mut entity = ParsedEntity::new(
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
        for callee in [
            "normalize_email",
            "verify_credentials_or_fail",
            "issue_token",
        ] {
            entity.reference_intents.push(ReferenceIntent::Call {
                method: callee.to_string(),
                receiver: None,
                line: 1,
                arg_count: None,
            });
        }
        let embed_text = build_embed_text(&entity);
        let expected = "login — calls normalize email, verify credentials or fail, issue token";
        assert!(
            embed_text.starts_with(expected),
            "doc-less callable must lead with its behavioural vocabulary: {embed_text:?}"
        );
        // The full Calls: section is still there (doc-less recall contract).
        assert!(embed_text.contains("\nCalls: normalize email"));
    }

    #[test]
    fn role_sentence_bare_identifier_when_nothing_known() {
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
        assert!(
            embed_text.starts_with("use change password\n"),
            "{embed_text:?}"
        );
        assert!(
            !embed_text.contains("—"),
            "no call list may appear: {embed_text:?}"
        );
    }

    #[test]
    fn role_sentence_truncates_long_docstring_on_char_boundary() {
        // 1000 chars of multibyte prose with no sentence boundary: must not
        // panic (byte-slicing UTF-8 would) and must emit the ellipsis.
        let mut doc = String::new();
        while doc.chars().count() < 1000 {
            doc.push_str("sin() y así ");
        }
        let entity = ParsedEntity::new(
            "weave",
            EntityKind::RustFunction,
            "weave",
            None,
            Some(doc),
            "rust",
            "src/weave.rs",
            1,
            10,
            None,
            "test-repo",
        );
        let embed_text = build_embed_text(&entity);
        let role = embed_text
            .lines()
            .next()
            .expect("role line exists")
            .to_string();
        let char_count = role.chars().count();
        // phrase ("weave") + ": " + sentence(max 200) + …
        assert!(
            char_count <= ROLE_SENTENCE_MAX_CHARS + "weave: ".chars().count() + 1,
            "role line must be bounded: {char_count} chars"
        );
        assert!(
            role.ends_with('…'),
            "cut mid-sentence must show ellipsis: {role:?}"
        );
    }

    #[test]
    fn role_sentence_strips_comment_markers() {
        let entity = ParsedEntity::new(
            "init",
            EntityKind::GroovyMethod,
            "nextflow.plugin.extension.PluginExtensionPoint.init",
            None,
            Some("/**\n * Channel factory initialization.\n */".to_string()),
            "groovy",
            "PluginExtensionPoint.groovy",
            12,
            12,
            Some("PluginExtensionPoint".to_string()),
            "test-repo",
        );
        let embed_text = build_embed_text(&entity);
        assert!(
            embed_text.starts_with("init: Channel factory initialization.\n"),
            "comment markers must not leak into the role line: {embed_text:?}"
        );
    }

    #[test]
    fn embed_text_role_line_precedes_structural_fields() {
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
        entity.reference_intents.push(ReferenceIntent::Call {
            method: "to_database".to_string(),
            receiver: None,
            line: 1,
            arg_count: None,
        });
        let embed_text = build_embed_text(&entity);
        let role_idx = embed_text.lines().take(1).count();
        let sig_idx = embed_text
            .lines()
            .position(|l| l.starts_with("Signature:"))
            .expect("signature present")
            + 1;
        let calls_idx = embed_text
            .lines()
            .position(|l| l.starts_with("Calls:"))
            .expect("calls present")
            + 1;
        assert!(role_idx < sig_idx, "role must precede Signature");
        assert!(role_idx < calls_idx, "role must precede Calls");
        // The docstring body itself still follows.
        assert!(embed_text.contains("\nSaves a new user to the database.\n"));
    }

    #[test]
    fn role_sentence_handles_non_code_kinds() {
        // Config/build entities have no behaviour: the role line collapses
        // to the identifier phrase and must not panic.
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
        let first = embed_text.lines().next().unwrap();
        // Docstring branch fires for any kind; the phrase still leads and
        // nothing about config/build kinds panics or misformats.
        assert!(
            first.starts_with("org springframework spring core 5 3 29: "),
            "{first:?}"
        );
    }

    #[test]
    fn role_sentence_uses_shorter_call_cap_than_calls_section() {
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
        for i in 0..MAX_CALL_NAMES + 1 {
            entity.reference_intents.push(ReferenceIntent::Call {
                method: format!("callee_{i:02}"),
                receiver: None,
                line: i + 1,
                arg_count: None,
            });
        }
        let embed_text = build_embed_text(&entity);
        let role = embed_text.lines().next().unwrap();
        assert_eq!(
            role.matches("callee").count(),
            ROLE_CALL_NAMES,
            "role line capped at {ROLE_CALL_NAMES} names: {role:?}"
        );
        let calls = embed_text
            .lines()
            .find(|l| l.starts_with("Calls:"))
            .expect("Calls section");
        assert_eq!(calls.matches("callee").count(), MAX_CALL_NAMES);
    }
}
