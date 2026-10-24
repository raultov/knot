//! Method-level `OVERRIDES` linking for override-capable languages
//! (Java, Kotlin, Groovy, C#).
//!
//! knot models inheritance only at the type level (`EXTENDS` / `IMPLEMENTS`).
//! This pass adds an additive, method-granular edge
//! `subtype.method -[OVERRIDES]-> supertype.method` so that reverse-dependency
//! queries can surface the relationship between an interface/superclass method
//! and its implementations/overrides.
//!
//! Design:
//! - **Additive only** — never mutates or removes existing entities/edges.
//! - **Override-capable guard** — a file-extension guard plus a kind
//!   allowlist. The generic `Class`/`Interface`/`Method` kinds are shared
//!   with TypeScript, so the extension guard is what keeps non-participating
//!   languages at zero `OVERRIDES` edges. C# joins the guard via dedicated
//!   `CSharp*` kinds and the `.cs` extension.
//! - **Nearest-declaration linking** — each method links only to the nearest
//!   declaration(s) of the same name walking up its type hierarchy. Full
//!   transitive visibility is resolved at query time with variable-length
//!   Cypher (`-[:OVERRIDES*1..]->`).
//! - **Overload-aware matching** — same-named declarations are filtered by
//!   arity and parameter types parsed from the raw signatures (see
//!   [`params`]); when several survive, the most specific wins. Methods
//!   without a parseable signature (e.g. synthetic Groovy accessors) fall back
//!   to name-only matching.
//! - **Batch-local** — only links methods whose supertype methods are present
//!   in the same indexing batch (incremental limitation, documented in the spec).

mod params;

use std::collections::{HashMap, HashSet};

use uuid::Uuid;

use crate::models::{EntityKind, RelationshipType, ResolutionEntity};

pub(super) use params::DeclaredArity;
use params::{ParamList, match_params};
use params::{declared_arity, last_param_is_array};

/// A method declared by an override-capable type.
struct MethodEntry<'a> {
    name: &'a str,
    uuid: Uuid,
    idx: usize,
    /// Parsed parameter list; `None` when the signature is absent or unreadable.
    params: Option<ParamList>,
}

/// Source file extensions of override-capable languages. The guard is
/// applied FIRST to every entity; this is what excludes TypeScript, which
/// shares the generic `Class`/`Interface`/`Method` kinds.
const OVERRIDE_CAPABLE_EXTENSIONS: &[&str] =
    &[".java", ".kt", ".kts", ".groovy", ".gvy", ".gradle", ".cs"];

/// Groovy source extensions: default parameter values compile to one overload
/// per omittable parameter, so a declaration answers for several arities.
const DEFAULTS_EXPAND_ARITY_EXTENSIONS: &[&str] = &[".groovy", ".gvy", ".gradle"];

fn has_extension(file_path: &str, extensions: &[&str]) -> bool {
    extensions.iter().any(|ext| file_path.ends_with(ext))
}

/// JavaScript sources: a call may pass fewer arguments than declared (the
/// rest are `undefined`) or more (they are ignored), so arity never rules a
/// call target out.
const JAVASCRIPT_EXTENSIONS: &[&str] = &[".js", ".mjs", ".cjs", ".jsx"];

/// Whether the argument count of a call written in `file_path` constrains
/// its target (issue #38). False for JavaScript.
pub(super) fn call_arity_is_binding(file_path: &str) -> bool {
    !has_extension(file_path, JAVASCRIPT_EXTENSIONS)
}

/// The call-site arity of a method declared in `file_path`, with the
/// language rules [`declared_arity`] cannot see from the signature alone:
/// JavaScript parameters are all optional, and a trailing Groovy array
/// parameter accepts varargs.
pub(super) fn declared_call_arity(
    signature: &str,
    method_name: &str,
    file_path: &str,
) -> DeclaredArity {
    if !call_arity_is_binding(file_path) {
        return DeclaredArity::Variable;
    }
    if has_extension(file_path, DEFAULTS_EXPAND_ARITY_EXTENSIONS)
        && last_param_is_array(signature, method_name)
    {
        return DeclaredArity::Variable;
    }
    declared_arity(signature, method_name)
}

/// Returns `true` if the file path ends in an override-capable extension.
fn is_override_capable_file(file_path: &str) -> bool {
    has_extension(file_path, OVERRIDE_CAPABLE_EXTENSIONS)
}

/// Whether overloads declared in `file_path` span the class hierarchy, so a
/// call may bind to an inherited overload rather than the receiver's own
/// same-named method. True for the override-capable languages (Java, Kotlin,
/// Groovy, C#); in TypeScript a subclass method replaces the inherited one.
pub(super) fn overloads_span_hierarchy(file_path: &str) -> bool {
    is_override_capable_file(file_path)
}

/// Method-like kinds that can override/implement a supertype method.
///
/// `KotlinFunction` is intentionally excluded: top-level and extension
/// functions are statically dispatched and cannot override. `CSharpProperty`
/// is included because C# properties can be `virtual`/`override`.
fn is_override_capable_method(kind: &EntityKind) -> bool {
    matches!(
        kind,
        EntityKind::Method
            | EntityKind::KotlinMethod
            | EntityKind::GroovyMethod
            | EntityKind::CSharpMethod
            | EntityKind::CSharpProperty
    )
}

/// Type-like kinds that can declare overridable methods and participate in
/// an inheritance hierarchy (includes Kotlin objects/companions, enums, and
/// the C# type kinds).
fn is_override_capable_type(kind: &EntityKind) -> bool {
    matches!(
        kind,
        EntityKind::Class
            | EntityKind::Interface
            | EntityKind::Enum
            | EntityKind::KotlinClass
            | EntityKind::KotlinInterface
            | EntityKind::KotlinEnum
            | EntityKind::KotlinObject
            | EntityKind::KotlinCompanionObject
            | EntityKind::GroovyClass
            | EntityKind::GroovyInterface
            | EntityKind::GroovyTrait
            | EntityKind::GroovyEnum
            | EntityKind::CSharpClass
            | EntityKind::CSharpInterface
            | EntityKind::CSharpStruct
            | EntityKind::CSharpRecord
            | EntityKind::CSharpEnum
    )
}

/// Strips a method FQN down to its enclosing type FQN.
///
/// All override-capable parsers emit `method_fqn == type_fqn + "." + method_name`
/// (including nested classes `Outer.Inner.method` and Kotlin anonymous objects
/// `Foo.bar.<anonymous@30>.foo`). Returns `None` for top-level entities with no
/// enclosing type (e.g. a method FQN equal to its own name).
fn enclosing_type_fqn<'a>(method_fqn: &'a str, method_name: &str) -> Option<&'a str> {
    // Prefer stripping the exact ".<method_name>" suffix (robust for names that
    // themselves contain dots, though method names in these languages do not).
    if let Some(prefix) = method_fqn.strip_suffix(method_name)
        && let Some(without_dot) = prefix.strip_suffix('.')
        && !without_dot.is_empty()
    {
        return Some(without_dot);
    }
    // Fallback: strip the last dot-delimited segment.
    let idx = method_fqn.rfind('.')?;
    if idx == 0 {
        return None;
    }
    Some(&method_fqn[..idx])
}

/// Returns `true` if a method-like entity is a constructor and must be excluded.
///
/// Uses the name-based heuristic that covers all supported parsers: the method name
/// equals its enclosing type's name, or equals `<init>`.
fn is_constructor(method_name: &str, enclosing_type_name: &str) -> bool {
    method_name == "<init>" || method_name == enclosing_type_name
}

fn build_type_index(entities: &[ResolutionEntity]) -> (HashMap<&str, Uuid>, HashMap<Uuid, usize>) {
    let mut type_fqn_to_uuid = HashMap::new();
    let mut uuid_to_index = HashMap::new();
    for (idx, e) in entities.iter().enumerate() {
        uuid_to_index.insert(e.uuid, idx);
        if is_override_capable_file(&e.file_path) && is_override_capable_type(&e.kind) {
            type_fqn_to_uuid.insert(e.fqn.as_str(), e.uuid);
        }
    }
    (type_fqn_to_uuid, uuid_to_index)
}

type HierarchyMaps<'a> = (
    HashMap<Uuid, Vec<Uuid>>,
    HashMap<Uuid, Vec<MethodEntry<'a>>>,
);

fn collect_hierarchy<'a>(
    entities: &'a [ResolutionEntity],
    type_fqn_to_uuid: &HashMap<&str, Uuid>,
    uuid_to_index: &HashMap<Uuid, usize>,
) -> HierarchyMaps<'a> {
    let mut supertypes: HashMap<Uuid, Vec<Uuid>> = HashMap::new();
    let mut methods_by_type: HashMap<Uuid, Vec<MethodEntry<'_>>> = HashMap::new();

    for (idx, e) in entities.iter().enumerate() {
        if !is_override_capable_file(&e.file_path) {
            continue;
        }

        if is_override_capable_type(&e.kind) {
            let parents: Vec<Uuid> = e
                .relationships
                .iter()
                .filter(|(_, rel)| {
                    matches!(
                        rel,
                        RelationshipType::Extends | RelationshipType::Implements
                    )
                })
                .map(|(uuid, _)| *uuid)
                .collect();
            if !parents.is_empty() {
                supertypes.entry(e.uuid).or_default().extend(parents);
            }
        } else if is_override_capable_method(&e.kind) {
            let Some(type_fqn) = enclosing_type_fqn(&e.fqn, &e.name) else {
                continue;
            };
            let Some(&type_uuid) = type_fqn_to_uuid.get(type_fqn) else {
                continue;
            };
            if let Some(&type_idx) = uuid_to_index.get(&type_uuid)
                && is_constructor(&e.name, &entities[type_idx].name)
            {
                continue;
            }
            let params = e.signature.as_deref().and_then(|sig| {
                ParamList::parse(
                    sig,
                    &e.name,
                    has_extension(&e.file_path, DEFAULTS_EXPAND_ARITY_EXTENSIONS),
                )
            });
            methods_by_type
                .entry(type_uuid)
                .or_default()
                .push(MethodEntry {
                    name: e.name.as_str(),
                    uuid: e.uuid,
                    idx,
                    params,
                });
        }
    }
    (supertypes, methods_by_type)
}

/// The declarations of type `s` that `method` can override: same name, and —
/// when both signatures were parsed — compatible arity and parameter types.
/// Among the compatible declarations only the most specific ones are kept.
/// Declarations whose signature could not be compared are a fallback used only
/// when no comparable declaration matches.
fn declared_methods(
    methods_by_type: &HashMap<Uuid, Vec<MethodEntry<'_>>>,
    s: Uuid,
    method: &MethodEntry<'_>,
) -> Vec<Uuid> {
    let Some(candidates) = methods_by_type.get(&s) else {
        return Vec::new();
    };
    let mut best_score = None;
    let mut best = Vec::new();
    let mut uncomparable = Vec::new();
    for candidate in candidates.iter().filter(|c| c.name == method.name) {
        let (Some(sub), Some(sup)) = (&method.params, &candidate.params) else {
            uncomparable.push(candidate.uuid);
            continue;
        };
        let Some(score) = match_params(sub, sup) else {
            continue;
        };
        if best_score.is_none_or(|b| score > b) {
            best_score = Some(score);
            best.clear();
        }
        if best_score == Some(score) {
            best.push(candidate.uuid);
        }
    }
    if best.is_empty() { uncomparable } else { best }
}

fn nearest_declarations(
    type_uuid: Uuid,
    method: &MethodEntry<'_>,
    supertypes: &HashMap<Uuid, Vec<Uuid>>,
    methods_by_type: &HashMap<Uuid, Vec<MethodEntry<'_>>>,
) -> Vec<Uuid> {
    let mut declarations = Vec::new();
    let mut visited: HashSet<Uuid> = HashSet::new();
    visited.insert(type_uuid);
    let mut frontier: Vec<Uuid> = supertypes.get(&type_uuid).cloned().unwrap_or_default();

    while !frontier.is_empty() {
        let mut next: Vec<Uuid> = Vec::new();
        for s in frontier {
            if !visited.insert(s) {
                continue;
            }
            let declared = declared_methods(methods_by_type, s, method);
            if declared.is_empty() {
                if let Some(parents) = supertypes.get(&s) {
                    next.extend(parents.iter().copied());
                }
            } else {
                declarations.extend(declared);
            }
        }
        if !declarations.is_empty() {
            break;
        }
        frontier = next;
    }
    declarations
}

fn apply_override_edges(entities: &mut [ResolutionEntity], edges: Vec<(usize, Uuid)>) {
    for (idx, decl_uuid) in edges {
        let rels = &mut entities[idx].relationships;
        if !rels.contains(&(decl_uuid, RelationshipType::Overrides)) {
            rels.push((decl_uuid, RelationshipType::Overrides));
        }
    }
}

/// Adds `subtype.method -[Overrides]-> supertype.method` edges for override-capable entities.
///
/// Runs AFTER type-level `Extends`/`Implements` edges are resolved and BEFORE
/// upsert. Pure in-memory, batch-local, additive: it only pushes new
/// `RelationshipType::Overrides` tuples onto method entities' `relationships`.
pub(crate) fn link_method_overrides(entities: &mut [ResolutionEntity]) {
    let (type_fqn_to_uuid, uuid_to_index) = build_type_index(entities);
    let (supertypes, methods_by_type) =
        collect_hierarchy(entities, &type_fqn_to_uuid, &uuid_to_index);

    let mut edges: Vec<(usize, Uuid)> = Vec::new();
    let mut emitted: HashSet<(usize, Uuid)> = HashSet::new();

    for (&type_uuid, methods) in &methods_by_type {
        for method in methods {
            let decls = nearest_declarations(type_uuid, method, &supertypes, &methods_by_type);
            for decl_uuid in decls {
                if emitted.insert((method.idx, decl_uuid)) {
                    edges.push((method.idx, decl_uuid));
                }
            }
        }
    }

    apply_override_edges(entities, edges);
}

#[cfg(test)]
mod tests {
    use super::super::test_utils::mock_resolution_entity_with_kind;
    use super::*;
    use crate::models::{EntityKind, RelationshipType};

    /// Convenience: build a type entity in an override-capable file.
    fn capable_type(name: &str, fqn: &str, file: &str, kind: EntityKind) -> ResolutionEntity {
        mock_resolution_entity_with_kind(name, fqn, None, file, kind)
    }

    /// Convenience: build a method entity in an override-capable file.
    fn capable_method(name: &str, fqn: &str, file: &str, kind: EntityKind) -> ResolutionEntity {
        mock_resolution_entity_with_kind(name, fqn, None, file, kind)
    }

    /// Convenience: build a method entity carrying a parameter signature.
    fn method_with_sig(
        name: &str,
        fqn: &str,
        file: &str,
        kind: EntityKind,
        signature: &str,
    ) -> ResolutionEntity {
        let mut m = capable_method(name, fqn, file, kind);
        m.signature = Some(signature.to_string());
        m
    }

    fn has_override(e: &ResolutionEntity, target: Uuid) -> bool {
        e.relationships
            .contains(&(target, RelationshipType::Overrides))
    }

    fn override_count(e: &ResolutionEntity) -> usize {
        e.relationships
            .iter()
            .filter(|(_, r)| *r == RelationshipType::Overrides)
            .count()
    }

    // --- Scenario A — Groovy interface implementation ---------------------
    #[test]
    fn scenario_a_groovy_interface_impl() {
        let iface = capable_type(
            "ISession",
            "nf.ISession",
            "ISession.groovy",
            EntityKind::GroovyInterface,
        );
        let iface_method = capable_method(
            "getUniqueId",
            "nf.ISession.getUniqueId",
            "ISession.groovy",
            EntityKind::GroovyMethod,
        );
        let mut cls = capable_type(
            "Session",
            "nf.Session",
            "Session.groovy",
            EntityKind::GroovyClass,
        );
        cls.relationships
            .push((iface.uuid, RelationshipType::Implements));
        let cls_method = capable_method(
            "getUniqueId",
            "nf.Session.getUniqueId",
            "Session.groovy",
            EntityKind::GroovyMethod,
        );

        let iface_method_uuid = iface_method.uuid;
        let mut entities = vec![iface, iface_method, cls, cls_method];
        link_method_overrides(&mut entities);

        // Session.getUniqueId -> ISession.getUniqueId
        assert!(has_override(&entities[3], iface_method_uuid));
        // ISession.getUniqueId has no outgoing OVERRIDES.
        assert_eq!(override_count(&entities[1]), 0);
    }

    // --- Scenario B — Java interface implementation -----------------------
    #[test]
    fn scenario_b_java_interface_impl() {
        let iface = capable_type(
            "Repository",
            "Repository",
            "Repo.java",
            EntityKind::Interface,
        );
        let iface_save = capable_method("save", "Repository.save", "Repo.java", EntityKind::Method);
        let mut cls = capable_type(
            "UserRepository",
            "UserRepository",
            "UserRepo.java",
            EntityKind::Class,
        );
        cls.relationships
            .push((iface.uuid, RelationshipType::Implements));
        let cls_save = capable_method(
            "save",
            "UserRepository.save",
            "UserRepo.java",
            EntityKind::Method,
        );

        let iface_save_uuid = iface_save.uuid;
        let mut entities = vec![iface, iface_save, cls, cls_save];
        link_method_overrides(&mut entities);

        assert!(has_override(&entities[3], iface_save_uuid));
    }

    // --- Scenario C — Kotlin superclass override --------------------------
    #[test]
    fn scenario_c_kotlin_superclass_override() {
        let base = capable_type("Base", "Base", "Base.kt", EntityKind::KotlinClass);
        let base_greet = capable_method("greet", "Base.greet", "Base.kt", EntityKind::KotlinMethod);
        let mut derived = capable_type("Derived", "Derived", "Derived.kt", EntityKind::KotlinClass);
        derived
            .relationships
            .push((base.uuid, RelationshipType::Extends));
        let derived_greet = capable_method(
            "greet",
            "Derived.greet",
            "Derived.kt",
            EntityKind::KotlinMethod,
        );

        let base_greet_uuid = base_greet.uuid;
        let mut entities = vec![base, base_greet, derived, derived_greet];
        link_method_overrides(&mut entities);

        assert!(has_override(&entities[3], base_greet_uuid));
    }

    // --- Scenario D — Multi-level hierarchy (nearest-declaration) ----------
    #[test]
    fn scenario_d_multilevel_nearest() {
        let a = capable_type("A", "A", "A.java", EntityKind::Interface);
        let a_run = capable_method("run", "A.run", "A.java", EntityKind::Method);
        let mut b = capable_type("B", "B", "B.java", EntityKind::Class);
        b.relationships.push((a.uuid, RelationshipType::Implements));
        let b_run = capable_method("run", "B.run", "B.java", EntityKind::Method);
        let mut c = capable_type("C", "C", "C.java", EntityKind::Class);
        c.relationships.push((b.uuid, RelationshipType::Extends));
        let c_run = capable_method("run", "C.run", "C.java", EntityKind::Method);

        let a_run_uuid = a_run.uuid;
        let b_run_uuid = b_run.uuid;
        let mut entities = vec![a, a_run, b, b_run, c, c_run];
        link_method_overrides(&mut entities);

        // C.run -> B.run (nearest only), NOT A.run.
        assert!(has_override(&entities[5], b_run_uuid));
        assert!(!has_override(&entities[5], a_run_uuid));
        assert_eq!(override_count(&entities[5]), 1);
        // B.run -> A.run.
        assert!(has_override(&entities[3], a_run_uuid));
    }

    // --- Scenario D2 — Skipped intermediate declaration (walk-through) -----
    #[test]
    fn scenario_d2_walk_through() {
        let a = capable_type("A", "A", "A.java", EntityKind::Interface);
        let a_run = capable_method("run", "A.run", "A.java", EntityKind::Method);
        // B implements A, but does NOT declare run.
        let mut b = capable_type("B", "B", "B.java", EntityKind::Class);
        b.relationships.push((a.uuid, RelationshipType::Implements));
        let mut c = capable_type("C", "C", "C.java", EntityKind::Class);
        c.relationships.push((b.uuid, RelationshipType::Extends));
        let c_run = capable_method("run", "C.run", "C.java", EntityKind::Method);

        let a_run_uuid = a_run.uuid;
        let mut entities = vec![a, a_run, b, c, c_run];
        link_method_overrides(&mut entities);

        // C.run -> A.run (B walked through transparently).
        assert!(has_override(&entities[4], a_run_uuid));
        assert_eq!(override_count(&entities[4]), 1);
    }

    // --- Scenario E — Diamond / cycle safety ------------------------------
    #[test]
    fn scenario_e_diamond_all_declare() {
        let top = capable_type("Top", "Top", "Top.java", EntityKind::Interface);
        let top_f = capable_method("f", "Top.f", "Top.java", EntityKind::Method);
        let mut left = capable_type("Left", "Left", "Left.java", EntityKind::Interface);
        left.relationships
            .push((top.uuid, RelationshipType::Extends));
        let left_f = capable_method("f", "Left.f", "Left.java", EntityKind::Method);
        let mut right = capable_type("Right", "Right", "Right.java", EntityKind::Interface);
        right
            .relationships
            .push((top.uuid, RelationshipType::Extends));
        let right_f = capable_method("f", "Right.f", "Right.java", EntityKind::Method);
        let mut impl_ = capable_type("Impl", "Impl", "Impl.java", EntityKind::Class);
        impl_
            .relationships
            .push((left.uuid, RelationshipType::Implements));
        impl_
            .relationships
            .push((right.uuid, RelationshipType::Implements));
        let impl_f = capable_method("f", "Impl.f", "Impl.java", EntityKind::Method);

        let top_f_uuid = top_f.uuid;
        let left_f_uuid = left_f.uuid;
        let right_f_uuid = right_f.uuid;
        let mut entities = vec![top, top_f, left, left_f, right, right_f, impl_, impl_f];
        link_method_overrides(&mut entities);

        // Impl.f -> Left.f and Right.f (nearest per path), not Top.f.
        let impl_f_e = &entities[7];
        assert!(has_override(impl_f_e, left_f_uuid));
        assert!(has_override(impl_f_e, right_f_uuid));
        assert!(!has_override(impl_f_e, top_f_uuid));
        assert_eq!(override_count(impl_f_e), 2);
        // Left.f -> Top.f, Right.f -> Top.f.
        assert!(has_override(&entities[3], top_f_uuid));
        assert!(has_override(&entities[5], top_f_uuid));
    }

    #[test]
    fn scenario_e_diamond_dedup_converging() {
        // Left/Right do NOT declare f -> Impl.f -> Top.f exactly once.
        let top = capable_type("Top", "Top", "Top.java", EntityKind::Interface);
        let top_f = capable_method("f", "Top.f", "Top.java", EntityKind::Method);
        let mut left = capable_type("Left", "Left", "Left.java", EntityKind::Interface);
        left.relationships
            .push((top.uuid, RelationshipType::Extends));
        let mut right = capable_type("Right", "Right", "Right.java", EntityKind::Interface);
        right
            .relationships
            .push((top.uuid, RelationshipType::Extends));
        let mut impl_ = capable_type("Impl", "Impl", "Impl.java", EntityKind::Class);
        impl_
            .relationships
            .push((left.uuid, RelationshipType::Implements));
        impl_
            .relationships
            .push((right.uuid, RelationshipType::Implements));
        let impl_f = capable_method("f", "Impl.f", "Impl.java", EntityKind::Method);

        let top_f_uuid = top_f.uuid;
        let mut entities = vec![top, top_f, left, right, impl_, impl_f];
        link_method_overrides(&mut entities);

        assert!(has_override(&entities[5], top_f_uuid));
        assert_eq!(override_count(&entities[5]), 1);
    }

    // --- Scenario F — FQN strip grouping (nested + no-match skip) ----------
    #[test]
    fn scenario_f_nested_class_grouping() {
        let outer_inner = capable_type("Inner", "Outer.Inner", "Outer.java", EntityKind::Interface);
        let inner_m = capable_method("m", "Outer.Inner.m", "Outer.java", EntityKind::Method);
        let mut sub = capable_type("Sub", "Sub", "Sub.java", EntityKind::Class);
        sub.relationships
            .push((outer_inner.uuid, RelationshipType::Implements));
        let sub_m = capable_method("m", "Sub.m", "Sub.java", EntityKind::Method);

        let inner_m_uuid = inner_m.uuid;
        let mut entities = vec![outer_inner, inner_m, sub, sub_m];
        link_method_overrides(&mut entities);

        assert!(has_override(&entities[3], inner_m_uuid));
    }

    #[test]
    fn scenario_f_unmatched_fqn_skipped() {
        // Method whose stripped FQN matches no type in the batch: no crash.
        let orphan = capable_method(
            "ghost",
            "NoSuchType.ghost",
            "Ghost.java",
            EntityKind::Method,
        );
        let mut entities = vec![orphan];
        link_method_overrides(&mut entities);
        assert_eq!(override_count(&entities[0]), 0);
    }

    // --- Scenario G — No false positives across unrelated types -----------
    #[test]
    fn scenario_g_unrelated_types() {
        let foo = capable_type("Foo", "Foo", "Foo.java", EntityKind::Class);
        let foo_p = capable_method("process", "Foo.process", "Foo.java", EntityKind::Method);
        let bar = capable_type("Bar", "Bar", "Bar.java", EntityKind::Class);
        let bar_p = capable_method("process", "Bar.process", "Bar.java", EntityKind::Method);

        let mut entities = vec![foo, foo_p, bar, bar_p];
        link_method_overrides(&mut entities);

        assert_eq!(override_count(&entities[1]), 0);
        assert_eq!(override_count(&entities[3]), 0);
    }

    // --- Scenario H — Overloads without signatures (name-only N×M fan-out) --
    #[test]
    fn scenario_h_overloads_without_signatures_fan_out() {
        let iface = capable_type("I", "I", "I.java", EntityKind::Interface);
        let i_visit_a = capable_method("visit", "I.visit", "I.java", EntityKind::Method);
        let i_visit_b = capable_method("visit", "I.visit", "I.java", EntityKind::Method);
        let mut impl_ = capable_type("Impl", "Impl", "Impl.java", EntityKind::Class);
        impl_
            .relationships
            .push((iface.uuid, RelationshipType::Implements));
        let impl_visit_a = capable_method("visit", "Impl.visit", "Impl.java", EntityKind::Method);
        let impl_visit_b = capable_method("visit", "Impl.visit", "Impl.java", EntityKind::Method);

        let ia = i_visit_a.uuid;
        let ib = i_visit_b.uuid;
        let mut entities = vec![
            iface,
            i_visit_a,
            i_visit_b,
            impl_,
            impl_visit_a,
            impl_visit_b,
        ];
        link_method_overrides(&mut entities);

        // No signature to compare: each Impl.visit links to both I.visit
        // declarations (N×M), the name-only fallback.
        assert!(has_override(&entities[4], ia));
        assert!(has_override(&entities[4], ib));
        assert!(has_override(&entities[5], ia));
        assert!(has_override(&entities[5], ib));
    }

    // --- Scenario H2 — Overloads with different arity (issue #30) ----------
    #[test]
    fn scenario_h2_arity_selects_the_overridden_overload() {
        let iface = capable_type(
            "SettingsService",
            "SettingsService",
            "SettingsService.java",
            EntityKind::Interface,
        );
        let one = method_with_sig(
            "update",
            "SettingsService.update",
            "SettingsService.java",
            EntityKind::Method,
            "(List<Setting> items)",
        );
        let two = method_with_sig(
            "update",
            "SettingsService.update",
            "SettingsService.java",
            EntityKind::Method,
            "(List<Setting> items, boolean skipValidation)",
        );
        let mut cls = capable_type(
            "DefaultSettingsService",
            "DefaultSettingsService",
            "DefaultSettingsService.java",
            EntityKind::Class,
        );
        cls.relationships
            .push((iface.uuid, RelationshipType::Implements));
        let impl_two = method_with_sig(
            "update",
            "DefaultSettingsService.update",
            "DefaultSettingsService.java",
            EntityKind::Method,
            "(List<Setting> items, boolean skipValidation)",
        );

        let (one_uuid, two_uuid) = (one.uuid, two.uuid);
        let mut entities = vec![iface, one, two, cls, impl_two];
        link_method_overrides(&mut entities);

        assert!(has_override(&entities[4], two_uuid));
        assert!(!has_override(&entities[4], one_uuid));
        assert_eq!(override_count(&entities[4]), 1);
    }

    // --- Scenario H3 — Walk past a type declaring only other arities -------
    #[test]
    fn scenario_h3_walks_past_incompatible_overloads() {
        let a = capable_type("A", "A", "A.java", EntityKind::Interface);
        let a_two = method_with_sig(
            "run",
            "A.run",
            "A.java",
            EntityKind::Method,
            "(int a, int b)",
        );
        let mut b = capable_type("B", "B", "B.java", EntityKind::Class);
        b.relationships.push((a.uuid, RelationshipType::Implements));
        let b_one = method_with_sig("run", "B.run", "B.java", EntityKind::Method, "(int a)");
        let mut c = capable_type("C", "C", "C.java", EntityKind::Class);
        c.relationships.push((b.uuid, RelationshipType::Extends));
        let c_two = method_with_sig(
            "run",
            "C.run",
            "C.java",
            EntityKind::Method,
            "(final int a, int b)",
        );

        let (a_two_uuid, b_one_uuid) = (a_two.uuid, b_one.uuid);
        let mut entities = vec![a, a_two, b, b_one, c, c_two];
        link_method_overrides(&mut entities);

        // C.run(int, int) skips B.run(int) and reaches A.run(int, int).
        assert!(has_override(&entities[5], a_two_uuid));
        assert!(!has_override(&entities[5], b_one_uuid));
        assert_eq!(override_count(&entities[5]), 1);
        // B.run(int) overrides nothing: A only declares the two-arg overload.
        assert_eq!(override_count(&entities[3]), 0);
    }

    // --- Scenario H4 — Same arity, parameter types decide ------------------
    #[test]
    fn scenario_h4_parameter_types_break_same_arity_ties() {
        let iface = capable_type(
            "Renderer",
            "Renderer",
            "Renderer.java",
            EntityKind::Interface,
        );
        let by_string = method_with_sig(
            "render",
            "Renderer.render",
            "Renderer.java",
            EntityKind::Method,
            "(String value)",
        );
        let by_long = method_with_sig(
            "render",
            "Renderer.render",
            "Renderer.java",
            EntityKind::Method,
            "(Long value)",
        );
        let mut cls = capable_type("Plain", "Plain", "Plain.java", EntityKind::Class);
        cls.relationships
            .push((iface.uuid, RelationshipType::Implements));
        let impl_string = method_with_sig(
            "render",
            "Plain.render",
            "Plain.java",
            EntityKind::Method,
            "(String value)",
        );

        let by_string_uuid = by_string.uuid;
        let mut entities = vec![iface, by_string, by_long, cls, impl_string];
        link_method_overrides(&mut entities);

        assert!(has_override(&entities[4], by_string_uuid));
        assert_eq!(override_count(&entities[4]), 1);
    }

    // --- Scenario H5 — Type variables, most specific overload wins ---------
    #[test]
    fn scenario_h5_type_variables_prefer_the_most_specific_overload() {
        let iface = capable_type("Store", "Store", "Store.kt", EntityKind::KotlinInterface);
        let by_var = method_with_sig(
            "put",
            "Store.put",
            "Store.kt",
            EntityKind::KotlinMethod,
            "(entry: T)",
        );
        let by_map = method_with_sig(
            "put",
            "Store.put",
            "Store.kt",
            EntityKind::KotlinMethod,
            "(entries: Map<String, T>)",
        );
        let mut cls = capable_type(
            "SettingStore",
            "SettingStore",
            "SettingStore.kt",
            EntityKind::KotlinClass,
        );
        cls.relationships
            .push((iface.uuid, RelationshipType::Implements));
        let impl_entry = method_with_sig(
            "put",
            "SettingStore.put",
            "SettingStore.kt",
            EntityKind::KotlinMethod,
            "(entry: Setting)",
        );
        let impl_map = method_with_sig(
            "put",
            "SettingStore.put",
            "SettingStore.kt",
            EntityKind::KotlinMethod,
            "(entries: Map<String, Setting>)",
        );

        let (var_uuid, map_uuid) = (by_var.uuid, by_map.uuid);
        let mut entities = vec![iface, by_var, by_map, cls, impl_entry, impl_map];
        link_method_overrides(&mut entities);

        assert!(has_override(&entities[4], var_uuid));
        assert_eq!(override_count(&entities[4]), 1);
        assert!(has_override(&entities[5], map_uuid));
        assert_eq!(override_count(&entities[5]), 1);
    }

    // --- Scenario H6 — Groovy defaults and the uncomparable fallback -------
    #[test]
    fn scenario_h6_groovy_default_parameters_and_missing_signatures() {
        let base = capable_type("Layout", "Layout", "Layout.groovy", EntityKind::GroovyClass);
        let with_default = method_with_sig(
            "layoutRow",
            "Layout.layoutRow",
            "Layout.groovy",
            EntityKind::GroovyMethod,
            "String layoutRow(String row, int indent = 2)",
        );
        let by_list = method_with_sig(
            "layoutRow",
            "Layout.layoutRow",
            "Layout.groovy",
            EntityKind::GroovyMethod,
            "String layoutRow(List<String> rows)",
        );
        let no_sig = capable_method(
            "getTitle",
            "Layout.getTitle",
            "Layout.groovy",
            EntityKind::GroovyMethod,
        );
        let mut sub = capable_type(
            "Compact",
            "Compact",
            "Compact.groovy",
            EntityKind::GroovyClass,
        );
        sub.relationships
            .push((base.uuid, RelationshipType::Extends));
        let sub_row = method_with_sig(
            "layoutRow",
            "Compact.layoutRow",
            "Compact.groovy",
            EntityKind::GroovyMethod,
            "String layoutRow(String row)",
        );
        let sub_title = method_with_sig(
            "getTitle",
            "Compact.getTitle",
            "Compact.groovy",
            EntityKind::GroovyMethod,
            "String getTitle()",
        );

        let (default_uuid, no_sig_uuid) = (with_default.uuid, no_sig.uuid);
        let mut entities = vec![base, with_default, by_list, no_sig, sub, sub_row, sub_title];
        link_method_overrides(&mut entities);

        // `layoutRow(String)` is one of the overloads the default generates.
        assert!(has_override(&entities[5], default_uuid));
        assert_eq!(override_count(&entities[5]), 1);
        // A supertype declaration without signature falls back to the name.
        assert!(has_override(&entities[6], no_sig_uuid));
    }

    // --- Scenario H7 — Real parser signatures (Java parser + resolver) -----
    #[test]
    fn scenario_h7_java_parser_signatures_issue_30() {
        use crate::pipeline::parser::{ParseConfig, parse_files};

        let dir = std::env::temp_dir().join(format!("knot_issue30_{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("SettingsService.java"),
            "package demo;\nimport java.util.List;\npublic interface SettingsService {\n    default void update(List<Setting> items) { update(items, false); }\n    void update(List<Setting> items, boolean skipValidation);\n}\n",
        )
        .unwrap();
        std::fs::write(
            dir.join("DefaultSettingsService.java"),
            "package demo;\nimport java.util.List;\npublic class DefaultSettingsService implements SettingsService {\n    @Override\n    public void update(List<Setting> items, boolean skipValidation) { }\n}\n",
        )
        .unwrap();
        let cfg = ParseConfig {
            repo_root: dir.clone(),
            custom_queries_path: None,
            repo_name: "issue30".into(),
            include_config_files: false,
            repo_path: None,
        };
        let files = vec![
            dir.join("SettingsService.java"),
            dir.join("DefaultSettingsService.java"),
        ];
        let parsed = parse_files(&files, &cfg);
        std::fs::remove_dir_all(&dir).unwrap();

        let mut entities: Vec<ResolutionEntity> =
            parsed.iter().map(ResolutionEntity::from).collect();
        super::super::resolve_reference_intents(&mut entities);

        let by_sig = |fqn: &str, sig: &str| {
            entities
                .iter()
                .find(|e| e.fqn == fqn && e.signature.as_deref() == Some(sig))
                .unwrap_or_else(|| panic!("missing {fqn}{sig}"))
        };
        let iface_one = by_sig("demo.SettingsService.update", "(List<Setting> items)").uuid;
        let iface_two = by_sig(
            "demo.SettingsService.update",
            "(List<Setting> items, boolean skipValidation)",
        )
        .uuid;
        let impl_two = by_sig(
            "demo.DefaultSettingsService.update",
            "(List<Setting> items, boolean skipValidation)",
        );
        assert!(has_override(impl_two, iface_two));
        assert!(!has_override(impl_two, iface_one));
        assert_eq!(override_count(impl_two), 1);
    }

    // --- Scenario I — No supertype method match ---------------------------
    #[test]
    fn scenario_i_no_supertype_method() {
        let iface = capable_type("I", "I", "I.java", EntityKind::Interface);
        // It declares nothing relevant.
        let mut cls = capable_type("C", "C", "C.java", EntityKind::Class);
        cls.relationships
            .push((iface.uuid, RelationshipType::Implements));
        let helper = capable_method("helper", "C.helper", "C.java", EntityKind::Method);

        let mut entities = vec![iface, cls, helper];
        link_method_overrides(&mut entities);

        assert_eq!(override_count(&entities[2]), 0);
    }

    // --- Scenario J — Constructors are excluded ---------------------------
    #[test]
    fn scenario_j_constructors_excluded() {
        let base = capable_type("Base", "Base", "Base.java", EntityKind::Class);
        // Constructor: name == type name.
        let base_ctor = capable_method("Base", "Base.Base", "Base.java", EntityKind::Method);
        let mut sub = capable_type("Sub", "Sub", "Sub.java", EntityKind::Class);
        sub.relationships
            .push((base.uuid, RelationshipType::Extends));
        // <init>-style constructor.
        let sub_ctor = capable_method("Sub", "Sub.Sub", "Sub.java", EntityKind::Method);
        let sub_init = capable_method("<init>", "Sub.<init>", "Sub.java", EntityKind::Method);

        let mut entities = vec![base, base_ctor, sub, sub_ctor, sub_init];
        link_method_overrides(&mut entities);

        assert_eq!(override_count(&entities[3]), 0);
        assert_eq!(override_count(&entities[4]), 0);
    }

    // --- Scenario K — Static/private methods (documented limitation) ------
    #[test]
    fn scenario_k_static_hiding_pinned() {
        // Modifiers are not persisted; name-only cannot distinguish hiding.
        let base = capable_type("Base", "Base", "Base.java", EntityKind::Class);
        let base_util = capable_method("util", "Base.util", "Base.java", EntityKind::Method);
        let mut sub = capable_type("Sub", "Sub", "Sub.java", EntityKind::Class);
        sub.relationships
            .push((base.uuid, RelationshipType::Extends));
        let sub_util = capable_method("util", "Sub.util", "Sub.java", EntityKind::Method);

        let base_util_uuid = base_util.uuid;
        let mut entities = vec![base, base_util, sub, sub_util];
        link_method_overrides(&mut entities);

        // Current behavior: links (static hiding indistinguishable today).
        assert!(has_override(&entities[3], base_util_uuid));
    }

    // --- Scenario M — Incremental indexing limitation ---------------------
    #[test]
    fn scenario_m_incremental_batch_missing_supertype() {
        // Only the subtype is in the batch; supertype methods absent.
        let mut cls = capable_type(
            "Session",
            "nf.Session",
            "Session.groovy",
            EntityKind::GroovyClass,
        );
        // Implements target uuid points to an entity not in this batch.
        cls.relationships
            .push((Uuid::new_v4(), RelationshipType::Implements));
        let cls_method = capable_method(
            "getUniqueId",
            "nf.Session.getUniqueId",
            "Session.groovy",
            EntityKind::GroovyMethod,
        );

        let mut entities = vec![cls, cls_method];
        link_method_overrides(&mut entities); // must not panic
        assert_eq!(override_count(&entities[1]), 0);
    }

    // --- Scenario N — Non-participating languages are unaffected ----------
    #[test]
    fn scenario_n_non_participating_unaffected() {
        // Rust trait + impl method.
        let rust_trait = capable_type("Greeter", "crate::Greeter", "lib.rs", EntityKind::RustTrait);
        let rust_trait_m = capable_method(
            "greet",
            "crate::Greeter::greet",
            "lib.rs",
            EntityKind::RustMethod,
        );
        // Python base/derived.
        let py_base = capable_type("Base", "Base", "base.py", EntityKind::PythonClass);
        let py_base_m = capable_method("run", "Base.run", "base.py", EntityKind::PythonMethod);
        let mut py_derived =
            capable_type("Derived", "Derived", "derived.py", EntityKind::PythonClass);
        py_derived
            .relationships
            .push((py_base.uuid, RelationshipType::Extends));
        let py_derived_m =
            capable_method("run", "Derived.run", "derived.py", EntityKind::PythonMethod);
        // TypeScript interface + class (shares generic Class/Interface/Method).
        let ts_iface = capable_type("Repo", "Repo", "repo.ts", EntityKind::Interface);
        let ts_iface_m = capable_method("save", "Repo.save", "repo.ts", EntityKind::Method);
        let mut ts_cls = capable_type("UserRepo", "UserRepo", "user.ts", EntityKind::Class);
        ts_cls
            .relationships
            .push((ts_iface.uuid, RelationshipType::Implements));
        let ts_cls_m = capable_method("save", "UserRepo.save", "user.ts", EntityKind::Method);

        let mut entities = vec![
            rust_trait,
            rust_trait_m,
            py_base,
            py_base_m,
            py_derived,
            py_derived_m,
            ts_iface,
            ts_iface_m,
            ts_cls,
            ts_cls_m,
        ];
        link_method_overrides(&mut entities);

        for e in &entities {
            assert_eq!(
                override_count(e),
                0,
                "non-participating entity {} gained an OVERRIDES edge",
                e.fqn
            );
        }
    }

    // --- Helper unit tests ------------------------------------------------
    #[test]
    fn enclosing_type_fqn_cases() {
        assert_eq!(
            enclosing_type_fqn("Outer.Inner.m", "m"),
            Some("Outer.Inner")
        );
        assert_eq!(
            enclosing_type_fqn("pkg.Class.method", "method"),
            Some("pkg.Class")
        );
        assert_eq!(
            enclosing_type_fqn("Foo.bar.<anonymous@30>.foo", "foo"),
            Some("Foo.bar.<anonymous@30>")
        );
        // Top-level entity with no enclosing type.
        assert_eq!(enclosing_type_fqn("topLevel", "topLevel"), None);
    }

    #[test]
    fn is_override_capable_file_guard() {
        assert!(is_override_capable_file("a/b/C.java"));
        assert!(is_override_capable_file("x.kt"));
        assert!(is_override_capable_file("x.kts"));
        assert!(is_override_capable_file("x.groovy"));
        assert!(is_override_capable_file("x.gvy"));
        assert!(is_override_capable_file("build.gradle"));
        assert!(is_override_capable_file("Services/UserService.cs"));
        assert!(!is_override_capable_file("x.ts"));
        assert!(!is_override_capable_file("x.py"));
        assert!(!is_override_capable_file("lib.rs"));
    }

    // --- Scenario O — Groovy property accessor overrides interface getter ---
    #[test]
    fn scenario_o_groovy_property_accessor_overrides_interface_getter() {
        let iface = capable_type(
            "ISession",
            "nf.ISession",
            "ISession.groovy",
            EntityKind::GroovyInterface,
        );
        let iface_method = capable_method(
            "getBaseDir",
            "nf.ISession.getBaseDir",
            "ISession.groovy",
            EntityKind::GroovyMethod,
        );
        let mut cls = capable_type(
            "Session",
            "nf.Session",
            "Session.groovy",
            EntityKind::GroovyClass,
        );
        cls.relationships
            .push((iface.uuid, RelationshipType::Implements));
        // Synthetic accessor from Session.baseDir
        let cls_getter = capable_method(
            "getBaseDir",
            "nf.Session.getBaseDir",
            "Session.groovy",
            EntityKind::GroovyMethod,
        );
        // The property itself — must NOT get an Overrides edge
        let _cls_prop = capable_method(
            "baseDir",
            "nf.Session.baseDir",
            "Session.groovy",
            EntityKind::GroovyProperty,
        );

        let iface_method_uuid = iface_method.uuid;
        let mut entities = vec![iface, iface_method, cls, cls_getter, _cls_prop];
        link_method_overrides(&mut entities);

        // Session.getBaseDir -> ISession.getBaseDir
        assert!(has_override(&entities[3], iface_method_uuid));
        // The property node itself should have ZERO Overrides edges.
        assert_eq!(override_count(&entities[4]), 0);
    }

    #[test]
    fn scenario_o2_property_node_itself_does_not_override() {
        // GIVEN ISession with getBaseDir, AND Session with baseDir property
        // (NOT the synthetic getter), WHEN link_method_overrides runs,
        // THEN the GroovyProperty entity gains ZERO Overrides edges.
        let iface = capable_type(
            "ISession",
            "nf.ISession",
            "ISession.groovy",
            EntityKind::GroovyInterface,
        );
        let iface_method = capable_method(
            "getBaseDir",
            "nf.ISession.getBaseDir",
            "ISession.groovy",
            EntityKind::GroovyMethod,
        );
        let mut cls = capable_type(
            "Session",
            "nf.Session",
            "Session.groovy",
            EntityKind::GroovyClass,
        );
        cls.relationships
            .push((iface.uuid, RelationshipType::Implements));
        // Only the property node, no synthetic getter
        let cls_prop = capable_method(
            "baseDir",
            "nf.Session.baseDir",
            "Session.groovy",
            EntityKind::GroovyProperty,
        );

        let mut entities = vec![iface, iface_method, cls, cls_prop];
        link_method_overrides(&mut entities);

        assert_eq!(override_count(&entities[3]), 0);
    }

    #[test]
    fn nextflow_isession_getbasedir_regression() {
        // End-to-end parser + overrides integration test reproducing the
        // reported bug: Session.baseDir must override ISession.getBaseDir.
        // We build the entities manually (mimicking what the Groovy parser
        // now produces after Phase 1-3).
        let iface = capable_type(
            "ISession",
            "nf.ISession",
            "ISession.groovy",
            EntityKind::GroovyInterface,
        );
        let iface_get_base_dir = capable_method(
            "getBaseDir",
            "nf.ISession.getBaseDir",
            "ISession.groovy",
            EntityKind::GroovyMethod,
        );
        let iface_get_script_name = capable_method(
            "getScriptName",
            "nf.ISession.getScriptName",
            "ISession.groovy",
            EntityKind::GroovyMethod,
        );

        let mut cls = capable_type(
            "Session",
            "nf.Session",
            "Session.groovy",
            EntityKind::GroovyClass,
        );
        cls.relationships
            .push((iface.uuid, RelationshipType::Implements));

        let cls_base_dir = capable_method(
            "baseDir",
            "nf.Session.baseDir",
            "Session.groovy",
            EntityKind::GroovyProperty,
        );
        let cls_script_name = capable_method(
            "scriptName",
            "nf.Session.scriptName",
            "Session.groovy",
            EntityKind::GroovyProperty,
        );
        // Synthetic getters emitted by the parser
        let cls_get_base_dir = capable_method(
            "getBaseDir",
            "nf.Session.getBaseDir",
            "Session.groovy",
            EntityKind::GroovyMethod,
        );
        let cls_get_script_name = capable_method(
            "getScriptName",
            "nf.Session.getScriptName",
            "Session.groovy",
            EntityKind::GroovyMethod,
        );
        // Real setter suppresses synthetic one
        let cls_set_base_dir = capable_method(
            "setBaseDir",
            "nf.Session.setBaseDir",
            "Session.groovy",
            EntityKind::GroovyMethod,
        );

        let iface_get_base_dir_uuid = iface_get_base_dir.uuid;
        let iface_get_script_name_uuid = iface_get_script_name.uuid;

        let mut entities = vec![
            iface,
            iface_get_base_dir,
            iface_get_script_name,
            cls,
            cls_base_dir,
            cls_script_name,
            cls_get_base_dir,
            cls_get_script_name,
            cls_set_base_dir,
        ];
        link_method_overrides(&mut entities);

        // Session.getBaseDir -[Overrides]-> ISession.getBaseDir
        assert!(
            has_override(&entities[6], iface_get_base_dir_uuid),
            "Session.getBaseDir must override ISession.getBaseDir"
        );
        // Session.getScriptName -[Overrides]-> ISession.getScriptName
        assert!(
            has_override(&entities[7], iface_get_script_name_uuid),
            "Session.getScriptName must override ISession.getScriptName"
        );
        // The property nodes themselves must NOT carry Overrides edges
        assert_eq!(
            override_count(&entities[4]),
            0,
            "GroovyProperty baseDir must not have Overrides"
        );
        assert_eq!(
            override_count(&entities[5]),
            0,
            "GroovyProperty scriptName must not have Overrides"
        );
        // No phantom entity named 'name' should exist in any test data
        // (verified by construction — we never create such entities here)
    }
}
