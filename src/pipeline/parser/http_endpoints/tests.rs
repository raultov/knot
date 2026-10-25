use super::annotation::{StringConstants, eval_strings, parse_annotation};
use super::normalize_path;
use crate::models::{EntityKind, ParsedEntity, ReferenceIntent};
use crate::pipeline::parser::extractor::extract_entities;

fn extract(lang: &str, source: &str) -> Vec<ParsedEntity> {
    let (language, query, file) = match lang {
        "java" => (
            tree_sitter_java::LANGUAGE.into(),
            include_str!("../../../../queries/java.scm"),
            "src/Api.java",
        ),
        _ => (
            tree_sitter_kotlin_ng::LANGUAGE.into(),
            include_str!("../../../../queries/kotlin.scm"),
            "src/Api.kt",
        ),
    };
    extract_entities(source, language, query, lang, file, "test-repo").unwrap()
}

/// Sorted `<endpoint name> -> <handler FQN>` of every endpoint.
fn endpoints(lang: &str, source: &str) -> Vec<String> {
    let entities = extract(lang, source);
    let mut out: Vec<String> = entities
        .iter()
        .filter(|e| e.kind == EntityKind::HttpEndpoint)
        .map(|endpoint| {
            let handler = endpoint
                .reference_intents
                .iter()
                .find_map(|i| match i {
                    ReferenceIntent::RoutesTo { handler, .. } => {
                        entities.iter().find(|e| e.uuid == *handler)
                    }
                    _ => None,
                })
                .map_or("?", |h| h.fqn.as_str());
            format!("{} -> {handler}", endpoint.name)
        })
        .collect();
    out.sort();
    out
}

#[test]
fn test_parse_annotation_arguments() {
    let a =
        parse_annotation(r#"@PatchMapping(value = "/p/{id}", consumes = { "a", "b" })"#).unwrap();
    assert_eq!(a.name, "PatchMapping");
    assert_eq!(a.arg(&["value", "path"]), Some(r#""/p/{id}""#));
    assert_eq!(a.arg(&["consumes"]), Some(r#"{ "a", "b" }"#));
    let b = parse_annotation(r#"@javax.ws.rs.Path("/x, y")"#).unwrap();
    assert_eq!(b.name, "Path");
    assert_eq!(b.arg(&["value"]), Some(r#""/x, y""#));
    assert!(parse_annotation("@GET").unwrap().args.is_empty());
    assert!(parse_annotation("RestController").is_none());
}

#[test]
fn test_eval_strings_arrays_concatenation_and_constants() {
    let constants: StringConstants = [
        ("BASE".to_string(), "/api".to_string()),
        ("Paths.ITEMS".to_string(), "/items".to_string()),
    ]
    .into_iter()
    .collect();
    assert_eq!(
        eval_strings(r#"{ "/a", "/b" }"#, &constants),
        vec!["/a", "/b"]
    );
    assert_eq!(eval_strings(r#"["/a"]"#, &constants), vec!["/a"]);
    assert_eq!(eval_strings(r#"BASE + "/x""#, &constants), vec!["/api/x"]);
    assert_eq!(
        eval_strings(r#"com.x.Paths.ITEMS + "/{id}""#, &constants),
        vec!["/items/{id}"]
    );
    assert_eq!(eval_strings(r#""$BASE/k""#, &constants), vec!["/api/k"]);
    assert_eq!(
        eval_strings(r#""${Paths.ITEMS}""#, &constants),
        vec!["/items"]
    );
    assert_eq!(
        eval_strings("Other.MISSING", &constants),
        vec!["<Other.MISSING>"]
    );
}

#[test]
fn test_normalize_path() {
    assert_eq!(normalize_path("api//items/"), "/api/items");
    assert_eq!(normalize_path("/"), "/");
    assert_eq!(normalize_path(""), "/");
    assert_eq!(normalize_path("/{id: [0-9]+}"), "/{id}");
    assert_eq!(normalize_path("/{code:[a-z]{3}}/x"), "/{code}/x");
}

/// JAX-RS: class `@Path` joined with method `@Path`, one endpoint per verb
/// annotation; a sub-resource locator (`@Path` without a verb) is not an
/// endpoint, and constants of the file (interface ones included) resolve.
#[test]
fn test_jaxrs_endpoints() {
    let source = r#"
package com.shop;

interface Paths {
    String ITEMS = "items";
}

@Path(Paths.ITEMS)
public class ItemsResource {
    private static final String BY_ID = "/{id: [0-9]+}";

    @GET
    public Response readAll() { return null; }

    @GET
    @Path(BY_ID)
    public Response read(@PathParam("id") long id) { return null; }

    @POST
    @PUT
    public Response write(Item item) { return null; }

    @Path("sub")
    public SubResource sub() { return null; }
}
"#;
    assert_eq!(
        endpoints("java", source),
        vec![
            "GET /items -> com.shop.ItemsResource.readAll",
            "GET /items/{id} -> com.shop.ItemsResource.read",
            "POST /items -> com.shop.ItemsResource.write",
            "PUT /items -> com.shop.ItemsResource.write",
        ]
    );
}

/// Spring MVC: class `@RequestMapping` base, verb-specific mappings,
/// `path`/`value` attributes, arrays and `@RequestMapping(method = …)`.
#[test]
fn test_spring_endpoints() {
    let source = r#"
package com.shop;

@RestController
@RequestMapping("/api")
public class OwnerController {
    @GetMapping({ "/owners", "/owners/" })
    public List<Owner> list() { return null; }

    @PostMapping(path = "/owners/{id}")
    public void save() {}

    @PatchMapping(value = "/owners/{id}", consumes = { "a", "b" })
    public void patch() {}

    @RequestMapping(value = "/legacy", method = { RequestMethod.GET, RequestMethod.HEAD })
    public void legacy() {}

    @RequestMapping("/any")
    public void any() {}

    @DeleteMapping
    public void deleteAll() {}

    public void helper() {}
}
"#;
    assert_eq!(
        endpoints("java", source),
        vec![
            "ANY /api/any -> com.shop.OwnerController.any",
            "DELETE /api -> com.shop.OwnerController.deleteAll",
            "GET /api/legacy -> com.shop.OwnerController.legacy",
            "GET /api/owners -> com.shop.OwnerController.list",
            "HEAD /api/legacy -> com.shop.OwnerController.legacy",
            "PATCH /api/owners/{id} -> com.shop.OwnerController.patch",
            "POST /api/owners/{id} -> com.shop.OwnerController.save",
        ]
    );
}

/// Declarative HTTP clients describe calls, not served endpoints.
#[test]
fn test_http_client_interfaces_are_not_endpoints() {
    let source = r#"
package com.shop;

@FeignClient(name = "items")
public interface ItemsClient {
    @GetMapping("/items")
    List<Item> items();
}

@RegisterRestClient
@Path("/stock")
public interface StockClient {
    @GET
    Stock stock();
}
"#;
    assert!(endpoints("java", source).is_empty());
}

/// Kotlin Spring: class annotations the grammar parses before the class,
/// array literals and `const val` (top level and companion) with templates.
#[test]
fn test_kotlin_spring_endpoints() {
    let source = r#"
package blog

const val API = "/api"

@RestController
@RequestMapping("$API/article")
class ArticleController(private val repository: ArticleRepository) {

    @GetMapping("/")
    fun findAll() = repository.findAllByOrderByAddedAtDesc()

    @GetMapping(SLUG)
    fun findOne(@PathVariable slug: String) = repository.findBySlug(slug)

    @PostMapping(value = ["/a", "/b"])
    fun post() {}

    companion object {
        const val SLUG = "/{slug}"
    }
}
"#;
    assert_eq!(
        endpoints("kotlin", source),
        vec![
            "GET /api/article -> blog.ArticleController.findAll",
            "GET /api/article/{slug} -> blog.ArticleController.findOne",
            "POST /api/article/a -> blog.ArticleController.post",
            "POST /api/article/b -> blog.ArticleController.post",
        ]
    );
}

/// Endpoints sit on their handler's line inside its type, so the CONTAINS
/// auto-link attaches them to it.
#[test]
fn test_endpoint_entity_shape() {
    let entities = extract(
        "java",
        "package com.shop;\n\n@Path(\"items\")\npublic class ItemsResource {\n    /** Lists items. */\n    @GET\n    public Response readAll() { return null; }\n}\n",
    );
    let endpoint = entities
        .iter()
        .find(|e| e.kind == EntityKind::HttpEndpoint)
        .unwrap();
    assert_eq!(endpoint.name, "GET /items");
    assert_eq!(endpoint.fqn, "com.shop.ItemsResource.readAll[GET /items]");
    assert_eq!(endpoint.enclosing_class.as_deref(), Some("ItemsResource"));
    assert_eq!(
        endpoint.enclosing_class_fqn.as_deref(),
        Some("com.shop.ItemsResource")
    );
    let handler = entities.iter().find(|e| e.name == "readAll").unwrap();
    assert_eq!(endpoint.start_line, handler.start_line);
    assert!(endpoint.docstring.as_deref().is_some_and(|d| {
        d.starts_with("HTTP endpoint GET /items, served by ItemsResource.readAll.")
    }));
}

/// Write `files` under a temporary repository root, parse the ones listed
/// in `parse` and return the sorted endpoint names.
fn parsed_endpoint_names(files: &[(&str, &str)], parse: &[&str]) -> Vec<String> {
    let root = tempfile::tempdir().unwrap();
    for (path, source) in files {
        let full = root.path().join(path);
        std::fs::create_dir_all(full.parent().unwrap()).unwrap();
        std::fs::write(full, source).unwrap();
    }
    let cfg = crate::pipeline::parser::ParseConfig {
        repo_root: root.path().to_path_buf(),
        repo_name: "test-repo".to_string(),
        ..Default::default()
    };
    let paths: Vec<std::path::PathBuf> = parse.iter().map(|p| root.path().join(p)).collect();
    let mut names: Vec<String> = crate::pipeline::parser::parse_files(&paths, &cfg)
        .into_iter()
        .filter(|e| e.kind == EntityKind::HttpEndpoint)
        .map(|e| e.name)
        .collect();
    names.sort();
    names
}

const PATHS_JAVA: (&str, &str) = (
    "src/main/java/com/shop/api/Paths.java",
    "package com.shop.api;\n\npublic final class Paths {\n    public static final String ROOT = \"/api\";\n    public static final String ITEMS = ROOT + \"/items\";\n}\n",
);

const RESOURCE_JAVA: (&str, &str) = (
    "src/main/java/com/shop/web/ItemsResource.java",
    "package com.shop.web;\n\nimport com.shop.api.Paths;\n\n@Path(Paths.ITEMS)\npublic class ItemsResource {\n    @GET\n    @Path(\"{id}\")\n    public Item read(long id) { return null; }\n\n    @DELETE\n    @Path(Missing.GONE)\n    public void gone() {}\n}\n",
);

/// Issue #33: a route constant declared in another file resolves through
/// the imports, whether that file is parsed in the same batch or only
/// found on disk (an incremental run that re-parses the resource alone).
/// A constant found nowhere keeps its placeholder.
#[test]
fn test_route_constants_from_other_files() {
    let expected = vec!["DELETE /api/items/<Missing.GONE>", "GET /api/items/{id}"];
    let files = [PATHS_JAVA, RESOURCE_JAVA];
    assert_eq!(
        parsed_endpoint_names(&files, &[PATHS_JAVA.0, RESOURCE_JAVA.0]),
        expected
    );
    assert_eq!(parsed_endpoint_names(&files, &[RESOURCE_JAVA.0]), expected);
}

/// A renamed endpoint keeps its route and gets the FQN and UUID of its new
/// name.
#[test]
fn test_renamed_endpoint_identity() {
    let entities = extract(
        "java",
        "package com.shop;\n\n@Path(Other.BASE)\npublic class R {\n    @GET\n    public void get() {}\n}\n",
    );
    let endpoint = entities
        .iter()
        .find(|e| e.kind == EntityKind::HttpEndpoint)
        .unwrap();
    assert_eq!(endpoint.name, "GET /<Other.BASE>");
    let renamed = super::rename_endpoint(endpoint, "GET /base");
    assert_eq!(renamed.fqn, "com.shop.R.get[GET /base]");
    assert_ne!(renamed.uuid, endpoint.uuid);
    assert_eq!(
        renamed.docstring.as_deref(),
        Some("HTTP endpoint GET /base, served by R.get.")
    );
    assert!(matches!(
        renamed.reference_intents.as_slice(),
        [ReferenceIntent::RoutesTo { .. }]
    ));
}
