use crate::models::{EntityKind, ParsedEntity, ReferenceIntent};

/// Maximum depth for values.yaml tree walking.
#[allow(dead_code)] // Used in tree walking depth checks
const MAX_DEPTH: usize = 10;

#[allow(dead_code)] // Reserved for future Chart.yaml parsing
pub(crate) fn extract_chart_yaml(
    source: &str,
    file_path: &str,
    repo_name: &str,
) -> Vec<ParsedEntity> {
    let mut entities = Vec::new();

    let yaml: serde_yaml::Value = match serde_yaml::from_str(source) {
        Ok(v) => v,
        Err(_) => return entities,
    };

    let chart_name = yaml
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");
    let version = yaml
        .get("version")
        .and_then(|v| v.as_str())
        .unwrap_or("0.0.0");
    let app_version = yaml
        .get("appVersion")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");
    let description = yaml.get("description").and_then(|v| v.as_str());
    let chart_type = yaml
        .get("type")
        .and_then(|v| v.as_str())
        .unwrap_or("application");

    let fqn = format!("helm:{}:{}", chart_name, version);

    let mut entity = ParsedEntity::new(
        chart_name,
        EntityKind::HelmChart,
        &fqn,
        Some(format!("appVersion: {}, type: {}", app_version, chart_type)),
        description.map(|d| d.to_string()),
        "helm",
        file_path,
        1,
        1,
        None,
        repo_name,
    );
    entity.embed_text = format!("Helm chart: {} v{}", chart_name, version);
    entities.push(entity);

    // Extract chart dependencies as BuildDependency
    if let Some(deps) = yaml.get("dependencies").and_then(|v| v.as_sequence()) {
        for dep in deps {
            if let (Some(dep_name), Some(dep_ver)) = (
                dep.get("name").and_then(|v| v.as_str()),
                dep.get("version").and_then(|v| v.as_str()),
            ) {
                let dep_fqn = format!("helm:{}:{}", dep_name, dep_ver);
                entities.push(ParsedEntity::new(
                    dep_name,
                    EntityKind::BuildDependency,
                    &dep_fqn,
                    Some(format!("scope: helm, version: {}", dep_ver)),
                    Some(format!("Helm chart dependency: {}:{}", dep_name, dep_ver)),
                    "helm",
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

#[allow(dead_code)] // Reserved for future values.yaml parsing
pub(crate) fn extract_values_yaml(
    source: &str,
    file_path: &str,
    repo_name: &str,
    chart_name: &str,
) -> Vec<ParsedEntity> {
    let mut entities = Vec::new();

    let value: serde_yaml::Value = match serde_yaml::from_str(source) {
        Ok(v) => v,
        Err(_) => return entities,
    };

    walk_helm_values(
        "",
        &value,
        0,
        file_path,
        repo_name,
        chart_name,
        &mut entities,
    );

    entities
}

#[allow(dead_code)] // Reserved for future values tree walking
fn walk_helm_values(
    prefix: &str,
    value: &serde_yaml::Value,
    depth: usize,
    file_path: &str,
    repo_name: &str,
    chart_name: &str,
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
                walk_helm_values(
                    &new_prefix,
                    val,
                    depth + 1,
                    file_path,
                    repo_name,
                    chart_name,
                    entities,
                );
            }
        }
        serde_yaml::Value::Sequence(seq) => {
            for (i, item) in seq.iter().enumerate() {
                let new_prefix = format!("{}[{}]", prefix, i);
                walk_helm_values(
                    &new_prefix,
                    item,
                    depth + 1,
                    file_path,
                    repo_name,
                    chart_name,
                    entities,
                );
            }
        }
        _ => {
            let val_str = helm_value_to_string(value);
            let name = prefix.rsplit('.').next().unwrap_or(prefix);
            let fqn = format!("helm:values.{}.{}", chart_name, prefix);
            let signature = truncate_string(&val_str, 200);

            let mut entity = ParsedEntity::new(
                name,
                EntityKind::HelmValue,
                &fqn,
                Some(signature),
                Some(format!("Helm value: {} = {}", prefix, val_str)),
                "helm",
                file_path,
                1,
                1,
                None,
                repo_name,
            );
            entity.embed_text = format!("{} = {}", prefix, val_str);
            entities.push(entity);
        }
    }
}

#[allow(dead_code)] // Reserved for future Helm value serialization
fn helm_value_to_string(value: &serde_yaml::Value) -> String {
    match value {
        serde_yaml::Value::Null => "null".to_string(),
        serde_yaml::Value::Bool(b) => b.to_string(),
        serde_yaml::Value::Number(n) => n.to_string(),
        serde_yaml::Value::String(s) => s.clone(),
        _ => "[complex]".to_string(),
    }
}

pub(crate) fn extract_helm_template(
    source: &str,
    file_path: &str,
    repo_name: &str,
    chart_name: &str,
) -> Vec<ParsedEntity> {
    let mut entities = Vec::new();
    let mut seen_vars: std::collections::HashSet<String> = std::collections::HashSet::new();

    // Find all Go template expressions {{ ... }}
    let mut i = 0;
    let bytes = source.as_bytes();
    while i + 1 < bytes.len() {
        if bytes[i] == b'{' && bytes[i + 1] == b'{' {
            let start = i + 2;
            // Skip optional dash or whitespace after opening braces
            let content_start = if bytes.get(start).copied() == Some(b'-') {
                start + 1
            } else {
                start
            };
            if let Some(end) = source[content_start..].find("}}") {
                let content = source[content_start..content_start + end].trim();
                extract_template_refs(
                    content,
                    file_path,
                    repo_name,
                    chart_name,
                    &mut entities,
                    &mut seen_vars,
                );
                i = content_start + end + 2;
            } else {
                i += 2;
            }
        } else {
            i += 1;
        }
    }

    entities
}

fn extract_template_refs(
    content: &str,
    file_path: &str,
    repo_name: &str,
    chart_name: &str,
    entities: &mut Vec<ParsedEntity>,
    seen_vars: &mut std::collections::HashSet<String>,
) {
    // .Values.X.Y
    if let Some(var_path) = content.strip_prefix(".Values.") {
        let var_path = var_path.trim();
        let segments: Vec<&str> = var_path
            .split('|')
            .next()
            .unwrap_or(var_path)
            .trim()
            .split('.')
            .collect();
        let full_path = if !segments.is_empty() {
            segments.join(".")
        } else {
            var_path.to_string()
        };
        let last_seg = segments.last().copied().unwrap_or(&full_path);
        let fqn = format!("helm:template:{}.Values.{}", chart_name, full_path);
        let dedup_key = format!(".Values.{}", full_path);

        if seen_vars.insert(dedup_key.clone()) {
            let mut entity = ParsedEntity::new(
                last_seg,
                EntityKind::HelmTemplateVar,
                &fqn,
                None,
                Some(format!("Helm template variable: {}", dedup_key)),
                "helm",
                file_path,
                1,
                1,
                None,
                repo_name,
            );
            entity.embed_text = dedup_key.clone();
            entity
                .reference_intents
                .push(ReferenceIntent::ValueReference {
                    value_name: last_seg.to_string(),
                    line: 1,
                });
            entities.push(entity);
        }
    } else if let Some(var_name) = content.strip_prefix(".Release.") {
        let var_name = var_name.trim();
        let var_name = var_name.split('|').next().unwrap_or(var_name).trim();
        let name = var_name.rsplit('.').next().unwrap_or(var_name);
        let fqn = format!("helm:template:{}.Release.{}", chart_name, var_name);
        let dedup_key = format!(".Release.{}", var_name);

        if seen_vars.insert(dedup_key) {
            let mut entity = ParsedEntity::new(
                name,
                EntityKind::HelmTemplateVar,
                &fqn,
                None,
                Some(format!("Helm template variable: .Release.{}", var_name)),
                "helm",
                file_path,
                1,
                1,
                None,
                repo_name,
            );
            entity.embed_text = format!(".Release.{}", var_name);
            entities.push(entity);
        }
    } else if let Some(var_name) = content.strip_prefix(".Chart.") {
        let var_name = var_name.trim();
        let var_name = var_name.split('|').next().unwrap_or(var_name).trim();
        let name = var_name.rsplit('.').next().unwrap_or(var_name);
        let fqn = format!("helm:template:{}.Chart.{}", chart_name, var_name);
        let dedup_key = format!(".Chart.{}", var_name);

        if seen_vars.insert(dedup_key) {
            let mut entity = ParsedEntity::new(
                name,
                EntityKind::HelmTemplateVar,
                &fqn,
                None,
                Some(format!("Helm template variable: .Chart.{}", var_name)),
                "helm",
                file_path,
                1,
                1,
                None,
                repo_name,
            );
            entity.embed_text = format!(".Chart.{}", var_name);
            entities.push(entity);
        }
    } else if let Some(include_name) = content
        .strip_prefix("include \"")
        .and_then(|s| s.split('"').next())
    {
        let dedup_key = format!("include:{}", include_name);
        if seen_vars.insert(dedup_key) {
            let mut entity = ParsedEntity::new(
                include_name,
                EntityKind::HelmTemplateVar,
                format!("helm:template:{}.include.{}", chart_name, include_name),
                None,
                Some(format!("Helm include: {}", include_name)),
                "helm",
                file_path,
                1,
                1,
                None,
                repo_name,
            );
            entity.embed_text = format!("include \"{}\"", include_name);
            entities.push(entity);
        }
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
    fn test_parse_chart_yaml() {
        let source = r#"
apiVersion: v2
name: my-app
version: 1.2.3
appVersion: 2.0.0
description: A test Helm chart
type: application
"#;
        let entities = extract_chart_yaml(source, "Chart.yaml", "test-repo");
        let charts: Vec<_> = entities
            .iter()
            .filter(|e| e.kind == EntityKind::HelmChart)
            .collect();
        assert_eq!(charts.len(), 1);
        assert_eq!(charts[0].name, "my-app");
        assert_eq!(charts[0].fqn, "helm:my-app:1.2.3");
        assert!(charts[0].signature.as_ref().unwrap().contains("2.0.0"));
        assert!(
            charts[0]
                .docstring
                .as_ref()
                .unwrap()
                .contains("test Helm chart")
        );
    }

    #[test]
    fn test_parse_chart_dependencies() {
        let source = r#"
apiVersion: v2
name: parent-chart
version: 1.0.0
dependencies:
  - name: redis
    version: 17.0.0
  - name: postgresql
    version: 12.0.0
"#;
        let entities = extract_chart_yaml(source, "Chart.yaml", "test-repo");
        let deps: Vec<_> = entities
            .iter()
            .filter(|e| e.kind == EntityKind::BuildDependency)
            .collect();
        assert_eq!(deps.len(), 2);
        assert!(deps.iter().any(|d| d.name == "redis"));
        assert!(deps.iter().any(|d| d.name == "postgresql"));
    }

    #[test]
    fn test_parse_values_yaml() {
        let source = r#"
replicaCount: 3
image:
  repository: nginx
  tag: "1.21"
service:
  type: ClusterIP
  port: 80
"#;
        let entities = extract_values_yaml(source, "values.yaml", "test-repo", "my-chart");
        let vals: Vec<_> = entities
            .iter()
            .filter(|e| e.kind == EntityKind::HelmValue)
            .collect();
        assert_eq!(vals.len(), 5);
        assert!(vals.iter().any(|v| v.name == "replicaCount"));
        assert!(vals.iter().any(|v| v.name == "repository"));
        assert!(vals.iter().any(|v| v.name == "tag"));
    }

    #[test]
    fn test_parse_template_variables() {
        let source = r#"
apiVersion: apps/v1
kind: Deployment
metadata:
  name: {{ .Values.appName }}
spec:
  replicas: {{ .Values.replicaCount }}
  template:
    spec:
      containers:
        - image: {{ .Values.image.repository }}:{{ .Values.image.tag }}
          env:
            - name: RELEASE
              value: {{ .Release.Name }}
"#;
        let entities =
            extract_helm_template(source, "templates/deployment.yaml", "test-repo", "my-chart");
        let vars: Vec<_> = entities
            .iter()
            .filter(|e| e.kind == EntityKind::HelmTemplateVar)
            .collect();
        // appName, replicaCount, repository, tag, Name
        assert!(vars.len() >= 4);
        assert!(vars.iter().any(|v| v.name == "appName"));
        assert!(vars.iter().any(|v| v.name == "replicaCount"));
        assert!(vars.iter().any(|v| v.name == "repository"));
        assert!(vars.iter().any(|v| v.name == "tag"));

        let release_var = vars.iter().find(|v| v.name == "Name");
        assert!(release_var.is_some());
    }

    #[test]
    fn test_parse_release_and_chart_references() {
        let source = r#"
metadata:
  labels:
    release: {{ .Release.Name }}
    chart: {{ .Chart.Version }}
"#;
        let entities =
            extract_helm_template(source, "templates/deploy.yaml", "test-repo", "my-chart");
        let vars: Vec<_> = entities
            .iter()
            .filter(|e| e.kind == EntityKind::HelmTemplateVar)
            .collect();
        assert!(vars.iter().any(|v| v.name == "Name"));
        assert!(vars.iter().any(|v| v.name == "Version"));
    }
}
