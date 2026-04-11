use uuid::Uuid;

/// Fold a 128-bit UUID into a 64-bit Qdrant point ID via XOR.
///
/// Collision probability for typical codebase sizes is negligible.
pub(crate) fn uuid_to_point_id(uuid: Uuid) -> u64 {
    let bytes = uuid.as_u128();
    let hi = (bytes >> 64) as u64;
    let lo = bytes as u64;
    hi ^ lo
}

/// Convert a Qdrant payload value to JSON.
pub(crate) fn qdrant_value_to_json(value: &qdrant_client::qdrant::Value) -> serde_json::Value {
    use qdrant_client::qdrant::value::Kind;

    match &value.kind {
        Some(Kind::StringValue(s)) => serde_json::json!(s),
        Some(Kind::IntegerValue(i)) => serde_json::json!(i),
        Some(Kind::DoubleValue(d)) => serde_json::json!(d),
        Some(Kind::BoolValue(b)) => serde_json::json!(b),
        Some(Kind::ListValue(list)) => {
            let values = list
                .values
                .iter()
                .map(qdrant_value_to_json)
                .collect::<Vec<_>>();
            serde_json::json!(values)
        }
        Some(Kind::NullValue(_)) => serde_json::json!(null),
        Some(Kind::StructValue(_)) => serde_json::json!(null),
        None => serde_json::json!(null),
    }
}
