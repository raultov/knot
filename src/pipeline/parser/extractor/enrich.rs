use crate::models::{EntityKind, ParsedEntity, ReferenceIntent};
use crate::pipeline::parser::comments::{extract_comments, extract_decorators};
use crate::pipeline::parser::context::{ClassContext, compute_fqn_and_context};
use crate::pipeline::parser::languages::{
    cpp, csharp, java, javascript, kotlin, markdown, python, typescript,
};
use crate::pipeline::parser::utils::{extract_decorator_references, extract_type_references};

use super::captures::CaptureState;

/// `true` for every `CSharp*` entity kind — the gate for the C# FQN prefix
/// branch below.
fn is_csharp_kind(kind: &EntityKind) -> bool {
    matches!(
        kind,
        EntityKind::CSharpClass
            | EntityKind::CSharpInterface
            | EntityKind::CSharpStruct
            | EntityKind::CSharpRecord
            | EntityKind::CSharpEnum
            | EntityKind::CSharpMethod
            | EntityKind::CSharpConstructor
            | EntityKind::CSharpProperty
            | EntityKind::CSharpField
            | EntityKind::CSharpConstant
            | EntityKind::CSharpDelegate
            | EntityKind::CSharpEvent
            | EntityKind::CSharpIndexer
            | EntityKind::CSharpOperator
            | EntityKind::CSharpNamespace
            | EntityKind::CSharpLocalFunction
    )
}

#[expect(
    clippy::too_many_arguments,
    reason = "entity enrichment requires state, source, language, and callbacks from extraction context"
)]
#[expect(
    clippy::too_many_lines,
    reason = "function is verbose but correct — extraction deferred"
)]
#[expect(
    clippy::cognitive_complexity,
    reason = "function is verbose but correct — extraction deferred"
)]
pub(crate) fn enrich_and_create_entity<'a>(
    state: &mut CaptureState<'a>,
    source_bytes: &[u8],
    lang_name: &str,
    file_path: &str,
    repo_name: &str,
    class_contexts: &[ClassContext],
    package_prefix: &Option<String>,
    entities: &mut Vec<ParsedEntity>,
    covered_ranges: &mut Vec<(usize, usize)>,
) {
    let name = state.name.take();
    let kind = state.kind.take();
    let mut signature = state.signature.take();
    let start_line = state.start_line;
    let end_line;
    let entity_node = state.entity_node.take();
    let mut reference_intents = std::mem::take(&mut state.reference_intents);
    let mut enclosing_class_fqn: Option<String> = None;

    if let (Some(mut name), Some(kind)) = (name, kind) {
        // Extract docstring and inline comments dynamically from the entity node
        let (docstring, inline_comments) = if let Some(node) = entity_node {
            extract_comments(node, source_bytes, lang_name, &kind, class_contexts)
        } else {
            (None, Vec::new())
        };

        // Extract decorators/annotations from the entity node
        let mut decorators = if let Some(node) = entity_node {
            extract_decorators(node, source_bytes, lang_name)
        } else {
            Vec::new()
        };

        // Phase 5: For Python entities, extract decorator names for display
        if lang_name == "python"
            && let Some(entity_n) = entity_node
        {
            python::extract_decorator_names_python(entity_n, source_bytes, &mut decorators);
        }

        // Determine FQN and enclosing class based on context
        let (mut fqn, mut enclosing_class) =
            compute_fqn_and_context(&name, &kind, start_line, lang_name, class_contexts);

        // Prefix FQN with Java package name if available
        if lang_name == "java"
            && let Some(pkg) = package_prefix
        {
            fqn = format!("{}.{}", pkg, fqn);
        }

        if matches!(
            kind,
            EntityKind::CppMethod
                | EntityKind::CFunction
                | EntityKind::CppClass
                | EntityKind::CStruct
                | EntityKind::CppNamespace
                | EntityKind::MacroDefinition
        ) && let Some(node) = entity_node
            && let Some(cpp_fqn) = cpp::build_cpp_fqn(node, source_bytes)
            && !cpp_fqn.is_empty()
        {
            enclosing_class = Some(cpp_fqn.clone());
            fqn = format!("{}::{}", cpp_fqn, name);
        }

        // C#: namespace-qualified FQN — the file-scoped namespace pre-pass
        // supplies the outermost scope, then an ancestor walk collects block
        // namespaces and containing types (plan §3.2). Namespace entities
        // anchor at their own name and are skipped by the prefix logic.
        if is_csharp_kind(&kind)
            && let Some(node) = entity_node
        {
            let is_namespace_decl = kind == EntityKind::CSharpNamespace
                && matches!(
                    node.kind(),
                    "namespace_declaration" | "file_scoped_namespace_declaration"
                );
            if is_namespace_decl {
                if let Some(anc) = csharp::build_csharp_fqn_prefix(node, source_bytes) {
                    fqn = format!("{}.{}", anc, name);
                    enclosing_class_fqn = Some(anc);
                }
            } else {
                let mut parts: Vec<String> = Vec::new();
                if lang_name == "csharp"
                    && let Some(pkg) = package_prefix
                {
                    parts.push(pkg.clone());
                }
                if let Some(anc) = csharp::build_csharp_fqn_prefix(node, source_bytes) {
                    parts.push(anc);
                }
                if !parts.is_empty() {
                    let prefix = parts.join(".");
                    fqn = format!("{}.{}", prefix, name);
                    enclosing_class_fqn = Some(prefix);
                }
            }
        }

        // derive the document's display name from repo + file path, and the FQN from the file path so entities don't collide.
        if lang_name == "markdown" {
            if matches!(kind, EntityKind::MarkdownDocument) {
                name = format!("{}::{}", repo_name, file_path);
            }

            fqn = match kind {
                EntityKind::MarkdownDocument => file_path.to_string(),
                EntityKind::MarkdownSection => {
                    if let Some(node) = entity_node {
                        let chain = markdown::build_markdown_fqn(node, source_bytes);
                        format!("{}::{}", file_path, chain)
                    } else {
                        format!("{}::{}", file_path, name)
                    }
                }
                _ => fqn,
            };
        }

        // For classes, also extract extends/implements from AST
        if matches!(
            kind,
            EntityKind::Class
                | EntityKind::Interface
                | EntityKind::KotlinClass
                | EntityKind::KotlinInterface
                | EntityKind::KotlinObject
                | EntityKind::KotlinEnum
                | EntityKind::CSharpClass
                | EntityKind::CSharpInterface
                | EntityKind::CSharpStruct
                | EntityKind::CSharpRecord
                | EntityKind::CSharpEnum
        ) && let Some(class_node) = entity_node
        {
            if lang_name == "javascript" {
                javascript::extract_class_inheritance_js(
                    class_node,
                    source_bytes,
                    &mut reference_intents,
                );
                // Extract decorator references for JavaScript (e.g., @Component, @Injectable)
                // Decorators may be in the parent node (export_statement) rather than class_declaration
                let decorator_node = class_node
                    .parent()
                    .filter(|p| p.kind() == "export_statement")
                    .unwrap_or(class_node);
                extract_decorator_references(decorator_node, source_bytes, &mut reference_intents);
            } else if lang_name == "typescript" {
                typescript::extract_class_inheritance(
                    class_node,
                    source_bytes,
                    &mut reference_intents,
                );
                // Extract decorator references (e.g., @Component, @NgModule)
                // Decorators may be in the parent node (export_statement) rather than class_declaration
                let decorator_node = class_node
                    .parent()
                    .filter(|p| p.kind() == "export_statement")
                    .unwrap_or(class_node);
                extract_decorator_references(decorator_node, source_bytes, &mut reference_intents);
                // Extract type references (e.g., constructor parameters, property types)
                extract_type_references(class_node, source_bytes, &mut reference_intents);
            } else if lang_name == "java" {
                // Extract extends/implements from Java class/interface declarations
                java::extract_class_inheritance_java(
                    class_node,
                    source_bytes,
                    &mut reference_intents,
                );
                // Extract annotation references (e.g., @Component, @Autowired)
                java::extract_annotation_references(
                    class_node,
                    source_bytes,
                    &mut reference_intents,
                );
                // Extract type references (e.g., constructor parameters, field types)
                extract_type_references(class_node, source_bytes, &mut reference_intents);
            } else if lang_name == "kotlin" {
                // Extract extends/implements from Kotlin class/interface declarations
                kotlin::extract_class_inheritance_kotlin(
                    class_node,
                    source_bytes,
                    &mut reference_intents,
                );
                // Extract annotation references (e.g., @Component, @Composable)
                kotlin::extract_annotation_references(
                    class_node,
                    source_bytes,
                    &mut reference_intents,
                );
                // Extract type references (e.g., constructor parameters, property types)
                kotlin::extract_type_references(class_node, source_bytes, &mut reference_intents);
            } else if lang_name == "csharp" {
                // Extract extends/implements from the C# base_list using the
                // §3.3 heuristic (first entry extends unless `^I[A-Z]`, …)
                csharp::extract_class_inheritance_csharp(
                    class_node,
                    source_bytes,
                    &mut reference_intents,
                );
                // Extract attribute references (e.g., [Obsolete], [Authorize]) —
                // attributes live in attribute_list children, not modifiers.
                csharp::extract_attribute_references(
                    class_node,
                    source_bytes,
                    &mut reference_intents,
                );
                // Extract type references (e.g., property types, generic args)
                csharp::extract_type_references_csharp(
                    class_node,
                    source_bytes,
                    &mut reference_intents,
                );
            }
        }

        // Calculate end_line from entity_node if available
        if let Some(node) = entity_node {
            end_line = node.end_position().row + 1;
        } else {
            // If no entity_node, use start_line as a fallback
            end_line = start_line;
        }

        // Extract C++ signature from entity_node if not captured by query
        if signature.is_none()
            && matches!(kind, EntityKind::CppMethod | EntityKind::CFunction)
            && let Some(node) = entity_node
        {
            signature = cpp::extract_cpp_signature(node, source_bytes);
        }

        let mut entity = ParsedEntity::new(
            name,
            kind,
            fqn,
            signature,
            docstring,
            lang_name,
            file_path,
            start_line,
            end_line,
            enclosing_class,
            repo_name,
        );
        entity.reference_intents = reference_intents;
        // Filter self-references (TypeReference/ValueReference to own name).
        // A type or value reference whose target is the enclosing entity itself
        // would produce a self-loop edge in Neo4j; useless visually and semantically.
        entity.reference_intents.retain(|intent| match intent {
            ReferenceIntent::TypeReference { type_name, .. } => type_name != entity.name.as_str(),
            ReferenceIntent::ValueReference { value_name, .. } => {
                value_name != entity.name.as_str()
            }
            _ => true,
        });
        entity.inline_comments = inline_comments;
        entity.decorators = decorators;
        // C#: persist the FQN of the enclosing scope (namespace + containing
        // types) so the CONTAINS auto-link in Neo4j can match by FQN instead
        // of by bare name (mirrors the Rust post-pass).
        entity.enclosing_class_fqn = enclosing_class_fqn;

        // Extract text below header/document entity for each markdown entity
        if lang_name == "markdown"
            && let Some(node) = entity_node
        {
            let body = match entity.kind {
                EntityKind::MarkdownDocument => {
                    markdown::extract_document_intro(node, source_bytes)
                }
                EntityKind::MarkdownSection => {
                    node.utf8_text(source_bytes).unwrap_or("").to_string()
                }
                _ => String::new(),
            };
            let header = format!("[{}] {}", entity.kind, entity.name);
            let location = format!("File: {}:{}", entity.file_path, entity.start_line);
            entity.embed_text = format!("{header}\n{location}\n\n{body}");
        }

        // Extract alias module path for JS require() aliases
        if lang_name == "javascript"
            && entity.kind == EntityKind::Constant
            && let Some(node) = entity_node
        {
            entity.alias_module_path = javascript::extract_require_module_path(node, source_bytes);
        }

        // Track byte range of this entity for orphan detection
        // Must be done for ALL entities to keep indices aligned with the entities vector
        if let Some(node) = entity_node {
            covered_ranges.push((node.start_byte(), node.end_byte()));
        } else {
            // If we don't have a node, use a dummy range that won't match any orphans
            covered_ranges.push((usize::MAX, usize::MAX));
        }

        entities.push(entity);
    }
}
