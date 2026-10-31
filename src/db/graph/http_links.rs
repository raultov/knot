//! Graph storage of the `HTTP_CALLS` links between declarative HTTP client
//! calls and the endpoints they hit (issue #46).

use std::collections::HashMap;

use anyhow::{Context, Result};
use neo4rs::{BoltType, query};

use super::GraphDb;

/// An indexed `http_endpoint` entity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpEndpointRow {
    pub uuid: String,
    pub repo_name: String,
    /// `<METHOD> <path>`.
    pub name: String,
}

/// An indexed `http_client_call` entity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpClientCallRow {
    pub uuid: String,
    /// `<METHOD> <path>`.
    pub name: String,
    /// `service: a | b`, when the client names its services.
    pub signature: Option<String>,
}

/// How a repository is addressed over HTTP, recorded on its
/// `:Repository` node.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RepoHttpFacts {
    pub repo_name: String,
    /// `spring.application.name`, `quarkus.application.name`.
    pub service_names: Vec<String>,
    /// Paths every endpoint is served under (`server.servlet.context-path`,
    /// `@ApplicationPath`, …).
    pub context_paths: Vec<String>,
}

/// Everything the HTTP call linker reads, across every indexed repository.
#[derive(Debug, Default)]
pub struct HttpLinkInputs {
    pub endpoints: Vec<HttpEndpointRow>,
    pub calls: Vec<HttpClientCallRow>,
    pub repos: Vec<RepoHttpFacts>,
}

#[expect(
    async_fn_in_trait,
    reason = "async trait method is required for the db interfaces"
)]
pub trait HttpLinkExt {
    /// The decorators of the repository's `@ApplicationPath` types.
    async fn application_path_decorators(&self, repo_name: &str) -> Result<Vec<Vec<String>>>;
    async fn upsert_repo_http_facts(&self, facts: &RepoHttpFacts) -> Result<()>;
    async fn load_http_link_inputs(&self) -> Result<HttpLinkInputs>;
    /// Replace every `HTTP_CALLS` edge with `links` (client call UUID,
    /// endpoint UUID).
    async fn replace_http_calls(&self, links: &[(String, String)]) -> Result<()>;
}

impl HttpLinkExt for GraphDb {
    async fn application_path_decorators(&self, repo_name: &str) -> Result<Vec<Vec<String>>> {
        let mut rows = self
            .graph
            .execute(
                query(
                    "MATCH (e:Entity {repo_name: $repo_name})
                     WHERE any(d IN coalesce(e.decorators, []) WHERE d CONTAINS 'ApplicationPath')
                     RETURN e.decorators AS decorators",
                )
                .param("repo_name", repo_name),
            )
            .await
            .context("Failed to query @ApplicationPath types")?;
        let mut result = Vec::new();
        while let Ok(Some(row)) = rows.next().await {
            result.push(row.get::<Vec<String>>("decorators").unwrap_or_default());
        }
        Ok(result)
    }

    async fn upsert_repo_http_facts(&self, facts: &RepoHttpFacts) -> Result<()> {
        self.graph
            .run(
                query(
                    "MERGE (r:Repository {name: $repo_name})
                     SET r.http_service_names = $service_names,
                         r.http_context_paths = $context_paths",
                )
                .param("repo_name", facts.repo_name.as_str())
                .param("service_names", facts.service_names.clone())
                .param("context_paths", facts.context_paths.clone()),
            )
            .await
            .context("Failed to record the repository HTTP facts")
    }

    async fn load_http_link_inputs(&self) -> Result<HttpLinkInputs> {
        let mut inputs = HttpLinkInputs::default();
        let mut rows = self
            .graph
            .execute(query(
                "MATCH (e:Entity)
                 WHERE e.kind IN ['http_endpoint', 'http_client_call']
                 RETURN e.kind AS kind, e.uuid AS uuid, e.name AS name,
                        e.repo_name AS repo_name, e.signature AS signature",
            ))
            .await
            .context("Failed to load HTTP endpoints and client calls")?;
        while let Ok(Some(row)) = rows.next().await {
            let (Ok(kind), Ok(uuid), Ok(name)) = (
                row.get::<String>("kind"),
                row.get::<String>("uuid"),
                row.get::<String>("name"),
            ) else {
                continue;
            };
            if kind == "http_endpoint" {
                inputs.endpoints.push(HttpEndpointRow {
                    uuid,
                    repo_name: row.get::<String>("repo_name").unwrap_or_default(),
                    name,
                });
            } else {
                inputs.calls.push(HttpClientCallRow {
                    uuid,
                    name,
                    signature: row.get::<String>("signature").ok(),
                });
            }
        }
        let mut rows = self
            .graph
            .execute(query(
                "MATCH (r:Repository)
                 RETURN r.name AS repo_name,
                        coalesce(r.http_service_names, []) AS service_names,
                        coalesce(r.http_context_paths, []) AS context_paths",
            ))
            .await
            .context("Failed to load the repository HTTP facts")?;
        while let Ok(Some(row)) = rows.next().await {
            let Ok(repo_name) = row.get::<String>("repo_name") else {
                continue;
            };
            inputs.repos.push(RepoHttpFacts {
                repo_name,
                service_names: row.get::<Vec<String>>("service_names").unwrap_or_default(),
                context_paths: row.get::<Vec<String>>("context_paths").unwrap_or_default(),
            });
        }
        Ok(inputs)
    }

    async fn replace_http_calls(&self, links: &[(String, String)]) -> Result<()> {
        self.graph
            .run(query("MATCH ()-[r:HTTP_CALLS]->() DELETE r"))
            .await
            .context("Failed to delete HTTP_CALLS edges")?;
        if links.is_empty() {
            return Ok(());
        }
        let params: Vec<HashMap<String, BoltType>> = links
            .iter()
            .map(|(call, endpoint)| {
                HashMap::from([
                    ("call".to_string(), call.clone().into()),
                    ("endpoint".to_string(), endpoint.clone().into()),
                ])
            })
            .collect();
        self.graph
            .run(
                query(
                    "UNWIND $links AS link
                     MATCH (c:Entity {uuid: link.call}), (e:Entity {uuid: link.endpoint})
                     MERGE (c)-[r:HTTP_CALLS]->(e)
                     SET r.heuristic = true",
                )
                .param("links", params),
            )
            .await
            .context("Failed to create HTTP_CALLS edges")
    }
}
