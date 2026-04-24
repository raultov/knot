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
