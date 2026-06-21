# Spec: Resolución de llamadas a constructor en Python (`ClassName(...)` → `ClassName.__init__`)

**Status:** Proposed
**Owner:** Raul Tovar
**Created:** 2026-06-21
**Affected components:** `src/pipeline/ingest/resolve/`

---

## 1. Problema

Cuando una clase Python se instancia con `ClassName(...)`, el resolver de `knot-indexer`
enlaza esa llamada al entity de la **clase** y no al método `__init__` de esa clase.
Resultado: `find_callers ClassName.__init__` pierde a todos los callers reales que
instancian la clase.

### Caso reproducible (LlamaFactory)

- Definición: `Muon.__init__` en
  `src/llamafactory/third_party/muon/muon.py:102` (clase `Muon` declarada en línea 76).
- Caller: `_create_muon_optimizer` en
  `src/llamafactory/train/trainer_utils.py:513` ejecuta
  `Muon(lr=..., wd=..., muon_params=..., adamw_params=..., adamw_betas=..., adamw_eps=...)`.

### Estado observado tras indexar

```
$ knot callers Muon.__init__
# Calls (1): __init__ → __init__   (solo el super().__init__() interno)

$ knot callers Muon
# Calls (3): _create_muon_optimizer → Muon (clase)
#           step → Muon.adjust_lr_for_muon
#           __init__ → Muon.__init__
```

El edge `_create_muon_optimizer → Muon.__init__` **debería existir** y no existe.

---

## 2. Causa raíz

`src/pipeline/ingest/resolve/calls.rs::resolve_single_call_intent` recibe el intent

```rust
Call { method: "Muon", receiver: None, arg_count: Some(6), .. }
```

y, en la rama `intent.receiver.is_none()` (líneas 189-206), busca en
`name_to_uuids["Muon"]` y devuelve el UUID del entity `Muon` (la clase). No existe
ningún paso posterior que, sabiendo que el target es una clase, redirija la relación
al método `__init__` de esa clase.

En Python, invocar una clase **es** invocar a su constructor; el resolver debe modelar
esa semántica.

---

## 3. Solución propuesta — redirección post-resolución

Aplicar la redirección **después** de que `resolve_single_call_intent` devuelva un
UUID. Esto:

- No requiere tocar ningún parser de lenguaje.
- Es trivialmente extensible a otros lenguajes con constructores
  (`<init>` Java, `constructor` JS/TS) sin reescribir parsers.
- No introduce ambigüedad: solo dispara cuando el UUID resuelto pertenece a una clase
  con `__init__` explícito (o heredado).

### Algoritmo

```
fn redirect_class_call_to_constructor(uuid, ctx) -> uuid:
    if ctx.uuid_to_kind[uuid] not in {Class, Struct}:
        return uuid
    let class_name = ctx.uuid_to_name[uuid]
    if let Some(init_uuid) = lookup_fqn(class_name, "__init__", ctx.fqn_to_uuid):
        return init_uuid
    # Buscar en la cadena de herencia
    for parent in ctx.extends_map.get(class_name).unwrap_or(&[]):
        if let Some(init_uuid) = lookup_fqn(parent, "__init__", ctx.fqn_to_uuid):
            return init_uuid
    uuid  # sin __init__ alguno → conservar el comportamiento actual
```

Solo aplica a `ReferenceIntent::Call` (no a `TypeReference`, `ValueReference`,
`Extends`, etc.). Los type hints como `optimizer: Muon` siguen apuntando a la clase.

---

## 4. Plan de implementación — **TDD/BDD estricto**

> **Regla:** ningún cambio de producción se commitea hasta que existan tests que
> **fallen** reproduciendo el bug. Solo después se implementa la corrección y se
> verifica que los tests pasen.

### Fase 0 — Escribir tests que reproduzcan el bug (deben fallar)

#### 0.1 Unit tests en `src/pipeline/ingest/resolve/calls.rs#tests`

Añadir, en este orden:

1. **`test_python_class_call_redirects_to_init`**
   - Setup: entity `class Foo` (kind=`Class`, fqn=`mod.Foo`), entity método
     `__init__` (fqn=`mod.Foo.__init__`, enclosing_class=`Foo`), entity caller
     `factory` con `Call { method: "Foo", receiver: None, .. }`.
   - Aserción: `factory.relationships` contiene `(__init__.uuid, Calls)` y
     **no** contiene `(Foo.uuid, Calls)`.
   - **Debe fallar** antes del fix (la relación apunta a `Foo`, no a `__init__`).

2. **`test_python_class_call_no_init_keeps_class`**
   - Setup: `class Bar` sin método `__init__`. Caller hace `Bar()`.
   - Aserción: la relación apunta a la clase `Bar` (comportamiento legacy preservado).
   - **Debe pasar** antes y después del fix (regresión guard).

3. **`test_python_class_call_inherited_init`**
   - Setup: `class Parent` con `Parent.__init__`; `class Child(Parent)` sin
     `__init__` propio. Caller hace `Child(...)`.
   - Aserción: la relación apunta a `Parent.__init__`.
   - **Debe fallar** antes del fix.

4. **`test_function_named_like_class_not_redirected`**
   - Setup: módulo con función `foo` (kind=`Function`, no clase). Caller hace `foo()`.
   - Aserción: la relación apunta a la función `foo`. No se busca ningún
     `foo.__init__`.
   - **Debe pasar** antes y después (regresión guard contra falsos positivos).

5. **`test_python_class_call_with_existing_init_no_duplicate`**
   - Setup: caller con dos intents en `reference_intents`: `Call { method: "Foo", .. }`
     y `Call { method: "__init__", receiver: Some("Foo"), .. }`.
   - Aserción: tras la resolución hay **exactamente una** relación
     `(Foo.__init__, Calls)`.
   - Garantiza que el `seen: HashSet` de `mod.rs:156` deduplica correctamente cuando
     redirect y parser apuntan al mismo target.
   - **Debe fallar o duplicar** antes del fix dependiendo del orden de inserción.

#### 0.2 E2E regresión en `tests/testing_files/python/sample.py`

Añadir fixture mínimo:

```python
# ============================================================
# Regression: ClassName(...) must resolve to ClassName.__init__
# ============================================================
class MuonLike:
    def __init__(self, lr, wd):
        self.lr = lr
        self.wd = wd

def create_muon_like():
    return MuonLike(lr=1e-3, wd=0.1)   # caller esperado de MuonLike.__init__
```

#### 0.3 Aserción E2E en `tests/run_python_e2e.sh`

Añadir un bloque que invoque el CLI y verifique:

```bash
echo "Test: class instantiation resolves to __init__ caller"
$KNOT_BIN callers "MuonLike.__init__" --repo "$REPO_NAME" \
  | grep -q "create_muon_like" || fail "MuonLike.__init__ should list create_muon_like as caller"
```

Replicar la misma aserción contra `knot-mcp` (mismo patrón ya usado en el suite).

#### 0.4 Verificación de fallo

```bash
cargo test --lib pipeline::ingest::resolve::calls   # tests 1, 3, 5 deben FALLAR
./tests/run_python_e2e.sh                            # aserción E2E debe FALLAR
```

**No avanzar a Fase 1 hasta confirmar que los tests fallan por la razón correcta.**

---

### Fase 1 — Extender `ResolutionContext`

Archivo: `src/pipeline/ingest/resolve/context.rs`

```rust
pub struct ResolutionContext<'a> {
    // ...campos existentes...
    pub uuid_to_kind: Option<&'a HashMap<Uuid, EntityKind>>,
    pub uuid_to_name: Option<&'a HashMap<Uuid, String>>,
}
```

Justificación de añadir `uuid_to_name`: derivar el nombre desde `uuid_to_fqn`
implica parsear separadores `::`/`.` y es frágil ante FQNs con módulos anidados.
Un mapa explícito es O(1) y robusto.

---

### Fase 2 — Poblar los nuevos mapas

Archivo: `src/pipeline/ingest/resolve/mod.rs` (función
`resolve_reference_intents_with_context`, alrededor de la línea 97-103).

```rust
let uuid_to_kind: HashMap<Uuid, EntityKind> = entities
    .iter()
    .map(|e| (e.uuid, e.kind.clone()))
    .collect();

let uuid_to_name: HashMap<Uuid, String> = entities
    .iter()
    .map(|e| (e.uuid, e.name.clone()))
    .collect();
```

Pasar referencias a estos mapas en la construcción del `ResolutionContext`
(línea 144).

---

### Fase 3 — Implementar `redirect_class_call_to_constructor`

Archivo: `src/pipeline/ingest/resolve/calls.rs`

```rust
pub(crate) fn redirect_class_call_to_constructor(
    uuid: Uuid,
    ctx: &ResolutionContext,
) -> Uuid {
    let Some(kind_map) = ctx.uuid_to_kind else { return uuid; };
    let Some(name_map) = ctx.uuid_to_name else { return uuid; };

    let Some(kind) = kind_map.get(&uuid) else { return uuid; };
    if !matches!(kind, EntityKind::Class | EntityKind::Struct) {
        return uuid;
    }

    let Some(class_name) = name_map.get(&uuid) else { return uuid; };

    // Lookup directo en la clase
    if let Some(init_uuid) = lookup_fqn(class_name, "__init__", ctx.fqn_to_uuid) {
        return init_uuid;
    }

    // Lookup vía herencia
    if let Some(parents) = ctx.extends_map.get(class_name) {
        for parent in parents {
            if let Some(init_uuid) = lookup_fqn(parent, "__init__", ctx.fqn_to_uuid) {
                return init_uuid;
            }
        }
    }

    uuid
}
```

---

### Fase 4 — Aplicar la redirección en `mod.rs`

En el match de `ReferenceIntent::Call` (líneas 159-181), envolver el resultado:

```rust
ReferenceIntent::Call { method, receiver, arg_count, .. } => {
    let call_intent = crate::models::CallIntent { /* ... */ };
    let resolved = calls::resolve_single_call_intent(&call_intent, /* ... */, &ctx)
        .map(|uuid| calls::redirect_class_call_to_constructor(uuid, &ctx));
    (resolved, RelationshipType::Calls)
}
```

---

### Fase 5 — Verificar que los tests pasan

```bash
cargo fmt
cargo clippy --all-targets -- -D warnings
cargo test --lib pipeline::ingest::resolve     # incluye los 5 unit tests nuevos
./tests/run_python_e2e.sh                      # E2E Python pasa
./tests/run_all_e2e_fast.sh                    # ningún otro lenguaje rompe
```

---

### Fase 6 — Validación manual sobre LlamaFactory

```bash
# Re-indexar LlamaFactory
./target/release/knot-indexer --clean --repo-path /var/lib/knot/repos/LlamaFactory \
  --repo-name LlamaFactory

# Confirmar el fix
./target/release/knot callers "Muon.__init__" | grep _create_muon_optimizer
```

Resultado esperado: `_create_muon_optimizer` aparece como caller de `Muon.__init__`
en `trainer_utils.py:498` (o la línea de la función contenedora; el detalle de la
línea exacta del intent es 513).

---

## 5. Edge cases cubiertos

| Caso | Comportamiento | Cubierto por |
|---|---|---|
| Clase sin `__init__` | Relación queda en la clase | Test 2 |
| Type hint `x: Foo` | `TypeReference`, no afectado | Por construcción (intent distinto) |
| Herencia: `Child()` sin `__init__` propio | Resuelve a `Parent.__init__` | Test 3 |
| Función con nombre de clase | No redirige | Test 4 |
| `super().__init__()` interno | Resuelto antes en rama `self`/`super` | Comportamiento legacy intacto |
| Cross-repo: clase en repo dependencia | Funciona si `__init__` está en `fqn_to_uuid` global | `load_entity_mappings` ya carga deps |
| Duplicado por parser + redirect | Deduplicado por `seen: HashSet` en `mod.rs:156` | Test 5 |

---

## 6. Riesgos y mitigaciones

| Riesgo | Probabilidad | Impacto | Mitigación |
|---|---|---|---|
| Redirección rompe Java/Kotlin/TS | Baja | Alto | Test 4 + `run_all_e2e_fast.sh` antes de mergear |
| Doble relación (parser ya emite `__init__`) | Media | Bajo | Test 5 + `seen: HashSet` existente |
| `EntityKind::Struct` aplica a Rust (no usa `__init__`) | Baja | Bajo | Lookup `<Struct>.__init__` simplemente no encuentra nada → retorna UUID original |
| Performance | Muy baja | Bajo | Lookup adicional es O(1); solo dispara para `Call` resueltos a `Class`/`Struct` |

---

## 7. Métrica de éxito

- Antes del fix: `knot callers Muon.__init__` devuelve **1 caller** (el self-call).
- Después del fix: devuelve **al menos 2 callers**, incluyendo `_create_muon_optimizer`.
- Todos los suites E2E de la matriz `run_all_e2e_fast.sh` siguen verdes.
- Los 5 unit tests nuevos pasan.

---

## 8. Out of scope (no incluido en este spec)

- Soporte de `<init>` en Java o `constructor` en JS/TS (puede añadirse extendiendo
  la lista de nombres en `redirect_class_call_to_constructor` en un spec posterior).
- Resolución de `Foo.from_string(...)` (factory classmethods) a `__init__` —
  esto no es semánticamente equivalente.
- Reescribir el parser de Python para emitir `Call { method: "__init__", receiver: Some(cls) }`
  directamente. Más invasivo y duplicaría lógica con otros lenguajes.

---

## 9. Referencias

- `src/pipeline/ingest/resolve/calls.rs:85` — `resolve_single_call_intent`
- `src/pipeline/ingest/resolve/mod.rs:153` — bucle de resolución de intents
- `src/pipeline/ingest/resolve/context.rs` — `ResolutionContext`
- `AGENTS.md` — sección "Fixing a Bug" (E2E regression test first)
