/// The package and imports of one JVM source file (Java, Kotlin). Call and
/// type resolution use them to tell same-named types of different packages
/// apart (issue #45).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ImportScope {
    /// The file's `package`, if any.
    pub package: Option<String>,
    /// Single-type imports: local name (the simple name, or a Kotlin `as`
    /// alias) → imported FQN.
    pub imports: Vec<(String, String)>,
    /// Packages imported on demand (`import com.x.*`).
    pub wildcards: Vec<String>,
}

impl ImportScope {
    /// The FQN a single-type import binds `local_name` to.
    pub fn imported(&self, local_name: &str) -> Option<&str> {
        self.imports
            .iter()
            .find(|(local, _)| local == local_name)
            .map(|(_, fqn)| fqn.as_str())
    }

    /// The FQNs an unqualified type `name` may denote without a single-type
    /// import, in precedence order: the file's own package, then each
    /// on-demand import.
    pub fn implicit_candidates<'a>(&'a self, name: &'a str) -> impl Iterator<Item = String> + 'a {
        let package = match &self.package {
            Some(package) => format!("{package}.{name}"),
            None => name.to_string(),
        };
        std::iter::once(package).chain(self.wildcards.iter().map(move |w| format!("{w}.{name}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_import_scope_precedence() {
        let scope = ImportScope {
            package: Some("com.app".to_string()),
            imports: vec![("BConfig".to_string(), "com.b.Config".to_string())],
            wildcards: vec!["com.c".to_string()],
        };
        assert_eq!(scope.imported("BConfig"), Some("com.b.Config"));
        assert_eq!(scope.imported("Config"), None);
        assert_eq!(
            scope.implicit_candidates("Config").collect::<Vec<_>>(),
            vec!["com.app.Config", "com.c.Config"]
        );
    }
}
