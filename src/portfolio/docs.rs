use std::collections::HashMap;

use super::models::RepoAsset;

const DESCRIPTION_MAX: usize = 400;
const README_EXCERPT_MAX: usize = 1200;

#[derive(Default)]
struct DocAccumulator {
    description: String,
    readme_excerpt: String,
}

pub fn attach_documentation(assets: &mut [RepoAsset], rows: &[serde_json::Value]) {
    let mut by_repo: HashMap<String, DocAccumulator> = HashMap::new();

    for row in rows {
        let repo = row
            .get("repo_name")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        if repo.is_empty() {
            continue;
        }
        let kind = row.get("kind").and_then(|v| v.as_str()).unwrap_or("");
        let file_path = row.get("file_path").and_then(|v| v.as_str()).unwrap_or("");
        let embed = row.get("embed_text").and_then(|v| v.as_str()).unwrap_or("");
        let signature = row.get("signature").and_then(|v| v.as_str()).unwrap_or("");
        let docstring = row.get("docstring").and_then(|v| v.as_str()).unwrap_or("");
        let name = row.get("name").and_then(|v| v.as_str()).unwrap_or("");

        let acc = by_repo.entry(repo).or_default();

        if acc.description.is_empty() {
            if kind == "config_property" && !embed.is_empty() {
                acc.description = truncate_text(embed, DESCRIPTION_MAX);
            } else if kind == "cargo_package" && !embed.is_empty() {
                acc.description = truncate_text(embed, DESCRIPTION_MAX);
            } else if kind == "project_identity" && !signature.is_empty() {
                acc.description = truncate_text(signature, DESCRIPTION_MAX);
            } else if !docstring.is_empty() {
                acc.description = truncate_text(docstring, DESCRIPTION_MAX);
            }
        }

        if acc.readme_excerpt.is_empty()
            && file_path.to_lowercase().ends_with("readme.md")
            && !embed.is_empty()
        {
            acc.readme_excerpt = truncate_text(embed, README_EXCERPT_MAX);
        }

        // package.json description sometimes stored as name=description with embed = value
        if acc.description.is_empty()
            && kind == "config_property"
            && name.eq_ignore_ascii_case("description")
            && !embed.is_empty()
        {
            acc.description = truncate_text(embed, DESCRIPTION_MAX);
        }
    }

    for asset in assets {
        if let Some(doc) = by_repo.get(&asset.name) {
            asset.description = doc.description.clone();
            asset.readme_excerpt = doc.readme_excerpt.clone();
        }
        asset.identity = format_identity(&asset.group_id, &asset.artifact_id, &asset.version);
    }
}

fn format_identity(group_id: &str, artifact_id: &str, version: &str) -> String {
    match (
        group_id.is_empty(),
        artifact_id.is_empty(),
        version.is_empty(),
    ) {
        (false, false, false) => format!("{group_id}:{artifact_id}@{version}"),
        (false, false, true) => format!("{group_id}:{artifact_id}"),
        (true, false, false) => format!("{artifact_id}@{version}"),
        (true, false, true) => artifact_id.to_string(),
        _ => String::new(),
    }
}

pub fn truncate_text(text: &str, max: usize) -> String {
    let collapsed = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.chars().count() <= max {
        return collapsed;
    }
    let truncated: String = collapsed.chars().take(max).collect();
    format!("{truncated}…")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::portfolio::models::RepoRole;

    #[test]
    fn test_truncate_text_short_unchanged() {
        assert_eq!(truncate_text("hello world", 100), "hello world");
    }

    #[test]
    fn test_truncate_text_long_adds_ellipsis() {
        let long = "word ".repeat(100);
        let out = truncate_text(&long, 50);
        assert!(out.ends_with('…'));
        assert!(out.chars().count() <= 51);
    }

    #[test]
    fn test_attach_documentation_from_readme() {
        let rows = vec![serde_json::json!({
            "repo_name": "my-app",
            "kind": "markdown_section",
            "file_path": "README.md",
            "embed_text": "This is the project README intro.",
            "name": "Overview",
            "signature": "",
            "docstring": ""
        })];
        let mut assets = vec![RepoAsset {
            name: "my-app".to_string(),
            entity_count: 1,
            file_count: 1,
            build_system: String::new(),
            primary_language: String::new(),
            group_id: "com.example".to_string(),
            artifact_id: "my-app".to_string(),
            version: "1.0".to_string(),
            indexed_at: String::new(),
            role: RepoRole::Balanced,
            weight_pct: 100.0,
            dependency_count: 0,
            dependent_count: 0,
            description: String::new(),
            readme_excerpt: String::new(),
            identity: String::new(),
        }];
        attach_documentation(&mut assets, &rows);
        assert!(assets[0].readme_excerpt.contains("README intro"));
        assert_eq!(assets[0].identity, "com.example:my-app@1.0");
    }
}
