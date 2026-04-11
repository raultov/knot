use std::collections::HashMap;
use uuid::Uuid;

use crate::models::EmbeddedEntity;

/// Resolve reference intents to actual entity UUIDs (legacy version).
///
/// This function builds lookup maps from the provided entities only.
/// For incremental indexing, use `resolve_reference_intents_with_context` instead.
pub fn resolve_reference_intents(entities: &mut [EmbeddedEntity]) {
    // Build lookup maps for efficient resolution
    let fqn_to_uuid: HashMap<String, Uuid> = entities
        .iter()
        .map(|e| (e.entity.fqn.clone(), e.entity.uuid))
        .collect();

    let name_to_uuids: HashMap<String, Vec<Uuid>> = {
        let mut map: HashMap<String, Vec<Uuid>> = HashMap::new();
        for e in entities.iter() {
            map.entry(e.entity.name.clone())
                .or_default()
                .push(e.entity.uuid);
        }
        map
    };

    resolve_reference_intents_with_context(entities, fqn_to_uuid, name_to_uuids);
}

/// Resolve reference intents using pre-loaded context (for incremental indexing).
///
/// The provided hashmaps should include ALL entities in the repository (from Neo4j),
/// not just the newly parsed ones. This enables incremental indexing where we only
/// re-parse modified files but still resolve calls to unchanged files.
pub fn resolve_reference_intents_with_context(
    entities: &mut [EmbeddedEntity],
    mut fqn_to_uuid: HashMap<String, Uuid>,
    mut name_to_uuids: HashMap<String, Vec<Uuid>>,
) {
    use crate::models::RelationshipType;

    // Merge newly parsed entities into the context
    for e in entities.iter() {
        fqn_to_uuid.insert(e.entity.fqn.clone(), e.entity.uuid);
        name_to_uuids
            .entry(e.entity.name.clone())
            .or_default()
            .push(e.entity.uuid);
    }

    // For each entity, resolve its reference_intents to UUIDs with typed relationships
    for entity in entities.iter_mut() {
        let reference_intents = entity.entity.reference_intents.clone();

        // Deduplication set: defense-in-depth to prevent duplicate relationships
        use std::collections::HashSet;
        let mut seen_relationships: HashSet<(Uuid, RelationshipType)> = HashSet::new();

        for intent in reference_intents {
            use crate::models::ReferenceIntent;
            match intent {
                ReferenceIntent::Call {
                    method, receiver, ..
                } => {
                    let call_intent = crate::models::CallIntent {
                        method,
                        receiver,
                        line: 0, // Not used in resolution
                    };
                    let resolved_uuid = resolve_single_call_intent(
                        &call_intent,
                        &entity.entity,
                        &fqn_to_uuid,
                        &name_to_uuids,
                    );

                    if let Some(uuid) = resolved_uuid {
                        let relationship = (uuid, RelationshipType::Calls);
                        if seen_relationships.insert(relationship) {
                            entity.entity.calls.push(uuid);
                            entity.entity.relationships.push(relationship);
                        }
                    }
                }
                ReferenceIntent::Extends { parent, .. } => {
                    // Resolve parent class by name
                    if let Some(uuids) = name_to_uuids.get(&parent)
                        && let Some(&uuid) = uuids.first()
                    {
                        let relationship = (uuid, RelationshipType::Extends);
                        if seen_relationships.insert(relationship) {
                            entity.entity.calls.push(uuid);
                            entity.entity.relationships.push(relationship);
                        }
                    }
                }
                ReferenceIntent::Implements { interface, .. } => {
                    // Resolve interface by name
                    if let Some(uuids) = name_to_uuids.get(&interface)
                        && let Some(&uuid) = uuids.first()
                    {
                        let relationship = (uuid, RelationshipType::Implements);
                        if seen_relationships.insert(relationship) {
                            entity.entity.calls.push(uuid);
                            entity.entity.relationships.push(relationship);
                        }
                    }
                }
                ReferenceIntent::TypeReference { type_name, .. } => {
                    // Resolve type reference by name (class or interface)
                    if let Some(uuids) = name_to_uuids.get(&type_name)
                        && let Some(&uuid) = uuids.first()
                    {
                        let relationship = (uuid, RelationshipType::References);
                        if seen_relationships.insert(relationship) {
                            entity.entity.calls.push(uuid);
                            entity.entity.relationships.push(relationship);
                        }
                    }
                }
            }
        }
    }
}

/// Legacy alias for backward compatibility.
#[deprecated = "Use resolve_reference_intents instead"]
pub fn resolve_call_intents(entities: &mut [EmbeddedEntity]) {
    resolve_reference_intents(entities);
}

/// Resolve a single CallIntent to a UUID using available context.
fn resolve_single_call_intent(
    intent: &crate::models::CallIntent,
    caller: &crate::models::ParsedEntity,
    fqn_to_uuid: &HashMap<String, Uuid>,
    name_to_uuids: &HashMap<String, Vec<Uuid>>,
) -> Option<Uuid> {
    // Strategy 1: Local call (no receiver or receiver is "this")
    // Look for a method in the same class
    if (intent.receiver.is_none() || intent.receiver.as_deref() == Some("this"))
        && let Some(enclosing_class) = &caller.enclosing_class
    {
        let fqn = format!("{}.{}", enclosing_class, intent.method);
        if let Some(&uuid) = fqn_to_uuid.get(&fqn) {
            return Some(uuid);
        }
    }

    // Strategy 2: Static call (receiver is a class name, capitalized)
    if let Some(receiver) = &intent.receiver
        && receiver.chars().next().is_some_and(|c| c.is_uppercase())
        && receiver != "this"
    {
        // Try to find a method with FQN = Class.method
        let fqn = format!("{}.{}", receiver, intent.method);
        if let Some(&uuid) = fqn_to_uuid.get(&fqn) {
            return Some(uuid);
        }
    }

    // Strategy 3: Instance call (receiver is lowercase variable/object or nested like "this.browserService")
    // Without type information, we use fuzzy matching on method name + receiver heuristics
    if let Some(receiver) = &intent.receiver {
        // If receiver contains a dot (nested like "this.browserService"), extract the last component
        let receiver_class = if receiver.contains('.') {
            receiver
                .split('.')
                .next_back()
                .map(|s| s.trim())
                .unwrap_or(receiver)
        } else {
            receiver
        };

        // If receiver_class looks like a class name (contains alphanumerics), try to find a matching method
        // by searching for a method where the class name appears at the start or end of the FQN
        if !receiver_class.is_empty() {
            // Try exact match first: ReceiverClass.method
            let exact_fqn = format!("{}.{}", receiver_class, intent.method);
            if let Some(&uuid) = fqn_to_uuid.get(&exact_fqn) {
                return Some(uuid);
            }

            // Try capitalized version: browserService -> BrowserService (heuristic for class names)
            let capitalized = if !receiver_class.is_empty() {
                let mut chars = receiver_class.chars();
                let first = chars.next().unwrap().to_uppercase().to_string();
                first + chars.as_str()
            } else {
                receiver_class.to_string()
            };

            let capitalized_fqn = format!("{}.{}", capitalized, intent.method);
            if let Some(&uuid) = fqn_to_uuid.get(&capitalized_fqn) {
                return Some(uuid);
            }

            // Fuzzy match: search in fqn_to_uuid for a method where the receiver class appears
            for (fqn, uuid) in fqn_to_uuid.iter() {
                if fqn.contains(&format!("{}.{}", receiver_class, intent.method))
                    || fqn.contains(&format!("{}.{}", capitalized, intent.method))
                {
                    return Some(*uuid);
                }
            }
        }

        // Final fallback: just match on method name
        if let Some(uuids) = name_to_uuids.get(&intent.method) {
            return uuids.first().copied();
        }
    }

    // Strategy 4: Fallback for local calls without enclosing class (top-level functions)
    if intent.receiver.is_none()
        && let Some(uuids) = name_to_uuids.get(&intent.method)
    {
        return uuids.first().copied();
    }

    None
}
