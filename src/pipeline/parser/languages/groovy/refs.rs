use crate::models::{ParsedEntity, ReferenceIntent};

/// Scans source for method call patterns and returns reference intents.
// Reserved for future reference extraction
use std::iter::Peekable;
use std::str::CharIndices;

const GROOVY_KEYWORDS: [&str; 40] = [
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

fn skip_string_literal(chars: &mut Peekable<CharIndices<'_>>, quote: char) {
    while let Some((_, nc)) = chars.next() {
        if nc == quote {
            break;
        }
        if nc == '\\' {
            let _ = chars.next();
        }
    }
}

fn read_word<'a>(
    trimmed: &'a str,
    start: usize,
    first_char: char,
    chars: &mut Peekable<CharIndices<'a>>,
) -> (&'a str, &'a str) {
    let mut end = start + first_char.len_utf8();
    while let Some(&(i, nc)) = chars.peek() {
        if nc.is_alphanumeric() || nc == '_' {
            end = i + nc.len_utf8();
            chars.next();
        } else {
            break;
        }
    }
    let word = &trimmed[start..end];
    let after_word = &trimmed[end..];
    (word, after_word)
}

fn receiver_call(word: &str, after_trimmed: &str, line_num: usize) -> Option<ReferenceIntent> {
    let dot_rest = after_trimmed.strip_prefix('.')?;
    let dot_trimmed = dot_rest.trim_start();
    let (next_word, rest) = split_identifier(dot_trimmed)?;
    let after_next = rest.trim_start();
    if after_next.starts_with('(') {
        Some(ReferenceIntent::Call {
            method: next_word.to_string(),
            receiver: Some(word.to_string()),
            line: line_num,
            arg_count: None,
        })
    } else {
        None
    }
}

fn bare_call(word: &str, after_trimmed: &str, line_num: usize) -> Option<ReferenceIntent> {
    if GROOVY_KEYWORDS.contains(&word) || word.len() <= 1 {
        return None;
    }

    if after_trimmed.starts_with('(') {
        return Some(ReferenceIntent::Call {
            method: word.to_string(),
            receiver: None,
            line: line_num,
            arg_count: None,
        });
    }

    if !after_trimmed.is_empty()
        && !after_trimmed.starts_with('.')
        && !after_trimmed.starts_with('=')
        && !after_trimmed.starts_with('{')
        && !after_trimmed.starts_with(')')
        && !after_trimmed.starts_with(':')
        && !after_trimmed.starts_with(';')
    {
        let first_arg_char = after_trimmed.chars().next()?;
        if first_arg_char == '"'
            || first_arg_char == '\''
            || first_arg_char.is_alphabetic()
            || first_arg_char == '$'
        {
            return Some(ReferenceIntent::Call {
                method: word.to_string(),
                receiver: None,
                line: line_num,
                arg_count: None,
            });
        }
    }

    None
}

fn scan_line_for_calls(trimmed: &str, line_num: usize, refs: &mut Vec<ReferenceIntent>) {
    let mut chars = trimmed.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        if c == '"' || c == '\'' {
            skip_string_literal(&mut chars, c);
            continue;
        }
        if !c.is_alphabetic() && c != '_' {
            continue;
        }

        let (word, after_word) = read_word(trimmed, i, c, &mut chars);
        if GROOVY_KEYWORDS.contains(&word) {
            continue;
        }

        let after_trimmed = after_word.trim_start();
        if let Some(r) = receiver_call(word, after_trimmed, line_num) {
            refs.push(r);
        } else if let Some(r) = bare_call(word, after_trimmed, line_num) {
            refs.push(r);
        }
    }
}

/// Scans source for method call patterns and returns reference intents.
// Reserved for future reference extraction
pub(super) fn extract_method_calls(
    source: &str,
    _entities: &[ParsedEntity],
) -> Vec<ReferenceIntent> {
    let mut refs = Vec::new();

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

        scan_line_for_calls(trimmed, line_num, &mut refs);
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
