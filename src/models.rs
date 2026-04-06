//! Core data models shared across all pipeline stages.
//!
//! Every extracted entity receives a deterministic UUID v5 that acts as the primary key
//! bridging Qdrant (vector store) and Neo4j (graph store).
//!
//! UUIDs are deterministic (derived from repo_name + file_path + fqn) to enable
//! incremental indexing without breaking graph relationships.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Namespace UUID for knot entity generation.
/// All entity UUIDs are derived from this namespace using UUID v5.
/// This ensures deterministic, reproducible UUIDs across indexing runs.
pub const NAMESPACE_KNOT: Uuid = Uuid::from_bytes([
    0x6b, 0x6e, 0x6f, 0x74, 0x2d, 0x69, 0x6e, 0x64, 0x65, 0x78, 0x65, 0x72, 0x2d, 0x76, 0x35, 0x00,
]);

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

/// Represents a reference to another entity within source code.
///
/// This enum captures different types of code dependencies:
/// - **Call**: method or function invocation (e.g., `obj.method()`, `new MyClass()`)
/// - **Extends**: class inheritance (e.g., `class Child extends Parent { }`)
/// - **Implements**: interface implementation (e.g., `class Impl implements IFace { }`)
/// - **TypeReference**: type annotation/usage (e.g., `prop: SomeType`, `returns: ReturnType`)
///
/// All reference types include the target name and a line number for source location.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ReferenceIntent {
    /// A method, function, or constructor call.
    /// Examples: `this.method()`, `obj.func()`, `new MyClass()`
    Call {
        /// The method/function/class name being called.
        method: String,
        /// The receiver object/class (if any).
        receiver: Option<String>,
        /// Line number where this call occurs.
        line: usize,
    },
    /// A class or interface is extended (inheritance).
    /// Example: `class Child extends Parent { }`
    Extends {
        /// The parent class or interface name.
        parent: String,
        /// Line number where extends clause appears.
        line: usize,
    },
    /// An interface is implemented.
    /// Example: `class Impl implements IFace { }`
    Implements {
        /// The interface name being implemented.
        interface: String,
        /// Line number where implements clause appears.
        line: usize,
    },
    /// A type is referenced in an annotation or signature.
    /// Examples: `prop: SomeType`, `returns: ReturnType`, `param: ArgType`
    TypeReference {
        /// The referenced type name.
        type_name: String,
        /// Line number where the reference appears.
        line: usize,
    },
}

/// Represents a typed relationship edge in the dependency graph.
/// Created during the ingest stage after resolving reference intents to UUIDs.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum RelationshipType {
    /// Method/function call or constructor invocation.
    Calls,
    /// Class inheritance (extends).
    Extends,
    /// Interface implementation (implements).
    Implements,
    /// Type annotation or usage in a signature/variable.
    References,
}

impl std::fmt::Display for RelationshipType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RelationshipType::Calls => write!(f, "CALLS"),
            RelationshipType::Extends => write!(f, "EXTENDS"),
            RelationshipType::Implements => write!(f, "IMPLEMENTS"),
            RelationshipType::References => write!(f, "REFERENCES"),
        }
    }
}

/// Legacy alias for backward compatibility (Call variant only).
/// New code should use [`ReferenceIntent`] directly.
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

impl From<CallIntent> for ReferenceIntent {
    fn from(call: CallIntent) -> Self {
        ReferenceIntent::Call {
            method: call.method,
            receiver: call.receiver,
            line: call.line,
        }
    }
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
    /// - For Java: `com.example.connector.AlternativeConnectorService.proxyManager`
    /// - For TypeScript: `ClassName.methodName` or just `functionName` for top-level functions
    /// - For classes/interfaces: `com.example.connector.AlternativeConnectorService`
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
    /// Example: "my-java-repo", "my-typescript-repo", "my-microservice"
    pub repo_name: String,

    /// Raw reference intents extracted from this entity.
    /// Populated during the parse stage; may be empty if no references were found.
    /// These are resolved to UUIDs during the ingest stage and converted to graph relationships.
    pub reference_intents: Vec<ReferenceIntent>,

    /// UUIDs of entities this entity directly calls / depends on.
    /// **DEPRECATED:** Use `relationships` instead for typed edges.
    /// Populated during the ingest stage based on reference_intents resolution.
    /// Kept for backward compatibility.
    pub calls: Vec<Uuid>,

    /// Typed relationships to other entities.
    /// Each edge includes the target UUID and the relationship type (CALLS, EXTENDS, IMPLEMENTS, REFERENCES).
    /// Populated during the ingest stage based on reference_intents resolution.
    pub relationships: Vec<(Uuid, RelationshipType)>,

    /// The text that will be fed to the embedding model.
    /// Constructed in Stage 3 (prepare) from `name`, `signature`, and `docstring`.
    pub embed_text: String,
}

impl ParsedEntity {
    /// Create a new entity with a deterministic UUID v5.
    ///
    /// The UUID is derived from the entity's unique identity (repo_name:file_path:fqn)
    /// to ensure the same entity always receives the same UUID across indexing runs.
    /// This is critical for incremental indexing to avoid breaking graph relationships.
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
        let name = name.into();
        let fqn = fqn.into();
        let file_path = file_path.into();
        let repo_name = repo_name.into();

        // Generate deterministic UUID from unique identity
        let identity = format!("{}:{}:{}", repo_name, file_path, fqn);
        let uuid = Uuid::new_v5(&NAMESPACE_KNOT, identity.as_bytes());

        Self {
            uuid,
            name,
            kind,
            fqn,
            signature,
            docstring,
            language: language.into(),
            file_path,
            start_line,
            enclosing_class,
            repo_name,
            reference_intents: Vec::new(),
            calls: Vec::new(),
            relationships: Vec::new(),
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
