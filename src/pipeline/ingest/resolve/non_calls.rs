use std::collections::HashMap;
use std::sync::atomic::Ordering;
use tracing::debug;
use uuid::Uuid;

use super::calls::find_entity_in_same_file;
use super::context::RunMetrics;
use crate::models::EntityKind;

/// Type-like kinds that may be the target of an inheritance or type-usage
/// reference. C# constructor entities share their class's name
/// (`BaseService` class + `BaseService` constructor), so name-only
/// resolution for `Extends` / `Implements` / `TypeReference` intents must
/// restrict candidates to types or the constructor wins the ambiguity and
/// the edge is dropped as ambiguous.
fn is_type_like(kind: &EntityKind) -> bool {
    matches!(
        kind,
        EntityKind::Class
            | EntityKind::Interface
            | EntityKind::Enum
            | EntityKind::KotlinClass
            | EntityKind::KotlinInterface
            | EntityKind::KotlinObject
            | EntityKind::KotlinCompanionObject
            | EntityKind::KotlinEnum
            | EntityKind::GroovyClass
            | EntityKind::GroovyInterface
            | EntityKind::GroovyTrait
            | EntityKind::GroovyEnum
            | EntityKind::CSharpClass
            | EntityKind::CSharpInterface
            | EntityKind::CSharpStruct
            | EntityKind::CSharpRecord
            | EntityKind::CSharpEnum
            | EntityKind::CSharpDelegate
            | EntityKind::CSharpNamespace
            | EntityKind::CppClass
            | EntityKind::CppNamespace
            | EntityKind::CStruct
            | EntityKind::PythonClass
            | EntityKind::RustStruct
            | EntityKind::RustEnum
            | EntityKind::RustUnion
            | EntityKind::RustTrait
    )
}

#[expect(
    clippy::too_many_arguments,
    reason = "function is verbose but correct — extraction deferred"
)]
pub(crate) fn resolve_non_call_reference(
    name: &str,
    source_file: &str,
    enclosing_class: Option<&str>,
    fqn_to_uuid: &HashMap<String, Uuid>,
    name_to_uuids: &HashMap<String, Vec<Uuid>>,
    uuid_to_file: &HashMap<Uuid, String>,
    uuid_to_fqn: Option<&HashMap<Uuid, String>>,
    metrics: &RunMetrics,
) -> Option<Uuid> {
    resolve_non_call_reference_typed(
        name,
        source_file,
        enclosing_class,
        fqn_to_uuid,
        name_to_uuids,
        uuid_to_file,
        None,
        uuid_to_fqn,
        metrics,
    )
}

/// Type-aware variant of [`resolve_non_call_reference`]: when
/// `type_targets_only` is set, candidates are first filtered to type-like
/// kinds (see [`is_type_like`]) before the disambiguation ladder runs.
#[expect(
    clippy::too_many_arguments,
    reason = "function is verbose but correct — extraction deferred"
)]
pub(crate) fn resolve_non_call_reference_typed(
    name: &str,
    source_file: &str,
    enclosing_class: Option<&str>,
    fqn_to_uuid: &HashMap<String, Uuid>,
    name_to_uuids: &HashMap<String, Vec<Uuid>>,
    uuid_to_file: &HashMap<Uuid, String>,
    uuid_to_kind: Option<&HashMap<Uuid, EntityKind>>,
    uuid_to_fqn: Option<&HashMap<Uuid, String>>,
    metrics: &RunMetrics,
) -> Option<Uuid> {
    if name.contains('.') || name.contains("::") {
        // Exact FQN wins outright.
        if let Some(&uuid) = fqn_to_uuid.get(name) {
            metrics.references_resolved.fetch_add(1, Ordering::Relaxed);
            return Some(uuid);
        }
        // Otherwise fall back to candidates whose FQN ends with the qualified
        // name — this catches partially qualified refs (`GestureOwner.Off`).
        let candidates =
            qualified_suffix_candidates(name, name_to_uuids, uuid_to_fqn).unwrap_or_default();
        return pick_unambiguous_candidate(&candidates, source_file, uuid_to_file, metrics);
    }

    let Some(all_candidates) = name_to_uuids.get(name) else {
        metrics
            .references_unresolved
            .fetch_add(1, Ordering::Relaxed);
        return None;
    };

    let candidate_uuids: Vec<Uuid> = match uuid_to_kind {
        // Type targets (Extends/Implements/TypeReference): a constructor or
        // method sharing the name is never the right target.
        Some(kinds) => all_candidates
            .iter()
            .filter(|u| kinds.get(*u).is_some_and(is_type_like))
            .copied()
            .collect(),
        None => all_candidates.clone(),
    };

    if candidate_uuids.is_empty() {
        metrics
            .references_unresolved
            .fetch_add(1, Ordering::Relaxed);
        return None;
    }

    if let Some(same_file_uuid) =
        find_entity_in_same_file(&candidate_uuids, source_file, uuid_to_file)
    {
        metrics.references_resolved.fetch_add(1, Ordering::Relaxed);
        return Some(same_file_uuid);
    }

    if let Some(enclosing) = enclosing_class {
        let dot_fqn = format!("{}.{}", enclosing, name);
        if let Some(&uuid) = fqn_to_uuid.get(&dot_fqn) {
            metrics.references_resolved.fetch_add(1, Ordering::Relaxed);
            return Some(uuid);
        }
        let colon_fqn = format!("{}::{}", enclosing, name);
        if let Some(&uuid) = fqn_to_uuid.get(&colon_fqn) {
            metrics.references_resolved.fetch_add(1, Ordering::Relaxed);
            return Some(uuid);
        }
    }

    if candidate_uuids.len() == 1 {
        metrics.references_resolved.fetch_add(1, Ordering::Relaxed);
        return candidate_uuids.first().copied();
    }

    debug!(
        name = name,
        candidates = candidate_uuids.len(),
        source_file = source_file,
        "Ambiguous reference skipped: multiple candidates with no disambiguating context"
    );
    metrics
        .references_ambiguous_skipped
        .fetch_add(1, Ordering::Relaxed);
    None
}

/// Pick a single target from `candidates`, preferring one declared in
/// `source_file`, and record the outcome in `metrics`.
fn pick_unambiguous_candidate(
    candidates: &[Uuid],
    source_file: &str,
    uuid_to_file: &HashMap<Uuid, String>,
    metrics: &RunMetrics,
) -> Option<Uuid> {
    if candidates.is_empty() {
        metrics
            .references_unresolved
            .fetch_add(1, Ordering::Relaxed);
        return None;
    }

    if let Some(same_file_uuid) = find_entity_in_same_file(candidates, source_file, uuid_to_file) {
        metrics.references_resolved.fetch_add(1, Ordering::Relaxed);
        return Some(same_file_uuid);
    }

    if let [only] = candidates {
        metrics.references_resolved.fetch_add(1, Ordering::Relaxed);
        return Some(*only);
    }

    metrics
        .references_ambiguous_skipped
        .fetch_add(1, Ordering::Relaxed);
    None
}

/// Candidates sharing the last segment of `name` whose FQN ends with `name`.
fn qualified_suffix_candidates(
    name: &str,
    name_to_uuids: &HashMap<String, Vec<Uuid>>,
    uuid_to_fqn: Option<&HashMap<Uuid, String>>,
) -> Option<Vec<Uuid>> {
    let uuid_to_fqn_map = uuid_to_fqn?;
    let last_segment = if name.contains("::") {
        name.rsplit("::").next()?
    } else {
        name.rsplit('.').next()?
    };
    let candidates = name_to_uuids.get(last_segment)?;

    Some(
        candidates
            .iter()
            .filter(|u| {
                uuid_to_fqn_map.get(*u).is_some_and(|fqn| {
                    fqn == name
                        || fqn.ends_with(&format!(".{name}"))
                        || fqn.ends_with(&format!("::{name}"))
                })
            })
            .copied()
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::super::test_utils::*;
    use crate::models::{EntityKind, ReferenceIntent, RelationshipType};
    use crate::pipeline::ingest::resolve::resolve_reference_intents;

    #[test]
    fn test_resolve_inheritance() {
        let mut child = mock_resolution_entity("Child", "com.Child", None);
        // Inheritance targets must be type-like entities.
        let parent = mock_resolution_entity_with_kind(
            "Parent",
            "com.Parent",
            None,
            "test/file.java",
            EntityKind::Class,
        );

        child.reference_intents.push(ReferenceIntent::Extends {
            parent: "Parent".to_string(),
            line: 1,
        });

        let mut entities = vec![child, parent];
        resolve_reference_intents(&mut entities);

        assert_eq!(entities[0].relationships.len(), 1);
        assert_eq!(
            entities[0].relationships[0],
            (entities[1].uuid, RelationshipType::Extends)
        );
    }

    #[test]
    fn test_resolve_type_reference() {
        let mut entity = mock_resolution_entity("service", "service", None);
        // Type targets must be type-like entities.
        let type_entity = mock_resolution_entity_with_kind(
            "MyType",
            "com.MyType",
            None,
            "test/file.java",
            EntityKind::Class,
        );

        entity
            .reference_intents
            .push(ReferenceIntent::TypeReference {
                type_name: "MyType".to_string(),
                line: 1,
            });

        let mut entities = vec![entity, type_entity];
        resolve_reference_intents(&mut entities);

        assert_eq!(entities[0].relationships.len(), 1);
        assert_eq!(
            entities[0].relationships[0],
            (entities[1].uuid, RelationshipType::References)
        );
    }

    #[test]
    fn test_resolve_self_method_inherited_from_parent_class() {
        let animal_speak = mock_resolution_entity_with_kind(
            "Animal",
            "Animal",
            None,
            "animals.py",
            EntityKind::Class,
        );
        let animal_speak_method = mock_resolution_entity_with_kind(
            "speak",
            "Animal.speak",
            Some("Animal"),
            "animals.py",
            EntityKind::Method,
        );

        let mut dog_class =
            mock_resolution_entity_with_kind("Dog", "Dog", None, "animals.py", EntityKind::Class);
        dog_class.reference_intents = vec![ReferenceIntent::Extends {
            parent: "Animal".to_string(),
            line: 10,
        }];

        let mut dog_compute = mock_resolution_entity_with_kind(
            "compute",
            "Dog.compute",
            Some("Dog"),
            "animals.py",
            EntityKind::Method,
        );
        dog_compute.reference_intents = vec![ReferenceIntent::Call {
            method: "speak".to_string(),
            receiver: Some("self".to_string()),
            line: 15,
            arg_count: None,
        }];

        let mut entities = vec![animal_speak, animal_speak_method, dog_class, dog_compute];
        resolve_reference_intents(&mut entities);

        let dog = entities.iter().find(|e| e.name == "compute").unwrap();
        let speak_method = entities.iter().find(|e| e.fqn == "Animal.speak").unwrap();
        assert!(
            dog.relationships
                .contains(&(speak_method.uuid, RelationshipType::Calls))
        );
    }

    #[test]
    fn test_type_reference_prefers_same_file() {
        let config_in_file_a = mock_resolution_entity_with_kind(
            "Config",
            "Config",
            None,
            "src/config.rs",
            EntityKind::RustStruct,
        );
        let config_in_file_b = mock_resolution_entity_with_kind(
            "Config",
            "Config",
            None,
            "src/other/types.rs",
            EntityKind::RustStruct,
        );

        let mut caller = mock_resolution_entity_with_kind(
            "load",
            "Config::load",
            Some("Config"),
            "src/config.rs",
            EntityKind::RustMethod,
        );
        caller.reference_intents = vec![ReferenceIntent::TypeReference {
            type_name: "Config".to_string(),
            line: 10,
        }];

        let config_a_uuid = config_in_file_a.uuid;
        let config_b_uuid = config_in_file_b.uuid;

        let mut entities = vec![config_in_file_a, config_in_file_b, caller];
        resolve_reference_intents(&mut entities);

        let caller_entity = entities.last().unwrap();
        assert!(
            caller_entity
                .relationships
                .contains(&(config_a_uuid, RelationshipType::References))
        );
        assert!(
            !caller_entity
                .relationships
                .contains(&(config_b_uuid, RelationshipType::References))
        );
    }

    #[test]
    fn test_type_reference_skips_when_ambiguous_no_hints() {
        let config_file_a = mock_resolution_entity_with_kind(
            "Config",
            "Config",
            None,
            "src/a.rs",
            EntityKind::RustStruct,
        );
        let config_file_b = mock_resolution_entity_with_kind(
            "Config",
            "Config",
            None,
            "src/b.rs",
            EntityKind::RustStruct,
        );

        let mut caller = mock_resolution_entity_with_kind(
            "do_work",
            "do_work",
            None,
            "src/c.rs",
            EntityKind::RustFunction,
        );
        caller.reference_intents = vec![ReferenceIntent::TypeReference {
            type_name: "Config".to_string(),
            line: 5,
        }];

        let mut entities = vec![config_file_a, config_file_b, caller];
        resolve_reference_intents(&mut entities);

        let caller_entity = entities.last().unwrap();
        assert_eq!(caller_entity.relationships.len(), 0);
    }

    #[test]
    fn test_type_reference_enclosing_class_disambiguates() {
        let config_struct = mock_resolution_entity_with_kind(
            "Config",
            "Config",
            None,
            "src/config.rs",
            EntityKind::RustStruct,
        );
        let other_config = mock_resolution_entity_with_kind(
            "Config",
            "Config",
            None,
            "src/other.rs",
            EntityKind::RustStruct,
        );

        let mut caller = mock_resolution_entity_with_kind(
            "load",
            "Config::load",
            Some("Config"),
            "src/other.rs",
            EntityKind::RustMethod,
        );
        caller.reference_intents = vec![ReferenceIntent::TypeReference {
            type_name: "Config".to_string(),
            line: 10,
        }];

        let config_uuid = config_struct.uuid;
        let other_config_uuid = other_config.uuid;

        let mut entities = vec![config_struct, other_config, caller];
        resolve_reference_intents(&mut entities);

        let caller_entity = entities.last().unwrap();
        assert!(
            caller_entity
                .relationships
                .contains(&(other_config_uuid, RelationshipType::References))
        );
        assert!(
            !caller_entity
                .relationships
                .contains(&(config_uuid, RelationshipType::References))
        );
    }

    #[test]
    fn test_extends_uses_same_ladder() {
        let parent_in_same_file = mock_resolution_entity_with_kind(
            "MyTrait",
            "MyTrait",
            None,
            "src/lib.rs",
            EntityKind::RustTrait,
        );
        let parent_other_file = mock_resolution_entity_with_kind(
            "MyTrait",
            "MyTrait",
            None,
            "src/other.rs",
            EntityKind::RustTrait,
        );

        let mut child = mock_resolution_entity_with_kind(
            "Child",
            "Child",
            None,
            "src/lib.rs",
            EntityKind::RustStruct,
        );
        child.reference_intents = vec![ReferenceIntent::Implements {
            interface: "MyTrait".to_string(),
            line: 5,
        }];

        let same_uuid = parent_in_same_file.uuid;
        let other_uuid = parent_other_file.uuid;

        let mut entities = vec![parent_in_same_file, parent_other_file, child];
        resolve_reference_intents(&mut entities);

        let child_entity = entities.last().unwrap();
        assert!(
            child_entity
                .relationships
                .contains(&(same_uuid, RelationshipType::Implements))
        );
        assert!(
            !child_entity
                .relationships
                .contains(&(other_uuid, RelationshipType::Implements))
        );
    }

    /// End-to-end check that the Groovy parser emits an `Extends` reference
    /// intent on `Ext1` and that `resolve_reference_intents` converts it into
    /// an `Extends` edge pointing at the parent `PluginExtensionPoint` parsed
    /// in the same file.
    #[test]
    fn test_groovy_extends_resolves_to_extends_relationship() {
        use crate::models::ResolutionEntity;
        use crate::pipeline::parser::languages::groovy::extract_entities_groovy;

        const FILE: &str = "Plugin.groovy";

        let parent_source = "abstract class PluginExtensionPoint { }";
        let child_source = "class Ext1 extends PluginExtensionPoint { }";

        let parent_entities = extract_entities_groovy(parent_source, FILE, "test-repo");
        let child_entities = extract_entities_groovy(child_source, FILE, "test-repo");

        let child_parsed = child_entities
            .iter()
            .find(|e| e.name == "Ext1")
            .expect("Ext1 should be parsed");

        // Sanity check: the parser must emit an `Extends` reference on the
        // child entity before resolution runs.
        let has_extends = child_parsed.reference_intents.iter().any(|r| {
            matches!(r, ReferenceIntent::Extends { parent, .. } if parent == "PluginExtensionPoint")
        });
        assert!(
            has_extends,
            "Parser should emit Extends(PluginExtensionPoint) on Ext1; got {:?}",
            child_parsed.reference_intents
        );

        // Convert ParsedEntity -> ResolutionEntity (clones reference_intents).
        let mut resolution_entities: Vec<ResolutionEntity> = parent_entities
            .iter()
            .chain(child_entities.iter())
            .map(ResolutionEntity::from)
            .collect();

        resolve_reference_intents(&mut resolution_entities);

        let parent_uuid = resolution_entities
            .iter()
            .find(|e| e.name == "PluginExtensionPoint")
            .expect("PluginExtensionPoint not present after conversion")
            .uuid;
        let ext1 = resolution_entities
            .iter()
            .find(|e| e.name == "Ext1")
            .expect("Ext1 not present after conversion");

        assert!(
            ext1.relationships
                .contains(&(parent_uuid, RelationshipType::Extends)),
            "Ext1 should have an Extends edge to PluginExtensionPoint; got {:?}",
            ext1.relationships
        );
    }

    #[test]
    fn test_qualified_type_reference_resolves_by_fqn_suffix() {
        let target = mock_resolution_entity_with_kind(
            "Off",
            "MyApp.Gestures.GestureOwner.Off",
            None,
            "src/GestureOwner.cs",
            EntityKind::CSharpRecord,
        );
        let mut caller = mock_resolution_entity_with_kind(
            "GesturesEnabled",
            "MyApp.Gestures.GestureConfig.GesturesEnabled",
            None,
            "src/GestureConfig.cs",
            EntityKind::CSharpMethod,
        );
        caller.reference_intents = vec![ReferenceIntent::TypeReference {
            type_name: "GestureOwner.Off".to_string(),
            line: 10,
        }];
        let target_uuid = target.uuid;
        let mut entities = vec![target, caller];
        resolve_reference_intents(&mut entities);
        let caller_entity = entities
            .iter()
            .find(|e| e.name == "GesturesEnabled")
            .unwrap();
        assert!(
            caller_entity
                .relationships
                .contains(&(target_uuid, RelationshipType::References)),
            "should resolve by FQN suffix, got {:?}",
            caller_entity.relationships
        );
    }

    #[test]
    fn test_qualified_reference_resolves_non_type_member() {
        let target = mock_resolution_entity_with_kind(
            "OffValue",
            "MyApp.Gestures.GestureOwner.OffValue",
            None,
            "src/GestureOwner.cs",
            EntityKind::CSharpField,
        );
        let mut caller = mock_resolution_entity_with_kind(
            "Disable",
            "MyApp.Gestures.GestureConfig.Disable",
            None,
            "src/GestureConfig.cs",
            EntityKind::CSharpMethod,
        );
        caller.reference_intents = vec![ReferenceIntent::TypeReference {
            type_name: "GestureOwner.OffValue".to_string(),
            line: 10,
        }];
        let target_uuid = target.uuid;
        let mut entities = vec![target, caller];
        resolve_reference_intents(&mut entities);
        let caller_entity = entities.iter().find(|e| e.name == "Disable").unwrap();
        assert!(
            caller_entity
                .relationships
                .contains(&(target_uuid, RelationshipType::References)),
            "should resolve field target by dotted path, got {:?}",
            caller_entity.relationships
        );
    }

    #[test]
    fn test_qualified_reference_has_no_bare_last_segment_fallback() {
        let target = mock_resolution_entity_with_kind(
            "FromResult",
            "SomeNamespace.FromResult",
            None,
            "src/other.cs",
            EntityKind::CSharpMethod,
        );
        let mut caller = mock_resolution_entity_with_kind(
            "Disable",
            "MyApp.Gestures.GestureConfig.Disable",
            None,
            "src/GestureConfig.cs",
            EntityKind::CSharpMethod,
        );
        caller.reference_intents = vec![ReferenceIntent::TypeReference {
            type_name: "Task.FromResult".to_string(),
            line: 10,
        }];
        let mut entities = vec![target, caller];
        resolve_reference_intents(&mut entities);
        let caller_entity = entities.iter().find(|e| e.name == "Disable").unwrap();
        assert!(
            caller_entity.relationships.is_empty(),
            "should not resolve to bare last segment fallback, got {:?}",
            caller_entity.relationships
        );
    }

    #[test]
    fn test_qualified_reference_ambiguous_suffix_is_skipped() {
        let target1 = mock_resolution_entity_with_kind(
            "Off",
            "Namespace1.GestureOwner.Off",
            None,
            "src/File1.cs",
            EntityKind::CSharpRecord,
        );
        let target2 = mock_resolution_entity_with_kind(
            "Off",
            "Namespace2.GestureOwner.Off",
            None,
            "src/File2.cs",
            EntityKind::CSharpRecord,
        );
        let mut caller = mock_resolution_entity_with_kind(
            "GesturesEnabled",
            "MyApp.Gestures.GestureConfig.GesturesEnabled",
            None,
            "src/GestureConfig.cs",
            EntityKind::CSharpMethod,
        );
        caller.reference_intents = vec![ReferenceIntent::TypeReference {
            type_name: "GestureOwner.Off".to_string(),
            line: 10,
        }];
        let mut entities = vec![target1, target2, caller];
        resolve_reference_intents(&mut entities);
        let caller_entity = entities
            .iter()
            .find(|e| e.name == "GesturesEnabled")
            .unwrap();
        assert!(
            caller_entity.relationships.is_empty(),
            "should skip ambiguous suffix, got {:?}",
            caller_entity.relationships
        );
    }

    #[test]
    fn test_qualified_reference_prefers_same_file() {
        let target_foreign = mock_resolution_entity_with_kind(
            "Off",
            "Namespace1.GestureOwner.Off",
            None,
            "src/File1.cs",
            EntityKind::CSharpRecord,
        );
        let target_same = mock_resolution_entity_with_kind(
            "Off",
            "Namespace2.GestureOwner.Off",
            None,
            "src/GestureConfig.cs",
            EntityKind::CSharpRecord,
        );
        let mut caller = mock_resolution_entity_with_kind(
            "GesturesEnabled",
            "MyApp.Gestures.GestureConfig.GesturesEnabled",
            None,
            "src/GestureConfig.cs",
            EntityKind::CSharpMethod,
        );
        caller.reference_intents = vec![ReferenceIntent::TypeReference {
            type_name: "GestureOwner.Off".to_string(),
            line: 10,
        }];
        let same_uuid = target_same.uuid;
        let foreign_uuid = target_foreign.uuid;
        let mut entities = vec![target_foreign, target_same, caller];
        resolve_reference_intents(&mut entities);
        let caller_entity = entities
            .iter()
            .find(|e| e.name == "GesturesEnabled")
            .unwrap();
        assert!(
            caller_entity
                .relationships
                .contains(&(same_uuid, RelationshipType::References)),
            "should prefer same file, got {:?}",
            caller_entity.relationships
        );
        assert!(
            !caller_entity
                .relationships
                .contains(&(foreign_uuid, RelationshipType::References))
        );
    }
}
