use std::collections::HashMap;

use crate::cli_tools::{compact_signature, doc_summary};

/// How much of each search result is rendered (issue #50).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Detail {
    /// Every section: file, FQN, signature, docs and relationship summaries.
    #[default]
    Full,
    /// One line per result: name, kind, location and relationship counts.
    Brief,
}

impl Detail {
    /// `"full"` or `"brief"`, case-insensitive; `None` for anything else.
    pub fn parse(value: &str) -> Option<Self> {
        match value.to_ascii_lowercase().as_str() {
            "full" => Some(Self::Full),
            "brief" => Some(Self::Brief),
            _ => None,
        }
    }
}

/// Rendering options of [`format_search_results_with`].
#[derive(Debug, Clone, Copy, Default)]
pub struct SearchFormat {
    pub detail: Detail,
    /// List callers and usages from test code in the samples instead of
    /// only counting them.
    pub include_tests: bool,
}

/// Items of a relationship list (`Calls:`, `Subclasses`, …) shown before
/// the rest is summarized as `+N more`.
const LIST_CAP: usize = 8;

pub fn format_search_results(context: &serde_json::Value) -> String {
    format_search_results_with(context, SearchFormat::default())
}

pub fn format_search_results_with(context: &serde_json::Value, opts: SearchFormat) -> String {
    let mut output = String::from("# Search Results\n\n");
    let entities: Vec<&serde_json::Value> = match context.as_array() {
        Some(entities) => entities.iter().collect(),
        None if context.is_object() => vec![context],
        None => Vec::new(),
    };

    // A file already shown by an earlier result is named by that result
    // instead of repeating its path.
    let mut first_in_file: HashMap<&str, &str> = HashMap::new();
    for entity in &entities {
        if opts.detail == Detail::Brief {
            output.push_str(&format_brief_line(entity));
            continue;
        }
        let file = str_field(entity, "file_path");
        let earlier = file.and_then(|f| first_in_file.get(f).copied());
        output.push_str(&format_entity_with(entity, opts, earlier));
        if let (Some(file), Some(name)) = (file, str_field(entity, "name")) {
            first_in_file.entry(file).or_insert(name);
        }
    }

    if entities.is_empty() {
        output.push_str("No results found.");
    } else if opts.detail == Detail::Brief {
        output.push_str(
            "\nRun `find_callers` / `explore_file` on a result, or repeat the search with `detail: \"full\"`, to expand it.\n",
        );
    }
    output
}

fn str_field<'a>(entity: &'a serde_json::Value, key: &str) -> Option<&'a str> {
    entity.get(key).and_then(|v| v.as_str())
}

fn count_field(entity: &serde_json::Value, key: &str) -> i64 {
    entity.get(key).and_then(|v| v.as_i64()).unwrap_or(0)
}

/// `- \`name\` (kind) — \`file:line\` — called by N, calls M`.
fn format_brief_line(entity: &serde_json::Value) -> String {
    let mut line = format!("- `{}`", str_field(entity, "name").unwrap_or("?"));
    if let Some(kind) = str_field(entity, "kind") {
        line.push_str(&format!(" ({kind})"));
    }
    if let Some(file) = str_field(entity, "file_path") {
        match entity.get("start_line").and_then(|v| v.as_i64()) {
            Some(start) => line.push_str(&format!(" — `{file}:{start}`")),
            None => line.push_str(&format!(" — `{file}`")),
        }
    }
    if let Some(repo) = str_field(entity, "repo_name") {
        line.push_str(&format!(" (repo: {repo})"));
    }
    let calls = entity
        .get("dependencies")
        .and_then(|v| v.as_array())
        .map_or(0, Vec::len) as i64;
    let counts: Vec<String> = [
        ("called by", count_field(entity, "caller_count")),
        ("dispatched", count_field(entity, "dispatched_caller_count")),
        ("referenced", count_field(entity, "type_usage_count")),
        ("calls", calls),
    ]
    .into_iter()
    .filter(|(_, n)| *n > 0)
    .map(|(label, n)| format!("{label} {n}"))
    .collect();
    if !counts.is_empty() {
        line.push_str(" — ");
        line.push_str(&counts.join(", "));
    }
    line.push('\n');
    line
}

#[cfg(test)]
pub(crate) fn format_entity(entity: &serde_json::Value) -> String {
    format_entity_with(entity, SearchFormat::default(), None)
}

/// One full result. `earlier_in_file` names an earlier result declared in
/// the same file, whose `File:` line this one refers to.
fn format_entity_with(
    entity: &serde_json::Value,
    opts: SearchFormat,
    earlier_in_file: Option<&str>,
) -> String {
    let mut output = String::new();

    if let Some(name) = entity.get("name").and_then(|v| v.as_str()) {
        let repo_suffix = entity
            .get("repo_name")
            .and_then(|v| v.as_str())
            .map(|r| format!(" (repo: {r})"))
            .unwrap_or_default();
        if let Some(kind) = entity.get("kind").and_then(|v| v.as_str()) {
            output.push_str(&format!("## `{}` ({}){}\n\n", name, kind, repo_suffix));
        } else {
            output.push_str(&format!("## `{}`{}\n\n", name, repo_suffix));
        }
    }

    let file_path = str_field(entity, "file_path");
    match (file_path, earlier_in_file) {
        (Some(_), Some(earlier)) => {
            output.push_str(&format!("**File:** same as `{earlier}`\n"));
        }
        (Some(file_path), None) => output.push_str(&format!("**File:** `{file_path}`\n")),
        (None, _) => {}
    }

    // Show FQN when it differs from the entity name (e.g., package-qualified Java names)
    if let Some(fqn) = str_field(entity, "fqn") {
        let name = str_field(entity, "name").unwrap_or("");
        if fqn != name && !fqn.is_empty() {
            output.push_str(&format!("**FQN:** `{}`\n", fqn));
        }
    }

    if let Some(signature) = str_field(entity, "signature").and_then(compact_signature) {
        output.push_str(&format!("**Signature:** `{signature}`\n"));
    }
    if let Some(summary) = str_field(entity, "docstring")
        .map(doc_summary)
        .filter(|summary| !summary.is_empty())
    {
        output.push_str(&format!("**Documentation:** {summary}\n"));
    }

    output.push_str(&format_inline_list(
        entity,
        "subclasses",
        "**Subclasses (extends):**",
    ));
    output.push_str(&format_inline_list(
        entity,
        "implementers",
        "**Implementers:**",
    ));

    let summary = SummaryScope {
        entity,
        opts,
        result_file: file_path,
    };
    summary.append("type_usage", "**Type Usage:** Referenced in", &mut output);
    summary.append("caller", "**Called by:**", &mut output);
    // Callers reaching this entity only through dynamic dispatch, kept apart
    // from direct callers exactly like `find_callers` does.
    summary.append("dispatched_caller", "**Called via dispatch:**", &mut output);

    output.push_str(&format_inline_list(entity, "dependencies", "**Calls:**"));

    output.push('\n');
    output
}

/// Formats a non-empty string list on one line, capped at [`LIST_CAP`]
/// items; empty output when the key is absent or empty.
fn format_inline_list(entity: &serde_json::Value, key: &str, title: &str) -> String {
    let items: Vec<&str> = entity
        .get(key)
        .and_then(|v| v.as_array())
        .map(|items| items.iter().filter_map(|item| item.as_str()).collect())
        .unwrap_or_default();
    if items.is_empty() {
        return String::new();
    }
    let shown: Vec<String> = items
        .iter()
        .take(LIST_CAP)
        .map(|name| format!("`{name}`"))
        .collect();
    let mut line = format!("{title} {}", shown.join(", "));
    if items.len() > LIST_CAP {
        line.push_str(&format!(", … +{} more", items.len() - LIST_CAP));
    }
    line.push('\n');
    line
}

/// What a relationship summary is rendered against: its result, the
/// rendering options and the result's own file (for compact samples).
struct SummaryScope<'a> {
    entity: &'a serde_json::Value,
    opts: SearchFormat,
    result_file: Option<&'a str>,
}

impl SummaryScope<'_> {
    /// Appends one line, `"{title} N location(s)[, K in tests]: samples"`,
    /// when `<prefix>_count` is present; nothing otherwise. Test rows stay
    /// in the count, reported as `K in tests`; their samples
    /// (`<prefix>_test_samples`) are listed only with `include_tests`.
    fn append(&self, prefix: &str, title: &str, output: &mut String) {
        let Some(count) = self
            .entity
            .get(format!("{prefix}_count"))
            .and_then(|v| v.as_i64())
        else {
            return;
        };
        let test_count = count_field(self.entity, &format!("{prefix}_test_count"));
        output.push_str(&format!("{title} {count} location(s)"));
        if test_count > 0 && test_count == count {
            output.push_str(", all in tests");
        } else if test_count > 0 {
            output.push_str(&format!(", {test_count} in tests"));
        }

        let mut samples = self.samples(&format!("{prefix}_samples"));
        let mut sampled_from = count - test_count;
        if self.opts.include_tests {
            samples.extend(self.samples(&format!("{prefix}_test_samples")));
            sampled_from = count;
        }
        append_samples(&samples, sampled_from, self.result_file, output);
        output.push('\n');
    }

    fn samples(&self, key: &str) -> Vec<&str> {
        self.entity
            .get(key)
            .and_then(|v| v.as_array())
            .map(|samples| samples.iter().filter_map(|s| s.as_str()).collect())
            .unwrap_or_default()
    }
}

/// Appends `: a, b, c` to a summary line when `samples` is non-empty.
///
/// `total` is the true reference count the samples were drawn from. When the
/// samples are fewer, the list ends with `… +N more`, so a sample can never
/// be mistaken for the complete set.
fn append_samples(samples: &[&str], total: i64, result_file: Option<&str>, output: &mut String) {
    if samples.is_empty() {
        return;
    }
    let shown: Vec<String> = samples
        .iter()
        .map(|sample| compact_sample(sample, result_file))
        .collect();
    output.push_str(&format!(": {}", shown.join(", ")));
    let hidden = total - samples.len() as i64;
    if hidden > 0 {
        output.push_str(&format!(", … +{hidden} more"));
    }
}

/// `name in path` shortened: `name (same file)` in the result's own file,
/// otherwise `name in file.ext`. The full path is one `find_callers` away.
fn compact_sample(sample: &str, result_file: Option<&str>) -> String {
    let Some((name, path)) = sample.split_once(" in ") else {
        return sample.to_string();
    };
    if Some(path) == result_file {
        return format!("{name} (same file)");
    }
    let file = path.rsplit(['/', '\\']).next().unwrap_or(path);
    format!("{name} in {file}")
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_format_search_results_empty_array() {
        let results = json!([]);
        let formatted = format_search_results(&results);
        assert!(formatted.contains("No results found"));
    }

    #[test]
    fn test_format_search_results_empty_string() {
        let results = json!("# Search Results\n\n");
        let formatted = format_search_results(&results);
        assert!(formatted.contains("No results found"));
    }

    #[test]
    fn test_format_entity_with_name_and_kind() {
        let entity = json!({
            "name": "MyClass",
            "kind": "class"
        });
        let formatted = format_entity(&entity);
        assert!(formatted.contains("MyClass"));
        assert!(formatted.contains("class"));
    }

    #[test]
    fn test_format_entity_with_file_path() {
        let entity = json!({
            "name": "MyClass",
            "kind": "class",
            "file_path": "src/main/MyClass.java"
        });
        let formatted = format_entity(&entity);
        assert!(formatted.contains("src/main/MyClass.java"));
        assert!(formatted.contains("**File:**"));
    }

    #[test]
    fn test_format_entity_with_signature() {
        let entity = json!({
            "name": "myMethod",
            "kind": "method",
            "signature": "public void myMethod(String param)"
        });
        let formatted = format_entity(&entity);
        assert!(formatted.contains("public void myMethod(String param)"));
        assert!(formatted.contains("**Signature:**"));
    }

    #[test]
    fn test_format_entity_with_docstring() {
        let entity = json!({
            "name": "MyClass",
            "kind": "class",
            "docstring": "This is a test class\nwith multiple lines"
        });
        let formatted = format_entity(&entity);
        assert!(formatted.contains("This is a test class"));
        assert!(formatted.contains("**Documentation:**"));
    }

    #[test]
    fn test_format_entity_ignores_empty_docstring() {
        let entity = json!({
            "name": "MyClass",
            "kind": "class",
            "docstring": "   \n  "
        });
        let formatted = format_entity(&entity);
        assert!(!formatted.contains("**Documentation:**"));
    }

    #[test]
    fn test_format_entity_with_subclasses() {
        let entity = json!({
            "name": "BaseClass",
            "kind": "class",
            "subclasses": ["ChildClass1", "ChildClass2"]
        });
        let formatted = format_entity(&entity);
        assert!(formatted.contains("**Subclasses (extends):**"));
        assert!(formatted.contains("ChildClass1"));
        assert!(formatted.contains("ChildClass2"));
    }

    #[test]
    fn test_format_entity_ignores_empty_subclasses() {
        let entity = json!({
            "name": "BaseClass",
            "kind": "class",
            "subclasses": []
        });
        let formatted = format_entity(&entity);
        assert!(!formatted.contains("**Subclasses"));
    }

    #[test]
    fn test_format_entity_with_implementers() {
        let entity = json!({
            "name": "MyInterface",
            "kind": "interface",
            "implementers": ["Impl1", "Impl2", "Impl3"]
        });
        let formatted = format_entity(&entity);
        assert!(formatted.contains("**Implementers:**"));
        assert!(formatted.contains("Impl1"));
        assert!(formatted.contains("Impl2"));
        assert!(formatted.contains("Impl3"));
    }

    #[test]
    fn test_format_entity_with_type_usage() {
        let entity = json!({
            "name": "MyClass",
            "kind": "class",
            "type_usage_count": 5,
            "type_usage_samples": ["usage1 in file1.java", "usage2 in file2.java"]
        });
        let formatted = format_entity(&entity);
        assert!(formatted.contains(
            "**Type Usage:** Referenced in 5 location(s): usage1 in file1.java, usage2 in file2.java, … +3 more\n"
        ));
    }

    #[test]
    fn test_format_entity_ignores_empty_usage_samples() {
        let entity = json!({
            "name": "MyClass",
            "kind": "class",
            "type_usage_count": 5,
            "type_usage_samples": []
        });
        let formatted = format_entity(&entity);
        assert!(formatted.contains("**Type Usage:** Referenced in 5 location(s)\n"));
    }

    #[test]
    fn test_format_entity_with_callers() {
        let entity = json!({
            "name": "myMethod",
            "kind": "method",
            "caller_count": 3,
            "caller_samples": ["caller1 in file1.java", "caller2 in file2.java"]
        });
        let formatted = format_entity(&entity);
        // 2 samples out of 3 callers is a truncated sample — the line must
        // say so explicitly (v1.10.0).
        assert!(formatted.contains(
            "**Called by:** 3 location(s): caller1 in file1.java, caller2 in file2.java, … +1 more\n"
        ));
    }

    #[test]
    fn test_format_entity_with_dispatched_callers() {
        let entity = json!({
            "name": "persist",
            "kind": "method",
            "dispatched_caller_count": 1,
            "dispatched_caller_samples": ["archiveViaConcreteType in ReceiptClient.java"]
        });
        let formatted = format_entity(&entity);
        assert!(formatted.contains(
            "**Called via dispatch:** 1 location(s): archiveViaConcreteType in ReceiptClient.java\n"
        ));
        // No direct caller: no "Called by" line at all.
        assert!(!formatted.contains("**Called by:**"));
    }

    #[test]
    fn test_format_entity_caller_sample_covering_everything_has_no_notice() {
        let entity = json!({
            "name": "myMethod",
            "kind": "method",
            "caller_count": 2,
            "caller_samples": ["caller1 in file1.java", "caller2 in file2.java"]
        });
        let formatted = format_entity(&entity);
        assert!(formatted.contains(
            "**Called by:** 2 location(s): caller1 in file1.java, caller2 in file2.java\n"
        ));
        assert!(!formatted.contains("more"));
    }

    #[test]
    fn test_format_entity_many_callers_quantifies_sample() {
        // The bug report scenario: a ubiquitous symbol with 21 callers whose
        // 3-string sample read as if it were the complete set.
        let entity = json!({
            "name": "delete",
            "kind": "method",
            "caller_count": 21,
            "caller_samples": [
                "caller1 in a.java", "caller2 in b.java", "caller3 in c.java"
            ]
        });
        let formatted = format_entity(&entity);
        assert!(formatted.contains(
            "**Called by:** 21 location(s): caller1 in a.java, caller2 in b.java, caller3 in c.java, … +18 more\n"
        ));
    }

    #[test]
    fn test_format_entity_type_usage_quantifies_sample() {
        let entity = json!({
            "name": "MyClass",
            "kind": "class",
            "type_usage_count": 21,
            "type_usage_samples": ["u1 in f1.java"]
        });
        let formatted = format_entity(&entity);
        assert!(formatted.contains("Referenced in 21 location(s): u1 in f1.java, … +20 more\n"));
    }

    /// Issue #50: a sample in the result's own file says so instead of
    /// repeating the path; other samples keep their file name only.
    #[test]
    fn test_samples_are_relative_to_the_result_file() {
        let entity = json!({
            "name": "persist",
            "file_path": "src/store/cart.ts",
            "caller_count": 2,
            "caller_samples": ["reset in src/store/cart.ts", "checkout in src/api/orders.ts"]
        });
        assert!(
            format_entity(&entity).contains(
                "**Called by:** 2 location(s): reset (same file), checkout in orders.ts\n"
            )
        );
    }

    /// Issue #50: test callers stay in the count, are reported as
    /// `N in tests`, and are listed only with `include_tests`.
    #[test]
    fn test_test_callers_are_counted_and_listed_on_demand() {
        let entity = json!({
            "name": "login",
            "file_path": "src/auth.rs",
            "caller_count": 3,
            "caller_samples": ["handle in src/api.rs"],
            "caller_test_count": 2,
            "caller_test_samples": ["test_login in tests/auth.rs", "spec in src/auth.spec.ts"]
        });
        assert!(
            format_entity(&entity)
                .contains("**Called by:** 3 location(s), 2 in tests: handle in api.rs\n")
        );
        let with_tests = SearchFormat {
            include_tests: true,
            ..SearchFormat::default()
        };
        assert!(format_entity_with(&entity, with_tests, None).contains(
            "**Called by:** 3 location(s), 2 in tests: handle in api.rs, test_login in auth.rs, spec in auth.spec.ts\n"
        ));

        let only_tests = json!({"name": "helper", "caller_count": 2, "caller_test_count": 2});
        assert!(
            format_entity(&only_tests).contains("**Called by:** 2 location(s), all in tests\n")
        );
    }

    /// Issue #50: `Calls:` is one line capped at eight items.
    #[test]
    fn test_calls_list_is_capped() {
        let deps: Vec<String> = (1..=11).map(|i| format!("dep{i}")).collect();
        let entity = json!({"name": "run", "dependencies": deps});
        let formatted = format_entity(&entity);
        assert!(formatted.contains("`dep8`, … +3 more\n"));
        assert!(!formatted.contains("dep9"));
    }

    /// Issue #50: empty signatures and `()` print nothing; multi-line ones
    /// print on one line. Docs keep their first paragraph, tags dropped.
    #[test]
    fn test_signature_and_docs_are_compact() {
        let entity = json!({
            "name": "build",
            "signature": "(\n    a: A,\n    b: B,\n)",
            "docstring": "Builds the thing\nin two steps.\n@param a the a"
        });
        let formatted = format_entity(&entity);
        assert!(formatted.contains("**Signature:** `(a: A, b: B)`\n"));
        assert!(formatted.contains("**Documentation:** Builds the thing in two steps.\n"));
        assert!(!formatted.contains("@param"));
        for empty in ["", "()", "  \n "] {
            let entity = json!({"name": "x", "signature": empty});
            assert!(!format_entity(&entity).contains("Signature"), "{empty:?}");
        }
    }

    /// Issue #50: a result in a file shown by an earlier result names it.
    #[test]
    fn test_repeated_file_refers_to_earlier_result() {
        let results = json!([
            {"name": "a", "file_path": "src/x.rs"},
            {"name": "b", "file_path": "src/x.rs"},
            {"name": "c", "file_path": "src/y.rs"}
        ]);
        let formatted = format_search_results(&results);
        assert_eq!(formatted.matches("src/x.rs").count(), 1);
        assert!(formatted.contains("**File:** same as `a`\n"));
        assert!(formatted.contains("**File:** `src/y.rs`\n"));
    }

    /// Issue #50: `brief` renders one line per result.
    #[test]
    fn test_brief_detail_renders_one_line_per_result() {
        let results = json!([{
            "name": "login",
            "kind": "rust_function",
            "file_path": "src/auth.rs",
            "start_line": 12,
            "repo_name": "app",
            "caller_count": 3,
            "dependencies": ["a", "b"],
            "docstring": "Logs in."
        }]);
        let brief = SearchFormat {
            detail: Detail::Brief,
            ..SearchFormat::default()
        };
        let formatted = format_search_results_with(&results, brief);
        assert!(formatted.contains(
            "- `login` (rust_function) — `src/auth.rs:12` (repo: app) — called by 3, calls 2\n"
        ));
        assert!(!formatted.contains("Logs in."));
    }

    #[test]
    fn test_format_entity_with_dependencies() {
        let entity = json!({
            "name": "myMethod",
            "kind": "method",
            "dependencies": ["dep1", "dep2", "dep3"]
        });
        let formatted = format_entity(&entity);
        assert!(formatted.contains("**Calls:**"));
        assert!(formatted.contains("dep1"));
        assert!(formatted.contains("dep2"));
    }

    #[test]
    fn test_format_entity_ignores_empty_dependencies() {
        let entity = json!({
            "name": "myMethod",
            "kind": "method",
            "dependencies": []
        });
        let formatted = format_entity(&entity);
        assert!(!formatted.contains("**Calls:**"));
    }

    #[test]
    fn test_format_search_results_multiple_entities() {
        let results = json!([
            {"name": "Class1", "kind": "class", "file_path": "file1.java"},
            {"name": "Class2", "kind": "class", "file_path": "file2.java"}
        ]);
        let formatted = format_search_results(&results);
        assert!(formatted.contains("Class1"));
        assert!(formatted.contains("Class2"));
        assert!(formatted.contains("file1.java"));
        assert!(formatted.contains("file2.java"));
    }

    #[test]
    fn test_format_entity_without_kind() {
        let entity = json!({
            "name": "UnknownEntity"
        });
        let formatted = format_entity(&entity);
        assert!(formatted.contains("UnknownEntity"));
        // Should not have the kind in parentheses
        assert!(!formatted.contains("UnknownEntity ()"));
    }
}
