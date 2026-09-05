use super::utils::{is_valid_identifier, is_valid_type_name, strip_balanced_generics};

/// Metadata for a Groovy property declaration, carried forward into accessor
/// synthesis so the synthetic entity can inherit the declared type and `final`
/// flag.
#[derive(Debug, Clone)]
pub(super) struct GroovyPropertyDecl {
    pub(super) name: String,
    pub(super) declared_type: Option<String>,
    pub(super) is_final: bool,
}
/// Tries to extract properties (fields, script variables) from a single line.
///
/// Recognizes both:
/// - Initialized: `String name = 'test'`, `def count = 0`
/// - Bare (no initializer): `Path baseDir`, `private static final Path ROOT`
///
/// The caller gates extraction via `scope_stack` depth so method-body locals
/// are never promoted to properties.
pub(super) fn try_extract_property(line: &str) -> Option<GroovyPropertyDecl> {
    let cleaned = strip_leading_annotations(line);

    if cleaned.is_empty() {
        return None;
    }

    // Reject pure comments (shouldn't happen after strip_comments_line, but defensive)
    if cleaned.starts_with("//") || cleaned.starts_with("/*") || cleaned.starts_with('*') {
        return None;
    }

    if let Some(eq_idx) = cleaned.find('=') {
        try_extract_initialized_property(&cleaned, eq_idx)
    } else {
        try_extract_bare_property(&cleaned)
    }
}

fn strip_leading_annotations(line: &str) -> String {
    let mut cleaned = line.trim().trim_end_matches(';').trim().to_string();

    // Strip leading annotations (@Lazy, @PackageScope, @Deprecated, ...)
    loop {
        let trimmed = cleaned.trim_start();
        if let Some(rest) = trimmed.strip_prefix('@') {
            let end = rest.find(|c: char| c.is_whitespace()).unwrap_or(rest.len());
            cleaned = rest[end..].trim().to_string();
        } else {
            break;
        }
    }
    cleaned
}

fn try_extract_initialized_property(cleaned: &str, eq_idx: usize) -> Option<GroovyPropertyDecl> {
    // Discard `==`, `!=`
    if cleaned.chars().nth(eq_idx + 1) == Some('=') {
        return None;
    }
    let left_side = cleaned[..eq_idx].trim();
    if left_side.is_empty() {
        return None;
    }

    let tokens: Vec<&str> = left_side.split_whitespace().collect();
    if tokens.len() < 2 {
        return None;
    }

    let name = tokens.last().unwrap();
    if !is_valid_identifier(name) {
        return None;
    }

    let first_token = tokens[0];
    let declared_type = if first_token == "def" {
        if tokens.len() >= 2 {
            Some(tokens[tokens.len() - 2].to_string())
        } else {
            None
        }
    } else if is_valid_type_name(first_token) {
        Some(first_token.to_string())
    } else {
        tokens
            .iter()
            .find(|t| is_valid_type_name(t))
            .map(|t| t.to_string())
    };
    let is_final = tokens.contains(&"final");
    Some(GroovyPropertyDecl {
        name: name.to_string(),
        declared_type,
        is_final,
    })
}

fn try_extract_bare_property(cleaned: &str) -> Option<GroovyPropertyDecl> {
    // Reject keywords that can appear as the first token
    let rejection_keywords = [
        "return",
        "import",
        "package",
        "class",
        "interface",
        "trait",
        "enum",
        "new",
        "throw",
        "assert",
        "case",
        "else",
        "extends",
        "implements",
        "instanceof",
    ];

    if cleaned.contains('(')
        || cleaned.contains(')')
        || cleaned.contains('{')
        || cleaned.contains('}')
    {
        return None;
    }
    let tokens: Vec<&str> = cleaned.split_whitespace().collect();
    if tokens.len() < 2 {
        return None;
    }

    // Reject if the first significant token is a keyword
    let first_token = tokens[0];
    if rejection_keywords.contains(&first_token) {
        return None;
    }

    // Remove modifier tokens
    let modifiers: &[&str] = &[
        "private",
        "protected",
        "public",
        "static",
        "final",
        "transient",
        "volatile",
        "synchronized",
        "abstract",
        "native",
    ];
    let non_modifiers: Vec<&&str> = tokens.iter().filter(|t| !modifiers.contains(t)).collect();

    if non_modifiers.len() < 2 {
        return None;
    }

    let name = tokens.last().unwrap();
    if !is_valid_identifier(name) {
        return None;
    }

    let type_token = find_bare_property_type(&tokens, modifiers)?;

    let is_final = tokens.contains(&"final");
    Some(GroovyPropertyDecl {
        name: name.to_string(),
        declared_type: Some(type_token),
        is_final,
    })
}

fn find_bare_property_type(tokens: &[&str], modifiers: &[&str]) -> Option<String> {
    if tokens.len() < 2 {
        return None;
    }
    let candidate = tokens[tokens.len() - 2];
    let candidate_stripped = strip_balanced_generics(candidate);
    if candidate == "def" || is_valid_type_name(&candidate_stripped) {
        Some(candidate.to_string())
    } else if modifiers.contains(&candidate) {
        // e.g., `private final String name` — search backwards
        tokens[..tokens.len() - 1]
            .iter()
            .rev()
            .find(|t| {
                !modifiers.contains(t)
                    && **t != "def"
                    && is_valid_type_name(&strip_balanced_generics(t))
            })
            .map(|t| t.to_string())
    } else {
        None
    }
}
