use crate::models::{EntityKind, ParsedEntity};
use crate::pipeline::parser::utils::truncate_string;

/// Line-by-line parser for Java .properties files.
/// Supports: key=value, key: value, key value formats.
/// Comments: lines starting with # or ! are skipped.
/// Line continuation: trailing \ joins with the next line.
#[allow(dead_code)] // Reserved for future .properties parsing
pub(crate) fn extract_entities_properties(
    source: &str,
    file_path: &str,
    repo_name: &str,
) -> Vec<ParsedEntity> {
    let mut entities = Vec::new();
    let mut pending_comments: Vec<String> = Vec::new();
    let mut current_line = String::new();

    for raw_line in source.lines() {
        let trimmed = raw_line.trim();

        // Handle line continuation
        if let Some(stripped) = trimmed.strip_suffix('\\') {
            let continuation = stripped.trim_end();
            if !current_line.is_empty() {
                current_line.push(' ');
            }
            current_line.push_str(continuation);
            continue;
        }

        // Complete the current property (may have been built from continuations)
        if !current_line.is_empty() {
            if !trimmed.is_empty() {
                current_line.push(' ');
                current_line.push_str(trimmed);
            }
            process_property_line(
                &current_line,
                &pending_comments,
                file_path,
                repo_name,
                &mut entities,
            );
            current_line.clear();
            pending_comments.clear();
            continue;
        }

        // Skip empty lines
        if trimmed.is_empty() {
            continue;
        }

        // Skip comment lines (but accumulate them for docstring)
        if trimmed.starts_with('#') || trimmed.starts_with('!') {
            pending_comments.push(trimmed.to_string());
            continue;
        }

        // Property line
        process_property_line(
            trimmed,
            &pending_comments,
            file_path,
            repo_name,
            &mut entities,
        );
        pending_comments.clear();
    }

    // Handle any trailing property (no newline at EOF)
    if !current_line.is_empty() {
        process_property_line(
            &current_line,
            &pending_comments,
            file_path,
            repo_name,
            &mut entities,
        );
    }

    entities
}

#[allow(dead_code)] // Reserved for future property line processing
fn process_property_line(
    line: &str,
    comments: &[String],
    file_path: &str,
    repo_name: &str,
    entities: &mut Vec<ParsedEntity>,
) {
    let trimmed = line.trim();
    if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with('!') {
        return;
    }

    // Find first delimiter: =, :, or space (in that order)
    let (key, value) = if let Some(pos) = trimmed.find('=') {
        (trimmed[..pos].trim(), trimmed[pos + 1..].trim())
    } else if let Some(pos) = trimmed.find(':') {
        (trimmed[..pos].trim(), trimmed[pos + 1..].trim())
    } else if let Some(pos) = trimmed.find(' ') {
        (trimmed[..pos].trim(), trimmed[pos + 1..].trim())
    } else {
        // Key only, no value
        (trimmed, "")
    };

    let name = key.rsplit('.').next().unwrap_or(key);
    let fqn = format!("{}:{}:{}", repo_name, file_path, key);
    let signature = truncate_string(value, 200);

    let docstring = if !comments.is_empty() {
        Some(comments.join("\n"))
    } else {
        Some(format!("Config property: {}", key))
    };

    let embed_text = if value.is_empty() {
        key.to_string()
    } else {
        format!("{} = {}", key, value)
    };

    let mut entity = ParsedEntity::new(
        name,
        EntityKind::ConfigProperty,
        fqn,
        Some(signature),
        docstring,
        "properties",
        file_path,
        1,
        1,
        None,
        repo_name,
    );
    entity.embed_text = embed_text;
    entities.push(entity);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_simple_pairs() {
        let source = r#"
key1=value1
key2=value2
key3=value3
"#;
        let entities = extract_entities_properties(source, "app.properties", "test-repo");
        let props: Vec<_> = entities
            .iter()
            .filter(|e| e.kind == EntityKind::ConfigProperty)
            .collect();
        assert_eq!(props.len(), 3);
        assert!(props.iter().any(|p| p.name == "key1"));
        assert!(props.iter().any(|p| p.name == "key2"));
        assert!(props.iter().any(|p| p.name == "key3"));
    }

    #[test]
    fn test_parse_colon_delimiter() {
        let source = r#"
key1: value1
key2: value2
"#;
        let entities = extract_entities_properties(source, "app.properties", "test-repo");
        let props: Vec<_> = entities
            .iter()
            .filter(|e| e.kind == EntityKind::ConfigProperty)
            .collect();
        assert_eq!(props.len(), 2);
        let e1 = props.iter().find(|p| p.name == "key1").unwrap();
        assert_eq!(e1.signature.as_ref().unwrap(), "value1");
    }

    #[test]
    fn test_parse_comments() {
        let source = r#"
# This is a comment
! This is also a comment
key1=value1
# Another comment
key2=value2
"#;
        let entities = extract_entities_properties(source, "app.properties", "test-repo");
        let props: Vec<_> = entities
            .iter()
            .filter(|e| e.kind == EntityKind::ConfigProperty)
            .collect();
        assert_eq!(props.len(), 2);
    }

    #[test]
    fn test_parse_comment_as_docstring() {
        let source = r#"
# Database configuration
# Connection settings
spring.datasource.url=jdbc:mysql://localhost:3306/db
"#;
        let entities = extract_entities_properties(source, "app.properties", "test-repo");
        let props: Vec<_> = entities
            .iter()
            .filter(|e| e.kind == EntityKind::ConfigProperty)
            .collect();
        assert_eq!(props.len(), 1);
        let doc = props[0].docstring.as_ref().unwrap();
        assert!(doc.contains("Database configuration"));
        assert!(doc.contains("Connection settings"));
        assert_eq!(props[0].name, "url");
    }

    #[test]
    fn test_parse_multiline_values() {
        let source = r#"
spring.datasource.url=jdbc:mysql://localhost:3306/db \
    ?useSSL=false&serverTimezone=UTC
simple=value
"#;
        let entities = extract_entities_properties(source, "app.properties", "test-repo");
        let props: Vec<_> = entities
            .iter()
            .filter(|e| e.kind == EntityKind::ConfigProperty)
            .collect();
        assert_eq!(props.len(), 2);
        let url_entity = props.iter().find(|p| p.name == "url").unwrap();
        assert!(
            url_entity
                .signature
                .as_ref()
                .unwrap()
                .contains("useSSL=false")
        );
        assert!(
            url_entity
                .signature
                .as_ref()
                .unwrap()
                .contains("serverTimezone=UTC")
        );
    }

    #[test]
    fn test_parse_empty_file() {
        let source = "";
        let entities = extract_entities_properties(source, "empty.properties", "test-repo");
        assert!(entities.is_empty());
    }
}
