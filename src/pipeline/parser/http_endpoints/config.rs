//! Application configuration of a JVM repository, read from disk for HTTP
//! client linking (issue #46): `application*` / `bootstrap*`
//! `.yml|.yaml|.properties` and `microprofile-config.properties`, whether or
//! not configuration files are indexed.
//!
//! Keys are relaxed the way Spring Boot binds them (`rest-client` ≡
//! `restclient`, case-insensitive, quotes dropped). A key declared in
//! several files keeps every value; the base files (`application.yml`)
//! come before the profile ones (`application-dev.yml`) and `bootstrap*`.

use std::collections::HashMap;
use std::path::Path;

use walkdir::WalkDir;

use super::annotation::unmatched;

/// Directories never holding the application's own configuration.
const SKIPPED_DIRS: &[&str] = &["target", "build", "node_modules", "out", "bin"];

/// Nested `${…}` placeholders resolved at most this deep.
const MAX_RESOLVE_DEPTH: usize = 5;

/// Flattened configuration properties: relaxed key → values, in priority
/// order.
#[derive(Debug, Default)]
pub(crate) struct AppConfig {
    values: HashMap<String, Vec<String>>,
}

impl AppConfig {
    /// The configuration files under `repo_root`, test sources excluded.
    pub(crate) fn load(repo_root: &Path) -> Self {
        let mut files: Vec<(u8, String, std::path::PathBuf)> = WalkDir::new(repo_root)
            .into_iter()
            .filter_entry(|entry| entry.depth() == 0 || !is_skipped_dir(entry))
            .filter_map(Result::ok)
            .filter(|entry| entry.file_type().is_file())
            .filter_map(|entry| {
                let name = entry.file_name().to_str()?.to_string();
                let rank = file_rank(&name)?;
                let relative = entry.path().strip_prefix(repo_root).ok()?;
                let relative = relative.to_str()?.replace('\\', "/");
                (!relative.contains("src/test/")).then(|| (rank, relative, entry.into_path()))
            })
            .collect();
        files.sort();
        let mut config = Self::default();
        for (_, relative, path) in files {
            if let Ok(content) = std::fs::read_to_string(&path) {
                config.add_source(&relative, &content);
            }
        }
        config
    }

    /// Add the properties of one file, after those already added.
    pub(crate) fn add_source(&mut self, file_name: &str, content: &str) {
        let pairs = if file_name.ends_with(".properties") {
            properties_pairs(content)
        } else {
            yaml_pairs(content)
        };
        for (key, value) in pairs {
            self.values.entry(relax_key(&key)).or_default().push(value);
        }
    }

    /// The first value of `key`, placeholders resolved.
    pub(crate) fn get(&self, key: &str) -> Option<String> {
        self.raw(key).map(|value| self.resolve(value))
    }

    /// Every value of `key`, placeholders resolved.
    pub(crate) fn all(&self, key: &str) -> Vec<String> {
        self.values
            .get(&relax_key(key))
            .map(|values| values.iter().map(|v| self.resolve(v)).collect())
            .unwrap_or_default()
    }

    fn raw(&self, key: &str) -> Option<&str> {
        self.values
            .get(&relax_key(key))
            .and_then(|values| values.first())
            .map(String::as_str)
    }

    /// `${key}` / `${key:default}` replaced by the property value or the
    /// default; a placeholder with neither is kept as written.
    pub(crate) fn resolve(&self, text: &str) -> String {
        self.resolve_at(text, 0)
    }

    fn resolve_at(&self, text: &str, depth: usize) -> String {
        let mut out = String::with_capacity(text.len());
        let mut rest = text;
        while let Some(start) = rest.find("${") {
            out.push_str(&rest[..start]);
            let inner_start = start + 2;
            let Some(len) = placeholder_len(&rest[inner_start..]) else {
                out.push_str(&rest[start..]);
                return out;
            };
            let inner = &rest[inner_start..inner_start + len];
            let (key, default) = match split_default(inner) {
                Some((key, default)) => (key, Some(default)),
                None => (inner, None),
            };
            let value = self
                .raw(key.trim())
                .or(default)
                .filter(|_| depth < MAX_RESOLVE_DEPTH);
            match value {
                Some(value) => out.push_str(&self.resolve_at(value, depth + 1)),
                None => out.push_str(&rest[start..inner_start + len + 1]),
            }
            rest = &rest[inner_start + len + 1..];
        }
        out.push_str(rest);
        out
    }
}

fn is_skipped_dir(entry: &walkdir::DirEntry) -> bool {
    entry.file_type().is_dir()
        && entry
            .file_name()
            .to_str()
            .is_some_and(|name| name.starts_with('.') || SKIPPED_DIRS.contains(&name))
}

/// Priority of a configuration file by name; `None` for other files.
fn file_rank(name: &str) -> Option<u8> {
    let (stem, ext) = name.rsplit_once('.')?;
    if !matches!(ext, "yml" | "yaml" | "properties") {
        return None;
    }
    match stem {
        "application" => Some(0),
        "microprofile-config" if ext == "properties" => Some(0),
        _ if stem.starts_with("application-") => Some(1),
        "bootstrap" => Some(2),
        _ if stem.starts_with("bootstrap-") => Some(3),
        _ => None,
    }
}

/// Spring Boot's relaxed binding, reduced: case, dashes and quotes ignored.
fn relax_key(key: &str) -> String {
    key.chars()
        .filter(|c| !matches!(c, '-' | '"' | '\''))
        .flat_map(char::to_lowercase)
        .collect()
}

/// Length of the placeholder body `text` starts with, up to its `}`.
fn placeholder_len(text: &str) -> Option<usize> {
    unmatched(text.char_indices(), '{', '}')
}

/// `key:default` → (`key`, `default`), the `:` outside nested placeholders.
fn split_default(inner: &str) -> Option<(&str, &str)> {
    let mut depth = 0usize;
    for (i, c) in inner.char_indices() {
        match c {
            '{' => depth += 1,
            '}' => depth = depth.saturating_sub(1),
            ':' if depth == 0 => return Some((&inner[..i], &inner[i + 1..])),
            _ => {}
        }
    }
    None
}

/// `key=value`, `key: value` and `key value` lines; comments skipped.
fn properties_pairs(content: &str) -> Vec<(String, String)> {
    content
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#') && !line.starts_with('!'))
        .filter_map(|line| {
            let split = line.find(['=', ':', ' ', '\t'])?;
            let key = line[..split].trim();
            let value = line[split + 1..].trim_start_matches([' ', '\t', '=', ':']);
            (!key.is_empty()).then(|| (key.to_string(), value.trim().to_string()))
        })
        .collect()
}

/// Every scalar of every YAML document, under its dotted path.
fn yaml_pairs(content: &str) -> Vec<(String, String)> {
    let mut pairs = Vec::new();
    for document in serde_yaml::Deserializer::from_str(content) {
        if let Ok(value) = serde::Deserialize::deserialize(document) {
            flatten_yaml("", &value, &mut pairs);
        }
    }
    pairs
}

fn flatten_yaml(prefix: &str, value: &serde_yaml::Value, pairs: &mut Vec<(String, String)>) {
    let join = |key: &str| {
        if prefix.is_empty() {
            key.to_string()
        } else {
            format!("{prefix}.{key}")
        }
    };
    match value {
        serde_yaml::Value::Mapping(map) => {
            for (key, child) in map {
                if let Some(key) = scalar(key) {
                    flatten_yaml(&join(&key), child, pairs);
                }
            }
        }
        serde_yaml::Value::Sequence(items) => {
            for (i, child) in items.iter().enumerate() {
                flatten_yaml(&format!("{prefix}[{i}]"), child, pairs);
            }
        }
        serde_yaml::Value::Tagged(tagged) => flatten_yaml(prefix, &tagged.value, pairs),
        other => {
            if let Some(text) = scalar(other) {
                pairs.push((prefix.to_string(), text));
            }
        }
    }
}

fn scalar(value: &serde_yaml::Value) -> Option<String> {
    match value {
        serde_yaml::Value::String(s) => Some(s.clone()),
        serde_yaml::Value::Number(n) => Some(n.to_string()),
        serde_yaml::Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(sources: &[(&str, &str)]) -> AppConfig {
        let mut config = AppConfig::default();
        for (name, content) in sources {
            config.add_source(name, content);
        }
        config
    }

    #[test]
    fn yaml_is_flattened_with_relaxed_keys() {
        let c = config(&[(
            "application.yml",
            "quarkus:\n  rest-client:\n    \"items\":\n      url: http://items:8080/api\nserver:\n  port: 8080\n",
        )]);
        assert_eq!(
            c.get("quarkus.rest-client.items.url").as_deref(),
            Some("http://items:8080/api")
        );
        assert_eq!(
            c.get("quarkus.restClient.items.url").as_deref(),
            Some("http://items:8080/api")
        );
        assert_eq!(c.get("server.port").as_deref(), Some("8080"));
    }

    #[test]
    fn properties_lines_are_parsed() {
        let c = config(&[(
            "application.properties",
            "# comment\nspring.application.name=billing-service\nitems/mp-rest/url: http://items/api\nkey value\n",
        )]);
        assert_eq!(
            c.get("spring.application.name").as_deref(),
            Some("billing-service")
        );
        assert_eq!(
            c.get("items/mp-rest/url").as_deref(),
            Some("http://items/api")
        );
        assert_eq!(c.get("key").as_deref(), Some("value"));
    }

    #[test]
    fn placeholders_resolve_to_values_then_defaults() {
        let c = config(&[(
            "application.properties",
            "host=items\nbase=http://${host}:8080\n",
        )]);
        assert_eq!(c.resolve("${base}/api"), "http://items:8080/api");
        assert_eq!(c.resolve("${missing:http://x}/y"), "http://x/y");
        assert_eq!(c.resolve("${missing}/y"), "${missing}/y");
        assert_eq!(c.resolve("${missing:${host}}"), "items");
    }

    #[test]
    fn first_source_wins_and_every_value_is_kept() {
        let c = config(&[
            ("application.yml", "spring.application.name: a\n"),
            ("application-dev.yml", "spring.application.name: b\n"),
        ]);
        assert_eq!(c.get("spring.application.name").as_deref(), Some("a"));
        assert_eq!(c.all("spring.application.name"), vec!["a", "b"]);
    }

    #[test]
    fn multi_document_yaml_reads_every_document() {
        let c = config(&[("application.yml", "a: 1\n---\nb: 2\n")]);
        assert_eq!(c.get("a").as_deref(), Some("1"));
        assert_eq!(c.get("b").as_deref(), Some("2"));
    }

    #[test]
    fn configuration_files_are_ranked_by_name() {
        assert_eq!(file_rank("application.yml"), Some(0));
        assert_eq!(file_rank("microprofile-config.properties"), Some(0));
        assert_eq!(file_rank("application-dev.properties"), Some(1));
        assert_eq!(file_rank("bootstrap.yaml"), Some(2));
        assert_eq!(file_rank("applications.yml"), None);
        assert_eq!(file_rank("application.json"), None);
    }

    #[test]
    fn load_skips_test_sources_and_build_output() {
        let root = tempfile::tempdir().expect("tempdir");
        let write = |relative: &str, content: &str| {
            let path = root.path().join(relative);
            std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
            std::fs::write(path, content).expect("write");
        };
        write("src/main/resources/application.yml", "a: main\n");
        write("src/test/resources/application.yml", "a: test\nb: test\n");
        write("target/classes/application.yml", "c: build\n");
        let c = AppConfig::load(root.path());
        assert_eq!(c.all("a"), vec!["main"]);
        assert_eq!(c.get("b"), None);
        assert_eq!(c.get("c"), None);
    }
}
