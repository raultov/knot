pub mod aliases;
pub mod calls;
pub mod context;
pub mod cross_repo;
pub mod non_calls;

#[cfg(test)]
mod test_utils;

use anyhow::Result;
use rayon::prelude::*;
use std::collections::{HashMap, HashSet};
use tracing::info;
use uuid::Uuid;

use crate::config::Config;
use crate::db::graph::{GraphDb, RepoQueryExt, UpsertExt};
use crate::models::{EntityKind, ReferenceIntent, RelationshipType, ResolutionEntity};

pub use context::{ResolutionContext, RunMetrics};
pub use cross_repo::link_cross_repo_dependencies;

pub async fn resolve_and_save_relationships(
    entities: &mut [ResolutionEntity],
    graph_db: &GraphDb,
    cfg: &Config,
) -> Result<RunMetrics> {
    if !entities.is_empty() {
        let auto_deps = graph_db
            .find_repo_dependencies(&cfg.repo_name, 3)
            .await
            .unwrap_or_default();

        let mut repos_to_load = vec![cfg.repo_name.clone()];
        repos_to_load.extend(cfg.dependency_repos.clone());
        repos_to_load.extend(auto_deps.clone());
        repos_to_load.sort();
        repos_to_load.dedup();

        info!("Loading global entity context from Neo4j for relationship resolution...");
        let (fqn_to_uuid, name_to_uuids) = graph_db.load_entity_mappings(&repos_to_load).await?;

        if !cfg.dependency_repos.is_empty() || !auto_deps.is_empty() {
            info!(
                "Cross-repository resolution enabled: {} local repo(s) + {} manual dep(s) + {} auto dep(s)",
                1,
                cfg.dependency_repos.len(),
                auto_deps.len()
            );
        }

        info!(
            "Resolving reference intents with global context ({} FQNs, {} names)...",
            fqn_to_uuid.len(),
            name_to_uuids.len()
        );

        let metrics = resolve_reference_intents_with_context(entities, fqn_to_uuid, name_to_uuids);

        info!("Creating typed relationships in Neo4j...");
        graph_db.upsert_relationships(entities).await?;

        Ok(metrics)
    } else {
        Ok(RunMetrics::new(0))
    }
}

pub fn print_run_summary(metrics: &RunMetrics) {
    use std::sync::atomic::Ordering;
    let entities = metrics.entities_indexed.load(Ordering::Relaxed);
    let resolved = metrics.references_resolved.load(Ordering::Relaxed);
    let ambiguous = metrics.references_ambiguous_skipped.load(Ordering::Relaxed);
    let unresolved = metrics.references_unresolved.load(Ordering::Relaxed);
    info!(
        "Indexing complete: {} entities, {} references resolved, {} ambiguous skipped, {} unresolved",
        entities, resolved, ambiguous, unresolved
    );
    if ambiguous > 0 {
        info!(
            "Ambiguous references skipped: {} (set RUST_LOG=debug for details)",
            ambiguous
        );
    }
}

pub fn resolve_reference_intents_with_context(
    entities: &mut [ResolutionEntity],
    mut fqn_to_uuid: HashMap<String, Uuid>,
    mut name_to_uuids: HashMap<String, Vec<Uuid>>,
) -> RunMetrics {
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

    let uuid_to_kind: HashMap<Uuid, EntityKind> =
        entities.iter().map(|e| (e.uuid, e.kind.clone())).collect();

    let uuid_to_name: HashMap<Uuid, String> =
        entities.iter().map(|e| (e.uuid, e.name.clone())).collect();

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

    for e in entities.iter() {
        fqn_to_uuid.insert(e.fqn.clone(), e.uuid);
        name_to_uuids
            .entry(e.name.clone())
            .or_default()
            .push(e.uuid);
    }

    for uuids in name_to_uuids.values_mut() {
        uuids.sort();
        uuids.dedup();
    }

    let alias_map = aliases::build_alias_map(entities, &uuid_to_file);

    let metrics = RunMetrics::new(entities.len() as u64);

    let ctx = ResolutionContext {
        fqn_to_uuid: &fqn_to_uuid,
        name_to_uuids: &name_to_uuids,
        uuid_to_file: &uuid_to_file,
        extends_map: &extends_map,
        uuid_to_arg_count: Some(&uuid_to_arg_count),
        uuid_to_fqn: Some(&uuid_to_fqn),
        uuid_to_kind: Some(&uuid_to_kind),
        uuid_to_name: Some(&uuid_to_name),
    };

    entities.par_iter_mut().for_each(|entity| {
        let reference_intents = entity.reference_intents.clone();

        let mut seen: HashSet<(Uuid, RelationshipType)> = HashSet::new();

        for intent in reference_intents {
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
                    let resolved = calls::resolve_single_call_intent(
                        &call_intent,
                        &entity.file_path,
                        entity.enclosing_class.as_deref(),
                        &ctx,
                    )
                    .map(|uuid| calls::redirect_class_call_to_constructor(uuid, &ctx));
                    (resolved, RelationshipType::Calls)
                }
                ReferenceIntent::Extends { parent, .. } => (
                    non_calls::resolve_non_call_reference(
                        parent,
                        &entity.file_path,
                        entity.enclosing_class.as_deref(),
                        ctx.fqn_to_uuid,
                        ctx.name_to_uuids,
                        ctx.uuid_to_file,
                        &metrics,
                    ),
                    RelationshipType::Extends,
                ),
                ReferenceIntent::Implements { interface, .. } => (
                    non_calls::resolve_non_call_reference(
                        interface,
                        &entity.file_path,
                        entity.enclosing_class.as_deref(),
                        ctx.fqn_to_uuid,
                        ctx.name_to_uuids,
                        ctx.uuid_to_file,
                        &metrics,
                    ),
                    RelationshipType::Implements,
                ),
                ReferenceIntent::TypeReference { type_name, .. } => (
                    non_calls::resolve_non_call_reference(
                        type_name,
                        &entity.file_path,
                        entity.enclosing_class.as_deref(),
                        ctx.fqn_to_uuid,
                        ctx.name_to_uuids,
                        ctx.uuid_to_file,
                        &metrics,
                    ),
                    RelationshipType::References,
                ),
                ReferenceIntent::ValueReference { value_name, .. } => (
                    non_calls::resolve_non_call_reference(
                        value_name,
                        &entity.file_path,
                        entity.enclosing_class.as_deref(),
                        ctx.fqn_to_uuid,
                        ctx.name_to_uuids,
                        ctx.uuid_to_file,
                        &metrics,
                    ),
                    RelationshipType::References,
                ),
                ReferenceIntent::DomElementReference { element_id, .. } => (
                    non_calls::resolve_non_call_reference(
                        element_id,
                        &entity.file_path,
                        entity.enclosing_class.as_deref(),
                        ctx.fqn_to_uuid,
                        ctx.name_to_uuids,
                        ctx.uuid_to_file,
                        &metrics,
                    ),
                    RelationshipType::ReferencesDOM,
                ),
                ReferenceIntent::CssClassUsage { class_name, .. } => (
                    non_calls::resolve_non_call_reference(
                        class_name,
                        &entity.file_path,
                        entity.enclosing_class.as_deref(),
                        ctx.fqn_to_uuid,
                        ctx.name_to_uuids,
                        ctx.uuid_to_file,
                        &metrics,
                    ),
                    RelationshipType::UsesCSSClass,
                ),
                ReferenceIntent::HtmlFileImport { file_path, .. } => (
                    ctx.fqn_to_uuid.get(file_path).copied(),
                    RelationshipType::ImportsScript,
                ),
                ReferenceIntent::CssFileImport { file_path, .. } => (
                    ctx.fqn_to_uuid.get(file_path).copied(),
                    RelationshipType::ImportsStylesheet,
                ),
                ReferenceIntent::RustMacroCall { macro_name, .. } => (
                    non_calls::resolve_non_call_reference(
                        macro_name,
                        &entity.file_path,
                        entity.enclosing_class.as_deref(),
                        ctx.fqn_to_uuid,
                        ctx.name_to_uuids,
                        ctx.uuid_to_file,
                        &metrics,
                    ),
                    RelationshipType::MacroCalls,
                ),
            };

            if let Some(mut uuid) = resolved_uuid {
                if let Some(&target) = alias_map.get(&uuid) {
                    uuid = target;
                }
                if seen.insert((uuid, rel_type)) {
                    entity.relationships.push((uuid, rel_type));
                }
            }
        }
    });

    metrics
}

pub fn resolve_reference_intents(entities: &mut [ResolutionEntity]) -> RunMetrics {
    let fqn_to_uuid: HashMap<String, Uuid> =
        entities.iter().map(|e| (e.fqn.clone(), e.uuid)).collect();

    let mut name_to_uuids: HashMap<String, Vec<Uuid>> = HashMap::new();
    for e in entities.iter() {
        name_to_uuids
            .entry(e.name.clone())
            .or_default()
            .push(e.uuid);
    }

    resolve_reference_intents_with_context(entities, fqn_to_uuid, name_to_uuids)
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

#[cfg(test)]
mod tests {
    use super::test_utils::*;
    use super::*;
    use crate::models::{ReferenceIntent, RelationshipType, ResolutionEntity};

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

    #[test]
    fn test_resolve_context_deduplication() {
        let mut caller = mock_resolution_entity("caller", "caller", None);
        caller.reference_intents.push(ReferenceIntent::Call {
            method: "unique_func".to_string(),
            receiver: None,
            line: 1,
            arg_count: None,
        });

        let entity =
            mock_resolution_entity_at("unique_func", "unique_func", None, "test/unique.java");
        let uuid = entity.uuid;

        let fqn_to_uuid = HashMap::from([("unique_func".to_string(), uuid)]);
        let name_to_uuids = HashMap::from([("unique_func".to_string(), vec![uuid])]);

        let mut batch = vec![caller, entity];
        resolve_reference_intents_with_context(&mut batch, fqn_to_uuid, name_to_uuids);

        assert_eq!(batch[0].relationships.len(), 1);
        assert_eq!(batch[0].relationships[0].0, uuid);
    }

    #[test]
    fn test_parallel_resolution_deterministic() {
        let mut entities: Vec<ResolutionEntity> = (0..50)
            .map(|i| {
                let mut e = mock_resolution_entity(
                    &format!("Entity{i}"),
                    &format!("com.example.Entity{i}"),
                    Some(&format!("Class{i}")),
                );
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

        for i in (0..50).step_by(2) {
            if i + 1 < 50 {
                let caller = &entities[i];
                let callee = &entities[i + 1];
                assert!(
                    caller
                        .relationships
                        .contains(&(callee.uuid, RelationshipType::References))
                );
            }
        }
    }

    #[test]
    fn test_parallel_resolution_deduplication() {
        let mut caller = mock_resolution_entity("A", "A", None);
        let callee = mock_resolution_entity("B", "B", None);

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

        assert_eq!(entities[0].relationships.len(), 1);
    }

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

        for e in entities.iter().skip(1) {
            assert_eq!(e.relationships.len(), 1);
            assert_eq!(
                e.relationships[0],
                (callee_uuid, RelationshipType::References)
            );
        }
    }
}
