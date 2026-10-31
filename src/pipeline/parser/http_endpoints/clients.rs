//! HTTP calls made by declarative JVM clients (issue #46).
//!
//! Every routed method of an OpenFeign (`@FeignClient`), MicroProfile REST
//! Client (`@RegisterRestClient`) or Spring HTTP interface (`@GetExchange`,
//! `@HttpExchange`) client becomes an `HttpClientCall` entity named
//! `<METHOD> <path>`. The client's attributes are kept in the signature
//! until [`resolve_http_client_calls`] reads the base URL (`url`,
//! `baseUri`, configuration properties) once the whole batch is parsed:
//! the URL path becomes a prefix of the call path, and the service name
//! and URL host become the `service:` hints of the signature, which the
//! indexer later matches against the endpoints of every indexed repository.

use std::path::Path;

use serde::{Deserialize, Serialize};

use super::annotation::{Annotation, StringConstants, eval_property_strings, eval_strings};
use super::config::AppConfig;
use super::{
    ANY_METHOD, AnnotatedMethod, Route, annotated_methods, combine, normalize_path, routes,
};
use crate::models::{EntityKind, ParsedEntity};

/// Spring HTTP interface verb-specific exchanges.
const EXCHANGE_MAPPINGS: &[(&str, &str)] = &[
    ("GetExchange", "GET"),
    ("PostExchange", "POST"),
    ("PutExchange", "PUT"),
    ("DeleteExchange", "DELETE"),
    ("PatchExchange", "PATCH"),
];

/// Prefix of the signature of a client call that has not been resolved yet.
const RAW_PREFIX: &str = "client-raw:";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum ClientKind {
    Feign,
    MicroProfile,
    HttpInterface,
    /// `RestTemplate`, `WebClient`, `RestClient` call sites
    /// ([`super::imperative`]).
    Imperative,
}

/// A client's attributes as written in its annotations, or the base URL
/// of an imperative call.
#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct RawClient {
    kind: ClientKind,
    /// Feign `name` / `value`.
    service: Option<String>,
    /// Feign `contextId`.
    context_id: Option<String>,
    /// Feign `url`, MicroProfile `baseUri`, an absolute `@HttpExchange` URL.
    url: Option<String>,
    /// Feign `path`.
    path: Option<String>,
    /// MicroProfile `configKey`.
    config_key: Option<String>,
    /// FQN of the client interface.
    interface: Option<String>,
}

impl RawClient {
    /// An imperative call sent to `base_url` (`None`: unknown).
    pub(super) fn imperative(base_url: Option<String>) -> Self {
        Self {
            kind: ClientKind::Imperative,
            service: None,
            context_id: None,
            url: base_url,
            path: None,
            config_key: None,
            interface: None,
        }
    }

    /// The signature holding the client until it is resolved.
    fn raw_signature(&self) -> String {
        format!(
            "{RAW_PREFIX}{}",
            serde_json::to_string(self).unwrap_or_default()
        )
    }
}

/// Where a client sends its calls, once the configuration is read.
#[derive(Debug, Default, PartialEq, Eq)]
struct ClientTarget {
    /// Service names and URL hosts the client may address.
    services: Vec<String>,
    /// Path every call starts with: the base URL path, then Feign `path`.
    prefix: String,
}

/// The `HttpClientCall` entities of one file's client methods.
pub(crate) fn extract_http_client_calls(
    entities: &[ParsedEntity],
    constants: &StringConstants,
) -> Vec<ParsedEntity> {
    annotated_methods(entities)
        .flat_map(|annotated| {
            let Some((client, routes)) = client_routes(&annotated, constants) else {
                return Vec::new();
            };
            routes
                .iter()
                .map(|route| {
                    client_call_entity(
                        annotated.method,
                        annotated.owner,
                        route,
                        &client,
                        CallSite {
                            line: annotated.method.start_line,
                            via: None,
                        },
                    )
                })
                .collect()
        })
        .collect()
}

/// The client a method belongs to and the routes it calls; `None` for a
/// method of anything else.
fn client_routes(
    annotated: &AnnotatedMethod<'_>,
    constants: &StringConstants,
) -> Option<(RawClient, Vec<Route>)> {
    let class_annotation = |name: &str| annotated.class_annotations.iter().find(|a| a.name == name);
    let attribute = |annotation: &Annotation, keys: &[&str]| {
        annotation
            .arg(keys)
            .and_then(|expr| eval_property_strings(expr, constants).into_iter().next())
            .filter(|value| !value.is_empty())
    };
    let interface = annotated.owner.map(|owner| owner.fqn.clone());
    if let Some(feign) = class_annotation("FeignClient") {
        let client = RawClient {
            kind: ClientKind::Feign,
            service: attribute(feign, &["value", "name"]),
            context_id: attribute(feign, &["contextId"]),
            url: attribute(feign, &["url"]),
            path: attribute(feign, &["path"]),
            config_key: None,
            interface,
        };
        let routes = routes(
            &annotated.class_annotations,
            &annotated.method_annotations,
            constants,
        );
        return Some((client, routes));
    }
    if let Some(rest_client) = class_annotation("RegisterRestClient") {
        let client = RawClient {
            kind: ClientKind::MicroProfile,
            service: None,
            context_id: None,
            url: attribute(rest_client, &["baseUri"]),
            path: None,
            config_key: attribute(rest_client, &["configKey"]),
            interface,
        };
        let routes = routes(
            &annotated.class_annotations,
            &annotated.method_annotations,
            constants,
        );
        return Some((client, routes));
    }
    let exchange = class_annotation("HttpExchange").and_then(|a| attribute(a, &["value", "url"]));
    let (url, base) = match exchange {
        Some(value) if is_url(&value) => (Some(value), String::new()),
        Some(value) => (None, value),
        None => (None, String::new()),
    };
    let routes = exchange_routes(&annotated.method_annotations, &base, constants);
    if routes.is_empty() {
        return None;
    }
    let client = RawClient {
        kind: ClientKind::HttpInterface,
        service: None,
        context_id: None,
        url,
        path: None,
        config_key: None,
        interface,
    };
    Some((client, routes))
}

/// An absolute URL (`http://host/x`) or a placeholder for one, as opposed
/// to a path.
fn is_url(value: &str) -> bool {
    value.contains("://") || value.starts_with("${")
}

/// Spring HTTP interface exchanges of a method, under the type-level path.
fn exchange_routes(
    method_annotations: &[Annotation],
    base: &str,
    constants: &StringConstants,
) -> Vec<Route> {
    let bases = [base.to_string()];
    let mut routes = Vec::new();
    for annotation in method_annotations {
        let verbs: Vec<String> = match EXCHANGE_MAPPINGS
            .iter()
            .find(|(name, _)| *name == annotation.name)
        {
            Some((_, verb)) => vec![verb.to_string()],
            None if annotation.name == "HttpExchange" => annotation
                .arg(&["method"])
                .map(|expr| eval_strings(expr, constants))
                .unwrap_or_default()
                .into_iter()
                .map(|verb| verb.to_uppercase())
                .filter(|verb| !verb.is_empty())
                .collect(),
            None => continue,
        };
        let verbs = if verbs.is_empty() {
            vec![ANY_METHOD.to_string()]
        } else {
            verbs
        };
        let subs = annotation
            .arg(&["value", "url"])
            .map(|expr| eval_strings(expr, constants))
            .unwrap_or_else(|| vec![String::new()]);
        let verbs: Vec<&str> = verbs.iter().map(String::as_str).collect();
        routes.extend(combine(&verbs, &bases, &subs));
    }
    routes
}

/// Where in its method a call is made.
pub(super) struct CallSite<'a> {
    pub line: usize,
    /// The client API of an imperative call (`RestTemplate.getForObject`);
    /// `None` for a declarative client method.
    pub via: Option<&'a str>,
}

pub(super) fn client_call_entity(
    method: &ParsedEntity,
    owner: Option<&ParsedEntity>,
    route: &Route,
    client: &RawClient,
    site: CallSite<'_>,
) -> ParsedEntity {
    let name = format!("{} {}", route.verb, route.path);
    let owner_name = owner.map_or_else(
        || method.enclosing_class.clone().unwrap_or_default(),
        |t| t.name.clone(),
    );
    let method_label = if owner_name.is_empty() {
        method.name.clone()
    } else {
        format!("{owner_name}.{}", method.name)
    };
    let mut docstring = match site.via {
        Some(via) => format!("HTTP call {name}, made by {method_label} through {via}."),
        None => format!("HTTP call {name}, made by the client method {method_label}."),
    };
    if let Some(doc) = &method.docstring {
        docstring.push_str("\n\n");
        docstring.push_str(doc);
    }
    let mut call = ParsedEntity::new(
        &name,
        EntityKind::HttpClientCall,
        format!("{}[client {name}]", method.fqn),
        Some(client.raw_signature()),
        Some(docstring),
        method.language.clone(),
        method.file_path.clone(),
        site.line,
        site.line,
        method.enclosing_class.clone(),
        method.repo_name.clone(),
    );
    call.enclosing_class_fqn = owner.map(|t| t.fqn.clone());
    call
}

/// Apply the base URL of every client call of the batch, read from its
/// annotations or the configuration of the repository rooted at
/// `repo_root`, and record the services it may address.
pub(crate) fn resolve_http_client_calls(entities: &mut [ParsedEntity], repo_root: &Path) {
    if !entities.iter().any(is_raw_client_call) {
        return;
    }
    let config = AppConfig::load(repo_root);
    for entity in entities.iter_mut().filter(|e| is_raw_client_call(e)) {
        let client = entity
            .signature
            .as_deref()
            .and_then(|s| s.strip_prefix(RAW_PREFIX))
            .and_then(|json| serde_json::from_str::<RawClient>(json).ok());
        let target = client.map(|c| c.target(&config)).unwrap_or_default();
        *entity = resolved_call(entity, &target);
    }
}

fn is_raw_client_call(entity: &ParsedEntity) -> bool {
    entity.kind == EntityKind::HttpClientCall
        && entity
            .signature
            .as_deref()
            .is_some_and(|s| s.starts_with(RAW_PREFIX))
}

/// `call` with the target's prefix applied to its name, FQN and docstring,
/// and its services as signature.
fn resolved_call(call: &ParsedEntity, target: &ClientTarget) -> ParsedEntity {
    let (verb, path) = call.name.split_once(' ').unwrap_or((ANY_METHOD, "/"));
    let name = format!(
        "{verb} {}",
        normalize_path(&format!("{}/{path}", target.prefix))
    );
    let method_fqn = call
        .fqn
        .strip_suffix(&format!("[client {}]", call.name))
        .unwrap_or(&call.fqn);
    let signature =
        (!target.services.is_empty()).then(|| format!("service: {}", target.services.join(" | ")));
    let docstring = call
        .docstring
        .as_ref()
        .map(|doc| doc.replacen(&call.name, &name, 1));
    let mut resolved = ParsedEntity::new(
        &name,
        EntityKind::HttpClientCall,
        format!("{method_fqn}[client {name}]"),
        signature,
        docstring,
        call.language.clone(),
        call.file_path.clone(),
        call.start_line,
        call.end_line,
        call.enclosing_class.clone(),
        call.repo_name.clone(),
    );
    resolved.enclosing_class_fqn = call.enclosing_class_fqn.clone();
    resolved.import_scope = call.import_scope.clone();
    resolved
}

impl RawClient {
    fn target(&self, config: &AppConfig) -> ClientTarget {
        let resolve = |value: &Option<String>| {
            value
                .as_deref()
                .map(|v| config.resolve(v))
                .filter(|v| !v.is_empty() && !v.contains("${"))
        };
        let service = resolve(&self.service).filter(|s| !is_url(s));
        let url = resolve(&self.url).or_else(|| {
            self.url_keys(config)
                .iter()
                .find_map(|key| config.get(key))
                .filter(|v| !v.is_empty() && !v.contains("${"))
        });
        let (host, url_path) = url.as_deref().map(split_url).unwrap_or_default();
        let mut services: Vec<String> = service.into_iter().chain(host).collect();
        services.dedup();
        let prefix = format!("{url_path}/{}", resolve(&self.path).unwrap_or_default());
        ClientTarget { services, prefix }
    }

    /// Configuration keys holding the base URL when the annotation has none.
    fn url_keys(&self, config: &AppConfig) -> Vec<String> {
        let names = |values: [&Option<String>; 2]| -> Vec<String> {
            values
                .into_iter()
                .flatten()
                .map(|v| config.resolve(v))
                .filter(|v| !v.is_empty() && !v.contains("${"))
                .collect()
        };
        match self.kind {
            ClientKind::Feign => names([&self.context_id, &self.service])
                .iter()
                .flat_map(|name| {
                    [
                        format!("spring.cloud.openfeign.client.config.{name}.url"),
                        format!("feign.client.config.{name}.url"),
                    ]
                })
                .collect(),
            ClientKind::MicroProfile => names([&self.config_key, &self.interface])
                .iter()
                .flat_map(|key| {
                    [
                        format!("quarkus.rest-client.{key}.url"),
                        format!("quarkus.rest-client.{key}.uri"),
                        format!("{key}/mp-rest/url"),
                        format!("{key}/mp-rest/uri"),
                    ]
                })
                .collect(),
            ClientKind::HttpInterface | ClientKind::Imperative => Vec::new(),
        }
    }
}

/// `http://user@host:8080/api/v1?q` → (`host`, `/api/v1`).
fn split_url(url: &str) -> (Option<String>, String) {
    let rest = url.split_once("://").map_or(url, |(_, rest)| rest);
    let rest = rest.split(['?', '#']).next().unwrap_or_default();
    let (authority, path) = rest.split_once('/').unwrap_or((rest, ""));
    let host = authority.rsplit('@').next().unwrap_or_default();
    let host = host.split(':').next().unwrap_or_default();
    let host = (!host.is_empty()).then(|| host.to_lowercase());
    (host, format!("/{path}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn method(name: &str, decorators: &[&str]) -> ParsedEntity {
        let mut entity = ParsedEntity::new(
            name,
            EntityKind::Method,
            format!("com.example.Client.{name}"),
            None,
            None,
            "java",
            "src/main/java/com/example/Client.java",
            10,
            12,
            Some("Client".to_string()),
            "repo",
        );
        entity.decorators = decorators.iter().map(|d| d.to_string()).collect();
        entity
    }

    fn interface(decorators: &[&str]) -> ParsedEntity {
        let mut entity = ParsedEntity::new(
            "Client",
            EntityKind::Interface,
            "com.example.Client",
            None,
            None,
            "java",
            "src/main/java/com/example/Client.java",
            3,
            20,
            None,
            "repo",
        );
        entity.decorators = decorators.iter().map(|d| d.to_string()).collect();
        entity
    }

    fn calls(class: &[&str], methods: &[(&str, &[&str])]) -> Vec<ParsedEntity> {
        let mut entities = vec![interface(class)];
        entities.extend(methods.iter().map(|(name, d)| method(name, d)));
        extract_http_client_calls(&entities, &StringConstants::new())
    }

    fn resolve(mut calls: Vec<ParsedEntity>, config: &str) -> Vec<(String, Option<String>)> {
        let root = tempfile::tempdir().expect("tempdir");
        let resources = root.path().join("src/main/resources");
        std::fs::create_dir_all(&resources).expect("mkdir");
        std::fs::write(resources.join("application.yml"), config).expect("write");
        resolve_http_client_calls(&mut calls, root.path());
        calls.into_iter().map(|c| (c.name, c.signature)).collect()
    }

    #[test]
    fn feign_methods_become_client_calls_under_the_client_path() {
        let found = calls(
            &[r#"@FeignClient(name = "items-service", path = "/api/items")"#],
            &[
                ("fetch", &[r#"@GetMapping("/{id}")"#]),
                ("helper", &["@Deprecated"]),
            ],
        );
        let resolved = resolve(found, "");
        assert_eq!(
            resolved,
            vec![(
                "GET /api/items/{id}".to_string(),
                Some("service: items-service".to_string())
            )]
        );
    }

    #[test]
    fn feign_url_placeholder_is_read_from_the_configuration() {
        let found = calls(
            &[r#"@FeignClient(name = "billing", url = "${billing.url}")"#],
            &[("create", &[r#"@PostMapping("/invoices")"#])],
        );
        let resolved = resolve(found, "billing:\n  url: http://localhost:9090/billing\n");
        assert_eq!(
            resolved,
            vec![(
                "POST /billing/invoices".to_string(),
                Some("service: billing | localhost".to_string())
            )]
        );
    }

    #[test]
    fn feign_url_falls_back_to_the_client_configuration() {
        let found = calls(
            &[r#"@FeignClient("stock")"#],
            &[("get", &[r#"@GetMapping("/levels")"#])],
        );
        let resolved = resolve(
            found,
            "spring:\n  cloud:\n    openfeign:\n      client:\n        config:\n          stock:\n            url: http://stock-api/v2\n",
        );
        assert_eq!(resolved[0].0, "GET /v2/levels");
        assert_eq!(resolved[0].1.as_deref(), Some("service: stock | stock-api"));
    }

    #[test]
    fn unresolved_url_adds_no_prefix() {
        let found = calls(
            &[r#"@FeignClient(name = "x", url = "${missing.url}")"#],
            &[("get", &[r#"@GetMapping("/a")"#])],
        );
        assert_eq!(
            resolve(found, ""),
            vec![("GET /a".to_string(), Some("service: x".to_string()))]
        );
    }

    #[test]
    fn microprofile_base_url_comes_from_the_config_key() {
        let found = calls(
            &[
                r#"@RegisterRestClient(configKey = "items")"#,
                r#"@Path("/items")"#,
            ],
            &[
                ("create", &["@POST"]),
                ("read", &["@GET", r#"@Path("/{id}")"#]),
            ],
        );
        let resolved = resolve(
            found,
            "quarkus:\n  rest-client:\n    items:\n      url: http://items-service:8080/api\n",
        );
        let names: Vec<&str> = resolved.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, vec!["POST /api/items", "GET /api/items/{id}"]);
        assert_eq!(resolved[0].1.as_deref(), Some("service: items-service"));
    }

    #[test]
    fn microprofile_mp_rest_key_uses_the_interface_name() {
        let mut found = calls(
            &["@RegisterRestClient", r#"@Path("/orders")"#],
            &[("list", &["@GET"])],
        );
        let root = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            root.path().join("microprofile-config.properties"),
            "com.example.Client/mp-rest/url=http://orders/v1\n",
        )
        .expect("write");
        resolve_http_client_calls(&mut found, root.path());
        assert_eq!(found[0].name, "GET /v1/orders");
    }

    #[test]
    fn http_interface_exchanges_are_client_calls() {
        let found = calls(
            &[r#"@HttpExchange("/health")"#],
            &[
                ("health", &["@GetExchange"]),
                ("put", &[r#"@HttpExchange(url = "/{id}", method = "PUT")"#]),
            ],
        );
        let resolved = resolve(found, "");
        assert_eq!(
            resolved,
            vec![
                ("GET /health".to_string(), None),
                ("PUT /health/{id}".to_string(), None)
            ]
        );
    }

    #[test]
    fn http_interface_without_type_annotation_and_absolute_url() {
        assert_eq!(
            calls(&[], &[("a", &[r#"@GetExchange("/a")"#])])[0].name,
            "GET /a"
        );
        let found = calls(
            &[r#"@HttpExchange("http://catalog:8080/v1")"#],
            &[("list", &[r#"@GetExchange("/products")"#])],
        );
        assert_eq!(
            resolve(found, ""),
            vec![(
                "GET /v1/products".to_string(),
                Some("service: catalog".to_string())
            )]
        );
    }

    #[test]
    fn controllers_make_no_client_calls() {
        assert!(calls(&["@RestController"], &[("a", &[r#"@GetMapping("/a")"#])]).is_empty());
    }

    #[test]
    fn client_call_keeps_the_method_fqn() {
        let mut found = calls(
            &[r#"@FeignClient(name = "s", path = "/p")"#],
            &[("get", &[r#"@GetMapping("/a")"#])],
        );
        resolve_http_client_calls(&mut found, Path::new("/nonexistent-knot-root"));
        assert_eq!(found[0].fqn, "com.example.Client.get[client GET /p/a]");
        assert!(found[0].docstring.as_deref().is_some_and(|d| {
            d.starts_with("HTTP call GET /p/a, made by the client method Client.get.")
        }));
    }

    #[test]
    fn urls_split_into_host_and_path() {
        assert_eq!(
            split_url("http://user@Items:8080/api/v1?q=1"),
            (Some("items".to_string()), "/api/v1".to_string())
        );
        assert_eq!(
            split_url("lb://items"),
            (Some("items".to_string()), "/".to_string())
        );
        assert_eq!(
            split_url("localhost:9090/billing"),
            (Some("localhost".to_string()), "/billing".to_string())
        );
    }
}
