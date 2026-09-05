//! MSBuild (`.csproj`, `Directory.Packages.props`) parser.
//!
//! Handwritten, `roxmltree`-based — mirrors the structure of
//! [`crate::pipeline::parser::languages::xml`] (Maven). Two public entry
//! points are required because the dispatcher in
//! [`crate::pipeline::parser::mod`] uses both extension-based dispatch
//! (`.csproj`) and filename-first dispatch (`Directory.Packages.props`):
//!
//! - [`extract_entities_csproj`] — emits one `ProjectIdentity` and one
//!   `BuildDependency` per `<PackageReference>`. Version comes from the
//!   element's `Version` attribute; if absent, falls back to the
//!   Central Package Management map built from
//!   [`extract_entities_props`], then to `"unknown"`.
//! - [`extract_entities_props`] — emits no entities in v1, but parses the
//!   file so the process-local CPM cache can be populated. Exists so the
//!   dispatcher is total and the `props` filename branch has a target.
//!
//! Identity fallback chain for the project FQN:
//!
//! 1. `<PackageId>...</PackageId>` (explicit; signals a published package)
//! 2. `<AssemblyName>...</AssemblyName>`
//! 3. The csproj's file stem (the `gradle.rs:127-134` containing-directory
//!    precedent)
//!
//! MSBuild files are commonly UTF-8 with a leading BOM on Windows — defensive
//! strip is applied unconditionally before parsing (see §10.4).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use crate::models::{EntityKind, ParsedEntity};

/// Identity marker carried by `ProjectIdentity.signature` when the identity
/// came from an explicit `<PackageId>`. The cross-repo resolver
/// (`cross_repo.rs`) prefers this marker over the depth tie (see §11.5).
const PACKAGE_ID_MARKER: &str = "identity: package_id";

/// UTF-8 byte-order mark.
const UTF8_BOM: &[u8] = &[0xEF, 0xBB, 0xBF];

/// Strip a leading UTF-8 BOM if present. The Maven XML parser never had to
/// do this, but MSBuild files authored on Windows routinely carry one and
/// the spec pins the behavior rather than relying on `roxmltree`'s
/// tolerance (which is reported inconsistently across versions — see
/// §10.4).
fn strip_utf8_bom(source: &str) -> &str {
    source.strip_prefix('\u{FEFF}').unwrap_or(source)
}

/// Local-name child lookup — recursive, since `<PackageId>`, `<Version>`,
/// `<AssemblyName>` live one or two levels deep (e.g. inside
/// `<PropertyGroup>`). Mirrors `xml.rs:130-137` semantics for direct
/// children but walks the subtree for MSBuild's nested shape.
fn child_text(parent: &roxmltree::Node, tag: &str) -> Option<String> {
    for node in parent.descendants() {
        if node.tag_name().name() == tag
            && let Some(text) = node.text()
        {
            return Some(text.trim().to_string());
        }
    }
    None
}

/// Locate the first attribute value matching `attr` on `node`. Local-name
/// match — namespaces are ignored.
fn attr(node: roxmltree::Node, attr: &str) -> Option<String> {
    node.attributes()
        .find(|a| a.name() == attr)
        .map(|a| a.value().trim().to_string())
}

/// Strip a leading UTF-8 BOM from raw bytes. Used by the dispatcher which
/// reads the source as bytes.
fn strip_bom_bytes(bytes: &[u8]) -> &[u8] {
    bytes.strip_prefix(UTF8_BOM).unwrap_or(bytes)
}

// ─── Central Package Management (CPM) cache ────────────────────────────
//
// `parse_single_file` runs under Rayon, so N csproj files in a repo must
// not each re-read the props file. A process-local cache keyed by the
// resolved absolute path of the props file is the cheapest correct option
// (see §11.4).
//
// Lifetime is process; build files do not change mid-run.

type CpmMap = HashMap<String, String>;

/// Process-wide cache: resolved props path → parsed CPM map.
static CPM_CACHE: OnceLock<Mutex<HashMap<PathBuf, CpmMap>>> = OnceLock::new();

fn cpm_cache() -> &'static Mutex<HashMap<PathBuf, CpmMap>> {
    CPM_CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Find the nearest ancestor `Directory.Packages.props`, starting from
/// `start_dir` and walking up to `repo_root`. Returns the resolved path
/// (which may not exist if no props file was found) or `None` when the
/// walk exhausts the repo root.
///
/// `start_dir` and `repo_root` are absolute paths; canonicalization is the
/// caller's responsibility (the dispatcher already canonicals
/// `repo_root` once per run, §10.1).
fn find_nearest_props(start_dir: &Path, repo_root: &Path) -> Option<PathBuf> {
    let mut current: Option<&Path> = Some(start_dir);
    while let Some(dir) = current {
        let candidate = dir.join("Directory.Packages.props");
        if candidate.is_file() {
            return Some(candidate);
        }
        // Stop at the repo root — beyond that, CPM inheritance does not
        // exist by definition.
        if dir == repo_root {
            return None;
        }
        current = dir.parent();
    }
    None
}

/// Read the props file at `path` and parse the `<PackageVersion>` entries
/// into a `(Include → Version)` map. Returns an empty map on parse error
/// or missing file — callers treat missing as "no overrides available".
fn parse_props_file(path: &Path) -> CpmMap {
    let Ok(bytes) = std::fs::read(path) else {
        return HashMap::new();
    };
    let Ok(source) = std::str::from_utf8(strip_bom_bytes(&bytes)) else {
        return HashMap::new();
    };
    let doc = match roxmltree::Document::parse(source) {
        Ok(d) => d,
        Err(_) => return HashMap::new(),
    };
    let mut map = HashMap::new();
    for item in doc.descendants().filter(|n| {
        n.tag_name().name() == "PackageVersion" || n.tag_name().name() == "PackageVersionOverride"
    }) {
        if let (Some(include), Some(version)) = (attr(item, "Include"), attr(item, "Version")) {
            map.insert(include, version);
        }
    }
    map
}

/// Get (and lazily populate) the CPM map for the props file nearest to
/// `start_dir`. The cache key is the resolved absolute path of the props
/// file; same props file shared across N csproj projects → 1 read.
fn cpm_map_for(start_dir: &Path, repo_root: &Path) -> CpmMap {
    let Some(props_path) = find_nearest_props(start_dir, repo_root) else {
        return HashMap::new();
    };
    let cache = cpm_cache();
    let mut guard = cache
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(map) = guard.get(&props_path) {
        return map.clone();
    }
    let map = parse_props_file(&props_path);
    guard.insert(props_path, map.clone());
    map
}

// ─── Public API ─────────────────────────────────────────────────────────

/// Parse a `.csproj` source and emit one `ProjectIdentity` and one
/// `BuildDependency` per `<PackageReference>`. Used by the dispatcher when
/// `Path::extension() == "csproj"`.
///
/// `source` is the file contents; `file_path` is the canonical repo-relative
/// path; `repo_name` is the logical repo name; `csproj_abs_dir` is the
/// directory of the csproj (used to locate `Directory.Packages.props`);
/// `repo_root` is the canonical repo root (used as the CPM walk
/// boundary).
pub(crate) fn extract_entities_csproj(ctx: &MsbuildContext<'_>) -> Vec<ParsedEntity> {
    let mut entities = Vec::new();

    let doc = match roxmltree::Document::parse(strip_utf8_bom(ctx.source)) {
        Ok(d) => d,
        Err(_) => return entities,
    };
    let root = doc.root_element();

    // Build the CPM map once per csproj (cached by props path).
    let cpm = cpm_map_for(ctx.csproj_abs_dir, ctx.repo_root);

    // Identity resolution: PackageId → AssemblyName → file stem.
    let stem = Path::new(ctx.file_path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown");
    let (identity, identity_marker) = if let Some(pkg_id) = child_text(&root, "PackageId") {
        (pkg_id, Some(PACKAGE_ID_MARKER))
    } else if let Some(asm) = child_text(&root, "AssemblyName") {
        (asm, None)
    } else {
        (stem.to_string(), None)
    };

    let version = child_text(&root, "Version").unwrap_or_else(|| "unknown".to_string());

    let fqn = format!("nuget:{identity}");
    let mut signature = format!("version: {}, build_system: nuget", version);
    if let Some(marker) = identity_marker {
        signature.push_str(", ");
        signature.push_str(marker);
    }

    entities.push(ParsedEntity::new(
        &identity,
        EntityKind::ProjectIdentity,
        &fqn,
        Some(signature),
        Some(format!("NuGet project identity: {identity}")),
        "csproj",
        ctx.file_path,
        1,
        1,
        None,
        ctx.repo_name,
    ));

    // Walk for <PackageReference> elements. We accept any of:
    //   <PackageReference Include="X" Version="Y" />
    //   <PackageReference Include="X">         ← version via CPM or unknown
    //   <PackageReference Update="X" Version="Y" />   (Update is rare but valid)
    //   <PackageReference Update="X" />        (CPM fallback)
    //
    // <ProjectReference> is intentionally skipped (see §10.4 / §15).
    for item in root
        .descendants()
        .filter(|n| n.tag_name().name() == "PackageReference")
    {
        let include = attr(item, "Include").or_else(|| attr(item, "Update"));
        let Some(name) = include else { continue };

        // Version resolution: attribute → CPM → "unknown"
        let version = if let Some(v) = attr(item, "Version") {
            v
        } else if let Some(v) = cpm.get(&name) {
            v.clone()
        } else {
            "unknown".to_string()
        };

        let dep_name = format!("nuget:{name}:{version}");
        let dep_fqn = dep_name.clone();

        entities.push(ParsedEntity::new(
            &dep_name,
            EntityKind::BuildDependency,
            &dep_fqn,
            None,
            Some(format!("NuGet dependency: {name}")),
            "csproj",
            ctx.file_path,
            1,
            1,
            None,
            ctx.repo_name,
        ));
    }

    entities
}

/// Aggregated inputs to the MSBuild parser.
///
/// The csproj parsing needs five distinct inputs (file contents + path + repo
/// name + csproj directory + repo root for CPM). Bundling them into one
/// struct keeps the function signature under `clippy::too_many_arguments`'s
/// threshold without sacrificing the explicit call site at the dispatcher.
pub(crate) struct MsbuildContext<'a> {
    pub source: &'a str,
    pub file_path: &'a str,
    pub repo_name: &'a str,
    pub csproj_abs_dir: &'a Path,
    pub repo_root: &'a Path,
}

/// Placeholder entry for `Directory.Packages.props`. Emits no entities —
/// the file only contributes to the CPM map (populated lazily by
/// `extract_entities_csproj` via [`cpm_map_for`]). Exists so the
/// dispatcher has a target and the file is "discovered" even when no
/// csproj consumes it.
pub(crate) fn extract_entities_props(
    _source: &str,
    _file_path: &str,
    _repo_name: &str,
) -> Vec<ParsedEntity> {
    // Deliberately empty: the only artefact that matters (the CPM map)
    // is built on demand by `cpm_map_for`, not by iterating every props
    // file at parse time.
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build the [`MsbuildContext`] and call the parser. Tests pass only
    /// the four inputs they care about; `repo_name` is fixed at
    /// `"test-repo"` (matches the cross-repo tests' convention).
    fn call_extract(
        source: &str,
        file_path: &str,
        abs_dir: &str,
        abs_root: &str,
    ) -> Vec<ParsedEntity> {
        let ctx = MsbuildContext {
            source,
            file_path,
            repo_name: "test-repo",
            csproj_abs_dir: Path::new(abs_dir),
            repo_root: Path::new(abs_root),
        };
        extract_entities_csproj(&ctx)
    }

    // ---- B-2 .csproj parser core ----

    #[test]
    fn test_extract_csproj_package_reference_attribute_version() {
        // Realistic shape — `<PackageReference Include="X" Version="Y"/>`.
        let source = r#"<Project Sdk="Microsoft.NET.Sdk">
  <PropertyGroup>
    <TargetFramework>net8.0</TargetFramework>
  </PropertyGroup>
  <ItemGroup>
    <PackageReference Include="Tomlyn" Version="0.17.0" />
  </ItemGroup>
</Project>"#;
        let entities = call_extract(source, "src/App/App.csproj", "/repo/src/App", "/repo");

        let deps: Vec<_> = entities
            .iter()
            .filter(|e| e.kind == EntityKind::BuildDependency)
            .collect();
        assert_eq!(deps.len(), 1, "expected one BuildDependency");
        assert_eq!(deps[0].name, "nuget:Tomlyn:0.17.0");
        assert_eq!(deps[0].fqn, "nuget:Tomlyn:0.17.0");
        assert!(deps[0].docstring.as_ref().unwrap().contains("NuGet"));
    }

    #[test]
    fn test_extract_csproj_identity_from_package_id() {
        // Real `CodeMap.Daemon.csproj` shape.
        let source = r#"<Project Sdk="Microsoft.NET.Sdk">
  <PropertyGroup>
    <PackageId>codemap-mcp</PackageId>
    <Version>2.8.1</Version>
    <TargetFramework>net8.0</TargetFramework>
  </PropertyGroup>
</Project>"#;
        let entities = call_extract(
            source,
            "src/CodeMap.Daemon/CodeMap.Daemon.csproj",
            "/repo/src/CodeMap.Daemon",
            "/repo",
        );

        let identities: Vec<_> = entities
            .iter()
            .filter(|e| e.kind == EntityKind::ProjectIdentity)
            .collect();
        assert_eq!(identities.len(), 1);
        assert_eq!(identities[0].name, "codemap-mcp");
        assert_eq!(identities[0].fqn, "nuget:codemap-mcp");
        let sig = identities[0].signature.as_ref().unwrap();
        assert!(sig.contains("version: 2.8.1"));
        assert!(sig.contains("build_system: nuget"));
        assert!(
            sig.contains("identity: package_id"),
            "PackageId path must carry the marker, got {sig}"
        );
    }

    #[test]
    fn test_extract_csproj_identity_falls_back_to_assembly_name() {
        let source = r#"<Project Sdk="Microsoft.NET.Sdk">
  <PropertyGroup>
    <AssemblyName>OpenLogi.Agent</AssemblyName>
  </PropertyGroup>
</Project>"#;
        let entities = call_extract(
            source,
            "src/OpenLogi/OpenLogi.Agent.csproj",
            "/repo/src/OpenLogi",
            "/repo",
        );

        let identities: Vec<_> = entities
            .iter()
            .filter(|e| e.kind == EntityKind::ProjectIdentity)
            .collect();
        assert_eq!(identities.len(), 1);
        assert_eq!(identities[0].name, "OpenLogi.Agent");
        assert_eq!(identities[0].fqn, "nuget:OpenLogi.Agent");
        let sig = identities[0].signature.as_ref().unwrap();
        assert!(
            !sig.contains("identity: package_id"),
            "AssemblyName path must NOT carry the marker, got {sig}"
        );
    }

    #[test]
    fn test_extract_csproj_identity_falls_back_to_file_stem() {
        // No `<PackageId>`, no `<AssemblyName>` → csproj file stem.
        let source = r#"<Project Sdk="Microsoft.NET.Sdk">
  <PropertyGroup>
    <TargetFramework>net8.0</TargetFramework>
  </PropertyGroup>
</Project>"#;
        let entities = call_extract(source, "src/MyApp/MyApp.csproj", "/repo/src/MyApp", "/repo");

        let identities: Vec<_> = entities
            .iter()
            .filter(|e| e.kind == EntityKind::ProjectIdentity)
            .collect();
        assert_eq!(identities.len(), 1);
        assert_eq!(identities[0].name, "MyApp");
        assert_eq!(identities[0].fqn, "nuget:MyApp");
    }

    #[test]
    fn test_extract_csproj_version_property() {
        let source = r#"<Project Sdk="Microsoft.NET.Sdk">
  <PropertyGroup>
    <Version>1.4.0</Version>
  </PropertyGroup>
</Project>"#;
        let entities = call_extract(source, "src/MyApp/MyApp.csproj", "/repo/src/MyApp", "/repo");
        let identities: Vec<_> = entities
            .iter()
            .filter(|e| e.kind == EntityKind::ProjectIdentity)
            .collect();
        let sig = identities[0].signature.as_ref().unwrap();
        assert!(sig.contains("version: 1.4.0"), "got {sig}");
    }

    #[test]
    fn test_extract_csproj_project_reference_skipped() {
        // `<ProjectReference>` must NOT emit any `BuildDependency`.
        let source = r#"<Project Sdk="Microsoft.NET.Sdk">
  <ItemGroup>
    <ProjectReference Include="..\Other\Other.csproj" />
  </ItemGroup>
</Project>"#;
        let entities = call_extract(source, "src/App/App.csproj", "/repo/src/App", "/repo");
        let deps: Vec<_> = entities
            .iter()
            .filter(|e| e.kind == EntityKind::BuildDependency)
            .collect();
        assert!(deps.is_empty(), "ProjectReference must be skipped");
    }

    #[test]
    fn test_extract_csproj_tolerates_utf8_bom() {
        // Exact BOM bytes (EF BB BF) followed by a tiny project file.
        let source = "\u{FEFF}<Project Sdk=\"Microsoft.NET.Sdk\">
  <PropertyGroup>
    <PackageId>codemap-mcp</PackageId>
    <Version>2.8.1</Version>
  </PropertyGroup>
  <ItemGroup>
    <PackageReference Include=\"Tomlyn\" Version=\"0.17.0\" />
  </ItemGroup>
</Project>";

        let entities = call_extract(
            source,
            "src/CodeMap.Daemon/CodeMap.Daemon.csproj",
            "/repo/src/CodeMap.Daemon",
            "/repo",
        );

        // Without the BOM strip, roxmltree 0.21.1 would refuse to parse
        // and the function would silently return an empty vec.
        assert!(
            !entities.is_empty(),
            "BOM-stripped parse must yield entities"
        );

        let identities: Vec<_> = entities
            .iter()
            .filter(|e| e.kind == EntityKind::ProjectIdentity)
            .collect();
        assert_eq!(
            identities.len(),
            1,
            "ProjectIdentity must survive BOM strip"
        );
        assert_eq!(identities[0].name, "codemap-mcp");

        let deps: Vec<_> = entities
            .iter()
            .filter(|e| e.kind == EntityKind::BuildDependency)
            .collect();
        assert_eq!(deps.len(), 1);
        assert_eq!(deps[0].name, "nuget:Tomlyn:0.17.0");
    }

    #[test]
    fn test_extract_csproj_empty_project_yields_identity_only() {
        // An empty `<Project />` still has a file stem → still emits the
        // identity (with "unknown" version and stem-derived name). No
        // dependencies though.
        let source = r#"<Project />"#;
        let entities = call_extract(source, "src/Empty/Empty.csproj", "/repo/src/Empty", "/repo");
        let deps: Vec<_> = entities
            .iter()
            .filter(|e| e.kind == EntityKind::BuildDependency)
            .collect();
        assert!(deps.is_empty(), "no dependencies from an empty project");
        let identities: Vec<_> = entities
            .iter()
            .filter(|e| e.kind == EntityKind::ProjectIdentity)
            .collect();
        assert_eq!(identities.len(), 1);
        assert_eq!(identities[0].name, "Empty");
        assert_eq!(identities[0].fqn, "nuget:Empty");
    }

    #[test]
    fn test_extract_csproj_xmlns_and_conditions_ignored() {
        // Legacy namespace and a `Condition` attribute on a PackageReference
        // must not disrupt extraction.
        let source = r#"<?xml version="1.0" encoding="utf-8"?>
<Project xmlns="http://schemas.microsoft.com/developer/msbuild/2003">
  <ItemGroup>
    <PackageReference Include="Tomlyn" Version="0.17.0" Condition="'$(Configuration)' == 'Release'" />
  </ItemGroup>
</Project>"#;
        let entities = call_extract(
            source,
            "src/Legacy/Legacy.csproj",
            "/repo/src/Legacy",
            "/repo",
        );
        let deps: Vec<_> = entities
            .iter()
            .filter(|e| e.kind == EntityKind::BuildDependency)
            .collect();
        assert_eq!(deps.len(), 1, "xmlns + Condition must not block extraction");
        assert_eq!(deps[0].name, "nuget:Tomlyn:0.17.0");
    }

    #[test]
    fn test_extract_csproj_versionless_without_props_is_unknown() {
        // No `Version` attribute and no CPM available → version "unknown".
        let source = r#"<Project Sdk="Microsoft.NET.Sdk">
  <ItemGroup>
    <PackageReference Include="LibGit2Sharp" />
  </ItemGroup>
</Project>"#;
        // Use a unique tmp-style repo root with NO `Directory.Packages.props`
        // (the test's CWD has none) and walk boundary `/repo`.
        let entities = call_extract(
            source,
            "src/App/App.csproj",
            "/__nonexistent_repo__/src/App",
            "/__nonexistent_repo__",
        );
        let deps: Vec<_> = entities
            .iter()
            .filter(|e| e.kind == EntityKind::BuildDependency)
            .collect();
        assert_eq!(deps.len(), 1);
        assert_eq!(deps[0].name, "nuget:LibGit2Sharp:unknown");
    }

    #[test]
    fn test_extract_csproj_package_reference_update_attribute() {
        // The rare `Update="..."` form must still extract.
        let source = r#"<Project Sdk="Microsoft.NET.Sdk">
  <ItemGroup>
    <PackageReference Update="Serilog" Version="3.1.0" />
  </ItemGroup>
</Project>"#;
        let entities = call_extract(source, "src/App/App.csproj", "/repo/src/App", "/repo");
        let deps: Vec<_> = entities
            .iter()
            .filter(|e| e.kind == EntityKind::BuildDependency)
            .collect();
        assert_eq!(deps.len(), 1);
        assert_eq!(deps[0].name, "nuget:Serilog:3.1.0");
    }

    // ---- B-3 CPM resolution ----

    #[test]
    fn test_extract_csproj_cpm_resolves_version_from_props() {
        // Real csharp-code-map shapes: 78/83 of the PackageReferences are
        // version-less and resolve via `Directory.Packages.props`.
        let csproj = r#"<Project Sdk="Microsoft.NET.Sdk">
  <ItemGroup>
    <PackageReference Include="LibGit2Sharp" />
    <PackageReference Include="Tomlyn" Version="0.17.0" />
  </ItemGroup>
</Project>"#;

        let dir = tempfile::tempdir().unwrap();
        let repo_root = dir.path();
        let app_dir = repo_root.join("src").join("App");
        std::fs::create_dir_all(&app_dir).unwrap();
        std::fs::write(app_dir.join("App.csproj"), csproj).unwrap();

        let props_source = r#"<Project>
  <ItemGroup>
    <PackageVersion Include="LibGit2Sharp" Version="0.30.0" />
    <PackageVersion Include="Tomlyn" Version="0.17.0" />
  </ItemGroup>
</Project>"#;
        // CPM file at the repo root.
        std::fs::write(repo_root.join("Directory.Packages.props"), props_source).unwrap();

        let entities = call_extract(
            csproj,
            "src/App/App.csproj",
            app_dir.to_str().unwrap(),
            repo_root.to_str().unwrap(),
        );
        let deps: Vec<_> = entities
            .iter()
            .filter(|e| e.kind == EntityKind::BuildDependency)
            .collect();
        assert_eq!(deps.len(), 2);

        let lib = deps
            .iter()
            .find(|d| d.name.starts_with("nuget:LibGit2Sharp"))
            .unwrap();
        assert_eq!(
            lib.name, "nuget:LibGit2Sharp:0.30.0",
            "CPM must resolve the version"
        );

        let tomlyn = deps
            .iter()
            .find(|d| d.name.starts_with("nuget:Tomlyn"))
            .unwrap();
        assert_eq!(tomlyn.name, "nuget:Tomlyn:0.17.0");
    }

    #[test]
    fn test_extract_csproj_cpm_nearest_ancestor_wins() {
        // Props in the project dir shadows a root props file.
        let csproj = r#"<Project Sdk="Microsoft.NET.Sdk">
  <ItemGroup>
    <PackageReference Include="Foo" />
  </ItemGroup>
</Project>"#;

        let dir = tempfile::tempdir().unwrap();
        let repo_root = dir.path();
        let app_dir = repo_root.join("src").join("App");
        std::fs::create_dir_all(&app_dir).unwrap();
        std::fs::write(app_dir.join("App.csproj"), csproj).unwrap();

        // Root props declares 1.0.0.
        let root_props = r#"<Project>
  <ItemGroup>
    <PackageVersion Include="Foo" Version="1.0.0" />
  </ItemGroup>
</Project>"#;
        std::fs::write(repo_root.join("Directory.Packages.props"), root_props).unwrap();

        // Project-local props overrides with 2.0.0.
        let local_props = r#"<Project>
  <ItemGroup>
    <PackageVersion Include="Foo" Version="2.0.0" />
  </ItemGroup>
</Project>"#;
        std::fs::write(app_dir.join("Directory.Packages.props"), local_props).unwrap();

        let entities = call_extract(
            csproj,
            "src/App/App.csproj",
            app_dir.to_str().unwrap(),
            repo_root.to_str().unwrap(),
        );
        let deps: Vec<_> = entities
            .iter()
            .filter(|e| e.kind == EntityKind::BuildDependency)
            .collect();
        assert_eq!(deps.len(), 1);
        assert_eq!(
            deps[0].name, "nuget:Foo:2.0.0",
            "nearest ancestor must win (2.0.0 from the project dir)"
        );
    }

    #[test]
    fn test_cpm_cache_parses_props_once() {
        // Two csproj projects sharing the same props file → only one
        // parse (verified indirectly: after both calls, the cache key for
        // this props path exists and resolves the right version). The
        // process-wide cache may accumulate entries from sibling tests
        // running concurrently — that's expected; we only assert on the
        // path under test.
        let csproj = r#"<Project Sdk="Microsoft.NET.Sdk">
  <ItemGroup>
    <PackageReference Include="Shared" />
  </ItemGroup>
</Project>"#;

        let dir = tempfile::tempdir().unwrap();
        let repo_root = dir.path();
        let a_dir = repo_root.join("src").join("A");
        let b_dir = repo_root.join("src").join("B");
        std::fs::create_dir_all(&a_dir).unwrap();
        std::fs::create_dir_all(&b_dir).unwrap();
        std::fs::write(a_dir.join("A.csproj"), csproj).unwrap();
        std::fs::write(b_dir.join("B.csproj"), csproj).unwrap();

        let props_source = r#"<Project>
  <ItemGroup>
    <PackageVersion Include="Shared" Version="9.9.9" />
  </ItemGroup>
</Project>"#;
        let props_path = repo_root.join("Directory.Packages.props");
        std::fs::write(&props_path, props_source).unwrap();

        // First call populates the cache for this props path.
        let entities_a = call_extract(
            csproj,
            "src/A/A.csproj",
            a_dir.to_str().unwrap(),
            repo_root.to_str().unwrap(),
        );
        // Second call must hit the cache (no second read) and produce the
        // same resolution.
        let entities_b = call_extract(
            csproj,
            "src/B/B.csproj",
            b_dir.to_str().unwrap(),
            repo_root.to_str().unwrap(),
        );

        for entities in [&entities_a, &entities_b] {
            let dep = entities
                .iter()
                .find(|e| e.kind == EntityKind::BuildDependency)
                .unwrap();
            assert_eq!(dep.name, "nuget:Shared:9.9.9");
        }

        // The cache must contain this exact props path (independently of
        // any other tests that may be populating it concurrently).
        let cache = cpm_cache().lock().unwrap();
        let entry = cache
            .get(&props_path)
            .expect("props path must be cached after at least one csproj lookup");
        assert_eq!(entry.get("Shared").map(String::as_str), Some("9.9.9"));
    }

    // ---- B-1 fallback / no-props ----

    #[test]
    fn test_extract_csproj_under_root_dir_with_no_props() {
        // Repo root with no props file → version falls through to "unknown".
        let csproj = r#"<Project Sdk="Microsoft.NET.Sdk">
  <ItemGroup>
    <PackageReference Include="Foo" />
  </ItemGroup>
</Project>"#;

        let dir = tempfile::tempdir().unwrap();
        let repo_root = dir.path();
        let app_dir = repo_root.join("src").join("App");
        std::fs::create_dir_all(&app_dir).unwrap();
        std::fs::write(app_dir.join("App.csproj"), csproj).unwrap();

        let entities = call_extract(
            csproj,
            "src/App/App.csproj",
            app_dir.to_str().unwrap(),
            repo_root.to_str().unwrap(),
        );
        let deps: Vec<_> = entities
            .iter()
            .filter(|e| e.kind == EntityKind::BuildDependency)
            .collect();
        assert_eq!(deps.len(), 1);
        assert_eq!(deps[0].name, "nuget:Foo:unknown");
    }

    #[test]
    fn test_strip_bom_bytes_handles_no_bom() {
        let no_bom = b"<Project />";
        assert_eq!(strip_bom_bytes(no_bom), no_bom);
    }

    #[test]
    fn test_strip_bom_bytes_strips_leading_bom() {
        let bom = [0xEF, 0xBB, 0xBF];
        let mut with_bom = Vec::from(bom);
        with_bom.extend_from_slice(b"<Project />");
        let stripped = strip_bom_bytes(&with_bom);
        assert_eq!(stripped, b"<Project />");
    }

    #[test]
    fn test_extract_entities_props_is_empty() {
        // v1: props file produces no entities of its own. CPM is consumed
        // lazily by csproj.
        let entities = extract_entities_props("<Project />", "Directory.Packages.props", "x");
        assert!(entities.is_empty());
    }
}
