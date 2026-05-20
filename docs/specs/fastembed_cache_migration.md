# Especificación: Migración del Caché de Fastembed al Directorio `.knot/`

## Contexto

Actualmente, la librería `fastembed` (utilizada para generar vectores de embeddings) almacena por defecto sus modelos descargados en un directorio `.fastembed_cache/` relativo al directorio de trabajo actual (CWD) o en la ruta definida por `FASTEMBED_CACHE_DIR` / `HF_HOME`. 

Esto provoca que los artefactos generados por `knot` se dispersen fuera del directorio `.knot/`, que fue diseñado para contener todo el estado y artefactos específicos de `knot` (como `index_state.json`).

## Objetivo

Asegurar que todos los artefactos generados por `knot`, en particular los modelos descargados por `fastembed`, se guarden exclusivamente dentro del directorio `.knot/` en la raíz del repositorio indexado, manteniendo el entorno de trabajo limpio.

## Decisión de Diseño (Opción C - Híbrida)

Se ha elegido una estrategia híbrida:
1. **Comportamiento por defecto:** El caché de `fastembed` se almacenará en `<repo_path>/.knot/fastembed_cache/`. Cada repositorio tendrá su propia copia del modelo (aprox. 23MB para `AllMiniLML6V2`), lo que mantiene los repositorios completamente autocontenidos.
2. **Escape Hatch (Override):** Se introducirá una variable de entorno `KNOT_FASTEMBED_CACHE_DIR` que permitirá a los usuarios sobrescribir la ruta del caché. Esto es útil para usuarios o entornos de CI/CD que prefieran compartir un único modelo (ej. `~/.cache/knot/fastembed_cache/`) entre múltiples repositorios para ahorrar espacio.

## Plan de Implementación

### 1. Modificar Helper de Rutas en `src/pipeline/state.rs`

Añadir una función pública que determine la ruta del caché:

```rust
use std::env;

/// Obtiene la ruta del caché de fastembed.
/// Prioriza la variable de entorno `KNOT_FASTEMBED_CACHE_DIR`.
/// Si no está definida, usa `<repo_path>/.knot/fastembed_cache/`.
pub fn fastembed_cache_dir(repo_path: &str) -> PathBuf {
    if let Ok(custom_dir) = env::var("KNOT_FASTEMBED_CACHE_DIR") {
        return PathBuf::from(custom_dir);
    }
    Path::new(repo_path).join(STATE_DIR).join("fastembed_cache")
}
```

### 2. Modificar la Inicialización del `Embedder`

**Archivo:** `src/pipeline/embed.rs`

Actualizar la firma de `Embedder::init()` para aceptar el directorio de caché y configurar `InitOptions`:

```rust
// Para #[cfg(feature = "indexer")]
pub fn init(cache_dir: std::path::PathBuf) -> Result<Self> {
    // Asegurar que el directorio exista
    std::fs::create_dir_all(&cache_dir).context("Failed to create fastembed cache dir")?;
    
    let options = InitOptions::new(DEFAULT_MODEL)
        .with_cache_dir(cache_dir)
        .with_show_download_progress(true);
        
    let model = TextEmbedding::try_new(options)
        .context("Failed to initialise fastembed TextEmbedding model")?;
    // ...
}

// Para #[cfg(not(feature = "indexer"))]
pub fn init(_cache_dir: std::path::PathBuf) -> Result<Self> {
    // ...
}
```

### 3. Propagar la Configuración en los Binarios

- **`knot-indexer`** (`src/pipeline/runner.rs`):
  Pasar el `repo_path` (desde `&cfg.repo_path`) al momento de inicializar el `Embedder` dentro de `run_indexing_pipeline()`.
  
- **`knot-mcp`** (`src/mcp_handler.rs` y `src/bin/knot-mcp.rs`):
  Actualizar `KnotMcpHandler::new()` para recibir `cache_dir: std::path::PathBuf`. En `knot-mcp.rs`, utilizar `fastembed_cache_dir(&cfg.repo_path)` y pasarlo al handler.
  
- **`knot` (CLI)** (`src/bin/knot.rs`):
  Utilizar `fastembed_cache_dir(&cfg.repo_path)` y pasarlo a `Embedder::init()` en `main()`.

### 4. Limpieza y Documentación

- **`.gitignore`**: Eliminar la entrada `.fastembed_cache/` (o dejarla por retrocompatibilidad, pero asegurarse de que `.knot/` cubre la nueva ruta).
- **`README.md`**: Actualizar la documentación para reflejar que el modelo de embeddings se almacena en `.knot/fastembed_cache/`.
- **`AGENTS.md`**: Añadir una nota indicando la nueva estructura de `.knot/`.
- **`.env.example`**: Añadir la entrada opcional `# KNOT_FASTEMBED_CACHE_DIR=~/.cache/knot/fastembed_cache` documentada.

### 5. Pruebas (Tests)

- Actualizar el test unitario `test_embedder_init_and_embed_basic` en `embed.rs` para pasar un directorio temporal como `cache_dir`.
- Escribir un test unitario para `fastembed_cache_dir` en `state.rs` validando la variable de entorno y el fallback por defecto.
- Ejecutar la suite de tests E2E para confirmar que todo funciona correctamente y que el modelo se descarga en el lugar correcto.
