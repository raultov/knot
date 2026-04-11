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
