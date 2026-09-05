use super::utils::{is_valid_type_name, strip_balanced_generics};

use crate::models::{EntityKind, ReferenceIntent};

// Reserved for future type declaration parsing
pub(super) fn try_extract_type_declaration(line: &str) -> Option<(String, EntityKind)> {
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
pub(super) fn build_type_declaration(source: &str, line_idx: usize) -> Option<String> {
    const MAX_LOOKAHEAD: usize = 5;
    let lines: Vec<&str> = source.lines().collect();
    let mut buf = String::new();
    for offset in 0..MAX_LOOKAHEAD {
        let raw = lines.get(line_idx + offset)?.trim();
        if raw.is_empty() {
            buf.push(' ');
            continue;
        }
        // Skip pure comment / Javadoc continuations (mirrors the main loop's policy).
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
///   are stripped before tokenization so the `extends Comparable` token never
///   reaches the matcher.
/// - Generic arguments on the parent (`extends AbstractRepo<Order, Long>`) are
///   also stripped so resolution receives just the simple/FQN name.
/// - Declarations with embedded block comments on the same line are out of
///   scope — same robustness bar as the rest of the lexical parser.
pub(super) fn extract_inheritance_intents(
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
