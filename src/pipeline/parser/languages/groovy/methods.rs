/// Tries to extract class, interface, enum, or trait declarations
/// Scans forward from `line_num` to find the matching closing `}` of the method body.
// Reserved for future method body parsing
pub(super) fn find_method_body_end(source: &str, line_num: usize) -> Option<usize> {
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
/// Tries to extract a method name from a multi-line method signature.
///
/// Handles cases like:
///   private static SimpleHttpServer restartHttpServer(String id, String webRootPath,
///                                                      Closure handler = {null},
///                                                      Closure errorListener = {}) {
///
/// where the opening `(` and closing `)` are on different lines.
// Reserved for future multiline method parsing
#[expect(
    clippy::too_many_lines,
    reason = "function is verbose but correct — extraction deferred"
)]
pub(super) fn try_extract_typed_method_multiline(
    source: &str,
    line_idx: usize,
) -> Option<(String, usize)> {
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
pub(super) fn try_extract_typed_method(line: &str) -> Option<(String, String)> {
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
pub(super) fn try_extract_def_method(line: &str) -> Option<(String, String)> {
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
