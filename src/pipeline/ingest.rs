//! Stage 5 — Ingest: dual-write to Qdrant (vector) and Neo4j (graph).
//!
//! Both writes are issued concurrently via `tokio::try_join!` to avoid
//! bottlenecking on either database. The two stores are kept in sync
//! through the shared UUID that every entity carries.

use anyhow::Result;
use std::collections::HashMap;
use tracing::info;
use uuid::Uuid;

use crate::{
    db::{graph::GraphDb, vector::VectorDb},
    models::EmbeddedEntity,
};

/// Write a batch of [`EmbeddedEntity`] records to both databases simultaneously.
/// NOTE: This only creates the nodes. Relationship edges must be created in a separate
/// pass after ALL nodes have been upserted, to prevent missing-callee failures.
pub async fn ingest_batch(
    entities: &[EmbeddedEntity],
    vector_db: &VectorDb,
    graph_db: &GraphDb,
) -> Result<()> {
    if entities.is_empty() {
        return Ok(());
    }

    info!("Ingesting batch of {} entities…", entities.len());

    // Fire both writes concurrently; surface the first failure.
    tokio::try_join!(
        vector_db.upsert(entities),
        graph_db.upsert_entities(entities),
    )?;

    info!("Batch ingestion complete");
    Ok(())
}

/// Resolve call intents to actual entity UUIDs.
///
/// This function takes raw `call_intents` (method name + receiver context) and
/// resolves them to actual method/function UUIDs based on the available entities.
/// It must be called on the FULL set of entities before ingestion.
pub fn resolve_call_intents(entities: &mut [EmbeddedEntity]) {
    // Build lookup maps for efficient resolution
    let fqn_to_uuid: HashMap<String, Uuid> = entities
        .iter()
        .map(|e| (e.entity.fqn.clone(), e.entity.uuid))
        .collect();

    let method_name_to_uuids: HashMap<String, Vec<Uuid>> = {
        let mut map: HashMap<String, Vec<Uuid>> = HashMap::new();
        for e in entities.iter() {
            map.entry(e.entity.name.clone())
                .or_default()
                .push(e.entity.uuid);
        }
        map
    };

    // For each entity, resolve its call_intents to UUIDs
    for entity in entities.iter_mut() {
        let call_intents = entity.entity.call_intents.clone();
        for intent in call_intents {
            let resolved_uuid = resolve_single_call_intent(
                &intent,
                &entity.entity,
                &fqn_to_uuid,
                &method_name_to_uuids,
            );

            if let Some(uuid) = resolved_uuid {
                entity.entity.calls.push(uuid);
            }
        }
    }
}

/// Resolve a single CallIntent to a UUID using available context.
fn resolve_single_call_intent(
    intent: &crate::models::CallIntent,
    caller: &crate::models::ParsedEntity,
    fqn_to_uuid: &HashMap<String, Uuid>,
    method_name_to_uuids: &HashMap<String, Vec<Uuid>>,
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
        if let Some(uuids) = method_name_to_uuids.get(&intent.method) {
            return uuids.first().copied();
        }
    }

    // Strategy 4: Fallback for local calls without enclosing class (top-level functions)
    if intent.receiver.is_none()
        && let Some(uuids) = method_name_to_uuids.get(&intent.method)
    {
        return uuids.first().copied();
    }

    None
}
