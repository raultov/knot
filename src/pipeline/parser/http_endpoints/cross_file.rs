//! Route constants declared in another file (`@Path(Paths.ITEMS)` with
//! `Paths` elsewhere in the repository), resolved after every file of the
//! batch is parsed.
//!
//! The owner type is found through the endpoint file's imports and package
//! ([`ImportScope`]), then its file among the parsed entities or, when it
//! was not re-parsed (incremental runs), by the JVM layout convention
//! `<source root>/<package path>/<Type>.java|.kt`. Constants of that file
//! are read from its source; a constant that still cannot be resolved keeps
//! its `<expression>` placeholder.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use super::annotation::StringConstants;
use super::{extract_string_constants, normalize_path, rename_endpoint};
use crate::models::{EntityKind, ImportScope, ParsedEntity};

/// Constants of the files read so far, by path (`None`: unreadable).
type ConstantsCache = HashMap<PathBuf, Option<StringConstants>>;

/// Replace `<Owner.NAME>` placeholders of endpoint names with constants
/// declared in other files of the repository rooted at `repo_root`.
pub(crate) fn resolve_cross_file_constants(entities: &mut [ParsedEntity], repo_root: &Path) {
    let type_files: HashMap<String, String> = entities
        .iter()
        .filter(|e| is_jvm_type(&e.kind))
        .map(|e| (e.fqn.clone(), e.file_path.clone()))
        .collect();
    let mut cache = ConstantsCache::new();
    for entity in entities.iter_mut() {
        if entity.kind != EntityKind::HttpEndpoint || !entity.name.contains('<') {
            continue;
        }
        let Some((verb, path)) = entity.name.split_once(' ') else {
            continue;
        };
        let lookup = OwnerLookup {
            endpoint: entity,
            type_files: &type_files,
            repo_root,
        };
        let resolved = normalize_path(&substitute_placeholders(path, |expr| {
            lookup.constant(expr, &mut cache)
        }));
        if resolved != path {
            *entity = rename_endpoint(entity, &format!("{verb} {resolved}"));
        }
    }
}

fn is_jvm_type(kind: &EntityKind) -> bool {
    matches!(
        kind,
        EntityKind::Class
            | EntityKind::Interface
            | EntityKind::Enum
            | EntityKind::KotlinClass
            | EntityKind::KotlinInterface
            | EntityKind::KotlinObject
    )
}

/// Rewrite every `<expr>` of `path` with `resolve(expr)` when it succeeds.
fn substitute_placeholders(path: &str, mut resolve: impl FnMut(&str) -> Option<String>) -> String {
    let mut out = String::with_capacity(path.len());
    let mut rest = path;
    while let Some(open) = rest.find('<') {
        let Some(close) = rest[open..].find('>').map(|i| open + i) else {
            break;
        };
        out.push_str(&rest[..open]);
        let expr = &rest[open + 1..close];
        match resolve(expr) {
            Some(value) => out.push_str(&value),
            None => out.push_str(&rest[open..=close]),
        }
        rest = &rest[close + 1..];
    }
    out.push_str(rest);
    out
}

/// Where the owner types an endpoint's placeholders name can be found.
struct OwnerLookup<'a> {
    endpoint: &'a ParsedEntity,
    type_files: &'a HashMap<String, String>,
    repo_root: &'a Path,
}

impl OwnerLookup<'_> {
    /// The value of `Owner.NAME` / `pkg.Owner.NAME`.
    fn constant(&self, expr: &str, cache: &mut ConstantsCache) -> Option<String> {
        let (owner, name) = expr.rsplit_once('.')?;
        let owner_simple = owner.rsplit('.').next()?;
        self.owner_fqns(owner)
            .into_iter()
            .filter_map(|fqn| self.owner_file(&fqn))
            .find_map(|file| {
                let constants = cache
                    .entry(file.clone())
                    .or_insert_with(|| read_constants(&file))
                    .as_ref()?;
                constants.get(&format!("{owner_simple}.{name}")).cloned()
            })
    }

    /// Candidate FQNs of `owner` in Java/Kotlin precedence order.
    fn owner_fqns(&self, owner: &str) -> Vec<String> {
        if owner.contains('.') {
            return vec![owner.to_string()];
        }
        let Some(scope) = self.endpoint.import_scope.as_deref() else {
            return Vec::new();
        };
        match scope.imported(owner) {
            Some(fqn) => vec![fqn.to_string()],
            None => scope.implicit_candidates(owner).collect(),
        }
    }

    /// The file declaring `owner_fqn`: a parsed entity's, else the JVM
    /// layout convention relative to the endpoint file's source root.
    fn owner_file(&self, owner_fqn: &str) -> Option<PathBuf> {
        if let Some(file) = self.type_files.get(owner_fqn) {
            return Some(self.repo_root.join(file));
        }
        let root = source_root(
            &self.endpoint.file_path,
            self.endpoint.import_scope.as_deref(),
        )?;
        let relative = owner_fqn.replace('.', "/");
        ["java", "kt"]
            .iter()
            .map(|ext| self.repo_root.join(&root).join(format!("{relative}.{ext}")))
            .find(|candidate| candidate.is_file())
    }
}

/// `src/main/java` for `src/main/java/com/x/Api.java` in package `com.x`.
fn source_root(file_path: &str, scope: Option<&ImportScope>) -> Option<PathBuf> {
    let package_dir = scope?.package.as_deref()?.replace('.', "/");
    let dir = Path::new(file_path).parent()?;
    let dir = dir.to_str()?.replace('\\', "/");
    let root = dir.strip_suffix(&package_dir)?.trim_end_matches('/');
    Some(PathBuf::from(root))
}

/// String constants of a Java/Kotlin source file.
fn read_constants(path: &Path) -> Option<StringConstants> {
    let source = std::fs::read_to_string(path).ok()?;
    let (language, lang_name): (tree_sitter::Language, &str) =
        match path.extension().and_then(|e| e.to_str()) {
            Some("java") => (tree_sitter_java::LANGUAGE.into(), "java"),
            Some("kt") => (tree_sitter_kotlin_ng::LANGUAGE.into(), "kotlin"),
            _ => return None,
        };
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&language).ok()?;
    let tree = parser.parse(&source, None)?;
    Some(extract_string_constants(
        tree.root_node(),
        source.as_bytes(),
        lang_name,
    ))
}
