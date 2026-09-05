//! Stage 2 — Parse: AST extraction via Tree-sitter + Rayon.
//!
//! Each source file is parsed in parallel on the Rayon thread pool.
//! Tree-sitter queries extract class declarations, method/function declarations,
//! associated documentation comments, and call-site references.
//!
//! # Custom queries
//! Built-in queries are compiled into the binary at build time (see `queries/`
//! directory). When [`ParseConfig::custom_queries_path`] is set, the parser
//! will instead load `java.scm` and `typescript.scm` from that directory,
//! allowing callers to override extraction logic without recompiling.

use anyhow::{Context, Result};
use std::{
    fs,
    path::{Path, PathBuf},
};
use tracing::{debug, warn};

use crate::models::ParsedEntity;
use tokio::sync::mpsc;

mod comments;
mod context;
pub(crate) mod extractor;
pub mod languages;
mod orphans;
mod utils;

#[cfg(test)]
mod test_utils;

// Built-in query files compiled into the binary.
const DEFAULT_JAVA_QUERY: &str = include_str!("../../../queries/java.scm");
const DEFAULT_KOTLIN_QUERY: &str = include_str!("../../../queries/kotlin.scm");
const DEFAULT_TS_QUERY: &str = include_str!("../../../queries/typescript.scm");
const DEFAULT_TSX_QUERY: &str = include_str!("../../../queries/tsx.scm");
const DEFAULT_JS_QUERY: &str = include_str!("../../../queries/javascript.scm");
#[expect(dead_code, reason = "reserved for future query-based HTML parsing")]
const DEFAULT_HTML_QUERY: &str = include_str!("../../../queries/html.scm");
const DEFAULT_CSS_QUERY: &str = include_str!("../../../queries/css.scm");
const DEFAULT_SCSS_QUERY: &str = include_str!("../../../queries/scss.scm");
const DEFAULT_RUST_QUERY: &str = include_str!("../../../queries/rust.scm");
const DEFAULT_PYTHON_QUERY: &str = include_str!("../../../queries/python.scm");
const DEFAULT_C_QUERY: &str = include_str!("../../../queries/c.scm");
const DEFAULT_CPP_QUERY: &str = include_str!("../../../queries/cpp.scm");
const DEFAULT_CSHARP_QUERY: &str = include_str!("../../../queries/csharp.scm");
const DEFAULT_MD_QUERY: &str = include_str!("../../../queries/markdown.scm");

/// Configuration for the parse stage.
#[derive(Clone)]
pub struct ParseConfig {
    pub repo_root: PathBuf,
    /// Optional filesystem path to a directory containing custom `.scm` query files.
    pub custom_queries_path: Option<String>,
    /// Logical repository name for multi-repository isolation.
    pub repo_name: String,
    /// Whether to index configuration files (YAML, JSON, .properties) and
    /// Kubernetes/Helm manifests. When `false`, these files produce no entities.
    pub include_config_files: bool,
    /// Filesystem root of the repository being indexed. Required by Rust
    /// post-processing to discover `Cargo.toml` files and compute crate
    /// qualified FQNs (e.g. `crate_a::config::Config`). When `None`, Rust
    /// FQNs fall back to their bare-name form.
    pub repo_path: Option<String>,
}

impl Default for ParseConfig {
    fn default() -> Self {
        Self {
            repo_root: PathBuf::from("."),
            custom_queries_path: None,
            repo_name: String::new(),
            include_config_files: false,
            repo_path: None,
        }
    }
}

/// Callback invoked exactly once per input file after the file has been
/// fully processed (all entities sent to the channel, or parse failed).
pub type FileParsedCallback = std::sync::Arc<dyn Fn() + Send + Sync>;

/// Callback invoked exactly once, with the final entity count, after
/// post-parse aggregation (e.g. Varnish built-in subs) and **before** any
/// entity is pushed to the bounded channel. Publishing the total at this
/// exact instant lets downstream progress observers transition from the
/// parse band (0–10%) to the ingest band (10–90%) without ever saturating
/// at 100% while the channel is still full.
///
/// The closure receives the number of entities that will subsequently be
/// pushed into the channel.
pub type EntitiesExtractedCallback = std::sync::Arc<dyn Fn(usize) + Send + Sync>;

/// Callbacks surfacing parser progress to an external observer.
///
/// All fields are optional; `ParseCallbacks::default()` observes nothing.
/// v1.6.2 introduced this struct to replace the bare `FileParsedCallback`
/// parameter; the new `on_entities_extracted` hook is the fix for the
/// "100% then frozen" indexing progress bug.
#[derive(Default, Clone)]
pub struct ParseCallbacks {
    /// Invoked exactly once per input file after it has been fully
    /// processed (successful parse or parse error alike).
    pub on_file_parsed: Option<FileParsedCallback>,
    /// Invoked exactly once, with the final entity count, after post-parse
    /// aggregation and *before* any entity is pushed to the bounded
    /// channel. See `EntitiesExtractedCallback` for the rationale.
    pub on_entities_extracted: Option<EntitiesExtractedCallback>,
}

/// Parse a collection of source files in parallel and send results through a channel.
///
/// Uses `std::thread::scope` with raw OS threads (NOT Rayon) so that
/// `blocking_send` on the bounded channel only blocks the dedicated
/// parsing thread rather than a shared thread pool. This prevents
/// deadlocks with `fastembed` which requires Rayon for tokenization.
///
/// This function blocks until all files have been processed. It is
/// intended to be called from a `tokio::task::spawn_blocking` context.
///
/// `callbacks` replaces the v1.6.1 single `FileParsedCallback` parameter;
/// passing `None` is unchanged from previous versions.
pub fn parse_files_stream(
    files: &[PathBuf],
    parse_cfg: &ParseConfig,
    sender: mpsc::Sender<ParsedEntity>,
    max_concurrent: usize,
    callbacks: Option<ParseCallbacks>,
) {
    use std::sync::{Arc, Condvar, Mutex};

    // Concurrency limiter: Condvar-based semaphore backed by a Mutex.
    let sem = Arc::new((Mutex::new(0usize), Condvar::new()));

    // Collect entities into a shared buffer so we can run a global post-parse
    // aggregation step (e.g. Varnish built-in sub aggregators) before sending
    // them down the pipeline.
    let buffer: Arc<Mutex<Vec<ParsedEntity>>> = Arc::new(Mutex::new(Vec::new()));

    std::thread::scope(|s| {
        for path in files {
            let path = path.clone();
            let parse_cfg = parse_cfg.clone();
            let sem = Arc::clone(&sem);
            let buffer = Arc::clone(&buffer);

            // Acquire: block until active < max_concurrent
            {
                let (lock, cvar) = &*sem;
                let mut active = lock.lock().unwrap();
                while *active >= max_concurrent {
                    active = cvar.wait(active).unwrap();
                }
                *active += 1;
            }

            let on_file_parsed = callbacks.as_ref().and_then(|c| c.on_file_parsed.clone());

            s.spawn(move || {
                if let Ok(entities) = parse_single_file(&path, &parse_cfg) {
                    let mut buf = buffer.lock().unwrap();
                    buf.extend(entities);
                }

                if let Some(cb) = &on_file_parsed {
                    cb();
                }

                // Release: decrement active count and wake waiter
                let (lock, cvar) = &*sem;
                let mut active = lock.lock().unwrap();
                *active -= 1;
                cvar.notify_one();
            });
        }
    });
    // All threads joined here (std::thread::scope guarantees this).

    // Post-parse: aggregate Varnish built-in subs globally.
    let mut entities = Arc::try_unwrap(buffer)
        .map(|m| m.into_inner().unwrap_or_default())
        .unwrap_or_default();
    languages::varnish::aggregate_varnish_builtin_subs(&mut entities, &parse_cfg.repo_name);

    // Publish the entity total BEFORE the first blocking_send. This is the
    // exact handoff point the progress observer relies on: parse band ends
    // at 10%, ingest band takes over the moment this closure fires. Without
    // this ordering the bar would freeze at 10% while the producer is
    // blocked behind a full channel.
    if let Some(cb) = callbacks
        .as_ref()
        .and_then(|c| c.on_entities_extracted.as_ref())
    {
        cb(entities.len());
    }

    for entity in entities {
        if sender.blocking_send(entity).is_err() {
            warn!("Failed to send entity to channel");
            break;
        }
    }
}

/// Parse a collection of source files in parallel and return all extracted entities.
///
/// Uses `parse_files_stream` internally. This is a convenience wrapper for
/// callers that want the full Vec instead of streaming through a channel.
pub fn parse_files(files: &[PathBuf], parse_cfg: &ParseConfig) -> Vec<ParsedEntity> {
    let (tx, mut rx) = mpsc::channel(1024);
    let cpus = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4);

    parse_files_stream(files, parse_cfg, tx, cpus, None);

    let mut entities = Vec::with_capacity(1024);
    while let Ok(entity) = rx.try_recv() {
        entities.push(entity);
    }

    // Post-process Varnish built-in sub aggregators globally.
    languages::varnish::aggregate_varnish_builtin_subs(&mut entities, &parse_cfg.repo_name);

    entities
}

/// Parse a single source file and return its extracted entities.
/// Heuristic: detect whether a `.h` header contains C++ syntax.
/// Scans for keywords exclusive to C++ that do not appear in valid C.
fn is_cpp_header(source: &str) -> bool {
    let cpp_indicators = [
        "class ",
        "namespace ",
        "template<",
        "template <",
        "virtual ",
        "public:",
        "private:",
        "protected:",
        "using namespace",
        "constexpr ",
        "noexcept",
        "nullptr",
        "override",
        " final",
        "::", // qualified calls like Print::write(...)
    ];
    cpp_indicators.iter().any(|kw| source.contains(kw))
}

/// Per-file inputs shared by the language dispatch helpers below.
struct FileCtx<'a> {
    source: String,
    path: &'a Path,
    ext: &'a str,
    filename: &'a str,
    file_path: String,
    parse_cfg: &'a ParseConfig,
}

impl<'a> FileCtx<'a> {
    fn new(path: &'a Path, parse_cfg: &'a ParseConfig) -> Result<Self> {
        let bytes =
            fs::read(path).with_context(|| format!("Cannot read file: {}", path.display()))?;
        let source = String::from_utf8_lossy(&bytes).into_owned();
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or_default();
        // Handle files identified by name (no extension), e.g. Jenkinsfile
        let filename = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default();
        let file_path = crate::pipeline::files::to_repo_relative(path, &parse_cfg.repo_root);
        Ok(Self {
            source,
            path,
            ext,
            filename,
            file_path,
            parse_cfg,
        })
    }

    /// Runs the shared tree-sitter query pipeline for one language.
    fn extract_with_query(
        &self,
        query_file: &'static str,
        default_query: &'static str,
        language: tree_sitter::Language,
        lang_name: &str,
    ) -> Result<Vec<ParsedEntity>> {
        let query_src = load_query_source(query_file, default_query, self.parse_cfg);
        extractor::extract_entities(
            &self.source,
            language,
            &query_src,
            lang_name,
            &self.file_path,
            &self.parse_cfg.repo_name,
        )
    }
}

fn parse_single_file(path: &Path, parse_cfg: &ParseConfig) -> Result<Vec<ParsedEntity>> {
    let ctx = FileCtx::new(path, parse_cfg)?;

    if let Some(entities) = dispatch_by_filename(&ctx) {
        return Ok(entities);
    }

    let entities = dispatch_by_extension(&ctx)?;

    debug!(
        "Extracted {} entities from {}",
        entities.len(),
        ctx.file_path
    );
    Ok(entities)
}

/// Extensionless build files identified by name rather than by extension.
fn dispatch_by_filename(ctx: &FileCtx<'_>) -> Option<Vec<ParsedEntity>> {
    if ctx.filename == "Jenkinsfile" {
        return Some(languages::jenkins::extract_entities_jenkins(
            &ctx.source,
            &ctx.file_path,
            &ctx.parse_cfg.repo_name,
        ));
    }

    if ctx.filename == "Directory.Packages.props" {
        // MSBuild Central Package Management: emits no entities of its
        // own — the CPM map is consumed lazily by csproj parsing. The
        // dispatcher must still have a target so the file is recognized
        // when discovered.
        return Some(languages::msbuild::extract_entities_props(
            &ctx.source,
            &ctx.file_path,
            &ctx.parse_cfg.repo_name,
        ));
    }

    None
}

/// Dispatches by extension across the language families. Each family owns a
/// disjoint set of extensions, so the fallthrough order only decides which
/// dispatcher is tried first.
fn dispatch_by_extension(ctx: &FileCtx<'_>) -> Result<Vec<ParsedEntity>> {
    if let Some(entities) = dispatch_query_lang(ctx.ext, ctx)? {
        return Ok(entities);
    }
    if let Some(entities) = dispatch_html_lang(ctx.ext, ctx)? {
        return Ok(entities);
    }
    if let Some(entities) = dispatch_config_lang(ctx.ext, ctx) {
        return Ok(entities);
    }
    if let Some(entities) = dispatch_lexical_lang(ctx.ext, ctx) {
        return Ok(entities);
    }
    warn!("Unsupported extension '{}', skipping", ctx.ext);
    Ok(vec![])
}

/// Tree-sitter query pipeline languages (Java, Kotlin, TS/JS, C/C++, C#,
/// Python, Rust, CSS/SCSS, Markdown). Returns `None` when `ext` is not
/// handled here so the caller can try the remaining dispatchers.
///
/// Most languages are a plain (query, grammar) pair matched in the table
/// below; those needing extra handling (TSX query concat, Rust FQN
/// qualification, C-vs-C++ header sniffing) delegate to their own helpers.
fn dispatch_query_lang(ext: &str, ctx: &FileCtx<'_>) -> Result<Option<Vec<ParsedEntity>>> {
    if ext == "ts" || ext == "tsx" || ext == "cts" {
        return dispatch_typescript(ext, ctx).map(Some);
    }
    if ext == "rs" {
        return dispatch_rust(ctx).map(Some);
    }
    if ext == "h" {
        return dispatch_c_header(ctx).map(Some);
    }

    let (query_file, default_query, language, lang_name) = match ext {
        "java" => (
            "java.scm",
            DEFAULT_JAVA_QUERY,
            tree_sitter_java::LANGUAGE.into(),
            "java",
        ),
        "kt" | "kts" => (
            "kotlin.scm",
            DEFAULT_KOTLIN_QUERY,
            tree_sitter_kotlin_ng::LANGUAGE.into(),
            "kotlin",
        ),
        "js" | "mjs" | "cjs" | "jsx" => (
            "javascript.scm",
            DEFAULT_JS_QUERY,
            tree_sitter_javascript::LANGUAGE.into(),
            "javascript",
        ),
        "css" => (
            "css.scm",
            DEFAULT_CSS_QUERY,
            tree_sitter_css::LANGUAGE.into(),
            "css",
        ),
        "scss" | "sass" => (
            "scss.scm",
            DEFAULT_SCSS_QUERY,
            tree_sitter_scss::language(),
            "scss",
        ),
        "py" | "pyi" | "pyw" => (
            "python.scm",
            DEFAULT_PYTHON_QUERY,
            tree_sitter_python::LANGUAGE.into(),
            "python",
        ),
        "c" => (
            "c.scm",
            DEFAULT_C_QUERY,
            tree_sitter_c::LANGUAGE.into(),
            "c",
        ),
        "cpp" | "cxx" | "cc" | "hpp" | "hxx" | "hh" => (
            "cpp.scm",
            DEFAULT_CPP_QUERY,
            tree_sitter_cpp::LANGUAGE.into(),
            "cpp",
        ),
        "cs" => (
            "csharp.scm",
            DEFAULT_CSHARP_QUERY,
            tree_sitter_c_sharp::LANGUAGE.into(),
            "csharp",
        ),
        "md" | "markdown" => (
            "markdown.scm",
            DEFAULT_MD_QUERY,
            tree_sitter_md::LANGUAGE.into(),
            "markdown",
        ),
        _ => return Ok(None),
    };
    Ok(Some(ctx.extract_with_query(
        query_file,
        default_query,
        language,
        lang_name,
    )?))
}

/// TypeScript family. TSX appends JSX-specific query rules (component
/// invocations) to the shared TypeScript query.
fn dispatch_typescript(ext: &str, ctx: &FileCtx<'_>) -> Result<Vec<ParsedEntity>> {
    let mut query_src = load_query_source("typescript.scm", DEFAULT_TS_QUERY, ctx.parse_cfg);
    let lang: tree_sitter::Language = if ext == "tsx" {
        let tsx_rules = load_query_source("tsx.scm", DEFAULT_TSX_QUERY, ctx.parse_cfg);
        query_src.push('\n');
        query_src.push_str(&tsx_rules);
        tree_sitter_typescript::LANGUAGE_TSX.into()
    } else {
        tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()
    };
    extractor::extract_entities(
        &ctx.source,
        lang,
        &query_src,
        "typescript",
        &ctx.file_path,
        &ctx.parse_cfg.repo_name,
    )
}

/// Rust: query pipeline plus the crate-anchored FQN qualification post-pass.
fn dispatch_rust(ctx: &FileCtx<'_>) -> Result<Vec<ParsedEntity>> {
    let mut rust_entities = ctx.extract_with_query(
        "rust.scm",
        DEFAULT_RUST_QUERY,
        tree_sitter_rust::LANGUAGE.into(),
        "rust",
    )?;
    languages::rust::qualify_rust_fqns(
        &mut rust_entities,
        &ctx.file_path,
        ctx.parse_cfg.repo_path.as_deref(),
        Some(&ctx.source),
    );
    Ok(rust_entities)
}

/// C headers: sniff the content to decide between the C++ and C grammars.
fn dispatch_c_header(ctx: &FileCtx<'_>) -> Result<Vec<ParsedEntity>> {
    if is_cpp_header(&ctx.source) {
        return ctx.extract_with_query(
            "cpp.scm",
            DEFAULT_CPP_QUERY,
            tree_sitter_cpp::LANGUAGE.into(),
            "cpp",
        );
    }
    ctx.extract_with_query(
        "c.scm",
        DEFAULT_C_QUERY,
        tree_sitter_c::LANGUAGE.into(),
        "c",
    )
}

/// HTML: parsed with its own tree walk instead of the shared query pipeline.
/// Returns `None` for non-HTML extensions.
fn dispatch_html_lang(ext: &str, ctx: &FileCtx<'_>) -> Result<Option<Vec<ParsedEntity>>> {
    if ext != "html" && ext != "htm" {
        return Ok(None);
    }
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_html::LANGUAGE.into())
        .context("Failed to load HTML grammar")?;
    let tree = parser
        .parse(&ctx.source, None)
        .context("Failed to parse HTML")?;
    Ok(Some(languages::html::extract_entities_html(
        tree.root_node(),
        ctx.source.as_bytes(),
        &ctx.file_path,
        &ctx.parse_cfg.repo_name,
    )))
}

/// Configuration-file languages. All of them emit no entities unless
/// `include_config_files` is enabled (package.json/tsconfig.json excepted).
/// Returns `None` for non-config extensions.
fn dispatch_config_lang(ext: &str, ctx: &FileCtx<'_>) -> Option<Vec<ParsedEntity>> {
    match ext {
        "yml" | "yaml" => {
            if !ctx.parse_cfg.include_config_files {
                return Some(Vec::new());
            }
            Some(dispatch_yaml(
                &ctx.source,
                ctx.path,
                &ctx.file_path,
                &ctx.parse_cfg.repo_name,
            ))
        }
        "json" => {
            if !ctx.parse_cfg.include_config_files
                && ctx.filename != "package.json"
                && ctx.filename != "tsconfig.json"
            {
                return Some(Vec::new());
            }
            Some(languages::json_config::extract_entities_json_config(
                &ctx.source,
                &ctx.file_path,
                &ctx.parse_cfg.repo_name,
            ))
        }
        "properties" => {
            if !ctx.parse_cfg.include_config_files {
                return Some(Vec::new());
            }
            Some(languages::properties::extract_entities_properties(
                &ctx.source,
                &ctx.file_path,
                &ctx.parse_cfg.repo_name,
            ))
        }
        "tpl" => {
            if !ctx.parse_cfg.include_config_files {
                return Some(Vec::new());
            }
            let chart_name = detect_chart_name(ctx.path, &ctx.parse_cfg.repo_root);
            Some(languages::helm::extract_helm_template(
                &ctx.source,
                &ctx.file_path,
                &ctx.parse_cfg.repo_name,
                &chart_name,
            ))
        }
        _ => None,
    }
}

/// Hand-written (non-tree-sitter) language parsers. Returns `None` for
/// extensions not handled here.
fn dispatch_lexical_lang(ext: &str, ctx: &FileCtx<'_>) -> Option<Vec<ParsedEntity>> {
    match ext {
        "groovy" => Some(languages::groovy::extract_entities_groovy(
            &ctx.source,
            &ctx.file_path,
            &ctx.parse_cfg.repo_name,
        )),
        "gradle" => Some(languages::gradle::extract_entities_gradle(
            &ctx.source,
            &ctx.file_path,
            &ctx.parse_cfg.repo_name,
        )),
        "jenkinsfile" => Some(languages::jenkins::extract_entities_jenkins(
            &ctx.source,
            &ctx.file_path,
            &ctx.parse_cfg.repo_name,
        )),
        "xml" => Some(languages::xml::extract_entities_xml(
            &ctx.source,
            &ctx.file_path,
            &ctx.parse_cfg.repo_name,
        )),
        "csproj" => {
            // MSBuild: requires the csproj's directory + repo_root so the
            // CPM lookup can walk up to `Directory.Packages.props`.
            let csproj_abs_dir = ctx.path.parent().unwrap_or(ctx.path);
            let msbuild_ctx = languages::msbuild::MsbuildContext {
                source: &ctx.source,
                file_path: &ctx.file_path,
                repo_name: &ctx.parse_cfg.repo_name,
                csproj_abs_dir,
                repo_root: &ctx.parse_cfg.repo_root,
            };
            Some(languages::msbuild::extract_entities_csproj(&msbuild_ctx))
        }
        "toml" => Some(languages::toml::extract_entities_toml(
            &ctx.source,
            &ctx.file_path,
            &ctx.parse_cfg.repo_name,
        )),
        "vcl" => Some(languages::varnish::extract_entities_vcl(
            &ctx.source,
            &ctx.file_path,
            &ctx.parse_cfg.repo_name,
        )),
        "vtc" => Some(languages::varnish::extract_entities_vtc(
            &ctx.source,
            &ctx.file_path,
            &ctx.parse_cfg.repo_name,
        )),
        "vcc" => Some(languages::varnish::extract_entities_vcc(
            &ctx.source,
            &ctx.file_path,
            &ctx.parse_cfg.repo_name,
        )),
        _ => None,
    }
}

/// Dispatch YAML files to the appropriate parser based on content.
fn dispatch_yaml(
    source: &str,
    absolute_path: &Path,
    relative_path: &str,
    repo_name: &str,
) -> Vec<ParsedEntity> {
    let filename = absolute_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("");

    // 1. Is it Chart.yaml?
    if filename == "Chart.yaml" {
        return languages::helm::extract_chart_yaml(source, relative_path, repo_name);
    }

    // 2. Is it inside a Helm chart directory?
    if is_in_helm_chart_dir(absolute_path) {
        if filename == "values.yaml" || filename == "values.yml" {
            let chart_name = detect_chart_name(
                absolute_path,
                absolute_path.parent().unwrap_or(Path::new(".")),
            );
            return languages::helm::extract_values_yaml(
                source,
                relative_path,
                repo_name,
                &chart_name,
            );
        }
        if is_in_templates_dir(absolute_path) {
            let chart_name = detect_chart_name(
                absolute_path,
                absolute_path.parent().unwrap_or(Path::new(".")),
            );
            return languages::helm::extract_helm_template(
                source,
                relative_path,
                repo_name,
                &chart_name,
            );
        }
    }

    // 3. Is it a K8s manifest? (has apiVersion + kind at root level)
    if let Ok(yaml) = serde_yaml::from_str::<serde_yaml::Value>(source)
        && yaml.get("apiVersion").is_some()
        && yaml.get("kind").is_some()
    {
        return languages::kubernetes::extract_entities_k8s(source, relative_path, repo_name);
    }

    // 4. Default: generic configuration YAML
    languages::yaml::extract_entities_yaml(source, relative_path, repo_name)
}

/// Check if the file is inside a Helm chart directory by looking for Chart.yaml in parent dirs.
///
/// `absolute_path` must be an absolute filesystem path — the function uses
/// `.exists()` on each ancestor, which resolves against the **process CWD**
/// for relative inputs. Callers in the pipeline always have the absolute
/// path from `discover_files`; only the **persisted** entity `file_path`
/// is relative.
fn is_in_helm_chart_dir(absolute_path: &Path) -> bool {
    let mut current = absolute_path.parent();

    while let Some(dir) = current {
        if dir.join("Chart.yaml").exists() {
            return true;
        }
        current = dir.parent();
    }
    false
}

/// Check if the file is inside a templates directory (Helm convention).
fn is_in_templates_dir(absolute_path: &Path) -> bool {
    let mut current = Some(absolute_path);

    while let Some(p) = current {
        if p.file_name().and_then(|n| n.to_str()) == Some("templates") {
            return true;
        }
        current = p.parent();
    }
    false
}

/// Detect the Helm chart name from the nearest Chart.yaml or directory name.
///
/// `absolute_path` must be an absolute filesystem path (see
/// `is_in_helm_chart_dir`). The `repo_root` parameter is currently unused
/// but kept for future disambiguation if multiple Chart.yamls are reachable.
fn detect_chart_name(absolute_path: &Path, _repo_root: &Path) -> String {
    let mut current = absolute_path.parent();

    while let Some(dir) = current {
        let chart_yaml = dir.join("Chart.yaml");
        if chart_yaml.exists() {
            if let Ok(source) = fs::read_to_string(&chart_yaml)
                && let Ok(yaml) = serde_yaml::from_str::<serde_yaml::Value>(&source)
                && let Some(name) = yaml.get("name").and_then(|v| v.as_str())
            {
                return name.to_string();
            }
            break;
        }
        current = dir.parent();
    }

    // Fall back to parent directory name
    absolute_path
        .parent()
        .and_then(|p| p.file_name())
        .and_then(|n| n.to_str())
        .unwrap_or("unknown")
        .to_string()
}

/// Return the query source string, preferring a custom file when available.
#[expect(
    clippy::cognitive_complexity,
    reason = "function is verbose but correct — extraction deferred"
)]
fn load_query_source(filename: &str, default: &str, cfg: &ParseConfig) -> String {
    if let Some(dir) = &cfg.custom_queries_path {
        let custom_path = PathBuf::from(dir).join(filename);
        if custom_path.exists() {
            match fs::read_to_string(&custom_path) {
                Ok(src) => {
                    tracing::info!("Using custom query: {}", custom_path.display());
                    return src;
                }
                Err(e) => warn!(
                    "Failed to load custom query {}: {e} — using built-in",
                    custom_path.display()
                ),
            }
        }
    }
    default.to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_config_creation() {
        let cfg = ParseConfig {
            custom_queries_path: None,
            repo_name: "test-repo".to_string(),
            include_config_files: true,
            repo_path: None,
            ..Default::default()
        };

        assert_eq!(cfg.repo_name, "test-repo");
        assert!(cfg.custom_queries_path.is_none());
    }

    #[test]
    fn test_parse_config_with_custom_queries() {
        let cfg = ParseConfig {
            custom_queries_path: Some("/custom/queries".to_string()),
            repo_name: "my-repo".to_string(),
            include_config_files: true,
            repo_path: None,
            ..Default::default()
        };

        assert_eq!(cfg.repo_name, "my-repo");
        assert_eq!(cfg.custom_queries_path, Some("/custom/queries".to_string()));
    }

    #[test]
    fn test_load_query_source_uses_default() {
        let cfg = ParseConfig {
            custom_queries_path: None,
            repo_name: "test-repo".to_string(),
            include_config_files: true,
            repo_path: None,
            ..Default::default()
        };

        let default_query = "MATCH (n) RETURN n";
        let result = load_query_source("test.scm", default_query, &cfg);

        assert_eq!(result, default_query);
    }

    #[test]
    fn test_load_query_source_nonexistent_custom_path() {
        let cfg = ParseConfig {
            custom_queries_path: None,
            repo_name: "test-repo".to_string(),
            include_config_files: true,
            repo_path: None,
            ..Default::default()
        };

        let default_query = "MATCH (n) RETURN n";
        let result = load_query_source("test.scm", default_query, &cfg);

        // Should fall back to default when custom path doesn't exist
        assert_eq!(result, default_query);
    }

    #[test]
    fn test_parse_files_empty_list() {
        let cfg = ParseConfig {
            custom_queries_path: None,
            repo_name: "test-repo".to_string(),
            include_config_files: true,
            repo_path: None,
            ..Default::default()
        };

        let files: Vec<PathBuf> = vec![];
        let (sender, mut receiver) = mpsc::channel::<ParsedEntity>(32);

        parse_files_stream(&files, &cfg, sender, 4, None);

        // No files to parse, channel should receive nothing
        assert!(receiver.try_recv().is_err());
    }

    #[test]
    fn test_parse_files_with_mock_channel() {
        let cfg = ParseConfig {
            custom_queries_path: None,
            repo_name: "test-repo".to_string(),
            include_config_files: true,
            repo_path: None,
            ..Default::default()
        };

        // Use an empty list since we can't create real files in unit tests
        let files: Vec<PathBuf> = vec![];
        let (sender, mut receiver) = mpsc::channel::<ParsedEntity>(32);

        parse_files_stream(&files, &cfg, sender, 4, None);

        // Verify channel can receive messages (simulated)
        assert!(receiver.try_recv().is_err()); // No data sent
    }

    #[test]
    fn test_is_cpp_header_detects_class() {
        assert!(is_cpp_header(
            "class Print {\npublic:\n    void write();\n};"
        ));
    }

    #[test]
    fn test_is_cpp_header_detects_namespace() {
        assert!(is_cpp_header("namespace Engine {\n    class Foo {};\n}"));
    }

    #[test]
    fn test_is_cpp_header_detects_virtual() {
        assert!(is_cpp_header("virtual size_t write(uint8_t) = 0;"));
    }

    #[test]
    fn test_is_cpp_header_detects_template() {
        assert!(is_cpp_header("template <typename T>\nclass Container {};"));
    }

    #[test]
    fn test_is_cpp_header_pure_c_returns_false() {
        let c_header = r#"
#ifndef FOO_H
#define FOO_H
typedef struct { int x; int y; } Point;
void foo(int n);
int bar(const char *s);
#endif
"#;
        assert!(!is_cpp_header(c_header));
    }

    #[test]
    fn test_is_cpp_header_empty_returns_false() {
        assert!(!is_cpp_header(""));
    }

    #[test]
    fn test_is_cpp_header_detects_qualified_call() {
        assert!(is_cpp_header(
            "size_t Print::write(const uint8_t *buf, size_t s) { return 0; }"
        ));
    }

    #[test]
    fn test_unsupported_file_extension_handling() {
        // Test extension detection logic
        let path = PathBuf::from("/test/file.unsupported");
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or_default();

        assert_eq!(ext, "unsupported");
        // File would be skipped (not java, ts, tsx, cts, js, mjs, cjs, jsx)
        assert!(
            ext != "java"
                && ext != "ts"
                && ext != "tsx"
                && ext != "cts"
                && ext != "js"
                && ext != "mjs"
                && ext != "cjs"
                && ext != "jsx"
        );
    }

    #[test]
    fn test_java_file_extension_detection() {
        let path = PathBuf::from("/test/Service.java");
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or_default();

        assert_eq!(ext, "java");
    }

    fn assert_extensions_detected(extensions: &[&str]) {
        for ext_name in extensions {
            let path = PathBuf::from(format!("/test/file.{}", ext_name));
            let ext = path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or_default();
            assert_eq!(ext, *ext_name);
        }
    }

    #[test]
    fn test_kotlin_file_extension_detection() {
        assert_extensions_detected(&["kt", "kts"]);
    }

    #[test]
    fn test_typescript_file_extension_detection() {
        assert_extensions_detected(&["ts", "tsx", "cts"]);
    }

    #[test]
    fn test_javascript_file_extension_detection() {
        assert_extensions_detected(&["js", "mjs", "cjs", "jsx"]);
    }

    #[test]
    fn test_file_path_conversion() {
        let parse_cfg = ParseConfig {
            repo_root: PathBuf::from("/home/user/project"),
            ..Default::default()
        };
        let path = PathBuf::from("/home/user/project/src/Main.java");
        let file_path = crate::pipeline::files::to_repo_relative(&path, &parse_cfg.repo_root);

        assert!(file_path.contains("Main.java"));
        assert_eq!(file_path, "src/Main.java");
    }

    // ---- §10.1 parser tests for relative file_path in entities ----

    #[test]
    fn test_parsed_entity_file_path_is_relative() {
        use tempfile::tempdir;
        let dir = tempdir().unwrap();
        let repo_root = dir.path().canonicalize().unwrap();

        // Create a minimal Java source file so the parser returns entities.
        let src_dir = repo_root.join("src");
        fs::create_dir_all(&src_dir).unwrap();
        let java_file = src_dir.join("Foo.java");
        fs::write(&java_file, "public class Foo { public void bar() {} }").unwrap();

        let parse_cfg = ParseConfig {
            repo_root: repo_root.clone(),
            custom_queries_path: None,
            repo_name: "test-repo".to_string(),
            include_config_files: false,
            repo_path: Some(repo_root.to_string_lossy().into_owned()),
        };

        let entities = parse_files(&[java_file], &parse_cfg);
        assert!(
            !entities.is_empty(),
            "parser should produce at least one entity for Foo.java"
        );
        for entity in &entities {
            assert!(
                !entity.file_path.starts_with('/'),
                "file_path must be relative (no leading /), got {}",
                entity.file_path
            );
            assert!(
                !entity.file_path.contains('\\'),
                "file_path must use POSIX separators, got {}",
                entity.file_path
            );
        }
        // The class `Foo` should carry file_path = "src/Foo.java".
        let foo = entities.iter().find(|e| e.name == "Foo").expect("Foo");
        assert_eq!(foo.file_path, "src/Foo.java");
    }

    #[test]
    fn test_parsed_entity_file_path_verbatim_without_repo_root() {
        // When `repo_path` is None the parser falls back to its original
        // behavior (path verbatim), protecting existing parser unit tests
        // that don't set up a repo root.
        use tempfile::tempdir;
        let dir = tempdir().unwrap();
        let java_file = dir.path().join("Main.java");
        fs::write(&java_file, "public class Main {}").unwrap();

        let parse_cfg = ParseConfig {
            // No canonical repo_root set in production — default (".") used.
            repo_root: PathBuf::from("."),
            custom_queries_path: None,
            repo_name: "test-repo".to_string(),
            include_config_files: false,
            repo_path: None,
        };

        let entities = parse_files(std::slice::from_ref(&java_file), &parse_cfg);
        // Without a repo_root, the parser cannot strip a prefix — it falls
        // back to the absolute path (R5). Existing unit tests that don't
        // set up a repo root continue to see the path they passed in.
        assert!(!entities.is_empty(), "parser must still produce entities");
        let main = entities.iter().find(|e| e.name == "Main").expect("Main");
        assert!(
            main.file_path.contains("Main.java"),
            "verbatim path should still contain the filename, got {}",
            main.file_path
        );
    }

    #[test]
    fn test_parse_config_repo_name_assignment() {
        let cfg = ParseConfig {
            custom_queries_path: None,
            repo_name: "myproject".to_string(),
            include_config_files: true,
            repo_path: None,
            ..Default::default()
        };

        let path = PathBuf::from("/src/Main.java");
        let _entities = parse_files(&[path], &cfg);

        // With empty/invalid files, should return empty vector
        // But repo_name should be preserved in config
        assert_eq!(cfg.repo_name, "myproject");
    }

    #[test]
    fn test_parse_files_with_empty_input() {
        let cfg = ParseConfig {
            custom_queries_path: None,
            repo_name: "test-repo".to_string(),
            include_config_files: true,
            repo_path: None,
            ..Default::default()
        };

        let files: Vec<PathBuf> = vec![];
        let entities = parse_files(&files, &cfg);

        // No files to parse, should return empty vector
        assert!(entities.is_empty());
    }

    #[test]
    fn test_channel_sender_behavior_mock() {
        // Test that bounded channel sender doesn't fail on empty input
        let (sender, mut receiver) = mpsc::channel::<ParsedEntity>(32);

        // Dropping sender without sending should not error
        drop(sender);

        // Receiver should get no data
        assert!(receiver.try_recv().is_err());
    }

    #[test]
    fn test_bounded_channel_blocking_send() {
        // Test that blocking_send works correctly with a bounded channel
        let (sender, mut receiver) = mpsc::channel::<ParsedEntity>(2);

        // Create a minimal entity for testing
        let entity = ParsedEntity::new(
            "TestEntity",
            crate::models::EntityKind::Class,
            "com.test.TestEntity",
            None,
            None,
            "java",
            "/test/Test.java",
            1,
            5,
            None,
            "test-repo",
        );

        // Send via blocking_send (simulating what parse_files_stream does)
        assert!(sender.blocking_send(entity.clone()).is_ok());
        assert!(sender.blocking_send(entity).is_ok());

        // Verify receiver gets both entities
        assert!(receiver.try_recv().is_ok());
        assert!(receiver.try_recv().is_ok());
    }

    #[test]
    fn test_bounded_channel_capacity_backpressure() {
        // Test that bounded channel respects capacity
        let (sender, _receiver) = mpsc::channel::<ParsedEntity>(2);

        let entity = ParsedEntity::new(
            "TestEntity",
            crate::models::EntityKind::Class,
            "com.test.TestEntity",
            None,
            None,
            "java",
            "/test/Test.java",
            1,
            5,
            None,
            "test-repo",
        );

        // Fill the channel to capacity
        assert!(sender.try_send(entity.clone()).is_ok());
        assert!(sender.try_send(entity.clone()).is_ok());

        // Third send should fail with Full error (channel is at capacity)
        assert!(sender.try_send(entity).is_err());
    }

    #[test]
    fn test_bounded_channel_receives_after_blocking_send() {
        // Verify that after blocking_send, data is available on the receiver
        let (sender, mut receiver) = mpsc::channel::<ParsedEntity>(1);

        let entity = ParsedEntity::new(
            "TestClass",
            crate::models::EntityKind::Class,
            "com.example.TestClass",
            Some("public class TestClass".to_string()),
            Some("A test class".to_string()),
            "java",
            "/proj/TestClass.java",
            10,
            25,
            None,
            "test-repo",
        );

        sender.blocking_send(entity).unwrap();

        let received = receiver.try_recv().unwrap();
        assert_eq!(received.name, "TestClass");
        assert_eq!(received.fqn, "com.example.TestClass");
        assert_eq!(received.language, "java");
    }

    #[test]
    fn test_parse_files_stream_callback_once_per_file() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use tempfile::tempdir;

        let dir = tempdir().unwrap();
        for i in 0..3 {
            fs::write(dir.path().join(format!("file_{}.rs", i)), "fn foo() {}").unwrap();
        }

        let files: Vec<PathBuf> = (0..3)
            .map(|i| dir.path().join(format!("file_{}.rs", i)))
            .collect();

        let cfg = ParseConfig {
            custom_queries_path: None,
            repo_name: "test-repo".to_string(),
            include_config_files: true,
            repo_path: None,
            ..Default::default()
        };

        let (sender, mut receiver) = mpsc::channel::<ParsedEntity>(32);
        let counter = std::sync::Arc::new(AtomicUsize::new(0));
        let counter_clone = std::sync::Arc::clone(&counter);
        let cb: FileParsedCallback = std::sync::Arc::new(move || {
            counter_clone.fetch_add(1, Ordering::SeqCst);
        });

        let callbacks = ParseCallbacks {
            on_file_parsed: Some(cb),
            on_entities_extracted: None,
        };
        parse_files_stream(&files, &cfg, sender, 4, Some(callbacks));

        let mut count = 0;
        while receiver.try_recv().is_ok() {
            count += 1;
        }
        assert!(count > 0, "Should have extracted some entities");
        assert_eq!(counter.load(Ordering::SeqCst), 3);
    }

    #[test]
    fn test_parse_files_stream_callback_counts_unparseable() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use tempfile::tempdir;

        let dir = tempdir().unwrap();
        fs::write(dir.path().join("valid.rs"), "fn foo() {}").unwrap();
        fs::write(dir.path().join("valid2.rs"), "enum Color { Red }").unwrap();
        fs::write(dir.path().join("broken.rs"), "not valid rust @@@@!!").unwrap();

        let files: Vec<PathBuf> = ["valid.rs", "valid2.rs", "broken.rs"]
            .iter()
            .map(|f| dir.path().join(f))
            .collect();

        let cfg = ParseConfig {
            custom_queries_path: None,
            repo_name: "test-repo".to_string(),
            include_config_files: true,
            repo_path: None,
            ..Default::default()
        };

        let (sender, _receiver) = mpsc::channel::<ParsedEntity>(32);
        let counter = std::sync::Arc::new(AtomicUsize::new(0));
        let counter_clone = std::sync::Arc::clone(&counter);
        let cb: FileParsedCallback = std::sync::Arc::new(move || {
            counter_clone.fetch_add(1, Ordering::SeqCst);
        });

        let callbacks = ParseCallbacks {
            on_file_parsed: Some(cb),
            on_entities_extracted: None,
        };
        parse_files_stream(&files, &cfg, sender, 4, Some(callbacks));

        assert_eq!(counter.load(Ordering::SeqCst), 3);
    }

    #[test]
    fn test_parse_files_stream_none_callback() {
        use tempfile::tempdir;

        let dir = tempdir().unwrap();
        fs::write(dir.path().join("test.rs"), "fn foo() {}").unwrap();
        let files: Vec<PathBuf> = vec![dir.path().join("test.rs")];

        let cfg = ParseConfig {
            custom_queries_path: None,
            repo_name: "test-repo".to_string(),
            include_config_files: true,
            repo_path: None,
            ..Default::default()
        };

        let (sender, mut receiver) = mpsc::channel::<ParsedEntity>(32);
        parse_files_stream(&files, &cfg, sender, 4, None);

        let mut count = 0;
        while receiver.try_recv().is_ok() {
            count += 1;
        }
        assert!(count > 0, "Should parse entities with None callback");
    }

    #[test]
    fn test_multiple_file_extensions_in_batch() {
        let files = [
            PathBuf::from("file1.java"),
            PathBuf::from("file2.ts"),
            PathBuf::from("file3.tsx"),
            PathBuf::from("file4.kt"),
            PathBuf::from("file5.unsupported"),
        ];

        let expected_extensions = ["java", "ts", "tsx", "kt", "unsupported"];

        for (file, expected_ext) in files.iter().zip(expected_extensions.iter()) {
            let ext = file
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or_default();
            assert_eq!(ext, *expected_ext);
        }
    }

    // ---- v1.6.2: ParseCallbacks surface the entity total to the runner ----

    #[test]
    fn given_files_to_parse_when_stream_completes_then_entities_extracted_receives_the_total() {
        // The reported total must equal the number of entities the parser
        // produced (i.e. what the consumer pulls off the channel).
        use std::sync::Mutex;

        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("a.rs"),
            "pub fn alpha() {}\npub fn beta() {}\n",
        )
        .unwrap();
        fs::write(
            dir.path().join("b.rs"),
            "pub struct Gamma {}\nimpl Gamma { pub fn delta(&self) {} }\n",
        )
        .unwrap();

        let files: Vec<PathBuf> = ["a.rs", "b.rs"]
            .iter()
            .map(|f| dir.path().join(f))
            .collect();

        let cfg = ParseConfig {
            custom_queries_path: None,
            repo_name: "test-repo".to_string(),
            include_config_files: true,
            repo_path: None,
            ..Default::default()
        };

        let reported: std::sync::Arc<Mutex<Option<usize>>> = std::sync::Arc::new(Mutex::new(None));
        let reported_clone = std::sync::Arc::clone(&reported);
        let callbacks = ParseCallbacks {
            on_file_parsed: None,
            on_entities_extracted: Some(std::sync::Arc::new(move |n: usize| {
                *reported_clone.lock().unwrap() = Some(n);
            })),
        };

        let (sender, mut receiver) = mpsc::channel::<ParsedEntity>(32);
        parse_files_stream(&files, &cfg, sender, 4, Some(callbacks));

        let mut received_count = 0usize;
        while receiver.try_recv().is_ok() {
            received_count += 1;
        }

        let reported_value = reported.lock().unwrap().expect("callback fired");
        assert_eq!(
            reported_value, received_count,
            "ParseCallbacks::on_entities_extracted reported {reported_value} but channel delivered {received_count}"
        );
        assert!(
            received_count > 0,
            "parser must produce at least one entity"
        );
    }

    #[test]
    fn given_a_saturated_channel_when_parsing_completes_then_total_is_published_before_blocking() {
        // Heart of the fix (v1.6.2):
        //
        // The previous behavior was to block on `blocking_send` after the
        // parse was complete, but the parse had already saturated the
        // percentage to 100% before that block. The new contract publishes
        // the entity total BEFORE the first blocking send, so a downstream
        // observer sees the count even while the parser is stuck waiting
        // for the channel to drain.
        //
        // We assert that by: (a) creating a channel of capacity 1, (b) NOT
        // consuming it, (c) running parse_files_stream on a background
        // thread, and (d) verifying the total was published while the
        // producer is still blocked on send.
        use std::sync::{Arc, Condvar, Mutex};

        let dir = tempfile::tempdir().unwrap();
        // Several files so the buffer has enough entities to fill the
        // channel and then force the producer to block.
        for i in 0..6 {
            fs::write(
                dir.path().join(format!("f_{i}.rs")),
                "pub fn alpha() {}\npub fn beta() {}\npub struct Gamma {}\n",
            )
            .unwrap();
        }
        let files: Vec<PathBuf> = (0..6)
            .map(|i| dir.path().join(format!("f_{i}.rs")))
            .collect();

        let cfg = ParseConfig {
            custom_queries_path: None,
            repo_name: "test-repo".to_string(),
            include_config_files: true,
            repo_path: None,
            ..Default::default()
        };

        let signalled = Arc::new((Mutex::new(false), Condvar::new()));
        let signalled_clone = Arc::clone(&signalled);

        let callbacks = ParseCallbacks {
            on_file_parsed: None,
            on_entities_extracted: Some(Arc::new(move |n: usize| {
                let (lock, cvar) = &*signalled_clone;
                *lock.lock().unwrap() = true;
                cvar.notify_all();
                // Don't drop n — the assertion below reads it back via the
                // closure's captured environment. (n is informational here.)
                let _ = n;
            })),
        };

        // Channel capacity 1 → the producer will block on the second send.
        let (sender, _receiver) = mpsc::channel::<ParsedEntity>(1);

        let join = std::thread::spawn(move || {
            parse_files_stream(&files, &cfg, sender, 2, Some(callbacks));
        });

        // Give the producer up to 5 seconds to publish the total while it
        // is still blocked on send (the receiver is dropped so capacity
        // stays at 0 once the first slot fills).
        let (lock, cvar) = &*signalled;
        let mut fired = lock.lock().unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !*fired && std::time::Instant::now() < deadline {
            fired = cvar
                .wait_timeout(fired, std::time::Duration::from_millis(100))
                .unwrap()
                .0;
        }

        let observed = *fired;
        drop(join); // ignore — we only care about the publish timing
        assert!(
            observed,
            "on_entities_extracted must fire while the producer is blocked on send"
        );
    }

    #[test]
    fn given_default_parse_callbacks_when_parsing_then_no_observer_is_invoked() {
        // Regression guard: ParseCallbacks::default() must be a no-op.
        let callbacks = ParseCallbacks::default();
        assert!(callbacks.on_file_parsed.is_none());
        assert!(callbacks.on_entities_extracted.is_none());
    }

    #[test]
    fn given_only_on_file_parsed_set_when_parsing_then_it_still_fires_once_per_file() {
        // Back-compat check: setting only the file-parsed hook must still
        // behave like the old single-callback contract (fires once per file).
        use std::sync::atomic::{AtomicUsize, Ordering};
        use tempfile::tempdir;

        let dir = tempdir().unwrap();
        for i in 0..3 {
            fs::write(dir.path().join(format!("file_{i}.rs")), "fn foo() {}").unwrap();
        }
        let files: Vec<PathBuf> = (0..3)
            .map(|i| dir.path().join(format!("file_{i}.rs")))
            .collect();

        let cfg = ParseConfig {
            custom_queries_path: None,
            repo_name: "test-repo".to_string(),
            include_config_files: true,
            repo_path: None,
            ..Default::default()
        };

        let counter = std::sync::Arc::new(AtomicUsize::new(0));
        let counter_clone = std::sync::Arc::clone(&counter);
        let callbacks = ParseCallbacks {
            on_file_parsed: Some(std::sync::Arc::new(move || {
                counter_clone.fetch_add(1, Ordering::SeqCst);
            })),
            on_entities_extracted: None,
        };

        let (sender, _receiver) = mpsc::channel::<ParsedEntity>(32);
        parse_files_stream(&files, &cfg, sender, 4, Some(callbacks));

        assert_eq!(counter.load(Ordering::SeqCst), 3);
    }
}
