//! Text-level reading of Java/Kotlin annotations and the compile-time
//! string expressions they carry: literals, `+` concatenation, arrays
//! (`{..}`, `[..]`, `arrayOf(..)`), constant names and enum values.

use std::collections::HashMap;

/// Compile-time string constants: `NAME` and `Owner.NAME` → value.
pub(crate) type StringConstants = HashMap<String, String>;

/// One annotation: its simple name and its arguments, each with its
/// attribute name when written `name = value`.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Annotation {
    pub name: String,
    pub args: Vec<(Option<String>, String)>,
}

impl Annotation {
    /// The raw expression of attribute `key`; the positional argument
    /// stands for `value`.
    pub fn arg(&self, keys: &[&str]) -> Option<&str> {
        self.args
            .iter()
            .find(|(key, _)| match key {
                Some(key) => keys.contains(&key.as_str()),
                None => keys.contains(&"value"),
            })
            .map(|(_, expr)| expr.as_str())
    }
}

/// `@Name`, `@pkg.Name(args)` → [`Annotation`]; `None` for anything else.
pub(crate) fn parse_annotation(text: &str) -> Option<Annotation> {
    let body = text.trim().strip_prefix('@')?;
    let name_end = body
        .find(|c: char| !(c.is_alphanumeric() || c == '_' || c == '.' || c == '$'))
        .unwrap_or(body.len());
    let name = body[..name_end].rsplit('.').next()?.to_string();
    if name.is_empty() {
        return None;
    }
    let rest = body[name_end..].trim_start();
    let args = match rest.strip_prefix('(') {
        Some(inner) => {
            let close = closing_paren(inner)?;
            split_top_level(&inner[..close], ',')
                .into_iter()
                .filter(|arg| !arg.trim().is_empty())
                .map(named_argument)
                .collect()
        }
        None => Vec::new(),
    };
    Some(Annotation { name, args })
}

/// `key = expr` → (`Some(key)`, `expr`); anything else is positional.
fn named_argument(arg: &str) -> (Option<String>, String) {
    let arg = arg.trim();
    let key_end = arg
        .find(|c: char| !(c.is_alphanumeric() || c == '_'))
        .unwrap_or(arg.len());
    let rest = arg[key_end..].trim_start();
    if key_end > 0 && rest.starts_with('=') && !rest.starts_with("==") {
        return (
            Some(arg[..key_end].to_string()),
            rest[1..].trim().to_string(),
        );
    }
    (None, arg.to_string())
}

/// Index of the `)` closing the parenthesis `text` follows.
fn closing_paren(text: &str) -> Option<usize> {
    let mut depth = 0usize;
    let mut chars = text.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        match c {
            '"' | '\'' => skip_literal(text, i, &mut chars),
            '(' | '[' | '{' => depth += 1,
            ')' if depth == 0 => return Some(i),
            ')' | ']' | '}' => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    None
}

/// Advance `chars` past the string or char literal opening at `start`.
fn skip_literal(
    text: &str,
    start: usize,
    chars: &mut std::iter::Peekable<std::str::CharIndices<'_>>,
) {
    let Some(quote) = text[start..].chars().next() else {
        return;
    };
    if quote == '"' && text[start..].starts_with("\"\"\"") {
        chars.next();
        chars.next();
        while let Some((i, _)) = chars.next() {
            if text[i..].starts_with("\"\"\"") {
                chars.next();
                chars.next();
                return;
            }
        }
        return;
    }
    while let Some((_, c)) = chars.next() {
        match c {
            '\\' => {
                chars.next();
            }
            c if c == quote => return,
            _ => {}
        }
    }
}

/// Split `text` on `sep` outside literals, parentheses, brackets and braces.
fn split_top_level(text: &str, sep: char) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut depth = 0usize;
    let mut start = 0;
    let mut chars = text.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        match c {
            '"' | '\'' => skip_literal(text, i, &mut chars),
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth = depth.saturating_sub(1),
            c if c == sep && depth == 0 => {
                parts.push(&text[start..i]);
                start = i + c.len_utf8();
            }
            _ => {}
        }
    }
    parts.push(&text[start..]);
    parts
}

/// The elements of an array expression (`{a, b}`, `[a, b]`,
/// `arrayOf(a, b)`), or the expression itself when it is not an array.
fn array_elements(expr: &str) -> Vec<&str> {
    let expr = expr.trim();
    let inner = expr
        .strip_prefix('{')
        .and_then(|e| e.strip_suffix('}'))
        .or_else(|| expr.strip_prefix('[').and_then(|e| e.strip_suffix(']')))
        .or_else(|| {
            expr.strip_prefix("arrayOf(")
                .and_then(|e| e.strip_suffix(')'))
        });
    match inner {
        Some(inner) => split_top_level(inner, ',')
            .into_iter()
            .filter(|e| !e.trim().is_empty())
            .collect(),
        None => vec![expr],
    }
}

/// Every string an annotation attribute denotes: one per array element.
/// A part that cannot be evaluated (a constant declared in another file, a
/// method call) is kept as `<expression>`.
pub(crate) fn eval_strings(expr: &str, constants: &StringConstants) -> Vec<String> {
    array_elements(expr)
        .into_iter()
        .map(|element| eval_concatenation(element, constants))
        .collect()
}

/// `"/a" + BASE + "/b"` → the concatenated value.
pub(crate) fn eval_concatenation(expr: &str, constants: &StringConstants) -> String {
    split_top_level(expr, '+')
        .into_iter()
        .map(|term| eval_term(term.trim(), constants))
        .collect()
}

fn eval_term(term: &str, constants: &StringConstants) -> String {
    if let Some(literal) = string_literal(term) {
        return interpolate(&literal, constants);
    }
    if is_qualified_name(term) {
        return lookup_constant(term, constants).unwrap_or_else(|| format!("<{term}>"));
    }
    format!("<{term}>")
}

/// The content of a `"..."` / `"""..."""` literal, escapes decoded.
fn string_literal(term: &str) -> Option<String> {
    if let Some(raw) = term
        .strip_prefix("\"\"\"")
        .and_then(|t| t.strip_suffix("\"\"\""))
    {
        return Some(raw.to_string());
    }
    let inner = term.strip_prefix('"')?.strip_suffix('"')?;
    let mut out = String::with_capacity(inner.len());
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some(other) => out.push(other),
                None => {}
            }
        } else {
            out.push(c);
        }
    }
    Some(out)
}

/// Kotlin string templates over constants: `"$BASE/items"`,
/// `"${Api.BASE}/items"`. Java literals contain no templates in practice.
fn interpolate(literal: &str, constants: &StringConstants) -> String {
    let mut out = String::with_capacity(literal.len());
    let mut rest = literal;
    while let Some(dollar) = rest.find('$') {
        out.push_str(&rest[..dollar]);
        let after = &rest[dollar + 1..];
        let (name, consumed) = match after.strip_prefix('{') {
            Some(braced) => match braced.find('}') {
                Some(end) => (&braced[..end], end + 2),
                None => ("", 0),
            },
            None => {
                let end = after
                    .find(|c: char| !(c.is_alphanumeric() || c == '_'))
                    .unwrap_or(after.len());
                (&after[..end], end)
            }
        };
        if name.is_empty() {
            out.push('$');
            rest = after;
            continue;
        }
        out.push_str(&lookup_constant(name, constants).unwrap_or_else(|| format!("<{name}>")));
        rest = &after[consumed..];
    }
    out.push_str(rest);
    out
}

/// `NAME`, `Owner.NAME`, `pkg.Owner.NAME`.
fn is_qualified_name(term: &str) -> bool {
    !term.is_empty()
        && term.split('.').all(|s| {
            s.chars()
                .next()
                .is_some_and(|c| c.is_alphabetic() || c == '_')
        })
        && term
            .chars()
            .all(|c| c.is_alphanumeric() || c == '_' || c == '.' || c == '$')
}

/// A constant by its last two segments (`Owner.NAME`), then by name.
fn lookup_constant(name: &str, constants: &StringConstants) -> Option<String> {
    let mut segments = name.rsplit('.');
    let last = segments.next()?;
    if let Some(owner) = segments.next()
        && let Some(value) = constants.get(&format!("{owner}.{last}"))
    {
        return Some(value.clone());
    }
    constants.get(last).cloned()
}

/// Last segment of every element: `RequestMethod.GET` → `GET`,
/// `{RequestMethod.GET, POST}` → `GET`, `POST`.
pub(crate) fn eval_identifiers(expr: &str) -> Vec<String> {
    array_elements(expr)
        .into_iter()
        .filter_map(|element| {
            let element = element.trim();
            is_qualified_name(element)
                .then(|| element.rsplit('.').next().unwrap_or(element).to_string())
        })
        .collect()
}
