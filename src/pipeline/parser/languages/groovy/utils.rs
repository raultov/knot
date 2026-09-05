use std::borrow::Cow;
use std::iter::Peekable;
use std::str::CharIndices;

use crate::pipeline::parser::comments::strip_comment_markers;

fn finish(s: &str) -> Cow<'static, str> {
    let effective = s.trim();
    if effective.is_empty() {
        Cow::Owned(String::new())
    } else {
        Cow::Owned(effective.to_string())
    }
}

fn resume_block<'a>(trimmed: &'a str, in_block: &mut bool) -> Cow<'a, str> {
    if let Some(end_idx) = trimmed.find("*/") {
        *in_block = false;
        let rest = &trimmed[end_idx + 2..];
        finish(rest)
    } else {
        Cow::Owned(String::new())
    }
}

fn skip_block_comment(chars: &mut Peekable<CharIndices<'_>>) -> bool {
    while let Some((_, c2)) = chars.next() {
        if c2 == '*'
            && let Some(&(_, '/')) = chars.peek()
        {
            chars.next(); // consume '/'
            return true;
        }
    }
    false
}

fn copy_string_literal(quote: char, chars: &mut Peekable<CharIndices<'_>>, result: &mut String) {
    result.push(quote);
    while let Some((_, c2)) = chars.next() {
        result.push(c2);
        if c2 == '\\' {
            if let Some((_, esc)) = chars.next() {
                result.push(esc);
            }
        } else if c2 == quote {
            break;
        }
    }
}

/// Strips comment spans from a single source line, tracking multi-line
/// `/* … */` state across calls. Returns the code-bearing remainder.
///
/// The caller should count braces and inspect for declarations on the
/// returned effective line, *not* on the raw line — this is what prevents
/// Javadoc continuation lines from producing phantom entities and corrupting
/// scope tracking.
pub(super) fn strip_comments_line<'a>(line: &'a str, in_block: &mut bool) -> Cow<'a, str> {
    let trimmed = line.trim();
    if !*in_block && !trimmed.contains('/') && !trimmed.contains('*') {
        return Cow::Borrowed(trimmed);
    }

    if *in_block {
        return resume_block(trimmed, in_block);
    }

    let mut result = String::with_capacity(trimmed.len());
    let mut chars = trimmed.char_indices().peekable();

    while let Some((_i, c)) = chars.next() {
        if c == '/'
            && let Some(&(_, next)) = chars.peek()
        {
            if next == '/' {
                return finish(&result);
            }
            if next == '*' {
                chars.next(); // consume '*'
                let found_close = skip_block_comment(&mut chars);
                if !found_close {
                    *in_block = true;
                    return finish(&result);
                }
                continue;
            }
        }
        if c == '"' || c == '\'' {
            copy_string_literal(c, &mut chars, &mut result);
            continue;
        }
        result.push(c);
    }

    finish(&result)
}
/// Extract package name from source (e.g., `package com.example.service`)
// Reserved for future package resolution
pub(super) fn extract_package(source: &str) -> Option<String> {
    for line in source.lines().take(20) {
        let trimmed = line.trim();
        if let Some(pkg) = trimmed.strip_prefix("package ") {
            let name = pkg.trim().trim_end_matches(';').trim();
            if !name.is_empty() {
                return Some(name.to_string());
            }
        }
    }
    None
}
/// Build a fully-qualified name: package.parent.method or package.method
// Reserved for future FQN construction
pub(super) fn build_fqn(package: &Option<String>, parent: &Option<String>, name: &str) -> String {
    match (package, parent) {
        (Some(pkg), Some(enclosing_class)) => format!("{}.{}.{}", pkg, enclosing_class, name),
        (Some(pkg), None) => format!("{}.{}", pkg, name),
        (None, Some(enclosing_class)) => format!("{}.{}", enclosing_class, name),
        (None, None) => name.to_string(),
    }
}
/// Walks backwards from the line preceding `decl_line_idx` (0-based) collecting
/// the GroovyDoc / comment block that documents the declaration.
///
/// Policy (backwards walk from the declaration):
/// 1. Skip (without stopping the search): annotation lines (`@X`) and at most
///    one blank line — same tolerance as the generic tree-sitter extractor.
/// 2. Capture: an adjacent `/** ... */` / `/* ... */` block, or a burst of
///    consecutive `//` lines. Only the adjacent block is taken.
/// 3. Stop immediately (returning whatever was captured, or `None`) on any
///    other non-empty code line (`package`, `import`, statements) or at the
///    start of the file — this protects against license headers leaking into
///    the first class of a file.
/// 4. Markers (`/**`, `*/`, leading `*`, `//`) are stripped via
///    [`strip_comment_markers`]; an empty cleaned result maps to `None`.
pub(super) fn extract_preceding_docstring(lines: &[&str], decl_line_idx: usize) -> Option<String> {
    let non_empty = |cleaned: String| (!cleaned.trim().is_empty()).then_some(cleaned);

    // Phase 1: skip annotations and at most one blank line.
    let mut idx = decl_line_idx;
    let mut blank_seen = false;
    while idx > 0 {
        let prev = lines[idx - 1].trim();
        if prev.starts_with('@') {
            idx -= 1;
            continue;
        }
        if prev.is_empty() && !blank_seen {
            blank_seen = true;
            idx -= 1;
            continue;
        }
        break;
    }
    if idx == 0 {
        return None;
    }

    let last = lines[idx - 1].trim();

    // Case A: block comment — `/** ... */` or `/* ... */`.
    if last.ends_with("*/") {
        if last.starts_with("/*") {
            // Opener and closer on the same line (or this IS the opener line of
            // a block whose body sits above is impossible: the closer is here).
            return non_empty(strip_comment_markers(last));
        }
        if !last.starts_with('*') {
            // `code(); /* inline */` — trailing comment on a code line is not a
            // docstring.
            return None;
        }
        // Multi-line block: walk back through `*` continuation lines until the
        // `/*` opener.
        let mut block: Vec<&str> = vec![lines[idx - 1]];
        let mut j = idx - 1;
        while j > 0 {
            j -= 1;
            let t = lines[j].trim();
            if t.starts_with("/*") {
                block.push(lines[j]);
                block.reverse();
                return non_empty(strip_comment_markers(&block.join("\n")));
            }
            if t.starts_with('*') {
                block.push(lines[j]);
                continue;
            }
            // Non-comment line reached before the opener → malformed block.
            return None;
        }
        // Start of file reached without an opener → malformed block.
        return None;
    }

    // Case B: burst of consecutive `//` line comments.
    if last.starts_with("//") {
        let mut j = idx - 1;
        let mut burst: Vec<&str> = Vec::new();
        loop {
            if !lines[j].trim().starts_with("//") {
                break;
            }
            burst.push(lines[j]);
            if j == 0 {
                break;
            }
            j -= 1;
        }
        burst.reverse();
        return non_empty(strip_comment_markers(&burst.join("\n")));
    }

    None
}
/// Strips every balanced `<...>` section from `input`, preserving any characters
/// outside them. We use a manual depth counter instead of a regex so that nested
/// generics like `Map<List<X>, Y>` are erased in a single pass. This both
/// neutralizes generic bounds (`class Box<T extends Comparable>`) and discards
/// type arguments on the parent (`extends AbstractRepo<Order, Long>`).
pub(super) fn strip_balanced_generics(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut depth: i32 = 0;
    for ch in input.chars() {
        match ch {
            '<' => depth += 1,
            '>' if depth > 0 => depth -= 1,
            '>' => {} // unbalanced '>' — drop silently (defensive)
            _ if depth == 0 => out.push(ch),
            _ => {} // skip chars inside generics
        }
    }
    out
}
/// Validates a single parent/interface name token: must start with an
/// alphabetic character, may contain alphanumerics, underscores and dots.
pub(super) fn is_valid_type_name(s: &str) -> bool {
    if s.is_empty() {
        return false;
    }
    let mut chars = s.chars();
    let first = chars.next().unwrap();
    if !first.is_alphabetic() && first != '_' {
        return false;
    }
    chars.all(|c| c.is_alphanumeric() || c == '_' || c == '.')
}
/// Returns true when `s` is a Groovy/Java identifier.
pub(super) fn is_valid_identifier(s: &str) -> bool {
    if s.is_empty() {
        return false;
    }
    let mut chars = s.chars();
    let first = chars.next().unwrap();
    if !first.is_alphabetic() && first != '_' {
        return false;
    }
    chars.all(|c| c.is_alphanumeric() || c == '_')
}
