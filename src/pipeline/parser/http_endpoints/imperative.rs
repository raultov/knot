//! HTTP calls made by imperative JVM clients (issue #46, phase 2):
//! `RestTemplate` (`getForObject`, `postForEntity`, `exchange(url,
//! HttpMethod.X, …)`, …) and the fluent `WebClient` / `RestClient` API
//! (`.get().uri(…)`, `.method(HttpMethod.X).uri(…)`), in Java and Kotlin.
//!
//! Call sites are found in the source text, comments masked: Kotlin
//! offers no declared receiver types, so only patterns specific to these
//! APIs count, and the generic `put` / `delete` need a receiver the file
//! declares as `RestTemplate` / `RestOperations`. Each call becomes an
//! `HttpClientCall` of the innermost method containing it.
//!
//! The URL argument is evaluated like an annotation attribute — literals,
//! `+` concatenation, Kotlin templates, constants — plus `@Value("${…}")`
//! fields and parameters, `URI.create(…)`, `String.format(…)` and
//! `UriComponentsBuilder` / `uriBuilder` `path(…)` calls. An unknown
//! leading part (a `baseUrl` variable) stands for scheme and host, an
//! unknown later part for a `{variable}` segment, and the query string is
//! dropped. A relative URL takes the base URL of the file's single
//! `baseUrl(…)` / `rootUri(…)` builder call. Test files are skipped.

use std::collections::HashMap;
use std::ops::Range;

use super::annotation::{
    StringConstants, eval_property_strings, is_qualified_name, lookup_constant, parse_annotation,
    skip_literal, split_top_level, string_literal, unmatched,
};
use super::clients::{CallSite, RawClient, client_call_entity};
use super::{Route, normalize_path, owner_type};
use crate::cli_tools::search_hybrid_context::rank::is_test_path;
use crate::models::{EntityKind, ParsedEntity};

/// `RestTemplate` methods and the verb each one sends.
const REST_TEMPLATE_METHODS: &[(&str, &str)] = &[
    ("getForObject", "GET"),
    ("getForEntity", "GET"),
    ("postForObject", "POST"),
    ("postForEntity", "POST"),
    ("postForLocation", "POST"),
    ("patchForObject", "PATCH"),
    ("headForHeaders", "HEAD"),
    ("optionsForAllow", "OPTIONS"),
    ("put", "PUT"),
    ("delete", "DELETE"),
];

/// `RestTemplate` methods sending their `HttpMethod` second argument.
const REST_TEMPLATE_DYNAMIC: &[&str] = &["exchange", "execute"];

/// `RestTemplate` methods too generic to be recognised by name alone.
const TYPED_ONLY: &[&str] = &["put", "delete"];

const REST_TEMPLATE_TYPES: &[&str] = &["RestTemplate", "RestOperations"];

/// `WebClient` / `RestClient` request starters preceding `.uri(…)`.
const FLUENT_VERBS: &[&str] = &["get", "post", "put", "delete", "patch", "head", "options"];

const HTTP_METHODS: &[&str] = &["GET", "POST", "PUT", "DELETE", "PATCH", "HEAD", "OPTIONS"];

/// Builder calls setting the base URL of a client.
const BASE_URL_SETTERS: &[&str] = &["baseUrl", "rootUri"];

/// URI builder calls appending to the path.
const PATH_BUILDERS: &[&str] = &[
    "fromHttpUrl",
    "fromUriString",
    "fromPath",
    "path",
    "pathSegment",
];

/// Delimits the name of an unknown part while a URL is assembled.
const HOLE: char = '\u{0}';

/// The `HttpClientCall` entities of one file's imperative client calls.
pub(crate) fn extract_imperative_calls(
    source: &str,
    lang_name: &str,
    entities: &[ParsedEntity],
    constants: &StringConstants,
) -> Vec<ParsedEntity> {
    let Some(file_path) = entities.first().map(|e| e.file_path.as_str()) else {
        return Vec::new();
    };
    if is_test_path(file_path) {
        return Vec::new();
    }
    let (code, literals) = mask_comments(source);
    let ctx = UrlContext {
        constants,
        values: value_bindings(&code, constants),
        kotlin: lang_name == "kotlin",
    };
    let builder_base = builder_base_url(&code, &ctx);
    call_sites(&code)
        .into_iter()
        .filter(|site| {
            !literals
                .iter()
                .any(|literal| literal.contains(&site.offset))
        })
        .filter_map(|site| {
            let pieces = match site.url {
                UrlArg::Expr(expr) => eval_url(expr, &ctx),
                UrlArg::Builder(body) => builder_pieces(body, &ctx),
            };
            let (base, path) = split_base(&pieces)?;
            let url = match base {
                Base::Url(url) => Some(url),
                Base::Unknown => None,
                Base::Relative => builder_base.clone(),
            };
            let line = code[..site.offset].matches('\n').count() + 1;
            let method = innermost_method(entities, line)?;
            let route = Route {
                verb: site.verb,
                path,
            };
            Some(client_call_entity(
                method,
                owner_type(method, entities),
                &route,
                &RawClient::imperative(url),
                CallSite {
                    line,
                    via: Some(&site.via),
                },
            ))
        })
        .collect()
}

/// One client call found in the source.
struct Site<'a> {
    verb: String,
    /// The API called, for the docstring.
    via: String,
    /// Byte offset of the call in the masked source.
    offset: usize,
    url: UrlArg<'a>,
}

enum UrlArg<'a> {
    /// A URL expression.
    Expr(&'a str),
    /// The body of a `uriBuilder -> uriBuilder.path(…)` lambda.
    Builder(&'a str),
}

fn call_sites(code: &str) -> Vec<Site<'_>> {
    let mut sites = rest_template_sites(code);
    sites.extend(fluent_sites(code));
    sites.sort_by_key(|site| site.offset);
    sites
}

fn rest_template_sites(code: &str) -> Vec<Site<'_>> {
    let fixed = REST_TEMPLATE_METHODS
        .iter()
        .map(|(name, verb)| (*name, Some(*verb)));
    let dynamic = REST_TEMPLATE_DYNAMIC.iter().map(|name| (*name, None));
    let mut sites = Vec::new();
    for (name, verb) in fixed.chain(dynamic) {
        for call in member_calls(code, name) {
            let Group::Args(args_text) = call.group else {
                continue;
            };
            let receiver = receiver_name(code, call.dot);
            if receiver.is_some_and(|r| declares(code, r, &["TestRestTemplate"])) {
                continue;
            }
            if TYPED_ONLY.contains(&name)
                && !receiver.is_some_and(|r| declares(code, r, REST_TEMPLATE_TYPES))
            {
                continue;
            }
            let args = split_top_level(args_text, ',');
            let verb = match verb {
                Some(verb) => verb.to_string(),
                None => match args.get(1).and_then(|arg| http_method(arg)) {
                    Some(verb) => verb,
                    None => continue,
                },
            };
            let Some(url) = args.first().filter(|a| !a.trim().is_empty()) else {
                continue;
            };
            sites.push(Site {
                verb,
                via: format!("RestTemplate.{name}"),
                offset: call.dot,
                url: UrlArg::Expr(url),
            });
        }
    }
    sites
}

fn fluent_sites(code: &str) -> Vec<Site<'_>> {
    let mut sites = Vec::new();
    for call in member_calls(code, "uri") {
        let Some((verb, starter, offset)) = fluent_verb(code, call.dot) else {
            continue;
        };
        let url = match call.group {
            Group::Lambda(body) => UrlArg::Builder(body),
            Group::Args(args_text) => {
                let Some(first) = split_top_level(args_text, ',')
                    .into_iter()
                    .next()
                    .filter(|a| !a.trim().is_empty())
                else {
                    continue;
                };
                match lambda_body(first) {
                    Some(body) => UrlArg::Builder(body),
                    None => UrlArg::Expr(first),
                }
            }
        };
        sites.push(Site {
            verb,
            via: format!("WebClient/RestClient {starter}.uri(…)"),
            offset,
            url,
        });
    }
    sites
}

/// The body of `b -> …` / `(b) -> …`.
fn lambda_body(arg: &str) -> Option<&str> {
    let (params, body) = arg.split_once("->")?;
    let params = params.trim().trim_start_matches('(').trim_end_matches(')');
    is_identifier(params.trim()).then_some(body)
}

/// The verb of the request a `.uri` call at `dot` continues, the starter
/// text and its offset: `.get()` → `GET`, `.method(HttpMethod.PUT)` → `PUT`.
fn fluent_verb(code: &str, dot: usize) -> Option<(String, String, usize)> {
    let before = code[..dot].trim_end();
    let inner_end = before.strip_suffix(')')?.len();
    let open = matching_open(&before[..inner_end])?;
    let args = &before[open + 1..inner_end];
    let (name, start) = identifier_before(before, open)?;
    if !before[..start].trim_end().ends_with('.') {
        return None;
    }
    let verb = if FLUENT_VERBS.contains(&name) && args.trim().is_empty() {
        name.to_uppercase()
    } else if name == "method" {
        http_method(args)?
    } else {
        return None;
    };
    Some((verb, format!(".{name}({})", args.trim()), start))
}

/// `HttpMethod.GET` → `GET`.
fn http_method(arg: &str) -> Option<String> {
    let name = arg.trim().rsplit('.').next()?;
    HTTP_METHODS.contains(&name).then(|| name.to_string())
}

/// What follows a member name: its argument list or a Kotlin trailing
/// lambda.
enum Group<'a> {
    Args(&'a str),
    Lambda(&'a str),
}

struct MemberCall<'a> {
    /// Offset of the `.` before the member name.
    dot: usize,
    group: Group<'a>,
}

/// Every `.name(…)` / `.name<T>(…)` / `.name { … }` of `code`.
fn member_calls<'a>(code: &'a str, name: &str) -> Vec<MemberCall<'a>> {
    let needle = format!(".{name}");
    let mut calls = Vec::new();
    for (dot, _) in code.match_indices(&needle) {
        let mut rest = &code[dot + needle.len()..];
        if rest.starts_with(is_identifier_char) {
            continue;
        }
        rest = rest.trim_start();
        if let Some(generic) = rest.strip_prefix('<') {
            let Some(end) = generic.find('>') else {
                continue;
            };
            rest = generic[end + 1..].trim_start();
        }
        let group = if let Some(inner) = rest.strip_prefix('(') {
            group_end(inner, ')').map(|end| Group::Args(&inner[..end]))
        } else if let Some(inner) = rest.strip_prefix('{') {
            group_end(inner, '}').map(|end| Group::Lambda(&inner[..end]))
        } else {
            None
        };
        if let Some(group) = group {
            calls.push(MemberCall { dot, group });
        }
    }
    calls
}

/// Index of the `close` delimiter ending the group `text` follows.
fn group_end(text: &str, close: char) -> Option<usize> {
    let mut depth = 0usize;
    let mut chars = text.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        match c {
            '"' | '\'' => skip_literal(text, i, &mut chars),
            c if c == close && depth == 0 => return Some(i),
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    None
}

/// Index of the `(` matching the `)` that ends right after `text`.
fn matching_open(text: &str) -> Option<usize> {
    unmatched(text.char_indices().rev(), ')', '(')
}

/// The identifier ending at `end` (whitespace skipped) and its start.
fn identifier_before(code: &str, end: usize) -> Option<(&str, usize)> {
    let text = code[..end].trim_end();
    let start = text
        .char_indices()
        .rev()
        .take_while(|(_, c)| is_identifier_char(*c))
        .last()
        .map(|(i, _)| i)?;
    Some((&text[start..], start))
}

/// The last name of the receiver before the `.` at `dot`
/// (`this.restTemplate.put` → `restTemplate`).
fn receiver_name(code: &str, dot: usize) -> Option<&str> {
    identifier_before(code, dot).map(|(name, _)| name)
}

/// Whether `code` declares `name` with one of `types`: `Type name`,
/// `Type<…> name`, `name: Type`, `name = new Type(` or `name = Type(`.
fn declares(code: &str, name: &str, types: &[&str]) -> bool {
    types.iter().any(|ty| {
        code.match_indices(ty).any(|(at, _)| {
            let end = at + ty.len();
            if code[..at].ends_with(is_identifier_char)
                || code[end..].starts_with(is_identifier_char)
            {
                return false;
            }
            let mut after = code[end..].trim_start();
            if let Some(generic) = after.strip_prefix('<') {
                after = generic
                    .find('>')
                    .map_or("", |i| generic[i + 1..].trim_start());
            }
            if starts_with_identifier(after, name) {
                return true;
            }
            let before = code[..at].trim_end();
            let before = before.strip_suffix("new").map_or(before, str::trim_end);
            let before = before
                .strip_suffix(':')
                .or_else(|| before.strip_suffix('='))
                .map(str::trim_end);
            before.is_some_and(|b| {
                b.ends_with(name) && !b[..b.len() - name.len()].ends_with(is_identifier_char)
            })
        })
    })
}

fn starts_with_identifier(text: &str, name: &str) -> bool {
    text.strip_prefix(name)
        .is_some_and(|rest| !rest.starts_with(is_identifier_char))
}

fn is_identifier_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '$'
}

fn is_identifier(text: &str) -> bool {
    !text.is_empty() && text.chars().all(is_identifier_char)
}

/// The innermost callable entity whose lines contain `line`.
fn innermost_method(entities: &[ParsedEntity], line: usize) -> Option<&ParsedEntity> {
    entities
        .iter()
        .filter(|e| {
            matches!(
                e.kind,
                EntityKind::Method
                    | EntityKind::Function
                    | EntityKind::KotlinMethod
                    | EntityKind::KotlinFunction
            )
        })
        .filter(|e| e.start_line <= line && line <= e.end_line)
        .min_by_key(|e| e.end_line - e.start_line)
}

/// `code` with every comment blanked out (line breaks kept), so commented
/// calls are not found, and the ranges of its string literals.
fn mask_comments(code: &str) -> (String, Vec<Range<usize>>) {
    let mut out = String::with_capacity(code.len());
    let mut literals = Vec::new();
    let mut chars = code.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        let rest = &code[i..];
        if rest.starts_with("//") {
            let end = rest.find('\n').unwrap_or(rest.len());
            out.extend(std::iter::repeat_n(' ', end));
            while chars.peek().is_some_and(|(j, _)| *j < i + end) {
                chars.next();
            }
        } else if let Some(body) = rest.strip_prefix("/*") {
            let end = body.find("*/").map_or(rest.len(), |e| e + 4);
            out.extend(
                rest[..end]
                    .chars()
                    .map(|c| if c == '\n' { '\n' } else { ' ' }),
            );
            while chars.peek().is_some_and(|(j, _)| *j < i + end) {
                chars.next();
            }
        } else if c == '"' || c == '\'' {
            let mut literal = chars.clone();
            skip_literal(code, i, &mut literal);
            let end = literal.peek().map_or(code.len(), |(j, _)| *j);
            let start = out.len();
            out.push_str(&code[i..end]);
            literals.push(start..out.len());
            chars = literal;
        } else {
            out.push(c);
        }
    }
    (out, literals)
}

/// What URL expressions are evaluated against.
struct UrlContext<'a> {
    constants: &'a StringConstants,
    /// `@Value("${…}")` fields and parameters → their placeholder.
    values: HashMap<String, String>,
    kotlin: bool,
}

/// `@Value("${items.url}") private String itemsUrl` → `itemsUrl` →
/// `${items.url}`, for fields and constructor parameters alike.
fn value_bindings(code: &str, constants: &StringConstants) -> HashMap<String, String> {
    let mut values = HashMap::new();
    for (at, _) in code.match_indices("@Value") {
        let rest = code[at + "@Value".len()..].trim_start();
        let Some(inner) = rest.strip_prefix('(') else {
            continue;
        };
        let Some(end) = group_end(inner, ')') else {
            continue;
        };
        let Some(annotation) = parse_annotation(&format!("@Value({})", &inner[..end])) else {
            continue;
        };
        let Some(value) = annotation
            .arg(&["value"])
            .and_then(|expr| eval_property_strings(expr, constants).into_iter().next())
        else {
            continue;
        };
        if let Some(name) = declared_name(&inner[end + 1..]) {
            values.insert(name.to_string(), value);
        }
    }
    values
}

/// The name a declaration following an annotation declares:
/// `private String url;` / `String url,` / `private val url: String`.
fn declared_name(after: &str) -> Option<&str> {
    let after = after.trim_start();
    let end = after
        .find([';', ',', ')', '=', '{', '\n'])
        .unwrap_or(after.len());
    let declaration = &after[..end];
    let declaration = declaration
        .split_once(':')
        .map_or(declaration, |(name, _)| name);
    declaration
        .split_whitespace()
        .last()
        .filter(|name| is_identifier(name))
}

/// The base URL of the file's single `baseUrl(…)` / `rootUri(…)` call.
fn builder_base_url(code: &str, ctx: &UrlContext<'_>) -> Option<String> {
    let mut bases = BASE_URL_SETTERS.iter().flat_map(|setter| {
        member_calls(code, setter)
            .into_iter()
            .map(|call| match call.group {
                Group::Args(args) => known_text(&eval_url(args, ctx)),
                Group::Lambda(_) => None,
            })
    });
    let first = bases.next()??;
    bases
        .all(|other| other.as_ref() == Some(&first))
        .then_some(first)
}

/// One part of an evaluated URL.
#[derive(Debug, PartialEq, Eq)]
enum Piece {
    Text(String),
    /// An unknown part, named after its expression when it is a name.
    Hole(String),
}

/// The text of `pieces` when every part is known.
fn known_text(pieces: &[Piece]) -> Option<String> {
    pieces
        .iter()
        .map(|piece| match piece {
            Piece::Text(text) => Some(text.as_str()),
            Piece::Hole(_) => None,
        })
        .collect()
}

fn eval_url(expr: &str, ctx: &UrlContext<'_>) -> Vec<Piece> {
    let expr = expr.trim();
    for wrapper in ["java.net.URI.create(", "URI.create(", "new URI(", "URI("] {
        if let Some(args) = call_args(expr, wrapper)
            && let Some(first) = args.first()
        {
            return eval_url(first, ctx);
        }
    }
    for formatter in ["java.lang.String.format(", "String.format("] {
        if let Some(args) = call_args(expr, formatter)
            && let Some((format, values)) = args.split_first()
        {
            return format_pieces(format, values, ctx);
        }
    }
    if let Some((format, values)) = format_receiver(expr) {
        return format_pieces(format, &values, ctx);
    }
    if ["UriComponentsBuilder", "fromHttpUrl(", "fromUriString("]
        .iter()
        .any(|marker| expr.contains(marker))
    {
        return builder_pieces(expr, ctx);
    }
    split_top_level(expr, '+')
        .into_iter()
        .flat_map(|term| term_pieces(term.trim(), ctx))
        .collect()
}

/// The arguments of `expr` when it is entirely a `prefix…)` call.
fn call_args<'a>(expr: &'a str, prefix: &str) -> Option<Vec<&'a str>> {
    let inner = expr.strip_prefix(prefix)?;
    let end = group_end(inner, ')')?;
    (end + 1 == inner.len()).then(|| split_top_level(&inner[..end], ','))
}

/// `"…".formatted(a)` / Kotlin `"…".format(a)` → the literal and the
/// arguments.
fn format_receiver(expr: &str) -> Option<(&str, Vec<&str>)> {
    if !expr.starts_with('"') {
        return None;
    }
    ["\".formatted(", "\".format("].iter().find_map(|marker| {
        let at = expr.rfind(marker)?;
        let literal = &expr[..=at];
        string_literal(literal)?;
        let args = &expr[at + marker.len()..];
        let end = group_end(args, ')')?;
        (end + 1 == args.len()).then(|| (literal, split_top_level(&args[..end], ',')))
    })
}

/// `String.format("%s/items/%d", base, id)`: each conversion takes the
/// pieces of its argument.
fn format_pieces(format: &str, values: &[&str], ctx: &UrlContext<'_>) -> Vec<Piece> {
    let Some(format) = string_literal(format.trim()) else {
        return vec![Piece::Hole("var".to_string())];
    };
    let mut pieces = Vec::new();
    let mut text = String::new();
    let mut values = values.iter();
    let mut chars = format.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '%' {
            text.push(c);
            continue;
        }
        if chars.peek() == Some(&'%') {
            chars.next();
            text.push('%');
            continue;
        }
        while chars.next_if(|c| !c.is_alphabetic()).is_some() {}
        chars.next();
        pieces.push(Piece::Text(std::mem::take(&mut text)));
        match values.next() {
            Some(value) => pieces.extend(term_pieces(value.trim(), ctx)),
            None => pieces.push(Piece::Hole("var".to_string())),
        }
    }
    pieces.push(Piece::Text(text));
    pieces
}

/// `UriComponentsBuilder.fromHttpUrl(base).path("/items/{id}")` and
/// `uriBuilder.path("/refunds").pathSegment(id)`: the base and path parts,
/// in order; query parameters and the rest are ignored.
fn builder_pieces(expr: &str, ctx: &UrlContext<'_>) -> Vec<Piece> {
    let mut calls: Vec<(usize, &str, &str)> = PATH_BUILDERS
        .iter()
        .flat_map(|name| {
            member_calls(expr, name)
                .into_iter()
                .filter_map(move |call| match call.group {
                    Group::Args(args) => Some((call.dot, *name, args)),
                    Group::Lambda(_) => None,
                })
        })
        .collect();
    calls.sort_by_key(|(at, _, _)| *at);
    let mut pieces = Vec::new();
    for (_, name, args) in calls {
        if name == "pathSegment" {
            for segment in split_top_level(args, ',') {
                pieces.push(Piece::Text("/".to_string()));
                pieces.extend(term_pieces(segment.trim(), ctx));
            }
        } else {
            pieces.extend(eval_url(args, ctx));
        }
    }
    pieces
}

fn term_pieces(term: &str, ctx: &UrlContext<'_>) -> Vec<Piece> {
    if term.starts_with('"') {
        return literal_pieces(term, ctx);
    }
    let name = term.strip_prefix("this.").unwrap_or(term);
    if is_qualified_name(name) {
        if let Some(value) = ctx.values.get(name) {
            return vec![Piece::Text(value.clone())];
        }
        if let Some(value) = lookup_constant(name, ctx.constants) {
            return vec![Piece::Text(value)];
        }
        let last = name.rsplit('.').next().unwrap_or(name);
        return vec![Piece::Hole(last.to_string())];
    }
    vec![Piece::Hole("var".to_string())]
}

/// A string literal; in Kotlin, its `$name` / `${expr}` templates are
/// evaluated as terms.
fn literal_pieces(literal: &str, ctx: &UrlContext<'_>) -> Vec<Piece> {
    if !ctx.kotlin {
        return match string_literal(literal) {
            Some(text) => vec![Piece::Text(text)],
            None => vec![Piece::Hole("var".to_string())],
        };
    }
    let Some(raw) = literal
        .strip_prefix("\"\"\"")
        .and_then(|l| l.strip_suffix("\"\"\""))
        .or_else(|| literal.strip_prefix('"').and_then(|l| l.strip_suffix('"')))
    else {
        return vec![Piece::Hole("var".to_string())];
    };
    let mut pieces = Vec::new();
    let mut text = String::new();
    let mut chars = raw.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        match c {
            '\\' => match chars.next().map(|(_, c)| c) {
                Some('n') => text.push('\n'),
                Some('t') => text.push('\t'),
                Some(other) => text.push(other),
                None => {}
            },
            '$' if raw[i + 1..].starts_with('{') => {
                let body = &raw[i + 2..];
                let Some(end) = group_end(body, '}') else {
                    text.push('$');
                    continue;
                };
                pieces.push(Piece::Text(std::mem::take(&mut text)));
                pieces.extend(term_pieces(body[..end].trim(), ctx));
                while chars.next_if(|(j, _)| *j < i + 2 + end + 1).is_some() {}
            }
            '$' if raw[i + 1..].starts_with(|c: char| c.is_alphabetic() || c == '_') => {
                let name_len = raw[i + 1..]
                    .find(|c: char| !(c.is_alphanumeric() || c == '_'))
                    .unwrap_or(raw.len() - i - 1);
                pieces.push(Piece::Text(std::mem::take(&mut text)));
                pieces.extend(term_pieces(&raw[i + 1..i + 1 + name_len], ctx));
                while chars.next_if(|(j, _)| *j < i + 1 + name_len).is_some() {}
            }
            _ => text.push(c),
        }
    }
    pieces.push(Piece::Text(text));
    pieces
}

/// Where a URL's base comes from.
#[derive(Debug, PartialEq, Eq)]
enum Base {
    /// Written in the URL: an absolute URL's scheme and authority, or a
    /// `${…}` placeholder.
    Url(String),
    /// An unknown leading part: scheme and host, maybe a context path.
    Unknown,
    /// A path only: the client's configured base URL applies.
    Relative,
}

/// The base of an evaluated URL and its normalized path; `None` when the
/// path has no literal segment.
fn split_base(pieces: &[Piece]) -> Option<(Base, String)> {
    let mut text = String::new();
    for piece in pieces {
        match piece {
            Piece::Text(t) => text.push_str(t),
            Piece::Hole(name) => {
                text.push(HOLE);
                text.push_str(name);
                text.push(HOLE);
            }
        }
    }
    let (base, rest) = if text.starts_with(HOLE) {
        let mut rest = text.as_str();
        while let Some(hole) = rest.strip_prefix(HOLE) {
            rest = hole.split_once(HOLE).map_or("", |(_, after)| after);
        }
        (Base::Unknown, rest)
    } else if text.starts_with("${") {
        let end = text.find('}')?;
        (Base::Url(text[..=end].to_string()), &text[end + 1..])
    } else if let Some(scheme_end) = text.find("://")
        && text[..scheme_end].chars().all(|c| c.is_ascii_alphabetic())
        && scheme_end > 0
    {
        let authority_end = text[scheme_end + 3..]
            .find('/')
            .map_or(text.len(), |i| scheme_end + 3 + i);
        let base = &text[..authority_end];
        let base = if base.contains(HOLE) {
            Base::Unknown
        } else {
            Base::Url(base.to_string())
        };
        (base, &text[authority_end..])
    } else {
        (Base::Relative, text.as_str())
    };
    let rest = rest.split(['?', '#']).next().unwrap_or_default();
    let segments: Vec<String> = rest
        .split('/')
        .filter(|s| !s.is_empty())
        .map(|segment| match segment.strip_prefix(HOLE) {
            Some(name) if name.ends_with(HOLE) && !name[..name.len() - 1].contains(HOLE) => {
                format!("{{{}}}", &name[..name.len() - 1])
            }
            _ if segment.contains(HOLE) => "<expression>".to_string(),
            _ => segment.to_string(),
        })
        .collect();
    let literal = segments
        .iter()
        .any(|s| !s.contains('{') && !s.contains('<'));
    literal.then(|| (base, normalize_path(&segments.join("/"))))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entities(file_path: &str, language: &str) -> Vec<ParsedEntity> {
        let class = ParsedEntity::new(
            "Svc",
            EntityKind::Class,
            "com.x.Svc",
            None,
            None,
            language,
            file_path,
            1,
            100,
            None,
            "repo",
        );
        let kind = if language == "kotlin" {
            EntityKind::KotlinMethod
        } else {
            EntityKind::Method
        };
        let method = ParsedEntity::new(
            "run",
            kind,
            "com.x.Svc.run",
            None,
            None,
            language,
            file_path,
            2,
            99,
            Some("Svc".to_string()),
            "repo",
        );
        vec![class, method]
    }

    fn calls_in(source: &str, language: &str) -> Vec<(String, Option<String>)> {
        let file = if language == "kotlin" {
            "src/main/kotlin/Svc.kt"
        } else {
            "src/main/java/Svc.java"
        };
        extract_imperative_calls(
            source,
            language,
            &entities(file, language),
            &StringConstants::new(),
        )
        .into_iter()
        .map(|call| (call.name, call.signature))
        .collect()
    }

    fn names(source: &str, language: &str) -> Vec<String> {
        calls_in(source, language)
            .into_iter()
            .map(|(n, _)| n)
            .collect()
    }

    fn base_of(source: &str) -> Option<String> {
        let calls = calls_in(source, "java");
        let raw = calls[0].1.as_deref()?.strip_prefix("client-raw:")?;
        let value: serde_json::Value = serde_json::from_str(raw).ok()?;
        value["url"].as_str().map(str::to_string)
    }

    #[test]
    fn rest_template_methods_and_value_fields() {
        let source = r#"
            @Value("${items.url}")
            private String itemsUrl;
            void run(long id) {
                restTemplate.getForObject(itemsUrl + "/items/{id}/stock", Integer.class, id);
                restTemplate.postForEntity(itemsUrl + "/items", item, Item.class);
            }"#;
        assert_eq!(
            names(source, "java"),
            vec!["GET /items/{id}/stock", "POST /items"]
        );
        assert_eq!(base_of(source).as_deref(), Some("${items.url}"));
    }

    #[test]
    fn put_and_delete_need_a_rest_template_receiver() {
        let source = r#"
            private final RestTemplate rest;
            private final Map<Long, Item> cache;
            void run(long id) {
                cache.put(id, item);
                cache.delete("/items/" + id);
                rest.put("http://items/items/" + id, item);
                this.rest.delete("http://items/items/" + id);
            }"#;
        assert_eq!(
            names(source, "java"),
            vec!["PUT /items/{id}", "DELETE /items/{id}"]
        );
    }

    #[test]
    fn kotlin_receivers_are_typed_by_their_declaration() {
        let source = r#"
            class Svc(private val rest: RestOperations, private val other: Cache) {
                fun run(id: Long) {
                    rest.delete("http://items/items/$id")
                    other.delete("/items/$id")
                }
            }"#;
        assert_eq!(names(source, "kotlin"), vec!["DELETE /items/{id}"]);
    }

    #[test]
    fn exchange_takes_its_http_method_argument() {
        let source = r#"
            void run() {
                restTemplate.exchange("http://billing/invoices", HttpMethod.PATCH, entity, Invoice.class);
                restTemplate.exchange(requestEntity, Invoice.class);
            }"#;
        assert_eq!(names(source, "java"), vec!["PATCH /invoices"]);
        assert_eq!(base_of(source).as_deref(), Some("http://billing"));
    }

    #[test]
    fn fluent_requests_and_uri_builder_lambdas() {
        let source = r#"
            void run(String number) {
                webClient.get().uri("/invoices/{number}", number).retrieve();
                webClient.post()
                    .uri(b -> b.path("/refunds").queryParam("x", 1).build())
                    .retrieve();
                restClient.method(HttpMethod.DELETE).uri("/invoices/" + number + "?force=true");
                HttpRequest.newBuilder().uri(URI.create("http://x/a")).GET();
            }"#;
        assert_eq!(
            names(source, "java"),
            vec![
                "GET /invoices/{number}",
                "POST /refunds",
                "DELETE /invoices/{number}"
            ]
        );
    }

    #[test]
    fn kotlin_templates_and_trailing_uri_lambdas() {
        let source = r#"
            fun run(id: Long) {
                restClient.put().uri("$baseUrl/api/items/$id").retrieve()
                webClient.get().uri { it.path("/items/{id}").build(id) }.retrieve()
                restTemplate.getForObject<Item>("${base}/items/${item.id}")
            }"#;
        assert_eq!(
            names(source, "kotlin"),
            vec!["PUT /api/items/{id}", "GET /items/{id}", "GET /items/{id}"]
        );
    }

    #[test]
    fn a_single_builder_base_url_applies_to_relative_calls() {
        let source = r#"
            Svc(WebClient.Builder builder, @Value("${billing.url}") String billingUrl) {
                this.client = builder.baseUrl(billingUrl).build();
            }
            void run() { client.get().uri("/invoices").retrieve(); }"#;
        assert_eq!(base_of(source).as_deref(), Some("${billing.url}"));
        let two = r#"
            void a() { x = builder.baseUrl("http://a").build(); y = builder.baseUrl("http://b").build(); }
            void run() { client.get().uri("/invoices").retrieve(); }"#;
        assert_eq!(base_of(two), None);
    }

    #[test]
    fn uri_wrappers_and_format_strings() {
        let source = r#"
            void run(long id) {
                restTemplate.getForObject(URI.create(String.format("%s/items/%d", base, id)), Item.class);
                restTemplate.getForObject(UriComponentsBuilder.fromHttpUrl("http://items/api").path("/items/{id}").buildAndExpand(id).toUri(), Item.class);
                restTemplate.getForObject("/items/%d/stock".formatted(id), Integer.class);
            }"#;
        assert_eq!(
            names(source, "java"),
            vec![
                "GET /items/{id}",
                "GET /api/items/{id}",
                "GET /items/{id}/stock"
            ]
        );
    }

    #[test]
    fn unknown_or_literal_free_urls_and_comments_make_no_call() {
        let source = r#"
            void run(String url) {
                restTemplate.getForObject(url, String.class);
                restTemplate.getForObject(base + "/" + id, String.class);
                // restTemplate.getForObject("http://x/commented", String.class);
                /* webClient.get().uri("/also/commented"); */
                String s = "restTemplate.getForObject(\"/in/a/string\")";
            }"#;
        assert_eq!(names(source, "java"), Vec::<String>::new());
    }

    #[test]
    fn test_files_and_test_rest_templates_are_skipped() {
        let source = r#"void run() { rest.getForObject("http://x/items", Item.class); }"#;
        let in_test = entities("src/test/java/SvcTest.java", "java");
        assert!(
            extract_imperative_calls(source, "java", &in_test, &StringConstants::new()).is_empty()
        );
        let typed = r#"
            private TestRestTemplate rest;
            void run() { rest.getForObject("http://x/items", Item.class); }"#;
        assert!(names(typed, "java").is_empty());
    }

    #[test]
    fn urls_split_into_base_and_path() {
        let text = |t: &str| Piece::Text(t.to_string());
        let hole = |n: &str| Piece::Hole(n.to_string());
        assert_eq!(
            split_base(&[hole("base"), text("/items/"), hole("id"), text("?q=1")]),
            Some((Base::Unknown, "/items/{id}".to_string()))
        );
        assert_eq!(
            split_base(&[text("http://"), hole("host"), text("/items")]),
            Some((Base::Unknown, "/items".to_string()))
        );
        assert_eq!(
            split_base(&[text("/items/item-"), hole("id")]),
            Some((Base::Relative, "/items/<expression>".to_string()))
        );
        assert_eq!(split_base(&[text("http://items")]), None);
    }
}
