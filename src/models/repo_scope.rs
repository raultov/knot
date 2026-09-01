//! Repository scope selection model for filtering queries across indexed repositories.

/// Repository scope selected by a tool caller or CLI user.
///
/// - `All`: No repository filter applied (queries span every indexed repository).
/// - `One(String)`: Exactly one repository is targeted.
/// - `Many(Vec<String>)`: Union of the listed repositories (OR semantics).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RepoScope {
    /// Query all indexed repositories without filtering.
    All,
    /// Query exactly one specified repository.
    One(String),
    /// Query a union of multiple specified repositories.
    Many(Vec<String>),
}

impl RepoScope {
    /// Parse a raw parameter string into a `RepoScope`.
    ///
    /// Normative parse rules (in order):
    /// 1. Trim whole input.
    /// 2. Split on `,`.
    /// 3. Trim each token; drop empty tokens.
    /// 4. If ANY remaining token equals `all` (case-insensitive) or is exactly `*` -> `All`.
    /// 5. If no tokens remain -> `All`.
    /// 6. Deduplicate tokens preserving first-occurrence order.
    /// 7. 1 token = `One(token)`, more = `Many(tokens)`.
    pub fn parse(raw: &str) -> Self {
        let trimmed_input = raw.trim();
        let tokens: Vec<&str> = trimmed_input
            .split(',')
            .map(|t| t.trim())
            .filter(|t| !t.is_empty())
            .collect();

        if tokens.is_empty() {
            return Self::All;
        }

        for token in &tokens {
            if token.eq_ignore_ascii_case("all") || *token == "*" {
                return Self::All;
            }
        }

        let mut deduped: Vec<String> = Vec::with_capacity(tokens.len());
        for token in tokens {
            let token_str = token.to_string();
            if !deduped.contains(&token_str) {
                deduped.push(token_str);
            }
        }

        match deduped.len() {
            0 => Self::All,
            1 => Self::One(deduped.remove(0)),
            _ => Self::Many(deduped),
        }
    }

    /// Parse an optional parameter. `None` maps to `All`.
    ///
    /// Note: CLI callers resolve their `cfg.repo_name` default before calling this.
    pub fn parse_optional(raw: Option<&str>) -> Self {
        match raw {
            Some(s) => Self::parse(s),
            None => Self::All,
        }
    }

    /// Build a `RepoScope` from a JSON argument value.
    ///
    /// Accepts:
    /// - `None` or `null` -> `All`.
    /// - JSON string -> parsed via `RepoScope::parse`.
    /// - JSON array of strings -> tokens joined in order then parsed (non-string items are skipped).
    pub fn from_json(value: Option<&serde_json::Value>) -> Self {
        match value {
            None | Some(serde_json::Value::Null) => Self::All,
            Some(serde_json::Value::String(s)) => Self::parse(s),
            Some(serde_json::Value::Array(arr)) => {
                let string_items: Vec<&str> = arr.iter().filter_map(|v| v.as_str()).collect();
                let joined = string_items.join(",");
                Self::parse(&joined)
            }
            Some(_) => Self::All,
        }
    }

    /// Return the list of repository names to filter by.
    ///
    /// Returns:
    /// - `All`: `vec![]` (empty list indicates no DB filter).
    /// - `One(n)`: `vec![n]`.
    /// - `Many(v)`: `v`.
    pub fn filter_names(&self) -> Vec<String> {
        match self {
            Self::All => vec![],
            Self::One(name) => vec![name.clone()],
            Self::Many(names) => names.clone(),
        }
    }

    /// Return `true` if no filter should be applied at the database layer (i.e. `All`).
    pub fn is_unfiltered(&self) -> bool {
        matches!(self, Self::All)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parse_empty_is_all() {
        assert_eq!(RepoScope::parse(""), RepoScope::All);
    }

    #[test]
    fn parse_whitespace_only_is_all() {
        assert_eq!(RepoScope::parse("   "), RepoScope::All);
    }

    #[test]
    fn parse_all_sentinel() {
        assert_eq!(RepoScope::parse("all"), RepoScope::All);
    }

    #[test]
    fn parse_all_sentinel_case_insensitive() {
        assert_eq!(RepoScope::parse("ALL"), RepoScope::All);
        assert_eq!(RepoScope::parse("All"), RepoScope::All);
    }

    #[test]
    fn parse_star_sentinel() {
        assert_eq!(RepoScope::parse("*"), RepoScope::All);
    }

    #[test]
    fn parse_star_wins_over_list() {
        assert_eq!(RepoScope::parse("a,*"), RepoScope::All);
    }

    #[test]
    fn parse_star_is_not_a_glob() {
        assert_eq!(
            RepoScope::parse("scope-*"),
            RepoScope::One("scope-*".to_string())
        );
    }

    #[test]
    fn parse_all_wins_over_list() {
        assert_eq!(RepoScope::parse("all,repo-a"), RepoScope::All);
    }

    #[test]
    fn parse_single_repo() {
        assert_eq!(
            RepoScope::parse("my-repo"),
            RepoScope::One("my-repo".to_string())
        );
    }

    #[test]
    fn parse_multi_repo() {
        assert_eq!(
            RepoScope::parse("a,b,c"),
            RepoScope::Many(vec!["a".to_string(), "b".to_string(), "c".to_string()])
        );
    }

    #[test]
    fn parse_multi_trims_tokens() {
        assert_eq!(
            RepoScope::parse(" a , b "),
            RepoScope::Many(vec!["a".to_string(), "b".to_string()])
        );
    }

    #[test]
    fn parse_multi_drops_empty_tokens() {
        assert_eq!(
            RepoScope::parse("a,,b,"),
            RepoScope::Many(vec!["a".to_string(), "b".to_string()])
        );
    }

    #[test]
    fn parse_multi_dedups_preserving_order() {
        assert_eq!(
            RepoScope::parse("b,a,b"),
            RepoScope::Many(vec!["b".to_string(), "a".to_string()])
        );
    }

    #[test]
    fn parse_preserves_repo_case() {
        assert_eq!(
            RepoScope::parse("MyRepo"),
            RepoScope::One("MyRepo".to_string())
        );
    }

    #[test]
    fn parse_optional_none_is_all() {
        assert_eq!(RepoScope::parse_optional(None), RepoScope::All);
    }

    #[test]
    fn parse_optional_some_parses() {
        assert_eq!(
            RepoScope::parse_optional(Some("a,b")),
            RepoScope::Many(vec!["a".to_string(), "b".to_string()])
        );
    }

    #[test]
    fn from_json_string() {
        let val = json!("a,b");
        assert_eq!(
            RepoScope::from_json(Some(&val)),
            RepoScope::Many(vec!["a".to_string(), "b".to_string()])
        );
    }

    #[test]
    fn from_json_array() {
        let val = json!(["a", "b"]);
        assert_eq!(
            RepoScope::from_json(Some(&val)),
            RepoScope::Many(vec!["a".to_string(), "b".to_string()])
        );
    }

    #[test]
    fn from_json_absent_is_all() {
        assert_eq!(RepoScope::from_json(None), RepoScope::All);
        let null_val = json!(null);
        assert_eq!(RepoScope::from_json(Some(&null_val)), RepoScope::All);
    }

    #[test]
    fn from_json_non_string_items_skipped() {
        let val = json!(["a", 42]);
        assert_eq!(
            RepoScope::from_json(Some(&val)),
            RepoScope::One("a".to_string())
        );
    }

    #[test]
    fn filter_names_empty_for_all() {
        assert!(RepoScope::All.filter_names().is_empty());
    }

    #[test]
    fn filter_names_one_and_many() {
        assert_eq!(
            RepoScope::One("repo-a".to_string()).filter_names(),
            vec!["repo-a".to_string()]
        );
        assert_eq!(
            RepoScope::Many(vec!["repo-a".to_string(), "repo-b".to_string()]).filter_names(),
            vec!["repo-a".to_string(), "repo-b".to_string()]
        );
    }

    #[test]
    fn is_unfiltered_only_for_all() {
        assert!(RepoScope::All.is_unfiltered());
        assert!(!RepoScope::One("repo-a".to_string()).is_unfiltered());
        assert!(!RepoScope::Many(vec!["repo-a".to_string()]).is_unfiltered());
    }
}
