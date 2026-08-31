//! Apply include/exclude filters to raw portfolio data.

use super::collect::RawPortfolioData;

/// Default repos excluded from portfolio analysis (third-party / noise).
pub const DEFAULT_EXCLUDED_REPOS: &[&str] = &["prowler"];

pub fn effective_exclude_list(cli_exclude: &[String]) -> Vec<String> {
    let mut names: Vec<String> = DEFAULT_EXCLUDED_REPOS
        .iter()
        .map(|s| (*s).to_string())
        .collect();
    if let Ok(env) = std::env::var("KNOT_PORTFOLIO_EXCLUDE") {
        for part in env.split(',') {
            let part = part.trim();
            if !part.is_empty() && !names.iter().any(|n| n.eq_ignore_ascii_case(part)) {
                names.push(part.to_string());
            }
        }
    }
    for e in cli_exclude {
        let e = e.trim();
        if !e.is_empty() && !names.iter().any(|n| n.eq_ignore_ascii_case(e)) {
            names.push(e.to_string());
        }
    }
    names
}

pub fn is_repo_excluded(name: &str, exclude: &[String]) -> bool {
    let lower = name.to_lowercase();
    exclude
        .iter()
        .any(|e| lower == e.to_lowercase() || lower.contains(&e.to_lowercase()))
}

pub fn apply_exclusions(raw: &mut RawPortfolioData, exclude: &[String]) {
    if exclude.is_empty() {
        return;
    }
    raw.repos.retain(|repo| {
        repo.get("name")
            .and_then(|v| v.as_str())
            .is_none_or(|name| !is_repo_excluded(name, exclude))
    });
    raw.dep_edges
        .retain(|(from, to)| !is_repo_excluded(from, exclude) && !is_repo_excluded(to, exclude));
    raw.call_coupling
        .retain(|(from, to, _)| !is_repo_excluded(from, exclude) && !is_repo_excluded(to, exclude));
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_default_excludes_prowler() {
        let list = effective_exclude_list(&[]);
        assert!(list.iter().any(|n| n.eq_ignore_ascii_case("prowler")));
    }

    #[test]
    fn test_apply_exclusions_removes_prowler() {
        let mut raw = RawPortfolioData {
            repos: vec![json!({"name": "Synthapse"}), json!({"name": "prowler"})],
            dep_edges: vec![("prowler".into(), "other".into())],
            call_coupling: vec![("a".into(), "prowler".into(), 5)],
        };
        apply_exclusions(&mut raw, &["prowler".to_string()]);
        assert_eq!(raw.repos.len(), 1);
        assert!(raw.dep_edges.is_empty());
        assert!(raw.call_coupling.is_empty());
    }
}
