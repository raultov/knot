//! Crate discovery for Rust repositories.
//!
//! Walks a repository tree looking for `Cargo.toml` files and records the
//! directory in which each one lives plus the parsed crate name. The result
//! is used by the parser to prefix Rust entity FQNs with their owning crate
//! (e.g. `crate_a::config::Config`) so that the same type defined in two
//! different crates does not collide on the bare name.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// A single Rust crate discovered in the repository.
#[derive(Debug, Clone)]
pub struct CrateRoot {
    pub crate_name: String,
    pub root_dir: PathBuf,
}

/// Result of walking a repository for `Cargo.toml` files.
#[derive(Debug, Clone, Default)]
pub struct CrateDiscovery {
    crates: BTreeMap<PathBuf, CrateRoot>,
}

impl CrateDiscovery {
    /// Walk `repo_path` recursively, registering one [`CrateRoot`] per
    /// `Cargo.toml` found. Hidden directories, `target/`, and `node_modules/`
    /// are skipped.
    pub fn discover(repo_path: &Path) -> Self {
        let mut crates = BTreeMap::new();
        Self::walk_for_cargo_toml(repo_path, &mut crates);
        Self { crates }
    }

    fn walk_for_cargo_toml(dir: &Path, crates: &mut BTreeMap<PathBuf, CrateRoot>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                if name.starts_with('.') || name == "target" || name == "node_modules" {
                    continue;
                }
                Self::walk_for_cargo_toml(&path, crates);
            } else if path.file_name().and_then(|n| n.to_str()) == Some("Cargo.toml")
                && let Some(crate_root) = Self::parse_cargo_toml(&path)
                && let Some(parent) = path.parent()
            {
                crates.insert(parent.to_path_buf(), crate_root);
            }
        }
    }

    fn parse_cargo_toml(path: &Path) -> Option<CrateRoot> {
        let content = std::fs::read_to_string(path).ok()?;
        let mut crate_name: Option<String> = None;
        let mut in_package_section = false;

        for raw_line in content.lines() {
            let trimmed = raw_line.trim();
            if let Some(section) = trimmed.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
                in_package_section = section.trim() == "package";
                continue;
            }

            if !in_package_section {
                continue;
            }

            if let Some(rest) = trimmed.strip_prefix("name") {
                let after_eq = rest.trim_start().strip_prefix('=')?;
                let value = after_eq.trim();
                let value = value.split('#').next().unwrap_or(value).trim();
                let name = value.trim_matches('"').trim_matches('\'');
                if !name.is_empty() {
                    crate_name = Some(name.replace('-', "_"));
                    break;
                }
            }
        }

        if crate_name.is_none() {
            for raw_line in content.lines() {
                let trimmed = raw_line.trim();
                if let Some(rest) = trimmed.strip_prefix("name") {
                    let Some(after_eq) = rest.trim_start().strip_prefix('=') else {
                        continue;
                    };
                    let value = after_eq.trim();
                    let value = value.split('#').next().unwrap_or(value).trim();
                    let name = value.trim_matches('"').trim_matches('\'');
                    if !name.is_empty() {
                        crate_name = Some(name.replace('-', "_"));
                        break;
                    }
                }
            }
        }

        let crate_name = crate_name?;
        let root_dir = path.parent()?.to_path_buf();
        Some(CrateRoot {
            crate_name,
            root_dir,
        })
    }

    /// Return the most deeply nested crate that contains `file_path`, if any.
    pub fn crate_for_file(&self, file_path: &Path) -> Option<&CrateRoot> {
        self.crates
            .iter()
            .rev()
            .find(|(crate_dir, _)| file_path.starts_with(crate_dir))
            .map(|(_, root)| root)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    fn write_cargo_toml(dir: &Path, name: &str) {
        fs::create_dir_all(dir).unwrap();
        let content = format!(
            "[package]\nname = \"{}\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
            name
        );
        fs::write(dir.join("Cargo.toml"), content).unwrap();
    }

    #[test]
    fn test_discover_single_crate() {
        let tmp = tempdir().unwrap();
        let root = tmp.path();
        write_cargo_toml(root, "my_crate");

        let discovery = CrateDiscovery::discover(root);
        let crate_root = discovery
            .crate_for_file(&root.join("src/lib.rs"))
            .expect("expected to find a crate root");
        assert_eq!(crate_root.crate_name, "my_crate");
        assert_eq!(crate_root.root_dir, root);
    }

    #[test]
    fn test_discover_workspace_members() {
        let tmp = tempdir().unwrap();
        let root = tmp.path();

        fs::write(
            root.join("Cargo.toml"),
            "[workspace]\nmembers = [\"crate_a\", \"crate_b\"]\n",
        )
        .unwrap();

        write_cargo_toml(&root.join("crate_a"), "crate_a");
        write_cargo_toml(&root.join("crate_b"), "crate_b");

        let discovery = CrateDiscovery::discover(root);

        let a = discovery
            .crate_for_file(&root.join("crate_a/src/lib.rs"))
            .expect("crate_a not found");
        assert_eq!(a.crate_name, "crate_a");

        let b = discovery
            .crate_for_file(&root.join("crate_b/src/lib.rs"))
            .expect("crate_b not found");
        assert_eq!(b.crate_name, "crate_b");
    }

    #[test]
    fn test_crate_for_file_nested() {
        let tmp = tempdir().unwrap();
        let root = tmp.path();
        write_cargo_toml(root, "deep_crate");

        let discovery = CrateDiscovery::discover(root);
        let crate_root = discovery
            .crate_for_file(&root.join("src/foo/bar/baz.rs"))
            .expect("expected to map nested file to crate");
        assert_eq!(crate_root.crate_name, "deep_crate");
    }

    #[test]
    fn test_crate_name_normalizes_dashes() {
        let tmp = tempdir().unwrap();
        let root = tmp.path();
        write_cargo_toml(root, "my-crate");

        let discovery = CrateDiscovery::discover(root);
        let crate_root = discovery
            .crate_for_file(&root.join("src/lib.rs"))
            .expect("expected to find a crate root");
        assert_eq!(crate_root.crate_name, "my_crate");
    }

    #[test]
    fn test_no_cargo_toml_returns_none() {
        let tmp = tempdir().unwrap();
        let root = tmp.path();
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(root.join("src/lib.rs"), "fn main() {}").unwrap();

        let discovery = CrateDiscovery::discover(root);
        assert!(discovery.crate_for_file(&root.join("src/lib.rs")).is_none());
    }

    #[test]
    fn test_nested_workspace_picks_deepest_crate() {
        let tmp = tempdir().unwrap();
        let root = tmp.path();
        write_cargo_toml(root, "outer");
        write_cargo_toml(&root.join("nested"), "inner");

        let discovery = CrateDiscovery::discover(root);
        let crate_root = discovery
            .crate_for_file(&root.join("nested/src/foo.rs"))
            .expect("expected to find nested crate");
        assert_eq!(crate_root.crate_name, "inner");
    }

    #[test]
    fn test_skips_target_and_node_modules() {
        let tmp = tempdir().unwrap();
        let root = tmp.path();
        write_cargo_toml(root, "main_crate");
        write_cargo_toml(&root.join("target/some_dep"), "should_skip_target");
        write_cargo_toml(
            &root.join("node_modules/some_pkg"),
            "should_skip_node_modules",
        );

        let discovery = CrateDiscovery::discover(root);
        assert!(
            discovery
                .crate_for_file(&root.join("target/some_dep/src/lib.rs"))
                .is_none_or(|c| c.crate_name == "main_crate")
        );
        assert!(
            discovery
                .crate_for_file(&root.join("node_modules/some_pkg/src/lib.rs"))
                .is_none_or(|c| c.crate_name == "main_crate")
        );
    }
}
