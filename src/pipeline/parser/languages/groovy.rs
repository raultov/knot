use crate::models::{EntityKind, ParsedEntity, ReferenceIntent};
use crate::pipeline::parser::comments::strip_comment_markers;

pub(crate) fn handle_groovy_capture(
    capture_name: &str,
    text: &str,
    _node: tree_sitter::Node,
) -> Option<(String, EntityKind, usize)> {
    let line = _node.start_position().row + 1;
    match capture_name {
        "groovy.class.name" => Some((text.to_string(), EntityKind::GroovyClass, line)),
        "groovy.interface.name" => Some((text.to_string(), EntityKind::GroovyInterface, line)),
        "groovy.enum.name" => Some((text.to_string(), EntityKind::GroovyEnum, line)),
        "groovy.method.name" => Some((text.to_string(), EntityKind::GroovyMethod, line)),
        "groovy.field.name" => Some((text.to_string(), EntityKind::GroovyProperty, line)),
        _ => None,
    }
}

// tree-sitter-groovy disabled: CI query compilation unreliable (v1.2.0)
pub(crate) fn extract_entities_groovy(
    source: &str,
    file_path: &str,
    repo_name: &str,
) -> Vec<ParsedEntity> {
    // tree-sitter-groovy (v0.1.2) query compilation fails intermittently on CI
    // runners with tree-sitter 0.26. The ad-hoc lexical parser provides equivalent
    // entity coverage (classes, interfaces, enums, traits, methods, properties) plus
    // scope tracking and reference intent extraction not available via tree-sitter alone.
    let mut entities: Vec<ParsedEntity> = vec![];

    // Extract package declaration
    let package = extract_package(source);

    // Keep track of lines where entities were found to avoid duplicates
    let mut known_lines = std::collections::HashSet::new();

    // Post-process entities from Tree-sitter
    for entity in entities.iter_mut() {
        known_lines.insert(entity.start_line);

        // Fix: tree-sitter-groovy parses `trait` as `class_declaration`.
        if entity.kind == EntityKind::GroovyClass
            && let Some(line_content) = source.lines().nth(entity.start_line.saturating_sub(1))
        {
            let trimmed = line_content.trim();
            if trimmed.starts_with("trait ") || trimmed.contains(" trait ") {
                entity.kind = EntityKind::GroovyTrait;
            }
        }

        // Set FQN for tree-sitter entities
        if let Some(pkg) = &package {
            entity.fqn = match entity.kind {
                EntityKind::GroovyClass
                | EntityKind::GroovyInterface
                | EntityKind::GroovyTrait
                | EntityKind::GroovyEnum => format!("{}.{}", pkg, entity.name),
                _ => continue,
            };
        }
    }

    // Ad-hoc extraction with scope tracking for enclosing_class
    // Scope stack: (name, brace_count_when_entered)
    let mut scope_stack: Vec<(String, usize)> = Vec::new();
    let mut brace_count = 0usize;

    // Materialize lines once: docstring extraction walks backwards from each
    // declaration and must not pay O(n²) re-scanning the source per entity.
    let lines: Vec<&str> = source.lines().collect();

    for (line_idx, line) in source.lines().enumerate() {
        let line_num = line_idx + 1;
        let trimmed = line.trim();

        // Track braces for scope
        brace_count += trimmed.matches('{').count();
        brace_count = brace_count.saturating_sub(trimmed.matches('}').count());

        // Pop scopes whose braces have closed
        while let Some((_, entry_brace)) = scope_stack.last() {
            if brace_count < *entry_brace {
                scope_stack.pop();
            } else {
                break;
            }
        }

        if trimmed.is_empty() || trimmed.starts_with("//") || trimmed.starts_with("/*") {
            continue;
        }

        // Try to extract class/interface/enum/trait if tree-sitter missed it
        if !known_lines.contains(&line_num)
            && let Some((name, kind)) = try_extract_type_declaration(trimmed)
        {
            // Push to scope stack BEFORE brace_count is updated for the current line's `{`
            let fqn = if let Some(pkg) = &package {
                format!("{}.{}", pkg, name)
            } else {
                name.clone()
            };
            let current_brace = brace_count;
            // Build a multi-line declaration so that `extends X` / `implements Y`
            // on a following line still feed `extract_inheritance_intents`.
            // Falls back to the single trimmed line when no `{` is in the
            // lookahead window.
            let decl_text =
                build_type_declaration(source, line_idx).unwrap_or_else(|| trimmed.to_string());
            let inheritance_intents = extract_inheritance_intents(&decl_text, &kind, line_num);
            let docstring = extract_preceding_docstring(&lines, line_idx);
            let mut new_entity = ParsedEntity::new(
                &name, kind, &fqn, None, docstring, "groovy", file_path, line_num, line_num, None,
                repo_name,
            );
            new_entity.reference_intents.extend(inheritance_intents);
            entities.push(new_entity);
            scope_stack.push((name, current_brace));
        }

        // Re-read enclosing after potential scope push
        let enclosing = scope_stack.last().map(|(n, _)| n.clone());

        // Ad-hoc method/field/closure extraction only if tree-sitter didn't already find an entity at this line
        if !known_lines.contains(&line_num) {
            // Try to find a `def` method declaration
            if let Some((method_name, signature)) = try_extract_def_method(trimmed) {
                let fqn = build_fqn(&package, &enclosing, &method_name);
                let docstring = extract_preceding_docstring(&lines, line_idx);
                entities.push(ParsedEntity::new(
                    &method_name,
                    EntityKind::GroovyMethod,
                    &fqn,
                    Some(signature),
                    docstring,
                    "groovy",
                    file_path,
                    line_num,
                    line_num,
                    enclosing,
                    repo_name,
                ));
                continue;
            }

            // Try to find typed methods or script-level methods missed by tree-sitter
            // First, try single-line detection
            if let Some((method_name, _signature)) = try_extract_typed_method(trimmed) {
                // Filter false positives: method names that contain dots or look like object.method()
                if method_name.contains('.')
                    || method_name.chars().all(|c| c.is_uppercase() || c == '_')
                {
                    continue;
                }
                let sig_end = trimmed.find('{').unwrap_or(trimmed.len());
                let signature_full = trimmed[..sig_end].trim().to_string();
                let fqn = build_fqn(&package, &enclosing, &method_name);
                let docstring = extract_preceding_docstring(&lines, line_idx);
                entities.push(ParsedEntity::new(
                    &method_name,
                    EntityKind::GroovyMethod,
                    &fqn,
                    Some(signature_full),
                    docstring,
                    "groovy",
                    file_path,
                    line_num,
                    line_num,
                    enclosing,
                    repo_name,
                ));
                continue;
            }

            // Multi-line method detection: method signature with `(` but no `)` on this line,
            // spanning multiple lines (e.g., closure default parameter values)
            if let Some((method_name, method_start_line)) =
                try_extract_typed_method_multiline(source, line_idx)
                && !method_name.contains('.')
            {
                let fqn = build_fqn(&package, &enclosing, &method_name);
                // The docstring sits above the first line of the signature
                // (method_start_line is 1-based → subtract 1 for the 0-based index).
                let docstring = extract_preceding_docstring(&lines, method_start_line - 1);
                entities.push(ParsedEntity::new(
                    &method_name,
                    EntityKind::GroovyMethod,
                    &fqn,
                    None,
                    docstring,
                    "groovy",
                    file_path,
                    method_start_line,
                    line_num,
                    enclosing,
                    repo_name,
                ));
                continue;
            }
        }

        // Try to extract properties or script-level variables
        if let Some(prop_name) = try_extract_property(trimmed) {
            let fqn = build_fqn(&package, &enclosing, &prop_name);
            let docstring = extract_preceding_docstring(&lines, line_idx);
            entities.push(ParsedEntity::new(
                &prop_name,
                EntityKind::GroovyProperty,
                &fqn,
                None,
                docstring,
                "groovy",
                file_path,
                line_num,
                line_num,
                enclosing,
                repo_name,
            ));
        }
    }

    // Fix end_line for all methods (both tree-sitter and ad-hoc) that
    // couldn't determine their body closing line.
    for entity in entities.iter_mut() {
        if entity.kind == EntityKind::GroovyMethod
            && entity.end_line == entity.start_line
            && let Some(end_line) = find_method_body_end(source, entity.start_line)
            && end_line > entity.start_line
        {
            entity.end_line = end_line;
        }
    }

    // Extract reference intents: for each method, scan source lines after its signature
    let mut method_spans: Vec<(usize, usize, usize)> = entities
        .iter()
        .enumerate()
        .filter(|(_, e)| {
            matches!(
                e.kind,
                EntityKind::GroovyMethod | EntityKind::GroovyFunction
            )
        })
        .map(|(i, e)| (e.start_line, e.end_line, i))
        .collect();
    method_spans.sort_by_key(|(s, _, _)| *s);

    let refs = extract_method_calls(source, &entities);

    // Assign each reference intent to the innermost containing method.
    // When methods are nested (e.g., hyperlinkUpdate inside showGrabbingFinishedMessage),
    // we assign the call to the deepest method, not the outer container.
    for method_ref in refs.iter() {
        if let ReferenceIntent::Call { line, .. } = method_ref {
            // Find all methods that contain this line
            let mut candidates: Vec<(usize, usize, usize)> = method_spans
                .iter()
                .filter(|(m_start, m_end, _)| {
                    let actual_end = if *m_end != *m_start { *m_end } else { *m_start };
                    *line > *m_start && *line <= actual_end
                })
                .copied()
                .collect();
            // Pick the innermost: smallest (end - start) wins
            candidates.sort_by_key(|(s, e, _)| e.saturating_sub(*s));
            if let Some(&(_, _, m_eidx)) = candidates.first() {
                entities[m_eidx].reference_intents.push(method_ref.clone());
            }
        }
    }

    entities
}

/// Scans source for method call patterns and returns reference intents.
// Reserved for future reference extraction
fn extract_method_calls(source: &str, _entities: &[ParsedEntity]) -> Vec<ReferenceIntent> {
    let mut refs = Vec::new();
    let keywords = [
        "if",
        "else",
        "while",
        "for",
        "return",
        "new",
        "throw",
        "catch",
        "switch",
        "case",
        "import",
        "package",
        "class",
        "interface",
        "trait",
        "enum",
        "def",
        "try",
        "finally",
        "assert",
        "println",
        "void",
        "int",
        "String",
        "boolean",
        "double",
        "float",
        "long",
        "byte",
        "short",
        "char",
        "public",
        "private",
        "protected",
        "static",
        "final",
        "abstract",
        "synchronized",
        "volatile",
        "transient",
    ];

    for (line_idx, line) in source.lines().enumerate() {
        let line_num = line_idx + 1;
        let trimmed = line.trim();

        if trimmed.starts_with("//")
            || trimmed.starts_with("/*")
            || trimmed.starts_with("*")
            || trimmed.starts_with("package ")
            || trimmed.starts_with("import ")
        {
            continue;
        }

        let mut chars = trimmed.char_indices().peekable();
        while let Some((i, c)) = chars.next() {
            // Skip string literals to avoid false positives
            if c == '\"' || c == '\'' {
                while let Some((_, nc)) = chars.next() {
                    if nc == c {
                        break; // closing quote found
                    }
                    if nc == '\\' {
                        let _ = chars.next(); // skip escaped char
                    }
                }
                continue;
            }
            if !c.is_alphabetic() && c != '_' {
                continue;
            }

            let word_start = i;
            let mut word_end = i;
            while let Some((_, nc)) = chars.peek() {
                if nc.is_alphanumeric() || *nc == '_' {
                    word_end = chars.next().unwrap().0;
                } else {
                    break;
                }
            }

            let word = &trimmed[word_start..=word_end];
            if keywords.contains(&word) {
                continue;
            }

            let after_word = &trimmed[word_end + 1..];
            let after_trimmed = after_word.trim_start();

            // Pattern: word.word(...)
            if let Some(dot_rest) = after_trimmed.strip_prefix('.') {
                let dot_trimmed = dot_rest.trim_start();
                if let Some((next_word, rest)) = split_identifier(dot_trimmed) {
                    let after_next = rest.trim_start();
                    if after_next.starts_with('(') {
                        refs.push(ReferenceIntent::Call {
                            method: next_word.to_string(),
                            receiver: Some(word.to_string()),
                            line: line_num,
                            arg_count: None,
                        });
                        continue;
                    }
                }
            }

            // Pattern: word(...)
            if after_trimmed.starts_with('(') && !keywords.contains(&word) && word.len() > 1 {
                refs.push(ReferenceIntent::Call {
                    method: word.to_string(),
                    receiver: None,
                    line: line_num,
                    arg_count: None,
                });
            }

            // Pattern: no-paren call — word followed by string literal or identifier args
            // Groovy style: runAnalyzer "abc", 123 or doSomething arg1, arg2
            if !after_trimmed.is_empty()
                && !keywords.contains(&word)
                && word.len() > 1
                && !after_trimmed.starts_with('(')
                && !after_trimmed.starts_with('.')
                && !after_trimmed.starts_with('=')
                && !after_trimmed.starts_with('{')
                && !after_trimmed.starts_with(')')
                && !after_trimmed.starts_with(':')
                && !after_trimmed.starts_with(';')
            {
                let first_arg_char = after_trimmed.chars().next().unwrap();
                // Argument must start with string quote or identifier char
                if first_arg_char == '"'
                    || first_arg_char == '\''
                    || first_arg_char.is_alphabetic()
                    || first_arg_char == '$'
                {
                    refs.push(ReferenceIntent::Call {
                        method: word.to_string(),
                        receiver: None,
                        line: line_num,
                        arg_count: None,
                    });
                }
            }
        }
    }
    refs
}

/// Splits an identifier from the start of `s`, returns (identifier, rest).
// Reserved for future FQN construction
fn split_identifier(s: &str) -> Option<(&str, &str)> {
    let first = s.chars().next()?;
    if !first.is_alphabetic() && first != '_' {
        return None;
    }
    let end = s
        .find(|c: char| !c.is_alphanumeric() && c != '_')
        .unwrap_or(s.len());
    Some((&s[..end], &s[end..]))
}

/// Extract package name from source (e.g., `package com.example.service`)
// Reserved for future package resolution
fn extract_package(source: &str) -> Option<String> {
    for line in source.lines().take(20) {
        let trimmed = line.trim();
        if let Some(pkg) = trimmed.strip_prefix("package ") {
            let name = pkg.trim().trim_end_matches(';').trim();
            if !name.is_empty() {
                return Some(name.to_string());
            }
        }
    }
    None
}

/// Build a fully-qualified name: package.parent.method or package.method
// Reserved for future FQN construction
fn build_fqn(package: &Option<String>, parent: &Option<String>, name: &str) -> String {
    match (package, parent) {
        (Some(pkg), Some(enclosing_class)) => format!("{}.{}.{}", pkg, enclosing_class, name),
        (Some(pkg), None) => format!("{}.{}", pkg, name),
        (None, Some(enclosing_class)) => format!("{}.{}", enclosing_class, name),
        (None, None) => name.to_string(),
    }
}

/// Walks backwards from the line preceding `decl_line_idx` (0-based) collecting
/// the GroovyDoc / comment block that documents the declaration.
///
/// Policy (backwards walk from the declaration):
/// 1. Skip (without stopping the search): annotation lines (`@X`) and at most
///    one blank line — same tolerance as the generic tree-sitter extractor.
/// 2. Capture: an adjacent `/** ... */` / `/* ... */` block, or a burst of
///    consecutive `//` lines. Only the adjacent block is taken.
/// 3. Stop immediately (returning whatever was captured, or `None`) on any
///    other non-empty code line (`package`, `import`, statements) or at the
///    start of the file — this protects against license headers leaking into
///    the first class of a file.
/// 4. Markers (`/**`, `*/`, leading `*`, `//`) are stripped via
///    [`strip_comment_markers`]; an empty cleaned result maps to `None`.
fn extract_preceding_docstring(lines: &[&str], decl_line_idx: usize) -> Option<String> {
    let non_empty = |cleaned: String| (!cleaned.trim().is_empty()).then_some(cleaned);

    // Phase 1: skip annotations and at most one blank line.
    let mut idx = decl_line_idx;
    let mut blank_seen = false;
    while idx > 0 {
        let prev = lines[idx - 1].trim();
        if prev.starts_with('@') {
            idx -= 1;
            continue;
        }
        if prev.is_empty() && !blank_seen {
            blank_seen = true;
            idx -= 1;
            continue;
        }
        break;
    }
    if idx == 0 {
        return None;
    }

    let last = lines[idx - 1].trim();

    // Case A: block comment — `/** ... */` or `/* ... */`.
    if last.ends_with("*/") {
        if last.starts_with("/*") {
            // Opener and closer on the same line (or this IS the opener line of
            // a block whose body sits above is impossible: the closer is here).
            return non_empty(strip_comment_markers(last));
        }
        if !last.starts_with('*') {
            // `code(); /* inline */` — trailing comment on a code line is not a
            // docstring.
            return None;
        }
        // Multi-line block: walk back through `*` continuation lines until the
        // `/*` opener.
        let mut block: Vec<&str> = vec![lines[idx - 1]];
        let mut j = idx - 1;
        while j > 0 {
            j -= 1;
            let t = lines[j].trim();
            if t.starts_with("/*") {
                block.push(lines[j]);
                block.reverse();
                return non_empty(strip_comment_markers(&block.join("\n")));
            }
            if t.starts_with('*') {
                block.push(lines[j]);
                continue;
            }
            // Non-comment line reached before the opener → malformed block.
            return None;
        }
        // Start of file reached without an opener → malformed block.
        return None;
    }

    // Case B: burst of consecutive `//` line comments.
    if last.starts_with("//") {
        let mut j = idx - 1;
        let mut burst: Vec<&str> = Vec::new();
        loop {
            if !lines[j].trim().starts_with("//") {
                break;
            }
            burst.push(lines[j]);
            if j == 0 {
                break;
            }
            j -= 1;
        }
        burst.reverse();
        return non_empty(strip_comment_markers(&burst.join("\n")));
    }

    None
}

/// Tries to extract class, interface, enum, or trait declarations
/// Scans forward from `line_num` to find the matching closing `}` of the method body.
// Reserved for future method body parsing
fn find_method_body_end(source: &str, line_num: usize) -> Option<usize> {
    let mut chars = source.chars().peekable();
    let mut current_line = 1usize;
    let mut brace_depth = 0i32;
    let mut found_opening = false;

    while current_line < line_num {
        match chars.next() {
            Some('\n') => current_line += 1,
            Some(_) => {}
            None => return None,
        }
    }

    while let Some(ch) = chars.next() {
        match ch {
            '\n' => current_line += 1,
            '/' if chars.peek() == Some(&'/') => {
                for c in chars.by_ref() {
                    if c == '\n' {
                        current_line += 1;
                        break;
                    }
                }
            }
            '"' | '\'' => {
                let quote = ch;
                while let Some(c) = chars.next() {
                    if c == '\\' {
                        let _ = chars.next();
                    } else if c == quote {
                        break;
                    }
                }
            }
            '{' => {
                brace_depth += 1;
                found_opening = true;
            }
            '}' => {
                brace_depth -= 1;
                if found_opening && brace_depth == 0 {
                    return Some(current_line);
                }
            }
            _ => {}
        }
    }
    None
}

// Reserved for future type declaration parsing
fn try_extract_type_declaration(line: &str) -> Option<(String, EntityKind)> {
    let tokens: Vec<&str> = line.split_whitespace().collect();

    for (i, token) in tokens.iter().enumerate() {
        let kind = match *token {
            "class" => EntityKind::GroovyClass,
            "interface" => EntityKind::GroovyInterface,
            "trait" => EntityKind::GroovyTrait,
            "enum" => EntityKind::GroovyEnum,
            _ => continue,
        };

        if i + 1 < tokens.len() {
            // The next token should be the name
            let name_raw = tokens[i + 1];
            // Remove generic types, extends, implements, curly braces
            let name = name_raw
                .split('<')
                .next()
                .unwrap_or(name_raw)
                .split('{')
                .next()
                .unwrap_or(name_raw)
                .trim();

            if !name.is_empty() && name.chars().next().unwrap().is_alphabetic() {
                return Some((name.to_string(), kind));
            }
        }
    }
    None
}

/// Builds the textual declaration of a type from `line_idx` onwards, stopping at
/// the first `{` (exclusive). Returns `None` if no `{` is found within
/// `MAX_LOOKAHEAD` lines — the caller then falls back to the single line.
fn build_type_declaration(source: &str, line_idx: usize) -> Option<String> {
    const MAX_LOOKAHEAD: usize = 5;
    let lines: Vec<&str> = source.lines().collect();
    let mut buf = String::new();
    for offset in 0..MAX_LOOKAHEAD {
        let raw = lines.get(line_idx + offset)?.trim();
        if raw.is_empty() {
            buf.push(' ');
            continue;
        }
        // Skip pure comment / javadoc continuations (mirrors the main loop's policy).
        if raw.starts_with("//") || raw.starts_with("/*") || raw.starts_with("* ") || raw == "*" {
            continue;
        }
        if !buf.is_empty() {
            buf.push(' ');
        }
        buf.push_str(raw);
        if raw.contains('{') {
            return Some(buf);
        }
    }
    None
}

/// Strips every balanced `<...>` section from `input`, preserving any characters
/// outside them. We use a manual depth counter instead of a regex so that nested
/// generics like `Map<List<X>, Y>` are erased in a single pass. This both
/// neutralises generic bounds (`class Box<T extends Comparable>`) and discards
/// type arguments on the parent (`extends AbstractRepo<Order, Long>`).
fn strip_balanced_generics(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut depth: i32 = 0;
    for ch in input.chars() {
        match ch {
            '<' => depth += 1,
            '>' if depth > 0 => depth -= 1,
            '>' => {} // unbalanced '>' — drop silently (defensive)
            _ if depth == 0 => out.push(ch),
            _ => {} // skip chars inside generics
        }
    }
    out
}

/// Validates a single parent/interface name token: must start with an
/// alphabetic character, may contain alphanumerics, underscores and dots.
fn is_valid_type_name(s: &str) -> bool {
    if s.is_empty() {
        return false;
    }
    let mut chars = s.chars();
    let first = chars.next().unwrap();
    if !first.is_alphabetic() && first != '_' {
        return false;
    }
    chars.all(|c| c.is_alphanumeric() || c == '_' || c == '.')
}

/// Extracts `ReferenceIntent::Extends` / `Implements` from a Groovy type
/// declaration.
///
/// `decl` is the complete declaration text (possibly concatenated across
/// multiple lines, up to the opening `{`). `kind` decides the inheritance
/// semantics:
///
/// - `GroovyInterface`: every name after `extends` becomes an `Extends` intent
///   (mirrors the Kotlin parser' choice to treat interface-extends-interface as
///   `Extends`).
/// - `GroovyClass`, `GroovyTrait`, `GroovyEnum`: the first name after `extends`
///   becomes an `Extends` intent (Groovy only allows a single parent), and
///   every name after `implements` becomes an `Implements` intent.
///
/// Notes:
/// - Generic bounds inside the type header (`class Box<T extends Comparable>`)
///   are stripped before tokenisation so the `extends Comparable` token never
///   reaches the matcher.
/// - Generic arguments on the parent (`extends AbstractRepo<Order, Long>`) are
///   also stripped so resolution receives just the simple/FQN name.
/// - Declarations with embedded block comments on the same line are out of
///   scope — same robustness bar as the rest of the lexical parser.
pub(crate) fn extract_inheritance_intents(
    decl: &str,
    kind: &EntityKind,
    line: usize,
) -> Vec<ReferenceIntent> {
    let stripped = strip_balanced_generics(decl);
    let mut intents = Vec::new();

    // Look for `extends` and `implements` keywords (case-sensitive, word-bounded).
    let tokens: Vec<&str> = stripped.split_whitespace().collect();
    let extends_idx = tokens.iter().position(|t| *t == "extends");
    let implements_idx = tokens.iter().position(|t| *t == "implements");

    if let Some(idx) = extends_idx {
        let from = idx + 1;
        let to = implements_idx.unwrap_or(tokens.len());
        let parents: Vec<&str> = tokens[from..to]
            .iter()
            .copied()
            .flat_map(|t| t.split(','))
            .map(str::trim)
            .filter(|t| is_valid_type_name(t))
            .collect();

        match kind {
            EntityKind::GroovyInterface => {
                for parent in parents {
                    intents.push(ReferenceIntent::Extends {
                        parent: parent.to_string(),
                        line,
                    });
                }
            }
            _ => {
                if let Some(first) = parents.into_iter().next() {
                    intents.push(ReferenceIntent::Extends {
                        parent: first.to_string(),
                        line,
                    });
                }
            }
        }
    }

    if let Some(idx) = implements_idx {
        let from = idx + 1;
        for tok in &tokens[from..] {
            // Any `{` opens the class body — stop processing names so we don't
            // pick up enum constants or inner-class members as interfaces.
            if tok.contains('{') {
                break;
            }
            for piece in tok.split(',') {
                let trimmed = piece.trim();
                if trimmed.is_empty() {
                    continue;
                }
                if is_valid_type_name(trimmed) {
                    intents.push(ReferenceIntent::Implements {
                        interface: trimmed.to_string(),
                        line,
                    });
                }
            }
        }
    }

    intents
}

/// Tries to extract properties (fields, script variables)
// Reserved for future property extraction
fn try_extract_property(line: &str) -> Option<String> {
    // A very basic heuristic for `Type name = ...` or `def name = ...`
    if let Some(eq_idx) = line.find('=') {
        let left_side = line[..eq_idx].trim();
        // Discard things like `a == b` or assignments in methods
        if left_side.is_empty() || line.chars().nth(eq_idx + 1) == Some('=') {
            return None;
        }

        let tokens: Vec<&str> = left_side.split_whitespace().collect();
        if tokens.len() >= 2 {
            let name = tokens.last().unwrap();
            let first_char = name.chars().next().unwrap();
            // Must start with letter/underscore and not contain weird chars
            if (first_char.is_alphabetic() || first_char == '_')
                && name.chars().all(|c| c.is_alphanumeric() || c == '_')
            {
                // Ignore if it looks like a method signature or control structure
                if !line.contains("if ") && !line.contains("while ") && !line.contains("for ") {
                    return Some(name.to_string());
                }
            }
        }
    }
    None
}

/// Tries to extract a method name from a multi-line method signature.
///
/// Handles cases like:
///   private static SimpleHttpServer restartHttpServer(String id, String webRootPath,
///                                                      Closure handler = {null},
///                                                      Closure errorListener = {}) {
///
/// where the opening `(` and closing `)` are on different lines.
// Reserved for future multiline method parsing
fn try_extract_typed_method_multiline(source: &str, line_idx: usize) -> Option<(String, usize)> {
    let lines: Vec<&str> = source.lines().collect();
    let start_line = lines.get(line_idx)?;
    let trimmed = start_line.trim();

    let method_start_keywords = [
        "private",
        "public",
        "protected",
        "static",
        "final",
        "abstract",
        "synchronized",
        "volatile",
        "transient",
        "native",
        "void",
        "boolean",
        "byte",
        "short",
        "int",
        "long",
        "float",
        "double",
        "char",
        "String",
        "Object",
        "List",
        "Map",
        "Set",
        "Closure",
        "SimpleHttpServer",
    ];

    if trimmed.starts_with("if ")
        || trimmed.starts_with("while ")
        || trimmed.starts_with("for ")
        || trimmed.starts_with("catch ")
        || trimmed.starts_with("switch ")
        || trimmed.starts_with("return ")
    {
        return None;
    }

    // Must contain `(` but not `)` on the same line
    if !trimmed.contains('(') || trimmed.contains(')') {
        return None;
    }

    let paren_idx = trimmed.find('(').unwrap();
    if trimmed[..paren_idx].contains('=') {
        return None;
    }

    let before_paren = trimmed[..paren_idx].trim();
    let tokens: Vec<&str> = before_paren.split_whitespace().collect();

    // Need at least 2 tokens (type keyword + method name)
    if tokens.len() < 2 {
        return None;
    }

    // Check that tokens look like access modifiers / type / name pattern
    let has_modifier = tokens.iter().any(|t| method_start_keywords.contains(t));
    if !has_modifier {
        // Also check if the second-to-last token looks like a type (starts with uppercase)
        if tokens.len() >= 2 {
            let second_last = tokens[tokens.len() - 2];
            if !second_last.chars().next().is_some_and(|c| c.is_uppercase()) {
                return None;
            }
        } else {
            return None;
        }
    }

    let name = tokens.last().unwrap();
    let first_char = name.chars().next()?;
    if !first_char.is_alphabetic() && first_char != '_' {
        return None;
    }

    // Scan ahead for the closing `)` and opening `{` (within a reasonable window)
    let max_lookahead = 10;
    let mut found_close_paren = false;
    for offset in 1..=max_lookahead {
        let next_line = lines.get(line_idx + offset)?;
        let next_trimmed = next_line.trim();

        if !found_close_paren && next_trimmed.contains(')') {
            found_close_paren = true;
        }

        if next_trimmed.contains('{') {
            // Must have found `)` before `{`
            if found_close_paren {
                return Some((name.to_string(), line_idx + 1));
            }
            // `{` before `)` indicates a closure literal, not the method body
        }

        if next_trimmed.is_empty()
            || next_trimmed.starts_with("//")
            || next_trimmed.starts_with("/*")
        {
            continue;
        }
    }

    None
}

/// Tries to extract a typed method name and signature
// Reserved for future typed method parsing
fn try_extract_typed_method(line: &str) -> Option<(String, String)> {
    // Quick heuristic: contains `(` and `)` and `{`, doesn't start with `if`/`while`/`for`/`catch`
    if line.contains('(') && line.contains(')') && (line.contains('{') || line.ends_with(')')) {
        if line.starts_with("if ")
            || line.starts_with("while ")
            || line.starts_with("for ")
            || line.starts_with("catch ")
            || line.starts_with("switch ")
        {
            return None;
        }

        let paren_idx = line.find('(').unwrap();

        // Reject assignment patterns like `def foo = bar(...)` — these are calls, not declarations
        if line[..paren_idx].contains('=') {
            return None;
        }

        // Reject constructor calls like `new File(...)` or `new SimpleHttpServer()`
        if line[..paren_idx].contains("new ") || line[..paren_idx].ends_with(" new") {
            return None;
        }

        let before_paren = line[..paren_idx].trim();

        // Handle quoted method names (Spock feature methods)
        if let Some(quote_idx) = before_paren.find('\"') {
            // Find the closing quote
            if let Some(close_idx) = before_paren[quote_idx + 1..].find('\"') {
                let inner_name = &before_paren[quote_idx + 1..quote_idx + 1 + close_idx];
                let sig_end = line.find('{').unwrap_or(line.len());
                let signature = line[..sig_end].trim().to_string();
                return Some((inner_name.to_string(), signature));
            }
        }

        let tokens: Vec<&str> = before_paren.split_whitespace().collect();
        if tokens.len() >= 2 {
            let name = tokens.last().unwrap();
            let first_char = name.chars().next().unwrap();
            if first_char.is_alphabetic() || first_char == '_' {
                let sig_end = line.find('{').unwrap_or(line.len());
                let signature = line[..sig_end].trim().to_string();
                return Some((name.to_string(), signature));
            }
        }
    }
    None
}

/// Tries to extract a method name and signature from a line containing `def`
// Reserved for future def method parsing
fn try_extract_def_method(line: &str) -> Option<(String, String)> {
    // Look for `def `
    if let Some(def_idx) = line.find("def ") {
        // Ensure `def` is a word by checking the preceding character (if any)
        if def_idx > 0 {
            let prev_char = line.as_bytes()[def_idx - 1] as char;
            if prev_char.is_alphanumeric() || prev_char == '_' {
                return None;
            }
        }

        let after_def = &line[def_idx + 4..].trim_start();

        // Find the opening parenthesis for the method arguments
        if let Some(paren_idx) = after_def.find('(') {
            let potential_name = &after_def[..paren_idx].trim();

            // Validate the name: must be a valid identifier and not contain spaces
            if !potential_name.is_empty() && !potential_name.contains(|c: char| c.is_whitespace()) {
                // Must start with letter or underscore
                let first_char = potential_name.chars().next().unwrap();
                if first_char.is_alphabetic() || first_char == '_' {
                    // Extract signature from 'def' to the start of the block '{' or end of line
                    let sig_end = line[def_idx..]
                        .find('{')
                        .map(|i| i + def_idx)
                        .unwrap_or(line.len());
                    let signature = line[def_idx..sig_end].trim().to_string();

                    return Some((potential_name.to_string(), signature));
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pipeline::parser::test_utils::{
        assert_extends, assert_implements, collect_extends, collect_implements,
    };

    /// Helper: pick the Groovy class entity for `name` from the parser output.
    fn pick_class<'a>(entities: &'a [ParsedEntity], name: &str) -> &'a ParsedEntity {
        entities
            .iter()
            .find(|e| e.name == name && e.kind == EntityKind::GroovyClass)
            .unwrap_or_else(|| panic!("Groovy class '{name}' not found in entities"))
    }

    fn pick_entity<'a>(
        entities: &'a [ParsedEntity],
        name: &str,
        kind: EntityKind,
    ) -> &'a ParsedEntity {
        entities
            .iter()
            .find(|e| e.name == name && e.kind == kind)
            .unwrap_or_else(|| {
                panic!(
                    "Entity '{name}' ({kind:?}) not found in entities. Available: {:?}",
                    entities
                        .iter()
                        .map(|e| (&e.name, &e.kind))
                        .collect::<Vec<_>>()
                )
            })
    }

    // ---- Groovy Standard (tree-sitter) extraction tests ----

    #[test]
    fn test_groovy_class_extraction() {
        let source = "class MyGroovyClass { def method() {} }";
        let entities = extract_entities_groovy(source, "test.groovy", "test-repo");
        assert!(
            entities
                .iter()
                .any(|e| e.name == "MyGroovyClass" && e.kind == EntityKind::GroovyClass)
        );
    }

    #[test]
    fn test_groovy_interface_extraction() {
        let source = "interface MyGroovyInterface { void doIt() }";
        let entities = extract_entities_groovy(source, "test.groovy", "test-repo");
        assert!(
            entities
                .iter()
                .any(|e| e.name == "MyGroovyInterface" && e.kind == EntityKind::GroovyInterface)
        );
    }

    #[test]
    fn test_groovy_enum_extraction() {
        let source = "enum Color { RED, GREEN, BLUE }";
        let entities = extract_entities_groovy(source, "test.groovy", "test-repo");
        assert!(
            entities
                .iter()
                .any(|e| e.name == "Color" && e.kind == EntityKind::GroovyEnum)
        );
    }

    #[test]
    fn test_groovy_method_extraction() {
        let source = "class Foo { String greet(String name) { return name } }";
        let entities = extract_entities_groovy(source, "test.groovy", "test-repo");
        let method = entities.iter().find(|e| e.name == "greet");
        assert!(method.is_some(), "Expected method 'greet' to be extracted");
        assert_eq!(method.unwrap().kind, EntityKind::GroovyMethod);
    }

    #[test]
    fn test_groovy_trait_extraction() {
        let source = "trait MyTrait { void doSomething() {} }";
        let entities = extract_entities_groovy(source, "test.groovy", "test-repo");
        assert!(
            entities
                .iter()
                .any(|e| e.name == "MyTrait" && e.kind == EntityKind::GroovyTrait)
        );
    }

    #[test]
    fn test_groovy_property_extraction() {
        let source = "class Foo { String name = 'test' }";
        let entities = extract_entities_groovy(source, "test.groovy", "test-repo");
        assert!(
            entities
                .iter()
                .any(|e| e.name == "name" && e.kind == EntityKind::GroovyProperty)
        );
    }

    #[test]
    fn test_groovy_multiple_classes() {
        let source = "package com.example\nclass First {}\nclass Second {}\nclass Third {}";
        let entities = extract_entities_groovy(source, "test.groovy", "test-repo");
        let class_names: Vec<_> = entities
            .iter()
            .filter(|e| e.kind == EntityKind::GroovyClass)
            .map(|e| e.name.clone())
            .collect();
        assert!(class_names.contains(&"First".to_string()));
        assert!(class_names.contains(&"Second".to_string()));
        assert!(class_names.contains(&"Third".to_string()));
    }

    #[test]
    fn test_groovy_constructor_extraction() {
        let source = "class User { User(String name) { this.name = name } }";
        let entities = extract_entities_groovy(source, "test.groovy", "test-repo");
        assert!(
            entities
                .iter()
                .any(|e| e.name == "User" && e.kind == EntityKind::GroovyMethod)
        );
    }

    #[test]
    fn test_groovy_empty_body_class() {
        let source = "class EmptyClass {}";
        let entities = extract_entities_groovy(source, "test.groovy", "test-repo");
        assert!(
            entities
                .iter()
                .any(|e| e.name == "EmptyClass" && e.kind == EntityKind::GroovyClass)
        );
    }

    #[test]
    fn test_groovy_method_in_class_extracts_correctly() {
        let source = "class Calculator {\n  int add(int a, int b) { return a + b }\n  int subtract(int a, int b) { return a - b }\n}";
        let entities = extract_entities_groovy(source, "test.groovy", "test-repo");
        assert!(
            entities
                .iter()
                .any(|e| e.name == "add" && e.kind == EntityKind::GroovyMethod)
        );
        assert!(
            entities
                .iter()
                .any(|e| e.name == "subtract" && e.kind == EntityKind::GroovyMethod)
        );
    }

    #[test]
    fn test_groovy_parse_sample_full_file() {
        let source = include_str!("../../../../tests/testing_files/sample_full.groovy");
        let entities = extract_entities_groovy(source, "sample_full.groovy", "test-repo");

        println!("--- Extracted Entities ---");
        for e in &entities {
            println!("{:?} - {}", e.kind, e.name);
        }
        println!("--------------------------");

        assert!(
            entities
                .iter()
                .any(|e| e.name == "UserService" && e.kind == EntityKind::GroovyClass)
        );
        assert!(
            entities
                .iter()
                .any(|e| e.name == "BaseService" && e.kind == EntityKind::GroovyClass)
        );
        assert!(
            entities
                .iter()
                .any(|e| e.name == "DatabaseConfig" && e.kind == EntityKind::GroovyClass)
        );
        assert!(
            entities
                .iter()
                .any(|e| e.name == "Repository" && e.kind == EntityKind::GroovyInterface)
        );
        assert!(
            entities
                .iter()
                .any(|e| e.name == "Auditable" && e.kind == EntityKind::GroovyTrait)
        );
        assert!(
            entities
                .iter()
                .any(|e| e.name == "Status" && e.kind == EntityKind::GroovyEnum)
        );
        assert!(
            entities
                .iter()
                .any(|e| e.name == "scriptMethod" && e.kind == EntityKind::GroovyMethod)
        );
        assert!(
            entities
                .iter()
                .any(|e| e.name == "anotherScriptMethod" && e.kind == EntityKind::GroovyMethod)
        );
        assert!(
            entities
                .iter()
                .any(|e| e.name == "globalConfig" && e.kind == EntityKind::GroovyProperty)
        );
        assert!(
            entities
                .iter()
                .any(|e| e.name == "processDataClosure" && e.kind == EntityKind::GroovyProperty)
        );
        assert!(
            entities
                .iter()
                .any(|e| e.name == "initialize" && e.kind == EntityKind::GroovyMethod)
        );
        assert!(
            entities
                .iter()
                .any(|e| e.name == "calculateTotal" && e.kind == EntityKind::GroovyMethod)
        );
        assert!(
            entities
                .iter()
                .any(|e| e.name == "logAction" && e.kind == EntityKind::GroovyMethod)
        );
        assert!(entities.iter().any(|e| e.name
            == "addition of #num1 and #num2 should be #expected"
            && e.kind == EntityKind::GroovyMethod));
        assert!(
            entities
                .iter()
                .any(|e| e.name == "DEFAULT_ROLE" && e.kind == EntityKind::GroovyProperty)
        );
        assert!(
            entities
                .iter()
                .any(|e| e.name == "maxLoginAttempts" && e.kind == EntityKind::GroovyProperty)
        );

        assert!(
            entities.len() >= 20,
            "Expected at least 20 entities, got {}",
            entities.len()
        );

        // Docstring extraction: comments in the fixture now surface as docstrings.
        let global_config = entities
            .iter()
            .find(|e| e.name == "globalConfig" && e.kind == EntityKind::GroovyProperty)
            .expect("globalConfig not extracted");
        assert_eq!(
            global_config.docstring.as_deref(),
            Some("1. Top-level script variables and closures")
        );
        let user_service = entities
            .iter()
            .find(|e| e.name == "UserService" && e.kind == EntityKind::GroovyClass)
            .expect("UserService not extracted");
        assert_eq!(
            user_service.docstring.as_deref(),
            Some("7. Main Class with Annotations, Inheritance, Traits, and inner classes")
        );
        let initialize = entities
            .iter()
            .find(|e| {
                e.name == "initialize"
                    && e.kind == EntityKind::GroovyMethod
                    && e.enclosing_class.as_deref() == Some("UserService")
            })
            .expect("UserService.initialize not extracted");
        assert_eq!(
            initialize.docstring.as_deref(),
            Some("Typed Method overriding base class")
        );
        // Regression: a property with no preceding comment keeps docstring == None.
        let max_login = entities
            .iter()
            .find(|e| e.name == "maxLoginAttempts" && e.kind == EntityKind::GroovyProperty)
            .expect("maxLoginAttempts not extracted");
        assert_eq!(max_login.docstring, None);
    }

    #[test]
    fn test_groovy_fqn_with_package() {
        let source = "package com.acme.app\nclass MyService { String greet(String name) { name } }";
        let entities = extract_entities_groovy(source, "test.groovy", "test-repo");

        let class_entity = entities
            .iter()
            .find(|e| e.name == "MyService")
            .expect("MyService class not extracted");
        assert_eq!(class_entity.fqn, "com.acme.app.MyService");

        let method_entity = entities
            .iter()
            .find(|e| e.name == "greet")
            .expect("greet method not extracted");
        assert_eq!(method_entity.fqn, "com.acme.app.MyService.greet");
        assert_eq!(method_entity.enclosing_class.as_deref(), Some("MyService"));
    }

    #[test]
    fn test_groovy_method_parent_class() {
        let source = "class Calculator {\n  int add(int a, int b) { a + b }\n  def multiply(int x, int y) { x * y }\n}";
        let entities = extract_entities_groovy(source, "test.groovy", "test-repo");

        let add_method = entities
            .iter()
            .find(|e| e.name == "add")
            .expect("add method not extracted");
        assert_eq!(add_method.enclosing_class.as_deref(), Some("Calculator"));
        assert_eq!(add_method.fqn, "Calculator.add");

        let multiply_method = entities
            .iter()
            .find(|e| e.name == "multiply")
            .expect("multiply method not extracted");
        assert_eq!(
            multiply_method.enclosing_class.as_deref(),
            Some("Calculator")
        );
        assert_eq!(multiply_method.fqn, "Calculator.multiply");
    }

    #[test]
    fn test_groovy_interface_method_has_parent() {
        let source = "interface Repository {\n  String findById(String id)\n}";
        let entities = extract_entities_groovy(source, "test.groovy", "test-repo");

        let method = entities
            .iter()
            .find(|e| e.name == "findById")
            .expect("findById not extracted");
        assert_eq!(method.enclosing_class.as_deref(), Some("Repository"));
    }

    #[test]
    fn test_groovy_nested_scope_tracking() {
        let source = "class Outer {\n  class Inner {\n    String getValue() { 'val' }\n  }\n}";
        let entities = extract_entities_groovy(source, "test.groovy", "test-repo");

        let outer = entities
            .iter()
            .find(|e| e.name == "Outer")
            .expect("Outer class not extracted");
        assert_eq!(outer.kind, EntityKind::GroovyClass);

        let inner = entities
            .iter()
            .find(|e| e.name == "Inner")
            .expect("Inner class not extracted");
        assert_eq!(inner.kind, EntityKind::GroovyClass);

        let method = entities
            .iter()
            .find(|e| e.name == "getValue")
            .expect("getValue method not extracted");
        assert_eq!(method.enclosing_class.as_deref(), Some("Inner"));
        assert_eq!(method.fqn, "Inner.getValue");
    }

    #[test]
    fn test_groovy_trait_method_has_parent() {
        let source = "trait Auditable {\n  def logAction(String msg) { println msg }\n}";
        let entities = extract_entities_groovy(source, "test.groovy", "test-repo");

        let method = entities
            .iter()
            .find(|e| e.name == "logAction")
            .expect("logAction not extracted");
        assert_eq!(method.enclosing_class.as_deref(), Some("Auditable"));
        assert_eq!(method.fqn, "Auditable.logAction");
    }

    #[test]
    fn test_groovy_resilience_empty_file() {
        let entities = extract_entities_groovy("", "test.groovy", "test-repo");
        assert!(entities.is_empty());
    }

    #[test]
    fn test_groovy_resilience_malformed() {
        let source = "garbage {{{ // not valid groovy\nclass ";
        let entities = extract_entities_groovy(source, "test.groovy", "test-repo");
        // Should not panic, just return what it can (likely empty)
        assert!(entities.is_empty() || entities.iter().any(|e| e.name == "class"));
    }

    #[test]
    fn test_innermost_assignment_nested_methods() {
        // Replicates code-history-mining UI.groovy pattern:
        // showGrabbingFinishedMessage contains hyperlinkUpdate which calls runAnalyzer.
        // Only hyperlinkUpdate (innermost) should get the reference, NOT the outer container.
        let source = r#"
package com.example

class NestedMethods {
    def showGrabbingFinishedMessage(String message) {
        show(message, new Listener() {
            @Override void hyperlinkUpdate(String event) {
                runAnalyzer("visualize")
            }
        })
    }

    def show(message, Listener listener) {
    }

    private void runAnalyzer(String action) {
        println action
    }
}
"#;
        let entities = extract_entities_groovy(source, "NestedMethods.groovy", "test-repo");

        // hyperlinkUpdate should get the runAnalyzer call
        let hyperlink = entities
            .iter()
            .find(|e| e.name == "hyperlinkUpdate")
            .expect("hyperlinkUpdate not found");
        let hyper_has_run = hyperlink
            .reference_intents
            .iter()
            .any(|r| matches!(r, ReferenceIntent::Call { method, .. } if method == "runAnalyzer"));
        assert!(
            hyper_has_run,
            "hyperlinkUpdate should have CALL to runAnalyzer"
        );

        // showGrabbingFinishedMessage must NOT have the runAnalyzer call
        let outer = entities
            .iter()
            .find(|e| e.name == "showGrabbingFinishedMessage")
            .expect("showGrabbingFinishedMessage not found");
        let outer_has_run = outer
            .reference_intents
            .iter()
            .any(|r| matches!(r, ReferenceIntent::Call { method, .. } if method == "runAnalyzer"));
        assert!(
            !outer_has_run,
            "showGrabbingFinishedMessage should NOT have CALL to runAnalyzer (belongs to hyperlinkUpdate)"
        );
    }

    #[test]
    fn test_groovy_resilience_missing_braces() {
        let source =
            "class Broken {\n  def method1() { }\n  def method2() { }\n// no closing brace";
        let entities = extract_entities_groovy(source, "test.groovy", "test-repo");
        // Should extract what it can without panicking
        assert!(entities.iter().any(|e| e.name == "Broken"));
        assert!(entities.iter().any(|e| e.name == "method1"));
        assert!(entities.iter().any(|e| e.name == "method2"));
    }

    // ─────────────────────────────────────────────────────────────────────
    // Group: Groovy inheritance intent extraction (Extends / Implements)
    // ─────────────────────────────────────────────────────────────────────

    #[test]
    fn test_groovy_class_extends() {
        let source = "class Ext1 extends PluginExtensionPoint { }";
        let entities = extract_entities_groovy(source, "test.groovy", "test-repo");
        let ext1 = pick_class(&entities, "Ext1");
        assert_extends(&ext1.reference_intents, "PluginExtensionPoint");
        assert!(collect_implements(&ext1.reference_intents).is_empty());
    }

    #[test]
    fn test_groovy_class_implements() {
        let source = "abstract class PluginExtensionPoint implements ExtensionPoint { }";
        let entities = extract_entities_groovy(source, "test.groovy", "test-repo");
        let cls = pick_class(&entities, "PluginExtensionPoint");
        assert_implements(&cls.reference_intents, "ExtensionPoint");
        assert!(collect_extends(&cls.reference_intents).is_empty());
    }

    #[test]
    fn test_groovy_class_extends_and_implements_multiple() {
        let source =
            "class OrderService extends BaseService implements Auditable, Serializable { }";
        let entities = extract_entities_groovy(source, "test.groovy", "test-repo");
        let cls = pick_class(&entities, "OrderService");
        let extends = collect_extends(&cls.reference_intents);
        let implements = collect_implements(&cls.reference_intents);
        assert_eq!(extends, vec!["BaseService"]);
        assert_eq!(implements.len(), 2);
        assert!(implements.contains(&"Auditable"));
        assert!(implements.contains(&"Serializable"));
    }

    #[test]
    fn test_groovy_extends_with_generics() {
        let source = "class Repo extends AbstractRepo<Order, Long> { }";
        let entities = extract_entities_groovy(source, "test.groovy", "test-repo");
        let cls = pick_class(&entities, "Repo");
        assert_extends(&cls.reference_intents, "AbstractRepo");
    }

    #[test]
    fn test_groovy_generic_bound_is_not_extends() {
        let source = "class Box<T extends Comparable> { }";
        let entities = extract_entities_groovy(source, "test.groovy", "test-repo");
        let cls = pick_class(&entities, "Box");
        assert!(collect_extends(&cls.reference_intents).is_empty());
        assert!(collect_implements(&cls.reference_intents).is_empty());
    }

    #[test]
    fn test_groovy_interface_extends_multiple() {
        let source = "interface EventBus extends Publisher, Subscriber { }";
        let entities = extract_entities_groovy(source, "test.groovy", "test-repo");
        let iface = pick_entity(&entities, "EventBus", EntityKind::GroovyInterface);
        let extends = collect_extends(&iface.reference_intents);
        assert_eq!(extends.len(), 2);
        assert!(extends.contains(&"Publisher"));
        assert!(extends.contains(&"Subscriber"));
        assert!(collect_implements(&iface.reference_intents).is_empty());
    }

    #[test]
    fn test_groovy_trait_implements() {
        let source = "trait Auditable implements Serializable { }";
        let entities = extract_entities_groovy(source, "test.groovy", "test-repo");
        let trait_entity = pick_entity(&entities, "Auditable", EntityKind::GroovyTrait);
        let implements = collect_implements(&trait_entity.reference_intents);
        assert_eq!(implements, vec!["Serializable"]);
        assert!(collect_extends(&trait_entity.reference_intents).is_empty());
    }

    #[test]
    fn test_groovy_enum_implements() {
        let source = "enum Status implements Describable { OK, KO }";
        let entities = extract_entities_groovy(source, "test.groovy", "test-repo");
        let enum_entity = pick_entity(&entities, "Status", EntityKind::GroovyEnum);
        let implements = collect_implements(&enum_entity.reference_intents);
        assert_eq!(implements, vec!["Describable"]);
    }

    #[test]
    fn test_groovy_extends_qualified_name() {
        let source = "class Foo extends nextflow.plugin.BasePlugin { }";
        let entities = extract_entities_groovy(source, "test.groovy", "test-repo");
        let cls = pick_class(&entities, "Foo");
        assert_extends(&cls.reference_intents, "nextflow.plugin.BasePlugin");
    }

    #[test]
    fn test_groovy_extends_multiline_declaration() {
        let source = "class OrderService extends BaseService<Order>\n        implements Auditable, Serializable {\n}";
        let entities = extract_entities_groovy(source, "test.groovy", "test-repo");
        let cls = pick_class(&entities, "OrderService");
        let extends = collect_extends(&cls.reference_intents);
        let implements = collect_implements(&cls.reference_intents);
        assert_eq!(extends, vec!["BaseService"]);
        assert_eq!(implements.len(), 2);
        assert!(implements.contains(&"Auditable"));
        assert!(implements.contains(&"Serializable"));
        // The line on the intent must point at the class declaration's start line.
        for intent in &cls.reference_intents {
            match intent {
                ReferenceIntent::Extends { line, .. }
                | ReferenceIntent::Implements { line, .. } => {
                    assert_eq!(*line, cls.start_line);
                }
                _ => {}
            }
        }
    }

    #[test]
    fn test_groovy_class_without_inheritance_has_no_intents() {
        let source = "class Plain { def m() {} }";
        let entities = extract_entities_groovy(source, "test.groovy", "test-repo");
        let cls = pick_class(&entities, "Plain");
        assert!(collect_extends(&cls.reference_intents).is_empty());
        assert!(collect_implements(&cls.reference_intents).is_empty());
    }

    #[test]
    fn test_groovy_extends_intent_attached_to_class_not_methods() {
        // Class with extends + a method body that contains a CALL.
        // The Extends intent must hang on the class, not on a method.
        let source = r#"
class Ext1 extends PluginExtensionPoint {
    protected void init(Object session) {
        runAnalyzer("foo")
    }
}
"#;
        let entities = extract_entities_groovy(source, "test.groovy", "test-repo");
        let cls = pick_class(&entities, "Ext1");
        assert_extends(&cls.reference_intents, "PluginExtensionPoint");
        let init = entities
            .iter()
            .find(|e| e.name == "init" && e.kind == EntityKind::GroovyMethod)
            .expect("method 'init' not extracted");
        // The method must NOT inherit its parent's Extends intent.
        assert!(
            !init
                .reference_intents
                .iter()
                .any(|r| matches!(r, ReferenceIntent::Extends { .. })),
            "method 'init' should not receive the class's Extends intent"
        );
        // The method should still have its Call intent intact.
        assert!(
            init.reference_intents.iter().any(
                |r| matches!(r, ReferenceIntent::Call { method, .. } if method == "runAnalyzer")
            ),
            "method 'init' should still have CALL to runAnalyzer"
        );
    }

    #[test]
    fn test_groovy_extends_line_number() {
        let source = "\n\nclass Foo extends Bar {\n}\n";
        let entities = extract_entities_groovy(source, "test.groovy", "test-repo");
        let cls = pick_class(&entities, "Foo");
        // The class is declared on line 3 (1-indexed).
        assert_eq!(cls.start_line, 3);
        let extends = collect_extends(&cls.reference_intents);
        assert_eq!(extends, vec!["Bar"]);
        let intent_line = cls
            .reference_intents
            .iter()
            .find_map(|r| match r {
                ReferenceIntent::Extends { line, .. } => Some(*line),
                _ => None,
            })
            .expect("expected Extends intent on Foo");
        assert_eq!(
            intent_line, cls.start_line,
            "Extends intent line must match class declaration line"
        );
    }

    #[test]
    fn test_groovy_extends_ignores_comments() {
        // The commented-out class must NOT produce any intent; only the real class does.
        let source = r#"
// class Fake extends Nope
class Real extends Base {
}
"#;
        let entities = extract_entities_groovy(source, "test.groovy", "test-repo");
        // No Fake entity should exist.
        assert!(
            !entities.iter().any(|e| e.name == "Fake"),
            "Fake should not be extracted from a comment line"
        );
        let cls = pick_class(&entities, "Real");
        assert_extends(&cls.reference_intents, "Base");
    }

    // ─────────────────────────────────────────────────────────────────────
    // Group: GroovyDoc / docstring extraction (extract_preceding_docstring)
    // ─────────────────────────────────────────────────────────────────────

    /// Helper: materialize lines and run the docstring walker against the
    /// 0-based index of the declaration line.
    fn doc_of(source: &str, decl_line_idx: usize) -> Option<String> {
        let lines: Vec<&str> = source.lines().collect();
        extract_preceding_docstring(&lines, decl_line_idx)
    }

    #[test]
    fn test_groovy_docstring_block_comment_adjacent() {
        let source = "/**\n * Channel factory initialization. This method is invoked one and only once\n *\n * @param session The current nextflow session\n */\nabstract protected void init(Session session)\n";
        let doc = doc_of(source, 5).expect("expected docstring for init");
        assert!(doc.contains("Channel factory initialization"));
        assert!(doc.contains("@param session The current nextflow session"));
        assert!(!doc.contains("/**"), "markers must be stripped: {doc:?}");
        assert!(!doc.contains("*/"), "markers must be stripped: {doc:?}");
        assert!(
            !doc.lines().any(|l| l.trim_start().starts_with('*')),
            "leading '*' must be stripped: {doc:?}"
        );
    }

    #[test]
    fn test_groovy_docstring_skips_annotations() {
        // Exact shape of the nextflow `checkInit` case: GroovyDoc, then an
        // annotation, then the declaration.
        let source = "/** doc */\n@PackageScope\nsynchronized void checkInit(Object session) {\n";
        let doc = doc_of(source, 2);
        assert_eq!(doc.as_deref(), Some("doc"));
    }

    #[test]
    fn test_groovy_docstring_skips_multiple_annotations() {
        let source = "/** doc */\n@PackageScope\n@Override\nvoid m() {\n";
        let doc = doc_of(source, 3);
        assert_eq!(doc.as_deref(), Some("doc"));
    }

    #[test]
    fn test_groovy_docstring_line_comments_burst() {
        let source = "// a\n// b\nclass Foo {\n";
        let doc = doc_of(source, 2);
        assert_eq!(doc.as_deref(), Some("a\nb"));
    }

    #[test]
    fn test_groovy_docstring_tolerates_single_blank_line() {
        let source = "/** doc */\n\nvoid m() {\n";
        let doc = doc_of(source, 2);
        assert_eq!(doc.as_deref(), Some("doc"));
    }

    #[test]
    fn test_groovy_docstring_two_blank_lines_breaks() {
        let source = "/** doc */\n\n\nvoid m() {\n";
        let doc = doc_of(source, 3);
        assert_eq!(doc, None);
    }

    #[test]
    fn test_groovy_docstring_none_when_absent() {
        let source = "void other() {\nvoid m() {\n";
        let doc = doc_of(source, 1);
        assert_eq!(doc, None);
    }

    #[test]
    fn test_groovy_docstring_stops_at_import() {
        // License header must never leak into the first class's docstring.
        let source = "/*\n * Licensed under the Apache License\n */\npackage com.example\n\nimport foo.Bar\n\nclass Foo {\n";
        let doc = doc_of(source, 7);
        assert_eq!(doc, None);
    }

    #[test]
    fn test_groovy_docstring_empty_comment_is_none() {
        let source = "/** */\nvoid m() {\n";
        assert_eq!(doc_of(source, 1), None);
        let source2 = "//\nvoid m() {\n";
        assert_eq!(doc_of(source2, 1), None);
    }

    #[test]
    fn test_groovy_docstring_first_line_of_file() {
        let source = "class Foo {\n";
        assert_eq!(doc_of(source, 0), None);
    }

    #[test]
    fn test_groovy_docstring_malformed_block_no_panic() {
        // Orphan `*/` with no visible opener: must not panic, returns None.
        let source = "*/\nclass Foo {\n";
        assert_eq!(doc_of(source, 1), None);
        // Orphan closer further down the file.
        let source2 = "package p\n\n * dangling\n */\nclass Foo {\n";
        assert_eq!(doc_of(source2, 4), None);
    }

    // ─────────────────────────────────────────────────────────────────────
    // Group: docstring wiring into extract_entities_groovy
    // ─────────────────────────────────────────────────────────────────────

    #[test]
    fn test_groovy_class_has_docstring() {
        let source = "/**\n * A service class.\n */\nclass MyService {\n}\n";
        let entities = extract_entities_groovy(source, "test.groovy", "test-repo");
        let cls = pick_class(&entities, "MyService");
        assert_eq!(cls.docstring.as_deref(), Some("A service class."));
    }

    #[test]
    fn test_groovy_abstract_method_has_docstring() {
        // Literal fragment of nextflow's PluginExtensionPoint.groovy — the exact
        // regression case: GroovyDoc on an abstract method with no body.
        let source = r#"package nextflow.plugin.extension

abstract class PluginExtensionPoint implements ExtensionPoint {

    private boolean initialised

    /**
     * Channel factory initialization. This method is invoked one and only once
     *
     * @param session The current nextflow session
     */
    abstract protected void init(Session session)
}
"#;
        let entities = extract_entities_groovy(source, "PluginExtensionPoint.groovy", "test-repo");
        let init = pick_entity(&entities, "init", EntityKind::GroovyMethod);
        let doc = init
            .docstring
            .as_deref()
            .expect("init must carry its GroovyDoc");
        assert!(doc.contains("Channel factory initialization"));
        assert!(!doc.contains("/**") && !doc.contains("*/"));
    }

    #[test]
    fn test_groovy_def_method_has_docstring() {
        let source = "class Foo {\n    /** Computes the answer. */\n    def compute() { 42 }\n}\n";
        let entities = extract_entities_groovy(source, "test.groovy", "test-repo");
        let m = pick_entity(&entities, "compute", EntityKind::GroovyMethod);
        assert_eq!(m.docstring.as_deref(), Some("Computes the answer."));
    }

    #[test]
    fn test_groovy_property_has_docstring() {
        let source = "class Foo {\n    // The default role\n    String role = \"USER\"\n}\n";
        let entities = extract_entities_groovy(source, "test.groovy", "test-repo");
        let prop = pick_entity(&entities, "role", EntityKind::GroovyProperty);
        assert_eq!(prop.docstring.as_deref(), Some("The default role"));
    }

    #[test]
    fn test_groovy_multiline_method_has_docstring() {
        // Multi-line signature (`(` without `)` on the first line): the docstring
        // must be located from the real method start line, not from the line
        // where the parser finished scanning the signature.
        let source = r#"class HttpUtil {
    /**
     * Restart the HTTP server.
     */
    private static SimpleHttpServer restartHttpServer(String id, String webRootPath,
                                                       Closure handler = {null},
                                                       Closure errorListener = {}) {
        new SimpleHttpServer()
    }
}
"#;
        let entities = extract_entities_groovy(source, "HttpUtil.groovy", "test-repo");
        let m = pick_entity(&entities, "restartHttpServer", EntityKind::GroovyMethod);
        assert_eq!(m.docstring.as_deref(), Some("Restart the HTTP server."));
    }

    #[test]
    fn test_groovy_method_without_doc_has_none() {
        // Regression: entities without a preceding comment keep docstring == None.
        let source = "class Foo {\n    int add(int a, int b) { a + b }\n}\n";
        let entities = extract_entities_groovy(source, "test.groovy", "test-repo");
        let m = pick_entity(&entities, "add", EntityKind::GroovyMethod);
        assert_eq!(m.docstring, None);
        let cls = pick_class(&entities, "Foo");
        assert_eq!(cls.docstring, None);
    }
}

#[test]
fn test_all_typed_methods_no_duplication() {
    // Both methods typed → tree-sitter finds both, ad-hoc must NOT duplicate (Fix 3: known_lines)
    let source = r#"
class HttpUtil {
    private static void restartHttpServer() {
        println "hello"
    }
    void loadIntoHttpServer(String html) {
        restartHttpServer()
    }
}
"#;
    let entities = extract_entities_groovy(source, "HttpUtil.groovy", "test-repo");
    let r_count = entities
        .iter()
        .filter(|e| e.name == "restartHttpServer")
        .count();
    let l_count = entities
        .iter()
        .filter(|e| e.name == "loadIntoHttpServer")
        .count();
    assert_eq!(r_count, 1, "restartHttpServer duplicated");
    assert_eq!(l_count, 1, "loadIntoHttpServer duplicated");
}

#[test]
fn test_def_methods_call_typed_private_method() {
    // Simulates LLM scenario: def method calling private typed method
    let source = r#"
class HttpUtil {
    private static void restartHttpServer() {
        println "hello"
    }
    def loadIntoHttpServer(String html) {
        restartHttpServer()
    }
}
"#;
    let entities = extract_entities_groovy(source, "HttpUtil.groovy", "test-repo");
    let load = entities.iter().find(|e| e.name == "loadIntoHttpServer");
    assert!(load.is_some(), "loadIntoHttpServer not found");
    let load = load.unwrap();
    let calls_to_restart = load
        .reference_intents
        .iter()
        .filter(
            |r| matches!(r, ReferenceIntent::Call { method, .. } if method == "restartHttpServer"),
        )
        .count();
    assert!(
        calls_to_restart > 0,
        "Expected def method to have CALL to restartHttpServer"
    );
}

#[test]
fn test_no_paren_call_detection() {
    // Fix 2: Groovy no-paren call style: runAnalyzer "abc", 123 and doSomething arg1
    let source = r#"
class Worker {
    void process() {
        runAnalyzer "abc", 123
        doSomething result
        println "hello"
    }
}
"#;
    let entities = extract_entities_groovy(source, "Worker.groovy", "test-repo");
    let process = entities
        .iter()
        .find(|e| e.name == "process")
        .expect("process not found");
    let refs: Vec<String> = process
        .reference_intents
        .iter()
        .map(|r| match r {
            ReferenceIntent::Call {
                method,
                receiver,
                line,
                arg_count: _,
            } => format!(
                "Call({}{}, line {})",
                receiver
                    .as_ref()
                    .map(|r| format!("{}.", r))
                    .unwrap_or_default(),
                method,
                line
            ),
            _ => format!("{:?}", r),
        })
        .collect();
    eprintln!("process reference_intents: {:?}", refs);

    // runAnalyzer "abc", 123 — no-paren call with string arg
    let has_run = process
        .reference_intents
        .iter()
        .any(|r| matches!(r, ReferenceIntent::Call { method, .. } if method == "runAnalyzer"));
    assert!(has_run);

    // doSomething result — no-paren call with identifier arg
    let has_do = process
        .reference_intents
        .iter()
        .any(|r| matches!(r, ReferenceIntent::Call { method, .. } if method == "doSomething"));
    assert!(has_do);

    // println — must NOT be captured (it's a keyword)
    let has_println = process
        .reference_intents
        .iter()
        .any(|r| matches!(r, ReferenceIntent::Call { method, .. } if method == "println"));
    assert!(!has_println);
}

#[test]
fn test_private_method_with_closure_args_is_callable() {
    // Replicates exact pattern from HttpUtil.groovy in code-history-mining:
    // private static method with closure args, called from a public static method
    let source = r#"
package test
import com.example.SimpleHttpServer
class HttpUtil {
    static String loadIntoHttpServer(String html) {
        def server = restartHttpServer("web", "/tmp", {null}, {log?.errorOnHttpRequest(it.toString())})
        "http://localhost"
    }

    private static SimpleHttpServer restartHttpServer(String id, String webRootPath,
                                                       Closure handler = {null},
                                                       Closure errorListener = {}) {
        def server = new SimpleHttpServer()
        server
    }
}
"#;
    let entities = extract_entities_groovy(source, "HttpUtil.groovy", "test-repo");

    let load = entities.iter().find(|e| e.name == "loadIntoHttpServer");
    assert!(load.is_some(), "loadIntoHttpServer not found");
    let load = load.unwrap();
    assert_eq!(
        load.enclosing_class.as_deref(),
        Some("HttpUtil"),
        "loadIntoHttpServer should have enclosing_class HttpUtil"
    );
    assert!(
        !load.fqn.is_empty(),
        "loadIntoHttpServer should have non-empty FQN, got: '{}'",
        load.fqn
    );

    let calls_restart = load
        .reference_intents
        .iter()
        .filter(
            |r| matches!(r, ReferenceIntent::Call { method, .. } if method == "restartHttpServer"),
        )
        .count();
    assert!(
        calls_restart > 0,
        "Expected loadIntoHttpServer to have CALL to restartHttpServer, but found {} call(s). refs: {:?}",
        calls_restart,
        load.reference_intents
            .iter()
            .filter_map(|r| match r {
                ReferenceIntent::Call { method, line, .. } => Some(format!("{}@L{}", method, line)),
                _ => None,
            })
            .collect::<Vec<_>>()
    );

    let restart = entities.iter().find(|e| e.name == "restartHttpServer");
    assert!(restart.is_some(), "restartHttpServer not found in entities");
    let restart = restart.unwrap();
    assert_eq!(
        restart.enclosing_class.as_deref(),
        Some("HttpUtil"),
        "restartHttpServer should have enclosing_class HttpUtil"
    );
    assert!(
        !restart.fqn.is_empty(),
        "restartHttpServer should have non-empty FQN, got: '{}'",
        restart.fqn
    );
    assert!(
        restart.enclosing_class.is_some(),
        "restartHttpServer should have enclosing_class set"
    );
}

#[test]
fn test_new_constructor_not_method_declaration() {
    // `new File(...).write(...)` and `new SimpleHttpServer()` are constructor calls,
    // NOT method declarations. They should not create spurious method entities.
    let source = r#"
class HttpUtil {
    static String loadIntoHttpServer(String html) {
        def tempDir = FileUtil.createTempDirectory("proj", "")
        new File("path").write(html)
        def server = restartHttpServer("web", "/tmp", {null}, {log?.errorOnHttpRequest(it.toString())})
        "http://localhost"
    }
    private static SimpleHttpServer restartHttpServer(String id, String webRootPath,
                                                       Closure handler = {null},
                                                       Closure errorListener = {}) {
        def server = new SimpleHttpServer()
        server
    }
}
"#;
    let entities = extract_entities_groovy(source, "HttpUtil.groovy", "test-repo");

    // `File` must NOT appear as a method entity
    assert!(
        !entities
            .iter()
            .any(|e| e.kind == EntityKind::GroovyMethod && e.name == "File"),
        "new File(...) was incorrectly extracted as a method declaration"
    );

    // `SimpleHttpServer` constructor call inside method body must NOT appear as method
    // (allow the one at the return type position in the private method signature via multi-line though)
    let ssh_methods: Vec<_> = entities
        .iter()
        .filter(|e| e.kind == EntityKind::GroovyMethod && e.name == "SimpleHttpServer")
        .collect();
    assert!(
        ssh_methods.len() <= 1,
        "new SimpleHttpServer() constructor should not create method entities, found {}: {:?}",
        ssh_methods.len(),
        ssh_methods.iter().map(|e| e.start_line).collect::<Vec<_>>()
    );

    // restartHttpServer should be callable from loadIntoHttpServer
    let load = entities
        .iter()
        .find(|e| e.name == "loadIntoHttpServer")
        .expect("loadIntoHttpServer not found");
    let calls_restart = load
        .reference_intents
        .iter()
        .filter(
            |r| matches!(r, ReferenceIntent::Call { method, .. } if method == "restartHttpServer"),
        )
        .count();
    assert!(
        calls_restart > 0,
        "loadIntoHttpServer should call restartHttpServer"
    );
}
