use std::collections::HashMap;
use std::sync::atomic::Ordering;
use tracing::debug;
use uuid::Uuid;

use super::calls::find_entity_in_same_file;
use super::context::RunMetrics;

pub(crate) fn resolve_non_call_reference(
    name: &str,
    source_file: &str,
    enclosing_class: Option<&str>,
    fqn_to_uuid: &HashMap<String, Uuid>,
    name_to_uuids: &HashMap<String, Vec<Uuid>>,
    uuid_to_file: &HashMap<Uuid, String>,
    metrics: &RunMetrics,
) -> Option<Uuid> {
    let Some(candidate_uuids) = name_to_uuids.get(name) else {
        metrics
            .references_unresolved
            .fetch_add(1, Ordering::Relaxed);
        return None;
    };

    if candidate_uuids.is_empty() {
        metrics
            .references_unresolved
            .fetch_add(1, Ordering::Relaxed);
        return None;
    }

    if let Some(same_file_uuid) =
        find_entity_in_same_file(candidate_uuids, source_file, uuid_to_file)
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

#[cfg(test)]
mod tests {
    use super::super::test_utils::*;
    use crate::models::{EntityKind, ReferenceIntent, RelationshipType};
    use crate::pipeline::ingest::resolve::resolve_reference_intents;

    #[test]
    fn test_resolve_inheritance() {
        let mut child = mock_resolution_entity("Child", "com.Child", None);
        let parent = mock_resolution_entity("Parent", "com.Parent", None);

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
        let type_entity = mock_resolution_entity("MyType", "com.MyType", None);

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
}
