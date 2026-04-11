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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_uuid_to_point_id_deterministic() {
        let uuid = Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap();
        let point_id_1 = uuid_to_point_id(uuid);
        let point_id_2 = uuid_to_point_id(uuid);

        assert_eq!(
            point_id_1, point_id_2,
            "Same UUID should produce same point ID"
        );
    }

    #[test]
    fn test_uuid_to_point_id_different_uuids() {
        let uuid1 = Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap();
        let uuid2 = Uuid::parse_str("6ba7b810-9dad-11d1-80b4-00c04fd430c8").unwrap();

        let point_id_1 = uuid_to_point_id(uuid1);
        let point_id_2 = uuid_to_point_id(uuid2);

        assert_ne!(
            point_id_1, point_id_2,
            "Different UUIDs should produce different point IDs"
        );
    }

    #[test]
    fn test_uuid_to_point_id_no_overflow() {
        let uuid = Uuid::new_v4();
        let _point_id = uuid_to_point_id(uuid);
        // Just ensure the operation doesn't panic or overflow
    }

    #[test]
    fn test_uuid_to_point_id_zero_uuid() {
        let uuid = Uuid::nil();
        let point_id = uuid_to_point_id(uuid);
        assert_eq!(point_id, 0, "Nil UUID should map to 0");
    }

    #[test]
    fn test_qdrant_value_to_json_string() {
        let value = qdrant_client::qdrant::Value {
            kind: Some(qdrant_client::qdrant::value::Kind::StringValue(
                "test".to_string(),
            )),
        };

        let json = qdrant_value_to_json(&value);
        assert_eq!(json, serde_json::json!("test"));
    }

    #[test]
    fn test_qdrant_value_to_json_integer() {
        let value = qdrant_client::qdrant::Value {
            kind: Some(qdrant_client::qdrant::value::Kind::IntegerValue(42)),
        };

        let json = qdrant_value_to_json(&value);
        assert_eq!(json, serde_json::json!(42));
    }

    #[test]
    fn test_qdrant_value_to_json_double() {
        let value = qdrant_client::qdrant::Value {
            kind: Some(qdrant_client::qdrant::value::Kind::DoubleValue(1.23)),
        };

        let json = qdrant_value_to_json(&value);
        assert_eq!(json, serde_json::json!(1.23));
    }

    #[test]
    fn test_qdrant_value_to_json_bool() {
        let value = qdrant_client::qdrant::Value {
            kind: Some(qdrant_client::qdrant::value::Kind::BoolValue(true)),
        };

        let json = qdrant_value_to_json(&value);
        assert_eq!(json, serde_json::json!(true));
    }

    #[test]
    fn test_qdrant_value_to_json_null() {
        let value = qdrant_client::qdrant::Value {
            kind: Some(qdrant_client::qdrant::value::Kind::NullValue(0)),
        };

        let json = qdrant_value_to_json(&value);
        assert_eq!(json, serde_json::json!(null));
    }

    #[test]
    fn test_qdrant_value_to_json_none() {
        let value = qdrant_client::qdrant::Value { kind: None };

        let json = qdrant_value_to_json(&value);
        assert_eq!(json, serde_json::json!(null));
    }

    #[test]
    fn test_qdrant_value_to_json_empty_list() {
        let value = qdrant_client::qdrant::Value {
            kind: Some(qdrant_client::qdrant::value::Kind::ListValue(
                qdrant_client::qdrant::ListValue { values: vec![] },
            )),
        };

        let json = qdrant_value_to_json(&value);
        assert_eq!(json, serde_json::json!([]));
    }

    #[test]
    fn test_qdrant_value_to_json_list_of_strings() {
        let values = vec![
            qdrant_client::qdrant::Value {
                kind: Some(qdrant_client::qdrant::value::Kind::StringValue(
                    "a".to_string(),
                )),
            },
            qdrant_client::qdrant::Value {
                kind: Some(qdrant_client::qdrant::value::Kind::StringValue(
                    "b".to_string(),
                )),
            },
        ];

        let list_value = qdrant_client::qdrant::Value {
            kind: Some(qdrant_client::qdrant::value::Kind::ListValue(
                qdrant_client::qdrant::ListValue { values },
            )),
        };

        let json = qdrant_value_to_json(&list_value);
        assert_eq!(json, serde_json::json!(["a", "b"]));
    }
}
