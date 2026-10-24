pub mod aliases;
pub mod calls;
pub mod context;
pub mod cross_repo;
pub mod non_calls;
pub mod overrides;

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

fn resolve_vcl_include(
    path: &str,
    file_path: &str,
    repo_name: &str,
    fqn_to_uuid: &HashMap<String, Uuid>,
) -> Option<Uuid> {
    let stripped_path = path.strip_prefix('/').unwrap_or(path);

    let root_fqn = format!("vcl:{}:{}", repo_name, stripped_path);
    if let Some(&uuid) = fqn_to_uuid.get(&root_fqn) {
        return Some(uuid);
    }

    let parent_dir = std::path::Path::new(file_path)
        .parent()
        .and_then(|p| p.to_str())
        .unwrap_or("");

    let relative_path = if parent_dir.is_empty() {
        path.to_string()
    } else {
        format!("{}/{}", parent_dir, stripped_path)
    };

    let relative_fqn = format!("vcl:{}:{}", repo_name, relative_path);
    if let Some(&uuid) = fqn_to_uuid.get(&relative_fqn) {
        return Some(uuid);
    }

    let file_name = std::path::Path::new(path)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(path);
    let vcl_prefix = format!("vcl:{}", repo_name);

    // The basename fallback matches every VCL file whose path ends with the
    // include's basename, so `alpha/shared.vcl` and `beta/shared.vcl` both
    // match `include "shared.vcl"`. Collect every hit and rank them instead of
    // returning the first one out of the map: `fqn_to_uuid` iteration order is
    // randomized per process. An include target is a different file from the
    // includer by construction, so same-file locality cannot disambiguate here
    // and the FQN / UUID ordering is the tie-break.
    let candidates: Vec<(&str, Uuid)> = fqn_to_uuid
        .iter()
        .filter(|(fqn, _)| fqn.starts_with(&vcl_prefix) && fqn.ends_with(file_name))
        .map(|(fqn, uuid)| (fqn.as_str(), *uuid))
        .collect();
    calls::pick_deterministic_candidate(&candidates, None)
}

/// Owned lookup maps for reference resolution: the incoming global context
/// seeded with the current batch, plus the per-batch derived maps.
struct LookupMaps {
    fqn_to_uuid: HashMap<String, Uuid>,
    name_to_uuids: HashMap<String, Vec<Uuid>>,
    uuid_to_file: HashMap<Uuid, String>,
    uuid_to_arg_count: HashMap<Uuid, usize>,
    uuid_to_fqn: HashMap<Uuid, String>,
    uuid_to_kind: HashMap<Uuid, EntityKind>,
    uuid_to_name: HashMap<Uuid, String>,
    extends_map: HashMap<String, Vec<String>>,
    alias_map: HashMap<Uuid, Uuid>,
    class_member_index: HashMap<String, Vec<(String, Uuid)>>,
    fqn_overloads: HashMap<String, Vec<Uuid>>,
}

impl LookupMaps {
    fn build(
        entities: &[ResolutionEntity],
        mut fqn_to_uuid: HashMap<String, Uuid>,
        mut name_to_uuids: HashMap<String, Vec<Uuid>>,
    ) -> Self {
        let uuid_to_file: HashMap<Uuid, String> = entities
            .iter()
            .map(|e| (e.uuid, e.file_path.clone()))
            .collect();

        let uuid_to_arg_count: HashMap<Uuid, usize> = entities
            .iter()
            .filter_map(|e| {
                // Variable-arity methods (varargs, defaults) get no entry: an
                // unknown arity never rules a candidate out.
                let signature = e.signature.as_deref()?;
                let count = match overrides::declared_arity(signature, &e.name) {
                    overrides::DeclaredArity::Fixed(count) => Some(count),
                    overrides::DeclaredArity::Variable => None,
                    overrides::DeclaredArity::Unparsed => count_params_from_signature(signature),
                };
                count.map(|c| (e.uuid, c))
            })
            .collect();

        // Overloads share one FQN, so `fqn_to_uuid` keeps a single one of
        // them; this map keeps them all (issue #35). Sorted for a
        // deterministic pick among same-arity overloads.
        let mut fqn_overloads: HashMap<String, Vec<Uuid>> = HashMap::new();
        for e in entities {
            fqn_overloads.entry(e.fqn.clone()).or_default().push(e.uuid);
        }
        fqn_overloads.retain(|_, uuids| uuids.len() > 1);
        for uuids in fqn_overloads.values_mut() {
            uuids.sort();
        }

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
        let class_member_index = context::build_class_member_index(&fqn_to_uuid);

        Self {
            fqn_to_uuid,
            name_to_uuids,
            uuid_to_file,
            uuid_to_arg_count,
            uuid_to_fqn,
            uuid_to_kind,
            uuid_to_name,
            extends_map,
            alias_map,
            class_member_index,
            fqn_overloads,
        }
    }

    fn context(&self) -> ResolutionContext<'_> {
        ResolutionContext {
            fqn_to_uuid: &self.fqn_to_uuid,
            name_to_uuids: &self.name_to_uuids,
            uuid_to_file: &self.uuid_to_file,
            extends_map: &self.extends_map,
            uuid_to_arg_count: Some(&self.uuid_to_arg_count),
            uuid_to_fqn: Some(&self.uuid_to_fqn),
            uuid_to_kind: Some(&self.uuid_to_kind),
            uuid_to_name: Some(&self.uuid_to_name),
            class_member_index: &self.class_member_index,
            fqn_overloads: &self.fqn_overloads,
        }
    }
}

pub fn resolve_reference_intents_with_context(
    entities: &mut [ResolutionEntity],
    fqn_to_uuid: HashMap<String, Uuid>,
    name_to_uuids: HashMap<String, Vec<Uuid>>,
) -> RunMetrics {
    let maps = LookupMaps::build(entities, fqn_to_uuid, name_to_uuids);
    let ctx = maps.context();
    let metrics = RunMetrics::new(entities.len() as u64);

    entities.par_iter_mut().for_each(|entity| {
        let reference_intents = entity.reference_intents.clone();
        let mut seen: HashSet<(Uuid, RelationshipType)> = HashSet::new();

        for intent in &reference_intents {
            let (uuid, rel_type) = resolve_intent(intent, entity, &ctx, &metrics);
            let Some(uuid) = uuid.map(|u| maps.alias_map.get(&u).copied().unwrap_or(u)) else {
                continue;
            };
            push_relationship(uuid, rel_type, entity, &ctx, &mut seen);
        }
    });

    // JVM method-level OVERRIDES linking. Runs after type-level
    // Extends/Implements edges are resolved above and before upsert. Additive
    // and batch-local (see `overrides::link_method_overrides`).
    overrides::link_method_overrides(entities);

    metrics
}

/// Pushes a resolved relationship onto the entity, applying the
/// nested-declaration self-reference filter and deduplication. The alias
/// redirection is applied by the caller before this point.
fn push_relationship(
    uuid: Uuid,
    rel_type: RelationshipType,
    entity: &mut ResolutionEntity,
    ctx: &ResolutionContext<'_>,
    seen: &mut HashSet<(Uuid, RelationshipType)>,
) {
    if rel_type == RelationshipType::References
        && let Some(uuid_to_fqn) = ctx.uuid_to_fqn
        && let Some(target_fqn) = uuid_to_fqn.get(&uuid)
        && target_fqn.starts_with(&format!("{}.", entity.fqn))
    {
        // A parent should not emit a References edge to one of
        // its own nested declarations (the parent → child
        // direction is implicit ownership, not a usage
        // reference). Examples: a record's static field typed
        // by one of its nested records.
        return;
    }
    if seen.insert((uuid, rel_type)) {
        entity.relationships.push((uuid, rel_type));
    }
}

/// The caller-side context call intents of `entity` are resolved from.
fn caller_scope(entity: &ResolutionEntity) -> calls::CallerScope<'_> {
    calls::CallerScope {
        file_path: &entity.file_path,
        enclosing_class: entity.enclosing_class.as_deref(),
        enclosing_class_fqn: entity.enclosing_class_fqn.as_deref(),
        is_rust: matches!(
            entity.kind,
            EntityKind::RustFunction
                | EntityKind::RustMethod
                | EntityKind::RustConstant
                | EntityKind::RustStatic
                | EntityKind::RustMacroDef
        ),
    }
}

/// Maps a single reference intent to its resolved target UUID (if any) and
/// the relationship type the intent produces.
fn resolve_intent(
    intent: &ReferenceIntent,
    entity: &ResolutionEntity,
    ctx: &ResolutionContext<'_>,
    metrics: &RunMetrics,
) -> (Option<Uuid>, RelationshipType) {
    match intent {
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
            let resolved =
                calls::resolve_single_call_intent(&call_intent, &caller_scope(entity), ctx).map(
                    |uuid| calls::redirect_class_call_to_constructor(uuid, &entity.file_path, ctx),
                );
            (resolved, RelationshipType::Calls)
        }
        ReferenceIntent::Extends { parent, .. } => (
            resolve_typed(parent, entity, ctx, metrics),
            RelationshipType::Extends,
        ),
        ReferenceIntent::Implements { interface, .. } => (
            resolve_typed(interface, entity, ctx, metrics),
            RelationshipType::Implements,
        ),
        ReferenceIntent::TypeReference { type_name, .. } => (
            resolve_typed(type_name, entity, ctx, metrics),
            RelationshipType::References,
        ),
        ReferenceIntent::ValueReference { value_name, .. } => (
            resolve_plain(value_name, entity, ctx, metrics),
            RelationshipType::References,
        ),
        ReferenceIntent::DomElementReference { element_id, .. } => (
            resolve_web_reference(element_id, EntityKind::HtmlId, entity, ctx, metrics),
            RelationshipType::ReferencesDOM,
        ),
        ReferenceIntent::CssClassUsage { class_name, .. } => (
            resolve_web_reference(class_name, EntityKind::CssClass, entity, ctx, metrics),
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
            resolve_plain(macro_name, entity, ctx, metrics),
            RelationshipType::MacroCalls,
        ),
        ReferenceIntent::VclSubCall { sub_name, .. } => {
            let call_intent = crate::models::CallIntent {
                method: sub_name.clone(),
                receiver: None,
                line: 0,
                arg_count: None,
            };
            let resolved =
                calls::resolve_single_call_intent(&call_intent, &caller_scope(entity), ctx);
            (resolved, RelationshipType::Calls)
        }
        ReferenceIntent::VclBackendRef { backend_name, .. } => (
            resolve_plain(backend_name, entity, ctx, metrics),
            RelationshipType::UsesBackend,
        ),
        ReferenceIntent::VclProbeRef { probe_name, .. } => (
            resolve_plain(probe_name, entity, ctx, metrics),
            RelationshipType::UsesProbe,
        ),
        ReferenceIntent::VclAclRef { acl_name, .. } => (
            resolve_plain(acl_name, entity, ctx, metrics),
            RelationshipType::UsesAcl,
        ),
        ReferenceIntent::VclInclude { path, .. } => {
            let repo_name = entity.fqn.split(':').nth(1).unwrap_or("");
            let resolved_uuid =
                resolve_vcl_include(path, &entity.file_path, repo_name, ctx.fqn_to_uuid);
            (resolved_uuid, RelationshipType::Includes)
        }
        ReferenceIntent::VclVmodImport { module, .. } => (
            resolve_plain(module, entity, ctx, metrics),
            RelationshipType::ImportsVmod,
        ),
        ReferenceIntent::VclUnusedRef { name, .. } => (
            resolve_plain(name, entity, ctx, metrics),
            RelationshipType::DeclaredUnused,
        ),
    }
}

/// Name-only non-call reference resolution (no kind/FQN disambiguation).
fn resolve_plain(
    name: &str,
    entity: &ResolutionEntity,
    ctx: &ResolutionContext<'_>,
    metrics: &RunMetrics,
) -> Option<Uuid> {
    non_calls::resolve_non_call_reference(
        name,
        &entity.file_path,
        entity.enclosing_class.as_deref(),
        ctx.fqn_to_uuid,
        ctx.name_to_uuids,
        ctx.uuid_to_file,
        ctx.uuid_to_fqn,
        metrics,
    )
}

/// Web-reference resolution (REFERENCES_DOM / USES_CSS_CLASS): routed to the
/// kind-exact resolver — the intent already names the exact kind it targets,
/// so the plain ambiguity-prone ladder must not run (see
/// [`non_calls::resolve_web_reference`]).
fn resolve_web_reference(
    name: &str,
    preferred: EntityKind,
    entity: &ResolutionEntity,
    ctx: &ResolutionContext<'_>,
    metrics: &RunMetrics,
) -> Option<Uuid> {
    non_calls::resolve_web_reference(
        name,
        preferred,
        &entity.file_path,
        ctx.name_to_uuids,
        ctx.uuid_to_file,
        ctx.uuid_to_kind,
        metrics,
    )
}

/// Non-call reference resolution with kind/FQN disambiguation.
fn resolve_typed(
    name: &str,
    entity: &ResolutionEntity,
    ctx: &ResolutionContext<'_>,
    metrics: &RunMetrics,
) -> Option<Uuid> {
    non_calls::resolve_non_call_reference_typed(
        name,
        &entity.file_path,
        entity.enclosing_class.as_deref(),
        ctx.fqn_to_uuid,
        ctx.name_to_uuids,
        ctx.uuid_to_file,
        ctx.uuid_to_kind,
        ctx.uuid_to_fqn,
        metrics,
    )
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
                let mut e = mock_resolution_entity_with_kind(
                    &format!("Entity{i}"),
                    &format!("com.example.Entity{i}"),
                    Some(&format!("Class{i}")),
                    "test/file.java",
                    EntityKind::Class,
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

    /// Collect every `(caller FQN, target UUID, relationship type)` triple as
    /// a set so runs can be compared without depending on push order.
    fn relationship_set(
        entities: &[ResolutionEntity],
    ) -> HashSet<(String, Uuid, RelationshipType)> {
        entities
            .iter()
            .flat_map(|e| {
                e.relationships
                    .iter()
                    .map(move |(uuid, rel)| (e.fqn.clone(), *uuid, *rel))
            })
            .collect()
    }

    /// Run [`resolve_reference_intents`] `runs` times over clones of
    /// `entities` and assert the resulting relationship set is identical on
    /// every run.
    ///
    /// Each run builds fresh `HashMap`s, so `RandomState` reshuffles their
    /// iteration order. Any "first match wins" scan over a map therefore
    /// returns a different target on each run — which is exactly the bug this
    /// harness exists to catch. Cloning the fixture keeps UUIDs stable so the
    /// sets are directly comparable.
    fn assert_resolution_deterministic(
        entities: &[ResolutionEntity],
        runs: usize,
    ) -> HashSet<(String, Uuid, RelationshipType)> {
        let mut reference: Option<HashSet<(String, Uuid, RelationshipType)>> = None;
        for run in 0..runs {
            let mut batch = entities.to_vec();
            resolve_reference_intents(&mut batch);
            let got = relationship_set(&batch);
            match &reference {
                None => reference = Some(got),
                Some(expected) => assert_eq!(
                    &got, expected,
                    "run {run} produced a different relationship set: HashMap \
                     iteration order leaked into the resolution result"
                ),
            }
        }
        reference.expect("at least one run is required")
    }

    /// Regression: `resolve_receiver_call` matched candidate FQNs by
    /// substring (`fqn.contains("Widget.render")`) and returned the first hit
    /// out of `fqn_to_uuid`. Three `render` methods under distinct module
    /// paths all contain `Widget.render`, so the edge landed on an arbitrary
    /// one of them depending on `RandomState`.
    #[test]
    fn test_resolution_deterministic_under_fqn_substring_collisions() {
        let mut entities: Vec<ResolutionEntity> = [
            ("pkg.alpha.Widget.render", "src/alpha.rs"),
            ("pkg.beta.Widget.render", "src/beta.rs"),
            ("pkg.gamma.delta.Widget.render", "src/gamma.rs"),
        ]
        .into_iter()
        .map(|(fqn, file)| {
            mock_resolution_entity_with_kind(
                "render",
                fqn,
                Some("Widget"),
                file,
                EntityKind::Method,
            )
        })
        .collect();
        // `pkg.alpha.Widget.render` is the lexicographically smallest match.
        let expected_target = entities[0].uuid;

        let mut caller = mock_resolution_entity_with_kind(
            "caller",
            "pkg.app.caller",
            None,
            "src/app.rs",
            EntityKind::Method,
        );
        caller.reference_intents.push(ReferenceIntent::Call {
            method: "render".to_string(),
            receiver: Some("widget".to_string()),
            line: 1,
            arg_count: None,
        });
        entities.push(caller);

        let rels = assert_resolution_deterministic(&entities, 64);
        assert_eq!(
            rels,
            HashSet::from([(
                "pkg.app.caller".to_string(),
                expected_target,
                RelationshipType::Calls
            )]),
            "an ambiguous receiver match must resolve to the lexicographically \
             smallest matching FQN"
        );
    }

    /// Same substring collision, but the caller lives in the same file as one
    /// of the candidates: same-file locality must win over the FQN ordering.
    #[test]
    fn test_resolution_prefers_same_file_candidate_in_fqn_scan() {
        let mut entities: Vec<ResolutionEntity> = [
            ("pkg.alpha.Widget.render", "src/alpha.rs"),
            ("pkg.beta.Widget.render", "src/beta.rs"),
            ("pkg.gamma.delta.Widget.render", "src/gamma.rs"),
        ]
        .into_iter()
        .map(|(fqn, file)| {
            mock_resolution_entity_with_kind(
                "render",
                fqn,
                Some("Widget"),
                file,
                EntityKind::Method,
            )
        })
        .collect();
        // Same file as `pkg.beta.Widget.render`, which is NOT the smallest FQN.
        let expected_target = entities[1].uuid;

        let mut caller = mock_resolution_entity_with_kind(
            "caller",
            "pkg.app.caller",
            None,
            "src/beta.rs",
            EntityKind::Method,
        );
        caller.reference_intents.push(ReferenceIntent::Call {
            method: "render".to_string(),
            receiver: Some("widget".to_string()),
            line: 1,
            arg_count: None,
        });
        entities.push(caller);

        let rels = assert_resolution_deterministic(&entities, 64);
        assert_eq!(
            rels,
            HashSet::from([(
                "pkg.app.caller".to_string(),
                expected_target,
                RelationshipType::Calls
            )]),
            "a candidate declared in the caller's own file must win over the FQN ordering"
        );
    }

    /// Regression: `lookup_method_in_class_hierarchy` matched classes by
    /// suffix (`class_fqn.ends_with(".ChatModel")`) and returned the first hit
    /// out of `uuid_to_fqn`. Several `ChatModel` classes across modules each
    /// own a `run` method, so `super.run()` landed on an arbitrary one
    /// depending on `RandomState`.
    #[test]
    fn test_resolution_deterministic_under_class_suffix_collisions() {
        let mut entities: Vec<ResolutionEntity> = [
            ("pkg.alpha.ChatModel", "src/alpha.rs"),
            ("pkg.beta.ChatModel", "src/beta.rs"),
            ("pkg.gamma.delta.ChatModel", "src/gamma.rs"),
        ]
        .into_iter()
        .map(|(fqn, file)| {
            mock_resolution_entity_with_kind("ChatModel", fqn, None, file, EntityKind::Class)
        })
        .collect();

        let mut run_methods: Vec<ResolutionEntity> = [
            ("pkg.alpha.ChatModel.run", "src/alpha.rs"),
            ("pkg.beta.ChatModel.run", "src/beta.rs"),
            ("pkg.gamma.delta.ChatModel.run", "src/gamma.rs"),
        ]
        .into_iter()
        .map(|(fqn, file)| {
            mock_resolution_entity_with_kind(
                "run",
                fqn,
                Some("ChatModel"),
                file,
                EntityKind::Method,
            )
        })
        .collect();
        // `pkg.alpha.ChatModel` is the lexicographically smallest class FQN
        // matching the `.ChatModel` suffix.
        let expected_target = run_methods[0].uuid;
        entities.append(&mut run_methods);

        // `Child extends ChatModel` populates the extends map that
        // `resolve_super_call` walks. The parent name itself is ambiguous
        // (three `ChatModel` classes), so that edge is skipped as ambiguous
        // and the expected set below holds only the super call.
        let mut child = mock_resolution_entity_with_kind(
            "Child",
            "pkg.app.Child",
            None,
            "src/app.rs",
            EntityKind::Class,
        );
        child.reference_intents.push(ReferenceIntent::Extends {
            parent: "ChatModel".to_string(),
            line: 1,
        });
        entities.push(child);

        let mut caller = mock_resolution_entity_with_kind(
            "caller",
            "pkg.app.Child.caller",
            Some("Child"),
            "src/app.rs",
            EntityKind::Method,
        );
        caller.reference_intents.push(ReferenceIntent::Call {
            method: "run".to_string(),
            receiver: Some("super".to_string()),
            line: 1,
            arg_count: None,
        });
        entities.push(caller);

        let rels = assert_resolution_deterministic(&entities, 64);
        assert_eq!(
            rels,
            HashSet::from([(
                "pkg.app.Child.caller".to_string(),
                expected_target,
                RelationshipType::Calls
            )]),
            "an ambiguous class-suffix match must resolve to the \
             lexicographically smallest matching class FQN"
        );
    }

    /// Regression: `resolve_vcl_include`'s basename fallback scanned
    /// `fqn_to_uuid` and returned the first hit. Several VCL files can share a
    /// basename (`alpha/shared.vcl`, `beta/shared.vcl`), so the `Includes`
    /// edge landed on an arbitrary one depending on `RandomState`.
    #[test]
    fn test_resolution_deterministic_under_vcl_basename_collisions() {
        let mut entities: Vec<ResolutionEntity> = [
            "vcl:myrepo:alpha/shared.vcl",
            "vcl:myrepo:beta/shared.vcl",
            "vcl:myrepo:gamma/delta/shared.vcl",
        ]
        .into_iter()
        .map(|fqn| {
            mock_resolution_entity_with_kind(
                "shared",
                fqn,
                None,
                fqn.trim_start_matches("vcl:myrepo:"),
                EntityKind::Function,
            )
        })
        .collect();
        // `vcl:myrepo:alpha/shared.vcl` is the lexicographically smallest
        // FQN with the `shared.vcl` basename.
        let expected_target = entities[0].uuid;

        let mut caller = mock_resolution_entity_with_kind(
            "entry",
            "vcl:myrepo:entry.vcl",
            None,
            "entry.vcl",
            EntityKind::Function,
        );
        caller.reference_intents.push(ReferenceIntent::VclInclude {
            path: "shared.vcl".to_string(),
            line: 1,
        });
        entities.push(caller);

        let rels = assert_resolution_deterministic(&entities, 64);
        assert_eq!(
            rels,
            HashSet::from([(
                "vcl:myrepo:entry.vcl".to_string(),
                expected_target,
                RelationshipType::Includes
            )]),
            "an ambiguous VCL include basename match must resolve to the \
             lexicographically smallest matching FQN"
        );
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
        let callee = mock_resolution_entity_with_kind(
            "Target",
            "com.Target",
            None,
            "test/target.java",
            EntityKind::Class,
        );
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
