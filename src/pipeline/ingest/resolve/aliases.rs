use std::collections::{HashMap, HashSet};
use tracing::warn;
use uuid::Uuid;

use crate::models::ResolutionEntity;

pub fn build_alias_map(
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

    let keys: Vec<Uuid> = alias_map.keys().copied().collect();
    for key in keys {
        if !alias_map.contains_key(&key) {
            continue;
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
            for &member in &visited_order[cycle_start_idx..] {
                if member != representative {
                    alias_map.insert(member, representative);
                }
            }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{EntityKind, ReferenceIntent, ResolutionEntity};
    use crate::pipeline::ingest::resolve::resolve_reference_intents_with_context;

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
            enclosing_class_fqn: None,
            signature: None,
            reference_intents: Vec::new(),
            relationships: Vec::new(),
            alias_module_path: alias_module_path.map(|s| s.to_string()),
            original_export_name: None,
            default_export: default_export.map(|s| s.to_string()),
            is_test_context: false,
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
        let a = mock_entity_with_alias("A", "file_a.js", Some("./file_a"), None);
        let entities = vec![a];
        let map = build_alias_map_for_test(&entities);
        assert!(map.is_empty(), "Self-loop should be skipped; got {:?}", map);
    }

    #[test]
    fn test_alias_map_two_node_cycle_picks_min_uuid() {
        let a = {
            let e = mock_entity_with_alias("CycleX", "file_a.js", Some("./file_b"), None);
            ResolutionEntity {
                uuid: Uuid::parse_str("00000000-0000-0000-0000-000000000002").unwrap(),
                ..e
            }
        };
        let b = {
            let e = mock_entity_with_alias("CycleX", "file_b.js", Some("./file_a"), None);
            ResolutionEntity {
                uuid: Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(),
                ..e
            }
        };
        let uuid_a = a.uuid;
        let uuid_b = b.uuid;

        let entities = vec![a, b];
        let map = build_alias_map_for_test(&entities);

        assert!(!map.contains_key(&uuid_b));
        assert_eq!(map.get(&uuid_a), Some(&uuid_b));
    }

    #[test]
    fn test_alias_map_three_node_cycle() {
        let a = {
            let e = mock_entity_with_alias("CycleX", "f_a.js", Some("./f_b"), None);
            ResolutionEntity {
                uuid: Uuid::parse_str("00000000-0000-0000-0000-000000000003").unwrap(),
                ..e
            }
        };
        let b = {
            let e = mock_entity_with_alias("CycleX", "f_b.js", Some("./f_c"), None);
            ResolutionEntity {
                uuid: Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(),
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

        assert!(!map.contains_key(&uuid_b));
        assert_eq!(map.get(&uuid_a), Some(&uuid_b));
        assert_eq!(map.get(&uuid_c), Some(&uuid_b));
    }

    #[test]
    fn test_alias_map_long_chain_collapses() {
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

        assert_eq!(map.get(&uuid_a), Some(&uuid_d));
        assert_eq!(map.get(&uuid_b), Some(&uuid_d));
        assert_eq!(map.get(&uuid_c), Some(&uuid_d));
        assert!(!map.contains_key(&uuid_d));
    }

    #[test]
    fn test_resolve_reference_intents_terminates_with_cyclic_aliases() {
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
        let name_map: HashMap<String, Vec<Uuid>> =
            HashMap::from([("CycleR".to_string(), vec![uuid_a, uuid_b])]);

        resolve_reference_intents_with_context(&mut entities, fqn_map, name_map);

        let rep = std::cmp::min(uuid_a, uuid_b);
        for e in &entities {
            for (target, _) in &e.relationships {
                assert_eq!(*target, rep);
            }
        }
    }
}
