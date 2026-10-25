//! HTTP endpoints served by JVM web frameworks (issue #33).
//!
//! JAX-RS (`@Path` + `@GET`/`@POST`/…) and Spring MVC (`@RequestMapping`,
//! `@GetMapping`/…) routing annotations on Java/Kotlin handler methods
//! become `HttpEndpoint` entities named `<METHOD> <path>` — the class-level
//! path joined with the method-level one — each routed to its handler by a
//! `ROUTES_TO` edge. Paths are relative to the application: the context
//! path (`server.servlet.context-path`, `@ApplicationPath`) is deployment
//! configuration and is not applied.

mod annotation;
mod constants;
mod cross_file;
#[cfg(test)]
mod tests;

pub(crate) use constants::extract_string_constants;
pub(crate) use cross_file::resolve_cross_file_constants;

use annotation::{Annotation, StringConstants, eval_identifiers, eval_strings, parse_annotation};

use crate::models::{EntityKind, ParsedEntity, ReferenceIntent};

const JAXRS_VERBS: &[&str] = &["GET", "POST", "PUT", "DELETE", "PATCH", "HEAD", "OPTIONS"];

/// Spring's verb-specific mappings.
const SPRING_MAPPINGS: &[(&str, &str)] = &[
    ("GetMapping", "GET"),
    ("PostMapping", "POST"),
    ("PutMapping", "PUT"),
    ("DeleteMapping", "DELETE"),
    ("PatchMapping", "PATCH"),
];

/// Type annotations marking a declarative HTTP *client* (OpenFeign,
/// MicroProfile REST Client): its routing annotations describe the calls
/// it makes, not endpoints it serves.
const CLIENT_MARKERS: &[&str] = &["FeignClient", "RegisterRestClient"];

/// Verb of a `@RequestMapping` without `method`: every HTTP method.
const ANY_METHOD: &str = "ANY";

/// One route a handler serves.
#[derive(Debug, PartialEq, Eq)]
struct Route {
    verb: String,
    path: String,
}

/// The `HttpEndpoint` entities of one file's handler methods.
pub(crate) fn extract_http_endpoints(
    entities: &[ParsedEntity],
    constants: &StringConstants,
) -> Vec<ParsedEntity> {
    entities
        .iter()
        .filter(|e| matches!(e.kind, EntityKind::Method | EntityKind::KotlinMethod))
        .filter(|e| !e.decorators.is_empty())
        .flat_map(|handler| {
            let owner = owner_type(handler, entities);
            let class_annotations: Vec<Annotation> = owner
                .map(|t| {
                    t.decorators
                        .iter()
                        .filter_map(|d| parse_annotation(d))
                        .collect()
                })
                .unwrap_or_default();
            if class_annotations
                .iter()
                .any(|a| CLIENT_MARKERS.contains(&a.name.as_str()))
            {
                return Vec::new();
            }
            let method_annotations: Vec<Annotation> = handler
                .decorators
                .iter()
                .filter_map(|d| parse_annotation(d))
                .collect();
            routes(&class_annotations, &method_annotations, constants)
                .into_iter()
                .map(|route| endpoint_entity(handler, owner, &route))
                .collect()
        })
        .collect()
}

/// The type declaring `handler`, among the entities of its file.
fn owner_type<'a>(
    handler: &ParsedEntity,
    entities: &'a [ParsedEntity],
) -> Option<&'a ParsedEntity> {
    let owner_fqn = handler.fqn.strip_suffix(&handler.name)?.strip_suffix('.')?;
    entities.iter().find(|e| {
        e.fqn == owner_fqn
            && matches!(
                e.kind,
                EntityKind::Class
                    | EntityKind::Interface
                    | EntityKind::KotlinClass
                    | EntityKind::KotlinInterface
                    | EntityKind::KotlinObject
            )
    })
}

/// Every route of a handler: JAX-RS verbs with `@Path`, and Spring
/// mappings, each under its class-level base paths.
fn routes(
    class_annotations: &[Annotation],
    method_annotations: &[Annotation],
    constants: &StringConstants,
) -> Vec<Route> {
    let mut routes = jaxrs_routes(class_annotations, method_annotations, constants);
    routes.extend(spring_routes(
        class_annotations,
        method_annotations,
        constants,
    ));
    routes.dedup();
    routes
}

/// JAX-RS: a verb annotation is required; a method with `@Path` alone is a
/// sub-resource locator, not an endpoint.
fn jaxrs_routes(
    class_annotations: &[Annotation],
    method_annotations: &[Annotation],
    constants: &StringConstants,
) -> Vec<Route> {
    let verbs: Vec<&str> = method_annotations
        .iter()
        .filter_map(|a| JAXRS_VERBS.iter().find(|v| **v == a.name).copied())
        .collect();
    if verbs.is_empty() {
        return Vec::new();
    }
    let bases = paths_of(class_annotations, "Path", &["value"], constants);
    let subs = paths_of(method_annotations, "Path", &["value"], constants);
    combine(&verbs, &bases, &subs)
}

fn spring_routes(
    class_annotations: &[Annotation],
    method_annotations: &[Annotation],
    constants: &StringConstants,
) -> Vec<Route> {
    let bases = paths_of(
        class_annotations,
        "RequestMapping",
        &["value", "path"],
        constants,
    );
    let mut routes = Vec::new();
    for annotation in method_annotations {
        let verbs: Vec<String> = match SPRING_MAPPINGS
            .iter()
            .find(|(name, _)| *name == annotation.name)
        {
            Some((_, verb)) => vec![verb.to_string()],
            None if annotation.name == "RequestMapping" => request_mapping_verbs(annotation),
            None => continue,
        };
        let subs = annotation
            .arg(&["value", "path"])
            .map(|expr| eval_strings(expr, constants))
            .unwrap_or_else(|| vec![String::new()]);
        let verbs: Vec<&str> = verbs.iter().map(String::as_str).collect();
        routes.extend(combine(&verbs, &bases, &subs));
    }
    routes
}

/// `@RequestMapping(method = …)`'s verbs; [`ANY_METHOD`] without `method`.
fn request_mapping_verbs(annotation: &Annotation) -> Vec<String> {
    let verbs = annotation
        .arg(&["method"])
        .map(eval_identifiers)
        .unwrap_or_default();
    if verbs.is_empty() {
        vec![ANY_METHOD.to_string()]
    } else {
        verbs
    }
}

/// The paths of the first annotation named `name`, `[""]` without one.
fn paths_of(
    annotations: &[Annotation],
    name: &str,
    keys: &[&str],
    constants: &StringConstants,
) -> Vec<String> {
    annotations
        .iter()
        .find(|a| a.name == name)
        .and_then(|a| a.arg(keys))
        .map(|expr| eval_strings(expr, constants))
        .unwrap_or_else(|| vec![String::new()])
}

fn combine(verbs: &[&str], bases: &[String], subs: &[String]) -> Vec<Route> {
    let mut routes = Vec::new();
    for verb in verbs {
        for base in bases {
            for sub in subs {
                routes.push(Route {
                    verb: verb.to_string(),
                    path: normalize_path(&format!("{base}/{sub}")),
                });
            }
        }
    }
    routes
}

/// One leading `/`, no empty segments, no trailing `/`, and path variables
/// without their regular expression (`{id: [0-9]+}` → `{id}`).
pub(crate) fn normalize_path(raw: &str) -> String {
    let stripped = strip_variable_patterns(raw);
    let segments: Vec<&str> = stripped.split('/').filter(|s| !s.is_empty()).collect();
    format!("/{}", segments.join("/"))
}

fn strip_variable_patterns(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut chars = raw.chars();
    while let Some(c) = chars.next() {
        if c != '{' {
            out.push(c);
            continue;
        }
        let mut name = String::new();
        let mut depth = 1usize;
        let mut in_pattern = false;
        for inner in chars.by_ref() {
            match inner {
                '{' => depth += 1,
                '}' if depth == 1 => break,
                '}' => depth -= 1,
                ':' if depth == 1 => in_pattern = true,
                _ if !in_pattern => name.push(inner),
                _ => {}
            }
        }
        out.push('{');
        out.push_str(name.trim());
        out.push('}');
    }
    out
}

fn endpoint_entity(
    handler: &ParsedEntity,
    owner: Option<&ParsedEntity>,
    route: &Route,
) -> ParsedEntity {
    let name = format!("{} {}", route.verb, route.path);
    let owner_name = owner.map_or_else(
        || handler.enclosing_class.clone().unwrap_or_default(),
        |t| t.name.clone(),
    );
    let handler_label = if owner_name.is_empty() {
        handler.name.clone()
    } else {
        format!("{owner_name}.{}", handler.name)
    };
    let mut docstring = format!("HTTP endpoint {name}, served by {handler_label}.");
    if let Some(doc) = &handler.docstring {
        docstring.push_str("\n\n");
        docstring.push_str(doc);
    }
    let mut endpoint = ParsedEntity::new(
        &name,
        EntityKind::HttpEndpoint,
        format!("{}[{name}]", handler.fqn),
        None,
        Some(docstring),
        handler.language.clone(),
        handler.file_path.clone(),
        handler.start_line,
        handler.start_line,
        handler.enclosing_class.clone(),
        handler.repo_name.clone(),
    );
    endpoint.enclosing_class_fqn = owner.map(|t| t.fqn.clone());
    endpoint.reference_intents.push(ReferenceIntent::RoutesTo {
        handler: handler.uuid,
        line: handler.start_line,
    });
    endpoint
}

/// `endpoint` under a new `<METHOD> <path>` name: FQN, docstring and UUID
/// follow; the route to the handler is kept.
fn rename_endpoint(endpoint: &ParsedEntity, name: &str) -> ParsedEntity {
    let handler_fqn = endpoint
        .fqn
        .strip_suffix(&format!("[{}]", endpoint.name))
        .unwrap_or(&endpoint.fqn);
    let docstring = endpoint
        .docstring
        .as_ref()
        .map(|doc| doc.replacen(&endpoint.name, name, 1));
    let mut renamed = ParsedEntity::new(
        name,
        EntityKind::HttpEndpoint,
        format!("{handler_fqn}[{name}]"),
        None,
        docstring,
        endpoint.language.clone(),
        endpoint.file_path.clone(),
        endpoint.start_line,
        endpoint.end_line,
        endpoint.enclosing_class.clone(),
        endpoint.repo_name.clone(),
    );
    renamed.enclosing_class_fqn = endpoint.enclosing_class_fqn.clone();
    renamed.import_scope = endpoint.import_scope.clone();
    renamed.reference_intents = endpoint.reference_intents.clone();
    renamed
}
