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

#[expect(
    clippy::cognitive_complexity,
    reason = "function is verbose but correct — extraction deferred"
)]
fn k8s_spec_summary(yaml: &serde_yaml::Value) -> Option<String> {
    let spec = yaml.get("spec")?;

    let mut parts = Vec::new();

    if let Some(replicas) = spec.get("replicas").and_then(|v| v.as_i64()) {
        parts.push(format!("replicas: {}", replicas));
    }

    if let Some(template) = spec.get("template").and_then(|t| t.get("spec"))
        && let Some(containers) = template.get("containers").and_then(|v| v.as_sequence())
    {
        for container in containers {
            if let Some(name) = container.get("name").and_then(|v| v.as_str())
                && let Some(image) = container.get("image").and_then(|v| v.as_str())
            {
                parts.push(format!("container: {} ({})", name, image));
            }
        }
    }

    if let Some(ports) = spec.get("ports").and_then(|v| v.as_sequence()) {
        for port in ports {
            if let (Some(p), Some(name)) = (
                port.get("port").and_then(|v| v.as_i64()),
                port.get("name").and_then(|v| v.as_str()),
            ) {
                parts.push(format!("port: {} ({})", p, name));
            } else if let Some(p) = port.get("port").and_then(|v| v.as_i64()) {
                parts.push(format!("port: {}", p));
            }
        }
    }

    if let Some(kind) = yaml.get("kind").and_then(|v| v.as_str()) {
        if kind == "Service"
            && let Some(svc_type) = spec.get("type").and_then(|v| v.as_str())
        {
            parts.push(format!("type: {}", svc_type));
        }
        if kind == "Ingress"
            && let Some(rules) = spec.get("rules").and_then(|v| v.as_sequence())
        {
            for rule in rules {
                if let Some(host) = rule.get("host").and_then(|v| v.as_str()) {
                    parts.push(format!("host: {}", host));
                }
            }
        }
    }

    if parts.is_empty() {
        None
    } else {
        Some(parts.join(", "))
    }
}

#[expect(
    clippy::cognitive_complexity,
    reason = "function is verbose but correct — extraction deferred"
)]
fn extract_k8s_references(yaml: &serde_yaml::Value, intents: &mut Vec<ReferenceIntent>) {
    // spec.template.spec.containers[].image -> ValueReference
    if let Some(template) = yaml
        .get("spec")
        .and_then(|s| s.get("template"))
        .and_then(|t| t.get("spec"))
    {
        if let Some(containers) = template.get("containers").and_then(|v| v.as_sequence()) {
            for container in containers {
                if let Some(image) = container.get("image").and_then(|v| v.as_str()) {
                    intents.push(ReferenceIntent::ValueReference {
                        value_name: image.to_string(),
                        line: 1,
                    });
                }
            }
        }

        // spec.template.spec.volumes[].configMap.name
        if let Some(volumes) = template.get("volumes").and_then(|v| v.as_sequence()) {
            for volume in volumes {
                if let Some(cm) = volume.get("configMap")
                    && let Some(cm_name) = cm.get("name").and_then(|v| v.as_str())
                {
                    intents.push(ReferenceIntent::ValueReference {
                        value_name: format!("configmap:{}", cm_name),
                        line: 1,
                    });
                }
                if let Some(secret) = volume.get("secret")
                    && let Some(secret_name) = secret.get("secretName").and_then(|v| v.as_str())
                {
                    intents.push(ReferenceIntent::ValueReference {
                        value_name: format!("secret:{}", secret_name),
                        line: 1,
                    });
                }
            }
        }
    }

    // spec.selector.matchLabels -> ValueReference
    if let Some(selector) = yaml.get("spec").and_then(|s| s.get("selector"))
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

    // spec.rules[].backend.service.name (Ingress)
    if let Some(rules) = yaml
        .get("spec")
        .and_then(|s| s.get("rules"))
        .and_then(|v| v.as_sequence())
    {
        for rule in rules {
            if let Some(http) = rule.get("http")
                && let Some(paths) = http.get("paths").and_then(|v| v.as_sequence())
            {
                for path in paths {
                    if let Some(backend) = path.get("backend")
                        && let Some(service) = backend.get("service")
                        && let Some(svc_name) = service.get("name").and_then(|v| v.as_str())
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
    }
}
