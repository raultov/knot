# Knot Endpoints: HTTP Endpoint Catalogue

**Command:** `knot endpoints [--method <verb>] [--path <path>] [--repo <name>]`
**MCP tool:** `list_endpoints`

## Purpose

List the HTTP endpoints the indexed Java and Kotlin code serves —
`<METHOD> <path>` with the handler method each one routes to. This is the
answer to "which endpoints does this service expose?" and "which handler
serves `GET /api/owners/42`?".

## Coverage

- **JAX-RS** (Jakarta/Javax, Quarkus, Jersey): class `@Path` joined with
  method `@Path`, one endpoint per verb annotation (`@GET`, `@POST`, `@PUT`,
  `@DELETE`, `@PATCH`, `@HEAD`, `@OPTIONS`). A method with `@Path` but no
  verb is a sub-resource locator, not an endpoint.
- **Spring MVC**: class `@RequestMapping` joined with `@GetMapping`,
  `@PostMapping`, `@PutMapping`, `@DeleteMapping`, `@PatchMapping` or
  `@RequestMapping(method = …)`. A `@RequestMapping` without `method` is
  listed as `ANY`. Path arrays yield one endpoint per path.
- Paths built from string constants resolve: literals, `+` concatenation,
  Kotlin templates (`"$API/items"`), and constants declared in the same
  file or in another file of the repository (found through the imports and
  the package).
- Path-variable patterns are dropped: `{id: [0-9]+}` → `{id}`.

## Not covered

- **Context path**: listed paths are relative to the application;
  `server.servlet.context-path` and `@ApplicationPath` are not applied
  here (HTTP client linking does take them into account, see below).
- **Declarative clients** (`@FeignClient`, `@RegisterRestClient`,
  `@HttpExchange` interfaces): their mappings are calls they make, not
  endpoints. They become HTTP client calls instead (see below).
- **Generated code**: interfaces generated at build time (e.g. by the
  OpenAPI generator) are not in the source tree, so their implementations
  carry no routing annotations to index.
- A constant that cannot be resolved stays visible as `<expression>`
  (`GET /api/<Routes.BASE>/items`).

## Parameters

- **`--method <verb>`** (optional): case-insensitive (`get`); `ANY`
  endpoints match every method.
- **`--path <path>`** (optional): matched segment by segment as a prefix —
  `/api/owners` matches `/api/owners` and `/api/owners/{id}`, never
  `/api/ownership`. An endpoint's `{variable}` segment matches any value,
  so a concrete URL path (`/api/owners/42`) finds its endpoint.
- **`--repo <name>`**: repository scope.

## Following up

- `knot callers "<handler FQN>"` — callers of the handler inside the
  repository; its **HTTP routes** section lists the endpoints routing to it,
  so a handler reached only over HTTP is never reported as unused.
- `knot callers "GET /api/owners/{id}"` — an endpoint is an entity too.
- `knot explore <file>` — a resource's endpoints are grouped under
  **HTTP Endpoints**.
- `knot search "GET /owners/{ownerId}"` — endpoints are searchable.

## HTTP callers (heuristic)

Every routed method of an OpenFeign (`@FeignClient`), MicroProfile REST
Client (`@RegisterRestClient`) or Spring HTTP interface (`@GetExchange`,
`@HttpExchange`) client, and every imperative call — `RestTemplate`
(`getForObject`, `postForEntity`, `exchange(url, HttpMethod.X, …)`, …) and
the fluent `WebClient` / `RestClient` API (`.get().uri(…)`,
`.method(HttpMethod.X).uri(…)`), Java and Kotlin — becomes an
`http_client_call` entity, named `<METHOD> <path>` with the client's base
URL path applied. An imperative call belongs to the method containing it. After every
indexing run, knot links each client call to the endpoint it hits with an
`HTTP_CALLS` edge, across every indexed repository and in any indexing
order — no build dependency between client and server is needed.

- `knot callers "<handler FQN>"` (or the endpoint name) lists the client
  calls under **HTTP callers (heuristic: client call → endpoint)**.
- `knot explore <client file>` groups them under **HTTP Client Calls**.

How a call is matched:

- **Base URL**: Feign `url`, `path`, `name`/`contextId`
  (`spring.cloud.openfeign.client.config.<name>.url`,
  `feign.client.config.<name>.url`); MicroProfile `baseUri`, `configKey`
  (`quarkus.rest-client.<key>.url`, `<key>/mp-rest/url`, also by
  interface name); an absolute `@HttpExchange` URL. `${prop:default}`
  placeholders resolve against the client repository's `application*` /
  `bootstrap*` `.yml|.yaml|.properties` and `microprofile-config.properties`
  (read even without `--include-config-files`; test sources excluded). An
  unresolved URL adds no path prefix.
- **Service**: the Feign service name and the URL host are matched against
  each server's `spring.application.name` / `quarkus.application.name`;
  when a server matches, only its endpoints are candidates.
- **Context path**: a server's context path (`server.servlet.context-path`,
  `spring.mvc.servlet.path`, `quarkus.http.root-path`, `@ApplicationPath`, …)
  may or may not appear in the client path.
- **Path and verb**: same verb (or `ANY` on either side), same segments, a
  `{variable}` matching any segment, and at least one literal segment
  matched exactly (`/{index}/embeddings` never links to `/items/{id}`). The most literal match wins, as the
  server would route it (`GET /items/{id}` beats `GET /items/export` for a
  client `GET /items/{id}`).
- **Ambiguity**: when several endpoints match equally well, nothing is
  linked.

Imperative calls:

- **Recognition** is textual (Kotlin has no declared receiver types): the
  `RestTemplate` method names, `exchange` / `execute` with an
  `HttpMethod.X` argument, and `.get()` / `.post()` / … /
  `.method(HttpMethod.X)` followed by `.uri(…)`. The generic `put` /
  `delete` count only on a receiver the file declares as `RestTemplate` /
  `RestOperations`. Comments and string literals are ignored.
- **URL**: literals, `+` concatenation, Kotlin templates, constants,
  `@Value("${…}")` fields and constructor parameters, `URI.create(…)`,
  `String.format(…)` / `.formatted(…)`, and `UriComponentsBuilder` or
  `uriBuilder -> uriBuilder.path(…)` lambdas. An unknown leading part (a
  `baseUrl` variable) stands for scheme and host; an unknown later part is
  a `{variable}` segment (`"/items/" + id` → `/items/{id}`); the query
  string is dropped. A URL without a literal segment is not recorded.
- **Base URL of a relative call**: the file's single `baseUrl(…)` /
  `rootUri(…)` builder call (`WebClient.builder()`, `RestClient.builder()`,
  `RestTemplateBuilder`); with several, it is unknown.
- **Test code** (`src/test/…`, `*Test.java`, `TestRestTemplate`) is
  skipped.

Not covered yet: Java's `HttpClient`, OpenAPI-generated clients, URLs
that only exist at runtime, and base URLs set in another file (they count
as unknown). A server's configuration
is re-read on every run, but a change to a client's configuration alone
does not refresh its calls: re-index with `--clean` to apply it.
