//! Core `list_files` logic shared between CLI and MCP.
//!
//! A read-only enumeration of the files an indexed repository carries,
//! backed by the Neo4j graph (`QueryExt::list_files`). Agents that do not
//! already know a path can discover the available files before narrowing
//! their searches (`search_hybrid_context` accepts the same prefixes via
//! its `path` parameter).
//!
//! # Semantics
//! - `path` is optional: absent/empty lists every file of the repository
//!   scope; otherwise it is normalized exactly like `explore_file` (POSIX
//!   separators, no leading `./`, local-root stripping) and matched as a
//!   **directory-prefix with path boundary**: `src/api` matches
//!   `src/api/users.ts` but not `src/api-notes.md`.
//! - A glob pattern (`*`, `?`, `**`) switches to pattern matching over the
//!   full path list fetched from the graph.
//! - Ordering is deterministic: `(repo_name, file_path)` from the graph
//!   query; glob filtering preserves that order; the reply carries
//!   `"truncated": true` when the cap cut the list.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Result;
use serde_json::json;

use crate::db::graph::{GraphDb, QueryExt};
use crate::models::RepoScope;

/// Hard cap on rows fetched from the graph: the read-only surface stays
/// cheap on monorepos, and the reply says `"truncated": true` so callers
/// know to narrow the filter.
const MAX_FILES: usize = 2000;

/// Normalize caller-supplied path input to the canonical relative form of
/// the index — the `explore_file` §4 behavior inherited wholesale: POSIX
/// separators, no leading `./`, local-root stripping when the input names
/// something that exists on disk.
pub fn normalize_list_path(path: &str, cwd: Option<&Path>, repo_root: Option<&Path>) -> String {
    super::explore_file::normalize_explore_input(path, repo_root)
        .pipe(|normalized| resolve_against_disk(path, cwd, repo_root, normalized))
}

/// Helper so the normalization stays a single readable expression.
trait Pipe: Sized {
    fn pipe<T>(self, f: impl FnOnce(Self) -> T) -> T {
        f(self)
    }
}
impl<T> Pipe for T {}

fn resolve_against_disk(
    raw: &str,
    cwd: Option<&Path>,
    repo_root: Option<&Path>,
    normalized: String,
) -> String {
    let resolved = super::explore_file::resolve_explore_input(raw, cwd, repo_root);
    // `resolve_explore_input` only returns a shorter, meaningful value
    // when the input exists on disk; otherwise it echoes the normalized
    // form. Accept whichever is the canonical relative path.
    if resolved.len() < raw.len() && !resolved.is_empty() {
        resolved
    } else {
        normalized
    }
}

/// Whether `candidate` (a normalized repo-relative path) matches the
/// normalized `prefix` under directory-boundary semantics.
///
/// An empty prefix (or `.`, or `/`) matches everything. A plain directory
/// prefix must match on a `/` boundary: `src/api` matches `src/api/users.ts`
/// and the bare `src/api` itself, but not `src/api-notes.md`. A prefix
/// containing `*` or `?` is treated as a glob ([`glob_match`]).
pub fn path_matches(prefix: &str, candidate: &str) -> bool {
    let p = prefix.trim_matches('/');
    if p.is_empty() || p == "." {
        return true;
    }
    if p.contains('*') || p.contains('?') {
        return glob_match(p, candidate);
    }
    candidate == p || candidate.starts_with(&format!("{p}/"))
}

/// Minimal deterministic glob matcher over `/`-separated paths.
///
/// Pattern language:
/// - `?` — exactly one character, never `/`
/// - `*` — any run of characters except `/` (segment-local)
/// - `**` — a full path segment that matches any number of segments
///   (including zero): `src/**/*.rs`
///
/// Case-sensitive on all platforms. Segment-wise, so `*` can never cross
/// a path edge.
pub fn glob_match(pattern: &str, candidate: &str) -> bool {
    let p_segs: Vec<&str> = pattern.split('/').filter(|s| !s.is_empty()).collect();
    let c_segs: Vec<&str> = candidate.split('/').filter(|s| !s.is_empty()).collect();
    segments_match(&p_segs, &c_segs)
}

/// Segment-wise matcher with recursion only on `**` patterns (patterns
/// chains are tiny; deterministic depth).
fn segments_match(p: &[&str], c: &[&str]) -> bool {
    match p.first() {
        None => c.is_empty(),
        Some(&"**") => {
            // Zero or more segments consumed.
            for skip in 0..=c.len() {
                if segments_match(&p[1..], &c[skip..]) {
                    return true;
                }
            }
            false
        }
        Some(&first) => {
            if c.is_empty() || !segment_match(first, c[0]) {
                return false;
            }
            segments_match(&p[1..], &c[1..])
        }
    }
}

/// One `/`-free segment: `*` wildcard runs and `?` single non-`/` chars.
fn segment_match(pattern: &str, segment: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let s: Vec<char> = segment.chars().collect();
    // Classic two-slot wildcard match (backtracking on the last `*`).
    let (mut pi, mut si) = (0usize, 0usize);
    let mut star: Option<(usize, usize)> = None;
    while si < s.len() {
        if pi < p.len() {
            match p[pi] {
                '*' => {
                    star = Some((pi, si));
                    pi += 1;
                    continue;
                }
                '?' => {
                    pi += 1;
                    si += 1;
                    continue;
                }
                ch if ch == s[si] => {
                    pi += 1;
                    si += 1;
                    continue;
                }
                _ => {}
            }
        }
        match star {
            Some((re_pi, advanced_si)) => {
                pi = re_pi + 1;
                si = advanced_si + 1;
                star = Some((re_pi, advanced_si + 1));
            }
            None => return false,
        }
    }
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
}

/// Format the JSON listing as a deterministic Markdown table; shared by
/// the MCP tool and the CLI.
pub fn format_list_files_markdown(files: &serde_json::Value) -> String {
    let rows = files
        .get("files")
        .or_else(|| files.as_array().map(|_| files))
        .and_then(|v| v.as_array())
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    if rows.is_empty() {
        return "No indexed files matched the given path.".to_string();
    }
    let mut out = String::from("| Repository | File | Entities |\n|---|---|---:|\n");
    for row in rows {
        let repo = row.get("repo_name").and_then(|v| v.as_str()).unwrap_or("?");
        let file = row.get("file_path").and_then(|v| v.as_str()).unwrap_or("?");
        let count = row
            .get("entity_count")
            .and_then(|v| v.as_i64())
            .unwrap_or(0);
        out.push_str(&format!("| {repo} | {file} | {count} |\n"));
    }
    if rows
        .first()
        .and_then(|r| r.get("truncated"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        return "truncated".to_string();
    }
    out
}

/// Format the JSON listing for the requested CLI output format, shared
/// with the MCP tool's markdown rendering.
pub fn format_files_output(
    result: &serde_json::Value,
    output: crate::config::OutputFormat,
) -> String {
    use crate::config::OutputFormat;
    match output {
        OutputFormat::Json => serde_json::to_string_pretty(result).unwrap_or_default(),
        OutputFormat::Table | OutputFormat::Markdown => format_list_files_markdown(result),
    }
}

/// Run the file listing shared by `knot files` and the `list_files` MCP
/// tool. Returns `{"files": [...], "truncated": bool}` or `null` when
/// nothing matched.
pub async fn run_list_files(
    path: Option<&str>,
    repo: &RepoScope,
    graph_db: &Arc<GraphDb>,
) -> Result<serde_json::Value> {
    let cwd = std::env::current_dir().ok();
    let repo_root = std::env::var("KNOT_REPO_PATH").ok().map(PathBuf::from);
    let normalized = path
        .filter(|p| !p.trim().is_empty())
        .map(|p| normalize_list_path(p, cwd.as_deref(), repo_root.as_deref()))
        .unwrap_or_default();
    let repo_names = repo.filter_names();

    // Globs must scan the full path list (the graph side can only filter
    // by a literal `STARTS WITH`); plain prefixes fetch only the matching
    // subtree.
    let query_prefix = if normalized.contains('*') || normalized.contains('?') {
        String::new()
    } else {
        normalized.clone()
    };

    let raw = graph_db
        .list_files(&query_prefix, &repo_names, MAX_FILES)
        .await?;

    let filtered: Vec<serde_json::Value> = raw
        .as_array()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter(|row| {
            let candidate = row.get("file_path").and_then(|v| v.as_str()).unwrap_or("");
            path_matches(&normalized, candidate)
        })
        .collect();

    if filtered.is_empty() {
        return Ok(serde_json::Value::Null);
    }
    Ok(json!({
        "files": filtered,
        "truncated": raw.as_array().map(|a| a.len()).unwrap_or(0) >= MAX_FILES,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- normalize_list_path ---

    #[test]
    fn normalize_list_path_posix() {
        assert_eq!(normalize_list_path("src/api", None, None), "src/api");
        assert_eq!(normalize_list_path("./src/api", None, None), "src/api");
        assert_eq!(normalize_list_path(r"src\api", None, None), "src/api");
        assert_eq!(normalize_list_path("", None, None), "");
    }

    #[test]
    fn normalize_list_path_strips_local_root() {
        let root = Path::new("/home/dev/repo");
        assert_eq!(
            normalize_list_path("/home/dev/repo/src/api", Some(root), Some(root)),
            "src/api"
        );
    }

    // --- path_matches ---

    #[test]
    fn empty_prefix_matches_everything() {
        assert!(path_matches("", "src/main.rs"));
        assert!(path_matches(".", "Cargo.toml"));
    }

    #[test]
    fn boundary_prefix_respects_directory_edges() {
        assert!(path_matches("src/api", "src/api/users.ts"));
        assert!(path_matches("src/api", "src/api"));
        assert!(!path_matches("src/api", "src/api-notes.md"));
        assert!(!path_matches("src/api", "src/hooks/useThing.ts"));
    }

    // --- glob_match ---

    #[test]
    fn glob_star_is_segment_local() {
        assert!(glob_match("src/*.rs", "src/main.rs"));
        assert!(!glob_match("src/*.rs", "src/cli/main.rs"));
        assert!(glob_match("*/tests/*.ts", "hooks/tests/a.ts"));
    }

    #[test]
    fn glob_double_star_crosses_segments() {
        assert!(glob_match("src/**", "src/api/v1/users.ts"));
        assert!(glob_match("src/**/*.rs", "src/cli_tools/x/tests/a.rs"));
        // Zero segments also match.
        assert!(glob_match("src/**", "src"));
    }

    #[test]
    fn glob_question_mark_is_single_non_slash() {
        assert!(glob_match("a?.rs", "ab.rs"));
        assert!(!glob_match("a?.rs", "abc.rs"));
    }

    // --- format ---

    #[test]
    fn markdown_table_lists_repo_file_count() {
        let files = json!({
            "files": [
                {"repo_name": "r", "file_path": "src/a.rs", "entity_count": 3},
                {"repo_name": "r", "file_path": "src/b.rs", "entity_count": 1},
            ],
            "truncated": false,
        });
        let md = format_list_files_markdown(&files);
        assert!(md.contains("| Repository | File | Entities |"));
        assert!(md.contains("| r | src/a.rs | 3 |"));
        assert!(md.contains("| r | src/b.rs | 1 |"));
    }

    #[test]
    fn markdown_empty_list_hits_the_no_match_message() {
        let md = format_list_files_markdown(&json!([]));
        assert!(md.contains("No indexed files matched"));
    }
}
