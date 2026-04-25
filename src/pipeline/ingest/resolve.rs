use anyhow::Result;
use std::collections::HashMap;
use tracing::info;
use uuid::Uuid;

use crate::config::Config;
use crate::db::graph::{GraphDb, UpsertExt};
use crate::models::{RelationshipType, ResolutionEntity};

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

/// Resolve reference intents to actual entity UUIDs for a batch of entities.
pub fn resolve_reference_intents_with_context(
    entities: &mut [ResolutionEntity],
    mut fqn_to_uuid: HashMap<String, Uuid>,
    mut name_to_uuids: HashMap<String, Vec<Uuid>>,
) {
    // Build uuid -> file_path mapping for same-file entity resolution
    let uuid_to_file: HashMap<Uuid, String> = entities
        .iter()
        .map(|e| (e.uuid, e.file_path.clone()))
        .collect();

    // Merge current entities into the resolution context.
    for e in entities.iter() {
        fqn_to_uuid.insert(e.fqn.clone(), e.uuid);
        name_to_uuids
            .entry(e.name.clone())
            .or_default()
            .push(e.uuid);
    }

    // Resolve reference intents for each entity.
    for entity in entities.iter_mut() {
        let reference_intents = entity.reference_intents.clone();

        // Deduplication set to prevent duplicate relationships.
        use std::collections::HashSet;
        let mut seen: HashSet<(Uuid, RelationshipType)> = HashSet::new();

        for intent in reference_intents {
            use crate::models::ReferenceIntent;
            let (resolved_uuid, rel_type) = match &intent {
                ReferenceIntent::Call {
                    method, receiver, ..
                } => {
                    let call_intent = crate::models::CallIntent {
                        method: method.clone(),
                        receiver: receiver.clone(),
                        line: 0,
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
    }
}

/// Resolve a single CallIntent to a UUID using available context.
fn resolve_single_call_intent(
    intent: &crate::models::CallIntent,
    _caller_fqn: String,
    caller_file_path: String,
    caller_enclosing_class: Option<String>,
    fqn_to_uuid: &HashMap<String, Uuid>,
    name_to_uuids: &HashMap<String, Vec<Uuid>>,
    uuid_to_file: &HashMap<Uuid, String>,
) -> Option<Uuid> {
    // Strategy 1: Local call (no receiver or receiver is "this")
    if (intent.receiver.is_none() || intent.receiver.as_deref() == Some("this"))
        && let Some(enclosing_class) = &caller_enclosing_class
    {
        let fqn = format!("{}.{}", enclosing_class, intent.method);
        if let Some(&uuid) = fqn_to_uuid.get(&fqn) {
            return Some(uuid);
        }
    }

    // Strategy 2: Static call (receiver is a class name)
    if let Some(receiver) = &intent.receiver
        && receiver.chars().next().is_some_and(|c| c.is_uppercase())
        && receiver != "this"
    {
        let fqn = format!("{}.{}", receiver, intent.method);
        if let Some(&uuid) = fqn_to_uuid.get(&fqn) {
            return Some(uuid);
        }
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
            let exact_fqn = format!("{}.{}", receiver_class, intent.method);
            if let Some(&uuid) = fqn_to_uuid.get(&exact_fqn) {
                return Some(uuid);
            }

            let mut chars = receiver_class.chars();
            let capitalized = if let Some(first) = chars.next() {
                first.to_uppercase().to_string() + chars.as_str()
            } else {
                receiver_class.to_string()
            };

            let capitalized_fqn = format!("{}.{}", capitalized, intent.method);
            if let Some(&uuid) = fqn_to_uuid.get(&capitalized_fqn) {
                return Some(uuid);
            }

            // Fuzzy match: search for ClassName.method in known FQNs.
            for (fqn, uuid) in fqn_to_uuid.iter() {
                if fqn.contains(&format!("{}.{}", receiver_class, intent.method))
                    || fqn.contains(&format!("{}.{}", capitalized, intent.method))
                {
                    return Some(*uuid);
                }
            }
        }

        // Fallback: just match on method name.
        if let Some(uuids) = name_to_uuids.get(&intent.method) {
            // Prioritize entities in the same file (for Rust private functions)
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
        // Prioritize entities in the same file (for Rust private functions)
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
        });
        caller.reference_intents.push(ReferenceIntent::Call {
            method: "B".to_string(),
            receiver: None,
            line: 2,
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
            reference_intents: Vec::new(),
            relationships: Vec::new(),
        };

        let rust_fn = ResolutionEntity {
            uuid: Uuid::new_v4(),
            name: "find_nearest_entity_by_line".to_string(),
            fqn: "knot::pipeline::parser::languages::rust::find_nearest_entity_by_line".to_string(),
            file_path: "src/pipeline/parser/languages/rust.rs".to_string(),
            enclosing_class: None,
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
            reference_intents: vec![ReferenceIntent::Call {
                method: "find_nearest_entity_by_line".to_string(),
                receiver: None,
                line: 258,
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
            reference_intents: vec![ReferenceIntent::Call {
                method: "find_nearest_entity_by_line".to_string(),
                receiver: None,
                line: 8,
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
}
