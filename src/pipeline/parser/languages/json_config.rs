use crate::models::{EntityKind, ParsedEntity};
use crate::pipeline::parser::utils::truncate_string;

/// Maximum nesting depth for JSON tree walking.
const MAX_DEPTH: usize = 10;

pub(crate) fn extract_entities_json_config(
    source: &str,
    file_path: &str,
    repo_name: &str,
) -> Vec<ParsedEntity> {
    let mut entities = Vec::new();

    let value: serde_json::Value = match serde_json::from_str(source) {
        Ok(v) => v,
        Err(_) => return entities,
    };

    // Detect package.json by checking for "name" + dependency fields
    let is_package_json = value
        .as_object()
        .map(|obj| {
            obj.contains_key("name")
                && (obj.contains_key("dependencies")
                    || obj.contains_key("devDependencies")
                    || obj.contains_key("peerDependencies"))
        })
        .unwrap_or(false);

    if is_package_json {
        extract_package_json(&value, file_path, repo_name, &mut entities);
    } else {
        walk_json("", &value, 0, file_path, repo_name, &mut entities);
    }

    entities
}

fn extract_package_json(
    value: &serde_json::Value,
    file_path: &str,
    repo_name: &str,
    entities: &mut Vec<ParsedEntity>,
) {
    let obj = match value.as_object() {
        Some(o) => o,
        None => return,
    };

    // Emit ProjectIdentity
    let package_name = obj
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");
    let version = obj
        .get("version")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");

    entities.push(ParsedEntity::new(
        package_name,
        EntityKind::ProjectIdentity,
        format!("npm:{}", package_name),
        Some(format!("version: {}, build_system: npm", version)),
        Some(format!("npm project identity: {}", package_name)),
        "json",
        file_path,
        1,
        1,
        None,
        repo_name,
    ));

    // Extract dependencies as BuildDependency
    let dep_sections = [
        ("dependencies", "scope: compile"),
        ("devDependencies", "scope: dev"),
        ("peerDependencies", "scope: peer"),
    ];

    for (section, scope) in &dep_sections {
        if let Some(deps) = obj.get(*section).and_then(|v| v.as_object()) {
            for (dep_name, dep_version) in deps {
                let ver_str = dep_version.as_str().unwrap_or("unknown");
                let name = format!("npm:{}:{}", dep_name, ver_str);
                let fqn = format!("npm:{}:{}", dep_name, ver_str);

                entities.push(ParsedEntity::new(
                    &name,
                    EntityKind::BuildDependency,
                    &fqn,
                    Some(scope.to_string()),
                    Some(format!("npm dependency: {}", name)),
                    "json",
                    file_path,
                    1,
                    1,
                    None,
                    repo_name,
                ));
            }
        }
    }

    // Extract scripts as ConfigProperty
    if let Some(scripts) = obj.get("scripts").and_then(|v| v.as_object()) {
        for (script_name, script_cmd) in scripts {
            let cmd = script_cmd.as_str().unwrap_or("?");
            let name = script_name.as_str();
            let fqn = format!("{}:{}:scripts.{}", repo_name, file_path, script_name);

            let mut entity = ParsedEntity::new(
                name,
                EntityKind::ConfigProperty,
                fqn,
                Some(cmd.to_string()),
                Some(format!("npm script: {}", script_name)),
                "json",
                file_path,
                1,
                1,
                None,
                repo_name,
            );
            entity.embed_text = format!("scripts.{} = {}", script_name, cmd);
            entities.push(entity);
        }
    }

    // Walk remaining fields as ConfigProperty (skip already-handled top-level keys)
    let skip_keys: std::collections::HashSet<&str> = [
        "name",
        "version",
        "dependencies",
        "devDependencies",
        "peerDependencies",
        "scripts",
    ]
    .iter()
    .copied()
    .collect();

    for (key, val) in obj {
        if skip_keys.contains(key.as_str()) {
            continue;
        }
        walk_json(key, val, 0, file_path, repo_name, entities);
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "function is verbose but correct — extraction deferred"
)]
fn walk_json(
    prefix: &str,
    value: &serde_json::Value,
    depth: usize,
    file_path: &str,
    repo_name: &str,
    entities: &mut Vec<ParsedEntity>,
) {
    if depth > MAX_DEPTH {
        return;
    }

    match value {
        serde_json::Value::Object(map) => {
            for (key, val) in map {
                let new_prefix = if prefix.is_empty() {
                    key.clone()
                } else {
                    format!("{}.{}", prefix, key)
                };
                walk_json(&new_prefix, val, depth + 1, file_path, repo_name, entities);
            }
        }
        serde_json::Value::Array(arr) => {
            for (i, item) in arr.iter().enumerate() {
                let new_prefix = format!("{}[{}]", prefix, i);
                walk_json(&new_prefix, item, depth + 1, file_path, repo_name, entities);
            }
        }
        _ => {
            let val_str = json_value_to_string(value);
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
                "json",
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

fn json_value_to_string(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::Null => "null".to_string(),
        serde_json::Value::Bool(b) => b.to_string(),
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Array(_) | serde_json::Value::Object(_) => "[complex]".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_simple_config() {
        let source = r#"{"host": "localhost", "port": 8080, "debug": true}"#;
        let entities = extract_entities_json_config(source, "/app/config.json", "test-repo");

        let props: Vec<_> = entities
            .iter()
            .filter(|e| e.kind == EntityKind::ConfigProperty)
            .collect();
        assert_eq!(props.len(), 3);
        assert!(props.iter().any(|p| p.name == "host"));
        assert!(props.iter().any(|p| p.name == "port"));
        assert!(props.iter().any(|p| p.name == "debug"));
    }

    #[test]
    fn test_parse_nested_config() {
        let source = r#"{
  "database": {
    "host": "localhost",
    "port": 5432
  }
}"#;
        let entities = extract_entities_json_config(source, "/app/db.json", "test-repo");

        let props: Vec<_> = entities
            .iter()
            .filter(|e| e.kind == EntityKind::ConfigProperty)
            .collect();
        assert_eq!(props.len(), 2);
        assert!(props.iter().any(|p| p.fqn.contains("database.host")));
        assert!(props.iter().any(|p| p.fqn.contains("database.port")));
    }

    #[test]
    fn test_parse_package_json_dependencies() {
        let source = r#"{
  "name": "my-app",
  "version": "1.0.0",
  "dependencies": {
    "express": "^4.18.0",
    "lodash": "4.17.21"
  },
  "devDependencies": {
    "jest": "29.0.0"
  },
  "scripts": {
    "start": "node index.js",
    "test": "jest"
  }
}"#;
        let entities = extract_entities_json_config(source, "package.json", "test-repo");

        // BuildDependency entities (3: express, lodash, jest)
        let deps: Vec<_> = entities
            .iter()
            .filter(|e| e.kind == EntityKind::BuildDependency)
            .collect();
        assert_eq!(deps.len(), 3);
        assert!(deps.iter().any(|d| d.name.contains("express")));
        assert!(deps.iter().any(|d| d.name.contains("lodash")));
        assert!(deps.iter().any(|d| d.name.contains("jest")));

        // dev-dependency should have scope: dev
        let jest_dep = deps.iter().find(|d| d.name.contains("jest")).unwrap();
        assert!(jest_dep.signature.as_ref().unwrap().contains("scope: dev"));

        // ConfigProperty entities for scripts
        let props: Vec<_> = entities
            .iter()
            .filter(|e| e.kind == EntityKind::ConfigProperty)
            .collect();
        assert!(props.iter().any(|p| p.name == "start"));
        assert!(props.iter().any(|p| p.name == "test"));
    }

    #[test]
    fn test_parse_package_json_project_identity() {
        let source = r#"{
  "name": "@my-scope/my-app",
  "version": "2.0.0",
  "dependencies": {
    "express": "^4.0.0"
  }
}"#;
        let entities = extract_entities_json_config(source, "package.json", "test-repo");

        let identities: Vec<_> = entities
            .iter()
            .filter(|e| e.kind == EntityKind::ProjectIdentity)
            .collect();
        assert_eq!(identities.len(), 1);
        assert_eq!(identities[0].name, "@my-scope/my-app");
        assert_eq!(identities[0].fqn, "npm:@my-scope/my-app");
        assert!(
            identities[0]
                .signature
                .as_ref()
                .unwrap()
                .contains("build_system: npm")
        );
    }

    #[test]
    fn test_parse_tsconfig() {
        let source = r#"{
  "compilerOptions": {
    "target": "ES2022",
    "module": "NodeNext",
    "strict": true
  }
}"#;
        let entities = extract_entities_json_config(source, "/app/tsconfig.json", "test-repo");

        let props: Vec<_> = entities
            .iter()
            .filter(|e| e.kind == EntityKind::ConfigProperty)
            .collect();
        assert_eq!(props.len(), 3);
        let target = props.iter().find(|p| p.name == "target").unwrap();
        assert!(target.fqn.contains("compilerOptions.target"));
        assert_eq!(target.signature.as_ref().unwrap(), "ES2022");
    }

    #[test]
    fn test_parse_empty_json() {
        let source = "{}";
        let entities = extract_entities_json_config(source, "/app/empty.json", "test-repo");
        assert!(entities.is_empty());
    }
}
