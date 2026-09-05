use crate::models::{ParsedEntity, ReferenceIntent};

/// Scans source for method call patterns and returns reference intents.
// Reserved for future reference extraction
#[expect(
    clippy::cognitive_complexity,
    reason = "function is verbose but correct — extraction deferred"
)]
pub(super) fn extract_method_calls(
    source: &str,
    _entities: &[ParsedEntity],
) -> Vec<ReferenceIntent> {
    let mut refs = Vec::new();
    let keywords = [
        "if",
        "else",
        "while",
        "for",
        "return",
        "new",
        "throw",
        "catch",
        "switch",
        "case",
        "import",
        "package",
        "class",
        "interface",
        "trait",
        "enum",
        "def",
        "try",
        "finally",
        "assert",
        "println",
        "void",
        "int",
        "String",
        "boolean",
        "double",
        "float",
        "long",
        "byte",
        "short",
        "char",
        "public",
        "private",
        "protected",
        "static",
        "final",
        "abstract",
        "synchronized",
        "volatile",
        "transient",
    ];

    for (line_idx, line) in source.lines().enumerate() {
        let line_num = line_idx + 1;
        let trimmed = line.trim();

        if trimmed.starts_with("//")
            || trimmed.starts_with("/*")
            || trimmed.starts_with("*")
            || trimmed.starts_with("package ")
            || trimmed.starts_with("import ")
        {
            continue;
        }

        let mut chars = trimmed.char_indices().peekable();
        while let Some((i, c)) = chars.next() {
            // Skip string literals to avoid false positives
            if c == '\"' || c == '\'' {
                while let Some((_, nc)) = chars.next() {
                    if nc == c {
                        break; // closing quote found
                    }
                    if nc == '\\' {
                        let _ = chars.next(); // skip escaped char
                    }
                }
                continue;
            }
            if !c.is_alphabetic() && c != '_' {
                continue;
            }

            let word_start = i;
            let mut word_end = i;
            while let Some((_, nc)) = chars.peek() {
                if nc.is_alphanumeric() || *nc == '_' {
                    word_end = chars.next().unwrap().0;
                } else {
                    break;
                }
            }

            let word = &trimmed[word_start..=word_end];
            if keywords.contains(&word) {
                continue;
            }

            let after_word = &trimmed[word_end + 1..];
            let after_trimmed = after_word.trim_start();

            // Pattern: word.word(...)
            if let Some(dot_rest) = after_trimmed.strip_prefix('.') {
                let dot_trimmed = dot_rest.trim_start();
                if let Some((next_word, rest)) = split_identifier(dot_trimmed) {
                    let after_next = rest.trim_start();
                    if after_next.starts_with('(') {
                        refs.push(ReferenceIntent::Call {
                            method: next_word.to_string(),
                            receiver: Some(word.to_string()),
                            line: line_num,
                            arg_count: None,
                        });
                        continue;
                    }
                }
            }

            // Pattern: word(...)
            if after_trimmed.starts_with('(') && !keywords.contains(&word) && word.len() > 1 {
                refs.push(ReferenceIntent::Call {
                    method: word.to_string(),
                    receiver: None,
                    line: line_num,
                    arg_count: None,
                });
            }

            // Pattern: no-paren call — word followed by string literal or identifier args
            // Groovy style: runAnalyzer "abc", 123 or doSomething arg1, arg2
            if !after_trimmed.is_empty()
                && !keywords.contains(&word)
                && word.len() > 1
                && !after_trimmed.starts_with('(')
                && !after_trimmed.starts_with('.')
                && !after_trimmed.starts_with('=')
                && !after_trimmed.starts_with('{')
                && !after_trimmed.starts_with(')')
                && !after_trimmed.starts_with(':')
                && !after_trimmed.starts_with(';')
            {
                let first_arg_char = after_trimmed.chars().next().unwrap();
                // Argument must start with string quote or identifier char
                if first_arg_char == '"'
                    || first_arg_char == '\''
                    || first_arg_char.is_alphabetic()
                    || first_arg_char == '$'
                {
                    refs.push(ReferenceIntent::Call {
                        method: word.to_string(),
                        receiver: None,
                        line: line_num,
                        arg_count: None,
                    });
                }
            }
        }
    }
    refs
}
/// Splits an identifier from the start of `s`, returns (identifier, rest).
// Reserved for future FQN construction
fn split_identifier(s: &str) -> Option<(&str, &str)> {
    let first = s.chars().next()?;
    if !first.is_alphabetic() && first != '_' {
        return None;
    }
    let end = s
        .find(|c: char| !c.is_alphanumeric() && c != '_')
        .unwrap_or(s.len());
    Some((&s[..end], &s[end..]))
}
