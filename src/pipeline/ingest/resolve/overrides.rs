//! Method-level `OVERRIDES` linking for JVM languages (Java, Kotlin, Groovy).
//!
//! knot models inheritance only at the type level (`EXTENDS` / `IMPLEMENTS`).
//! This pass adds an additive, method-granular edge
//! `subtype.method -[OVERRIDES]-> supertype.method` so that reverse-dependency
//! queries can surface the relationship between an interface/superclass method
//! and its implementations/overrides.
//!
//! Design (see `docs/specs/method_override_relationships.md`):
//! - **Additive only** — never mutates or removes existing entities/edges.
//! - **JVM guard** — a file-extension guard plus a kind allowlist. The generic
//!   `Class`/`Interface`/`Method` kinds are shared with TypeScript, so the
//!   extension guard is what keeps non-JVM languages at zero `OVERRIDES` edges.
//! - **Nearest-declaration linking** — each method links only to the nearest
//!   declaration(s) of the same name walking up its type hierarchy. Full
//!   transitive visibility is resolved at query time with variable-length
//!   Cypher (`-[:OVERRIDES*1..]->`).
//! - **Name-only matching** — the Groovy parser often lacks full signatures.
//! - **Batch-local** — only links methods whose supertype methods are present
//!   in the same indexing batch (incremental limitation, documented in the spec).

use std::collections::{HashMap, HashSet};

use uuid::Uuid;

use crate::models::{EntityKind, RelationshipType, ResolutionEntity};

/// JVM source file extensions. The guard is applied FIRST to every entity;
/// this is what excludes TypeScript, which shares the generic
/// `Class`/`Interface`/`Method` kinds.
const JVM_EXTENSIONS: &[&str] = &[".java", ".kt", ".kts", ".groovy", ".gvy", ".gradle"];

/// Returns `true` if the file path ends in a JVM source extension.
fn is_jvm_file(file_path: &str) -> bool {
    JVM_EXTENSIONS.iter().any(|ext| file_path.ends_with(ext))
}

/// JVM method-like kinds that can override/implement a supertype method.
///
/// `KotlinFunction` is intentionally excluded: top-level and extension
/// functions are statically dispatched and cannot override.
fn is_jvm_method_like(kind: &EntityKind) -> bool {
    matches!(
        kind,
        EntityKind::Method | EntityKind::KotlinMethod | EntityKind::GroovyMethod
    )
}

/// JVM type-like kinds that can declare overridable methods and participate in
/// an inheritance hierarchy (includes Kotlin objects/companions and enums).
fn is_jvm_type_like(kind: &EntityKind) -> bool {
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
    )
}

/// Strips a method FQN down to its enclosing type FQN.
///
/// All three JVM parsers emit `method_fqn == type_fqn + "." + method_name`
/// (including nested classes `Outer.Inner.method` and Kotlin anonymous objects
/// `Foo.bar.<anonymous@30>.foo`). Returns `None` for top-level entities with no
/// enclosing type (e.g. a method FQN equal to its own name).
fn enclosing_type_fqn<'a>(method_fqn: &'a str, method_name: &str) -> Option<&'a str> {
    // Prefer stripping the exact ".<method_name>" suffix (robust for names that
    // themselves contain dots, though JVM method names do not).
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
/// Uses the name-based heuristic that covers all JVM parsers: the method name
/// equals its enclosing type's name, or equals `<init>`.
fn is_constructor(method_name: &str, enclosing_type_name: &str) -> bool {
    method_name == "<init>" || method_name == enclosing_type_name
}

/// Adds `subtype.method -[Overrides]-> supertype.method` edges for JVM entities.
///
/// Runs AFTER type-level `Extends`/`Implements` edges are resolved and BEFORE
/// upsert. Pure in-memory, batch-local, additive: it only pushes new
/// `RelationshipType::Overrides` tuples onto method entities' `relationships`.
pub(crate) fn link_method_overrides(entities: &mut [ResolutionEntity]) {
    // Edges to apply in phase 2: (method entity index, declaration uuid).
    let edges: Vec<(usize, Uuid)> = {
        // --- Phase 1: immutable analysis ---------------------------------

        // Map JVM type FQN -> type uuid (types only; methods are never looked
        // up by FQN, since overloads share an FQN). Restricting to JVM files
        // prevents a same-FQN non-JVM type from being matched.
        let mut type_fqn_to_uuid: HashMap<&str, Uuid> = HashMap::new();
        // uuid -> index, for name lookups (constructor check) and presence tests.
        let mut uuid_to_index: HashMap<Uuid, usize> = HashMap::new();

        for (idx, e) in entities.iter().enumerate() {
            uuid_to_index.insert(e.uuid, idx);
            if is_jvm_file(&e.file_path) && is_jvm_type_like(&e.kind) {
                type_fqn_to_uuid.insert(e.fqn.as_str(), e.uuid);
            }
        }

        // supertypes: type uuid -> resolved Extends/Implements target uuids.
        let mut supertypes: HashMap<Uuid, Vec<Uuid>> = HashMap::new();
        // methods_by_type: type uuid -> [(method name, method uuid, method idx)].
        let mut methods_by_type: HashMap<Uuid, Vec<(&str, Uuid, usize)>> = HashMap::new();

        for (idx, e) in entities.iter().enumerate() {
            if !is_jvm_file(&e.file_path) {
                continue;
            }

            if is_jvm_type_like(&e.kind) {
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
            } else if is_jvm_method_like(&e.kind) {
                let Some(type_fqn) = enclosing_type_fqn(&e.fqn, &e.name) else {
                    continue;
                };
                let Some(&type_uuid) = type_fqn_to_uuid.get(type_fqn) else {
                    continue; // stripped FQN matches no type in the batch: skip
                };
                // Constructor exclusion (name == enclosing type name, or <init>).
                if let Some(&type_idx) = uuid_to_index.get(&type_uuid)
                    && is_constructor(&e.name, &entities[type_idx].name)
                {
                    continue;
                }
                methods_by_type
                    .entry(type_uuid)
                    .or_default()
                    .push((e.name.as_str(), e.uuid, idx));
            }
        }

        // Compute edges via nearest-declaration BFS.
        let mut edges: Vec<(usize, Uuid)> = Vec::new();
        let mut emitted: HashSet<(usize, Uuid)> = HashSet::new();

        for (&type_uuid, methods) in &methods_by_type {
            for &(method_name, _method_uuid, method_idx) in methods {
                // BFS up the hierarchy: link to the nearest declaration(s),
                // walking through supertypes that do not declare the method.
                let mut visited: HashSet<Uuid> = HashSet::new();
                visited.insert(type_uuid);
                let mut frontier: Vec<Uuid> =
                    supertypes.get(&type_uuid).cloned().unwrap_or_default();

                while !frontier.is_empty() {
                    let mut next: Vec<Uuid> = Vec::new();
                    for s in frontier {
                        if !visited.insert(s) {
                            continue;
                        }
                        let declared: Vec<Uuid> = methods_by_type
                            .get(&s)
                            .map(|ms| {
                                ms.iter()
                                    .filter(|(name, _, _)| *name == method_name)
                                    .map(|(_, uuid, _)| *uuid)
                                    .collect()
                            })
                            .unwrap_or_default();

                        if declared.is_empty() {
                            // Walk through: this supertype does not declare it.
                            if let Some(parents) = supertypes.get(&s) {
                                next.extend(parents.iter().copied());
                            }
                        } else {
                            // Nearest declaration found: emit, do NOT expand.
                            for decl_uuid in declared {
                                if emitted.insert((method_idx, decl_uuid)) {
                                    edges.push((method_idx, decl_uuid));
                                }
                            }
                        }
                    }
                    frontier = next;
                }
            }
        }

        edges
    };

    // --- Phase 2: mutation -----------------------------------------------
    for (idx, decl_uuid) in edges {
        let rels = &mut entities[idx].relationships;
        if !rels.contains(&(decl_uuid, RelationshipType::Overrides)) {
            rels.push((decl_uuid, RelationshipType::Overrides));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_utils::mock_resolution_entity_with_kind;
    use super::*;
    use crate::models::{EntityKind, RelationshipType};

    /// Convenience: build a type entity in a JVM file.
    fn jvm_type(name: &str, fqn: &str, file: &str, kind: EntityKind) -> ResolutionEntity {
        mock_resolution_entity_with_kind(name, fqn, None, file, kind)
    }

    /// Convenience: build a method entity in a JVM file.
    fn jvm_method(name: &str, fqn: &str, file: &str, kind: EntityKind) -> ResolutionEntity {
        mock_resolution_entity_with_kind(name, fqn, None, file, kind)
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
        let iface = jvm_type(
            "ISession",
            "nf.ISession",
            "ISession.groovy",
            EntityKind::GroovyInterface,
        );
        let iface_method = jvm_method(
            "getUniqueId",
            "nf.ISession.getUniqueId",
            "ISession.groovy",
            EntityKind::GroovyMethod,
        );
        let mut cls = jvm_type(
            "Session",
            "nf.Session",
            "Session.groovy",
            EntityKind::GroovyClass,
        );
        cls.relationships
            .push((iface.uuid, RelationshipType::Implements));
        let cls_method = jvm_method(
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
        let iface = jvm_type(
            "Repository",
            "Repository",
            "Repo.java",
            EntityKind::Interface,
        );
        let iface_save = jvm_method("save", "Repository.save", "Repo.java", EntityKind::Method);
        let mut cls = jvm_type(
            "UserRepository",
            "UserRepository",
            "UserRepo.java",
            EntityKind::Class,
        );
        cls.relationships
            .push((iface.uuid, RelationshipType::Implements));
        let cls_save = jvm_method(
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
        let base = jvm_type("Base", "Base", "Base.kt", EntityKind::KotlinClass);
        let base_greet = jvm_method("greet", "Base.greet", "Base.kt", EntityKind::KotlinMethod);
        let mut derived = jvm_type("Derived", "Derived", "Derived.kt", EntityKind::KotlinClass);
        derived
            .relationships
            .push((base.uuid, RelationshipType::Extends));
        let derived_greet = jvm_method(
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
        let a = jvm_type("A", "A", "A.java", EntityKind::Interface);
        let a_run = jvm_method("run", "A.run", "A.java", EntityKind::Method);
        let mut b = jvm_type("B", "B", "B.java", EntityKind::Class);
        b.relationships.push((a.uuid, RelationshipType::Implements));
        let b_run = jvm_method("run", "B.run", "B.java", EntityKind::Method);
        let mut c = jvm_type("C", "C", "C.java", EntityKind::Class);
        c.relationships.push((b.uuid, RelationshipType::Extends));
        let c_run = jvm_method("run", "C.run", "C.java", EntityKind::Method);

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
        let a = jvm_type("A", "A", "A.java", EntityKind::Interface);
        let a_run = jvm_method("run", "A.run", "A.java", EntityKind::Method);
        // B implements A but does NOT declare run.
        let mut b = jvm_type("B", "B", "B.java", EntityKind::Class);
        b.relationships.push((a.uuid, RelationshipType::Implements));
        let mut c = jvm_type("C", "C", "C.java", EntityKind::Class);
        c.relationships.push((b.uuid, RelationshipType::Extends));
        let c_run = jvm_method("run", "C.run", "C.java", EntityKind::Method);

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
        let top = jvm_type("Top", "Top", "Top.java", EntityKind::Interface);
        let top_f = jvm_method("f", "Top.f", "Top.java", EntityKind::Method);
        let mut left = jvm_type("Left", "Left", "Left.java", EntityKind::Interface);
        left.relationships
            .push((top.uuid, RelationshipType::Extends));
        let left_f = jvm_method("f", "Left.f", "Left.java", EntityKind::Method);
        let mut right = jvm_type("Right", "Right", "Right.java", EntityKind::Interface);
        right
            .relationships
            .push((top.uuid, RelationshipType::Extends));
        let right_f = jvm_method("f", "Right.f", "Right.java", EntityKind::Method);
        let mut impl_ = jvm_type("Impl", "Impl", "Impl.java", EntityKind::Class);
        impl_
            .relationships
            .push((left.uuid, RelationshipType::Implements));
        impl_
            .relationships
            .push((right.uuid, RelationshipType::Implements));
        let impl_f = jvm_method("f", "Impl.f", "Impl.java", EntityKind::Method);

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
        let top = jvm_type("Top", "Top", "Top.java", EntityKind::Interface);
        let top_f = jvm_method("f", "Top.f", "Top.java", EntityKind::Method);
        let mut left = jvm_type("Left", "Left", "Left.java", EntityKind::Interface);
        left.relationships
            .push((top.uuid, RelationshipType::Extends));
        let mut right = jvm_type("Right", "Right", "Right.java", EntityKind::Interface);
        right
            .relationships
            .push((top.uuid, RelationshipType::Extends));
        let mut impl_ = jvm_type("Impl", "Impl", "Impl.java", EntityKind::Class);
        impl_
            .relationships
            .push((left.uuid, RelationshipType::Implements));
        impl_
            .relationships
            .push((right.uuid, RelationshipType::Implements));
        let impl_f = jvm_method("f", "Impl.f", "Impl.java", EntityKind::Method);

        let top_f_uuid = top_f.uuid;
        let mut entities = vec![top, top_f, left, right, impl_, impl_f];
        link_method_overrides(&mut entities);

        assert!(has_override(&entities[5], top_f_uuid));
        assert_eq!(override_count(&entities[5]), 1);
    }

    // --- Scenario F — FQN strip grouping (nested + no-match skip) ----------
    #[test]
    fn scenario_f_nested_class_grouping() {
        let outer_inner = jvm_type("Inner", "Outer.Inner", "Outer.java", EntityKind::Interface);
        let inner_m = jvm_method("m", "Outer.Inner.m", "Outer.java", EntityKind::Method);
        let mut sub = jvm_type("Sub", "Sub", "Sub.java", EntityKind::Class);
        sub.relationships
            .push((outer_inner.uuid, RelationshipType::Implements));
        let sub_m = jvm_method("m", "Sub.m", "Sub.java", EntityKind::Method);

        let inner_m_uuid = inner_m.uuid;
        let mut entities = vec![outer_inner, inner_m, sub, sub_m];
        link_method_overrides(&mut entities);

        assert!(has_override(&entities[3], inner_m_uuid));
    }

    #[test]
    fn scenario_f_unmatched_fqn_skipped() {
        // Method whose stripped FQN matches no type in the batch: no crash.
        let orphan = jvm_method(
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
        let foo = jvm_type("Foo", "Foo", "Foo.java", EntityKind::Class);
        let foo_p = jvm_method("process", "Foo.process", "Foo.java", EntityKind::Method);
        let bar = jvm_type("Bar", "Bar", "Bar.java", EntityKind::Class);
        let bar_p = jvm_method("process", "Bar.process", "Bar.java", EntityKind::Method);

        let mut entities = vec![foo, foo_p, bar, bar_p];
        link_method_overrides(&mut entities);

        assert_eq!(override_count(&entities[1]), 0);
        assert_eq!(override_count(&entities[3]), 0);
    }

    // --- Scenario H — Overloads (name-only N×M fan-out) -------------------
    #[test]
    fn scenario_h_overload_fanout() {
        let iface = jvm_type("I", "I", "I.java", EntityKind::Interface);
        let i_visit_a = jvm_method("visit", "I.visit", "I.java", EntityKind::Method);
        let i_visit_b = jvm_method("visit", "I.visit", "I.java", EntityKind::Method);
        let mut impl_ = jvm_type("Impl", "Impl", "Impl.java", EntityKind::Class);
        impl_
            .relationships
            .push((iface.uuid, RelationshipType::Implements));
        let impl_visit_a = jvm_method("visit", "Impl.visit", "Impl.java", EntityKind::Method);
        let impl_visit_b = jvm_method("visit", "Impl.visit", "Impl.java", EntityKind::Method);

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

        // Each Impl.visit links to both I.visit declarations (N×M).
        assert!(has_override(&entities[4], ia));
        assert!(has_override(&entities[4], ib));
        assert!(has_override(&entities[5], ia));
        assert!(has_override(&entities[5], ib));
    }

    // --- Scenario I — No supertype method match ---------------------------
    #[test]
    fn scenario_i_no_supertype_method() {
        let iface = jvm_type("I", "I", "I.java", EntityKind::Interface);
        // I declares nothing relevant.
        let mut cls = jvm_type("C", "C", "C.java", EntityKind::Class);
        cls.relationships
            .push((iface.uuid, RelationshipType::Implements));
        let helper = jvm_method("helper", "C.helper", "C.java", EntityKind::Method);

        let mut entities = vec![iface, cls, helper];
        link_method_overrides(&mut entities);

        assert_eq!(override_count(&entities[2]), 0);
    }

    // --- Scenario J — Constructors are excluded ---------------------------
    #[test]
    fn scenario_j_constructors_excluded() {
        let base = jvm_type("Base", "Base", "Base.java", EntityKind::Class);
        // Constructor: name == type name.
        let base_ctor = jvm_method("Base", "Base.Base", "Base.java", EntityKind::Method);
        let mut sub = jvm_type("Sub", "Sub", "Sub.java", EntityKind::Class);
        sub.relationships
            .push((base.uuid, RelationshipType::Extends));
        // <init>-style constructor.
        let sub_ctor = jvm_method("Sub", "Sub.Sub", "Sub.java", EntityKind::Method);
        let sub_init = jvm_method("<init>", "Sub.<init>", "Sub.java", EntityKind::Method);

        let mut entities = vec![base, base_ctor, sub, sub_ctor, sub_init];
        link_method_overrides(&mut entities);

        assert_eq!(override_count(&entities[3]), 0);
        assert_eq!(override_count(&entities[4]), 0);
    }

    // --- Scenario K — Static/private methods (documented limitation) ------
    #[test]
    fn scenario_k_static_hiding_pinned() {
        // Modifiers are not persisted; name-only cannot distinguish hiding.
        let base = jvm_type("Base", "Base", "Base.java", EntityKind::Class);
        let base_util = jvm_method("util", "Base.util", "Base.java", EntityKind::Method);
        let mut sub = jvm_type("Sub", "Sub", "Sub.java", EntityKind::Class);
        sub.relationships
            .push((base.uuid, RelationshipType::Extends));
        let sub_util = jvm_method("util", "Sub.util", "Sub.java", EntityKind::Method);

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
        let mut cls = jvm_type(
            "Session",
            "nf.Session",
            "Session.groovy",
            EntityKind::GroovyClass,
        );
        // Implements target uuid points to an entity not in this batch.
        cls.relationships
            .push((Uuid::new_v4(), RelationshipType::Implements));
        let cls_method = jvm_method(
            "getUniqueId",
            "nf.Session.getUniqueId",
            "Session.groovy",
            EntityKind::GroovyMethod,
        );

        let mut entities = vec![cls, cls_method];
        link_method_overrides(&mut entities); // must not panic
        assert_eq!(override_count(&entities[1]), 0);
    }

    // --- Scenario N — Non-JVM languages are unaffected --------------------
    #[test]
    fn scenario_n_non_jvm_unaffected() {
        // Rust trait + impl method.
        let rust_trait = jvm_type("Greeter", "crate::Greeter", "lib.rs", EntityKind::RustTrait);
        let rust_trait_m = jvm_method(
            "greet",
            "crate::Greeter::greet",
            "lib.rs",
            EntityKind::RustMethod,
        );
        // Python base/derived.
        let py_base = jvm_type("Base", "Base", "base.py", EntityKind::PythonClass);
        let py_base_m = jvm_method("run", "Base.run", "base.py", EntityKind::PythonMethod);
        let mut py_derived = jvm_type("Derived", "Derived", "derived.py", EntityKind::PythonClass);
        py_derived
            .relationships
            .push((py_base.uuid, RelationshipType::Extends));
        let py_derived_m = jvm_method("run", "Derived.run", "derived.py", EntityKind::PythonMethod);
        // TypeScript interface + class (shares generic Class/Interface/Method).
        let ts_iface = jvm_type("Repo", "Repo", "repo.ts", EntityKind::Interface);
        let ts_iface_m = jvm_method("save", "Repo.save", "repo.ts", EntityKind::Method);
        let mut ts_cls = jvm_type("UserRepo", "UserRepo", "user.ts", EntityKind::Class);
        ts_cls
            .relationships
            .push((ts_iface.uuid, RelationshipType::Implements));
        let ts_cls_m = jvm_method("save", "UserRepo.save", "user.ts", EntityKind::Method);

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
                "non-JVM entity {} gained an OVERRIDES edge",
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
    fn is_jvm_file_guard() {
        assert!(is_jvm_file("a/b/C.java"));
        assert!(is_jvm_file("x.kt"));
        assert!(is_jvm_file("x.kts"));
        assert!(is_jvm_file("x.groovy"));
        assert!(is_jvm_file("x.gvy"));
        assert!(is_jvm_file("build.gradle"));
        assert!(!is_jvm_file("x.ts"));
        assert!(!is_jvm_file("x.py"));
        assert!(!is_jvm_file("lib.rs"));
    }
}
