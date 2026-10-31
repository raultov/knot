//! `HTTP_CALLS` links from declarative HTTP client calls to the endpoints
//! they hit, across every indexed repository (issue #46).
//!
//! After each run the indexed repository's HTTP facts (service names,
//! context paths) are recorded on its `:Repository` node, then every link
//! of the graph is recomputed, so the result does not depend on the order
//! repositories are indexed in. The pass is one transaction serialized
//! across concurrent indexers ([`HttpLinkExt::relink_http_calls`]). A call is linked when exactly one endpoint
//! matches it best:
//!
//! - the verbs are equal, or either side is `ANY`;
//! - the paths have the same segments, a `{var}` matching any segment;
//!   the endpoint path may be preceded by one of its context paths;
//! - endpoints of the repositories whose service names the client
//!   addresses (Feign `name`, URL host) are the only candidates, when any
//!   repository is addressed;
//! - at least one literal segment is matched exactly, so `/{a}/b` never
//!   links to `/items/{id}` on wildcards alone;
//! - the most literal match wins (`/{id}` ↔ `/{id}` over `/{id}` ↔
//!   `/export`), as a server routes it; a tie links nothing.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use anyhow::Result;
use tracing::info;

use crate::config::Config;
use crate::db::graph::{
    GraphDb, HttpClientCallRow, HttpEndpointRow, HttpLinkExt, HttpLinkInputs, RepoHttpFacts,
};
use crate::pipeline::parser::http_endpoints::{AppConfig, application_paths, normalize_path};

/// Verb of a route that accepts, or a call that may use, any HTTP method.
const ANY_METHOD: &str = "ANY";

/// Properties naming the service a repository runs as.
const SERVICE_NAME_KEYS: &[&str] = &["spring.application.name", "quarkus.application.name"];

/// Properties holding a path every endpoint is served under.
const CONTEXT_PATH_KEYS: &[&str] = &[
    "server.servlet.context-path",
    "server.context-path",
    "spring.mvc.servlet.path",
    "spring.webflux.base-path",
    "quarkus.http.root-path",
    "quarkus.rest.path",
    "quarkus.resteasy.path",
    "quarkus.resteasy-reactive.path",
];

/// Prefix of the services a client call's signature lists.
const SERVICE_PREFIX: &str = "service: ";

/// Record the HTTP facts of `cfg`'s repository, then recompute every
/// `HTTP_CALLS` edge of the graph.
pub async fn link_http_calls(graph_db: &GraphDb, cfg: &Config) -> Result<()> {
    let config = AppConfig::load(Path::new(&cfg.repo_path));
    let decorators = graph_db.application_path_decorators(&cfg.repo_name).await?;
    let facts = repo_http_facts(&cfg.repo_name, &config, &decorators);
    let summary = graph_db.relink_http_calls(&facts, match_http_calls).await?;
    if summary.calls > 0 {
        info!(
            "Linked {} of {} HTTP client call(s) to endpoints",
            summary.links, summary.calls
        );
    }
    Ok(())
}

/// Service names and context paths of a repository, from its
/// configuration and its `@ApplicationPath` types' decorators.
fn repo_http_facts(
    repo_name: &str,
    config: &AppConfig,
    application_path_decorators: &[Vec<String>],
) -> RepoHttpFacts {
    let usable = |value: &String| !value.is_empty() && !value.contains("${");
    let mut service_names: Vec<String> = SERVICE_NAME_KEYS
        .iter()
        .flat_map(|key| config.all(key))
        .filter(usable)
        .collect();
    service_names.sort();
    service_names.dedup();

    let mut context_paths: Vec<String> = CONTEXT_PATH_KEYS
        .iter()
        .flat_map(|key| config.all(key))
        .collect();
    for context in config.all("server.servlet.context-path") {
        for servlet in config.all("spring.mvc.servlet.path") {
            context_paths.push(format!("{context}/{servlet}"));
        }
    }
    context_paths.extend(
        application_path_decorators
            .iter()
            .flat_map(|decorators| application_paths(decorators)),
    );
    let mut context_paths: Vec<String> = context_paths
        .iter()
        .filter(|path| usable(path))
        .map(|path| normalize_path(path))
        .filter(|path| path != "/")
        .collect();
    context_paths.sort();
    context_paths.dedup();

    RepoHttpFacts {
        repo_name: repo_name.to_string(),
        service_names,
        context_paths,
    }
}

/// (client call UUID, endpoint UUID) of every call one endpoint matches
/// best.
fn match_http_calls(inputs: &HttpLinkInputs) -> Vec<(String, String)> {
    let facts: HashMap<&str, &RepoHttpFacts> = inputs
        .repos
        .iter()
        .map(|f| (f.repo_name.as_str(), f))
        .collect();
    inputs
        .calls
        .iter()
        .filter_map(|call| {
            best_endpoint(call, &inputs.endpoints, &facts)
                .map(|endpoint| (call.uuid.clone(), endpoint.uuid.clone()))
        })
        .collect()
}

/// Exact path segments ([`PathScore`]), then whether the verb is exact.
type Score = (PathScore, bool);

/// Literal segments matched exactly, then variables matched by variables.
type PathScore = (usize, usize);

fn best_endpoint<'a>(
    call: &HttpClientCallRow,
    endpoints: &'a [HttpEndpointRow],
    facts: &HashMap<&str, &RepoHttpFacts>,
) -> Option<&'a HttpEndpointRow> {
    let (verb, path) = call.name.split_once(' ')?;
    let addressed = addressed_repos(call, facts);
    let mut best: Option<(Score, &HttpEndpointRow)> = None;
    let mut tied = false;
    for endpoint in endpoints {
        if !addressed.is_empty() && !addressed.contains(endpoint.repo_name.as_str()) {
            continue;
        }
        let contexts = facts
            .get(endpoint.repo_name.as_str())
            .map_or(&[][..], |f| f.context_paths.as_slice());
        let Some(score) = endpoint_score(verb, path, endpoint, contexts) else {
            continue;
        };
        match best {
            Some((best_score, _)) if score < best_score => {}
            Some((best_score, _)) if score == best_score => tied = true,
            _ => {
                best = Some((score, endpoint));
                tied = false;
            }
        }
    }
    best.filter(|_| !tied).map(|(_, endpoint)| endpoint)
}

/// The repositories serving a service the call's signature names.
fn addressed_repos<'a>(
    call: &HttpClientCallRow,
    facts: &HashMap<&'a str, &RepoHttpFacts>,
) -> HashSet<&'a str> {
    let services: Vec<&str> = call
        .signature
        .as_deref()
        .and_then(|s| s.strip_prefix(SERVICE_PREFIX))
        .map(|s| s.split(" | ").collect())
        .unwrap_or_default();
    facts
        .iter()
        .filter(|(_, f)| {
            f.service_names
                .iter()
                .any(|name| services.iter().any(|s| s.eq_ignore_ascii_case(name)))
        })
        .map(|(repo, _)| *repo)
        .collect()
}

fn endpoint_score(
    verb: &str,
    path: &str,
    endpoint: &HttpEndpointRow,
    contexts: &[String],
) -> Option<Score> {
    let (endpoint_verb, endpoint_path) = endpoint.name.split_once(' ')?;
    let exact_verb = verb == endpoint_verb;
    if !exact_verb && verb != ANY_METHOD && endpoint_verb != ANY_METHOD {
        return None;
    }
    std::iter::once(endpoint_path.to_string())
        .chain(contexts.iter().map(|c| format!("{c}{endpoint_path}")))
        .filter_map(|served| path_score(path, &served))
        .max()
        .map(|segments| (segments, exact_verb))
}

/// Segments matched literally (`items` ↔ `items`) and variables matched by
/// variables (`{id}` ↔ `{key}`), when every segment matches and at least
/// one literal does; a `{var}` facing a literal matches without scoring,
/// an unresolved `<expression>` never matches.
fn path_score(client: &str, served: &str) -> Option<PathScore> {
    let client: Vec<&str> = client.split('/').filter(|s| !s.is_empty()).collect();
    let served: Vec<&str> = served.split('/').filter(|s| !s.is_empty()).collect();
    if client.len() != served.len() {
        return None;
    }
    let (literals, variables) =
        client
            .iter()
            .zip(&served)
            .try_fold((0, 0), |(literals, variables), (a, b)| {
                if a.contains('<') || b.contains('<') {
                    return None;
                }
                match (is_variable(a), is_variable(b)) {
                    (true, true) => Some((literals, variables + 1)),
                    (false, false) => (a == b).then_some((literals + 1, variables)),
                    _ => Some((literals, variables)),
                }
            })?;
    (literals > 0).then_some((literals, variables))
}

fn is_variable(segment: &str) -> bool {
    segment.starts_with('{') && segment.ends_with('}')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn endpoint(uuid: &str, repo: &str, name: &str) -> HttpEndpointRow {
        HttpEndpointRow {
            uuid: uuid.to_string(),
            repo_name: repo.to_string(),
            name: name.to_string(),
        }
    }

    fn call(uuid: &str, name: &str, services: &[&str]) -> HttpClientCallRow {
        HttpClientCallRow {
            uuid: uuid.to_string(),
            name: name.to_string(),
            signature: (!services.is_empty())
                .then(|| format!("{SERVICE_PREFIX}{}", services.join(" | "))),
        }
    }

    fn repo(name: &str, services: &[&str], contexts: &[&str]) -> RepoHttpFacts {
        RepoHttpFacts {
            repo_name: name.to_string(),
            service_names: services.iter().map(|s| s.to_string()).collect(),
            context_paths: contexts.iter().map(|s| s.to_string()).collect(),
        }
    }

    fn links(calls: Vec<HttpClientCallRow>) -> Vec<(String, String)> {
        let inputs = HttpLinkInputs {
            endpoints: vec![
                endpoint("read", "items", "GET /items/{id}"),
                endpoint("export", "items", "GET /items/export"),
                endpoint("create", "items", "POST /items"),
                endpoint("items-health", "items", "GET /health"),
                endpoint("invoice", "billing", "POST /invoices"),
                endpoint("billing-health", "billing", "GET /health"),
                endpoint("any", "billing", "ANY /legacy"),
            ],
            calls,
            repos: vec![
                repo("items", &["items-service"], &["/api"]),
                repo("billing", &["billing-service"], &["/billing"]),
            ],
        };
        match_http_calls(&inputs)
    }

    fn link(uuid: &str, endpoint: &str) -> (String, String) {
        (uuid.to_string(), endpoint.to_string())
    }

    #[test]
    fn the_context_path_is_optional_in_the_client_path() {
        assert_eq!(
            links(vec![
                call("a", "GET /api/items/{id}", &["items-service"]),
                call("b", "POST /items", &[]),
                call("c", "POST /billing/invoices", &["billing", "localhost"]),
            ]),
            vec![link("a", "read"), link("b", "create"), link("c", "invoice")]
        );
    }

    #[test]
    fn the_most_literal_endpoint_wins() {
        assert_eq!(
            links(vec![
                call("var", "GET /items/{key}", &[]),
                call("literal", "GET /items/export", &[]),
                call("value", "GET /items/42", &[]),
            ]),
            vec![
                link("var", "read"),
                link("literal", "export"),
                link("value", "read")
            ]
        );
    }

    #[test]
    fn ties_link_nothing_unless_a_service_is_addressed() {
        assert_eq!(links(vec![call("h", "GET /health", &[])]), vec![]);
        assert_eq!(
            links(vec![call("h", "GET /billing/health", &["BILLING-SERVICE"])]),
            vec![link("h", "billing-health")]
        );
    }

    #[test]
    fn addressed_services_restrict_the_candidates() {
        assert_eq!(
            links(vec![call("x", "POST /invoices", &["items-service"])]),
            vec![]
        );
    }

    #[test]
    fn verbs_must_agree_unless_one_side_is_any() {
        assert_eq!(
            links(vec![
                call("put", "PUT /items", &[]),
                call("legacy", "DELETE /billing/legacy", &[]),
            ]),
            vec![link("legacy", "any")]
        );
    }

    #[test]
    fn unresolved_expressions_never_match() {
        assert_eq!(path_score("/<Api.BASE>/x", "/{a}/x"), None);
        assert_eq!(path_score("/a/{b}", "/a/{c}"), Some((1, 1)));
        assert_eq!(path_score("/a/{b}", "/a/c"), Some((1, 0)));
        assert_eq!(path_score("/a", "/a/b"), None);
    }

    #[test]
    fn wildcards_alone_never_match() {
        assert_eq!(path_score("/{index}/embeddings", "/items/{id}"), None);
        assert_eq!(path_score("/{a}/{b}", "/{x}/{y}"), None);
        assert_eq!(
            links(vec![call("g", "DELETE /{index}/embeddings", &[])]),
            vec![]
        );
    }

    #[test]
    fn repository_facts_come_from_configuration_and_application_paths() {
        let mut config = AppConfig::default();
        config.add_source(
            "application.yml",
            "spring:\n  application:\n    name: items-service\n  mvc:\n    servlet:\n      path: /v1\nserver:\n  servlet:\n    context-path: /api/\n",
        );
        config.add_source("application-k8s.yml", "spring.application.name: ${NAME}\n");
        let decorators = vec![vec![r#"@ApplicationPath("/rest")"#.to_string()]];
        assert_eq!(
            repo_http_facts("items", &config, &decorators),
            repo(
                "items",
                &["items-service"],
                &["/api", "/api/v1", "/rest", "/v1"]
            )
        );
    }
}
