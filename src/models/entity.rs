use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::{ReferenceIntent, RelationshipType};

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
    // Kotlin-specific entities
    KotlinClass,           // class declarations
    KotlinInterface,       // interface declarations
    KotlinObject,          // object declarations (singletons)
    KotlinCompanionObject, // companion object declarations
    KotlinFunction,        // top-level and extension functions
    KotlinMethod,          // methods inside classes
    KotlinProperty,        // properties (val/var)
    // HTML/Web Components entities
    HtmlElement, // Custom elements like <app-profile>, <web-component>
    HtmlId,      // id="..." attributes
    HtmlClass,   // class="..." or className="..." attributes
    // CSS entities
    CssClass,    // .my-class
    CssId,       // #my-id
    CssVariable, // --my-var (CSS Custom Properties)
    // SCSS entities
    ScssVariable, // $my-var
    ScssMixin,    // @mixin my-mixin()
    ScssFunction, // @function my-function()
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
            EntityKind::KotlinClass => "kotlin_class",
            EntityKind::KotlinInterface => "kotlin_interface",
            EntityKind::KotlinObject => "kotlin_object",
            EntityKind::KotlinCompanionObject => "kotlin_companion_object",
            EntityKind::KotlinFunction => "kotlin_function",
            EntityKind::KotlinMethod => "kotlin_method",
            EntityKind::KotlinProperty => "kotlin_property",
            EntityKind::HtmlElement => "html_element",
            EntityKind::HtmlId => "html_id",
            EntityKind::HtmlClass => "html_class",
            EntityKind::CssClass => "css_class",
            EntityKind::CssId => "css_id",
            EntityKind::CssVariable => "css_variable",
            EntityKind::ScssVariable => "scss_variable",
            EntityKind::ScssMixin => "scss_mixin",
            EntityKind::ScssFunction => "scss_function",
        };
        write!(f, "{s}")
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

/// Minimal representation of an entity for relationship resolution.
///
/// Retains only the fields necessary to match reference intents to UUIDs,
/// allowing the heavy `embed_text`, `docstring`, and `vector` to be freed.
#[derive(Debug, Clone)]
pub struct ResolutionEntity {
    pub uuid: Uuid,
    pub name: String,
    pub fqn: String,
    pub enclosing_class: Option<String>,
    pub reference_intents: Vec<ReferenceIntent>,
    pub relationships: Vec<(Uuid, RelationshipType)>,
}

impl From<&ParsedEntity> for ResolutionEntity {
    fn from(entity: &ParsedEntity) -> Self {
        Self {
            uuid: entity.uuid,
            name: entity.name.clone(),
            fqn: entity.fqn.clone(),
            enclosing_class: entity.enclosing_class.clone(),
            reference_intents: entity.reference_intents.clone(),
            relationships: Vec::new(),
        }
    }
}

impl From<&EmbeddedEntity> for ResolutionEntity {
    fn from(ee: &EmbeddedEntity) -> Self {
        ResolutionEntity::from(&ee.entity)
    }
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
