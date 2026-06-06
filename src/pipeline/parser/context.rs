use super::languages::rust;
use super::utils::*;
use crate::models::EntityKind;
use tree_sitter::Node;

/// Helper struct to track class context for FQN computation.
#[derive(Debug, Clone)]
pub(crate) struct ClassContext {
    pub(crate) name: String,
    pub(crate) start_line: usize,
    pub(crate) end_line: usize,
}

/// Extract all class/interface/object declarations and their line ranges.
pub(crate) fn extract_class_contexts(
    node: Node<'_>,
    source: &[u8],
    contexts: &mut Vec<ClassContext>,
) {
    if matches!(
        node.kind(),
        "class_declaration"
            | "interface_declaration"
            | "abstract_class_declaration"
            | "class_definition" // Python
            | "object_declaration"
    ) {
        // Find the name child
        let mut child = node.child(0);
        let mut class_name: Option<String> = None;
        while let Some(c) = child {
            if c.kind() == "identifier" || c.kind() == "type_identifier" {
                class_name = Some(node_text(c, source));
                break;
            }
            child = c.next_sibling();
        }

        if let Some(name) = class_name {
            contexts.push(ClassContext {
                name,
                start_line: node.start_position().row + 1,
                end_line: node.end_position().row + 1,
            });
        }
    } else if node.kind() == "impl_item"
        && let Some(self_type) = rust::extract_impl_self_type(node, source)
    {
        // Rust `impl Foo` and `impl Trait for Foo` establish a class context
        // for the self-type. The trait name (e.g. `LogSink`) is ignored — we
        // only want the type being implemented, so methods inside get the
        // FQN `Foo::method` rather than `LogSink::method`.
        contexts.push(ClassContext {
            name: self_type,
            start_line: node.start_position().row + 1,
            end_line: node.end_position().row + 1,
        });
    }

    // Recursively process children
    let mut child = node.child(0);
    while let Some(c) = child {
        extract_class_contexts(c, source, contexts);
        child = c.next_sibling();
    }
}

/// Compute FQN and enclosing_class based on entity context.
pub(crate) fn compute_fqn_and_context(
    name: &str,
    kind: &EntityKind,
    start_line: usize,
    _lang_name: &str,
    class_contexts: &[ClassContext],
) -> (String, Option<String>) {
    // Find which class contains this entity (if any)
    let enclosing_class = class_contexts
        .iter()
        .find(|ctx| start_line > ctx.start_line && start_line < ctx.end_line)
        .map(|ctx| ctx.name.clone());

    // Compute FQN
    let fqn = match kind {
        EntityKind::Class
        | EntityKind::Interface
        | EntityKind::KotlinClass
        | EntityKind::KotlinInterface
        | EntityKind::KotlinEnum
        | EntityKind::CppClass
        | EntityKind::CStruct
        | EntityKind::CppNamespace => {
            // For Java/Kotlin/C++, include enclosing class for nested declarations
            // For C++, the FQN will be updated dynamically in the extractor later
            if let Some(class_name) = &enclosing_class {
                format!("{}.{}", class_name, name)
            } else {
                name.to_string()
            }
        }
        EntityKind::Method | EntityKind::KotlinMethod | EntityKind::CppMethod => {
            // Method FQN: ClassName.methodName
            if let Some(class_name) = &enclosing_class {
                format!("{}.{}", class_name, name)
            } else {
                name.to_string()
            }
        }
        EntityKind::Function | EntityKind::KotlinFunction | EntityKind::CFunction => {
            // Top-level function - just the function name
            name.to_string()
        }
        EntityKind::Constant | EntityKind::KotlinProperty => {
            // Constant FQN: ClassName.CONST_NAME or just CONST_NAME for top-level
            if let Some(class_name) = &enclosing_class {
                format!("{}.{}", class_name, name)
            } else {
                name.to_string()
            }
        }
        EntityKind::Enum => {
            // Enum FQN: EnumName or ClassName.EnumName if nested
            if let Some(class_name) = &enclosing_class {
                format!("{}.{}", class_name, name)
            } else {
                name.to_string()
            }
        }
        // HTML entities already have their FQN computed in the parser
        // (e.g., "#id-name", ".class-name", "<custom-element>")
        EntityKind::HtmlElement | EntityKind::HtmlId | EntityKind::HtmlClass => name.to_string(),
        // CSS entities: FQN is the selector/variable name
        EntityKind::CssClass => format!(".{}", name),
        EntityKind::CssId => format!("#{}", name),
        EntityKind::CssVariable => format!("--{}", name),
        // SCSS entities: FQN is the variable/mixin/function name with prefix
        EntityKind::ScssVariable => format!("${}", name),
        EntityKind::ScssMixin => format!("@mixin {}", name),
        EntityKind::ScssFunction => format!("@function {}", name),
        // Kotlin-specific entities that don't nest like classes
        EntityKind::KotlinObject | EntityKind::KotlinCompanionObject => {
            // For nested objects/companions, include enclosing class name
            if let Some(class_name) = &enclosing_class {
                format!("{}.{}", class_name, name)
            } else {
                name.to_string()
            }
        }
        // Rust entities
        EntityKind::RustStruct
        | EntityKind::RustEnum
        | EntityKind::RustUnion
        | EntityKind::RustTrait
        | EntityKind::RustImpl
        | EntityKind::RustFunction
        | EntityKind::RustMacroDef
        | EntityKind::RustTypeAlias
        | EntityKind::RustConstant
        | EntityKind::RustStatic
        | EntityKind::RustModule => name.to_string(),
        EntityKind::RustMethod => {
            if let Some(class_name) = &enclosing_class {
                format!("{}::{}", class_name, name)
            } else {
                name.to_string()
            }
        }
        EntityKind::RustMacroInvoke => format!("{}!", name),
        // Python entities
        EntityKind::PythonClass
        | EntityKind::PythonFunction
        | EntityKind::PythonModule
        | EntityKind::PythonConstant => name.to_string(),
        EntityKind::PythonMethod => {
            if let Some(class_name) = &enclosing_class {
                format!("{}.{}", class_name, name)
            } else {
                name.to_string()
            }
        }
        // Build Systems & CI/CD entities — use name as FQN
        EntityKind::BuildDependency
        | EntityKind::BuildPlugin
        | EntityKind::BuildTask
        | EntityKind::PipelineStage
        | EntityKind::PipelineStep => name.to_string(),
        // Cargo (Rust build system) entities
        EntityKind::CargoPackage | EntityKind::CargoFeature | EntityKind::WorkspaceMember => {
            name.to_string()
        }
        // Configuration entities
        EntityKind::ConfigProperty => name.to_string(),
        // Kubernetes entities
        EntityKind::K8sDeployment
        | EntityKind::K8sService
        | EntityKind::K8sConfigMap
        | EntityKind::K8sSecret
        | EntityKind::K8sIngress
        | EntityKind::K8sNamespace
        | EntityKind::K8sResource => name.to_string(),
        // Helm entities
        EntityKind::HelmChart | EntityKind::HelmValue | EntityKind::HelmTemplateVar => {
            name.to_string()
        }
        // Project identity for cross-repo linking
        EntityKind::ProjectIdentity => name.to_string(),
        // Groovy entities
        EntityKind::GroovyClass
        | EntityKind::GroovyInterface
        | EntityKind::GroovyTrait
        | EntityKind::GroovyFunction
        | EntityKind::GroovyEnum
        | EntityKind::GroovyProperty => name.to_string(),
        EntityKind::GroovyMethod => {
            if let Some(class_name) = &enclosing_class {
                format!("{}.{}", class_name, name)
            } else {
                name.to_string()
            }
        }
        EntityKind::MacroDefinition => name.to_string(),
    };

    (fqn, enclosing_class)
}

/// Classification of a Rust file based on its location relative to the crate root.
///
/// Determines the FQN prefix strategy:
/// - `CrateSrc`: files inside `<crate_root>/src/` → FQN: `<crate>::<module>::<Entity>`
/// - `Fixture`: files inside a crate root but outside `src/` (tests, benches, examples)
///   → FQN: `__fixture::<segments>::<Entity>`
/// - `Loose`: files without any `Cargo.toml` ancestor
///   → FQN: `__loose::<file_stem>::<Entity>`
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RustFileKind {
    CrateSrc { module_path: String },
    Fixture { synthetic_path: String },
    Loose { synthetic_path: String },
}

/// Classify a Rust file into a [`RustFileKind`] based on its location relative
/// to the nearest `Cargo.toml` ancestor (if any).
///
/// - `file_path`: absolute or repo-relative path to the `.rs` file.
/// - `crate_root`: the directory containing the nearest `Cargo.toml` (if discovered).
/// - `repo_root`: the repository root directory.
pub(crate) fn compute_rust_file_kind(
    file_path: &str,
    crate_root: Option<&std::path::Path>,
    repo_root: &std::path::Path,
) -> RustFileKind {
    let file = std::path::Path::new(file_path);

    if let Some(root) = crate_root {
        // Try to strip the crate_root/src prefix — if it works, it's a normal crate source file.
        if let Ok(rel) = file.strip_prefix(root.join("src")) {
            let module_path = compute_module_path_from_relative(rel);
            return RustFileKind::CrateSrc { module_path };
        }

        // File is inside the crate root but outside src/ — it's a fixture.
        if let Ok(rel) = file.strip_prefix(root) {
            let synthetic = path_to_synthetic_segments(rel);
            return RustFileKind::Fixture {
                synthetic_path: synthetic,
            };
        }
    }

    // No crate root or file is outside it — it's a loose file.
    let synthetic = if let Ok(rel) = file.strip_prefix(repo_root) {
        path_to_synthetic_segments(rel)
    } else {
        // Fallback: use the file stem only.
        file.file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown")
            .to_string()
    };

    RustFileKind::Loose {
        synthetic_path: synthetic,
    }
}

/// Convert a file path into `::`-separated segments for use as a synthetic FQN prefix.
/// Strips the `.rs` extension and converts path separators to `::`.
fn path_to_synthetic_segments(path: &std::path::Path) -> String {
    let components: Vec<String> = path
        .components()
        .filter_map(|c| c.as_os_str().to_str().map(|s| s.to_string()))
        .collect();

    if components.is_empty() {
        return "unknown".to_string();
    }

    let last = components.last().unwrap();
    let stem = last.strip_suffix(".rs").unwrap_or(last);

    let mut parts: Vec<&str> = components[..components.len() - 1]
        .iter()
        .map(|s| s.as_str())
        .collect();
    if !stem.is_empty() {
        parts.push(stem);
    }

    parts.join("::")
}

/// Extract the module path from a relative path (relative to crate_root/src).
/// Shared logic between `compute_rust_module_path` and `compute_rust_file_kind`.
fn compute_module_path_from_relative(relative: &std::path::Path) -> String {
    let components: Vec<&str> = relative
        .components()
        .filter_map(|c| c.as_os_str().to_str())
        .collect();

    if components.is_empty() {
        return String::new();
    }

    let last = components.last().unwrap();
    let file_stem = last.strip_suffix(".rs").unwrap_or(last);

    let mut module_parts: Vec<&str> = components[..components.len() - 1].to_vec();

    match file_stem {
        "lib" | "main" => {}
        "mod" => {}
        other => module_parts.push(other),
    }

    module_parts.join("::")
}

/// Compute the Rust module path that owns `file_path`, relative to a crate
/// root (the directory that contains `Cargo.toml`).
///
/// Returns an empty string for the crate root files (`src/lib.rs`,
/// `src/main.rs`), the parent module for `mod.rs`, and the dotted module
/// path otherwise (e.g. `src/db/graph.rs` → `db::graph`).
///
/// **Note:** prefer [`compute_rust_file_kind`] for new code; this function
/// is retained for backward compatibility and tests.
#[cfg(test)]
pub(crate) fn compute_rust_module_path(file_path: &str, crate_root: &std::path::Path) -> String {
    let file = std::path::Path::new(file_path);
    let relative = match file.strip_prefix(crate_root.join("src")) {
        Ok(rel) => rel,
        Err(_) => return String::new(),
    };

    compute_module_path_from_relative(relative)
}

/// Build a fully-qualified Rust name from its crate, module path, optional
/// enclosing impl type, and entity name.
///
/// The returned string is always anchored at the crate name so two files
/// that declare a `Config` struct in different crates produce distinct FQNs
/// (`crate_a::config::Config` vs. `crate_b::config::Config`).
pub(crate) fn compute_rust_qualified_fqn(
    name: &str,
    kind: &EntityKind,
    crate_name: &str,
    module_path: &str,
    enclosing_class: Option<&str>,
) -> String {
    let mut parts: Vec<String> = Vec::new();
    parts.push(crate_name.to_string());
    if !module_path.is_empty() {
        parts.push(module_path.to_string());
    }

    let is_method = matches!(kind, EntityKind::RustMethod);

    if let Some(class_name) = enclosing_class {
        if is_method {
            let base = if parts.is_empty() {
                class_name.to_string()
            } else {
                format!("{}::{}", parts.join("::"), class_name)
            };
            return format!("{}::{}", base, name);
        }
        parts.push(class_name.to_string());
    }

    if parts.is_empty() {
        name.to_string()
    } else {
        format!("{}::{}", parts.join("::"), name)
    }
}

/// Build a fully-qualified Rust name using a [`RustFileKind`] to determine
/// the prefix strategy.
///
/// - `CrateSrc`: standard `<crate>::<module>::<Entity>` format.
/// - `Fixture`: `__fixture::<segments>::<Entity>` format.
/// - `Loose`: `__loose::<file_stem>::<Entity>` format.
#[cfg(test)]
pub(crate) fn compute_rust_qualified_fqn_from_kind(
    name: &str,
    kind: &EntityKind,
    file_kind: &RustFileKind,
    crate_name: &str,
    enclosing_class: Option<&str>,
) -> String {
    compute_rust_qualified_fqn_with_inline_modules(
        name,
        kind,
        file_kind,
        crate_name,
        "",
        enclosing_class,
    )
}

/// Same as [`compute_rust_qualified_fqn_from_kind`] but also splices in any
/// **inline** `mod foo { ... }` blocks that contain the entity.
///
/// `inline_module_path` is a `::`-separated chain such as `"tests"` or
/// `"outer::inner"`. An empty string means the entity sits at the file's
/// top level. The inline path is inserted *between* the per-file module
/// path and the entity name (and, for methods, before the enclosing class):
///
/// ```text
/// crate::config::tests::test_foo                   // function in #[cfg(test)] mod tests
/// crate::config::outer::inner::Bar                  // struct in nested inline mods
/// crate::config::tests::Cache::new                  // method on Cache inside mod tests
/// ```
pub(crate) fn compute_rust_qualified_fqn_with_inline_modules(
    name: &str,
    kind: &EntityKind,
    file_kind: &RustFileKind,
    crate_name: &str,
    inline_module_path: &str,
    enclosing_class: Option<&str>,
) -> String {
    match file_kind {
        RustFileKind::CrateSrc { module_path } => {
            let combined = combine_module_paths(module_path, inline_module_path);
            compute_rust_qualified_fqn(name, kind, crate_name, &combined, enclosing_class)
        }
        RustFileKind::Fixture { synthetic_path } => {
            let combined = combine_module_paths(synthetic_path, inline_module_path);
            build_synthetic_fqn("__fixture", &combined, name, kind, enclosing_class)
        }
        RustFileKind::Loose { synthetic_path } => {
            let combined = combine_module_paths(synthetic_path, inline_module_path);
            build_synthetic_fqn("__loose", &combined, name, kind, enclosing_class)
        }
    }
}

/// Join two `::`-separated module paths, ignoring empty segments.
fn combine_module_paths(file_module_path: &str, inline_module_path: &str) -> String {
    match (file_module_path.is_empty(), inline_module_path.is_empty()) {
        (true, true) => String::new(),
        (false, true) => file_module_path.to_string(),
        (true, false) => inline_module_path.to_string(),
        (false, false) => format!("{}::{}", file_module_path, inline_module_path),
    }
}

/// Build an FQN with a synthetic prefix (`__fixture` or `__loose`).
fn build_synthetic_fqn(
    prefix: &str,
    synthetic_path: &str,
    name: &str,
    kind: &EntityKind,
    enclosing_class: Option<&str>,
) -> String {
    let is_method = matches!(kind, EntityKind::RustMethod);

    if is_method && let Some(class_name) = enclosing_class {
        let base = if synthetic_path.is_empty() {
            format!("{}::{}", prefix, class_name)
        } else {
            format!("{}::{}::{}", prefix, synthetic_path, class_name)
        };
        return format!("{}::{}", base, name);
    }

    if synthetic_path.is_empty() {
        format!("{}::{}", prefix, name)
    } else {
        format!("{}::{}::{}", prefix, synthetic_path, name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compute_fqn_and_context_class() {
        let contexts = vec![];
        let (fqn, enclosing_class) =
            compute_fqn_and_context("MyClass", &EntityKind::Class, 10, "java", &contexts);
        assert_eq!(fqn, "MyClass");
        assert!(enclosing_class.is_none());
    }

    #[test]
    fn test_compute_fqn_and_context_method_with_class() {
        let contexts = vec![ClassContext {
            name: "MyClass".to_string(),
            start_line: 5,
            end_line: 20,
        }];
        let (fqn, enclosing_class) =
            compute_fqn_and_context("myMethod", &EntityKind::Method, 10, "java", &contexts);
        assert_eq!(fqn, "MyClass.myMethod");
        assert_eq!(enclosing_class, Some("MyClass".to_string()));
    }

    #[test]
    fn test_compute_fqn_and_context_method_without_class() {
        let contexts = vec![];
        let (fqn, enclosing_class) =
            compute_fqn_and_context("myFunction", &EntityKind::Method, 10, "java", &contexts);
        assert_eq!(fqn, "myFunction");
        assert!(enclosing_class.is_none());
    }

    #[test]
    fn test_compute_fqn_and_context_function() {
        let contexts = vec![];
        let (fqn, enclosing_class) = compute_fqn_and_context(
            "topLevelFunction",
            &EntityKind::Function,
            10,
            "typescript",
            &contexts,
        );
        assert_eq!(fqn, "topLevelFunction");
        assert!(enclosing_class.is_none());
    }

    #[test]
    fn test_compute_fqn_and_context_constant_with_class() {
        let contexts = vec![ClassContext {
            name: "Constants".to_string(),
            start_line: 1,
            end_line: 50,
        }];
        let (fqn, enclosing_class) =
            compute_fqn_and_context("CONST_VALUE", &EntityKind::Constant, 25, "java", &contexts);
        assert_eq!(fqn, "Constants.CONST_VALUE");
        assert_eq!(enclosing_class, Some("Constants".to_string()));
    }

    #[test]
    fn test_compute_fqn_and_context_enum() {
        let contexts = vec![];
        let (fqn, enclosing_class) =
            compute_fqn_and_context("Color", &EntityKind::Enum, 1, "java", &contexts);
        assert_eq!(fqn, "Color");
        assert!(enclosing_class.is_none());
    }

    #[test]
    fn test_extract_class_contexts_java() {
        let code = "public class TestClass { }\npublic interface TestInterface { }";
        let tree = crate::pipeline::parser::test_utils::parse_java_snippet(code)
            .expect("Failed to parse Java code");

        let source = code.as_bytes();
        let mut contexts: Vec<ClassContext> = Vec::new();
        extract_class_contexts(tree.root_node(), source, &mut contexts);

        assert_eq!(contexts.len(), 2);
        assert!(contexts.iter().any(|c| c.name == "TestClass"));
        assert!(contexts.iter().any(|c| c.name == "TestInterface"));
    }

    #[test]
    fn test_extract_class_contexts_nested() {
        let code = "class Outer { \n  class Inner { } \n}";
        let tree = crate::pipeline::parser::test_utils::parse_typescript_snippet(code)
            .expect("Failed to parse TypeScript code");

        let source = code.as_bytes();
        let mut contexts: Vec<ClassContext> = Vec::new();
        extract_class_contexts(tree.root_node(), source, &mut contexts);

        // Both outer and inner classes should be captured
        assert!(!contexts.is_empty());
        assert!(contexts.iter().any(|c| c.name == "Outer"));
    }

    #[test]
    fn test_extract_class_contexts_kotlin_object() {
        let code = "object NodeUtils { fun stream() {} }";
        let tree = crate::pipeline::parser::test_utils::parse_kotlin_snippet(code)
            .expect("Failed to parse Kotlin code");

        let source = code.as_bytes();
        let mut contexts: Vec<ClassContext> = Vec::new();
        extract_class_contexts(tree.root_node(), source, &mut contexts);

        assert_eq!(contexts.len(), 1, "Expected 1 context for object NodeUtils");
        assert!(contexts.iter().any(|c| c.name == "NodeUtils"));
    }

    #[test]
    fn test_extract_class_contexts_kotlin_companion() {
        // tree-sitter-kotlin-ng v1.1.0 requires newlines in class body for companion_object.
        let code = "class Foo {\n    companion object {\n        fun bar() {}\n    }\n}";
        let tree = crate::pipeline::parser::test_utils::parse_kotlin_snippet(code)
            .expect("Failed to parse Kotlin code");

        let source = code.as_bytes();
        let mut contexts: Vec<ClassContext> = Vec::new();
        extract_class_contexts(tree.root_node(), source, &mut contexts);

        // Class Foo is captured via class_declaration.
        // companion_object does NOT create its own context (parent class already covers it).
        assert!(
            contexts.iter().any(|c| c.name == "Foo"),
            "Expected class Foo to be captured as context. Found: {:?}",
            contexts.iter().map(|c| &c.name).collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_compute_fqn_and_context_kotlin_method_in_object() {
        let contexts = vec![ClassContext {
            name: "NodeUtils".to_string(),
            start_line: 1,
            end_line: 3,
        }];
        let (fqn, enclosing_class) =
            compute_fqn_and_context("stream", &EntityKind::KotlinMethod, 2, "kotlin", &contexts);
        assert_eq!(fqn, "NodeUtils.stream");
        assert_eq!(enclosing_class, Some("NodeUtils".to_string()));
    }

    #[test]
    fn test_compute_fqn_and_context_kotlin_object_nested() {
        let contexts = vec![ClassContext {
            name: "OuterClass".to_string(),
            start_line: 1,
            end_line: 50,
        }];
        let (fqn, enclosing_class) = compute_fqn_and_context(
            "InnerObject",
            &EntityKind::KotlinObject,
            25,
            "kotlin",
            &contexts,
        );
        assert_eq!(fqn, "OuterClass.InnerObject");
        assert_eq!(enclosing_class, Some("OuterClass".to_string()));
    }

    #[test]
    fn test_compute_fqn_and_context_kotlin_function_top_level() {
        let contexts = vec![];
        let (fqn, enclosing_class) = compute_fqn_and_context(
            "greetUser",
            &EntityKind::KotlinFunction,
            5,
            "kotlin",
            &contexts,
        );
        assert_eq!(fqn, "greetUser");
        assert!(
            enclosing_class.is_none(),
            "Top-level functions should have no enclosing class"
        );
    }

    #[test]
    fn test_extract_class_contexts_includes_rust_impl_item() {
        // `impl Foo { ... }` should register Foo as a class context, so
        // methods inside get the qualified FQN `Foo::method`.
        let code = r#"
struct Foo;
impl Foo {
    pub fn new() -> Self { Foo }
}
"#;
        let tree = crate::pipeline::parser::test_utils::parse_rust_snippet(code)
            .expect("Failed to parse Rust code");

        let source = code.as_bytes();
        let mut contexts: Vec<ClassContext> = Vec::new();
        extract_class_contexts(tree.root_node(), source, &mut contexts);

        let foo_ctx = contexts
            .iter()
            .find(|c| c.name == "Foo")
            .expect("Expected ClassContext for Foo from impl_item");
        // The context must encompass the inner fn (line 4 in the snippet).
        assert!(foo_ctx.start_line <= 4 && foo_ctx.end_line >= 4);
    }

    #[test]
    fn test_extract_class_contexts_rust_impl_trait_for_uses_self_type() {
        // `impl Bar for Foo` should use `Foo` (self-type), NOT `Bar` (trait).
        let code = r#"
trait Bar { fn new() -> Self; }
struct Foo;
impl Bar for Foo {
    fn new() -> Self { Foo }
}
"#;
        let tree = crate::pipeline::parser::test_utils::parse_rust_snippet(code)
            .expect("Failed to parse Rust code");

        let source = code.as_bytes();
        let mut contexts: Vec<ClassContext> = Vec::new();
        extract_class_contexts(tree.root_node(), source, &mut contexts);

        // Must include Foo (self-type) …
        assert!(
            contexts.iter().any(|c| c.name == "Foo"),
            "Expected self-type Foo to appear as a context, got: {:?}",
            contexts.iter().map(|c| &c.name).collect::<Vec<_>>()
        );
        // … and must NOT include Bar (trait) as a class context, otherwise
        // methods inside would incorrectly get FQN `Bar::new`.
        assert!(
            !contexts.iter().any(|c| c.name == "Bar"),
            "Trait name Bar should NOT appear as a class context, got: {:?}",
            contexts.iter().map(|c| &c.name).collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_rust_module_path_from_lib_rs() {
        let crate_root = std::path::PathBuf::from("/repo/my_crate");
        let path = "/repo/my_crate/src/lib.rs";
        assert_eq!(compute_rust_module_path(path, &crate_root), "");
    }

    #[test]
    fn test_rust_module_path_from_nested_file() {
        let crate_root = std::path::PathBuf::from("/repo/my_crate");
        let path = "/repo/my_crate/src/db/graph.rs";
        assert_eq!(compute_rust_module_path(path, &crate_root), "db::graph");
    }

    #[test]
    fn test_rust_module_path_from_mod_rs() {
        let crate_root = std::path::PathBuf::from("/repo/my_crate");
        let path = "/repo/my_crate/src/db/mod.rs";
        assert_eq!(compute_rust_module_path(path, &crate_root), "db");
    }

    #[test]
    fn test_rust_module_path_strips_main() {
        let crate_root = std::path::PathBuf::from("/repo/my_crate");
        let path = "/repo/my_crate/src/main.rs";
        assert_eq!(compute_rust_module_path(path, &crate_root), "");
    }

    #[test]
    fn test_rust_module_path_outside_crate_returns_empty() {
        let crate_root = std::path::PathBuf::from("/repo/my_crate");
        let path = "/other_repo/src/lib.rs";
        assert_eq!(compute_rust_module_path(path, &crate_root), "");
    }

    #[test]
    fn test_rust_module_path_deeply_nested() {
        let crate_root = std::path::PathBuf::from("/repo/my_crate");
        let path = "/repo/my_crate/src/pipeline/parser/languages/rust.rs";
        assert_eq!(
            compute_rust_module_path(path, &crate_root),
            "pipeline::parser::languages::rust"
        );
    }

    #[test]
    fn test_rust_fqn_struct_uses_module_path() {
        let fqn = compute_rust_qualified_fqn(
            "Config",
            &EntityKind::RustStruct,
            "crate_a",
            "config",
            None,
        );
        assert_eq!(fqn, "crate_a::config::Config");
    }

    #[test]
    fn test_rust_fqn_method_includes_module() {
        let fqn = compute_rust_qualified_fqn(
            "load",
            &EntityKind::RustMethod,
            "crate_a",
            "config",
            Some("Config"),
        );
        assert_eq!(fqn, "crate_a::config::Config::load");
    }

    #[test]
    fn test_rust_fqn_function_no_enclosing() {
        let fqn = compute_rust_qualified_fqn(
            "helper",
            &EntityKind::RustFunction,
            "crate_a",
            "utils",
            None,
        );
        assert_eq!(fqn, "crate_a::utils::helper");
    }

    #[test]
    fn test_rust_fqn_struct_at_crate_root_no_module() {
        let fqn =
            compute_rust_qualified_fqn("Config", &EntityKind::RustStruct, "crate_a", "", None);
        assert_eq!(fqn, "crate_a::Config");
    }

    #[test]
    fn test_rust_fqn_method_at_crate_root() {
        let fqn = compute_rust_qualified_fqn(
            "load",
            &EntityKind::RustMethod,
            "crate_a",
            "",
            Some("Config"),
        );
        assert_eq!(fqn, "crate_a::Config::load");
    }

    #[test]
    fn test_rust_fqn_nested_struct_uses_enclosing_segment() {
        let fqn = compute_rust_qualified_fqn(
            "Inner",
            &EntityKind::RustStruct,
            "crate_a",
            "outer",
            Some("Outer"),
        );
        assert_eq!(fqn, "crate_a::outer::Outer::Inner");
    }

    // --- PR2: RustFileKind tests ---

    #[test]
    fn test_rust_file_kind_crate_src() {
        let tmp = tempfile::tempdir().unwrap();
        let crate_root = tmp.path();
        let file = crate_root.join("src/config.rs");

        let kind = compute_rust_file_kind(&file.to_string_lossy(), Some(crate_root), crate_root);
        assert_eq!(
            kind,
            RustFileKind::CrateSrc {
                module_path: "config".to_string()
            }
        );
    }

    #[test]
    fn test_rust_file_kind_fixture_in_tests() {
        let tmp = tempfile::tempdir().unwrap();
        let crate_root = tmp.path();
        let file = crate_root.join("tests/testing_files/sample.rs");

        let kind = compute_rust_file_kind(&file.to_string_lossy(), Some(crate_root), crate_root);
        match &kind {
            RustFileKind::Fixture { synthetic_path } => {
                assert_eq!(
                    synthetic_path, "tests::testing_files::sample",
                    "fixture path should use :: separators"
                );
            }
            _ => panic!("expected Fixture kind, got {:?}", kind),
        }
    }

    #[test]
    fn test_rust_file_kind_fixture_in_benches() {
        let tmp = tempfile::tempdir().unwrap();
        let crate_root = tmp.path();
        let file = crate_root.join("benches/pipeline_bench.rs");

        let kind = compute_rust_file_kind(&file.to_string_lossy(), Some(crate_root), crate_root);
        match &kind {
            RustFileKind::Fixture { synthetic_path } => {
                assert_eq!(synthetic_path, "benches::pipeline_bench");
            }
            _ => panic!("expected Fixture kind, got {:?}", kind),
        }
    }

    #[test]
    fn test_rust_file_kind_loose_no_cargo_ancestor() {
        let tmp = tempfile::tempdir().unwrap();
        let repo_root = tmp.path();
        let file = repo_root.join("scripts/helper.rs");

        // No crate_root (None) — should be Loose
        let kind = compute_rust_file_kind(&file.to_string_lossy(), None, repo_root);
        match &kind {
            RustFileKind::Loose { synthetic_path } => {
                assert_eq!(synthetic_path, "scripts::helper");
            }
            _ => panic!("expected Loose kind, got {:?}", kind),
        }
    }

    #[test]
    fn test_rust_fqn_fixture_entity() {
        let kind = RustFileKind::Fixture {
            synthetic_path: "tests::testing_files::sample".to_string(),
        };
        let fqn = compute_rust_qualified_fqn_from_kind(
            "Config",
            &EntityKind::RustStruct,
            &kind,
            "knot",
            None,
        );
        assert_eq!(
            fqn, "__fixture::tests::testing_files::sample::Config",
            "fixture FQN should use __fixture:: prefix"
        );
    }

    #[test]
    fn test_rust_fqn_fixture_method() {
        let kind = RustFileKind::Fixture {
            synthetic_path: "tests::testing_files::sample".to_string(),
        };
        let fqn = compute_rust_qualified_fqn_from_kind(
            "load_mcp",
            &EntityKind::RustMethod,
            &kind,
            "knot",
            Some("Config"),
        );
        assert_eq!(
            fqn, "__fixture::tests::testing_files::sample::Config::load_mcp",
            "fixture method FQN should include enclosing class"
        );
    }

    #[test]
    fn test_rust_fqn_loose_entity() {
        let kind = RustFileKind::Loose {
            synthetic_path: "scripts::helper".to_string(),
        };
        let fqn = compute_rust_qualified_fqn_from_kind(
            "my_func",
            &EntityKind::RustFunction,
            &kind,
            "__loose",
            None,
        );
        assert_eq!(
            fqn, "__loose::scripts::helper::my_func",
            "loose FQN should use __loose:: prefix"
        );
    }

    #[test]
    fn test_rust_fqn_crate_src_unchanged() {
        let kind = RustFileKind::CrateSrc {
            module_path: "config".to_string(),
        };
        let fqn = compute_rust_qualified_fqn_from_kind(
            "Config",
            &EntityKind::RustStruct,
            &kind,
            "knot",
            None,
        );
        assert_eq!(
            fqn, "knot::config::Config",
            "crate src FQN should be unchanged from original behavior"
        );
    }
}
