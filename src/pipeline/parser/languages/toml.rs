use crate::models::{EntityKind, ParsedEntity};

#[expect(
    clippy::too_many_lines,
    reason = "function is verbose but correct — extraction deferred"
)]
pub(crate) fn extract_entities_toml(
    source: &str,
    file_path: &str,
    repo_name: &str,
) -> Vec<ParsedEntity> {
    let mut entities = Vec::new();

    let value: toml::Value = match toml::from_str(source) {
        Ok(v) => v,
        Err(_) => return entities,
    };

    let table = match value {
        toml::Value::Table(t) => t,
        _ => return entities,
    };

    // Extract [package]
    if let Some(package) = table.get("package").and_then(|v| v.as_table()) {
        let name = package
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        let version = package
            .get("version")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        let edition = package
            .get("edition")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");

        let fqn = format!("cargo:{}:{}", name, version);

        entities.push(ParsedEntity::new(
            name,
            EntityKind::CargoPackage,
            &fqn,
            Some(format!("version: {}, edition: {}", version, edition)),
            Some(format!("Cargo package: {}", name)),
            "toml",
            file_path,
            1,
            1,
            None,
            repo_name,
        ));

        // Emit ProjectIdentity entity for cross-repo linking
        entities.push(ParsedEntity::new(
            name,
            EntityKind::ProjectIdentity,
            format!("cargo:{}", name),
            Some(format!("version: {}, build_system: cargo", version)),
            Some(format!("Cargo project identity: {}", name)),
            "toml",
            file_path,
            1,
            1,
            None,
            repo_name,
        ));
    }

    // Extract dependencies from [dependencies], [dev-dependencies], [build-dependencies]
    let dep_sections = [
        ("dependencies", "scope: compile"),
        ("dev-dependencies", "scope: dev"),
        ("build-dependencies", "scope: build"),
    ];

    for (section, scope) in &dep_sections {
        if let Some(deps) = table.get(*section).and_then(|v| v.as_table()) {
            for (dep_name, dep_value) in deps {
                entities.extend(extract_dependency(
                    dep_name, dep_value, scope, file_path, repo_name,
                ));
            }
        }
    }

    // Extract [features]
    if let Some(features) = table.get("features").and_then(|v| v.as_table()) {
        for (feature_name, feature_value) in features {
            let enabled_features = match feature_value {
                toml::Value::Array(arr) => {
                    let names: Vec<String> = arr
                        .iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect();
                    Some(names.join(", "))
                }
                _ => None,
            };

            entities.push(ParsedEntity::new(
                feature_name.as_str(),
                EntityKind::CargoFeature,
                format!("cargo:crate:feature:{}", feature_name),
                enabled_features.map(|ef| format!("enables: [{}]", ef)),
                Some(format!("Cargo feature: {}", feature_name)),
                "toml",
                file_path,
                1,
                1,
                None,
                repo_name,
            ));
        }
    }

    // Extract [workspace] members
    if let Some(workspace) = table.get("workspace").and_then(|v| v.as_table())
        && let Some(members) = workspace.get("members").and_then(|v| v.as_array())
    {
        for member in members {
            if let Some(member_path) = member.as_str() {
                entities.push(ParsedEntity::new(
                    member_path,
                    EntityKind::WorkspaceMember,
                    format!("cargo:workspace:{}", member_path),
                    None,
                    Some(format!("Workspace member: {}", member_path)),
                    "toml",
                    file_path,
                    1,
                    1,
                    None,
                    repo_name,
                ));
            }
        }
    }

    entities
}

fn extract_dependency(
    dep_name: &str,
    dep_value: &toml::Value,
    scope: &str,
    file_path: &str,
    repo_name: &str,
) -> Vec<ParsedEntity> {
    let mut entities = Vec::new();
    let mut version = String::new();
    let mut extra_sig = Vec::new();

    match dep_value {
        // Simple: serde = "1.0"
        toml::Value::String(v) => {
            version = v.clone();
        }
        // Table: serde = { version = "1.0", features = ["derive"], git = "...", path = "..." }
        toml::Value::Table(t) => {
            if let Some(v) = t.get("version").and_then(|val| val.as_str()) {
                version = v.to_string();
            }
            if let Some(v) = t.get("git").and_then(|val| val.as_str()) {
                version = v.to_string();
                if let Some(branch) = t.get("branch").and_then(|val| val.as_str()) {
                    version.push_str(&format!(" (branch: {})", branch));
                }
            } else if let Some(v) = t.get("path").and_then(|val| val.as_str()) {
                version = format!("path: {}", v);
                extra_sig.push(format!("path: {}", v));
            }
            if let Some(features) = t.get("features").and_then(|val| val.as_array()) {
                let feature_list: Vec<String> = features
                    .iter()
                    .filter_map(|f| f.as_str().map(String::from))
                    .collect();
                if !feature_list.is_empty() {
                    extra_sig.push(format!("features: [{}]", feature_list.join(", ")));
                }
            }
            if version.is_empty() && !extra_sig.is_empty() {
                version = "unknown".to_string();
            }
        }
        _ => {
            version = "unknown".to_string();
        }
    }

    if version.is_empty() {
        version = "unknown".to_string();
    }

    let dep_version = version.clone();

    let name = format!("{}:{}", dep_name, dep_version);
    let fqn = format!("cargo:{}:{}", dep_name, version);

    let mut signature_parts = vec![scope.to_string()];
    signature_parts.extend(extra_sig);
    let signature = Some(signature_parts.join(", "));

    entities.push(ParsedEntity::new(
        &name,
        EntityKind::BuildDependency,
        &fqn,
        signature,
        Some(format!("Cargo dependency: {}", name)),
        "toml",
        file_path,
        1,
        1,
        None,
        repo_name,
    ));

    entities
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_single_simple_dependency() {
        let source = r#"
[package]
name = "my-crate"
version = "0.1.0"
edition = "2024"

[dependencies]
serde = "1.0"
"#;

        let entities = extract_entities_toml(source, "Cargo.toml", "test-repo");

        let deps: Vec<_> = entities
            .iter()
            .filter(|e| e.kind == EntityKind::BuildDependency)
            .collect();
        assert_eq!(deps.len(), 1);
        assert_eq!(deps[0].name, "serde:1.0");
        assert_eq!(deps[0].fqn, "cargo:serde:1.0");
        assert!(
            deps[0]
                .signature
                .as_ref()
                .unwrap()
                .contains("scope: compile")
        );
        assert!(
            deps[0]
                .docstring
                .as_ref()
                .unwrap()
                .contains("Cargo dependency")
        );
    }

    #[test]
    fn test_extract_table_dependency_with_features() {
        let source = r#"
[package]
name = "my-crate"
version = "0.1.0"
edition = "2024"

[dependencies]
serde = { version = "1.0", features = ["derive"] }
"#;

        let entities = extract_entities_toml(source, "Cargo.toml", "test-repo");

        let deps: Vec<_> = entities
            .iter()
            .filter(|e| e.kind == EntityKind::BuildDependency)
            .collect();
        assert_eq!(deps.len(), 1);
        assert_eq!(deps[0].name, "serde:1.0");
        assert!(
            deps[0]
                .signature
                .as_ref()
                .unwrap()
                .contains("features: [derive]")
        );
        assert!(
            deps[0]
                .signature
                .as_ref()
                .unwrap()
                .contains("scope: compile")
        );
    }

    #[test]
    fn test_extract_git_dependency() {
        let source = r#"
[package]
name = "my-crate"
version = "0.1.0"
edition = "2024"

[dependencies]
my-lib = { git = "https://github.com/user/my-lib", branch = "main" }
"#;

        let entities = extract_entities_toml(source, "Cargo.toml", "test-repo");

        let deps: Vec<_> = entities
            .iter()
            .filter(|e| e.kind == EntityKind::BuildDependency)
            .collect();
        assert_eq!(deps.len(), 1);
        assert!(deps[0].name.contains("my-lib:https://github.com"));
        assert!(deps[0].name.contains("(branch: main)"));
    }

    #[test]
    fn test_extract_path_dependency() {
        let source = r#"
[package]
name = "my-crate"
version = "0.1.0"
edition = "2024"

[dependencies]
my-lib = { path = "../my-lib" }
"#;

        let entities = extract_entities_toml(source, "Cargo.toml", "test-repo");

        let deps: Vec<_> = entities
            .iter()
            .filter(|e| e.kind == EntityKind::BuildDependency)
            .collect();
        assert_eq!(deps.len(), 1);
        assert!(deps[0].name.contains("my-lib:path: ../my-lib"));
        assert!(
            deps[0]
                .signature
                .as_ref()
                .unwrap()
                .contains("path: ../my-lib")
        );
    }

    #[test]
    fn test_extract_dev_and_build_dependencies() {
        let source = r#"
[package]
name = "my-crate"
version = "0.1.0"
edition = "2024"

[dependencies]
serde = "1.0"

[dev-dependencies]
criterion = "0.4"

[build-dependencies]
cc = "1.0"
"#;

        let entities = extract_entities_toml(source, "Cargo.toml", "test-repo");

        let deps: Vec<_> = entities
            .iter()
            .filter(|e| e.kind == EntityKind::BuildDependency)
            .collect();
        assert_eq!(deps.len(), 3);

        let compile_dep = deps.iter().find(|d| d.name == "serde:1.0").unwrap();
        assert!(
            compile_dep
                .signature
                .as_ref()
                .unwrap()
                .contains("scope: compile")
        );

        let dev_dep = deps.iter().find(|d| d.name == "criterion:0.4").unwrap();
        assert!(dev_dep.signature.as_ref().unwrap().contains("scope: dev"));

        let build_dep = deps.iter().find(|d| d.name == "cc:1.0").unwrap();
        assert!(
            build_dep
                .signature
                .as_ref()
                .unwrap()
                .contains("scope: build")
        );
    }

    #[test]
    fn test_extract_package_metadata() {
        let source = r#"
[package]
name = "my-awesome-crate"
version = "2.0.0"
edition = "2024"
description = "A test crate"
"#;

        let entities = extract_entities_toml(source, "Cargo.toml", "test-repo");

        let packages: Vec<_> = entities
            .iter()
            .filter(|e| e.kind == EntityKind::CargoPackage)
            .collect();
        assert_eq!(packages.len(), 1);
        assert_eq!(packages[0].name, "my-awesome-crate");
        assert_eq!(packages[0].fqn, "cargo:my-awesome-crate:2.0.0");
        assert!(
            packages[0]
                .signature
                .as_ref()
                .unwrap()
                .contains("version: 2.0.0")
        );
        assert!(
            packages[0]
                .signature
                .as_ref()
                .unwrap()
                .contains("edition: 2024")
        );

        let identities: Vec<_> = entities
            .iter()
            .filter(|e| e.kind == EntityKind::ProjectIdentity)
            .collect();
        assert_eq!(identities.len(), 1);
        assert_eq!(identities[0].name, "my-awesome-crate");
        assert_eq!(identities[0].fqn, "cargo:my-awesome-crate");
        assert!(
            identities[0]
                .signature
                .as_ref()
                .unwrap()
                .contains("build_system: cargo")
        );
    }

    #[test]
    fn test_extract_features_section() {
        let source = r#"
[package]
name = "my-crate"
version = "0.1.0"
edition = "2024"

[features]
default = ["std", "derive"]
serde = ["serde/std"]
"#;

        let entities = extract_entities_toml(source, "Cargo.toml", "test-repo");

        let features: Vec<_> = entities
            .iter()
            .filter(|e| e.kind == EntityKind::CargoFeature)
            .collect();
        assert_eq!(features.len(), 2);

        let default_feat = features.iter().find(|f| f.name == "default").unwrap();
        assert_eq!(default_feat.fqn, "cargo:crate:feature:default");
        assert!(
            default_feat
                .signature
                .as_ref()
                .unwrap()
                .contains("enables: [std, derive]")
        );

        let serde_feat = features.iter().find(|f| f.name == "serde").unwrap();
        assert_eq!(serde_feat.fqn, "cargo:crate:feature:serde");
    }

    #[test]
    fn test_extract_workspace_members() {
        let source = r#"
[package]
name = "my-workspace"
version = "0.1.0"
edition = "2024"

[workspace]
members = ["crate-a", "crate-b", "lib/crate-c"]
"#;

        let entities = extract_entities_toml(source, "Cargo.toml", "test-repo");

        let members: Vec<_> = entities
            .iter()
            .filter(|e| e.kind == EntityKind::WorkspaceMember)
            .collect();
        assert_eq!(members.len(), 3);
        assert_eq!(members[0].name, "crate-a");
        assert_eq!(members[0].fqn, "cargo:workspace:crate-a");
        assert_eq!(members[1].name, "crate-b");
        assert_eq!(members[2].name, "lib/crate-c");
    }
}
