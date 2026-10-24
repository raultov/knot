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

/// Number of arguments of a parenthesized call, given the text from its `(`
/// to the end of the line. Commas inside strings, closures, lists, maps and
/// nested calls do not separate arguments, and a trailing closure
/// (`each(1) { ... }`) is one more argument.
///
/// `None` whenever the count is uncertain: named arguments (Groovy collects
/// them into a single `Map` parameter), a spread (`*parts`), or an argument
/// list that does not close on this line.
fn argument_count(text: &str) -> Option<usize> {
    let body = text.strip_prefix('(')?;
    let mut depth = 0usize;
    let mut args: Vec<&str> = Vec::new();
    let mut arg_start = 0;
    let mut chars = body.char_indices().peekable();
    let mut close = None;
    while let Some((i, c)) = chars.next() {
        match c {
            '"' | '\'' => skip_string_literal(&mut chars, c),
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' if depth > 0 => depth -= 1,
            ')' => {
                close = Some(i);
                break;
            }
            ',' if depth == 0 => {
                args.push(&body[arg_start..i]);
                arg_start = i + 1;
            }
            _ => {}
        }
    }
    let close = close?;
    let last = &body[arg_start..close];
    if !(args.is_empty() && last.trim().is_empty()) {
        args.push(last);
    }
    if args.iter().any(|arg| is_named_or_spread_argument(arg)) {
        return None;
    }
    let trailing_closure = body[close + 1..].trim_start().starts_with('{');
    Some(args.len() + usize::from(trailing_closure))
}

/// `name: value` / `'name': value` (a named argument) or `*values` (a spread).
fn is_named_or_spread_argument(arg: &str) -> bool {
    let arg = arg.trim();
    if arg.starts_with('*') {
        return true;
    }
    let key_end = match arg.chars().next() {
        Some(q @ ('"' | '\'')) => arg[1..].find(q).map(|i| i + 2),
        Some(c) if c.is_alphabetic() || c == '_' => Some(
            arg.find(|c: char| !c.is_alphanumeric() && c != '_')
                .unwrap_or(arg.len()),
        ),
        _ => None,
    };
    key_end.is_some_and(|end| {
        let rest = arg[end..].trim_start();
        rest.starts_with(':') && !rest.starts_with("::")
    })
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
            arg_count: argument_count(after_next),
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
            arg_count: argument_count(after_trimmed),
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
