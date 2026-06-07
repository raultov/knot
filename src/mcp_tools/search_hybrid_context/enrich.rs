#[cfg(test)]
mod tests {
    use crate::cli_tools::search_hybrid_context::enrich_single_entity;
    use serde_json::json;

    #[test]
    fn test_enrich_single_entity_with_all_relationships() {
        let mut entity = json!({"name": "MyInterface"});
        let references = json!({
            "extends": [{"name": "Child1"}],
            "implements": [{"name": "Impl1"}, {"name": "Impl2"}],
            "references": [
                {"name": "ref1", "file_path": "ref1.java"},
                {"name": "ref2", "file_path": "ref2.java"}
            ],
            "calls": [{"name": "caller1", "file_path": "caller.java"}]
        });

        enrich_single_entity(&mut entity, &references);

        assert!(entity.get("subclasses").is_some());
        assert!(entity.get("implementers").is_some());
        assert!(entity.get("type_usage_count").is_some());
        assert!(entity.get("caller_count").is_some());
    }

    #[test]
    fn test_enrich_single_entity_ignores_empty_arrays() {
        let mut entity = json!({"name": "MyClass"});
        let references = json!({
            "extends": [],
            "implements": [],
            "references": [],
            "calls": []
        });

        enrich_single_entity(&mut entity, &references);

        assert!(entity.get("subclasses").is_none());
        assert!(entity.get("implementers").is_none());
        assert!(entity.get("type_usage_count").is_none());
        assert!(entity.get("caller_count").is_none());
    }
}
