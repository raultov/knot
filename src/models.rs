//! Core data models shared across all pipeline stages.
//!
//! Every extracted entity receives a UUID v4 that acts as the primary key
//! bridging Qdrant (vector store) and Neo4j (graph store).

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// The kind of code entity extracted from the AST.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EntityKind {
    Class,
    Interface,
    Method,
    Function,
    Constant,
    Enum,
}

impl std::fmt::Display for EntityKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            EntityKind::Class => "class",
            EntityKind::Interface => "interface",
            EntityKind::Method => "method",
            EntityKind::Function => "function",
            EntityKind::Constant => "constant",
            EntityKind::Enum => "enum",
        };
        write!(f, "{s}")
    }
}

/// Represents a method or function invocation within source code.
///
/// Captures the call site and its context (receiver, method name) to enable
/// accurate resolution of which method is being called.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallIntent {
    /// The method name being called (e.g., `proxyManager`, `connect`).
    pub method: String,

    /// The receiver object/class (if any).
    /// Examples:
    /// - `None` for local calls like `proxyManager()` or `this.proxyManager()`
    /// - `Some("this")` for explicit this calls
    /// - `Some("ClassName")` for static calls like `AlternativeConnectorService.proxyManager()`
    /// - `Some("objectName")` for instance calls like `client.setProxy()`
    pub receiver: Option<String>,

    /// 1-based line number where this call occurs.
    pub line: usize,
}

/// A raw entity extracted from the AST before embedding.
///
/// Created in Stage 2 (parse) and enriched with a UUID in Stage 3 (prepare).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParsedEntity {
    /// Unique identifier — bridges Qdrant and Neo4j records.
    pub uuid: Uuid,

    /// Human-readable name (class name, method name, function name).
    pub name: String,

    /// The kind of code construct this entity represents.
    pub kind: EntityKind,

    /// Fully qualified name for disambiguation.
    /// - For Java: `com.shelob.connector.AlternativeConnectorService.proxyManager`
    /// - For TypeScript: `ClassName.methodName` or just `functionName` for top-level functions
    /// - For classes/interfaces: `com.shelob.connector.AlternativeConnectorService`
    pub fqn: String,

    /// Full signature string for methods/functions (e.g. `public void foo(int x)`).
    /// `None` for class/interface entities.
    pub signature: Option<String>,

    /// Associated documentation: JavaDoc, JSDoc, or preceding comments found
    /// immediately before this node in the source file.
    /// This captures comment blocks that directly precede declarations.
    pub docstring: Option<String>,

    /// Inline comments found within this entity's body.
    /// For classes: comments inside the class body but outside any methods.
    /// For methods/functions: comments inside the method/function body.
    /// These provide implementation context and are associated with the containing entity.
    pub inline_comments: Vec<String>,

    /// Decorators and annotations applied to this entity.
    /// Examples: `@Override`, `@OnEvent('foo')`, `@GetMapping("/path")`, etc.
    /// Populated during the parse stage for methods, functions, classes, and constants.
    pub decorators: Vec<String>,

    /// Source language (`"java"` or `"typescript"`).
    pub language: String,

    /// Absolute path of the source file containing this entity.
    pub file_path: String,

    /// 1-based line number where the entity declaration starts.
    pub start_line: usize,

    /// Enclosing class name for methods. Used to resolve local calls.
    /// `None` for class/interface entities and top-level functions.
    pub enclosing_class: Option<String>,

    /// Logical repository name for multi-repository isolation.
    /// Used to separate entities from different codebases in shared databases.
    /// Example: "shelob-java", "shelob-typescript", "my-microservice"
    pub repo_name: String,

    /// Raw call intents extracted from this entity's body.
    /// Populated during the parse stage; may be empty if no calls were found.
    /// These are resolved to UUIDs during the ingest stage.
    pub call_intents: Vec<CallIntent>,

    /// UUIDs of entities this entity directly calls / depends on.
    /// Populated during the ingest stage based on call_intents resolution.
    pub calls: Vec<Uuid>,

    /// The text that will be fed to the embedding model.
    /// Constructed in Stage 3 (prepare) from `name`, `signature`, and `docstring`.
    pub embed_text: String,
}

impl ParsedEntity {
    /// Create a new entity with a freshly generated UUID.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        name: impl Into<String>,
        kind: EntityKind,
        fqn: impl Into<String>,
        signature: Option<String>,
        docstring: Option<String>,
        language: impl Into<String>,
        file_path: impl Into<String>,
        start_line: usize,
        enclosing_class: Option<String>,
        repo_name: impl Into<String>,
    ) -> Self {
        Self {
            uuid: Uuid::new_v4(),
            name: name.into(),
            kind,
            fqn: fqn.into(),
            signature,
            docstring,
            language: language.into(),
            file_path: file_path.into(),
            start_line,
            enclosing_class,
            repo_name: repo_name.into(),
            call_intents: Vec::new(),
            calls: Vec::new(),
            inline_comments: Vec::new(),
            decorators: Vec::new(),
            embed_text: String::new(),
        }
    }
}

/// An entity that has been embedded and is ready for dual-write ingestion.
///
/// Produced by Stage 4 (embed) and consumed by Stage 5 (ingest).
#[derive(Debug, Clone)]
pub struct EmbeddedEntity {
    /// The original parsed entity with all metadata.
    pub entity: ParsedEntity,

    /// High-dimensional vector produced by the embedding model.
    pub vector: Vec<f32>,
}
