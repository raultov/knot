//! Declared-type lookup for Java call receivers.
//!
//! `source.fetchItems()` only resolves reliably when the resolver knows the
//! declared type of `source`. This module walks the scopes enclosing a call,
//! innermost first, and returns the type of the first declaration named like
//! the receiver: locals, parameters (method, constructor, lambda, catch,
//! for / enhanced-for, try-with-resources), pattern variables, fields and
//! record components.
//!
//! A declaration whose type cannot be expressed as a simple class name (`var`,
//! untyped lambda parameters, multi-catch, varargs, arrays, primitives) still
//! *shadows* outer declarations: the lookup stops there and the receiver is
//! left untouched, so an outer field never lends its type to a variable that
//! merely shares its name.

use tree_sitter::Node;

use crate::pipeline::parser::utils::node_text;

/// Result of looking a name up in one scope:
/// - `None` — the scope does not declare the name, keep walking outwards;
/// - `Some(None)` — declared here with an unknown type, stop without a type;
/// - `Some(Some(ty))` — declared here as `ty`.
type Binding = Option<Option<String>>;

/// Replace a `receiver` / `this.receiver` text with the receiver's declared
/// simple type when it is known; otherwise return the text unchanged.
pub(super) fn resolve_receiver_type_if_declared(
    call_node: Node<'_>,
    raw: &str,
    source: &[u8],
) -> String {
    let declared = if let Some(field) = raw.strip_prefix("this.") {
        if is_identifier(field) {
            innermost_class_field(call_node, field, source)
        } else {
            None
        }
    } else if raw != "this" && raw != "super" && is_identifier(raw) {
        lookup_visible_name(call_node, raw, source)
    } else {
        None
    };
    declared.unwrap_or_else(|| raw.to_string())
}

fn is_identifier(text: &str) -> bool {
    !text.is_empty()
        && text
            .chars()
            .all(|c| c.is_alphanumeric() || c == '_' || c == '$')
}

/// Walk every scope enclosing `call_node` and return the declared type of the
/// innermost declaration of `name`.
fn lookup_visible_name(call_node: Node<'_>, name: &str, source: &[u8]) -> Option<String> {
    let pos = call_node.start_byte();
    let mut via = call_node;
    let mut current = call_node.parent();
    while let Some(scope) = current {
        if let Some(binding) = binding_in_scope(scope, via, name, pos, source) {
            return binding;
        }
        via = scope;
        current = scope.parent();
    }
    None
}

/// `this.name` always denotes a member of the innermost enclosing class, so
/// only that class body is consulted.
fn innermost_class_field(call_node: Node<'_>, name: &str, source: &[u8]) -> Option<String> {
    let mut current = call_node.parent();
    while let Some(node) = current {
        if is_class_body(node.kind()) {
            return binding_in_class_body(node, name, source).flatten();
        }
        current = node.parent();
    }
    None
}

fn is_class_body(kind: &str) -> bool {
    matches!(
        kind,
        "class_body" | "interface_body" | "enum_body_declarations"
    )
}

/// Bindings `scope` introduces for `name` that are visible from `via` (the
/// child of `scope` on the path to the call) at byte offset `pos`.
fn binding_in_scope(
    scope: Node<'_>,
    via: Node<'_>,
    name: &str,
    pos: usize,
    source: &[u8],
) -> Binding {
    match scope.kind() {
        "block" | "constructor_body" => binding_in_statements(scope, name, pos, source),
        "switch_block_statement_group" => binding_in_switch_labels(scope, name, source)
            .or_else(|| binding_in_statements(scope, name, pos, source)),
        "switch_rule" => binding_in_switch_labels(scope, name, source),
        "for_statement" => binding_in_for_init(scope, name, source),
        "enhanced_for_statement" => binding_in_enhanced_for(scope, via, name, source),
        "catch_clause" => binding_in_catch(scope, name, source),
        "try_with_resources_statement" => binding_in_resources(scope, name, pos, source),
        "if_statement" | "while_statement" => binding_in_guarded_body(scope, via, name, source),
        "binary_expression" => binding_in_conjunction(scope, via, name, source),
        "lambda_expression" => binding_in_lambda_params(scope, name, source),
        "method_declaration" | "constructor_declaration" => {
            binding_in_method(scope, name, pos, source)
        }
        kind if is_class_body(kind) => binding_in_class_body(scope, name, source),
        _ => None,
    }
}

/// Simple class name of a declared type, or `None` when the type cannot
/// receive the calls the resolver links (`var`, primitives, arrays).
fn base_type_name(type_node: Node<'_>, source: &[u8]) -> Option<String> {
    if matches!(
        type_node.kind(),
        "array_type" | "integral_type" | "floating_point_type" | "boolean_type" | "void_type"
    ) {
        return None;
    }
    let raw = node_text(type_node, source);
    let without_generics = raw.split('<').next()?.trim();
    let simple = without_generics.rsplit('.').next()?.trim();
    if simple == "var" || !simple.chars().next().is_some_and(char::is_uppercase) {
        return None;
    }
    Some(simple.to_string())
}

fn typed_binding(type_node: Option<Node<'_>>, source: &[u8]) -> Option<String> {
    type_node.and_then(|t| base_type_name(t, source))
}

fn name_matches(node: Option<Node<'_>>, name: &str, source: &[u8]) -> bool {
    node.is_some_and(|n| node_text(n, source) == name)
}

/// `Type a = …, b = …;` — local variables, fields and interface constants.
/// Array declarators (`Type a[]`) bind no usable type.
fn binding_in_declaration(decl: Node<'_>, name: &str, source: &[u8]) -> Binding {
    let mut cursor = decl.walk();
    let declarator = decl
        .children_by_field_name("declarator", &mut cursor)
        .find(|d| name_matches(d.child_by_field_name("name"), name, source))?;
    if declarator.child_by_field_name("dimensions").is_some() {
        return Some(None);
    }
    let type_node = decl.child_by_field_name("type");
    if type_node.is_some_and(|t| node_text(t, source) == "var") {
        return Some(var_initializer_type(declarator, source));
    }
    Some(typed_binding(type_node, source))
}

/// `var x = new Foo<>(…)`: the only `var` initializer whose type is obvious
/// without inference (issue #42). Any other initializer (`Foo.create()`,
/// `a.b()`) leaves the type unknown.
fn var_initializer_type(declarator: Node<'_>, source: &[u8]) -> Option<String> {
    let value = declarator.child_by_field_name("value")?;
    if value.kind() != "object_creation_expression" {
        return None;
    }
    typed_binding(value.child_by_field_name("type"), source)
}

/// Locals declared by the statements of a block before `pos`.
fn binding_in_statements(block: Node<'_>, name: &str, pos: usize, source: &[u8]) -> Binding {
    let mut cursor = block.walk();
    block
        .named_children(&mut cursor)
        .take_while(|stmt| stmt.start_byte() < pos)
        .filter(|stmt| stmt.kind() == "local_variable_declaration")
        .filter_map(|stmt| binding_in_declaration(stmt, name, source))
        .last()
}

fn binding_in_for_init(for_stmt: Node<'_>, name: &str, source: &[u8]) -> Binding {
    let mut cursor = for_stmt.walk();
    for_stmt
        .children_by_field_name("init", &mut cursor)
        .filter(|init| init.kind() == "local_variable_declaration")
        .find_map(|init| binding_in_declaration(init, name, source))
}

/// `for (Type x : iterable)` — `x` is only in scope inside the loop body, not
/// in the iterable expression.
fn binding_in_enhanced_for(
    for_stmt: Node<'_>,
    via: Node<'_>,
    name: &str,
    source: &[u8],
) -> Binding {
    let in_body = for_stmt
        .child_by_field_name("body")
        .is_some_and(|body| body.id() == via.id());
    if !in_body || !name_matches(for_stmt.child_by_field_name("name"), name, source) {
        return None;
    }
    Some(typed_binding(for_stmt.child_by_field_name("type"), source))
}

/// `catch (Type x)` binds `Type`; a multi-catch `catch (A | B x)` binds an
/// unknown type.
fn binding_in_catch(catch_clause: Node<'_>, name: &str, source: &[u8]) -> Binding {
    let mut cursor = catch_clause.walk();
    let param = catch_clause
        .named_children(&mut cursor)
        .find(|c| c.kind() == "catch_formal_parameter")?;
    if !name_matches(param.child_by_field_name("name"), name, source) {
        return None;
    }
    let mut type_cursor = param.walk();
    let catch_type = param
        .named_children(&mut type_cursor)
        .find(|c| c.kind() == "catch_type")?;
    let mut alt_cursor = catch_type.walk();
    let alternatives: Vec<Node<'_>> = catch_type.named_children(&mut alt_cursor).collect();
    match alternatives.as_slice() {
        [single] => Some(base_type_name(*single, source)),
        _ => Some(None),
    }
}

/// `try (Type x = …; …)` — resources declared before `pos`.
fn binding_in_resources(try_stmt: Node<'_>, name: &str, pos: usize, source: &[u8]) -> Binding {
    let spec = try_stmt.child_by_field_name("resources")?;
    let mut cursor = spec.walk();
    spec.named_children(&mut cursor)
        .take_while(|res| res.start_byte() < pos)
        .filter(|res| name_matches(res.child_by_field_name("name"), name, source))
        .map(|res| typed_binding(res.child_by_field_name("type"), source))
        .last()
}

/// `case Type x ->` / `case Type x:` pattern labels.
fn binding_in_switch_labels(group: Node<'_>, name: &str, source: &[u8]) -> Binding {
    let mut cursor = group.walk();
    group
        .named_children(&mut cursor)
        .filter(|c| c.kind() == "switch_label")
        .find_map(|label| pattern_binding(label, name, None, source))
}

/// `if (o instanceof Type x) { … }` / `while (…) { … }`: pattern variables of
/// the condition are in scope inside the guarded body.
fn binding_in_guarded_body(stmt: Node<'_>, via: Node<'_>, name: &str, source: &[u8]) -> Binding {
    let guarded = ["consequence", "body"]
        .iter()
        .filter_map(|field| stmt.child_by_field_name(field))
        .any(|body| body.id() == via.id());
    if !guarded {
        return None;
    }
    pattern_binding(stmt.child_by_field_name("condition")?, name, None, source)
}

/// `o instanceof Type x && x.run()` — the right operand of `&&` sees the
/// pattern variables of the left operand.
fn binding_in_conjunction(expr: Node<'_>, via: Node<'_>, name: &str, source: &[u8]) -> Binding {
    let is_and = expr
        .child_by_field_name("operator")
        .is_some_and(|op| op.kind() == "&&");
    let via_right = expr
        .child_by_field_name("right")
        .is_some_and(|right| right.id() == via.id());
    if !is_and || !via_right {
        return None;
    }
    pattern_binding(expr.child_by_field_name("left")?, name, None, source)
}

/// Pattern variables (`instanceof Type x`, `case Type x`, record pattern
/// components) named `name` inside `node`, ignoring nested class bodies.
/// With `before = Some(pos)` only patterns starting before `pos` count.
fn pattern_binding(node: Node<'_>, name: &str, before: Option<usize>, source: &[u8]) -> Binding {
    if before.is_some_and(|pos| node.start_byte() >= pos) || is_class_body(node.kind()) {
        return None;
    }
    if let Some(binding) = pattern_declared_here(node, name, source) {
        return Some(binding);
    }
    let mut cursor = node.walk();
    let children: Vec<Node<'_>> = node.named_children(&mut cursor).collect();
    children
        .into_iter()
        .find_map(|child| pattern_binding(child, name, before, source))
}

fn pattern_declared_here(node: Node<'_>, name: &str, source: &[u8]) -> Binding {
    match node.kind() {
        "instanceof_expression" if name_matches(node.child_by_field_name("name"), name, source) => {
            Some(typed_binding(node.child_by_field_name("right"), source))
        }
        "type_pattern" => {
            let mut cursor = node.walk();
            let parts: Vec<Node<'_>> = node.named_children(&mut cursor).collect();
            match parts.as_slice() {
                [ty, ident] if node_text(*ident, source) == name => {
                    Some(base_type_name(*ty, source))
                }
                _ => None,
            }
        }
        _ => None,
    }
}

/// `x -> …`, `(x, y) -> …` bind unknown types; `(Type x) -> …` binds `Type`.
fn binding_in_lambda_params(lambda: Node<'_>, name: &str, source: &[u8]) -> Binding {
    let params = lambda.child_by_field_name("parameters")?;
    match params.kind() {
        "identifier" => (node_text(params, source) == name).then_some(None),
        "inferred_parameters" => {
            let mut cursor = params.walk();
            let declared = params
                .named_children(&mut cursor)
                .any(|p| node_text(p, source) == name);
            declared.then_some(None)
        }
        "formal_parameters" => binding_in_formal_parameters(params, name, source),
        _ => None,
    }
}

/// Method / constructor parameters, then — conservatively — any pattern
/// variable of the body declared before the call. Flow-scoped patterns
/// (`if (!(o instanceof T x)) return; x.run();`) are not modelled precisely,
/// so they only shadow outer declarations without lending their type.
fn binding_in_method(method: Node<'_>, name: &str, pos: usize, source: &[u8]) -> Binding {
    if let Some(params) = method.child_by_field_name("parameters")
        && let Some(binding) = binding_in_formal_parameters(params, name, source)
    {
        return Some(binding);
    }
    let body = method.child_by_field_name("body")?;
    pattern_binding(body, name, Some(pos), source).map(|_| None)
}

/// `(Type a, Type... rest)` — varargs bind an array, i.e. an unknown type.
fn binding_in_formal_parameters(params: Node<'_>, name: &str, source: &[u8]) -> Binding {
    let mut cursor = params.walk();
    let children: Vec<Node<'_>> = params.named_children(&mut cursor).collect();
    children.into_iter().find_map(|param| match param.kind() {
        "formal_parameter" if name_matches(param.child_by_field_name("name"), name, source) => {
            if param.child_by_field_name("dimensions").is_some() {
                Some(None)
            } else {
                Some(typed_binding(param.child_by_field_name("type"), source))
            }
        }
        "spread_parameter" => {
            let mut inner = param.walk();
            let declares = param
                .named_children(&mut inner)
                .filter(|c| c.kind() == "variable_declarator")
                .any(|d| name_matches(d.child_by_field_name("name"), name, source));
            declares.then_some(None)
        }
        _ => None,
    })
}

/// Fields and interface constants of a class body, plus the components of
/// the record that owns it.
fn binding_in_class_body(body: Node<'_>, name: &str, source: &[u8]) -> Binding {
    let mut cursor = body.walk();
    let member = body
        .named_children(&mut cursor)
        .filter(|m| matches!(m.kind(), "field_declaration" | "constant_declaration"))
        .find_map(|m| binding_in_declaration(m, name, source));
    if member.is_some() {
        return member;
    }
    let record = body.parent().filter(|p| p.kind() == "record_declaration")?;
    binding_in_formal_parameters(record.child_by_field_name("parameters")?, name, source)
}
