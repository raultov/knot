use serde::{Deserialize, Serialize};

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
        /// Number of arguments in the call (if known).
        /// Used for disambiguating overloaded methods.
        arg_count: Option<usize>,
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
    /// A class, function, or value is used as an argument in a keyword argument.
    /// Examples: `parser.add_argument(..., action=EnumAction)`, `callback=my_handler`
    /// This distinguishes from TypeReference (imports) to enable tracking
    /// "class/function passed as parameter" patterns.
    ValueReference {
        /// The value name being referenced (class, function, or variable).
        value_name: String,
        /// Line number where the reference appears.
        line: usize,
    },
    /// JavaScript references an HTML element by ID.
    /// Example: `document.getElementById('app')`, `querySelector('#main')`
    DomElementReference {
        /// The HTML element ID being referenced (without the `#` prefix).
        element_id: String,
        /// Line number where this reference occurs.
        line: usize,
    },
    /// JavaScript uses or manipulates a CSS class.
    /// Examples: `element.classList.add('active')`, `element.className = 'new-class'`
    CssClassUsage {
        /// The CSS class name being used (without the `.` prefix).
        class_name: String,
        /// Line number where this usage occurs.
        line: usize,
    },
    /// HTML imports a JavaScript file.
    /// Example: `<script src="main.js"></script>`
    HtmlFileImport {
        /// The imported file path (relative or absolute).
        file_path: String,
        /// Line number where this import occurs.
        line: usize,
    },
    /// HTML imports a CSS stylesheet.
    /// Example: `<link rel="stylesheet" href="style.css">`
    CssFileImport {
        /// The imported CSS file path (relative or absolute).
        file_path: String,
        /// Line number where this import occurs.
        line: usize,
    },
    /// Rust macro invocation.
    /// Example: `println!("hello")`, `vec![1, 2, 3]`
    RustMacroCall {
        /// The macro name being invoked (e.g., "println", "vec")
        macro_name: String,
        /// Line number where this macro invocation occurs.
        line: usize,
    },
    /// VCL subroutine call.
    /// Example: `call pipe_if_local;`
    VclSubCall {
        /// The subroutine name being called.
        sub_name: String,
        /// Line number where this call occurs.
        line: usize,
    },
    /// VCL backend reference (set req.backend_hint, beresp.backend etc).
    /// Example: `set req.backend_hint = b;`
    VclBackendRef {
        /// The backend name being referenced.
        backend_name: String,
        /// Line number where this reference occurs.
        line: usize,
    },
    /// VCL probe reference (.probe = myprobe;).
    VclProbeRef {
        /// The probe name being referenced.
        probe_name: String,
        /// Line number where this reference occurs.
        line: usize,
    },
    /// VCL ACL reference (client.ip ~ aclname).
    VclAclRef {
        /// The ACL name being referenced.
        acl_name: String,
        /// Line number where this reference occurs.
        line: usize,
    },
    /// VCL file include.
    /// Example: `include "foo.vcl";`
    VclInclude {
        /// The include path string.
        path: String,
        /// Line number where this include occurs.
        line: usize,
    },
    /// VCL VMOD import.
    /// Example: `import std;`
    VclVmodImport {
        /// The module name being imported.
        module: String,
        /// The alias if using `import std as s;`.
        alias: Option<String>,
        /// Line number where this import occurs.
        line: usize,
    },
    /// VCL `unused` marker.
    /// Example: `unused b1;`
    VclUnusedRef {
        /// The name of the unused declaration.
        name: String,
        /// Line number where this unused declaration occurs.
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
    /// JavaScript code references an HTML element by ID.
    ReferencesDOM,
    /// JavaScript code uses or manipulates a CSS class.
    UsesCSSClass,
    /// HTML file imports a JavaScript file via <script> tag.
    ImportsScript,
    /// HTML file imports a CSS stylesheet via <link> tag.
    ImportsStylesheet,
    /// Rust: Code invokes a macro
    MacroCalls,
    /// Rust: Parent-child containment (module contains function, impl contains method)
    Contains,
    /// Rust: Generic type parameter bound (e.g., `T: Clone`)
    GenericBound,
    /// Repository -> Repository dependency edge.
    DependsOn,
    /// JVM (Java/Kotlin/Groovy): a method in a subtype overrides or implements
    /// the corresponding method declared in a supertype (interface or superclass).
    /// Edge direction: `subtype.method -[Overrides]-> supertype.method`.
    Overrides,
    /// VCL: subroutine or entity uses a backend.
    UsesBackend,
    /// VCL: entity uses a probe.
    UsesProbe,
    /// VCL: entity uses an ACL.
    UsesAcl,
    /// VCL: file includes another file.
    Includes,
    /// VCL: file imports a VMOD.
    ImportsVmod,
    /// VCL: `unused` declaration marks an entity as intentionally unreferenced.
    DeclaredUnused,
}

impl std::fmt::Display for RelationshipType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RelationshipType::Calls => write!(f, "CALLS"),
            RelationshipType::Extends => write!(f, "EXTENDS"),
            RelationshipType::Implements => write!(f, "IMPLEMENTS"),
            RelationshipType::References => write!(f, "REFERENCES"),
            RelationshipType::ReferencesDOM => write!(f, "REFERENCES_DOM"),
            RelationshipType::UsesCSSClass => write!(f, "USES_CSS_CLASS"),
            RelationshipType::ImportsScript => write!(f, "IMPORTS_SCRIPT"),
            RelationshipType::ImportsStylesheet => write!(f, "IMPORTS_STYLESHEET"),
            RelationshipType::MacroCalls => write!(f, "MACRO_CALLS"),
            RelationshipType::Contains => write!(f, "CONTAINS"),
            RelationshipType::GenericBound => write!(f, "GENERIC_BOUND"),
            RelationshipType::DependsOn => write!(f, "DEPENDS_ON"),
            RelationshipType::Overrides => write!(f, "OVERRIDES"),
            RelationshipType::UsesBackend => write!(f, "USES_BACKEND"),
            RelationshipType::UsesProbe => write!(f, "USES_PROBE"),
            RelationshipType::UsesAcl => write!(f, "USES_ACL"),
            RelationshipType::Includes => write!(f, "INCLUDES"),
            RelationshipType::ImportsVmod => write!(f, "IMPORTS_VMOD"),
            RelationshipType::DeclaredUnused => write!(f, "DECLARED_UNUSED"),
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

    /// Number of arguments in the call (if known).
    /// Used for disambiguating overloaded methods.
    pub arg_count: Option<usize>,
}

impl From<CallIntent> for ReferenceIntent {
    fn from(call: CallIntent) -> Self {
        ReferenceIntent::Call {
            method: call.method,
            receiver: call.receiver,
            line: call.line,
            arg_count: call.arg_count,
        }
    }
}
