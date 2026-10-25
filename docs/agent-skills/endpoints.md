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

- **Context path**: paths are relative to the application;
  `server.servlet.context-path` and `@ApplicationPath` are deployment
  configuration and are not applied.
- **Declarative clients** (`@FeignClient`, `@RegisterRestClient`
  interfaces): their mappings are calls they make, not endpoints.
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
