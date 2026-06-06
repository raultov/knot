# Rust Macro Calls and Test Context Fix Plan

## Problema

Actualmente, `find_callers` sobre código Rust (como `is_supported_file`) ignora la inmensa mayoría de las llamadas reales desde unit tests y, en ocasiones, desde código de producción. Por ejemplo, de 33 llamadas a `is_supported_file`, el indexador solo captura 1.

Se han diagnosticado **dos causas raíz ortogonales** que explican este comportamiento asimétrico e incompleto y que se solucionarán juntas:

### A. Causa Raíz (Blind Spot en AST): Invocaciones dentro de Macros
Tree-sitter no parsea el interior de invocaciones de macros (`assert!`, `vec![]`, `println!`, etc.) como un AST estándar. El contenido de la macro se expone como un `token_tree` genérico sin semántica (e.g., sin nodos `call_expression`). Como `collect_rust_call_references` solo buscaba `call_expression`, perdía cualquier llamada envuelta en macros.

### B. Mejora Contextual: Módulos Inline y Tests
Los bloques `#[cfg(test)] mod tests { ... }` definen un sub-módulo que se ignoraba. Por tanto, todas las funciones de test vivían en el mismo FQN (Fully Qualified Name) del archivo, pudiendo chocar con funciones de otros tests en el mismo crate y sin que el cliente MCP supiese que esos "callers" provienen de código de prueba, no de código de producción.

## Diseño de la Solución Combinada

### 1. Fix A: Análisis de `token_tree` para llamadas a funciones
Modificar la fase de extracción (`collect_call_nodes` en `rust.rs`) para descender dentro de los `token_tree` y reconocer manualmente el patrón de una llamada a función.
- Si un `identifier` es inmediatamente seguido por un `token_tree` cuyo texto comienza por `(`, lo consideraremos un candidato a llamada y extraeremos el identificador como el nombre de la función / método llamado.

### 2. Fix B: FQN inline mod + `is_test_context` flag
- Añadir el booleano `is_test_context: false` por defecto en la estructura `ParsedEntity` y en su homóloga ligera `ResolutionEntity`.
- Añadir la lógica que parsea `mod_item` con cuerpo de tipo `declaration_list`. Esto permitirá llevar una pila ("stack") de contextos de módulos a la hora de procesar el archivo.
- Comprobar si al `mod_item` le antecede un atributo (`#[cfg(test)]` o `#[cfg_attr(test, ...)]`).
- Generar el FQN modificado mediante un nuevo helper de FQN combinando el "module_path" del archivo base con la jerarquía de los módulos inline encontrados en la misma posición (línea).
- Setear `is_test_context = true` a la entidad resultante si pertenece a una jerarquía que ha sido marcada como de test.

## Plan de Implementación (Orden Propuesto)

| Paso | Acción |
| --- | --- |
| 1 | Completar / finalizar la adaptación del modelo base (los `ParsedEntity` constructores que empezaron el proceso, asegurarse que no quedan fallos de compilación). |
| 2 | Refinar `inline_module_path_for_entity` y `extract_rust_module_contexts` (código que ya se escribió pero falta validar en `src/pipeline/parser/languages/rust.rs`). |
| 3 | Modificar `collect_call_nodes` para descender recursivamente en `token_tree`. Al iterar los hijos de un `token_tree`, detectar `identifier` seguido de `token_tree` cuyo texto comience por `(`. Añadir ese hallazgo a las `calls`. |
| 4 | Construir el script E2E (`tests/run_rust_test_module_e2e.sh`) ya propuesto con el fixture en `tests/testing_files/rust_test_module/` para validar tanto la extracción FQN (is_test_context y tests::) como la recuperación de las llamadas perdidas a `is_supported`. |
| 5 | Ejecutar clippy y `cargo test` locales. Correr E2E completo. |
| 6 | Integrar todo al servidor de CI o lanzar `./tests/run_all_e2e.sh`. |

## Validación

- **FQN**: El FQN de las entidades en `mod tests` deberá mostrar `crate::modulo::tests::nombre_funcion`.
- **Test Context**: Propiedad `is_test_context` debe exponerse en Neo4j como un atributo y ser verificable vía Cypher en el test de integración.
- **Callers Recuperados**: Un `cargo test` validará (gracias a los E2E) que ahora el número de llamadas entrantes se recupera a los niveles esperados (las llamadas que antes estaban enmascaradas bajo `assert!`).
