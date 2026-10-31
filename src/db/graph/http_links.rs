//! Graph storage of the `HTTP_CALLS` links between declarative HTTP client
//! calls and the endpoints they hit (issue #46).

use std::collections::HashMap;
use std::time::Duration;

use anyhow::{Context, Result};
use neo4rs::{BoltType, Txn, query};
use tracing::warn;

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

/// What a relink pass did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RelinkSummary {
    /// Client calls in the graph.
    pub calls: usize,
    /// `HTTP_CALLS` edges created.
    pub links: usize,
}

/// Attempts of a relink pass before its error is returned: a pass aborted
/// by Neo4j (deadlock with a concurrent write) is retried from scratch.
const RELINK_ATTEMPTS: u32 = 3;

#[expect(
    async_fn_in_trait,
    reason = "async trait method is required for the db interfaces"
)]
pub trait HttpLinkExt {
    /// The decorators of the repository's `@ApplicationPath` types.
    async fn application_path_decorators(&self, repo_name: &str) -> Result<Vec<Vec<String>>>;
    /// Record `facts` on their `:Repository` node, then replace every
    /// `HTTP_CALLS` edge with the links `link` computes (client call UUID,
    /// endpoint UUID) from the endpoints, client calls and repository facts
    /// of the graph.
    ///
    /// One transaction does it all, and it starts by locking the singleton
    /// `:HttpCallsLock` node: a concurrent pass (another indexer or cluster
    /// node) waits until this one commits, then reads every entity it saw.
    /// The last pass therefore sees every committed repository, and readers
    /// never see the edges half replaced.
    async fn relink_http_calls<F>(&self, facts: &RepoHttpFacts, link: F) -> Result<RelinkSummary>
    where
        F: Fn(&HttpLinkInputs) -> Vec<(String, String)>;
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

    async fn relink_http_calls<F>(&self, facts: &RepoHttpFacts, link: F) -> Result<RelinkSummary>
    where
        F: Fn(&HttpLinkInputs) -> Vec<(String, String)>,
    {
        let mut attempt = 1;
        loop {
            match self.relink_once(facts, &link).await {
                Ok(summary) => return Ok(summary),
                Err(error) if attempt < RELINK_ATTEMPTS => {
                    warn!("HTTP_CALLS relink attempt {attempt} failed, retrying: {error:#}");
                    tokio::time::sleep(Duration::from_millis(200 * u64::from(attempt))).await;
                    attempt += 1;
                }
                Err(error) => return Err(error),
            }
        }
    }
}

impl GraphDb {
    /// One relink pass in its own transaction, rolled back on error.
    async fn relink_once<F>(&self, facts: &RepoHttpFacts, link: &F) -> Result<RelinkSummary>
    where
        F: Fn(&HttpLinkInputs) -> Vec<(String, String)>,
    {
        let mut txn = self
            .graph
            .start_txn()
            .await
            .context("Failed to start the HTTP_CALLS transaction")?;
        match relink_in(&mut txn, facts, link).await {
            Ok(summary) => {
                txn.commit()
                    .await
                    .context("Failed to commit the HTTP_CALLS links")?;
                Ok(summary)
            }
            Err(error) => {
                if let Err(rollback) = txn.rollback().await {
                    warn!("Failed to roll back the HTTP_CALLS transaction: {rollback}");
                }
                Err(error)
            }
        }
    }
}

async fn relink_in<F>(txn: &mut Txn, facts: &RepoHttpFacts, link: &F) -> Result<RelinkSummary>
where
    F: Fn(&HttpLinkInputs) -> Vec<(String, String)>,
{
    txn.run(query(
        "MERGE (l:HttpCallsLock {name: 'http_calls'})
         SET l.locked_at = timestamp()",
    ))
    .await
    .context("Failed to lock the HTTP_CALLS links")?;
    txn.run(
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
    .context("Failed to record the repository HTTP facts")?;
    let inputs = load_inputs(txn).await?;
    let links = link(&inputs);
    txn.run(query("MATCH ()-[r:HTTP_CALLS]->() DELETE r"))
        .await
        .context("Failed to delete HTTP_CALLS edges")?;
    if !links.is_empty() {
        let params: Vec<HashMap<String, BoltType>> = links
            .iter()
            .map(|(call, endpoint)| {
                HashMap::from([
                    ("call".to_string(), call.clone().into()),
                    ("endpoint".to_string(), endpoint.clone().into()),
                ])
            })
            .collect();
        txn.run(
            query(
                "UNWIND $links AS link
                 MATCH (c:Entity {uuid: link.call}), (e:Entity {uuid: link.endpoint})
                 MERGE (c)-[r:HTTP_CALLS]->(e)
                 SET r.heuristic = true",
            )
            .param("links", params),
        )
        .await
        .context("Failed to create HTTP_CALLS edges")?;
    }
    Ok(RelinkSummary {
        calls: inputs.calls.len(),
        links: links.len(),
    })
}

/// Every endpoint, client call and repository's HTTP facts, read inside
/// the relink transaction.
async fn load_inputs(txn: &mut Txn) -> Result<HttpLinkInputs> {
    let mut inputs = HttpLinkInputs::default();
    let mut rows = txn
        .execute(query(
            "MATCH (e:Entity)
             WHERE e.kind IN ['http_endpoint', 'http_client_call']
             RETURN e.kind AS kind, e.uuid AS uuid, e.name AS name,
                    e.repo_name AS repo_name, e.signature AS signature",
        ))
        .await
        .context("Failed to load HTTP endpoints and client calls")?;
    while let Some(row) = rows.next(txn.handle()).await? {
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
    let mut rows = txn
        .execute(query(
            "MATCH (r:Repository)
             RETURN r.name AS repo_name,
                    coalesce(r.http_service_names, []) AS service_names,
                    coalesce(r.http_context_paths, []) AS context_paths",
        ))
        .await
        .context("Failed to load the repository HTTP facts")?;
    while let Some(row) = rows.next(txn.handle()).await? {
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
