use anyhow::Result;
use rayon::prelude::*;
use std::collections::{HashMap, HashSet};
use tracing::{info, warn};
use uuid::Uuid;

use crate::config::Config;
use crate::db::graph::{GraphDb, QueryExt, UpsertExt};
use crate::models::{EntityKind, ReferenceIntent, RelationshipType, ResolutionEntity};

/// Resolve cross-repository relationships and persist them to Neo4j.
pub async fn resolve_and_save_relationships(
    entities: &mut [ResolutionEntity],
    graph_db: &GraphDb,
    cfg: &Config,
) -> Result<()> {
    if !entities.is_empty() {
        // Auto-discover dependency repos from DEPENDS_ON relationships in Neo4j
        let auto_deps = graph_db
            .find_repo_dependencies(&cfg.repo_name, 3)
            .await
            .unwrap_or_default();

        let mut repos_to_load = vec![cfg.repo_name.clone()];
        repos_to_load.extend(cfg.dependency_repos.clone()); // manual overrides
        repos_to_load.extend(auto_deps.clone()); // auto-discovered
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

        resolve_reference_intents_with_context(entities, fqn_to_uuid, name_to_uuids);

        // Create typed relationships (CALLS, EXTENDS, IMPLEMENTS, REFERENCES)
        info!("Creating typed relationships in Neo4j...");
        graph_db.upsert_relationships(entities).await?;
    }
    Ok(())
}

/// Perform cross-repo dependency linking: upsert Repository nodes
/// from ProjectIdentity entities and create DEPENDS_ON edges.
pub async fn link_cross_repo_dependencies(
    entities: &[ResolutionEntity],
    graph_db: &GraphDb,
    cfg: &Config,
) -> Result<()> {
    // Step 1: Select the primary ProjectIdentity (closest to repo root)
    // and upsert a single Repository node.
    //
    // When a repo contains multiple build files (e.g., Cargo.toml at the root
    // plus test fixtures like tests/testing_files/sample_build.gradle), each
    // emits a ProjectIdentity.  upsert_repository uses MERGE + SET, so the
    // last identity processed would overwrite the fields (build_system,
    // group_id, artifact_id) with test-fixture data.  We avoid this by picking
    // the ProjectIdentity whose file is closest to the repository root.
    let project_identities: Vec<&ResolutionEntity> = entities
        .iter()
        .filter(|e| e.kind == EntityKind::ProjectIdentity)
        .collect();

    if let Some(primary) = project_identities.iter().min_by_key(|e| {
        // Compute directory depth relative to the repo root.
        // Cargo.toml at root              → 0
        // tests/testing_files/build.gradle → 2
        std::path::Path::new(&e.file_path)
            .strip_prefix(std::path::Path::new(&cfg.repo_path))
            .map(|p| p.components().count().saturating_sub(1))
            .unwrap_or(usize::MAX) // fallback: treat as deepest so it loses
    }) {
        let build_system = parse_build_system_from_fqn(&primary.fqn);
        let (group_id, artifact_id) = parse_artifact_identity(&primary.fqn, build_system);

        graph_db
            .upsert_repository(
                &cfg.repo_name,
                build_system,
                group_id,
                artifact_id,
                parse_version_from_signature(&primary.signature),
            )
            .await?;
    }

    // Step 2: Match BuildDependency entities against Repository nodes
    for entity in entities {
        if entity.kind == EntityKind::BuildDependency
            && let Some(matched_repo) =
                match_dependency_to_repository(&entity.name, graph_db).await?
            && matched_repo != cfg.repo_name
        {
            graph_db
                .upsert_repo_dependency(&cfg.repo_name, &matched_repo)
                .await?;
            info!(
                "Cross-repo link: '{}' -> '{}' (via build dependency: {})",
                cfg.repo_name, matched_repo, entity.name
            );
        }
    }

    Ok(())
}

fn parse_build_system_from_fqn(fqn: &str) -> &str {
    if fqn.starts_with("maven:") {
        "maven"
    } else if fqn.starts_with("gradle:") {
        "gradle"
    } else if fqn.starts_with("cargo:") {
        "cargo"
    } else if fqn.starts_with("npm:") {
        "npm"
    } else {
        "unknown"
    }
}

fn parse_artifact_identity<'a>(fqn: &'a str, build_system: &str) -> (&'a str, &'a str) {
    let prefix = format!("{}:", build_system);
    let rest = fqn.strip_prefix(&prefix).unwrap_or(fqn);

    match build_system {
        "maven" | "gradle" => {
            let mut parts = rest.splitn(2, ':');
            (
                parts.next().unwrap_or("unknown"),
                parts.next().unwrap_or(rest),
            )
        }
        "cargo" => ("", rest),
        "npm" => {
            if rest.starts_with('@') {
                // Scoped package: @scope/name -> group_id = "@scope", artifact_id = "name"
                let mut parts = rest.splitn(2, '/');
                (
                    parts.next().unwrap_or("unknown"),
                    parts.next().unwrap_or(rest),
                )
            } else {
                ("", rest)
            }
        }
        _ => ("", rest),
    }
}

fn parse_version_from_signature(signature: &Option<String>) -> &str {
    signature
        .as_deref()
        .and_then(|s| {
            s.strip_prefix("version: ")
                .and_then(|v| v.split(',').next())
        })
        .unwrap_or("unknown")
}

async fn match_dependency_to_repository(
    dep_name: &str,
    graph_db: &GraphDb,
) -> Result<Option<String>> {
    // Maven/Gradle: groupId:artifactId:version
    if let Some((group_id, artifact_id)) = parse_maven_style_dep(dep_name) {
        if let Some(repo) = graph_db
            .find_repository_by_artifact(group_id, artifact_id, "maven")
            .await?
        {
            return Ok(Some(repo));
        }
        if let Some(repo) = graph_db
            .find_repository_by_artifact(group_id, artifact_id, "gradle")
            .await?
        {
            return Ok(Some(repo));
        }
    }

    // Cargo: dep_name is formatted as "crate_name:version" (e.g., "serde:1.0").
    // The "scope: compile" text lives in entity.signature, not entity.name.
    // We extract the crate name by splitting on ':' and taking the first part.
    // No other build system uses the plain "word:version" format, so this
    // is a reliable heuristic for Cargo dependencies.
    if let Some(crate_name) = dep_name.split(':').next()
        && !crate_name.contains('.')
        && crate_name != "helm"
        && crate_name != "npm"
        && let Some(repo) = graph_db
            .find_repository_by_artifact("", crate_name, "cargo")
            .await?
    {
        return Ok(Some(repo));
    }

    // npm: dep_name could be "npm:name:version"
    if let Some(pkg) = dep_name.strip_prefix("npm:") {
        let name = pkg.split(':').next().unwrap_or(pkg);
        let (group_id, artifact_id) = if name.starts_with('@') {
            let mut parts = name.splitn(2, '/');
            (
                parts.next().unwrap_or("unknown"),
                parts.next().unwrap_or(name),
            )
        } else {
            ("", name)
        };
        if let Some(repo) = graph_db
            .find_repository_by_artifact(group_id, artifact_id, "npm")
            .await?
        {
            return Ok(Some(repo));
        }
    }

    // Helm: dep_name could be "helm:chart_name:version"
    if let Some(chart) = dep_name.strip_prefix("helm:") {
        let name = chart.split(':').next().unwrap_or(chart);
        // Helm charts might be matched by artifact_id
        if let Some(repo) = graph_db
            .find_repository_by_artifact("", name, "helm")
            .await?
        {
            return Ok(Some(repo));
        }
    }

    Ok(None)
}

fn parse_maven_style_dep(dep_name: &str) -> Option<(&str, &str)> {
    let after_prefix = if let Some(colon_idx) = dep_name.find(':') {
        let prefix = &dep_name[..colon_idx];
        if prefix.contains('.') {
            dep_name
        } else {
            &dep_name[colon_idx + 1..]
        }
    } else {
        dep_name
    };

    let parts: Vec<&str> = after_prefix.split(':').collect();
    if parts.len() >= 2 {
        Some((parts[0], parts[1]))
    } else {
        None
    }
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

    // Deduplicate UUIDs in name_to_uuids to avoid false positives when checking len() == 1.
    // This happens because load_entity_mappings may have already loaded the entities from Neo4j
    // that are also in the current batch.
    for uuids in name_to_uuids.values_mut() {
        uuids.sort();
        uuids.dedup();
    }

    // Build alias map for cross-file require/import resolution
    let alias_map = build_alias_map(entities, &uuid_to_file);

    // Resolve reference intents for each entity — parallelized via Rayon.
    // All context maps (fqn_to_uuid, name_to_uuids, etc.) are read-only
    // at this point, so no synchronization is needed.
    entities.par_iter_mut().for_each(|entity| {
        let reference_intents = entity.reference_intents.clone();

        // Deduplication set to prevent duplicate relationships.
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
}

/// Build an alias map from alias UUID → original definition UUID by resolving
/// `require()` / `import` module paths to entities in the target file.
///
/// Cycles (circular requires) are resolved deterministically: the entity with the
/// smallest UUID in the cycle is chosen as representative and all members point to
/// it. Self-loops are skipped with a warning.
fn build_alias_map(
    entities: &[ResolutionEntity],
    uuid_to_file: &HashMap<Uuid, String>,
) -> HashMap<Uuid, Uuid> {
    let mut alias_map: HashMap<Uuid, Uuid> = HashMap::new();

    let mut file_entities: HashMap<&str, Vec<&ResolutionEntity>> = HashMap::new();
    for e in entities {
        file_entities
            .entry(e.file_path.as_str())
            .or_default()
            .push(e);
    }

    let mut file_defaults: HashMap<&str, Uuid> = HashMap::new();
    for e in entities {
        if let Some(ref default_name) = e.default_export
            && let Some(ents) = file_entities.get(e.file_path.as_str())
            && let Some(target) = ents.iter().find(|en| en.name == *default_name)
        {
            file_defaults.insert(e.file_path.as_str(), target.uuid);
        }
    }

    for entity in entities {
        if let Some(ref module_path) = entity.alias_module_path
            && !module_path.is_empty()
            && let Some(source_file) = uuid_to_file.get(&entity.uuid)
            && let Some(target_file) = resolve_module_path(source_file, module_path, &file_entities)
            && let Some(target_entities) = file_entities.get(target_file.as_str())
        {
            let target = target_entities
                .iter()
                .find(|e| e.name == entity.name)
                .or_else(|| {
                    entity
                        .original_export_name
                        .as_ref()
                        .and_then(|original| target_entities.iter().find(|e| e.name == *original))
                })
                .or_else(|| {
                    file_defaults
                        .get(target_file.as_str())
                        .and_then(|&default_uuid| {
                            target_entities.iter().find(|e| e.uuid == default_uuid)
                        })
                });
            if let Some(t) = target {
                if entity.uuid == t.uuid {
                    warn!(
                        "Skipping self-referential alias for entity {} in {}",
                        entity.name, entity.file_path
                    );
                } else {
                    alias_map.insert(entity.uuid, t.uuid);
                }
            }
        }
    }

    // Transitive closure with cycle resolution.
    // If A → B and B → C, collapse to A → C (single-hop).
    // For cycles, pick the smallest UUID as the canonical representative and
    // make all other cycle members point to it directly. The representative
    // itself is removed from the map so it acts as the terminal.
    let keys: Vec<Uuid> = alias_map.keys().copied().collect();
    for key in keys {
        if !alias_map.contains_key(&key) {
            continue; // already processed as part of a cycle
        }
        let mut current = key;
        let mut visited_order: Vec<Uuid> = vec![current];
        let mut visited_set: HashSet<Uuid> = HashSet::from([current]);
        let mut cycle_detected = false;
        while let Some(&next) = alias_map.get(&current) {
            if visited_set.contains(&next) {
                cycle_detected = true;
                break;
            }
            visited_set.insert(next);
            visited_order.push(next);
            current = next;
        }
        let terminal = if cycle_detected {
            // Find where the cycle starts in visited_order
            let back_edge_target = alias_map.get(&current).copied().unwrap_or(current);
            let cycle_start_idx = visited_order
                .iter()
                .position(|u| *u == back_edge_target)
                .unwrap_or(0);
            let representative = visited_order[cycle_start_idx..]
                .iter()
                .min()
                .copied()
                .unwrap_or(current);
            warn!(
                "Alias cycle detected involving {} entities; collapsing to representative {}",
                visited_order.len() - cycle_start_idx,
                representative
            );
            // Redirect all cycle members to the representative.
            for &member in &visited_order[cycle_start_idx..] {
                if member != representative {
                    alias_map.insert(member, representative);
                }
            }
            // Remove the representative from the map so it acts as a terminal.
            alias_map.remove(&representative);
            representative
        } else {
            current
        };
        if !cycle_detected && terminal != key {
            alias_map.insert(key, terminal);
        }
    }

    alias_map
}

/// Resolve a JS/TS module specifier to a file path by matching against known entities.
fn resolve_module_path(
    from_file: &str,
    module_spec: &str,
    file_entities: &HashMap<&str, Vec<&ResolutionEntity>>,
) -> Option<String> {
    use std::path::{Component, Path, PathBuf};

    let from = Path::new(from_file);
    let parent = from.parent()?;

    let mut resolved = PathBuf::new();
    for component in parent.join(module_spec).components() {
        match component {
            Component::ParentDir => {
                resolved.pop();
            }
            Component::Normal(c) => {
                resolved.push(c);
            }
            Component::CurDir => {}
            _ => {
                resolved.push(component);
            }
        }
    }

    let extensions = ["js", "ts", "tsx", "jsx"];
    let index_extensions = ["js", "ts"];

    let exact = resolved.to_string_lossy().to_string();
    if file_entities.contains_key(exact.as_str()) {
        return Some(exact);
    }
    for ext in &extensions {
        let with_ext = resolved.with_extension(ext).to_string_lossy().to_string();
        if file_entities.contains_key(with_ext.as_str()) {
            return Some(with_ext);
        }
    }
    for ext in &index_extensions {
        let index_file = resolved
            .join(format!("index.{ext}"))
            .to_string_lossy()
            .to_string();
        if file_entities.contains_key(index_file.as_str()) {
            return Some(index_file);
        }
    }
    None
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
            if let Some(same_file_uuid) =
                find_entity_in_same_file(uuids, &caller_file_path, uuid_to_file)
            {
                return Some(same_file_uuid);
            }
            if let Some(ac) = intent.arg_count
                && let Some(ac_map) = uuid_to_arg_count
            {
                let ac_matches: Vec<&Uuid> = uuids
                    .iter()
                    .filter(|u| ac_map.get(u) == Some(&ac))
                    .collect();
                if ac_matches.len() == 1 {
                    return Some(*ac_matches[0]);
                }
            }
            if uuids.len() == 1 {
                return uuids.first().copied();
            }
        }
    }

    // Strategy 4: Fallback for local calls without enclosing class.
    if intent.receiver.is_none()
        && let Some(uuids) = name_to_uuids.get(&intent.method)
    {
        if let Some(same_file_uuid) =
            find_entity_in_same_file(uuids, &caller_file_path, uuid_to_file)
        {
            return Some(same_file_uuid);
        }
        if let Some(ac) = intent.arg_count
            && let Some(ac_map) = uuid_to_arg_count
        {
            let ac_matches: Vec<&Uuid> = uuids
                .iter()
                .filter(|u| ac_map.get(u) == Some(&ac))
                .collect();
            if ac_matches.len() == 1 {
                return Some(*ac_matches[0]);
            }
        }
        if uuids.len() == 1 {
            return uuids.first().copied();
        }
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
            kind: EntityKind::Method,
            enclosing_class: enclosing.map(|s| s.to_string()),
            signature: None,
            reference_intents: Vec::new(),
            relationships: Vec::new(),
            alias_module_path: None,
            original_export_name: None,
            default_export: None,
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
            kind: EntityKind::Function,
            name: "find_nearest_entity_by_line".to_string(),
            fqn: "knot::pipeline::parser::orphans::find_nearest_entity_by_line".to_string(),
            file_path: "src/pipeline/parser/orphans.rs".to_string(),
            enclosing_class: None,
            signature: None,
            reference_intents: Vec::new(),
            relationships: Vec::new(),
            alias_module_path: None,
            original_export_name: None,
            default_export: None,
        };

        let rust_fn = ResolutionEntity {
            uuid: Uuid::new_v4(),
            kind: EntityKind::Function,
            name: "find_nearest_entity_by_line".to_string(),
            fqn: "knot::pipeline::parser::languages::rust::find_nearest_entity_by_line".to_string(),
            file_path: "src/pipeline/parser/languages/rust.rs".to_string(),
            enclosing_class: None,
            signature: None,
            reference_intents: Vec::new(),
            relationships: Vec::new(),
            alias_module_path: None,
            original_export_name: None,
            default_export: None,
        };

        // Create a caller function in rust.rs that calls find_nearest_entity_by_line
        let rust_caller = ResolutionEntity {
            uuid: Uuid::new_v4(),
            kind: EntityKind::Function,
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
            alias_module_path: None,
            original_export_name: None,
            default_export: None,
        };

        // Create a caller function in orphans.rs that calls find_nearest_entity_by_line
        let orphans_caller = ResolutionEntity {
            uuid: Uuid::new_v4(),
            kind: EntityKind::Function,
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
            alias_module_path: None,
            original_export_name: None,
            default_export: None,
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
            kind: EntityKind::Class,
            name: "Animal".to_string(),
            fqn: "Animal".to_string(),
            file_path: "animals.py".to_string(),
            enclosing_class: None,
            signature: None,
            reference_intents: Vec::new(),
            relationships: Vec::new(),
            alias_module_path: None,
            original_export_name: None,
            default_export: None,
        };
        let animal_speak_method = ResolutionEntity {
            uuid: Uuid::new_v4(),
            kind: EntityKind::Method,
            name: "speak".to_string(),
            fqn: "Animal.speak".to_string(),
            file_path: "animals.py".to_string(),
            enclosing_class: Some("Animal".to_string()),
            signature: None,
            reference_intents: Vec::new(),
            relationships: Vec::new(),
            alias_module_path: None,
            original_export_name: None,
            default_export: None,
        };

        let dog_class = ResolutionEntity {
            uuid: Uuid::new_v4(),
            kind: EntityKind::Class,
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
            alias_module_path: None,
            original_export_name: None,
            default_export: None,
        };
        let dog_compute = ResolutionEntity {
            uuid: Uuid::new_v4(),
            kind: EntityKind::Method,
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
            alias_module_path: None,
            original_export_name: None,
            default_export: None,
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
            kind: EntityKind::Function,
            name: "do_thing".to_string(),
            fqn: "do_thing".to_string(),
            file_path: "lora.py".to_string(),
            enclosing_class: None,
            signature: None,
            reference_intents: Vec::new(),
            relationships: Vec::new(),
            alias_module_path: None,
            original_export_name: None,
            default_export: None,
        };

        // Class with method of same name
        let my_class = ResolutionEntity {
            uuid: Uuid::new_v4(),
            kind: EntityKind::Class,
            name: "MyLoader".to_string(),
            fqn: "MyLoader".to_string(),
            file_path: "nodes.py".to_string(),
            enclosing_class: None,
            signature: None,
            reference_intents: Vec::new(),
            relationships: Vec::new(),
            alias_module_path: None,
            original_export_name: None,
            default_export: None,
        };
        // Class method with same name as module function (different file, different FQN)
        let class_method = ResolutionEntity {
            uuid: Uuid::new_v4(),
            kind: EntityKind::Method,
            name: "do_thing".to_string(),
            fqn: "MyLoader.do_thing".to_string(),
            file_path: "nodes.py".to_string(),
            enclosing_class: Some("MyLoader".to_string()),
            signature: None,
            reference_intents: Vec::new(),
            relationships: Vec::new(),
            alias_module_path: None,
            original_export_name: None,
            default_export: None,
        };
        // Another method in same class calling self.do_thing()
        let caller_method = ResolutionEntity {
            uuid: Uuid::new_v4(),
            kind: EntityKind::Method,
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
            alias_module_path: None,
            original_export_name: None,
            default_export: None,
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

    #[test]
    fn test_parse_build_system_maven() {
        assert_eq!(
            parse_build_system_from_fqn("maven:com.example:app"),
            "maven"
        );
    }

    #[test]
    fn test_parse_build_system_cargo() {
        assert_eq!(parse_build_system_from_fqn("cargo:my-crate"), "cargo");
    }

    #[test]
    fn test_parse_build_system_npm() {
        assert_eq!(parse_build_system_from_fqn("npm:@scope/package"), "npm");
    }

    #[test]
    fn test_parse_build_system_gradle() {
        assert_eq!(
            parse_build_system_from_fqn("gradle:com.example:app"),
            "gradle"
        );
    }

    #[test]
    fn test_parse_artifact_identity_maven() {
        let (gid, aid) = parse_artifact_identity("maven:com.example:my-app", "maven");
        assert_eq!(gid, "com.example");
        assert_eq!(aid, "my-app");
    }

    #[test]
    fn test_parse_artifact_identity_cargo() {
        let (gid, aid) = parse_artifact_identity("cargo:my-crate", "cargo");
        assert_eq!(gid, "");
        assert_eq!(aid, "my-crate");
    }

    #[test]
    fn test_parse_artifact_identity_npm_scoped() {
        let (gid, aid) = parse_artifact_identity("npm:@scope/my-pkg", "npm");
        assert_eq!(gid, "@scope");
        assert_eq!(aid, "my-pkg");
    }

    #[test]
    fn test_parse_artifact_identity_npm_unscoped() {
        let (gid, aid) = parse_artifact_identity("npm:my-pkg", "npm");
        assert_eq!(gid, "");
        assert_eq!(aid, "my-pkg");
    }

    #[test]
    fn test_parse_version_from_signature() {
        assert_eq!(
            parse_version_from_signature(&Some("version: 1.0.0, build_system: maven".to_string())),
            "1.0.0"
        );
    }

    #[test]
    fn test_parse_version_from_signature_none() {
        assert_eq!(parse_version_from_signature(&None), "unknown");
    }

    #[test]
    fn test_parse_maven_style_dep_standard() {
        let result = parse_maven_style_dep("org.springframework:spring-core:5.3.29");
        assert_eq!(result, Some(("org.springframework", "spring-core")));
    }

    #[test]
    fn test_parse_maven_style_dep_with_config() {
        let result = parse_maven_style_dep("implementation:org.springframework:spring-core:5.3.29");
        // Result should extract the groupId:artifactId from the dep
        assert!(result.is_some());
        assert_eq!(result.unwrap().0, "org.springframework");
    }

    #[test]
    fn test_parse_maven_style_dep_short() {
        let result = parse_maven_style_dep("com.example:my-lib");
        assert_eq!(result, Some(("com.example", "my-lib")));
    }

    fn mock_resolution_entity_at(
        name: &str,
        fqn: &str,
        enclosing: Option<&str>,
        file_path: &str,
    ) -> ResolutionEntity {
        ResolutionEntity {
            uuid: Uuid::new_v4(),
            name: name.to_string(),
            fqn: fqn.to_string(),
            file_path: file_path.to_string(),
            kind: EntityKind::Method,
            enclosing_class: enclosing.map(|s| s.to_string()),
            signature: None,
            reference_intents: Vec::new(),
            relationships: Vec::new(),
            alias_module_path: None,
            original_export_name: None,
            default_export: None,
        }
    }

    #[test]
    fn test_resolve_fallback_uniqueness_guard() {
        // Scenario: Two entities named "new" in different files. Fallback should return None (ambiguous).
        let mut caller = mock_resolution_entity("main", "App.main", None);
        let callee1 =
            mock_resolution_entity_at("new", "Class1.new", Some("Class1"), "test/file1.java");
        let callee2 =
            mock_resolution_entity_at("new", "Class2.new", Some("Class2"), "test/file2.java");

        caller.reference_intents.push(ReferenceIntent::Call {
            method: "new".to_string(),
            receiver: None,
            line: 10,
            arg_count: None,
        });

        let mut entities = vec![caller, callee1, callee2];
        resolve_reference_intents(&mut entities);

        // Should NOT have resolved to either (ambiguous)
        assert_eq!(entities[0].relationships.len(), 0);
    }

    #[test]
    fn test_resolve_fallback_arg_count_disambiguation() {
        // Scenario: Two entities named "add" in different files, but with different arg counts.
        let mut caller = mock_resolution_entity("main", "App.main", None);
        let mut callee1 =
            mock_resolution_entity_at("add", "Calc.add", Some("Calc"), "test/calc.java");
        callee1.signature = Some("add(int, int)".to_string()); // 2 args

        let mut callee2 =
            mock_resolution_entity_at("add", "List.add", Some("List"), "test/list.java");
        callee2.signature = Some("add(int)".to_string()); // 1 arg

        caller.reference_intents.push(ReferenceIntent::Call {
            method: "add".to_string(),
            receiver: None,
            line: 10,
            arg_count: Some(2),
        });

        let mut entities = vec![caller, callee1, callee2];
        resolve_reference_intents(&mut entities);

        // Should have resolved to callee1 (2 args)
        assert_eq!(entities[0].relationships.len(), 1);
        assert_eq!(entities[0].relationships[0].0, entities[1].uuid);
    }

    #[test]
    fn test_resolve_context_deduplication() {
        // Scenario: Entity exists in Neo4j (already loaded) AND in current batch.
        // name_to_uuids should be deduplicated.
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

        // Simulate duplicate: same UUID in context and batch
        let fqn_to_uuid = HashMap::from([("unique_func".to_string(), uuid)]);
        let name_to_uuids = HashMap::from([("unique_func".to_string(), vec![uuid])]);

        let mut batch = vec![caller, entity];

        resolve_reference_intents_with_context(&mut batch, fqn_to_uuid, name_to_uuids);

        // Verification: If deduplication works, len() == 1 and it resolves.
        assert_eq!(batch[0].relationships.len(), 1);
        assert_eq!(batch[0].relationships[0].0, uuid);
    }

    // ── Alias map tests ──────────────────────────────────────────

    fn mock_entity_with_alias(
        name: &str,
        file: &str,
        alias_module_path: Option<&str>,
        default_export: Option<&str>,
    ) -> ResolutionEntity {
        ResolutionEntity {
            uuid: Uuid::new_v4(),
            name: name.to_string(),
            fqn: name.to_string(),
            file_path: file.to_string(),
            kind: EntityKind::Constant,
            enclosing_class: None,
            signature: None,
            reference_intents: Vec::new(),
            relationships: Vec::new(),
            alias_module_path: alias_module_path.map(|s| s.to_string()),
            original_export_name: None,
            default_export: default_export.map(|s| s.to_string()),
        }
    }

    fn build_alias_map_for_test(entities: &[ResolutionEntity]) -> HashMap<Uuid, Uuid> {
        let uuid_to_file: HashMap<Uuid, String> = entities
            .iter()
            .map(|e| (e.uuid, e.file_path.clone()))
            .collect();
        build_alias_map(entities, &uuid_to_file)
    }

    #[test]
    fn test_alias_map_skips_self_loop() {
        // Entity A in file_a.js has alias → file_a.js (resolves to itself)
        let a = mock_entity_with_alias("A", "file_a.js", Some("./file_a"), None);
        let entities = vec![a];
        let map = build_alias_map_for_test(&entities);
        assert!(map.is_empty(), "Self-loop should be skipped; got {:?}", map);
    }

    #[test]
    fn test_alias_map_two_node_cycle_picks_min_uuid() {
        // CycleX in file_a.js → file_b.js, CycleX in file_b.js → file_a.js
        // Both share the same name ("CycleX") so the alias_map forms a cycle.
        let a = {
            let e = mock_entity_with_alias("CycleX", "file_a.js", Some("./file_b"), None);
            // Override UUID for deterministic ordering
            ResolutionEntity {
                uuid: Uuid::parse_str("00000000-0000-0000-0000-000000000002").unwrap(), // larger
                ..e
            }
        };
        let b = {
            let e = mock_entity_with_alias("CycleX", "file_b.js", Some("./file_a"), None);
            ResolutionEntity {
                uuid: Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(), // smaller
                ..e
            }
        };
        let uuid_a = a.uuid;
        let uuid_b = b.uuid;

        let entities = vec![a, b];
        let map = build_alias_map_for_test(&entities);

        // b (smaller UUID) is the canonical representative → removed from keys.
        // a (larger UUID) points to b directly (single-hop).
        assert!(
            !map.contains_key(&uuid_b),
            "Smaller UUID should be terminal (not a key); map: {:?}",
            map
        );
        assert_eq!(
            map.get(&uuid_a),
            Some(&uuid_b),
            "Larger UUID should point to smaller; map: {:?}",
            map
        );
    }

    #[test]
    fn test_alias_map_three_node_cycle() {
        // CycleX in f_a.js → f_b.js, CycleX in f_b.js → f_c.js, CycleX in f_c.js → f_a.js
        let a = {
            let e = mock_entity_with_alias("CycleX", "f_a.js", Some("./f_b"), None);
            ResolutionEntity {
                uuid: Uuid::parse_str("00000000-0000-0000-0000-000000000003").unwrap(), // largest
                ..e
            }
        };
        let b = {
            let e = mock_entity_with_alias("CycleX", "f_b.js", Some("./f_c"), None);
            ResolutionEntity {
                uuid: Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(), // smallest
                ..e
            }
        };
        let c = {
            let e = mock_entity_with_alias("CycleX", "f_c.js", Some("./f_a"), None);
            ResolutionEntity {
                uuid: Uuid::parse_str("00000000-0000-0000-0000-000000000002").unwrap(),
                ..e
            }
        };
        let uuid_a = a.uuid;
        let uuid_b = b.uuid;
        let uuid_c = c.uuid;

        let entities = vec![a, b, c];
        let map = build_alias_map_for_test(&entities);

        // b (smallest UUID) is the canonical representative → removed from keys.
        // a and c should point to b directly (single-hop).
        assert!(
            !map.contains_key(&uuid_b),
            "Smallest UUID should be terminal; map: {:?}",
            map
        );
        assert_eq!(
            map.get(&uuid_a),
            Some(&uuid_b),
            "A should point to B; map: {:?}",
            map
        );
        assert_eq!(
            map.get(&uuid_c),
            Some(&uuid_b),
            "C should point to B; map: {:?}",
            map
        );
    }

    #[test]
    fn test_alias_map_long_chain_collapses() {
        // All named "Fwd" so alias resolution by name works.
        // Fwd in fa.js → fb.js, Fwd in fb.js → fc.js, Fwd in fc.js → fd.js, Fwd in fd.js (terminal).
        let a = {
            let e = mock_entity_with_alias("Fwd", "fa.js", Some("./fb"), None);
            ResolutionEntity {
                uuid: Uuid::new_v4(),
                ..e
            }
        };
        let b = {
            let e = mock_entity_with_alias("Fwd", "fb.js", Some("./fc"), None);
            ResolutionEntity {
                uuid: Uuid::new_v4(),
                ..e
            }
        };
        let c = {
            let e = mock_entity_with_alias("Fwd", "fc.js", Some("./fd"), None);
            ResolutionEntity {
                uuid: Uuid::new_v4(),
                ..e
            }
        };
        let d = {
            let e = mock_entity_with_alias("Fwd", "fd.js", None, None);
            ResolutionEntity {
                uuid: Uuid::new_v4(),
                ..e
            }
        };
        let uuid_a = a.uuid;
        let uuid_b = b.uuid;
        let uuid_c = c.uuid;
        let uuid_d = d.uuid;

        let entities = vec![a, b, c, d];
        let map = build_alias_map_for_test(&entities);

        // All should collapse to D (single-hop)
        assert_eq!(
            map.get(&uuid_a),
            Some(&uuid_d),
            "A should point to D; map: {:?}",
            map
        );
        assert_eq!(
            map.get(&uuid_b),
            Some(&uuid_d),
            "B should point to D; map: {:?}",
            map
        );
        assert_eq!(
            map.get(&uuid_c),
            Some(&uuid_d),
            "C should point to D; map: {:?}",
            map
        );
        assert!(
            !map.contains_key(&uuid_d),
            "D should be terminal (not a key); map: {:?}",
            map
        );
    }

    #[test]
    fn test_resolve_reference_intents_terminates_with_cyclic_aliases() {
        // CycleR in file_a.js requires ./file_b  → alias resolves to CycleR in file_b.js
        // CycleR in file_b.js requires ./file_a  → alias resolves to CycleR in file_a.js
        let a = {
            let mut e = mock_entity_with_alias("CycleR", "file_a.js", Some("./file_b"), None);
            e.uuid = Uuid::parse_str("00000000-0000-0000-0000-00000000000a").unwrap();
            e.reference_intents = vec![ReferenceIntent::ValueReference {
                value_name: "CycleR".to_string(),
                line: 1,
            }];
            e
        };
        let b = {
            let mut e = mock_entity_with_alias("CycleR", "file_b.js", Some("./file_a"), None);
            e.uuid = Uuid::parse_str("00000000-0000-0000-0000-00000000000b").unwrap();
            e.reference_intents = vec![ReferenceIntent::ValueReference {
                value_name: "CycleR".to_string(),
                line: 1,
            }];
            e
        };
        let uuid_a = a.uuid;
        let uuid_b = b.uuid;

        let mut entities = vec![a, b];
        let fqn_map: HashMap<String, Uuid> =
            entities.iter().map(|e| (e.fqn.clone(), e.uuid)).collect();
        // CycleR appears in two files → name_map has 2 UUIDs
        let name_map: HashMap<String, Vec<Uuid>> =
            HashMap::from([("CycleR".to_string(), vec![uuid_a, uuid_b])]);

        resolve_reference_intents_with_context(&mut entities, fqn_map, name_map);

        // Both entities resolve their ValueReference("CycleR") via
        // name_to_uuids["CycleR"].first() → the smaller UUID (uuid_a).
        // uuid_a is the canonical representative (it was removed from
        // alias_map by the cycle resolver), so no redirect happens.
        // Both relationships should point to uuid_a.
        let rep = std::cmp::min(uuid_a, uuid_b);
        for e in &entities {
            for (target, _) in &e.relationships {
                assert_eq!(
                    *target, rep,
                    "All relationships should point to the cycle representative (min UUID)"
                );
            }
        }
    }
}
