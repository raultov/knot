use anyhow::Result;
use rayon::prelude::*;
use std::collections::HashMap;
use tracing::info;
use uuid::Uuid;

use crate::config::Config;
use crate::db::graph::{GraphDb, UpsertExt};
use crate::models::{ReferenceIntent, RelationshipType, ResolutionEntity};

/// Resolve cross-repository relationships and persist them to Neo4j.
pub async fn resolve_and_save_relationships(
    entities: &mut [ResolutionEntity],
    graph_db: &GraphDb,
    cfg: &Config,
) -> Result<()> {
    if !entities.is_empty() {
        // Build list of repos to include in context (current repo + dependencies)
        let mut repos_to_load = vec![cfg.repo_name.clone()];
        repos_to_load.extend(cfg.dependency_repos.clone());

        info!("Loading global entity context from Neo4j for relationship resolution...");
        let (fqn_to_uuid, name_to_uuids) = graph_db.load_entity_mappings(&repos_to_load).await?;

        if !cfg.dependency_repos.is_empty() {
            info!(
                "Cross-repository resolution enabled: {} local repo(s) + {} dependency repo(s)",
                1,
                cfg.dependency_repos.len()
            );
        }

        info!(
            "Resolving reference intents with global context ({} FQNs, {} names)...",
            fqn_to_uuid.len(),
            name_to_uuids.len()
        );

        resolve_reference_intents_with_context(entities, fqn_to_uuid, name_to_uuids);

        // Create typed relationships (CALLS, EXTENDS, IMPLEMENTS, REFERENCES)
        info!("Creating typed relationships in Neo4j...");
        graph_db.upsert_relationships(entities).await?;
    }
    Ok(())
}

fn count_params_from_signature(sig: &str) -> Option<usize> {
    let open = sig.rfind('(')?;
    let close = sig.rfind(')')?;
    if close <= open {
        return None;
    }
    let params = sig[open + 1..close].trim();
    if params.is_empty() {
        Some(0)
    } else {
        Some(params.chars().filter(|&c| c == ',').count() + 1)
    }
}

pub fn resolve_reference_intents_with_context(
    entities: &mut [ResolutionEntity],
    mut fqn_to_uuid: HashMap<String, Uuid>,
    mut name_to_uuids: HashMap<String, Vec<Uuid>>,
) {
    let uuid_to_file: HashMap<Uuid, String> = entities
        .iter()
        .map(|e| (e.uuid, e.file_path.clone()))
        .collect();

    let uuid_to_arg_count: HashMap<Uuid, usize> = entities
        .iter()
        .filter_map(|e| count_params_from_signature(e.signature.as_deref()?).map(|c| (e.uuid, c)))
        .collect();

    let uuid_to_fqn: HashMap<Uuid, String> =
        entities.iter().map(|e| (e.uuid, e.fqn.clone())).collect();

    // Build class_name → parent_class_names map from EXTENDS intents for inherited self.method() resolution
    let extends_map: HashMap<String, Vec<String>> = entities
        .iter()
        .filter_map(|e| {
            let parents: Vec<String> = e
                .reference_intents
                .iter()
                .filter_map(|i| {
                    if let ReferenceIntent::Extends { parent, .. } = i {
                        Some(parent.clone())
                    } else {
                        None
                    }
                })
                .collect();
            if parents.is_empty() {
                None
            } else {
                Some((e.name.clone(), parents))
            }
        })
        .collect();

    // Merge current entities into the resolution context.
    for e in entities.iter() {
        fqn_to_uuid.insert(e.fqn.clone(), e.uuid);
        name_to_uuids
            .entry(e.name.clone())
            .or_default()
            .push(e.uuid);
    }

    // Resolve reference intents for each entity — parallelized via Rayon.
    // All context maps (fqn_to_uuid, name_to_uuids, etc.) are read-only
    // at this point, so no synchronization is needed.
    entities.par_iter_mut().for_each(|entity| {
        let reference_intents = entity.reference_intents.clone();

        // Deduplication set to prevent duplicate relationships.
        use std::collections::HashSet;
        let mut seen: HashSet<(Uuid, RelationshipType)> = HashSet::new();

        for intent in reference_intents {
            use crate::models::ReferenceIntent;
            let (resolved_uuid, rel_type) = match &intent {
                ReferenceIntent::Call {
                    method,
                    receiver,
                    arg_count,
                    ..
                } => {
                    let call_intent = crate::models::CallIntent {
                        method: method.clone(),
                        receiver: receiver.clone(),
                        line: 0,
                        arg_count: *arg_count,
                    };
                    (
                        resolve_single_call_intent(
                            &call_intent,
                            entity.fqn.clone(),
                            entity.file_path.clone(),
                            entity.enclosing_class.clone(),
                            &fqn_to_uuid,
                            &name_to_uuids,
                            &uuid_to_file,
                            &extends_map,
                            Some(&uuid_to_arg_count),
                            Some(&uuid_to_fqn),
                        ),
                        RelationshipType::Calls,
                    )
                }
                ReferenceIntent::Extends { parent, .. } => (
                    name_to_uuids
                        .get(parent)
                        .and_then(|uuids| uuids.first().copied()),
                    RelationshipType::Extends,
                ),
                ReferenceIntent::Implements { interface, .. } => (
                    name_to_uuids
                        .get(interface)
                        .and_then(|uuids| uuids.first().copied()),
                    RelationshipType::Implements,
                ),
                ReferenceIntent::TypeReference { type_name, .. } => (
                    name_to_uuids
                        .get(type_name)
                        .and_then(|uuids| uuids.first().copied()),
                    RelationshipType::References,
                ),
                ReferenceIntent::ValueReference { value_name, .. } => (
                    name_to_uuids
                        .get(value_name)
                        .and_then(|uuids| uuids.first().copied()),
                    RelationshipType::References,
                ),
                ReferenceIntent::DomElementReference { element_id, .. } => (
                    name_to_uuids
                        .get(element_id)
                        .and_then(|uuids| uuids.first().copied()),
                    RelationshipType::ReferencesDOM,
                ),
                ReferenceIntent::CssClassUsage { class_name, .. } => (
                    name_to_uuids
                        .get(class_name)
                        .and_then(|uuids| uuids.first().copied()),
                    RelationshipType::UsesCSSClass,
                ),
                ReferenceIntent::HtmlFileImport { file_path, .. } => (
                    fqn_to_uuid.get(file_path).copied(),
                    RelationshipType::ImportsScript,
                ),
                ReferenceIntent::CssFileImport { file_path, .. } => (
                    fqn_to_uuid.get(file_path).copied(),
                    RelationshipType::ImportsStylesheet,
                ),
                ReferenceIntent::RustMacroCall { macro_name, .. } => (
                    name_to_uuids
                        .get(macro_name)
                        .and_then(|uuids| uuids.first().copied()),
                    RelationshipType::MacroCalls,
                ),
            };

            if let Some(uuid) = resolved_uuid
                && seen.insert((uuid, rel_type))
            {
                entity.relationships.push((uuid, rel_type));
            }
        }
    });
}

/// When multiple entities share the same method name (overloads), use arg_count
/// to pick the correct target. If the FQN-matched UUID has a different arg_count
/// and another entity with the same name has the matching count, return that instead.
fn disambiguate_overload(
    fqn_uuid: Uuid,
    intent: &crate::models::CallIntent,
    name_to_uuids: &HashMap<String, Vec<Uuid>>,
    uuid_to_arg_count: Option<&HashMap<Uuid, usize>>,
    expected_enclosing_class: Option<&str>,
    uuid_to_fqn: Option<&HashMap<Uuid, String>>,
) -> Uuid {
    if let Some(ac) = intent.arg_count
        && let Some(ac_map) = uuid_to_arg_count
        && ac_map.get(&fqn_uuid) != Some(&ac)
    {
        // FQN matched the wrong overload — search for a better match by arg_count
        if let Some(uuids) = name_to_uuids.get(&intent.method) {
            // First, prefer overloads from the same enclosing class if possible
            if let Some(class_name) = expected_enclosing_class
                && let Some(fqn_map) = uuid_to_fqn
            {
                let dot_fqn = format!("{}.{}", class_name, intent.method);
                let colon_fqn = format!("{}::{}", class_name, intent.method);

                if let Some(&better) = uuids.iter().find(|&&u| {
                    ac_map.get(&u) == Some(&ac)
                        && fqn_map
                            .get(&u)
                            .is_some_and(|fqn| *fqn == dot_fqn || *fqn == colon_fqn)
                }) {
                    return better;
                }
            }

            // Fallback: any matching arg_count
            if let Some(&better) = uuids.iter().find(|u| ac_map.get(u) == Some(&ac)) {
                return better;
            }
        }
    }
    fqn_uuid
}

fn lookup_fqn(class: &str, method: &str, fqn_to_uuid: &HashMap<String, Uuid>) -> Option<Uuid> {
    let dot_fqn = format!("{}.{}", class, method);
    if let Some(&uuid) = fqn_to_uuid.get(&dot_fqn) {
        return Some(uuid);
    }
    let colon_fqn = format!("{}::{}", class, method);
    fqn_to_uuid.get(&colon_fqn).copied()
}

/// Resolve a single CallIntent to a UUID using available context.
#[allow(clippy::too_many_arguments)]
fn resolve_single_call_intent(
    intent: &crate::models::CallIntent,
    _caller_fqn: String,
    caller_file_path: String,
    caller_enclosing_class: Option<String>,
    fqn_to_uuid: &HashMap<String, Uuid>,
    name_to_uuids: &HashMap<String, Vec<Uuid>>,
    uuid_to_file: &HashMap<Uuid, String>,
    extends_map: &HashMap<String, Vec<String>>,
    uuid_to_arg_count: Option<&HashMap<Uuid, usize>>,
    uuid_to_fqn: Option<&HashMap<Uuid, String>>,
) -> Option<Uuid> {
    // Strategy 1: Local call (no receiver or receiver is "this"/"self")
    if (intent.receiver.is_none()
        || intent.receiver.as_deref() == Some("this")
        || intent.receiver.as_deref() == Some("self"))
        && let Some(enclosing_class) = &caller_enclosing_class
    {
        if let Some(uuid) = lookup_fqn(enclosing_class, &intent.method, fqn_to_uuid) {
            // For overloaded methods, verify or correct with arg_count
            return Some(disambiguate_overload(
                uuid,
                intent,
                name_to_uuids,
                uuid_to_arg_count,
                Some(enclosing_class),
                uuid_to_fqn,
            ));
        }

        // Check parent classes via EXTENDS (for inherited self.method() calls)
        if intent.receiver.as_deref() == Some("self")
            && let Some(parents) = extends_map.get(enclosing_class)
        {
            for parent in parents {
                if let Some(uuid) = lookup_fqn(parent, &intent.method, fqn_to_uuid) {
                    return Some(uuid);
                }
            }
        }
    }

    if let Some(receiver) = &intent.receiver
        && receiver.chars().next().is_some_and(|c| c.is_uppercase())
        && receiver != "this"
        && let Some(uuid) = lookup_fqn(receiver, &intent.method, fqn_to_uuid)
    {
        return Some(disambiguate_overload(
            uuid,
            intent,
            name_to_uuids,
            uuid_to_arg_count,
            Some(receiver),
            uuid_to_fqn,
        ));
    }

    // Strategy 3: Instance call (receiver is variable or object)
    if let Some(receiver) = &intent.receiver {
        let receiver_class = if receiver.contains('.') {
            receiver
                .split('.')
                .next_back()
                .map(|s| s.trim())
                .unwrap_or(receiver)
        } else {
            receiver
        };

        if !receiver_class.is_empty() {
            if let Some(uuid) = lookup_fqn(receiver_class, &intent.method, fqn_to_uuid) {
                return Some(disambiguate_overload(
                    uuid,
                    intent,
                    name_to_uuids,
                    uuid_to_arg_count,
                    Some(receiver_class),
                    uuid_to_fqn,
                ));
            }

            let mut chars = receiver_class.chars();
            let capitalized = if let Some(first) = chars.next() {
                first.to_uppercase().to_string() + chars.as_str()
            } else {
                receiver_class.to_string()
            };

            if let Some(uuid) = lookup_fqn(&capitalized, &intent.method, fqn_to_uuid) {
                return Some(disambiguate_overload(
                    uuid,
                    intent,
                    name_to_uuids,
                    uuid_to_arg_count,
                    Some(&capitalized),
                    uuid_to_fqn,
                ));
            }

            // Fuzzy match: search for ClassName.method in known FQNs.
            let method_dot = format!("{}.{}", receiver_class, intent.method);
            let capitalized_method_dot = format!("{}.{}", capitalized, intent.method);
            let method_colon = format!("{}::{}", receiver_class, intent.method);
            let capitalized_method_colon = format!("{}::{}", capitalized, intent.method);
            for (fqn, uuid) in fqn_to_uuid.iter() {
                if fqn.contains(&method_dot)
                    || fqn.contains(&capitalized_method_dot)
                    || fqn.contains(&method_colon)
                    || fqn.contains(&capitalized_method_colon)
                {
                    return Some(*uuid);
                }
            }
        }

        // Fallback: just match on method name.
        if let Some(uuids) = name_to_uuids.get(&intent.method) {
            if let Some(ac) = intent.arg_count
                && let Some(ac_map) = uuid_to_arg_count
                && let Some(&matching_uuid) = uuids.iter().find(|u| ac_map.get(u) == Some(&ac))
            {
                return Some(matching_uuid);
            }
            if let Some(same_file_uuid) =
                find_entity_in_same_file(uuids, &caller_file_path, uuid_to_file)
            {
                return Some(same_file_uuid);
            }
            return uuids.first().copied();
        }
    }

    // Strategy 4: Fallback for local calls without enclosing class.
    if intent.receiver.is_none()
        && let Some(uuids) = name_to_uuids.get(&intent.method)
    {
        if let Some(ac) = intent.arg_count
            && let Some(ac_map) = uuid_to_arg_count
            && let Some(&matching_uuid) = uuids.iter().find(|u| ac_map.get(u) == Some(&ac))
        {
            return Some(matching_uuid);
        }
        if let Some(same_file_uuid) =
            find_entity_in_same_file(uuids, &caller_file_path, uuid_to_file)
        {
            return Some(same_file_uuid);
        }
        return uuids.first().copied();
    }

    None
}

/// Helper function to find an entity in the same file as the caller.
/// Used for Rust to prioritize local private functions over imported ones.
fn find_entity_in_same_file(
    candidate_uuids: &[Uuid],
    caller_file_path: &str,
    uuid_to_file: &HashMap<Uuid, String>,
) -> Option<Uuid> {
    for &uuid in candidate_uuids {
        if let Some(file_path) = uuid_to_file.get(&uuid)
            && file_path == caller_file_path
        {
            return Some(uuid);
        }
    }
    None
}

/// Legacy alias for backward compatibility.
pub fn resolve_reference_intents(entities: &mut [ResolutionEntity]) {
    let fqn_to_uuid: HashMap<String, Uuid> =
        entities.iter().map(|e| (e.fqn.clone(), e.uuid)).collect();

    let mut name_to_uuids: HashMap<String, Vec<Uuid>> = HashMap::new();
    for e in entities.iter() {
        name_to_uuids
            .entry(e.name.clone())
            .or_default()
            .push(e.uuid);
    }

    resolve_reference_intents_with_context(entities, fqn_to_uuid, name_to_uuids);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{ReferenceIntent, RelationshipType};

    fn mock_resolution_entity(name: &str, fqn: &str, enclosing: Option<&str>) -> ResolutionEntity {
        ResolutionEntity {
            uuid: Uuid::new_v4(),
            name: name.to_string(),
            fqn: fqn.to_string(),
            file_path: "test/file.java".to_string(),
            enclosing_class: enclosing.map(|s| s.to_string()),
            signature: None,
            reference_intents: Vec::new(),
            relationships: Vec::new(),
        }
    }

    #[test]
    fn test_resolve_local_call() {
        let mut caller = mock_resolution_entity("methodA", "ClassA.methodA", Some("ClassA"));
        let callee = mock_resolution_entity("methodB", "ClassA.methodB", Some("ClassA"));

        caller.reference_intents.push(ReferenceIntent::Call {
            method: "methodB".to_string(),
            receiver: None,
            line: 10,
            arg_count: None,
        });

        let mut entities = vec![caller, callee];
        resolve_reference_intents(&mut entities);

        assert_eq!(entities[0].relationships.len(), 1);
        assert_eq!(
            entities[0].relationships[0],
            (entities[1].uuid, RelationshipType::Calls)
        );
    }

    #[test]
    fn test_resolve_static_call() {
        let mut caller = mock_resolution_entity("main", "App.main", None);
        let callee = mock_resolution_entity("staticMethod", "Utils.staticMethod", Some("Utils"));

        caller.reference_intents.push(ReferenceIntent::Call {
            method: "staticMethod".to_string(),
            receiver: Some("Utils".to_string()),
            line: 5,
            arg_count: None,
        });

        let mut entities = vec![caller, callee];
        resolve_reference_intents(&mut entities);

        assert_eq!(entities[0].relationships.len(), 1);
        assert_eq!(
            entities[0].relationships[0],
            (entities[1].uuid, RelationshipType::Calls)
        );
    }

    #[test]
    fn test_resolve_instance_call_fuzzy() {
        let mut caller = mock_resolution_entity("doWork", "Service.doWork", Some("Service"));
        let callee = mock_resolution_entity("execute", "Worker.execute", Some("Worker"));

        caller.reference_intents.push(ReferenceIntent::Call {
            method: "execute".to_string(),
            receiver: Some("worker".to_string()),
            line: 20,
            arg_count: None,
        });

        let mut entities = vec![caller, callee];
        resolve_reference_intents(&mut entities);

        assert_eq!(entities[0].relationships.len(), 1);
        assert_eq!(
            entities[0].relationships[0],
            (entities[1].uuid, RelationshipType::Calls)
        );
    }

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
    fn test_resolve_deduplication() {
        let mut caller = mock_resolution_entity("A", "A", None);
        let callee = mock_resolution_entity("B", "B", None);

        caller.reference_intents.push(ReferenceIntent::Call {
            method: "B".to_string(),
            receiver: None,
            line: 1,
            arg_count: None,
        });
        caller.reference_intents.push(ReferenceIntent::Call {
            method: "B".to_string(),
            receiver: None,
            line: 2,
            arg_count: None,
        });

        let mut entities = vec![caller, callee];
        resolve_reference_intents(&mut entities);

        assert_eq!(entities[0].relationships.len(), 1);
    }

    /// E2E test reproducing the exact bug: two functions with same name in different files.
    /// Verifies that calls resolve to the same-file function (Rust scope rules).
    ///
    /// Scenario:
    /// - orphans.rs has `pub(crate) fn find_nearest_entity_by_line()`
    /// - rust.rs has `fn find_nearest_entity_by_line()` (private, local)
    /// - Functions in rust.rs call `find_nearest_entity_by_line`
    /// - Expected: calls should resolve to rust.rs:445 (local function), not orphans.rs:92
    #[test]
    fn test_e2e_rust_same_file_function_resolution() {
        // Create the two target functions with identical names
        let orphans_fn = ResolutionEntity {
            uuid: Uuid::new_v4(),
            name: "find_nearest_entity_by_line".to_string(),
            fqn: "knot::pipeline::parser::orphans::find_nearest_entity_by_line".to_string(),
            file_path: "src/pipeline/parser/orphans.rs".to_string(),
            enclosing_class: None,
            signature: None,
            reference_intents: Vec::new(),
            relationships: Vec::new(),
        };

        let rust_fn = ResolutionEntity {
            uuid: Uuid::new_v4(),
            name: "find_nearest_entity_by_line".to_string(),
            fqn: "knot::pipeline::parser::languages::rust::find_nearest_entity_by_line".to_string(),
            file_path: "src/pipeline/parser/languages/rust.rs".to_string(),
            enclosing_class: None,
            signature: None,
            reference_intents: Vec::new(),
            relationships: Vec::new(),
        };

        // Create a caller function in rust.rs that calls find_nearest_entity_by_line
        let rust_caller = ResolutionEntity {
            uuid: Uuid::new_v4(),
            name: "collect_rust_type_references".to_string(),
            fqn: "knot::pipeline::parser::languages::rust::collect_rust_type_references"
                .to_string(),
            file_path: "src/pipeline/parser/languages/rust.rs".to_string(),
            enclosing_class: None,
            signature: None,
            reference_intents: vec![ReferenceIntent::Call {
                method: "find_nearest_entity_by_line".to_string(),
                receiver: None,
                line: 258,
                arg_count: None,
            }],
            relationships: Vec::new(),
        };

        // Create a caller function in orphans.rs that calls find_nearest_entity_by_line
        let orphans_caller = ResolutionEntity {
            uuid: Uuid::new_v4(),
            name: "collect_orphaned_references".to_string(),
            fqn: "knot::pipeline::parser::orphans::collect_orphaned_references".to_string(),
            file_path: "src/pipeline/parser/orphans.rs".to_string(),
            enclosing_class: None,
            signature: None,
            reference_intents: vec![ReferenceIntent::Call {
                method: "find_nearest_entity_by_line".to_string(),
                receiver: None,
                line: 8,
                arg_count: None,
            }],
            relationships: Vec::new(),
        };

        let orphans_fn_uuid = orphans_fn.uuid;
        let rust_fn_uuid = rust_fn.uuid;

        let mut entities = vec![orphans_fn, rust_fn, rust_caller, orphans_caller];
        resolve_reference_intents(&mut entities);

        // Verify rust_caller (from rust.rs) calls the LOCAL rust.rs function
        let rust_caller_rels = &entities[2].relationships;
        assert_eq!(
            rust_caller_rels.len(),
            1,
            "rust_caller should have exactly 1 CALLS relationship"
        );
        assert_eq!(
            rust_caller_rels[0],
            (rust_fn_uuid, RelationshipType::Calls),
            "rust_caller should call the LOCAL rust.rs function, not orphans.rs"
        );

        // Verify orphans_caller (from orphans.rs) calls the LOCAL orphans.rs function
        let orphans_caller_rels = &entities[3].relationships;
        assert_eq!(
            orphans_caller_rels.len(),
            1,
            "orphans_caller should have exactly 1 CALLS relationship"
        );
        assert_eq!(
            orphans_caller_rels[0],
            (orphans_fn_uuid, RelationshipType::Calls),
            "orphans_caller should call the LOCAL orphans.rs function"
        );
    }

    #[test]
    fn test_resolve_self_method_inherited_from_parent_class() {
        // Scenario: Dog extends Animal. Dog.compute calls self.speak().
        // speak() is defined in Animal (parent), not Dog.
        // The resolver should follow EXTENDS to find Animal.speak.

        let animal_speak = ResolutionEntity {
            uuid: Uuid::new_v4(),
            name: "Animal".to_string(),
            fqn: "Animal".to_string(),
            file_path: "animals.py".to_string(),
            enclosing_class: None,
            signature: None,
            reference_intents: Vec::new(),
            relationships: Vec::new(),
        };
        let animal_speak_method = ResolutionEntity {
            uuid: Uuid::new_v4(),
            name: "speak".to_string(),
            fqn: "Animal.speak".to_string(),
            file_path: "animals.py".to_string(),
            enclosing_class: Some("Animal".to_string()),
            signature: None,
            reference_intents: Vec::new(),
            relationships: Vec::new(),
        };

        let dog_class = ResolutionEntity {
            uuid: Uuid::new_v4(),
            name: "Dog".to_string(),
            fqn: "Dog".to_string(),
            file_path: "animals.py".to_string(),
            enclosing_class: None,
            signature: None,
            reference_intents: vec![ReferenceIntent::Extends {
                parent: "Animal".to_string(),
                line: 10,
            }],
            relationships: Vec::new(),
        };
        let dog_compute = ResolutionEntity {
            uuid: Uuid::new_v4(),
            name: "compute".to_string(),
            fqn: "Dog.compute".to_string(),
            file_path: "animals.py".to_string(),
            enclosing_class: Some("Dog".to_string()),
            signature: None,
            reference_intents: vec![ReferenceIntent::Call {
                method: "speak".to_string(),
                receiver: Some("self".to_string()),
                line: 15,
                arg_count: None,
            }],
            relationships: Vec::new(),
        };

        let mut entities = vec![animal_speak, animal_speak_method, dog_class, dog_compute];
        resolve_reference_intents(&mut entities);

        // Dog.compute should have a CALLS relationship to Animal.speak
        let dog = entities.iter().find(|e| e.name == "compute").unwrap();
        let speak_method = entities.iter().find(|e| e.fqn == "Animal.speak").unwrap();
        assert!(
            dog.relationships
                .contains(&(speak_method.uuid, RelationshipType::Calls)),
            "Dog.compute should call Animal.speak via self.speak()"
        );
    }

    #[test]
    fn test_resolve_self_method_same_class_name_collision() {
        // Bug: module-level function has same name as class method.
        // self.method() MUST resolve to local class method, NOT module function.
        // This replicates ComfyUI bug: self.load_lora() → class method, not lora.py:load_lora.

        // Module-level function (simulates lora.py:load_lora)
        let module_func = ResolutionEntity {
            uuid: Uuid::new_v4(),
            name: "do_thing".to_string(),
            fqn: "do_thing".to_string(),
            file_path: "lora.py".to_string(),
            enclosing_class: None,
            signature: None,
            reference_intents: Vec::new(),
            relationships: Vec::new(),
        };

        // Class with method of same name
        let my_class = ResolutionEntity {
            uuid: Uuid::new_v4(),
            name: "MyLoader".to_string(),
            fqn: "MyLoader".to_string(),
            file_path: "nodes.py".to_string(),
            enclosing_class: None,
            signature: None,
            reference_intents: Vec::new(),
            relationships: Vec::new(),
        };
        // Class method with same name as module function (different file, different FQN)
        let class_method = ResolutionEntity {
            uuid: Uuid::new_v4(),
            name: "do_thing".to_string(),
            fqn: "MyLoader.do_thing".to_string(),
            file_path: "nodes.py".to_string(),
            enclosing_class: Some("MyLoader".to_string()),
            signature: None,
            reference_intents: Vec::new(),
            relationships: Vec::new(),
        };
        // Another method in same class calling self.do_thing()
        let caller_method = ResolutionEntity {
            uuid: Uuid::new_v4(),
            name: "caller".to_string(),
            fqn: "MyLoader.caller".to_string(),
            file_path: "nodes.py".to_string(),
            enclosing_class: Some("MyLoader".to_string()),
            signature: None,
            reference_intents: vec![ReferenceIntent::Call {
                method: "do_thing".to_string(),
                receiver: Some("self".to_string()),
                line: 20,
                arg_count: None,
            }],
            relationships: Vec::new(),
        };

        let module_uuid = module_func.uuid;
        let class_method_uuid = class_method.uuid;

        let mut entities = vec![module_func, my_class, class_method, caller_method];
        resolve_reference_intents(&mut entities);

        // caller_method should call class_method (local), NOT module_func
        let caller = entities.iter().find(|e| e.name == "caller").unwrap();
        assert!(
            caller
                .relationships
                .contains(&(class_method_uuid, RelationshipType::Calls)),
            "self.do_thing() should resolve to MyLoader.do_thing (class method), not module function"
        );
        assert!(
            !caller
                .relationships
                .contains(&(module_uuid, RelationshipType::Calls)),
            "self.do_thing() should NOT resolve to module-level do_thing"
        );
    }

    /// Verify that parallel resolution produces the same results as
    /// sequential resolution would, even with a larger batch of entities.
    #[test]
    fn test_parallel_resolution_deterministic() {
        // Generate a batch of entities with mixed reference intents.
        let mut entities: Vec<ResolutionEntity> = (0..50)
            .map(|i| {
                let mut e = mock_resolution_entity(
                    &format!("Entity{i}"),
                    &format!("com.example.Entity{i}"),
                    Some(&format!("Class{i}")),
                );
                // Add a type reference to every other entity
                if i % 2 == 0 && i + 1 < 50 {
                    e.reference_intents.push(ReferenceIntent::TypeReference {
                        type_name: format!("Entity{}", i + 1),
                        line: 1,
                    });
                }
                e
            })
            .collect();

        resolve_reference_intents(&mut entities);

        // Every even-indexed entity should have resolved its reference to the odd-indexed one
        for i in (0..50).step_by(2) {
            if i + 1 < 50 {
                let caller = &entities[i];
                let callee = &entities[i + 1];
                assert!(
                    caller
                        .relationships
                        .contains(&(callee.uuid, RelationshipType::References)),
                    "Entity{i} should reference Entity{}",
                    i + 1
                );
            }
        }
    }

    /// Verify that parallel resolution correctly handles deduplication
    /// even when an entity has multiple references to the same target.
    #[test]
    fn test_parallel_resolution_deduplication() {
        let mut caller = mock_resolution_entity("A", "A", None);
        let callee = mock_resolution_entity("B", "B", None);

        // Add many duplicate reference intents to the same target
        for i in 0..100 {
            caller.reference_intents.push(ReferenceIntent::Call {
                method: "B".to_string(),
                receiver: None,
                line: i,
                arg_count: None,
            });
        }

        let mut entities = vec![caller, callee];
        resolve_reference_intents(&mut entities);

        // Only 1 CALLS relationship should exist (deduplicated)
        assert_eq!(entities[0].relationships.len(), 1);
    }

    /// Verify that parallel resolution works for multiple entities
    /// that all refer to the same single target (many-to-one).
    #[test]
    fn test_parallel_resolution_many_to_one() {
        let callee = mock_resolution_entity("Target", "com.Target", None);
        let callee_uuid = callee.uuid;

        let mut callers: Vec<ResolutionEntity> = (0..20)
            .map(|i| {
                let mut e =
                    mock_resolution_entity(&format!("Caller{i}"), &format!("com.Caller{i}"), None);
                e.reference_intents.push(ReferenceIntent::TypeReference {
                    type_name: "Target".to_string(),
                    line: 1,
                });
                e
            })
            .collect();

        let mut entities = vec![callee];
        entities.append(&mut callers);
        resolve_reference_intents(&mut entities);

        // All callers should reference the same callee
        for e in entities.iter().skip(1) {
            assert_eq!(e.relationships.len(), 1);
            assert_eq!(
                e.relationships[0],
                (callee_uuid, RelationshipType::References)
            );
        }
    }
}
