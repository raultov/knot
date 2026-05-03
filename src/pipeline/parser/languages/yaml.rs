use crate::models::{EntityKind, ParsedEntity};

/// Maximum nesting depth for YAML tree walking to prevent pathological files.
#[allow(dead_code)] // Used in tree walking depth checks
const MAX_DEPTH: usize = 10;

#[allow(dead_code)] // Reserved for future YAML parsing
pub(crate) fn extract_entities_yaml(
    source: &str,
    file_path: &str,
    repo_name: &str,
) -> Vec<ParsedEntity> {
    let mut entities = Vec::new();

    // Handle multi-document YAML (--- separated)
    for doc_str in source.split("\n---") {
        let trimmed = doc_str.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Ok(value) = serde_yaml::from_str::<serde_yaml::Value>(trimmed) {
            walk_yaml("", &value, 0, file_path, repo_name, &mut entities);
        }
    }

    entities
}

#[allow(dead_code)] // Reserved for future YAML tree walking
fn walk_yaml(
    prefix: &str,
    value: &serde_yaml::Value,
    depth: usize,
    file_path: &str,
    repo_name: &str,
    entities: &mut Vec<ParsedEntity>,
) {
    if depth > MAX_DEPTH {
        return;
    }

    match value {
        serde_yaml::Value::Mapping(map) => {
            for (key, val) in map {
                let key_str = key.as_str().unwrap_or("?");
                let new_prefix = if prefix.is_empty() {
                    key_str.to_string()
                } else {
                    format!("{}.{}", prefix, key_str)
                };
                walk_yaml(&new_prefix, val, depth + 1, file_path, repo_name, entities);
            }
        }
        serde_yaml::Value::Sequence(seq) => {
            for (i, item) in seq.iter().enumerate() {
                let new_prefix = format!("{}[{}]", prefix, i);
                walk_yaml(&new_prefix, item, depth + 1, file_path, repo_name, entities);
            }
        }
        _ => {
            let val_str = value_to_string(value);
            let name = prefix.rsplit('.').next().unwrap_or(prefix);
            let fqn = format!("{}:{}:{}", repo_name, file_path, prefix);
            let signature = truncate_string(&val_str, 200);
            let docstring = format!("Config property: {} = {}", prefix, val_str);
            let embed_text = format!("{} = {}", prefix, val_str);

            let mut entity = ParsedEntity::new(
                name,
                EntityKind::ConfigProperty,
                fqn,
                Some(signature),
                Some(docstring),
                "yaml",
                file_path,
                1,
                1,
                None,
                repo_name,
            );
            entity.embed_text = embed_text;
            entities.push(entity);
        }
    }
}

#[allow(dead_code)] // Reserved for future YAML value serialization
fn value_to_string(value: &serde_yaml::Value) -> String {
    match value {
        serde_yaml::Value::Null => "null".to_string(),
        serde_yaml::Value::Bool(b) => b.to_string(),
        serde_yaml::Value::Number(n) => n.to_string(),
        serde_yaml::Value::String(s) => s.clone(),
        serde_yaml::Value::Sequence(_)
        | serde_yaml::Value::Mapping(_)
        | serde_yaml::Value::Tagged(_) => "[complex]".to_string(),
    }
}

#[allow(dead_code)] // Reserved for future string truncation utility
fn truncate_string(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        format!("{}...", &s[..max_len.saturating_sub(3)])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_flat_keys() {
        let source = r#"
key1: value1
key2: 42
key3: true
"#;
        let entities = extract_entities_yaml(source, "/app/config.yml", "test-repo");
        let props: Vec<_> = entities
            .iter()
            .filter(|e| e.kind == EntityKind::ConfigProperty)
            .collect();
        assert_eq!(props.len(), 3);

        let names: Vec<&str> = props.iter().map(|p| p.name.as_str()).collect();
        assert!(names.contains(&"key1"));
        assert!(names.contains(&"key2"));
        assert!(names.contains(&"key3"));
    }

    #[test]
    fn test_parse_nested_keys() {
        let source = r#"
spring:
  datasource:
    url: jdbc:mysql://localhost:3306/db
    username: admin
"#;
        let entities = extract_entities_yaml(source, "/app/application.yml", "test-repo");

        let props: Vec<_> = entities
            .iter()
            .filter(|e| e.kind == EntityKind::ConfigProperty)
            .collect();
        assert_eq!(props.len(), 2);

        let url_entity = props.iter().find(|p| p.name == "url").unwrap();
        assert!(url_entity.fqn.contains("spring.datasource.url"));

        let user_entity = props.iter().find(|p| p.name == "username").unwrap();
        assert!(user_entity.signature.as_ref().unwrap() == "admin");
        assert!(user_entity.embed_text.contains("admin"));
    }

    #[test]
    fn test_parse_array_values() {
        let source = r#"
servers:
  - server1.example.com
  - server2.example.com
  - server3.example.com
"#;
        let entities = extract_entities_yaml(source, "/app/servers.yml", "test-repo");

        let props: Vec<_> = entities
            .iter()
            .filter(|e| e.kind == EntityKind::ConfigProperty)
            .collect();
        assert_eq!(props.len(), 3);
        assert!(props[0].fqn.contains("servers[0]"));
        assert!(props[1].fqn.contains("servers[1]"));
        assert!(props[2].fqn.contains("servers[2]"));
    }

    #[test]
    fn test_parse_depth_limit() {
        // Generate YAML nested 15 levels deep
        let mut yaml = String::new();
        for i in 0..15 {
            yaml.push_str(&format!("{}level{}:\n", "  ".repeat(i), i));
        }
        yaml.push_str(&"  ".repeat(15));
        yaml.push_str("leaf: too_deep\n");

        let entities = extract_entities_yaml(&yaml, "/app/deep.yml", "test-repo");

        let props: Vec<_> = entities
            .iter()
            .filter(|e| e.kind == EntityKind::ConfigProperty)
            .collect();
        // After depth 10, should stop walking deeper
        // Actually, depth 10 means we process at depth 10 and stop at 11+
        // The 10 levels + leaf = depth 11 which exceeds MAX_DEPTH (10)
        // So the leaf value at depth > 10 should NOT be extracted
        assert_eq!(
            props.len(),
            0,
            "Leaf beyond depth 10 should not be extracted"
        );
    }

    #[test]
    fn test_parse_multi_document_yaml() {
        let source = r#"
doc1_key: value1
---
doc2_key: value2
---
doc3_key: value3
"#;
        let entities = extract_entities_yaml(source, "/app/multi.yml", "test-repo");

        let props: Vec<_> = entities
            .iter()
            .filter(|e| e.kind == EntityKind::ConfigProperty)
            .collect();
        assert_eq!(props.len(), 3);
        assert!(props.iter().any(|p| p.fqn.contains("doc1_key")));
        assert!(props.iter().any(|p| p.fqn.contains("doc2_key")));
        assert!(props.iter().any(|p| p.fqn.contains("doc3_key")));
    }

    #[test]
    fn test_empty_yaml() {
        let source = "";
        let entities = extract_entities_yaml(source, "/app/empty.yml", "test-repo");
        assert!(entities.is_empty());
    }
}
