//! Core deps logic shared between CLI and MCP.
//!
//! Shows the dependency graph for a repository, including which
//! dependencies are locally indexed via the DEPENDS_ON relationship.

use std::sync::Arc;

use crate::db::graph::{GraphDb, RepoIdentity, RepoQueryExt};
use crate::pipeline::ingest::{find_declaring_consumers, resolve_declared_dependencies};

/// Default dependency-graph depth when the caller does not ask for one.
/// Must equal the `default` advertised by the MCP schema
/// ([`crate::mcp_tools::list_repo_dependencies`]) and the CLI's
/// `deps --depth` default.
pub const DEFAULT_MAX_DEPTH: u32 = 3;

/// Hard ceiling for the dependency-graph depth. Must equal the `maximum`
/// advertised by the MCP schema — the drift guard test in
/// `mcp_tools::list_repo_dependencies` pins the two together. A depth of 0
/// would also compile into invalid Cypher (`*1..0`), hence the floor of 1.
pub const MAX_DEPTH_CEILING: u32 = 10;

/// Clamp a caller-requested traversal depth into the advertised
/// `1..=MAX_DEPTH_CEILING` range (see [`run_deps`]).
pub fn resolve_max_depth(requested: u32) -> u32 {
    requested.clamp(1, MAX_DEPTH_CEILING)
}

/// One declared build dependency, classified against the repository registry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeclaredDependency {
    /// Declared name verbatim as stored (e.g. `npm:react:^19.2.8`).
    pub name: String,
    /// Repository the artifact resolves to, or `None` when no indexed
    /// repository matches the declared identity.
    pub resolved_repo: Option<String>,
}

/// An indexed repository that declares the queried repository as a build
/// dependency, without a `DEPENDS_ON` edge yet (stale graph).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeclaringConsumer {
    /// Repository that declares the dependency.
    pub repo_name: String,
    /// First declared name (verbatim) that resolves back to the queried repo.
    pub declared_as: String,
}

/// Why a dependency lookup came back empty, classified per direction.
///
/// Populated only when the result set is empty; it never affects the JSON
/// payload returned by [`run_deps`] (the JSON shape is an API contract and
/// stays an array of `{ "repo_name": ... }` objects).
#[derive(Debug, Clone)]
pub enum DepsDirection {
    /// Forward lookup: each declared `build_dependency` name of the queried
    /// repository, verbatim and in the query's ORDER BY order, classified
    /// against the registry of indexed repositories.
    Forward(Vec<DeclaredDependency>),
    /// Reverse lookup.
    ///
    /// - `Some(list)` — every indexed repository's declarations were checked:
    ///   `list` holds those that declare the queried repository without an
    ///   edge yet (possibly empty = genuinely nobody declares it).
    /// - `None` — the queried repository has no matchable build identity, so
    ///   the question is unanswerable and must not be reported as "nobody
    ///   declares it".
    Reverse(Option<Vec<DeclaringConsumer>>),
}

/// Diagnostics for an empty dependency lookup.
#[derive(Debug, Clone)]
pub struct DepsDiagnostics {
    /// Build-system identity recorded on the `:Repository` node, or `None`
    /// when no node exists for the requested name at all.
    pub identity: Option<RepoIdentity>,
    /// Direction-specific classification of the empty result.
    pub direction: DepsDirection,
}

/// Gather diagnostics so an empty lookup can be explained honestly instead of
/// reporting a bare "No dependencies found." Only fetched when the result is
/// empty, so the common path does not pay for it.
///
/// Best-effort: any database failure leaves the diagnostics at a level that
/// never *asserts* a negative the lookup did not verify — a reverse-side
/// failure degrades to `Reverse(None)` ("unanswerable"), never to an empty
/// list ("nobody declares it").
pub async fn collect_deps_diagnostics(
    repo_name: &str,
    reverse: bool,
    graph_db: &Arc<GraphDb>,
) -> anyhow::Result<DepsDiagnostics> {
    let identity = graph_db
        .find_repository_identity(repo_name)
        .await
        .ok()
        .flatten();

    let direction = if reverse {
        match identity.as_ref() {
            None => DepsDirection::Reverse(None),
            Some(id) => DepsDirection::Reverse(
                find_declaring_consumers(repo_name, id, graph_db)
                    .await
                    .ok()
                    .flatten()
                    .map(|consumers| {
                        consumers
                            .into_iter()
                            .map(|(repo_name, declared_as)| DeclaringConsumer {
                                repo_name,
                                declared_as,
                            })
                            .collect::<Vec<_>>()
                    }),
            ),
        }
    } else {
        let declared = graph_db
            .find_build_dependency_names(repo_name)
            .await
            .unwrap_or_default();
        let classified = resolve_declared_dependencies(&declared, graph_db)
            .await
            .unwrap_or_else(|_| declared.iter().map(|n| (n.clone(), None)).collect());
        DepsDirection::Forward(
            classified
                .into_iter()
                .map(|(name, resolved_repo)| DeclaredDependency {
                    name,
                    resolved_repo,
                })
                .collect(),
        )
    };

    Ok(DepsDiagnostics {
        identity,
        direction,
    })
}

pub async fn run_deps(
    repo_name: &str,
    max_depth: u32,
    reverse: bool,
    graph_db: &Arc<GraphDb>,
) -> anyhow::Result<serde_json::Value> {
    // Enforce the advertised bound before anything so no caller (CLI or MCP)
    // can push an unbounded `DEPENDS_ON*` traversal (or a zero depth, which
    // would compile into invalid Cypher).
    let max_depth = resolve_max_depth(max_depth);
    if reverse {
        let dependents = graph_db.find_repo_dependents(repo_name, max_depth).await?;
        let result: Vec<serde_json::Value> = dependents
            .into_iter()
            .map(|d| serde_json::json!({ "repo_name": d }))
            .collect();
        Ok(serde_json::json!(result))
    } else {
        let deps = graph_db
            .find_repo_dependencies(repo_name, max_depth)
            .await?;
        let result: Vec<serde_json::Value> = deps
            .into_iter()
            .map(|d| serde_json::json!({ "repo_name": d }))
            .collect();
        Ok(serde_json::json!(result))
    }
}

pub fn format_deps_output(repo_name: &str, reverse: bool, result: &serde_json::Value) -> String {
    format_deps_output_with_diagnostics(repo_name, reverse, result, None)
}

/// Format the human-readable deps output, using `diagnostics` (when the
/// lookup came back empty) to explain *why* the result is empty.
pub fn format_deps_output_with_diagnostics(
    repo_name: &str,
    reverse: bool,
    result: &serde_json::Value,
    diagnostics: Option<&DepsDiagnostics>,
) -> String {
    let mut output = String::new();

    if reverse {
        output.push_str(&format!(
            "# Repositories that depend on `{}`\n\n",
            repo_name
        ));
    } else {
        output.push_str(&format!("# Dependencies of `{}`\n\n", repo_name));
    }

    if let Some(arr) = result.as_array() {
        if arr.is_empty() {
            output.push_str(&format_empty_explanation(repo_name, reverse, diagnostics));
        } else {
            for dep in arr {
                if let Some(name) = dep.get("repo_name").and_then(|v| v.as_str()) {
                    output.push_str(&format!("+-- {}\n", name));
                }
            }
        }
    } else {
        output.push_str("No dependencies found.\n");
    }

    output
}

/// Explanation for an empty list. Never emits the bare "No dependencies
/// found." when diagnostics are available — that string is indistinguishable
/// between "no dependencies declared" and "dependencies exist but the graph
/// is empty". Classification is three-way per direction:
/// 1. resolves to an indexed repository and the edge exists — normal (never
///    reaches here),
/// 2. resolves but no edge yet — stale graph, re-index hint,
/// 3. does not resolve — the not-indexed list.
///
/// A declared artifact that *resolves* is never reported as "not indexed".
fn format_empty_explanation(
    repo_name: &str,
    reverse: bool,
    diagnostics: Option<&DepsDiagnostics>,
) -> String {
    let Some(diag) = diagnostics else {
        return "No dependencies found.\n".to_string();
    };

    let Some(identity) = &diag.identity else {
        return format!(
            "Repository `{repo_name}` is not indexed.\n\
             Run `knot-indexer --repo-path <path>` first, then retry.\n"
        );
    };

    if reverse {
        format_reverse_empty(repo_name, identity, diag)
    } else {
        format_forward_empty(repo_name, identity, diag)
    }
}

/// Identity fragment for the parenthetical "(this repository is indexed as
/// X with build_system `Y`)" — omitted when the identity carries no usable
/// artifact name.
fn identity_clause(identity: &RepoIdentity) -> String {
    let label = identity_label(identity);
    if label.is_empty() {
        format!("(build_system `{}`)", identity.build_system)
    } else {
        format!(
            "(this repository is indexed as {label} with build_system `{}`)",
            identity.build_system
        )
    }
}

fn format_forward_empty(
    repo_name: &str,
    identity: &RepoIdentity,
    diag: &DepsDiagnostics,
) -> String {
    let DepsDirection::Forward(declared) = &diag.direction else {
        // Unreachable by construction: forward callers always build Forward.
        return format!(
            "No indexed-repository dependencies found.\n\n{}.\n",
            identity_clause(identity)
        );
    };

    if declared.is_empty() {
        return format!(
            "No dependencies found: `{repo_name}` declares no build \
             dependencies.\n{}.\n",
            identity_clause(identity)
        );
    }

    let linked: Vec<&DeclaredDependency> = declared
        .iter()
        .filter(|d| d.resolved_repo.is_some())
        .collect();
    let unlinked: Vec<&DeclaredDependency> = declared
        .iter()
        .filter(|d| d.resolved_repo.is_none())
        .collect();

    if linked.is_empty() {
        // Byte-identical legacy branch: nothing resolves, list everything.
        let mut output = format!(
            "No indexed-repository dependencies found.\n\n\
             `{repo_name}` declares {} build dependencies, but none of them \
             resolves to a repository indexed in knot.\n\
             Index the dependency's own repository with \
             `knot-indexer --repo-path <path>` — the DEPENDS_ON edge is\n\
             created by that run (or by re-indexing {repo_name}), then retry.\n\n\
             Declared build dependencies ({}):\n",
            declared.len(),
            declared.len()
        );
        for dep in declared {
            output.push_str("  ");
            output.push_str(&dep.name);
            output.push('\n');
        }
        return output;
    }

    // Stale graph: some declared dependencies resolve to indexed
    // repositories, but no DEPENDS_ON edge exists.
    let mut output = format!(
        "No indexed-repository dependencies found.\n\n\
         `{repo_name}` declares {} build dependencies; {} of them resolve to \
         a repository indexed in knot but have no DEPENDS_ON edge yet:\n",
        declared.len(),
        linked.len()
    );
    for dep in &linked {
        output.push_str(&format!(
            "  {} -> {}\n",
            dep.name,
            dep.resolved_repo.as_deref().unwrap_or_default()
        ));
    }
    output.push_str(&format!(
        "The graph is stale: re-index either side to create the edge(s)\n\
         (`knot-indexer --repo-path <path>` on `{repo_name}` or on the \
         dependency's own repository).\n"
    ));
    if !unlinked.is_empty() {
        output.push_str(&format!(
            "\nThe other {} declared dependencies resolve to no indexed \
             repository:\n",
            unlinked.len()
        ));
        for dep in &unlinked {
            output.push_str("  ");
            output.push_str(&dep.name);
            output.push('\n');
        }
    }
    output
}

fn format_reverse_empty(
    repo_name: &str,
    identity: &RepoIdentity,
    diag: &DepsDiagnostics,
) -> String {
    let DepsDirection::Reverse(consumers) = &diag.direction else {
        // Unreachable by construction: reverse callers always build Reverse.
        return format!("No repositories depend on `{repo_name}`.\n");
    };

    let Some(consumers) = consumers else {
        // Unanswerable: no matchable build identity. Must not claim "nobody
        // declares it" — the declarations were never checked.
        return format!(
            "Cannot determine dependents of `{repo_name}`: the repository \
             has no matchable build identity {}.\n\
             Re-index it with a build manifest (pom.xml, build.gradle, \
             Cargo.toml, package.json, or .csproj) so its identity can be \
             matched against declared dependencies.\n",
            identity_clause(identity)
        );
    };

    if consumers.is_empty() {
        return format!(
            "No repositories depend on `{repo_name}`: no indexed repository \
             declares it as a build dependency\n{}.\n",
            identity_clause(identity)
        );
    }

    // Stale graph in reverse: indexed repositories declare this repo, but
    // the linking never created the DEPENDS_ON edges.
    let mut output = format!(
        "No DEPENDS_ON edges point at `{repo_name}`, but {} indexed \
         repositories declare it as a build dependency:\n",
        consumers.len()
    );
    for consumer in consumers {
        output.push_str(&format!(
            "  {} (declares `{}`)\n",
            consumer.repo_name, consumer.declared_as
        ));
    }
    output.push_str(
        "The graph is stale: re-index either side to create the edge(s)\n\
         (`knot-indexer --repo-path <path>` on this repository or on the \
         listed consumer(s)).\n",
    );
    output
}

fn identity_label(identity: &RepoIdentity) -> String {
    if identity.group_id.is_empty() {
        identity.artifact_id.clone()
    } else {
        format!("{}:{}", identity.group_id, identity.artifact_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // --- resolve_max_depth (advertised-bound contract) ---

    #[test]
    fn resolve_max_depth_floors_zero_on_one() {
        // 0 would compile into invalid Cypher (`*1..0`).
        assert_eq!(resolve_max_depth(0), 1);
    }

    #[test]
    fn resolve_max_depth_preserves_values_within_bound() {
        assert_eq!(resolve_max_depth(DEFAULT_MAX_DEPTH), DEFAULT_MAX_DEPTH);
        assert_eq!(resolve_max_depth(1), 1);
        assert_eq!(resolve_max_depth(MAX_DEPTH_CEILING), MAX_DEPTH_CEILING);
    }

    #[test]
    fn resolve_max_depth_clamps_above_ceiling() {
        assert_eq!(resolve_max_depth(11), MAX_DEPTH_CEILING);
        assert_eq!(resolve_max_depth(1_000_000), MAX_DEPTH_CEILING);
    }

    #[test]
    fn test_format_deps_output_empty() {
        let result = json!([]);
        let formatted = format_deps_output("my-app", false, &result);
        assert!(formatted.contains("Dependencies of `my-app`"));
        assert!(formatted.contains("No dependencies found"));
    }

    #[test]
    fn test_format_deps_output_with_deps() {
        let result = json!([
            {"repo_name": "auth-lib"},
            {"repo_name": "common-utils"}
        ]);
        let formatted = format_deps_output("my-app", false, &result);
        assert!(formatted.contains("Dependencies of `my-app`"));
        assert!(formatted.contains("+-- auth-lib"));
        assert!(formatted.contains("+-- common-utils"));
    }

    #[test]
    fn test_format_deps_output_reverse() {
        let result = json!([
            {"repo_name": "my-app"},
            {"repo_name": "admin-portal"}
        ]);
        let formatted = format_deps_output("auth-lib", true, &result);
        assert!(formatted.contains("Repositories that depend on `auth-lib`"));
        assert!(formatted.contains("+-- my-app"));
        assert!(formatted.contains("+-- admin-portal"));
    }

    #[test]
    fn test_format_deps_output_single_dep() {
        let result = json!([
            {"repo_name": "core-lib"}
        ]);
        let formatted = format_deps_output("my-app", false, &result);
        assert!(formatted.contains("Dependencies of `my-app`"));
        assert!(formatted.contains("+-- core-lib"));
    }

    #[test]
    fn test_format_deps_output_null_returns_no_deps() {
        let formatted = format_deps_output("my-app", false, &serde_json::Value::Null);
        assert!(formatted.contains("No dependencies found"));
    }

    #[test]
    fn test_format_deps_output_object_returns_no_deps() {
        let result = json!({"repo_name": "should-be-array"});
        let formatted = format_deps_output("my-app", false, &result);
        assert!(formatted.contains("No dependencies found"));
    }

    #[test]
    fn test_format_deps_output_missing_repo_name_field() {
        let result = json!([{"other": "value"}]);
        let formatted = format_deps_output("my-app", false, &result);
        assert!(!formatted.contains("No dependencies found"));
        assert!(!formatted.contains("+--"));
    }

    // ─── diagnostics-driven empty explanations ────────────────────────────

    fn identity(build_system: &str, gid: &str, aid: &str) -> RepoIdentity {
        RepoIdentity {
            build_system: build_system.to_string(),
            group_id: gid.to_string(),
            artifact_id: aid.to_string(),
            version: "1.0.0".to_string(),
        }
    }

    fn diag(identity: Option<RepoIdentity>, direction: DepsDirection) -> DepsDiagnostics {
        DepsDiagnostics {
            identity,
            direction,
        }
    }

    fn forward(declared: &[(&str, Option<&str>)]) -> DepsDirection {
        DepsDirection::Forward(
            declared
                .iter()
                .map(|(name, repo)| DeclaredDependency {
                    name: (*name).to_string(),
                    resolved_repo: repo.map(|r| r.to_string()),
                })
                .collect(),
        )
    }

    fn reverse(consumers: &[(&str, &str)]) -> DepsDirection {
        DepsDirection::Reverse(Some(
            consumers
                .iter()
                .map(|(repo, declared_as)| DeclaringConsumer {
                    repo_name: (*repo).to_string(),
                    declared_as: (*declared_as).to_string(),
                })
                .collect(),
        ))
    }

    #[test]
    fn test_empty_without_diagnostics_fall_back_unchanged() {
        let formatted = format_deps_output_with_diagnostics("my-app", false, &json!([]), None);
        assert!(formatted.contains("No dependencies found."));
    }

    #[test]
    fn test_empty_not_indexed_message() {
        let d = diag(None, forward(&[]));
        let formatted = format_deps_output_with_diagnostics("my-app", false, &json!([]), Some(&d));
        assert!(formatted.contains("Repository `my-app` is not indexed."));
        assert!(!formatted.contains("No dependencies found."));
    }

    #[test]
    fn test_empty_no_declared_dependencies_message() {
        let d = diag(Some(identity("npm", "", "knot-site")), forward(&[]));
        let formatted =
            format_deps_output_with_diagnostics("knot-site", false, &json!([]), Some(&d));
        assert!(formatted.contains("declares no build dependencies"));
        assert!(formatted.contains("build_system `npm`"));
        assert!(!formatted.contains("No dependencies found.\n"));
    }

    #[test]
    fn test_empty_declared_but_unresolved_lists_everything_verbatim() {
        // Registry mocks: every declared dependency fails to resolve, so the
        // legacy "none of them resolves" branch applies unchanged.
        let d = diag(
            Some(identity("npm", "", "job-watch-ui")),
            forward(&[
                ("npm:@eslint/js:^9.39.5", None),
                ("npm:@hookform/resolvers:^5.7.1", None),
                ("npm:react:^19.2.8", None),
                ("npm:vite:^6.4.3", None),
            ]),
        );
        let formatted =
            format_deps_output_with_diagnostics("job-watch-ui", false, &json!([]), Some(&d));
        // Quantified header with the exact total.
        assert!(formatted.contains("declares 4 build dependencies"));
        assert!(formatted.contains("none of them resolves to a repository indexed in knot"));
        // The list is uncapped and verbatim as stored.
        assert!(formatted.contains("npm:@eslint/js:^9.39.5"));
        assert!(formatted.contains("npm:@hookform/resolvers:^5.7.1"));
        assert!(formatted.contains("npm:react:^19.2.8"));
        assert!(formatted.contains("npm:vite:^6.4.3"));
        assert!(formatted.contains("Declared build dependencies (4):"));
        assert!(!formatted.contains("No dependencies found.\n"));
    }

    #[test]
    fn test_empty_resolves_but_no_edge_reports_stale_graph() {
        // The bug fix: a declared dependency that RESOLVES to an indexed
        // repository must never be reported as "not indexed"/"none resolves".
        let d = diag(
            Some(identity("cargo", "", "job-watch")),
            forward(&[
                ("cdp-browser-lite:0.3.4", Some("cdp-browser-lite")),
                ("serde:1.0", None),
                ("anyhow:1.0", None),
            ]),
        );
        let formatted =
            format_deps_output_with_diagnostics("job-watch", false, &json!([]), Some(&d));
        assert!(
            !formatted.contains("none of them resolves"),
            "resolved dependency must not be reported as unresolvable: {formatted}"
        );
        assert!(!formatted.contains("Declared build dependencies (3):"));
        assert!(formatted.contains("declares 3 build dependencies"));
        assert!(formatted.contains("1 of them resolve to a repository indexed in knot"));
        assert!(formatted.contains("cdp-browser-lite:0.3.4 -> cdp-browser-lite"));
        assert!(formatted.contains("The graph is stale"));
        assert!(formatted.contains("re-index either side"));
        // The non-resolving remainder is still listed, verbatim.
        assert!(formatted.contains("The other 2 declared dependencies"));
        assert!(formatted.contains("serde:1.0"));
        assert!(formatted.contains("anyhow:1.0"));
    }

    #[test]
    fn test_empty_all_resolve_but_no_edge_omits_remainder_block() {
        let d = diag(
            Some(identity("cargo", "", "job-watch")),
            forward(&[("cdp-browser-lite:0.3.4", Some("cdp-browser-lite"))]),
        );
        let formatted =
            format_deps_output_with_diagnostics("job-watch", false, &json!([]), Some(&d));
        assert!(!formatted.contains("none of them resolves"));
        assert!(formatted.contains("1 of them resolve"));
        assert!(!formatted.contains("The other"));
    }

    #[test]
    fn test_empty_reverse_declared_but_not_dependent() {
        // Registry mock: nothing anywhere declares `shared-lib`.
        let d = diag(Some(identity("npm", "", "shared-lib")), reverse(&[]));
        let formatted =
            format_deps_output_with_diagnostics("shared-lib", true, &json!([]), Some(&d));
        assert!(formatted.contains("No repositories depend on `shared-lib`"));
        assert!(formatted.contains("no indexed repository declares it"));
    }

    #[test]
    fn test_empty_reverse_consumer_declares_without_edge() {
        // Reverse stale graph: consumers declare the repo but no edge exists.
        let d = diag(
            Some(identity("cargo", "", "cdp-browser-lite")),
            reverse(&[
                ("job-watch", "cdp-browser-lite:0.3.4"),
                ("chrome-control-mcp", "cdp-browser-lite:0.3"),
            ]),
        );
        let formatted =
            format_deps_output_with_diagnostics("cdp-browser-lite", true, &json!([]), Some(&d));
        assert!(formatted.contains("No DEPENDS_ON edges point at `cdp-browser-lite`"));
        assert!(formatted.contains("2 indexed repositories declare it"));
        assert!(formatted.contains("job-watch (declares `cdp-browser-lite:0.3.4`)"));
        assert!(formatted.contains("chrome-control-mcp (declares `cdp-browser-lite:0.3`)"));
        assert!(formatted.contains("The graph is stale"));
        assert!(
            !formatted.contains("none of them resolves back"),
            "stale reverse case must not claim nobody resolves: {formatted}"
        );
        assert!(!formatted.contains("no indexed repository declares it"));
    }

    #[test]
    fn test_empty_reverse_unmatchable_identity_is_not_claimed_as_unanswered() {
        // Reverse(None): no matchable build identity. The message must NOT
        // assert "no indexed repository declares it" — the declarations were
        // never checked.
        let d = diag(Some(identity("none", "", "")), DepsDirection::Reverse(None));
        let formatted =
            format_deps_output_with_diagnostics("csharp-code-map", true, &json!([]), Some(&d));
        assert!(formatted.contains("Cannot determine dependents of `csharp-code-map`"));
        assert!(formatted.contains("no matchable build identity"));
        assert!(formatted.contains("build_system `none`"));
        assert!(
            !formatted.contains("no indexed repository declares it"),
            "unanswerable must not be rendered as a verified negative: {formatted}"
        );
        assert!(!formatted.contains("The graph is stale"));
    }

    #[test]
    fn test_identity_clause_omits_empty_label() {
        // Regression: `indexed as  with build_system` double-space artifact.
        let clause = identity_clause(&identity("none", "", ""));
        assert_eq!(clause, "(build_system `none`)");
        assert!(!clause.contains("indexed as"));
        let clause = identity_clause(&identity("maven", "com.acme", "auth-lib"));
        assert!(clause.contains("indexed as com.acme:auth-lib"));
    }

    #[test]
    fn test_reverse_without_diagnostics_fall_back_unchanged() {
        let formatted = format_deps_output_with_diagnostics("my-app", true, &json!([]), None);
        assert!(formatted.contains("No dependencies found."));
    }

    #[test]
    fn test_identity_label_renders_gav_or_flat_id() {
        assert_eq!(
            identity_label(&identity("maven", "com.acme", "auth-lib")),
            "com.acme:auth-lib"
        );
        assert_eq!(
            identity_label(&identity("npm", "", "job-watch-ui")),
            "job-watch-ui"
        );
    }

    #[test]
    fn test_diagnostics_keep_declared_verbatim_and_ordered_by_query() {
        // Forward classification preserves the input order (the query's
        // ORDER BY) and keeps names verbatim; formatting never re-sorts or
        // truncates.
        let d = diag(
            Some(identity("npm", "", "x")),
            forward(&[("b", Some("repo-b")), ("a", None)]),
        );
        let DepsDirection::Forward(declared) = &d.direction else {
            panic!("expected Forward");
        };
        assert_eq!(declared[0].name, "b");
        assert_eq!(declared[0].resolved_repo.as_deref(), Some("repo-b"));
        assert_eq!(declared[1].name, "a");
        assert_eq!(declared[1].resolved_repo, None);
    }
}
