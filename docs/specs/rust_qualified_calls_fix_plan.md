# Rust Qualified Calls — FQN & Receiver Fix Plan

## Problema

El indexador no captura llamadas de la forma `Type::method(...)` desde Rust
cuando el método se llama de forma cualificada y el nombre del método es
ambiguo en el codebase.

Ejemplo concreto observado contra el propio repo `knot`:

- Definición: `KnotMcpHandler::new(...)` en `src/mcp_handler.rs:42`.
- Llamada real: `src/bin/knot-mcp.rs:102` invoca `KnotMcpHandler::new(...)`.
- Resultado actual: `knot callers new --repo knot` no incluye `main` como
  caller. La llamada nunca se materializa como edge `CALLS` en Neo4j.

Contraste: `KnotMcpHandler::new_dry_run()` (mismo binario, línea 100) **sí**
se resuelve, porque `new_dry_run` es un nombre único en el codebase y cae
por la rama de fallback por nombre único.

## Causa raíz (dos bugs combinados)

### Bug 1 — FQN de RustMethod no incluye el target del `impl`

- `src/pipeline/parser/context.rs::extract_class_contexts` solo reconoce los
  nodos `class_declaration | interface_declaration |
  abstract_class_declaration | class_definition | object_declaration`.
- No reconoce el nodo `impl_item` de tree-sitter-rust.
- `compute_fqn_and_context` (rama `RustMethod`, `context.rs:147-153`) cae
  siempre por el `else`, produciendo FQN = `new` (en vez de
  `KnotMcpHandler::new`) y `enclosing_class = None`.
- Para C++ ya existe un parche posterior en `extractor.rs:643-649`
  (`cpp::build_cpp_fqn`); para Rust no hay equivalente.

### Bug 2 — Receiver descartado en `scoped_identifier`

- `src/pipeline/parser/languages/rust.rs::extract_from_scoped_identifier`
  devuelve solo el último identificador del path.
- `extract_call_details` (rust.rs:694-699) fuerza `receiver = None` para
  llamadas via `scoped_identifier`.
- Para `KnotMcpHandler::new(...)` se registra
  `Call { method: "new", receiver: None }`, perdiéndose el contexto de tipo.

### Efecto en `resolve_single_call_intent`

Para `main` llamando `KnotMcpHandler::new(...)`:

| Strategy | Condición | Resultado |
|---|---|---|
| 1: Local (`self.foo()`) | requiere `enclosing_class` en caller | `main` es top-level → falla |
| 2: Tipo en mayúscula | requiere `receiver` no-None | `receiver = None` → no aplica |
| 3: Instancia | requiere `receiver` | `receiver = None` → no aplica |
| 4: Fallback por nombre | `uuids.len() == 1` o `arg_count` único | hay muchos `new` → falla |

Por eso `new_dry_run` se resuelve y `new` no.

## Decisiones aprobadas

- **Heurística receiver scoped**: poblar receiver solo si el primer carácter
  es mayúscula ASCII. Coherente con Strategy 2 del resolver y evita falsos
  positivos contra módulos lowercase (`std::env`, `crate::mcp_handler`).
- **Tests E2E**: extender `tests/run_rust_e2e.sh` y
  `tests/testing_files/sample.rs` (no crear suite separada).
- **Cobertura de `impl_item`**: soportar `impl Foo` y `impl Trait for Foo`,
  ignorando genéricos. `impl Foo::Bar<T>` y casos exóticos fuera de alcance.
- **`Self::method()`**: traducir `Self` al `enclosing_class` del caller
  directamente en el parser. Más simple, una sola ubicación.
- **Compatibilidad de índices**: documentar en README y CHANGELOG que tras
  esta versión se requiere `knot-indexer --clean` una vez. Sin
  auto-migración.

## Fase 1 — E2E de regresión (debe fallar antes del fix)

### Fixtures (`tests/testing_files/sample.rs`)

- Mantener `Counter::with_label(10, "main")` (ya existe en línea 199).
- Añadir bloque con dos structs homónimos `WidgetA` / `WidgetB`, ambos con
  `fn new()`, y una función llamadora externa que invoca `WidgetA::new()`.
- Añadir caso `impl Trait for Type`:
  - `struct Logger`.
  - `impl LogSink for Logger { fn new() -> Self { ... } }`.
  - Llamada `Logger::new()` desde otra función.

### Script (`tests/run_rust_e2e.sh`)

- Test: `knot callers with_label -r rust_e2e_test_repo` incluye `main` como
  caller.
- Test: `knot callers new -r rust_e2e_test_repo` distingue `WidgetA::new` de
  `WidgetB::new` y atribuye la llamada a `WidgetA::new`. Validamos que el
  target devuelto contiene `WidgetA` en su FQN/path.
- Test: `Logger::new` (en `impl Trait for Type`) aparece como entidad con
  FQN `Logger::new`.

## Fase 2 — Tests unitarios

### `src/pipeline/parser/languages/rust.rs#tests`

- `test_extract_scoped_call_returns_receiver`: `KnotMcpHandler::new(...)` →
  `("new", Some("KnotMcpHandler"))`.
- `test_extract_scoped_call_multi_segment_uppercase`:
  `crate::mcp_handler::KnotMcpHandler::new(...)` →
  `("new", Some("KnotMcpHandler"))`.
- `test_extract_scoped_call_lowercase_module_drops_receiver`:
  `std::env::set_var(...)` → `("set_var", None)`.
- `test_extract_scoped_call_self_translated_to_enclosing_class`:
  `Self::new()` dentro de `impl Foo` → `("new", Some("Foo"))`.
- `test_rust_method_fqn_includes_impl_target_inherent`:
  `impl Foo { fn new() {} }` → entidad con `fqn == "Foo::new"`,
  `enclosing_class == Some("Foo")`.
- `test_rust_method_fqn_includes_impl_target_trait_for`:
  `impl Bar for Foo { fn new() {} }` → entidad con `fqn == "Foo::new"`,
  `enclosing_class == Some("Foo")` (self-type, no trait).
- `test_rust_method_fqn_with_generics`:
  `impl<T> Foo<T> { fn new() {} }` → `fqn == "Foo::new"` (genéricos
  ignorados).

### `src/pipeline/parser/context.rs#tests`

- `test_extract_class_contexts_includes_rust_impl_item`: parsear
  `impl Foo { ... }` añade `ClassContext { name: "Foo", ... }`.
- `test_extract_class_contexts_rust_impl_trait_for_uses_self_type`:
  `impl Bar for Foo { ... }` → `ClassContext { name: "Foo", ... }`
  (no `"Bar"`).

### `src/pipeline/ingest/resolve.rs#tests`

- `test_resolve_rust_qualified_call_with_homonyms`: dos entidades con FQN
  `WidgetA::new` y `WidgetB::new`, intent
  `Call { method: "new", receiver: Some("WidgetA") }` → resuelve a
  `WidgetA::new`.
- `test_resolve_rust_self_method_call`: caller `Foo::bar` con
  `enclosing_class = Some("Foo")`, intent
  `Call { method: "helper", receiver: Some("self") }`, target `Foo::helper`
  → resuelve correctamente (Strategy 1).

## Fase 3 — Implementación

### Cambio A — `impl_item` como contexto FQN

Nuevo helper en `src/pipeline/parser/languages/rust.rs`:

```text
pub(crate) fn extract_impl_self_type(node: Node, source: &[u8]) -> Option<String>
```

Lógica:

- Para un `impl_item`, leer hijos buscando el self-type:
  - Si existe el keyword `for`, self-type = nodo de tipo **después** del
    `for`.
  - Si no, self-type = primer nodo de tipo del impl.
- Resolver el "nombre base" del tipo, ignorando:
  - genéricos (`generic_type` → tomar el `type_identifier` interno).
  - paths (`scoped_type_identifier` → último segmento).
  - referencias (`reference_type` → tipo interno).
  - lifetimes.
- Casos no soportados → `None` (skip silencioso, no romper indexación).

En `src/pipeline/parser/context.rs::extract_class_contexts` extender el
`match` con una rama explícita para `"impl_item"` que llame al helper
(patrón cross-module ya usado por `cpp::build_cpp_fqn` en
`extractor.rs:643-649`).

Con esto, `compute_fqn_and_context` (rama `RustMethod` en
`context.rs:147-153`) produce automáticamente `Foo::new` y rellena
`enclosing_class`.

### Cambio B — Preservar receiver en llamadas `Type::method`

En `src/pipeline/parser/languages/rust.rs`:

1. Cambiar firma:
   ```text
   fn extract_from_scoped_identifier(node, source) -> Option<(String, Option<String>)>
   ```
   - Recolectar todos los segmentos `identifier` del `scoped_identifier`
     recursivamente.
   - Último segmento = `method` (function name).
   - Penúltimo segmento = candidato a receiver. Sólo poblarlo si su primer
     carácter es mayúscula ASCII; en caso contrario `None`.
2. `extract_call_details` (rust.rs:694-699) propaga el par devuelto.
3. Caso especial `Self::method()`: si receiver == `"Self"`, sustituirlo por
   el `enclosing_class` del entity al que se atribuye la llamada (resuelto
   en el parser).

Casos extremos cubiertos:

- `foo()` (call sin `::`) → no es `scoped_identifier`, no afectado.
- `module::function()` (módulo lowercase) → `receiver = None`, cae por
  Strategy 4 igual que antes.
- `Type::method()` → `receiver = Some("Type")`, dispara Strategy 2 del
  resolver y resuelve vía `lookup_fqn`.

### Cambio C — Ganancia colateral

`enclosing_class` ahora se rellena para `RustMethod`. Strategy 1 del
resolver (`self.foo()` / `Self::foo()`) empieza a funcionar para Rust.
No requiere código extra, sólo el test correspondiente en Fase 2.

## Fase 4 — Verificación

```bash
cargo fmt
cargo clippy --all-targets -- -D warnings
cargo test
./tests/run_rust_e2e.sh
./tests/run_all_e2e.sh
```

Validación empírica final tras re-indexado limpio:

```bash
knot-indexer --clean
knot callers new --repo knot
# Esperado: main (src/bin/knot-mcp.rs:102) listado como caller de
# KnotMcpHandler::new (src/mcp_handler.rs:42).
```

## Documentación

- `README.md`: nueva sección "Rust method FQN format" describiendo el
  formato `Type::method` y el requisito de `--clean` tras el upgrade.
- `CHANGELOG.md`: entrada bajo "Breaking changes" describiendo el cambio de
  FQN y el requisito de re-index.

## Riesgos

- `extract_class_contexts` se invoca para todos los lenguajes. Añadir
  `"impl_item"` solo afecta a Rust (Java/Kotlin/Python/C++ no producen ese
  nodo). Riesgo: bajo.
- Cambio de FQN invalida índices Rust existentes. Requiere `--clean`. Sin
  auto-migración por decisión consciente.
- Posible ajuste en aserciones de `run_rust_e2e.sh` si dependían del FQN
  antiguo `new` (revisar antes de mergear).

## Criterios de aceptación

- Tests unitarios nuevos en Fase 2 pasan.
- Test E2E nuevo pasa: `WidgetA::new` resuelve correctamente vs
  `WidgetB::new`.
- Test E2E nuevo pasa: `Logger::new` (impl trait for) tiene FQN cualificado.
- Verificación empírica: `knot callers new --repo knot` muestra `main`
  (`src/bin/knot-mcp.rs:102`) como caller de `KnotMcpHandler::new`.
- `./tests/run_all_e2e.sh` no introduce regresiones en otros lenguajes.
- `cargo clippy --all-targets -- -D warnings` y `cargo fmt -- --check`
  limpios.
