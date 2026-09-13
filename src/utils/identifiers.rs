//! Identifier tokenization shared by the indexing pipeline and the search
//! ranker.
//!
//! Both sides must slice identifiers the same way: the pipeline embeds the
//! token form (`useChangePassword` → `use change password`) in the embed
//! text and the Qdrant payload, and the search ranker splits query-side
//! identifiers with the same boundaries so lexical boosts agree on what a
//! "token" is.

/// Split an identifier into lowercase tokens at snake_case, kebab-case and
/// camelCase boundaries.
///
/// `link_cross_repo_dependencies` → `link, cross, repo, dependencies`;
/// `similaritySearch` → `similarity, search`; `useChangePassword` →
/// `use, change, password`.
pub fn identifier_tokens(name: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    for part in name.split(['_', '-', ' ', '.', ':', '/']) {
        let chars: Vec<char> = part.chars().collect();
        let mut current = String::new();
        for (i, &c) in chars.iter().enumerate() {
            let boundary = i > 0
                && (c.is_uppercase() && !chars[i - 1].is_uppercase()
                    || c.is_uppercase()
                        && chars[i - 1].is_uppercase()
                        && chars.get(i + 1).is_some_and(|next| next.is_lowercase()));
            if boundary {
                tokens.push(current.to_lowercase());
                current.clear();
            }
            current.push(c);
        }
        if !current.is_empty() {
            tokens.push(current.to_lowercase());
        }
    }
    tokens
}

/// Space-joined token form of an identifier (`useChangePassword` →
/// "use change password"). Single-token identifiers round-trip unchanged
/// apart from casing.
pub fn identifier_token_phrase(name: &str) -> String {
    identifier_tokens(name).join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokenizes_camel_case() {
        assert_eq!(
            identifier_tokens("useChangePassword"),
            vec!["use", "change", "password"]
        );
        assert_eq!(
            identifier_token_phrase("similaritySearch"),
            "similarity search"
        );
    }

    #[test]
    fn tokenizes_snake_case() {
        assert_eq!(
            identifier_tokens("verify_credentials_or_fail"),
            vec!["verify", "credentials", "or", "fail"]
        );
    }

    #[test]
    fn tokenizes_qualified_names() {
        // Dots and path separators act as boundaries too, so FQN/path
        // fragments tokenize into their components.
        assert_eq!(
            identifier_tokens("auth::credentials::verify_password"),
            vec!["auth", "credentials", "verify", "password"]
        );
        assert_eq!(
            identifier_tokens("src/api/auth.rs"),
            vec!["src", "api", "auth", "rs"]
        );
    }

    #[test]
    fn single_token_roundtrips() {
        assert_eq!(identifier_token_phrase("login"), "login");
        assert!(identifier_token_phrase("").is_empty());
    }

    #[test]
    fn upper_acronyms_stay_one_token() {
        // Trailing-uppercase handling keeps an opening caps run together
        // while splitting on the lowercase transition (`HTTPServer` →
        // http, server).
        assert_eq!(identifier_tokens("HTTPServer"), vec!["http", "server"]);
        assert_eq!(identifier_tokens("ALL_CAPS"), vec!["all", "caps"]);
    }
}
