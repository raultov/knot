//! Parameter-list parsing for overload-aware `OVERRIDES` linking (issue #30).
//!
//! Method signatures reach the resolver as raw source text whose shape depends
//! on the parser: `(List<Setting> items)` for Java, `(value: T)` for Kotlin,
//! `(string message)` for C#, and `String render(String item)` for Groovy
//! (return type and name included). This module extracts the parameter list,
//! splits it on top-level commas and reduces every parameter to a comparable
//! [`TypeShape`], so overloads sharing a name are told apart by arity and, when
//! the arity is shared, by parameter types.
//!
//! The comparison is deliberately lenient: anything it cannot read (untyped
//! Groovy parameters, type variables, wildcards) matches everything, so a
//! parsing gap can only keep an edge, never drop a real override.

/// Leading parameter modifiers that carry no type information.
const PARAM_MODIFIERS: &[&str] = &[
    "final",
    "vararg",
    "params",
    "ref",
    "out",
    "in",
    "this",
    "scoped",
    "readonly",
    "crossinline",
    "noinline",
    "def",
];

/// Modifiers that make the parameter an implicit array. C# `params` is absent:
/// its array type is spelled out (`params int[] values`).
const VARIADIC_MODIFIERS: &[&str] = &["vararg"];

/// A parameter type reduced to what overriding compares: the simple base name
/// (package/namespace qualifiers dropped), its generic arguments and its array
/// dimensions (`T[]` and `T...` both count as one dimension).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct TypeShape {
    base: String,
    args: Vec<TypeShape>,
    array_dims: usize,
}

/// One declared parameter. `ty` is `None` for untyped parameters (Groovy `def`
/// or bare names), which match any type.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Param {
    ty: Option<TypeShape>,
    has_default: bool,
}

/// The parsed parameter list of a method signature.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ParamList {
    params: Vec<Param>,
    /// Whether default values let callers omit trailing parameters, making the
    /// method answer for several arities. True for Groovy, which compiles one
    /// overload per omittable parameter; Java has no defaults and Kotlin/C#
    /// overrides must repeat the full parameter list.
    defaults_expand_arity: bool,
}

impl ParamList {
    /// Parses the parameter list of `signature`, anchoring on `method_name`
    /// when the signature also carries a return type or annotations (Groovy).
    /// Returns `None` when no balanced parameter list can be found.
    pub(super) fn parse(
        signature: &str,
        method_name: &str,
        defaults_expand_arity: bool,
    ) -> Option<Self> {
        let body = parameter_list_body(signature, method_name)?;
        let params = split_top_level(body, ',')
            .into_iter()
            .filter_map(parse_param)
            .collect();
        Some(Self {
            params,
            defaults_expand_arity,
        })
    }

    /// The `(min, max)` number of arguments the method accepts.
    fn arity_range(&self) -> (usize, usize) {
        let max = self.params.len();
        if self.defaults_expand_arity {
            let required = self.params.iter().filter(|p| !p.has_default).count();
            (required, max)
        } else {
            (max, max)
        }
    }
}

/// How many arguments a declared signature accepts at a call site.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::pipeline::ingest::resolve) enum DeclaredArity {
    /// Exactly this many arguments.
    Fixed(usize),
    /// A range: a variadic parameter (`String... parts`, `vararg`, `params`,
    /// `...rest`, `*args`) or an omittable one (default value, TS `x?`).
    Variable,
    /// No balanced parameter list was found.
    Unparsed,
}

/// The call-site arity `signature` declares, used as the target arity of
/// call resolution. Unlike counting commas, generic arguments
/// (`Map<String, Integer>`) and annotation arguments (`@Named("a", b)`) do
/// not add parameters.
pub(in crate::pipeline::ingest::resolve) fn declared_arity(
    signature: &str,
    method_name: &str,
) -> DeclaredArity {
    let Some(body) = parameter_list_body(signature, method_name) else {
        return DeclaredArity::Unparsed;
    };
    let params: Vec<&str> = split_top_level(body, ',')
        .into_iter()
        .map(str::trim)
        .filter(|p| !p.is_empty() && *p != "void")
        .collect();
    if params.iter().any(|p| accepts_variable_arity(p)) {
        DeclaredArity::Variable
    } else {
        DeclaredArity::Fixed(params.len())
    }
}

/// `true` for a parameter that is variadic or may be omitted.
fn accepts_variable_arity(raw: &str) -> bool {
    let decl = strip_annotations(raw).trim();
    let first_word = decl.split_whitespace().next().unwrap_or("");
    decl.contains("...")
        || decl.starts_with('*')
        || matches!(first_word, "vararg" | "params")
        || parse_param(raw).is_some_and(|p| p.has_default)
        || find_top_level(decl, |_, c, next| c == '?' && next == ':').is_some()
}

/// Matches the parameters of an overriding method (`sub`) against a supertype
/// declaration (`sup`). Returns `None` when the arities or a parameter type
/// rule the override out, otherwise a specificity score: the number of type
/// names that matched exactly (type variables and untyped parameters match
/// with no score), used to prefer `put(Map<String, T>)` over `put(T)` for an
/// override declared as `put(Map<String, Setting>)`.
pub(super) fn match_params(sub: &ParamList, sup: &ParamList) -> Option<usize> {
    let (sub_min, sub_max) = sub.arity_range();
    let (sup_min, sup_max) = sup.arity_range();
    if sub_min.max(sup_min) > sub_max.min(sup_max) {
        return None;
    }
    sub.params
        .iter()
        .zip(&sup.params)
        .try_fold(0, |score, (a, b)| match (&a.ty, &b.ty) {
            (Some(a), Some(b)) => match_type(a, b).map(|s| score + s),
            _ => Some(score),
        })
}

fn match_type(sub: &TypeShape, sup: &TypeShape) -> Option<usize> {
    if is_type_variable(&sup.base) || is_type_variable(&sub.base) {
        return Some(0);
    }
    if sub.base != sup.base || sub.array_dims != sup.array_dims {
        return None;
    }
    if sub.args.len() != sup.args.len() {
        // Raw type against a parameterized one: the base name decides.
        return Some(1);
    }
    sub.args
        .iter()
        .zip(&sup.args)
        .try_fold(1, |score, (a, b)| match_type(a, b).map(|s| score + s))
}

/// Heuristic for type variables and wildcards, which the signature alone
/// cannot distinguish from real types: `?`/`*` wildcards, short all-caps names
/// (`T`, `K`, `V`, `T1`) and the C# `TName` convention (`TKey`, `TEntity`).
/// A false positive only makes matching more lenient.
fn is_type_variable(base: &str) -> bool {
    if base == "?" || base == "*" {
        return true;
    }
    let mut chars = base.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !first.is_ascii_uppercase() {
        return false;
    }
    let rest: Vec<char> = chars.collect();
    if rest.len() <= 1
        && rest
            .iter()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
    {
        return true;
    }
    first == 'T'
        && rest.first().is_some_and(char::is_ascii_uppercase)
        && rest.iter().any(char::is_ascii_lowercase)
}

fn is_ident_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '$'
}

/// Locates the `(` opening the parameter list: the start of the signature for
/// Java/Kotlin/C#, or the `(` right after the method name for Groovy, whose
/// signature may also carry annotations with their own parentheses.
fn open_paren_index(signature: &str, method_name: &str) -> Option<usize> {
    let trimmed = signature.trim_start();
    if trimmed.starts_with('(') {
        return Some(signature.len() - trimmed.len());
    }
    if !method_name.is_empty() {
        let mut from = 0;
        while let Some(rel) = signature[from..].find(method_name) {
            let at = from + rel;
            let after = signature[at + method_name.len()..].trim_start();
            let boundary = signature[..at]
                .chars()
                .next_back()
                .is_none_or(|c| !is_ident_char(c));
            if boundary && after.starts_with('(') {
                return Some(signature.len() - after.len());
            }
            from = at + method_name.len();
        }
    }
    signature.find('(')
}

/// The text between the parameter list's parentheses.
fn parameter_list_body<'a>(signature: &'a str, method_name: &str) -> Option<&'a str> {
    let open = open_paren_index(signature, method_name)?;
    let inner_start = open + 1;
    let mut close = None;
    scan_top_level(&signature[inner_start..], |i, c| {
        if c == ')' {
            close = Some(inner_start + i);
            return false;
        }
        true
    });
    close.map(|end| &signature[inner_start..end])
}

/// Visits every character of `text` that sits at nesting depth zero, outside
/// string/char literals and outside `()`, `[]`, `{}` and generic `<>`
/// brackets. An unmatched closing bracket is reported too (it closes an
/// enclosing scope). `visit` returns `false` to stop the scan.
///
/// `->` and `=>` arrows never close a generic. Once a depth-zero `=` has been
/// seen (a default value), `<` stops opening generics until the next
/// depth-zero `,`, so comparisons in default expressions cannot swallow the
/// rest of the list.
fn scan_top_level(text: &str, mut visit: impl FnMut(usize, char) -> bool) {
    let mut stack: Vec<char> = Vec::new();
    let mut in_default = false;
    let mut quote: Option<char> = None;
    let mut escaped = false;
    let mut prev = '\0';
    for (i, c) in text.char_indices() {
        if let Some(q) = quote {
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == q {
                quote = None;
            }
            prev = c;
            continue;
        }
        if stack.is_empty() && !visit(i, c) {
            return;
        }
        match c {
            '"' | '\'' => quote = Some(c),
            '(' | '[' | '{' => stack.push(c),
            '<' if !in_default => stack.push(c),
            '>' if prev != '-' && prev != '=' => {
                if stack.last() == Some(&'<') {
                    stack.pop();
                }
            }
            ')' | ']' | '}' => {
                let opener = match c {
                    ')' => '(',
                    ']' => '[',
                    _ => '{',
                };
                while let Some(top) = stack.pop() {
                    if top == opener {
                        break;
                    }
                }
            }
            '=' if stack.is_empty() => in_default = true,
            ',' if stack.is_empty() => in_default = false,
            _ => {}
        }
        prev = c;
    }
}

/// Splits `text` on `sep` occurrences at nesting depth zero.
fn split_top_level(text: &str, sep: char) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut start = 0;
    scan_top_level(text, |i, c| {
        if c == sep {
            parts.push(&text[start..i]);
            start = i + c.len_utf8();
        }
        true
    });
    parts.push(&text[start..]);
    parts
}

/// Index of the first depth-zero character accepted by `pred`, which also
/// receives the previous and next characters.
fn find_top_level(text: &str, pred: impl Fn(char, char, char) -> bool) -> Option<usize> {
    let mut found = None;
    scan_top_level(text, |i, c| {
        let prev = text[..i].chars().next_back().unwrap_or('\0');
        let next = text[i + c.len_utf8()..].chars().next().unwrap_or('\0');
        if pred(prev, c, next) {
            found = Some(i);
            return false;
        }
        true
    });
    found
}

/// Removes leading Java/Kotlin annotations (`@NonNull`, `@Size(max = 3)`) and
/// C# attributes (`[FromBody]`).
fn strip_annotations(mut text: &str) -> &str {
    loop {
        text = text.trim_start();
        if let Some(rest) = text.strip_prefix('@') {
            let name_len = rest
                .find(|c: char| !(is_ident_char(c) || c == '.'))
                .unwrap_or(rest.len());
            text = &rest[name_len..];
            let after = text.trim_start();
            if after.starts_with('(') {
                text = skip_balanced(after);
            }
        } else if text.starts_with('[') {
            text = skip_balanced(text);
        } else {
            return text;
        }
    }
}

/// Skips a balanced bracket group starting at the beginning of `text`.
fn skip_balanced(text: &str) -> &str {
    let open_len = text.chars().next().map_or(0, char::len_utf8);
    let inner = &text[open_len..];
    let mut end = inner.len();
    scan_top_level(inner, |i, c| {
        if matches!(c, ')' | ']' | '}') {
            end = i + c.len_utf8();
            return false;
        }
        true
    });
    &inner[end..]
}

/// Removes leading modifiers, reporting whether one made the parameter variadic.
fn strip_modifiers(mut text: &str) -> (&str, bool) {
    let mut variadic = false;
    loop {
        text = text.trim_start();
        let word_len = text.find(char::is_whitespace).unwrap_or(text.len());
        let word = &text[..word_len];
        if word_len < text.len() && PARAM_MODIFIERS.contains(&word) {
            variadic |= VARIADIC_MODIFIERS.contains(&word);
            text = &text[word_len..];
        } else {
            return (text, variadic);
        }
    }
}

fn parse_param(raw: &str) -> Option<Param> {
    let text = raw.trim();
    if text.is_empty() {
        return None;
    }
    let default_at = find_top_level(text, |prev, c, next| {
        c == '=' && !matches!(prev, '=' | '!' | '<' | '>') && !matches!(next, '=' | '>')
    });
    let decl = default_at.map_or(text, |i| &text[..i]);
    let (decl, variadic) = strip_modifiers(strip_annotations(decl));
    let decl = strip_annotations(decl).trim();
    let type_text =
        match find_top_level(decl, |prev, c, next| c == ':' && prev != ':' && next != ':') {
            // Kotlin: `name: Type`.
            Some(colon) => Some(&decl[colon + 1..]),
            // Java/C#/Groovy: `Type name`; a lone token is an untyped Groovy name.
            None => find_last_top_level_whitespace(decl).map(|ws| &decl[..ws]),
        };
    Some(Param {
        ty: type_text.and_then(|t| parse_type(t, usize::from(variadic))),
        has_default: default_at.is_some(),
    })
}

fn find_last_top_level_whitespace(text: &str) -> Option<usize> {
    let mut last = None;
    scan_top_level(text, |i, c| {
        if c.is_whitespace() {
            last = Some(i);
        }
        true
    });
    last
}

fn parse_type(text: &str, mut array_dims: usize) -> Option<TypeShape> {
    let mut t = text.trim();
    // Kotlin projections and Java bounded wildcards inside generic arguments.
    for variance in ["out ", "in "] {
        t = t.strip_prefix(variance).unwrap_or(t).trim_start();
    }
    if t.starts_with('?') {
        return Some(TypeShape {
            base: "?".to_string(),
            args: Vec::new(),
            array_dims,
        });
    }
    loop {
        t = t.trim_end().trim_end_matches(['?', '!']).trim_end();
        if let Some(rest) = t.strip_suffix("[]") {
            t = rest;
        } else if let Some(rest) = t.strip_suffix("...") {
            t = rest;
        } else {
            break;
        }
        array_dims += 1;
    }
    if t.is_empty() {
        return None;
    }
    if t.starts_with('(') {
        // Kotlin function type or C# tuple: compared verbatim.
        return Some(TypeShape {
            base: t.split_whitespace().collect(),
            args: Vec::new(),
            array_dims,
        });
    }
    let (base, args) = match (t.find('<'), t.strip_suffix('>')) {
        (Some(lt), Some(without_gt)) => (
            &t[..lt],
            split_top_level(&without_gt[lt + 1..], ',')
                .into_iter()
                .filter_map(|arg| parse_type(arg, 0))
                .collect(),
        ),
        _ => (t, Vec::new()),
    };
    let base: String = base.split_whitespace().collect();
    let simple = base.rsplit('.').next().unwrap_or(&base).to_string();
    Some(TypeShape {
        base: simple,
        args,
        array_dims,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(signature: &str, name: &str) -> ParamList {
        ParamList::parse(signature, name, false).expect("parameter list")
    }

    fn arity(signature: &str, name: &str) -> usize {
        parse(signature, name).params.len()
    }

    /// Issue #35: the call-site arity used by call resolution.
    #[test]
    fn declared_arity_counts_fixed_parameters() {
        use DeclaredArity::Fixed;
        for (signature, expected) in [
            ("()", Fixed(0)),
            ("(void)", Fixed(0)),
            ("(int a, int b)", Fixed(2)),
            ("(Map<String, Integer> values)", Fixed(1)),
            (
                "(@Named(\"a\") int a, @Named(\"b\") int b, int c)",
                Fixed(3),
            ),
            ("(value: T, other: Map<K, V>)", Fixed(2)),
            ("(string message, int priority)", Fixed(2)),
            ("(final List<String>[] items)", Fixed(1)),
        ] {
            assert_eq!(declared_arity(signature, "m"), expected, "{signature}");
        }
    }

    #[test]
    fn declared_arity_detects_variable_parameters() {
        for signature in [
            "(String... parts)",
            "(String fmt, Object... args)",
            "(vararg values: Int)",
            "(params int[] values)",
            "(name: String = \"x\")",
            "(int a, int b = 2)",
            "(x?: number)",
            "(...rest: string[])",
            "(self, *args, **kwargs)",
        ] {
            assert_eq!(
                declared_arity(signature, "m"),
                DeclaredArity::Variable,
                "{signature}"
            );
        }
    }

    #[test]
    fn declared_arity_unparsed_without_parameter_list() {
        assert_eq!(declared_arity("m", "m"), DeclaredArity::Unparsed);
    }

    fn bases(signature: &str, name: &str) -> Vec<Option<String>> {
        parse(signature, name)
            .params
            .into_iter()
            .map(|p| p.ty.map(|t| t.base))
            .collect()
    }

    fn matches(sub: &str, sup: &str) -> Option<usize> {
        match_params(&parse(sub, "m"), &parse(sup, "m"))
    }

    #[test]
    fn arity_of_each_parser_shape() {
        assert_eq!(arity("()", "m"), 0);
        assert_eq!(arity("(  )", "m"), 0);
        assert_eq!(arity("(List<Setting> items)", "m"), 1);
        assert_eq!(
            arity("(List<Setting> items, boolean skipValidation)", "m"),
            2
        );
        assert_eq!(arity("(value: T, width: Int)", "m"), 2);
        assert_eq!(arity("(string message, int priority)", "m"), 2);
        assert_eq!(
            arity("String renderItem(String item, int width)", "renderItem"),
            2
        );
    }

    #[test]
    fn generic_arguments_do_not_split_parameters() {
        assert_eq!(arity("(Map<String, List<Integer>> index, int n)", "m"), 2);
        assert_eq!(
            arity(
                "Map<String, Object> build(Map<String, Object> item)",
                "build"
            ),
            1
        );
    }

    #[test]
    fn function_types_count_as_one_parameter() {
        assert_eq!(arity("(handler: (String, Int) -> Unit)", "m"), 1);
        assert_eq!(
            arity("(name: String, handler: (String, Map<K, V>) -> Unit)", "m"),
            2
        );
        assert_eq!(arity("(Func<int, string> map, Action<int> done)", "m"), 2);
    }

    #[test]
    fn annotations_and_attributes_are_ignored() {
        assert_eq!(
            bases(
                "(@Size(min = 1, max = 3) List<Setting> items, @NonNull String key)",
                "m"
            ),
            vec![Some("List".to_string()), Some("String".to_string())]
        );
        assert_eq!(
            bases(
                "([FromBody] UserDto dto, [FromQuery(Name = \"a,b\")] int page)",
                "m"
            ),
            vec![Some("UserDto".to_string()), Some("int".to_string())]
        );
        assert_eq!(
            arity(
                "@SuppressWarnings(\"a, b\") String layoutRow(String row)",
                "layoutRow"
            ),
            1
        );
    }

    #[test]
    fn modifiers_and_variadics() {
        let list = parse("(final List<Setting> items, String... names)", "m");
        assert_eq!(list.params.len(), 2);
        let names = list.params[1].ty.as_ref().unwrap();
        assert_eq!((names.base.as_str(), names.array_dims), ("String", 1));

        let kotlin = parse("(vararg items: String)", "m");
        let items = kotlin.params[0].ty.as_ref().unwrap();
        assert_eq!((items.base.as_str(), items.array_dims), ("String", 1));

        let csharp = parse("(this string s, params int[] values, ref int count)", "m");
        let shapes: Vec<_> = csharp
            .params
            .iter()
            .map(|p| {
                let t = p.ty.as_ref().unwrap();
                (t.base.clone(), t.array_dims)
            })
            .collect();
        assert_eq!(
            shapes,
            vec![
                ("string".to_string(), 0),
                ("int".to_string(), 1),
                ("int".to_string(), 0)
            ]
        );
    }

    #[test]
    fn default_values_with_commas_and_comparisons() {
        let list = ParamList::parse(
            "def render(String row, int indent = 2, Closure c = { a, b -> a < b }, String sep = \",\")",
            "render",
            true,
        )
        .unwrap();
        assert_eq!(list.params.len(), 4);
        assert_eq!(list.arity_range(), (1, 4));
        // Kotlin/C# defaults never expand the arity of an override.
        assert_eq!(parse("(a: Int, b: Int = 0)", "m").arity_range(), (2, 2));
    }

    #[test]
    fn generics_after_a_default_value_still_nest() {
        let list = ParamList::parse(
            "void put(int n = 1, Map<String, Object> extra)",
            "put",
            true,
        )
        .unwrap();
        assert_eq!(list.params.len(), 2);
        assert_eq!(list.params[1].ty.as_ref().unwrap().args.len(), 2);
    }

    #[test]
    fn untyped_groovy_parameters_have_no_type() {
        assert_eq!(bases("def run(def item, name)", "run"), vec![None, None]);
    }

    #[test]
    fn qualified_and_nullable_types_reduce_to_the_simple_name() {
        assert_eq!(
            bases("(java.util.List<String> a, b: String?, int? c)", "m"),
            vec![
                Some("List".to_string()),
                Some("String".to_string()),
                Some("int".to_string())
            ]
        );
    }

    #[test]
    fn method_name_anchor_requires_a_word_boundary() {
        // `prerender(` must not be mistaken for the parameter list of `render`.
        assert_eq!(
            arity("@Hook(prerender(1, 2)) String render(String row)", "render"),
            1
        );
    }

    #[test]
    fn unbalanced_signature_is_rejected() {
        assert!(ParamList::parse("(String a", "m", false).is_none());
        assert!(ParamList::parse("no parameters here", "m", false).is_none());
    }

    #[test]
    fn arity_mismatch_is_incompatible() {
        assert_eq!(
            matches("(List<Setting> a, boolean b)", "(List<Setting> a)"),
            None
        );
        assert!(
            matches(
                "(List<Setting> a, boolean b)",
                "(List<Setting> x, boolean y)"
            )
            .is_some()
        );
    }

    #[test]
    fn groovy_default_parameters_accept_the_shorter_override() {
        let sub = ParamList::parse("String layoutRow(String row)", "layoutRow", true).unwrap();
        let sup = ParamList::parse(
            "String layoutRow(String row, int indent = 2)",
            "layoutRow",
            true,
        )
        .unwrap();
        assert!(match_params(&sub, &sup).is_some());
    }

    #[test]
    fn different_types_are_incompatible() {
        assert_eq!(matches("(String value)", "(Long value)"), None);
        assert_eq!(matches("(int[] values)", "(int values)"), None);
    }

    #[test]
    fn type_variables_match_concrete_types_with_lower_specificity() {
        let via_variable = matches("(Map<String, Setting> entries)", "(T entry)");
        let via_map = matches("(Map<String, Setting> entries)", "(Map<String, T> entries)");
        assert_eq!(via_variable, Some(0));
        assert!(via_map.unwrap() > via_variable.unwrap());
        assert_eq!(matches("(Setting entry)", "(Map<String, T> entries)"), None);
        assert_eq!(matches("(values: List<Int>)", "(values: List<T>)"), Some(1));
        assert_eq!(matches("(TKey key)", "(string key)"), Some(0));
    }

    #[test]
    fn raw_types_match_parameterized_ones_on_the_base_name() {
        assert_eq!(matches("(List items)", "(List<String> items)"), Some(1));
    }

    #[test]
    fn untyped_parameters_match_anything() {
        let sub = ParamList::parse("def run(item)", "run", true).unwrap();
        let sup = ParamList::parse("void run(String item)", "run", true).unwrap();
        assert_eq!(match_params(&sub, &sup), Some(0));
    }

    #[test]
    fn type_variable_heuristic() {
        for var in ["T", "K", "V", "T1", "TKey", "TEntity", "?", "*"] {
            assert!(is_type_variable(var), "{var} should be a type variable");
        }
        for concrete in ["String", "Int", "Thing", "Setting", "int", "List", "URLs"] {
            assert!(!is_type_variable(concrete), "{concrete} is concrete");
        }
    }
}
