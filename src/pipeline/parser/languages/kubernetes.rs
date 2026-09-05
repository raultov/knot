use crate::models::{EntityKind, ParsedEntity, ReferenceIntent};

pub(crate) fn extract_entities_k8s(
    source: &str,
    file_path: &str,
    repo_name: &str,
) -> Vec<ParsedEntity> {
    let mut entities = Vec::new();

    for doc_str in source.split("\n---") {
        let trimmed = doc_str.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Ok(yaml) = serde_yaml::from_str::<serde_yaml::Value>(trimmed)
            && yaml.get("apiVersion").is_some()
            && yaml.get("kind").is_some()
            && let Some(kind_str) = yaml.get("kind").and_then(|v| v.as_str())
        {
            let entity_kind = map_k8s_kind(kind_str);
            if let Some(entity) =
                extract_single_k8s_resource(&yaml, file_path, repo_name, entity_kind)
            {
                entities.push(entity);
            }
        }
    }

    entities
}

fn map_k8s_kind(kind: &str) -> EntityKind {
    match kind {
        "Deployment" => EntityKind::K8sDeployment,
        "Service" => EntityKind::K8sService,
        "ConfigMap" => EntityKind::K8sConfigMap,
        "Secret" => EntityKind::K8sSecret,
        "Ingress" => EntityKind::K8sIngress,
        "Namespace" => EntityKind::K8sNamespace,
        _ => EntityKind::K8sResource,
    }
}

fn extract_single_k8s_resource(
    yaml: &serde_yaml::Value,
    file_path: &str,
    repo_name: &str,
    entity_kind: EntityKind,
) -> Option<ParsedEntity> {
    let kind_str = yaml.get("kind").and_then(|v| v.as_str()).unwrap_or("?");
    let metadata = yaml.get("metadata");

    let meta_name = metadata
        .and_then(|m| m.get("name"))
        .and_then(|v| v.as_str())
        .unwrap_or("unnamed");
    let namespace = metadata
        .and_then(|m| m.get("namespace"))
        .and_then(|v| v.as_str())
        .unwrap_or("default");
    let api_version = yaml
        .get("apiVersion")
        .and_then(|v| v.as_str())
        .unwrap_or("?");

    let name = format!("{} {}", kind_str, meta_name);
    let fqn = format!("k8s:{}/{}/{}", namespace, kind_str, meta_name);

    let mut signature = format!("apiVersion: {}", api_version);

    if let Some(labels) = metadata
        .and_then(|m| m.get("labels"))
        .and_then(|v| v.as_mapping())
    {
        let label_parts: Vec<String> = labels
            .iter()
            .filter_map(|(k, v)| Some(format!("{}={}", k.as_str()?, v.as_str().unwrap_or("?"))))
            .collect();
        if !label_parts.is_empty() {
            signature.push_str(&format!(", labels: [{}]", label_parts.join(", ")));
        }
    }

    let docstring = k8s_spec_summary(yaml);

    let inline_comments = metadata
        .and_then(|m| m.get("annotations"))
        .and_then(|v| v.as_mapping())
        .map(|annotations| {
            annotations
                .iter()
                .filter_map(|(k, v)| Some(format!("{}={}", k.as_str()?, v.as_str().unwrap_or("?"))))
                .collect()
        })
        .unwrap_or_default();

    let mut reference_intents = Vec::new();

    extract_k8s_references(yaml, &mut reference_intents);

    let mut entity = ParsedEntity::new(
        &name,
        entity_kind,
        &fqn,
        Some(signature),
        docstring,
        "yaml",
        file_path,
        1,
        1,
        None,
        repo_name,
    );
    entity.inline_comments = inline_comments;
    entity.reference_intents = reference_intents;
    Some(entity)
}

fn yaml_path<'a>(v: &'a serde_yaml::Value, path: &[&str]) -> Option<&'a serde_yaml::Value> {
    let mut curr = v;
    for key in path {
        curr = curr.get(*key)?;
    }
    Some(curr)
}

fn yaml_seq<'a>(v: &'a serde_yaml::Value, key: &str) -> &'a [serde_yaml::Value] {
    v.get(key)
        .and_then(|seq| seq.as_sequence())
        .map(|s| s.as_slice())
        .unwrap_or(&[])
}

fn yaml_str<'a>(v: &'a serde_yaml::Value, key: &str) -> Option<&'a str> {
    v.get(key).and_then(|val| val.as_str())
}

fn push_container_summaries(spec: &serde_yaml::Value, parts: &mut Vec<String>) {
    if let Some(template_spec) = yaml_path(spec, &["template", "spec"]) {
        for container in yaml_seq(template_spec, "containers") {
            if let (Some(name), Some(image)) =
                (yaml_str(container, "name"), yaml_str(container, "image"))
            {
                parts.push(format!("container: {} ({})", name, image));
            }
        }
    }
}

fn push_port_summaries(spec: &serde_yaml::Value, parts: &mut Vec<String>) {
    for port in yaml_seq(spec, "ports") {
        if let (Some(p), Some(name)) = (
            port.get("port").and_then(|v| v.as_i64()),
            yaml_str(port, "name"),
        ) {
            parts.push(format!("port: {} ({})", p, name));
        } else if let Some(p) = port.get("port").and_then(|v| v.as_i64()) {
            parts.push(format!("port: {}", p));
        }
    }
}

fn push_kind_specific_summary(
    yaml: &serde_yaml::Value,
    spec: &serde_yaml::Value,
    parts: &mut Vec<String>,
) {
    if let Some(kind) = yaml_str(yaml, "kind") {
        if kind == "Service"
            && let Some(svc_type) = yaml_str(spec, "type")
        {
            parts.push(format!("type: {}", svc_type));
        }
        if kind == "Ingress" {
            for rule in yaml_seq(spec, "rules") {
                if let Some(host) = yaml_str(rule, "host") {
                    parts.push(format!("host: {}", host));
                }
            }
        }
    }
}

fn k8s_spec_summary(yaml: &serde_yaml::Value) -> Option<String> {
    let spec = yaml.get("spec")?;
    let mut parts = Vec::new();

    if let Some(replicas) = spec.get("replicas").and_then(|v| v.as_i64()) {
        parts.push(format!("replicas: {}", replicas));
    }

    push_container_summaries(spec, &mut parts);
    push_port_summaries(spec, &mut parts);
    push_kind_specific_summary(yaml, spec, &mut parts);

    (!parts.is_empty()).then(|| parts.join(", "))
}

fn push_container_image_refs(
    template_spec: &serde_yaml::Value,
    intents: &mut Vec<ReferenceIntent>,
) {
    for container in yaml_seq(template_spec, "containers") {
        if let Some(image) = yaml_str(container, "image") {
            intents.push(ReferenceIntent::ValueReference {
                value_name: image.to_string(),
                line: 1,
            });
        }
    }
}

fn push_volume_refs(template_spec: &serde_yaml::Value, intents: &mut Vec<ReferenceIntent>) {
    for volume in yaml_seq(template_spec, "volumes") {
        if let Some(cm) = volume.get("configMap")
            && let Some(cm_name) = yaml_str(cm, "name")
        {
            intents.push(ReferenceIntent::ValueReference {
                value_name: format!("configmap:{}", cm_name),
                line: 1,
            });
        }
        if let Some(secret) = volume.get("secret")
            && let Some(secret_name) = yaml_str(secret, "secretName")
        {
            intents.push(ReferenceIntent::ValueReference {
                value_name: format!("secret:{}", secret_name),
                line: 1,
            });
        }
    }
}

fn push_selector_refs(yaml: &serde_yaml::Value, intents: &mut Vec<ReferenceIntent>) {
    if let Some(selector) = yaml_path(yaml, &["spec", "selector"])
        && let Some(match_labels) = selector.get("matchLabels").and_then(|v| v.as_mapping())
    {
        for (k, v) in match_labels {
            intents.push(ReferenceIntent::ValueReference {
                value_name: format!(
                    "{}={}",
                    k.as_str().unwrap_or("?"),
                    v.as_str().unwrap_or("?")
                ),
                line: 1,
            });
        }
    }
}

fn push_ingress_backend_refs(yaml: &serde_yaml::Value, intents: &mut Vec<ReferenceIntent>) {
    if let Some(spec) = yaml.get("spec") {
        for rule in yaml_seq(spec, "rules") {
            if let Some(http) = rule.get("http") {
                for path in yaml_seq(http, "paths") {
                    if let Some(backend) = path.get("backend")
                        && let Some(service) = backend.get("service")
                        && let Some(svc_name) = yaml_str(service, "name")
                    {
                        intents.push(ReferenceIntent::ValueReference {
                            value_name: svc_name.to_string(),
                            line: 1,
                        });
                    }
                }
            }
        }
    }
}

fn extract_k8s_references(yaml: &serde_yaml::Value, intents: &mut Vec<ReferenceIntent>) {
    if let Some(template_spec) = yaml_path(yaml, &["spec", "template", "spec"]) {
        push_container_image_refs(template_spec, intents);
        push_volume_refs(template_spec, intents);
    }

    push_selector_refs(yaml, intents);
    push_ingress_backend_refs(yaml, intents);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_deployment() {
        let source = r#"
apiVersion: apps/v1
kind: Deployment
metadata:
  name: nginx-deployment
  namespace: production
  labels:
    app: nginx
    tier: frontend
  annotations:
    description: Nginx web server
spec:
  replicas: 3
  selector:
    matchLabels:
      app: nginx
  template:
    spec:
      containers:
        - name: nginx
          image: nginx:1.21
"#;
        let entities = extract_entities_k8s(source, "/k8s/deploy.yml", "test-repo");
        let deploys: Vec<_> = entities
            .iter()
            .filter(|e| e.kind == EntityKind::K8sDeployment)
            .collect();
        assert_eq!(deploys.len(), 1);
        assert_eq!(deploys[0].name, "Deployment nginx-deployment");
        assert!(deploys[0].fqn.contains("production/Deployment/nginx"));
        assert!(deploys[0].signature.as_ref().unwrap().contains("apps/v1"));
        assert!(deploys[0].signature.as_ref().unwrap().contains("app=nginx"));
        assert!(
            deploys[0]
                .docstring
                .as_ref()
                .unwrap()
                .contains("replicas: 3")
        );
        assert!(
            deploys[0]
                .docstring
                .as_ref()
                .unwrap()
                .contains("nginx:1.21")
        );
        assert!(!deploys[0].inline_comments.is_empty());
    }

    #[test]
    fn test_parse_service() {
        let source = r#"
apiVersion: v1
kind: Service
metadata:
  name: backend-svc
  namespace: default
spec:
  type: ClusterIP
  ports:
    - port: 8080
      name: http
    - port: 8443
      name: https
  selector:
    app: backend
"#;
        let entities = extract_entities_k8s(source, "/k8s/svc.yml", "test-repo");
        let svcs: Vec<_> = entities
            .iter()
            .filter(|e| e.kind == EntityKind::K8sService)
            .collect();
        assert_eq!(svcs.len(), 1);
        assert_eq!(svcs[0].name, "Service backend-svc");
        assert!(svcs[0].docstring.as_ref().unwrap().contains("ClusterIP"));
        assert!(svcs[0].docstring.as_ref().unwrap().contains("8080"));
    }

    #[test]
    fn test_parse_configmap() {
        let source = r#"
apiVersion: v1
kind: ConfigMap
metadata:
  name: app-config
spec:
  data:
    APP_NAME: MyApp
"#;
        let entities = extract_entities_k8s(source, "/k8s/cm.yml", "test-repo");
        let cms: Vec<_> = entities
            .iter()
            .filter(|e| e.kind == EntityKind::K8sConfigMap)
            .collect();
        assert_eq!(cms.len(), 1);
        assert_eq!(cms[0].name, "ConfigMap app-config");
    }

    #[test]
    fn test_parse_ingress() {
        let source = r#"
apiVersion: networking.k8s.io/v1
kind: Ingress
metadata:
  name: api-ingress
spec:
  rules:
    - host: api.example.com
      http:
        paths:
          - path: /
            backend:
              service:
                name: backend-svc
                port:
                  number: 8080
"#;
        let entities = extract_entities_k8s(source, "/k8s/ingress.yml", "test-repo");
        let ingrs: Vec<_> = entities
            .iter()
            .filter(|e| e.kind == EntityKind::K8sIngress)
            .collect();
        assert_eq!(ingrs.len(), 1);
        assert_eq!(ingrs[0].name, "Ingress api-ingress");
        assert!(
            ingrs[0]
                .docstring
                .as_ref()
                .unwrap()
                .contains("api.example.com")
        );

        let refs = &ingrs[0].reference_intents;
        assert!(refs.iter().any(|r| {
            matches!(r, ReferenceIntent::ValueReference { value_name, .. } if value_name == "backend-svc")
        }));
    }

    #[test]
    fn test_parse_multi_resource_yaml() {
        let source = r#"
apiVersion: apps/v1
kind: Deployment
metadata:
  name: dep1
spec:
  replicas: 1
  template:
    spec:
      containers:
        - name: app
          image: app:v1
---
apiVersion: v1
kind: Service
metadata:
  name: svc1
spec:
  ports:
    - port: 80
      name: http
---
apiVersion: v1
kind: ConfigMap
metadata:
  name: cm1
"#;
        let entities = extract_entities_k8s(source, "/k8s/multi.yml", "test-repo");
        let kinds: Vec<_> = entities.iter().map(|e| &e.kind).collect();
        assert_eq!(kinds.len(), 3);
        assert!(kinds.contains(&&EntityKind::K8sDeployment));
        assert!(kinds.contains(&&EntityKind::K8sService));
        assert!(kinds.contains(&&EntityKind::K8sConfigMap));
    }

    #[test]
    fn test_parse_unknown_kind() {
        let source = r#"
apiVersion: batch/v1
kind: CronJob
metadata:
  name: daily-backup
spec:
  schedule: "0 0 * * *"
"#;
        let entities = extract_entities_k8s(source, "/k8s/cron.yml", "test-repo");
        assert_eq!(entities.len(), 1);
        assert_eq!(entities[0].kind, EntityKind::K8sResource);
        assert_eq!(entities[0].name, "CronJob daily-backup");
    }

    #[test]
    fn test_parse_references_between_resources() {
        let source = r#"
apiVersion: apps/v1
kind: Deployment
metadata:
  name: app-dep
spec:
  selector:
    matchLabels:
      app: myapp
  template:
    spec:
      containers:
        - name: app
          image: myapp:v2
      volumes:
        - name: config
          configMap:
            name: app-config
        - name: secrets
          secret:
            secretName: app-creds
"#;
        let entities = extract_entities_k8s(source, "/k8s/refs.yml", "test-repo");
        assert_eq!(entities.len(), 1);
        let refs = &entities[0].reference_intents;
        assert!(refs.iter().any(|r| {
            matches!(r, ReferenceIntent::ValueReference { value_name, .. } if value_name == "configmap:app-config")
        }));
        assert!(refs.iter().any(|r| {
            matches!(r, ReferenceIntent::ValueReference { value_name, .. } if value_name == "secret:app-creds")
        }));
        assert!(refs.iter().any(|r| {
            matches!(r, ReferenceIntent::ValueReference { value_name, .. } if value_name == "myapp:v2")
        }));
        assert!(refs.iter().any(|r| {
            matches!(r, ReferenceIntent::ValueReference { value_name, .. } if value_name == "app=myapp")
        }));
    }

    #[test]
    fn test_parse_unnamed_port() {
        let source = r#"
apiVersion: v1
kind: Service
metadata:
  name: unnamed-svc
spec:
  ports:
    - port: 8080
"#;
        let entities = extract_entities_k8s(source, "/k8s/unnamed.yml", "test-repo");
        assert_eq!(entities.len(), 1);
        assert_eq!(entities[0].docstring, Some("port: 8080".to_string()));
    }
}
