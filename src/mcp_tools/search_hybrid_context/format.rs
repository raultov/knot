use serde_json::json;

pub(crate) fn format_search_results(context: &serde_json::Value) -> String {
    let mut output = String::from("# Search Results\n\n");

    if let Some(entities) = context.as_array() {
        for entity in entities {
            output.push_str(&format_entity(entity));
        }
    } else if let Some(obj) = context.as_object() {
        output.push_str(&format_entity(&json!(obj)));
    }

    if output.is_empty() || output == "# Search Results\n\n" {
        output.push_str("No results found.");
    }

    output
}

pub(crate) fn format_entity(entity: &serde_json::Value) -> String {
    let mut output = String::new();

    if let Some(name) = entity.get("name").and_then(|v| v.as_str()) {
        if let Some(kind) = entity.get("kind").and_then(|v| v.as_str()) {
            output.push_str(&format!("## `{}` ({}) \n\n", name, kind));
        } else {
            output.push_str(&format!("## `{}`\n\n", name));
        }
    }

    if let Some(file_path) = entity.get("file_path").and_then(|v| v.as_str()) {
        output.push_str(&format!("**File:** `{}`\n\n", file_path));
    }

    if let Some(signature) = entity.get("signature").and_then(|v| v.as_str()) {
        output.push_str(&format!("**Signature:**\n```\n{}\n```\n\n", signature));
    }

    if let Some(docstring) = entity
        .get("docstring")
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
    {
        output.push_str(&format!("**Documentation:**\n{}\n\n", docstring));
    }

    // Show subclasses
    if let Some(subclasses) = entity
        .get("subclasses")
        .and_then(|v| v.as_array())
        .filter(|d| !d.is_empty())
    {
        output.push_str("**Subclasses (extends):**\n");
        for subclass in subclasses {
            if let Some(name) = subclass.as_str() {
                output.push_str(&format!("- `{}`\n", name));
            }
        }
        output.push('\n');
    }

    // Show implementers
    if let Some(implementers) = entity
        .get("implementers")
        .and_then(|v| v.as_array())
        .filter(|d| !d.is_empty())
    {
        output.push_str("**Implementers:**\n");
        for impl_class in implementers {
            if let Some(name) = impl_class.as_str() {
                output.push_str(&format!("- `{}`\n", name));
            }
        }
        output.push('\n');
    }

    // Show type usage summary
    if let Some(count) = entity.get("type_usage_count").and_then(|v| v.as_i64()) {
        output.push_str(&format!(
            "**Type Usage:** Referenced in {} location(s)\n",
            count
        ));
        if let Some(samples) = entity
            .get("type_usage_samples")
            .and_then(|v| v.as_array())
            .filter(|s| !s.is_empty())
        {
            output.push_str("Sample usages:\n");
            for sample in samples {
                if let Some(s) = sample.as_str() {
                    output.push_str(&format!("- {}\n", s));
                }
            }
        }
        output.push('\n');
    }

    // Show callers summary
    if let Some(count) = entity.get("caller_count").and_then(|v| v.as_i64()) {
        output.push_str(&format!("**Called by:** {} location(s)\n", count));
        if let Some(samples) = entity
            .get("caller_samples")
            .and_then(|v| v.as_array())
            .filter(|s| !s.is_empty())
        {
            output.push_str("Sample callers:\n");
            for sample in samples {
                if let Some(s) = sample.as_str() {
                    output.push_str(&format!("- {}\n", s));
                }
            }
        }
        output.push('\n');
    }

    if let Some(deps) = entity
        .get("dependencies")
        .and_then(|v| v.as_array())
        .filter(|d| !d.is_empty())
    {
        output.push_str("**Calls:**\n");
        for dep in deps {
            if let Some(dep_name) = dep.as_str() {
                output.push_str(&format!("- `{}`\n", dep_name));
            }
        }
        output.push('\n');
    }

    output
}
