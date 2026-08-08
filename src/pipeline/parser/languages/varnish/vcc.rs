use crate::models::{EntityKind, ParsedEntity};

#[expect(
    clippy::too_many_lines,
    reason = "function is verbose but correct — extraction deferred"
)]
#[expect(
    clippy::cognitive_complexity,
    reason = "function is verbose but correct — extraction deferred"
)]
pub(crate) fn extract_entities_vcc(
    source: &str,
    file_path: &str,
    repo_name: &str,
) -> Vec<ParsedEntity> {
    let mut entities = Vec::new();
    let mut current_module: Option<String> = None;
    let mut current_object: Option<String> = None;
    let mut pending_prose: Vec<String> = Vec::new();
    let mut current_object_fqn: Option<String> = None;

    for (line_num, raw_line) in source.lines().enumerate() {
        let line = raw_line.trim();
        let start_line = line_num + 1;

        // Skip RST prose (non-$ lines)
        if !line.starts_with('$') {
            if !line.is_empty() && current_module.is_some() {
                pending_prose.push(raw_line.to_string());
            }
            continue;
        }

        let directive = line.to_string();
        let prose = if pending_prose.is_empty() {
            None
        } else {
            let doc = pending_prose.join("\n");
            pending_prose.clear();
            if doc.trim().is_empty() {
                None
            } else {
                Some(doc)
            }
        };

        if directive.starts_with("$Module ") {
            current_object = None;
            current_object_fqn = None;
            let (name, section, desc) = parse_module_directive(&directive);
            let module_name = name.clone();
            current_module = Some(module_name.clone());

            let entity = ParsedEntity::new(
                module_name.clone(),
                EntityKind::VccModule,
                format!("vcc:{}", module_name),
                Some(format!(
                    "$Module {} {} {}",
                    name,
                    section.unwrap_or_default(),
                    desc.unwrap_or_default()
                )),
                prose,
                "vcc",
                file_path,
                start_line,
                start_line,
                None,
                repo_name,
            );
            entities.push(entity);
        } else if directive.starts_with("$Function ") {
            if let Some(module) = &current_module {
                let (ret_type, func_name, params) = parse_function_directive(&directive);
                // Exclude PRIV_* params from the signature
                let vcl_params: Vec<String> = params
                    .iter()
                    .filter(|p| {
                        !p.starts_with("PRIV_") && !p.starts_with("priv_") && !p.contains("PRIV_")
                    })
                    .cloned()
                    .collect();

                let sig = if vcl_params.is_empty() {
                    format!("$Function {} {}()", ret_type, func_name)
                } else {
                    format!(
                        "$Function {} {}({})",
                        ret_type,
                        func_name,
                        vcl_params.join(", ")
                    )
                };

                let entity = ParsedEntity::new(
                    func_name.clone(),
                    EntityKind::VccFunction,
                    format!("vcc:{}::{}", module, func_name),
                    Some(sig),
                    prose,
                    "vcc",
                    file_path,
                    start_line,
                    start_line,
                    None,
                    repo_name,
                );
                entities.push(entity);
                current_object = None;
                current_object_fqn = None;
            }
        } else if directive.starts_with("$Object ") {
            if let Some(module) = &current_module {
                let (obj_name, ctor_params) = parse_object_directive(&directive);
                let sig = if ctor_params.is_empty() {
                    format!("$Object {}()", obj_name)
                } else {
                    format!("$Object {}({})", obj_name, ctor_params.join(", "))
                };

                let fqn = format!("vcc:{}::{}", module, obj_name);
                current_object = Some(obj_name.clone());
                current_object_fqn = Some(fqn.clone());

                let entity = ParsedEntity::new(
                    obj_name,
                    EntityKind::VccObject,
                    fqn,
                    Some(sig),
                    prose,
                    "vcc",
                    file_path,
                    start_line,
                    start_line,
                    None,
                    repo_name,
                );
                entities.push(entity);
            }
        } else if directive.starts_with("$Method ") {
            // gotcha 14: positional binding to preceding $Object
            if let Some(_obj_name) = &current_object
                && let Some(_module) = &current_module
                && let Some(obj_fqn) = &current_object_fqn
            {
                let (ret_type, method_name, params) = parse_function_directive(&directive);
                // Strip leading '.' from method name
                let clean_name = method_name.strip_prefix('.').unwrap_or(&method_name);

                let vcl_params: Vec<String> = params
                    .iter()
                    .filter(|p| {
                        !p.starts_with("PRIV_") && !p.starts_with("priv_") && !p.contains("PRIV_")
                    })
                    .cloned()
                    .collect();

                let sig = if vcl_params.is_empty() {
                    format!("$Method {} .{}()", ret_type, clean_name)
                } else {
                    format!(
                        "$Method {} .{}({})",
                        ret_type,
                        clean_name,
                        vcl_params.join(", ")
                    )
                };

                let fqn = format!("{}::{}", obj_fqn, clean_name);

                let entity = ParsedEntity::new(
                    clean_name.to_string(),
                    EntityKind::VccMethod,
                    fqn,
                    Some(sig),
                    prose,
                    "vcc",
                    file_path,
                    start_line,
                    start_line,
                    None,
                    repo_name,
                );
                entities.push(entity);
            }
            // $Method before any $Object is malformed — skip
        } else {
            // $ABI, $Event, $Restrict, $Alias, $Synopsis, $Prefix — not entities
            current_object = None;
            current_object_fqn = None;
            if !directive.starts_with("$ABI")
                && !directive.starts_with("$Event")
                && !directive.starts_with("$Restrict")
                && !directive.starts_with("$Alias")
                && !directive.starts_with("$Synopsis")
                && !directive.starts_with("$Prefix")
            {
                // Unknown directive — skip
            }
        }
    }

    entities
}

fn parse_module_directive(line: &str) -> (String, Option<String>, Option<String>) {
    // $Module <name> <section> "<desc>"
    let rest = line.strip_prefix("$Module ").unwrap_or(line);
    let parts = split_quoted(rest);
    let name = parts.first().map(|s| s.to_string()).unwrap_or_default();
    let section = parts.get(1).map(|s| s.to_string());
    let desc = parts.get(2).map(|s| s.to_string());
    (name, section, desc)
}

fn parse_function_directive(line: &str) -> (String, String, Vec<String>) {
    // $Function <rettype> <name>(<params>)
    let rest = line
        .strip_prefix("$Function ")
        .or_else(|| line.strip_prefix("$Method "))
        .unwrap_or(line);
    let rest = rest.trim();

    // Find ret_type (first word)
    let (ret_type, rest) = rest.split_once(' ').unwrap_or((rest, ""));
    let rest = rest.trim();

    // Find name (up to opening paren)
    if let Some(paren) = rest.find('(') {
        let name = rest[..paren].trim().to_string();
        let params_str = rest[paren + 1..].trim();
        let params_str = params_str.strip_suffix(')').unwrap_or(params_str);
        let params = if params_str.is_empty() {
            Vec::new()
        } else {
            params_str
                .split(',')
                .map(|p| p.trim().to_string())
                .collect()
        };
        (ret_type.to_string(), name, params)
    } else {
        (ret_type.to_string(), rest.to_string(), Vec::new())
    }
}

fn parse_object_directive(line: &str) -> (String, Vec<String>) {
    // $Object <name>(<ctor-params>)
    let rest = line.strip_prefix("$Object ").unwrap_or(line);
    let rest = rest.trim();

    if let Some(paren) = rest.find('(') {
        let name = rest[..paren].trim().to_string();
        let params_str = rest[paren + 1..].trim();
        let params_str = params_str.strip_suffix(')').unwrap_or(params_str);
        let params = if params_str.is_empty() {
            Vec::new()
        } else {
            params_str
                .split(',')
                .map(|p| p.trim().to_string())
                .collect()
        };
        (name, params)
    } else {
        (rest.to_string(), Vec::new())
    }
}

/// Split on spaces but respect double-quoted arguments.
fn split_quoted(s: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;

    for ch in s.chars() {
        if ch == '"' {
            in_quotes = !in_quotes;
            current.push(ch);
        } else if ch == ' ' && !in_quotes {
            if !current.is_empty() {
                parts.push(current.clone());
                current.clear();
            }
        } else {
            current.push(ch);
        }
    }
    if !current.is_empty() {
        parts.push(current);
    }
    parts
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_vcc_module() {
        let src = r#"$Module cookie 3 "Varnish Cookie Module"
"#;
        let entities = extract_entities_vcc(src, "vmod_cookie.vcc", "test-repo");
        let modules: Vec<_> = entities
            .iter()
            .filter(|e| e.kind == EntityKind::VccModule)
            .collect();
        assert_eq!(modules.len(), 1);
        assert_eq!(modules[0].name, "cookie");
        assert_eq!(modules[0].fqn, "vcc:cookie");
    }

    #[test]
    fn test_extract_vcc_function() {
        let src = r#"$Module cookie 3 ""
$Function VOID parse(STRING cookieheader)
"#;
        let entities = extract_entities_vcc(src, "vmod_cookie.vcc", "test-repo");
        let funcs: Vec<_> = entities
            .iter()
            .filter(|e| e.kind == EntityKind::VccFunction)
            .collect();
        assert_eq!(funcs.len(), 1);
        assert_eq!(funcs[0].name, "parse");
        assert_eq!(funcs[0].fqn, "vcc:cookie::parse");
    }

    #[test]
    fn test_extract_vcc_function_with_rst_prose() {
        let src = r#"$Module cookie 3 ""

DESCRIPTION
===========

This VMOD parses cookies.

$Function VOID parse(STRING cookieheader)

Description of the parse function.

$Function STRING get(STRING cookiename)
"#;
        let entities = extract_entities_vcc(src, "vmod_cookie.vcc", "test-repo");
        let funcs: Vec<_> = entities
            .iter()
            .filter(|e| e.kind == EntityKind::VccFunction)
            .collect();
        assert_eq!(funcs.len(), 2);
        // First function has docstring
        let parse_fn = &funcs[0];
        assert!(parse_fn.docstring.is_some());
        assert!(
            parse_fn
                .docstring
                .as_ref()
                .unwrap()
                .contains("VMOD parses cookies")
        );
    }

    #[test]
    fn test_extract_vcc_object_and_method() {
        let src = r#"$Module cookie 3 ""
$Object counter(STRING name, INT initial = 0)
$Method VOID .incr(INT n = 1)
$Method INT .get()
"#;
        let entities = extract_entities_vcc(src, "vmod_cookie.vcc", "test-repo");
        let objects: Vec<_> = entities
            .iter()
            .filter(|e| e.kind == EntityKind::VccObject)
            .collect();
        assert_eq!(objects.len(), 1);
        assert_eq!(objects[0].name, "counter");
        assert_eq!(objects[0].fqn, "vcc:cookie::counter");

        let methods: Vec<_> = entities
            .iter()
            .filter(|e| e.kind == EntityKind::VccMethod)
            .collect();
        assert_eq!(methods.len(), 2);
        assert_eq!(methods[0].name, "incr");
        assert_eq!(methods[0].fqn, "vcc:cookie::counter::incr");
        assert_eq!(methods[1].name, "get");
        assert_eq!(methods[1].fqn, "vcc:cookie::counter::get");
    }

    #[test]
    fn test_extract_vcc_priv_params_excluded() {
        let src = r#"$Module test 1 ""
$Function VOID proc(STRING s, PRIV_TASK, INT n)
"#;
        let entities = extract_entities_vcc(src, "test.vcc", "test-repo");
        let funcs: Vec<_> = entities
            .iter()
            .filter(|e| e.kind == EntityKind::VccFunction)
            .collect();
        assert_eq!(funcs.len(), 1);
        let sig = funcs[0].signature.as_ref().unwrap();
        // PRIV_TASK should NOT appear in the signature
        assert!(!sig.contains("PRIV_TASK"));
        assert!(sig.contains("STRING s"));
        assert!(sig.contains("INT n"));
    }

    #[test]
    fn test_extract_vcc_enum_params() {
        let src = r#"$Module test 1 ""
$Function STRING pick(ENUM {one, two, three} e)
"#;
        let entities = extract_entities_vcc(src, "test.vcc", "test-repo");
        let funcs: Vec<_> = entities
            .iter()
            .filter(|e| e.kind == EntityKind::VccFunction)
            .collect();
        assert_eq!(funcs.len(), 1);
        let sig = funcs[0].signature.as_ref().unwrap();
        assert!(sig.contains("ENUM"));
    }

    #[test]
    fn test_extract_vcc_abi_directive_skipped() {
        let src = r#"$Module cookie 3 ""
$ABI strict
$Function VOID parse(STRING s)
"#;
        let entities = extract_entities_vcc(src, "test.vcc", "test-repo");
        // $ABI should not produce an entity
        let funcs: Vec<_> = entities
            .iter()
            .filter(|e| e.kind == EntityKind::VccFunction)
            .collect();
        assert_eq!(funcs.len(), 1);
    }

    #[test]
    fn test_extract_vcc_empty_source() {
        let entities = extract_entities_vcc("", "test.vcc", "test-repo");
        assert!(entities.is_empty());
    }
}
