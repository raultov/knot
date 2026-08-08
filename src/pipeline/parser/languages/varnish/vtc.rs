use super::vcl::extract_entities_vcl_with_offset;
use crate::models::{EntityKind, ParsedEntity, ReferenceIntent};

#[expect(
    clippy::too_many_lines,
    reason = "function is verbose but correct — extraction deferred"
)]
#[expect(
    clippy::cognitive_complexity,
    reason = "function is verbose but correct — extraction deferred"
)]
pub(crate) fn extract_entities_vtc(
    source: &str,
    file_path: &str,
    repo_name: &str,
) -> Vec<ParsedEntity> {
    let mut entities = Vec::new();
    let lines: Vec<&str> = source.lines().collect();
    let mut server_names: Vec<(String, usize)> = Vec::new();
    let mut varnish_names: Vec<(String, usize)> = Vec::new();

    // Pre-pass: collect server/varnish instance names for cross-referencing
    for (i, line) in lines.iter().enumerate() {
        let trimmed = line.trim();
        if trimmed.starts_with("server ") {
            if let Some(name) = parse_instance_name(trimmed, "server ") {
                server_names.push((name, i + 1));
            }
        } else if trimmed.starts_with("varnish ")
            && let Some(name) = parse_instance_name(trimmed, "varnish ")
        {
            varnish_names.push((name, i + 1));
        }
    }

    // Pass: parse top-level commands
    let mut i = 0;
    while i < lines.len() {
        let trimmed = lines[i].trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            i += 1;
            continue;
        }

        match () {
            _ if trimmed.starts_with("varnishtest ") || trimmed.starts_with("vtest ") => {
                let desc = parse_quoted_or_rest(trimmed);
                let mut entity = ParsedEntity::new(
                    desc.clone(),
                    EntityKind::VtcTestCase,
                    format!("vtc:{}:{}:testcase", repo_name, file_path),
                    Some(format!("varnishtest \"{}\"", desc)),
                    None,
                    "vtc",
                    file_path,
                    i + 1,
                    i + 1,
                    None,
                    repo_name,
                );
                entity.is_test_context = true;
                entities.push(entity);
                i += 1;
            }
            _ if trimmed.starts_with("server ") => {
                if let Some(name) = parse_instance_name(trimmed, "server ") {
                    let start_line = i + 1;
                    let (body, next_i) = extract_brace_block(&lines, i, trimmed);
                    let mut refs: Vec<ReferenceIntent> = Vec::new();
                    // Build refs for VTC entity names
                    for (sname, sline) in &server_names {
                        if sname.as_str() == "s1" {
                            // Skip self-reference
                            continue;
                        }
                        if body.contains(&format!("-connect ${{{}}}", sname))
                            || body.contains(&format!("${{{}}}", sname))
                        {
                            refs.push(ReferenceIntent::ValueReference {
                                value_name: format!("vtc:server:{}", sname),
                                line: *sline,
                            });
                        }
                    }
                    let mut entity = ParsedEntity::new(
                        name.clone(),
                        EntityKind::VtcServer,
                        format!("vtc:{}:{}:{}", repo_name, file_path, name),
                        Some(format!("server {} {{ ... }}", name)),
                        None,
                        "vtc",
                        file_path,
                        start_line,
                        start_line,
                        None,
                        repo_name,
                    );
                    entity.is_test_context = true;
                    entity.reference_intents = refs;
                    entities.push(entity);
                    i = next_i;
                } else {
                    i += 1;
                }
            }
            _ if trimmed.starts_with("client ") => {
                if let Some(name) = parse_instance_name(trimmed, "client ") {
                    let start_line = i + 1;
                    let (_, next_i) = extract_brace_block(&lines, i, trimmed);
                    let entity = ParsedEntity::new(
                        name.clone(),
                        EntityKind::VtcClient,
                        format!("vtc:{}:{}:{}", repo_name, file_path, name),
                        Some(format!("client {} {{ ... }}", name)),
                        None,
                        "vtc",
                        file_path,
                        start_line,
                        start_line,
                        None,
                        repo_name,
                    );
                    let mut entity = entity;
                    entity.is_test_context = true;
                    entities.push(entity);
                    i = next_i;
                } else {
                    i += 1;
                }
            }
            _ if trimmed.starts_with("varnish ") => {
                if let Some(name) = parse_instance_name(trimmed, "varnish ") {
                    let start_line = i + 1;
                    i = parse_varnish_command(
                        &lines,
                        i,
                        trimmed,
                        &name,
                        file_path,
                        repo_name,
                        &server_names,
                        &mut entities,
                    );
                    i = i.max(start_line);
                } else {
                    i += 1;
                }
            }
            _ if trimmed.starts_with("logexpect ") => {
                if let Some(name) = parse_instance_name(trimmed, "logexpect ") {
                    let start_line = i + 1;
                    let (_, next_i) = extract_brace_block(&lines, i, trimmed);
                    let entity = ParsedEntity::new(
                        name.clone(),
                        EntityKind::VtcLogexpect,
                        format!("vtc:{}:{}:{}", repo_name, file_path, name),
                        Some(format!("logexpect {} {{ ... }}", name)),
                        None,
                        "vtc",
                        file_path,
                        start_line,
                        start_line,
                        None,
                        repo_name,
                    );
                    let mut entity = entity;
                    entity.is_test_context = true;
                    entities.push(entity);
                    i = next_i;
                } else {
                    i += 1;
                }
            }
            _ if trimmed.starts_with("barrier ") => {
                if let Some(name) = parse_instance_name(trimmed, "barrier ") {
                    let start_line = i + 1;
                    // barrier may or may not have a block
                    let entity = ParsedEntity::new(
                        name.clone(),
                        EntityKind::VtcBarrier,
                        format!("vtc:{}:{}:{}", repo_name, file_path, name),
                        Some(format!("barrier {} ...", name)),
                        None,
                        "vtc",
                        file_path,
                        start_line,
                        start_line,
                        None,
                        repo_name,
                    );
                    let mut entity = entity;
                    entity.is_test_context = true;
                    entities.push(entity);
                }
                i += 1;
            }
            _ => {
                // Unknown command — skip
                i += 1;
            }
        }
    }

    entities
}

fn parse_instance_name(line: &str, prefix: &str) -> Option<String> {
    let rest = line.strip_prefix(prefix)?;
    let rest = rest.trim();
    rest.split_whitespace().next().map(|s| s.to_string())
}

fn parse_quoted_or_rest(line: &str) -> String {
    let rest = line.split_once(' ').map(|(_, r)| r).unwrap_or("");
    let rest = rest.trim();
    if rest.starts_with('"') {
        let inside = rest.strip_prefix('"').unwrap_or(rest);
        if let Some(end) = inside.rfind('"') {
            inside[..end].to_string()
        } else {
            inside.to_string()
        }
    } else {
        rest.to_string()
    }
}

/// Extract a brace-delimited block, handling embedded VCL `{"...}` long strings.
#[expect(
    clippy::cognitive_complexity,
    reason = "function is verbose but correct — extraction deferred"
)]
fn extract_brace_block(lines: &[&str], start: usize, first_line: &str) -> (String, usize) {
    let mut body = String::new();
    let mut i = start;
    let mut depth = 0;
    let mut started = false;
    let mut in_string = false;

    // Pass 1: check first line
    let mut first_line_content = String::new();
    for ch in first_line.chars() {
        if ch == '"' {
            in_string = !in_string;
        }
        if !in_string && ch == '{' {
            depth += 1;
            started = true;
            if depth == 1 {
                continue;
            }
        }
        if !in_string && ch == '}' && depth > 0 {
            depth -= 1;
            if depth == 0 {
                break;
            }
        }
        if started && depth > 0 {
            first_line_content.push(ch);
        }
    }

    if started {
        if !first_line_content.trim().is_empty() {
            body.push_str(&first_line_content);
            body.push('\n');
        }
        i += 1;

        while i < lines.len() && depth > 0 {
            let line = lines[i];
            let mut line_content = String::new();
            let mut in_str = false;
            for ch in line.chars() {
                if ch == '"' {
                    in_str = !in_str;
                }
                if !in_str && ch == '{' {
                    depth += 1;
                }
                if !in_str && ch == '}' && depth > 0 {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                if depth > 0 {
                    line_content.push(ch);
                }
            }
            if depth > 0 || !line_content.trim().is_empty() {
                body.push_str(&line_content);
                body.push('\n');
            }
            i += 1;
        }
        (body, i)
    } else {
        // Did not start on first line, check if -vcl+backend flag is present and next line starts block
        if first_line.contains("-vcl+backend") && i + 1 < lines.len() {
            let trimmed_next = lines[i + 1].trim();
            if trimmed_next.starts_with('{') {
                return extract_brace_block(lines, i + 1, lines[i + 1]);
            }
        }
        // Otherwise no block
        (String::new(), i + 1)
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "VTC command parsing requires line buffer, position, and context parameters"
)]
fn parse_varnish_command(
    lines: &[&str],
    i: usize,
    first_line: &str,
    name: &str,
    file_path: &str,
    repo_name: &str,
    server_names: &[(String, usize)],
    entities: &mut Vec<ParsedEntity>,
) -> usize {
    let _ = repo_name;
    let start_line = i + 1;
    let has_errvcl = first_line.contains("-errvcl");
    let has_vcl_plus_backend = first_line.contains("-vcl+backend");

    let (vcl_body, next_i) = extract_brace_block(lines, i, first_line);

    if !has_errvcl && !vcl_body.is_empty() {
        let mut vcl_entities =
            extract_entities_vcl_with_offset(&vcl_body, file_path, repo_name, start_line);
        for mut e in vcl_entities.drain(..) {
            e.is_test_context = true;
            entities.push(e);
        }

        if has_vcl_plus_backend {
            for (sname, sline) in server_names {
                let mut be = ParsedEntity::new(
                    sname.clone(),
                    EntityKind::VclBackend,
                    format!("vcl:{}:{}:{}", repo_name, file_path, sname),
                    Some(format!(
                        "backend {} {{ .host = \"${{{}_addr}}\"; .port = \"${{{}_port}}\"; }}",
                        sname, sname, sname
                    )),
                    Some(String::from("Synthesised by -vcl+backend")),
                    "vcl",
                    file_path,
                    *sline,
                    *sline,
                    None,
                    repo_name,
                );
                be.is_test_context = true;
                be.reference_intents.push(ReferenceIntent::ValueReference {
                    value_name: format!("vtc:server:{}", sname),
                    line: *sline,
                });
                entities.push(be);
            }
        }
    }

    let mut entity = ParsedEntity::new(
        name.to_string(),
        EntityKind::VtcVarnishInstance,
        format!("vtc:{}:{}:{}", repo_name, file_path, name),
        Some(format!("varnish {} {{ ... }}", name)),
        None,
        "vtc",
        file_path,
        start_line,
        start_line,
        None,
        repo_name,
    );
    entity.is_test_context = true;
    entities.push(entity);

    next_i
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_vtc_test_case() {
        let src = r#"varnishtest "Basic test"

server s1 {
    rxreq
    txresp -body "ok"
} -start"#;
        let entities = extract_entities_vtc(src, "test.vtc", "test-repo");
        let testcases: Vec<_> = entities
            .iter()
            .filter(|e| e.kind == EntityKind::VtcTestCase)
            .collect();
        assert_eq!(testcases.len(), 1);
        assert_eq!(testcases[0].name, "Basic test");
    }

    #[test]
    fn test_extract_vtc_vtest() {
        // vtest is equivalent to varnishtest
        let entities = extract_entities_vtc(
            r#"vtest "my test"

server s1 { } -start"#,
            "test.vtc",
            "test-repo",
        );
        let testcases: Vec<_> = entities
            .iter()
            .filter(|e| e.kind == EntityKind::VtcTestCase)
            .collect();
        assert_eq!(testcases.len(), 1);
        assert_eq!(testcases[0].name, "my test");
    }

    #[test]
    fn test_extract_vtc_server() {
        let src = r#"varnishtest "test"
server s1 {
    rxreq
    txresp
} -start"#;
        let entities = extract_entities_vtc(src, "test.vtc", "test-repo");
        let servers: Vec<_> = entities
            .iter()
            .filter(|e| e.kind == EntityKind::VtcServer)
            .collect();
        assert_eq!(servers.len(), 1);
        assert_eq!(servers[0].name, "s1");
        assert!(servers[0].is_test_context);
    }

    #[test]
    fn test_extract_vtc_client() {
        let src = r#"varnishtest "test"
client c1 {
    txreq -url "/"
    rxresp
} -run"#;
        let entities = extract_entities_vtc(src, "test.vtc", "test-repo");
        let clients: Vec<_> = entities
            .iter()
            .filter(|e| e.kind == EntityKind::VtcClient)
            .collect();
        assert_eq!(clients.len(), 1);
        assert_eq!(clients[0].name, "c1");
    }

    #[test]
    fn test_extract_vtc_varnish_instance() {
        let src = r#"varnishtest "test"
varnish v1 -vcl+backend {
    sub vcl_recv { return (hash); }
} -start"#;
        let entities = extract_entities_vtc(src, "test.vtc", "test-repo");
        let instances: Vec<_> = entities
            .iter()
            .filter(|e| e.kind == EntityKind::VtcVarnishInstance)
            .collect();
        assert_eq!(instances.len(), 1);
        assert_eq!(instances[0].name, "v1");
    }

    #[test]
    fn test_extract_vtc_logexpect() {
        let src = r#"varnishtest "test"
logexpect l1 -v v1 {
    expect * = ReqURL "^/foo$"
    expect * = End
} -run"#;
        let entities = extract_entities_vtc(src, "test.vtc", "test-repo");
        let logexpects: Vec<_> = entities
            .iter()
            .filter(|e| e.kind == EntityKind::VtcLogexpect)
            .collect();
        assert_eq!(logexpects.len(), 1);
        assert_eq!(logexpects[0].name, "l1");
    }

    #[test]
    fn test_extract_vtc_barrier() {
        let src = r#"varnishtest "test"
barrier b1 cond 2"#;
        let entities = extract_entities_vtc(src, "test.vtc", "test-repo");
        let barriers: Vec<_> = entities
            .iter()
            .filter(|e| e.kind == EntityKind::VtcBarrier)
            .collect();
        assert_eq!(barriers.len(), 1);
        assert_eq!(barriers[0].name, "b1");
    }

    #[test]
    fn test_vtc_backend_synthesis_vcl_plus_backend() {
        let src = r#"varnishtest "test"
server s1 {
    rxreq
    txresp
} -start

varnish v1 -vcl+backend {
    sub vcl_recv { set req.backend_hint = s1; }
} -start"#;
        let entities = extract_entities_vtc(src, "test.vtc", "test-repo");
        // Should have synthesised backend 's1'
        let backends: Vec<_> = entities
            .iter()
            .filter(|e| e.kind == EntityKind::VclBackend)
            .collect();
        assert_eq!(backends.len(), 1);
        assert_eq!(backends[0].name, "s1");
    }

    #[test]
    fn test_vtc_errvcl_skipped() {
        let src = r#"varnishtest "test"
varnish v1 -errvcl "expected msg" {
    invalid vcl that should fail
}"#;
        let entities = extract_entities_vtc(src, "test.vtc", "test-repo");
        // -errvcl block should not produce any VCL entities
        let vcl_entities: Vec<_> = entities.iter().filter(|e| e.language == "vcl").collect();
        assert!(vcl_entities.is_empty());
    }

    #[test]
    fn test_vtc_all_entities_test_context() {
        let src = r#"varnishtest "test"
server s1 { } -start
client c1 { } -run
varnish v1 -vcl+backend { sub vcl_recv { } } -start"#;
        let entities = extract_entities_vtc(src, "test.vtc", "test-repo");
        for e in &entities {
            if e.kind != EntityKind::VclVersion {
                assert!(
                    e.is_test_context,
                    "entity {} should have is_test_context",
                    e.name
                );
            }
        }
    }
}
