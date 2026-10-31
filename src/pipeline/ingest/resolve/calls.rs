use std::collections::{HashMap, HashSet, VecDeque};
use uuid::Uuid;

use super::context::{ResolutionContext, simple_type_name};
use crate::models::{CallIntent, EntityKind};

/// Re-pick the overload of an already resolved method when its arity does
/// not match the call's argument count.
///
/// Overloads share one FQN, so every FQN-keyed lookup returns a single,
/// arbitrary overload (issue #35). The candidates are, in order:
///
/// 1. an overload sharing `fqn_uuid`'s FQN (same class, same name) with
///    exactly the call's arity;
/// 2. when `fqn_uuid`'s declared arity contradicts the call, an overload of
///    that FQN with variable arity (varargs, default parameters);
/// 3. same-named methods of that class's supertypes with the call's arity,
///    nearest first — only in languages where overloads span the class
///    hierarchy (Java, Kotlin, Groovy, C#); elsewhere a subclass method
///    replaces the inherited one whatever its arity.
///
/// Otherwise `fqn_uuid` is kept: a same-name/same-arity method of an
/// unrelated class is never a better answer than the receiver's own method.
pub(crate) fn disambiguate_overload(
    fqn_uuid: Uuid,
    intent: &CallIntent,
    ctx: &ResolutionContext,
) -> Uuid {
    let (Some(arg_count), Some(ac_map)) = (intent.arg_count, ctx.uuid_to_arg_count) else {
        return fqn_uuid;
    };
    if ac_map.get(&fqn_uuid) == Some(&arg_count) {
        return fqn_uuid;
    }
    let Some(fqn) = ctx.uuid_to_fqn.and_then(|fqns| fqns.get(&fqn_uuid)) else {
        return fqn_uuid;
    };
    let matches_arity = |uuid: &Uuid| ac_map.get(uuid) == Some(&arg_count);
    let overloads = overloads_of(fqn, &fqn_uuid, ctx);

    if let Some(&overload) = overloads.iter().find(|u| matches_arity(u)) {
        return overload;
    }
    if ac_map.contains_key(&fqn_uuid)
        && let Some(&variable) = overloads.iter().find(|u| !ac_map.contains_key(u))
    {
        return variable;
    }
    let overloads_span_hierarchy = ctx
        .uuid_to_file
        .get(&fqn_uuid)
        .is_some_and(|file| super::overrides::overloads_span_hierarchy(file));
    if !overloads_span_hierarchy {
        return fqn_uuid;
    }
    inherited_overload(fqn, &intent.method, matches_arity, ctx).unwrap_or(fqn_uuid)
}

/// Every declaration of `fqn` — all of its overloads, or just the one the
/// FQN map knows.
fn overloads_of<'a>(fqn: &str, uuid: &'a Uuid, ctx: &'a ResolutionContext) -> &'a [Uuid] {
    ctx.fqn_overloads
        .get(fqn)
        .map_or(std::slice::from_ref(uuid), Vec::as_slice)
}

/// Breadth-first over the supertypes of the class declaring `method_fqn`:
/// the nearest supertype declaring `method` with a matching overload wins.
/// Several same-named supertypes (in different packages) are ranked by
/// [`pick_deterministic_candidate`].
fn inherited_overload(
    method_fqn: &str,
    method: &str,
    matches_arity: impl Fn(&Uuid) -> bool,
    ctx: &ResolutionContext,
) -> Option<Uuid> {
    let class_fqn = method_fqn
        .strip_suffix(method)?
        .trim_end_matches(['.', ':']);
    let class_name = simple_type_name(class_fqn);

    let mut visited: HashSet<String> = HashSet::from([class_name.to_string()]);
    let mut queue: VecDeque<String> = VecDeque::from([class_name.to_string()]);
    while let Some(class) = queue.pop_front() {
        let Some(parents) = ctx.extends_map.get(&class) else {
            continue;
        };
        for parent in parents.iter().map(|p| simple_type_name(p)) {
            if !visited.insert(parent.to_string()) {
                continue;
            }
            let suffix_dot = format!(".{parent}.{method}");
            let suffix_colon = format!("::{parent}::{method}");
            let candidates: Vec<(&str, Uuid)> = ctx
                .class_member_index
                .get(&format!("{parent}.{method}"))
                .into_iter()
                .flatten()
                .filter(|(fqn, _)| {
                    fqn == &format!("{parent}.{method}")
                        || fqn.ends_with(&suffix_dot)
                        || fqn.ends_with(&suffix_colon)
                })
                .flat_map(|(fqn, uuid)| {
                    overloads_of(fqn, uuid, ctx)
                        .iter()
                        .filter(|u| matches_arity(u) && !ctx.is_shadowed_dependency(u, parent))
                        .map(move |u| (fqn.as_str(), *u))
                })
                .collect();
            if let Some(uuid) = pick_deterministic_candidate(&candidates, None) {
                return Some(uuid);
            }
            queue.push_back(parent.to_string());
        }
    }
    None
}

/// Disambiguate candidates by scoring each one's FQN against the segments of a
/// chained receiver expression (e.g. `engine.chatter.method`).
///
/// For a receiver like `engine.chatter` and candidates whose FQNs are
/// `llamafactory.webui.chatter.WebChatModel.method` and
/// `llamafactory.model.loader.method`, the first FQN contains the receiver
/// segment `chatter` (score 1) while the second contains nothing (score 0).
/// The unique top-scorer wins; ties fall through to other heuristics.
///
/// Receivers without dots are ignored — the existing same-file / unique-name
/// fallbacks already handle them.
pub(crate) fn disambiguate_by_receiver_chain(
    candidate_uuids: &[Uuid],
    receiver: &str,
    ctx: &ResolutionContext,
) -> Option<Uuid> {
    if !receiver.contains('.') {
        return None;
    }
    let uuid_to_fqn = ctx.uuid_to_fqn?;

    let segments: Vec<&str> = receiver
        .split('.')
        .map(str::trim)
        .filter(|s| !s.is_empty() && *s != "self" && *s != "cls" && *s != "this")
        .collect();
    if segments.is_empty() {
        return None;
    }

    let mut best_score: usize = 0;
    let mut best_uuid: Option<Uuid> = None;
    let mut tied = false;

    for uuid in candidate_uuids {
        let Some(fqn) = uuid_to_fqn.get(uuid) else {
            continue;
        };
        let fqn_segments: Vec<&str> = fqn.split(['.', ':']).collect();
        let score = segments
            .iter()
            .filter(|seg| fqn_segments.contains(seg))
            .count();
        if score == 0 {
            continue;
        }
        match score.cmp(&best_score) {
            std::cmp::Ordering::Greater => {
                best_score = score;
                best_uuid = Some(*uuid);
                tied = false;
            }
            std::cmp::Ordering::Equal => {
                tied = true;
            }
            std::cmp::Ordering::Less => {}
        }
    }

    if tied { None } else { best_uuid }
}

/// Detects receiver expressions that denote a `super` invocation.
///
/// Covers:
/// - Java/Kotlin: `super.method()` → receiver is `"super"`.
/// - Python 3: `super().__init__()` → receiver is `"super()"`.
/// - Python 2 / explicit form: `super(WebChatModel, self).__init__()` →
///   receiver is `"super(WebChatModel, self)"`.
pub(crate) fn is_super_receiver(receiver: &str) -> bool {
    receiver == "super" || receiver == "super()" || receiver.starts_with("super(")
}

pub(crate) fn lookup_fqn(
    class: &str,
    method: &str,
    fqn_to_uuid: &HashMap<String, Uuid>,
) -> Option<Uuid> {
    let dot_fqn = format!("{}.{}", class, method);
    if let Some(&uuid) = fqn_to_uuid.get(&dot_fqn) {
        return Some(uuid);
    }
    let colon_fqn = format!("{}::{}", class, method);
    fqn_to_uuid.get(&colon_fqn).copied()
}

/// [`lookup_fqn`], except that a dependency member is never returned for a
/// type the local repository declares under the same name (issue #34).
fn lookup_local_fqn(class: &str, method: &str, ctx: &ResolutionContext) -> Option<Uuid> {
    lookup_fqn(class, method, ctx.fqn_to_uuid)
        .filter(|uuid| !ctx.is_shadowed_dependency(uuid, class))
}

/// `true` when `needle` occurs in `fqn` as a run of whole FQN segments: each
/// occurrence must start at the beginning of `fqn` or right after a `.` / `:`
/// separator, and end at the end of `fqn` or right before one. So
/// `LookupService.listOptions` matches `com.x.LookupService.listOptions` but
/// neither `com.x.DefaultLookupService.listOptions` nor
/// `com.x.LookupService.listOptionsAsync`.
pub(crate) fn contains_on_segment_boundary(fqn: &str, needle: &str) -> bool {
    let is_separator = |c: char| c == '.' || c == ':';
    fqn.match_indices(needle).any(|(start, matched)| {
        let left_ok = fqn[..start].chars().next_back().is_none_or(is_separator);
        let right_ok = fqn[start + matched.len()..]
            .chars()
            .next()
            .is_none_or(is_separator);
        left_ok && right_ok
    })
}

pub(crate) fn find_entity_in_same_file(
    candidate_uuids: &[Uuid],
    caller_file_path: &str,
    uuid_to_file: &HashMap<Uuid, String>,
) -> Option<Uuid> {
    for &uuid in candidate_uuids {
        if let Some(file_path) = uuid_to_file.get(&uuid)
            && file_path == caller_file_path
        {
            return Some(uuid);
        }
    }
    None
}

/// Rank equally-plausible resolution candidates with a total order and return
/// the winner.
///
/// `HashMap::iter()` / `HashMap::values()` order is randomized per process
/// (`std::collections::hash_map::RandomState`), so a "first match wins" scan
/// over a map emits a different edge target on every run whenever several
/// candidates match at once. Candidates are ranked by:
///
/// 1. same-file locality — a candidate declared in the caller's own file wins
///    outright, which is the criterion [`find_entity_in_same_file`] already
///    establishes elsewhere in this module;
/// 2. the lexicographically smallest FQN;
/// 3. the smallest UUID, reachable when two distinct entities carry the same
///    FQN — this completes the ordering so the winner is always unique.
///
/// `caller_file` is the caller's file path together with the uuid → file map.
/// Pass `None` when same-file locality is not meaningful for the match (a VCL
/// `include` target is a different file from the includer by construction);
/// rule 1 is then skipped and the FQN / UUID ordering decides.
///
/// This only picks *which* target an already-decided edge points at. It never
/// changes whether the edge is created: a caller that found at least one
/// candidate keeps resolving to exactly one target.
pub(crate) fn pick_deterministic_candidate(
    candidates: &[(&str, Uuid)],
    caller_file: Option<(&str, &HashMap<Uuid, String>)>,
) -> Option<Uuid> {
    candidates
        .iter()
        .map(|&(fqn, uuid)| {
            let in_caller_file = caller_file.is_some_and(|(file, uuid_to_file)| {
                uuid_to_file.get(&uuid).is_some_and(|f| f == file)
            });
            // `!in_caller_file` is `false` for same-file candidates so they
            // sort first; the tuple as a whole is a total order, so the winner
            // is unique and independent of the order candidates arrived in.
            (!in_caller_file, fqn, uuid)
        })
        .min()
        .map(|(_, _, uuid)| uuid)
}

/// Redirect a resolved class call UUID to its constructor (`__init__`) when applicable.
///
/// For languages where class instantiation is semantically equivalent to invoking
/// the constructor (Python `Foo()`, etc.), this turns a `(Foo, Calls)` edge into a
/// `(Foo.__init__, Calls)` edge. The redirect is applied **after** resolution so it
/// works for every parser without language-specific changes.
///
/// Only triggers when:
/// 1. `uuid` resolves to a class-like entity (`Class`, `PythonClass`, `RustStruct`,
///    or `CStruct`).
/// 2. The class has an explicit `__init__` method, or one is reachable via the
///    extends-map (inheritance chain).
///
/// Other intents (`TypeReference`, `ValueReference`, `Extends`, …) are unaffected.
pub(crate) fn redirect_class_call_to_constructor(
    uuid: Uuid,
    caller_file_path: &str,
    ctx: &ResolutionContext,
) -> Uuid {
    let Some(kind_map) = ctx.uuid_to_kind else {
        return uuid;
    };
    let Some(name_map) = ctx.uuid_to_name else {
        return uuid;
    };

    let Some(kind) = kind_map.get(&uuid) else {
        return uuid;
    };
    if !matches!(
        kind,
        EntityKind::Class | EntityKind::RustStruct | EntityKind::CStruct | EntityKind::PythonClass
    ) {
        return uuid;
    }

    let Some(class_name) = name_map.get(&uuid) else {
        return uuid;
    };

    if let Some(init_uuid) = lookup_init_in_class_hierarchy(class_name, caller_file_path, ctx) {
        return init_uuid;
    }

    if let Some(parents) = ctx.extends_map.get(class_name) {
        for parent in parents {
            if let Some(init_uuid) = lookup_init_in_class_hierarchy(parent, caller_file_path, ctx) {
                return init_uuid;
            }
        }
    }

    uuid
}

/// Locate a class's `__init__` constructor by trying every FQN shape the parser
/// may have produced (`<ClassName>.__init__`, `<mod.ClassName>.__init__`, …).
fn lookup_init_in_class_hierarchy(
    class_name: &str,
    caller_file_path: &str,
    ctx: &ResolutionContext,
) -> Option<Uuid> {
    lookup_method_in_class_hierarchy(class_name, "__init__", caller_file_path, ctx)
}

/// Locate `<class_name>.<method>` by trying every FQN shape the parser may
/// have produced. Falls back to suffix matching against the FQN map so that
/// short class names (e.g. `"ChatModel"`) resolve to fully-qualified
/// methods (e.g. `"llamafactory.chat.ChatModel.__init__"`).
fn lookup_method_in_class_hierarchy(
    class_name: &str,
    method: &str,
    caller_file_path: &str,
    ctx: &ResolutionContext,
) -> Option<Uuid> {
    if let Some(uuid) = lookup_local_fqn(class_name, method, ctx) {
        return Some(uuid);
    }

    if let Some(uuid_to_fqn) = ctx.uuid_to_fqn {
        // The suffix predicate matches every class that ends with the short
        // name, so a name reused across modules (`pkg.alpha.ChatModel`,
        // `pkg.beta.ChatModel`, …) yields several methods at once. Collect
        // them all and rank them instead of returning the first hit out of
        // the map: `uuid_to_fqn` iteration order is randomized per process.
        // Recall is unchanged — an edge is still produced whenever at least
        // one class FQN matches.
        let candidates: Vec<(&str, Uuid)> = uuid_to_fqn
            .values()
            .filter_map(|class_fqn| {
                let is_class_match = class_fqn == class_name
                    || class_fqn.ends_with(&format!(".{}", class_name))
                    || class_fqn.ends_with(&format!("::{}", class_name));
                if !is_class_match {
                    return None;
                }
                lookup_fqn_by_fqn(class_fqn, method, ctx.fqn_to_uuid)
                    .filter(|uuid| !ctx.is_shadowed_dependency(uuid, class_name))
                    .map(|uuid| (class_fqn.as_str(), uuid))
            })
            .collect();
        return pick_deterministic_candidate(
            &candidates,
            Some((caller_file_path, ctx.uuid_to_file)),
        );
    }

    None
}

/// Lookup `__init__` using a fully-qualified class path (handles `mod.Class.__init__`).
fn lookup_fqn_by_fqn(
    class_fqn: &str,
    method: &str,
    fqn_to_uuid: &HashMap<String, Uuid>,
) -> Option<Uuid> {
    lookup_fqn(class_fqn, method, fqn_to_uuid)
}

/// Fallback ladder for homonym candidates that share the callee's method name
/// (neither FQN nor receiver lookup matched).
///
/// When the call's argument count is known, candidates whose declared arity
/// contradicts it are discarded first. Arity only narrows the candidates; it
/// is never evidence on its own: "the only `isEmpty` with one parameter" in
/// a repository is not the target of every `x.isEmpty(y)` (issue #35). Then:
///
/// 1. a same-file candidate;
/// 2. receiver-chain disambiguation (only when a receiver is available);
/// 3. a callee name declared exactly once.
///
/// Steps 2 and 3 are skipped for a receiver of an external type (see
/// [`is_external_type_receiver`]): `String.valueOf(..)` or a field typed as
/// a library class never binds to a repository method that merely shares
/// the callee's name (issue #32).
///
/// Dependency candidates shadowed by a local type the receiver may denote
/// (see [`receiver_type_names`]) are discarded (issue #34), and the name then
/// no longer counts as declared once: the discarded candidate proves the
/// receiver's real member is not indexed, so the one left over belongs to
/// some other type.
fn resolve_homonym_fallback(
    uuids: &[Uuid],
    arg_count: Option<usize>,
    receiver: Option<&str>,
    caller_file_path: &str,
    ctx: &ResolutionContext,
) -> Option<Uuid> {
    let declared_arity = |uuid: &Uuid| ctx.uuid_to_arg_count.and_then(|arities| arities.get(uuid));
    let receiver_types = receiver.map(receiver_type_names);
    let unshadowed: Vec<Uuid> = uuids
        .iter()
        .copied()
        .filter(|uuid| {
            !receiver_types.as_ref().is_some_and(|(class, capitalized)| {
                ctx.is_shadowed_dependency(uuid, class)
                    || ctx.is_shadowed_dependency(uuid, capitalized)
            })
        })
        .collect();
    let declared_once = uuids.len() == 1 && unshadowed.len() == 1;
    let candidates: Vec<Uuid> = unshadowed
        .into_iter()
        .filter(|uuid| match (arg_count, declared_arity(uuid)) {
            (Some(call_arity), Some(&arity)) => arity == call_arity,
            _ => true,
        })
        .collect();

    if let Some(same_file_uuid) =
        find_entity_in_same_file(&candidates, caller_file_path, ctx.uuid_to_file)
    {
        return Some(same_file_uuid);
    }
    if receiver.is_some_and(|r| is_external_type_receiver(r, ctx)) {
        return None;
    }
    if let Some(receiver) = receiver
        && let Some(u) = disambiguate_by_receiver_chain(&candidates, receiver, ctx)
    {
        return Some(u);
    }
    if declared_once {
        return candidates.first().copied();
    }
    None
}

fn resolve_super_call(
    intent: &CallIntent,
    caller_enclosing_class: Option<&str>,
    caller_file_path: &str,
    ctx: &ResolutionContext,
) -> Option<Uuid> {
    let enclosing_class = caller_enclosing_class?;
    let parents = ctx.extends_map.get(enclosing_class)?;
    for parent in parents {
        if let Some(uuid) =
            lookup_method_in_class_hierarchy(parent, &intent.method, caller_file_path, ctx)
        {
            return Some(disambiguate_overload(uuid, intent, ctx));
        }
    }
    None
}

/// The caller-side context a call intent is resolved from.
pub(crate) struct CallerScope<'a> {
    pub file_path: &'a str,
    /// Short name of the enclosing type (`PriceAdjuster`).
    pub enclosing_class: Option<&'a str>,
    /// Fully qualified enclosing type when the parser provides it
    /// (`shop::pricing::PriceAdjuster` for Rust methods).
    pub enclosing_class_fqn: Option<&'a str>,
    /// The caller is Rust code: compound receivers are resolved by
    /// [`resolve_rust_compound_receiver_call`].
    pub is_rust: bool,
    /// The caller is an ES module or CommonJS file: receiverless calls obey
    /// file/import visibility instead of repository-global name fallback.
    pub is_javascript: bool,
}

/// `true` for a receiver the Rust parser renders from a compound expression:
/// `self`, a field path (`self.repo`) or a call chain (`a.b()`, `(expr)`).
/// A plain identifier (`sku`) or type (`Repo`) is not compound.
fn is_rust_compound_receiver(receiver: &str) -> bool {
    receiver == "self" || receiver.contains(['.', '(', '[', '!'])
}

/// `price_rules` → `PriceRules`.
fn snake_to_camel(segment: &str) -> String {
    segment
        .split('_')
        .map(|part| {
            let mut chars = part.chars();
            chars.next().map_or_else(String::new, |first| {
                first.to_uppercase().chain(chars).collect()
            })
        })
        .collect()
}

/// Resolve a Rust call on a compound receiver from evidence only.
///
/// The receiver's type is unknown without type inference, so the name-only
/// homonym fallback would bind `skus.iter().collect()` or
/// `self.label.as_str()` to whichever user method shares the std method's
/// name (issue #36). Instead:
///
/// - `self` binds to the enclosing type (or its parents) and is terminal;
/// - any other compound receiver binds to `<Type>::<method>` where `Type` is
///   its last path segment in CamelCase — a field or getter named after its
///   type (`self.repo` → `Repo`, `self.price_rules()` → `PriceRules`).
fn resolve_rust_compound_receiver_call(
    intent: &CallIntent,
    receiver: &str,
    caller: &CallerScope<'_>,
    ctx: &ResolutionContext,
) -> Option<Uuid> {
    if receiver == "self" {
        return resolve_implicit_this_call(intent, caller, ctx);
    }
    let segment = receiver
        .rsplit('.')
        .next()?
        .trim_end_matches("()")
        .trim_end_matches("[]")
        .rsplit("::")
        .next()?;
    if segment.is_empty()
        || segment == "self"
        || !segment.chars().all(|c| c.is_alphanumeric() || c == '_')
    {
        return None;
    }
    let type_name = snake_to_camel(segment);
    let uuid = lookup_method_by_class_and_name(&type_name, &intent.method, caller.file_path, ctx)?;
    Some(disambiguate_overload(uuid, intent, ctx))
}

fn resolve_implicit_this_call(
    intent: &CallIntent,
    caller: &CallerScope<'_>,
    ctx: &ResolutionContext,
) -> Option<Uuid> {
    let explicit_receiver = match intent.receiver.as_deref() {
        None => false,
        Some("this" | "self") => true,
        Some(_) => return None,
    };

    // The fully qualified enclosing type is exact: it never collides with a
    // same-named type in another module (Rust FQNs are crate-anchored, so the
    // short-name probe below cannot match them).
    if let Some(class_fqn) = caller.enclosing_class_fqn
        && let Some(uuid) = lookup_local_fqn(class_fqn, &intent.method, ctx)
    {
        return Some(disambiguate_overload(uuid, intent, ctx));
    }

    let enclosing_class = caller.enclosing_class?;
    if let Some(uuid) = lookup_local_fqn(enclosing_class, &intent.method, ctx) {
        return Some(disambiguate_overload(uuid, intent, ctx));
    }

    // An explicit `self.m()` / `this.m()` names a method of the enclosing
    // type or of one of its supertypes, even when its FQN is
    // module-qualified. Without this the call falls through to the
    // name-only homonym fallback and may bind to a same-named method of
    // another type, or stay unresolved when several types declare it
    // (issue #41). A bare `m()` is not covered: in Rust it is a
    // free-function call, never a method of the enclosing type.
    if explicit_receiver {
        return resolve_in_type_hierarchy(enclosing_class, intent, caller.file_path, ctx);
    }
    None
}

/// Locate `method` declared on a type whose simple (or partially qualified)
/// name is `class_name`: `CatalogProvider` matches
/// `com.api.CatalogProvider.fetchItems` but never
/// `com.impl.HttpCatalogProvider.fetchItems`. A single hash probe on the
/// class/member index replaces a scan over every FQN.
fn lookup_method_by_class_and_name(
    class_name: &str,
    method: &str,
    caller_file_path: &str,
    ctx: &ResolutionContext,
) -> Option<Uuid> {
    if let Some(uuid) = lookup_local_fqn(class_name, method, ctx) {
        return Some(uuid);
    }

    let simple_class = class_name.rsplit(['.', ':']).next()?;
    let entries = ctx
        .class_member_index
        .get(&format!("{simple_class}.{method}"))?;
    let suffix_dot = format!(".{class_name}.{method}");
    let suffix_colon = format!("::{class_name}::{method}");
    let candidates: Vec<(&str, Uuid)> = entries
        .iter()
        .filter(|(fqn, uuid)| {
            (fqn.ends_with(&suffix_dot) || fqn.ends_with(&suffix_colon))
                && !ctx.is_shadowed_dependency(uuid, class_name)
        })
        .map(|(fqn, uuid)| (fqn.as_str(), *uuid))
        .collect();
    pick_deterministic_candidate(&candidates, Some((caller_file_path, ctx.uuid_to_file)))
}

fn resolve_static_receiver_call(
    intent: &CallIntent,
    caller_file_path: &str,
    ctx: &ResolutionContext,
) -> Option<Uuid> {
    let receiver = intent.receiver.as_deref()?;
    if !receiver.chars().next().is_some_and(|c| c.is_uppercase()) {
        return None;
    }
    match pinned_type(receiver, caller_file_path, ctx) {
        Some(TypePin::Indexed(type_fqn)) => {
            resolve_in_type_hierarchy(&type_fqn, intent, caller_file_path, ctx)
        }
        Some(TypePin::External) => None,
        None => resolve_in_type_hierarchy(receiver, intent, caller_file_path, ctx),
    }
}

/// What the caller file's imports and package say about an unqualified,
/// capitalized type name (issue #45).
pub(crate) enum TypePin {
    /// The name denotes this indexed type.
    Indexed(String),
    /// A single-type import binds the name to a type no indexed repository
    /// declares.
    External,
}

/// Java/Kotlin precedence: a single-type import (by name or Kotlin alias),
/// then a type of the caller's own package, then an on-demand import.
/// Same-package and on-demand candidates only count when indexed, since
/// the name may equally come from `java.lang` or another implicit scope.
/// `None` when the caller's file carries no imports (other languages) or
/// nothing pins the name, so the name-based lookups apply unchanged.
pub(crate) fn pinned_type(
    name: &str,
    caller_file_path: &str,
    ctx: &ResolutionContext,
) -> Option<TypePin> {
    if !name.chars().next().is_some_and(char::is_uppercase)
        || !name
            .chars()
            .all(|c| c.is_alphanumeric() || c == '_' || c == '$')
    {
        return None;
    }
    let scope = ctx.import_scopes.get(caller_file_path)?;
    if let Some(type_fqn) = scope.imported(name) {
        return Some(if ctx.fqn_to_uuid.contains_key(type_fqn) {
            TypePin::Indexed(type_fqn.to_string())
        } else {
            TypePin::External
        });
    }
    scope
        .implicit_candidates(name)
        .find(|type_fqn| ctx.fqn_to_uuid.contains_key(type_fqn))
        .map(TypePin::Indexed)
}

/// Breadth-first over `class` and its `extends` ancestors, so a method
/// inherited from a parent type (e.g. a sub-interface field calling a
/// method declared on its super-interface) still resolves. The nearest
/// declaring type wins.
fn resolve_in_type_hierarchy(
    class: &str,
    intent: &CallIntent,
    caller_file_path: &str,
    ctx: &ResolutionContext,
) -> Option<Uuid> {
    let mut visited: HashSet<&str> = HashSet::from([class]);
    let mut queue: VecDeque<&str> = VecDeque::from([class]);
    while let Some(class_name) = queue.pop_front() {
        if let Some(uuid) =
            lookup_method_by_class_and_name(class_name, &intent.method, caller_file_path, ctx)
        {
            return Some(disambiguate_overload(uuid, intent, ctx));
        }
        let parents = ctx.extends_map.get(class_name).into_iter().flatten();
        for parent in parents {
            if visited.insert(parent.as_str()) {
                queue.push_back(parent.as_str());
            }
        }
    }

    None
}

/// The type names a receiver expression may denote: its last dotted segment
/// (`engine.chatter` → `chatter`) and that segment capitalized (`Chatter`),
/// for receivers named after their type.
fn receiver_type_names(receiver: &str) -> (&str, String) {
    let receiver_class = receiver.rsplit('.').next().map_or(receiver, str::trim);
    let mut chars = receiver_class.chars();
    let capitalized = chars.next().map_or_else(String::new, |first| {
        first.to_uppercase().chain(chars).collect()
    });
    (receiver_class, capitalized)
}

/// `true` when the receiver names a type nothing in the loaded repositories
/// declares: its last segment is a capitalized identifier (`String`,
/// `ModelMapper`, `java.util.Objects`, `std::string::String`) and no segment
/// of its path names an indexed entity. Such a type comes from the JDK, the
/// standard library or a third-party package, so its members are not
/// indexed (issue #32). An indexed segment keeps the receiver in the repo:
/// `StringUtils.INSTANCE` is a Kotlin `object` seen from Java. Lowercase
/// receivers carry no type evidence and are not covered.
fn is_external_type_receiver(receiver: &str, ctx: &ResolutionContext) -> bool {
    let name = simple_type_name(receiver);
    let names_type = name.chars().next().is_some_and(char::is_uppercase)
        && name
            .chars()
            .all(|c| c.is_alphanumeric() || c == '_' || c == '$');
    let path = receiver.split('<').next().unwrap_or(receiver);
    names_type
        && !path
            .split(['.', ':'])
            .any(|segment| ctx.name_to_uuids.contains_key(segment.trim()))
}

fn resolve_receiver_call(
    intent: &CallIntent,
    caller_file_path: &str,
    ctx: &ResolutionContext,
) -> Option<Uuid> {
    let receiver = intent.receiver.as_ref()?;
    let (receiver_class, capitalized) = receiver_type_names(receiver);
    // A receiver named after a type the caller's file pins (`config` for an
    // imported `com.b.Config`) binds within that type only: the name-based
    // scan below would mix in same-named types of other packages.
    match pinned_type(&capitalized, caller_file_path, ctx) {
        Some(TypePin::Indexed(type_fqn)) => {
            return resolve_in_type_hierarchy(&type_fqn, intent, caller_file_path, ctx);
        }
        Some(TypePin::External) => return None,
        None => {}
    }
    let shadowed = |uuid: &Uuid| {
        ctx.is_shadowed_dependency(uuid, receiver_class)
            || ctx.is_shadowed_dependency(uuid, &capitalized)
    };

    if !receiver_class.is_empty() {
        if let Some(uuid) = lookup_local_fqn(receiver_class, &intent.method, ctx) {
            return Some(disambiguate_overload(uuid, intent, ctx));
        }

        if let Some(uuid) = lookup_local_fqn(&capitalized, &intent.method, ctx) {
            return Some(disambiguate_overload(uuid, intent, ctx));
        }

        let needles = [
            format!("{}.{}", receiver_class, intent.method),
            format!("{}.{}", capitalized, intent.method),
            format!("{}::{}", receiver_class, intent.method),
            format!("{}::{}", capitalized, intent.method),
        ];
        // The predicate is deliberately loose about the module path
        // (`pkg.alpha.Widget.render` and `pkg.beta.Widget.render` both match
        // `Widget.render`), so several FQNs hit at once when a method name is
        // reused across module paths. Collect every hit and rank them instead
        // of returning the first one out of the map: `fqn_to_uuid` iteration
        // order is randomized per process. It is strict about segment
        // boundaries, though: `LookupService.listOptions` must never match
        // `DefaultLookupService.listOptions` (issue #29).
        let candidates: Vec<(&str, Uuid)> = ctx
            .fqn_to_uuid
            .iter()
            .filter(|(fqn, uuid)| {
                needles
                    .iter()
                    .any(|needle| contains_on_segment_boundary(fqn, needle))
                    && !shadowed(uuid)
            })
            .map(|(fqn, uuid)| (fqn.as_str(), *uuid))
            .collect();
        if let Some(uuid) =
            pick_deterministic_candidate(&candidates, Some((caller_file_path, ctx.uuid_to_file)))
        {
            return Some(disambiguate_overload(uuid, intent, ctx));
        }
    }

    if let Some(uuids) = ctx.name_to_uuids.get(&intent.method)
        && let Some(u) = resolve_homonym_fallback(
            uuids,
            intent.arg_count,
            Some(receiver.as_str()),
            caller_file_path,
            ctx,
        )
    {
        return Some(u);
    }

    None
}

/// Whether a JavaScript/TypeScript receiverless call may target `uuid`.
/// Cross-file targets require an import binding, and fields/constants are
/// never callable targets. Same-file import aliases stay eligible so the
/// alias pass can redirect them to their imported function (issue #48).
/// A target outside the batch (an unchanged file in an incremental run) has
/// no known kind; its import binding is enough to keep it.
fn javascript_bare_call_candidate(
    uuid: &Uuid,
    caller_file_path: &str,
    imported: bool,
    ctx: &ResolutionContext,
) -> bool {
    let same_file = ctx
        .uuid_to_file
        .get(uuid)
        .is_some_and(|file| file == caller_file_path);
    if !same_file && !imported {
        return false;
    }
    let callable = ctx
        .uuid_to_kind
        .and_then(|kinds| kinds.get(uuid))
        .is_none_or(|kind| {
            matches!(
                kind,
                EntityKind::Function | EntityKind::Method | EntityKind::Class
            )
        });
    callable || (same_file && ctx.alias_map.contains_key(uuid))
}

fn resolve_bare_call(
    intent: &CallIntent,
    caller: &CallerScope<'_>,
    ctx: &ResolutionContext,
) -> Option<Uuid> {
    if intent.receiver.is_some() {
        return None;
    }
    let caller_file_path = caller.file_path;
    let uuids = ctx.name_to_uuids.get(&intent.method)?;
    if caller.is_javascript {
        let imported = ctx
            .import_scopes
            .get(caller_file_path)
            .is_some_and(|scope| {
                scope
                    .module_bindings
                    .iter()
                    .any(|binding| binding == &intent.method)
            });
        let candidates: Vec<Uuid> = uuids
            .iter()
            .copied()
            .filter(|uuid| javascript_bare_call_candidate(uuid, caller_file_path, imported, ctx))
            .collect();
        return resolve_homonym_fallback(
            &candidates,
            intent.arg_count,
            None,
            caller_file_path,
            ctx,
        );
    }
    resolve_homonym_fallback(uuids, intent.arg_count, None, caller_file_path, ctx)
}

pub(crate) fn resolve_single_call_intent(
    intent: &CallIntent,
    caller: &CallerScope<'_>,
    ctx: &ResolutionContext,
) -> Option<Uuid> {
    let caller_file_path = caller.file_path;
    // `super.` calls are terminal: an unresolved super() must never fall
    // through to the bare-name fallback (it would be misattributed to self).
    if intent.receiver.as_deref().is_some_and(is_super_receiver) {
        return resolve_super_call(intent, caller.enclosing_class, caller_file_path, ctx);
    }
    if caller.is_rust
        && let Some(receiver) = intent.receiver.as_deref()
        && is_rust_compound_receiver(receiver)
    {
        return resolve_rust_compound_receiver_call(intent, receiver, caller, ctx);
    }
    resolve_implicit_this_call(intent, caller, ctx)
        .or_else(|| resolve_static_receiver_call(intent, caller_file_path, ctx))
        .or_else(|| resolve_receiver_call(intent, caller_file_path, ctx))
        .or_else(|| resolve_bare_call(intent, caller, ctx))
}

#[cfg(test)]
mod tests {
    use super::super::test_utils::*;
    use super::*;
    use crate::models::{
        EntityKind, ImportScope, ReferenceIntent, RelationshipType, ResolutionEntity,
    };
    use crate::pipeline::ingest::resolve::resolve_reference_intents;

    #[test]
    fn test_resolve_local_call() {
        let mut caller = mock_resolution_entity("methodA", "ClassA.methodA", Some("ClassA"));
        let callee = mock_resolution_entity("methodB", "ClassA.methodB", Some("ClassA"));

        caller.reference_intents.push(ReferenceIntent::Call {
            method: "methodB".to_string(),
            receiver: None,
            line: 10,
            arg_count: None,
        });

        let mut entities = vec![caller, callee];
        resolve_reference_intents(&mut entities);

        assert_eq!(entities[0].relationships.len(), 1);
        assert_eq!(
            entities[0].relationships[0],
            (entities[1].uuid, RelationshipType::Calls)
        );
    }

    /// Issue #48: JavaScript modules do not see declarations from unrelated
    /// files, and a receiverless call can never target a class field.
    #[test]
    fn test_js_bare_call_does_not_bind_cross_file_field() {
        let mut caller = mock_resolution_entity_with_kind(
            "update",
            "CartStore.update",
            Some("CartStore"),
            "orders.ts",
            EntityKind::Method,
        );
        caller.reference_intents.push(ReferenceIntent::Call {
            method: "snapshot".to_string(),
            receiver: None,
            line: 6,
            arg_count: None,
        });
        let field = mock_resolution_entity_with_kind(
            "snapshot",
            "ProductCardComponent.snapshot",
            Some("ProductCardComponent"),
            "catalog.ts",
            EntityKind::Constant,
        );

        let mut entities = vec![caller, field];
        resolve_reference_intents(&mut entities);
        assert!(entities[0].relationships.is_empty());
    }

    /// A cross-file JavaScript function requires an import binding; a
    /// same-file function remains visible without one.
    #[test]
    fn test_js_bare_call_uses_module_scope() {
        let imported = mock_resolution_entity_with_kind(
            "applyDiscount",
            "applyDiscount",
            None,
            "discount.ts",
            EntityKind::Function,
        );
        let local = mock_resolution_entity_with_kind(
            "formatTotal",
            "formatTotal",
            None,
            "orders.ts",
            EntityKind::Function,
        );
        let mut caller = mock_resolution_entity_with_kind(
            "update",
            "update",
            None,
            "orders.ts",
            EntityKind::Function,
        );
        caller.reference_intents = vec![
            ReferenceIntent::Call {
                method: "applyDiscount".to_string(),
                receiver: None,
                line: 6,
                arg_count: Some(2),
            },
            ReferenceIntent::Call {
                method: "formatTotal".to_string(),
                receiver: None,
                line: 7,
                arg_count: Some(1),
            },
        ];
        let imported_uuid = imported.uuid;
        let local_uuid = local.uuid;

        let mut entities = vec![caller, imported, local];
        resolve_reference_intents(&mut entities);
        assert_eq!(
            entities[0].relationships,
            vec![(local_uuid, RelationshipType::Calls)]
        );

        entities[0].import_scope = Some(std::sync::Arc::new(ImportScope {
            module_bindings: vec!["applyDiscount".to_string()],
            ..ImportScope::default()
        }));
        resolve_reference_intents(&mut entities);
        assert!(
            entities[0]
                .relationships
                .contains(&(imported_uuid, RelationshipType::Calls))
        );
    }

    /// Incremental runs resolve only the changed files, so an imported
    /// function declared in an unchanged file has no known kind. The import
    /// binding is enough evidence; only a known field/constant is rejected.
    #[test]
    fn test_js_bare_call_binds_imported_target_outside_the_batch() {
        let mut caller = mock_resolution_entity_with_kind(
            "update",
            "update",
            None,
            "orders.ts",
            EntityKind::Function,
        );
        caller.import_scope = Some(std::sync::Arc::new(ImportScope {
            module_bindings: vec!["applyDiscount".to_string()],
            ..ImportScope::default()
        }));
        caller.reference_intents.push(ReferenceIntent::Call {
            method: "applyDiscount".to_string(),
            receiver: None,
            line: 1,
            arg_count: None,
        });
        let unchanged = Uuid::new_v4();
        let mut entities = vec![caller];
        crate::pipeline::ingest::resolve::resolve_reference_intents_with_context(
            &mut entities,
            HashMap::from([("applyDiscount".to_string(), unchanged)]),
            HashMap::from([("applyDiscount".to_string(), vec![unchanged])]),
        );
        assert_eq!(
            entities[0].relationships,
            vec![(unchanged, RelationshipType::Calls)]
        );
    }

    /// Issue #49: a `this.x` access to a field is a reference; to a getter
    /// (a method) it is a call.
    #[test]
    fn test_member_access_edge_depends_on_target_kind() {
        let field = mock_resolution_entity_with_kind(
            "items",
            "CartStore.items",
            Some("CartStore"),
            "cart.ts",
            EntityKind::Constant,
        );
        let getter = mock_resolution_entity_with_kind(
            "total",
            "CartStore.total",
            Some("CartStore"),
            "cart.ts",
            EntityKind::Method,
        );
        let mut caller = mock_resolution_entity_with_kind(
            "reset",
            "CartStore.reset",
            Some("CartStore"),
            "cart.ts",
            EntityKind::Method,
        );
        caller.reference_intents = ["items", "total"]
            .into_iter()
            .map(|member| ReferenceIntent::MemberAccess {
                member: member.to_string(),
                receiver: "this".to_string(),
                line: 1,
            })
            .collect();
        let (field_uuid, getter_uuid) = (field.uuid, getter.uuid);

        let mut entities = vec![field, getter, caller];
        resolve_reference_intents(&mut entities);
        assert_eq!(
            entities[2].relationships,
            vec![
                (field_uuid, RelationshipType::References),
                (getter_uuid, RelationshipType::Calls),
            ]
        );
    }

    #[test]
    fn test_resolve_static_call() {
        let mut caller = mock_resolution_entity("main", "App.main", None);
        let callee = mock_resolution_entity("staticMethod", "Utils.staticMethod", Some("Utils"));

        caller.reference_intents.push(ReferenceIntent::Call {
            method: "staticMethod".to_string(),
            receiver: Some("Utils".to_string()),
            line: 5,
            arg_count: None,
        });

        let mut entities = vec![caller, callee];
        resolve_reference_intents(&mut entities);

        assert_eq!(entities[0].relationships.len(), 1);
        assert_eq!(
            entities[0].relationships[0],
            (entities[1].uuid, RelationshipType::Calls)
        );
    }

    #[test]
    fn test_resolve_instance_call_fuzzy() {
        let mut caller = mock_resolution_entity("doWork", "Service.doWork", Some("Service"));
        let callee = mock_resolution_entity("execute", "Worker.execute", Some("Worker"));

        caller.reference_intents.push(ReferenceIntent::Call {
            method: "execute".to_string(),
            receiver: Some("worker".to_string()),
            line: 20,
            arg_count: None,
        });

        let mut entities = vec![caller, callee];
        resolve_reference_intents(&mut entities);

        assert_eq!(entities[0].relationships.len(), 1);
        assert_eq!(
            entities[0].relationships[0],
            (entities[1].uuid, RelationshipType::Calls)
        );
    }

    #[test]
    fn test_e2e_rust_same_file_function_resolution() {
        let orphans_fn = ResolutionEntity {
            uuid: Uuid::new_v4(),
            kind: EntityKind::Function,
            name: "find_nearest_entity_by_line".to_string(),
            fqn: "knot::pipeline::parser::orphans::find_nearest_entity_by_line".to_string(),
            file_path: "src/pipeline/parser/orphans.rs".to_string(),
            enclosing_class: None,
            enclosing_class_fqn: None,
            signature: None,
            reference_intents: Vec::new(),
            relationships: Vec::new(),
            alias_module_path: None,
            original_export_name: None,
            default_export: None,
            is_test_context: false,
            import_scope: None,
        };

        let rust_fn = ResolutionEntity {
            uuid: Uuid::new_v4(),
            kind: EntityKind::Function,
            name: "find_nearest_entity_by_line".to_string(),
            fqn: "knot::pipeline::parser::languages::rust::find_nearest_entity_by_line".to_string(),
            file_path: "src/pipeline/parser/languages/rust.rs".to_string(),
            enclosing_class: None,
            enclosing_class_fqn: None,
            signature: None,
            reference_intents: Vec::new(),
            relationships: Vec::new(),
            alias_module_path: None,
            original_export_name: None,
            default_export: None,
            is_test_context: false,
            import_scope: None,
        };

        let rust_caller = ResolutionEntity {
            uuid: Uuid::new_v4(),
            kind: EntityKind::Function,
            name: "collect_rust_type_references".to_string(),
            fqn: "knot::pipeline::parser::languages::rust::collect_rust_type_references"
                .to_string(),
            file_path: "src/pipeline/parser/languages/rust.rs".to_string(),
            enclosing_class: None,
            enclosing_class_fqn: None,
            signature: None,
            reference_intents: vec![ReferenceIntent::Call {
                method: "find_nearest_entity_by_line".to_string(),
                receiver: None,
                line: 258,
                arg_count: None,
            }],
            relationships: Vec::new(),
            alias_module_path: None,
            original_export_name: None,
            default_export: None,
            is_test_context: false,
            import_scope: None,
        };

        let orphans_caller = ResolutionEntity {
            uuid: Uuid::new_v4(),
            kind: EntityKind::Function,
            name: "collect_orphaned_references".to_string(),
            fqn: "knot::pipeline::parser::orphans::collect_orphaned_references".to_string(),
            file_path: "src/pipeline/parser/orphans.rs".to_string(),
            enclosing_class: None,
            enclosing_class_fqn: None,
            signature: None,
            reference_intents: vec![ReferenceIntent::Call {
                method: "find_nearest_entity_by_line".to_string(),
                receiver: None,
                line: 8,
                arg_count: None,
            }],
            relationships: Vec::new(),
            alias_module_path: None,
            original_export_name: None,
            default_export: None,
            is_test_context: false,
            import_scope: None,
        };

        let orphans_fn_uuid = orphans_fn.uuid;
        let rust_fn_uuid = rust_fn.uuid;

        let mut entities = vec![orphans_fn, rust_fn, rust_caller, orphans_caller];
        resolve_reference_intents(&mut entities);

        let rust_caller_rels = &entities[2].relationships;
        assert_eq!(rust_caller_rels.len(), 1);
        assert_eq!(rust_caller_rels[0], (rust_fn_uuid, RelationshipType::Calls));

        let orphans_caller_rels = &entities[3].relationships;
        assert_eq!(orphans_caller_rels.len(), 1);
        assert_eq!(
            orphans_caller_rels[0],
            (orphans_fn_uuid, RelationshipType::Calls)
        );
    }

    #[test]
    fn test_resolve_self_method_same_class_name_collision() {
        let module_func = mock_resolution_entity_with_kind(
            "do_thing",
            "do_thing",
            None,
            "lora.py",
            EntityKind::Function,
        );
        let my_class = mock_resolution_entity_with_kind(
            "MyLoader",
            "MyLoader",
            None,
            "nodes.py",
            EntityKind::Class,
        );
        let class_method = mock_resolution_entity_with_kind(
            "do_thing",
            "MyLoader.do_thing",
            Some("MyLoader"),
            "nodes.py",
            EntityKind::Method,
        );

        let mut caller_method = mock_resolution_entity_with_kind(
            "caller",
            "MyLoader.caller",
            Some("MyLoader"),
            "nodes.py",
            EntityKind::Method,
        );
        caller_method.reference_intents = vec![ReferenceIntent::Call {
            method: "do_thing".to_string(),
            receiver: Some("self".to_string()),
            line: 20,
            arg_count: None,
        }];

        let module_uuid = module_func.uuid;
        let class_method_uuid = class_method.uuid;

        let mut entities = vec![module_func, my_class, class_method, caller_method];
        resolve_reference_intents(&mut entities);

        let caller = entities.iter().find(|e| e.name == "caller").unwrap();
        assert!(
            caller
                .relationships
                .contains(&(class_method_uuid, RelationshipType::Calls))
        );
        assert!(
            !caller
                .relationships
                .contains(&(module_uuid, RelationshipType::Calls))
        );
    }

    #[test]
    fn test_resolve_rust_qualified_call_with_homonyms() {
        let widget_a_new =
            mock_resolution_entity_at("new", "WidgetA::new", Some("WidgetA"), "test/sample.rs");
        let widget_b_new =
            mock_resolution_entity_at("new", "WidgetB::new", Some("WidgetB"), "test/sample.rs");

        let mut caller = mock_resolution_entity_with_kind(
            "exercise_qualified_calls",
            "exercise_qualified_calls",
            None,
            "test/sample.rs",
            EntityKind::Function,
        );
        caller.reference_intents = vec![ReferenceIntent::Call {
            method: "new".to_string(),
            receiver: Some("WidgetA".to_string()),
            line: 50,
            arg_count: None,
        }];

        let widget_a_new_uuid = widget_a_new.uuid;
        let widget_b_new_uuid = widget_b_new.uuid;

        let mut entities = vec![widget_a_new, widget_b_new, caller];
        resolve_reference_intents(&mut entities);

        let caller_entity = entities.last().unwrap();
        assert!(
            caller_entity
                .relationships
                .contains(&(widget_a_new_uuid, RelationshipType::Calls))
        );
        assert!(
            !caller_entity
                .relationships
                .contains(&(widget_b_new_uuid, RelationshipType::Calls))
        );
    }

    #[test]
    fn test_resolve_rust_self_method_call() {
        let foo_helper =
            mock_resolution_entity_at("helper", "Foo::helper", Some("Foo"), "test/sample.rs");

        let mut foo_bar =
            mock_resolution_entity_at("bar", "Foo::bar", Some("Foo"), "test/sample.rs");
        foo_bar.reference_intents = vec![ReferenceIntent::Call {
            method: "helper".to_string(),
            receiver: Some("Foo".to_string()),
            line: 30,
            arg_count: None,
        }];

        let foo_helper_uuid = foo_helper.uuid;

        let mut entities = vec![foo_helper, foo_bar];
        resolve_reference_intents(&mut entities);

        let foo_bar_entity = entities.last().unwrap();
        assert!(
            foo_bar_entity
                .relationships
                .contains(&(foo_helper_uuid, RelationshipType::Calls))
        );
    }

    /// A crate-anchored Rust method (`<crate>::<module>::<Type>::<method>`)
    /// as the Rust qualifier emits it, with its `enclosing_class_fqn`.
    fn rust_method(name: &str, type_fqn: &str, file_path: &str) -> ResolutionEntity {
        let type_name = type_fqn.rsplit("::").next().unwrap();
        let mut entity = mock_resolution_entity_with_kind(
            name,
            &format!("{type_fqn}::{name}"),
            Some(type_name),
            file_path,
            EntityKind::RustMethod,
        );
        entity.enclosing_class_fqn = Some(type_fqn.to_string());
        entity
    }

    fn self_call(method: &str) -> ReferenceIntent {
        ReferenceIntent::Call {
            method: method.to_string(),
            receiver: Some("self".to_string()),
            line: 1,
            arg_count: None,
        }
    }

    /// Issue #36: `self.helper()` must bind to the enclosing type's method
    /// through its crate-anchored FQN, never to a homonym on another type in
    /// the same file nor to the same-named type in another module.
    #[test]
    fn test_resolve_rust_self_call_with_crate_anchored_fqns() {
        let own = rust_method("compute_value", "shop::PriceAdjuster", "src/lib.rs");
        let same_file_homonym =
            rust_method("compute_value", "shop::DiscountAdjuster", "src/lib.rs");
        let other_module = rust_method(
            "compute_value",
            "shop::other::PriceAdjuster",
            "src/other.rs",
        );
        let mut caller = rust_method("adjust", "shop::PriceAdjuster", "src/lib.rs");
        caller.reference_intents = vec![self_call("compute_value")];
        let mut other_caller =
            rust_method("recompute", "shop::other::PriceAdjuster", "src/other.rs");
        other_caller.reference_intents = vec![self_call("compute_value")];

        let own_uuid = own.uuid;
        let other_module_uuid = other_module.uuid;

        let mut entities = vec![same_file_homonym, other_module, own, caller, other_caller];
        resolve_reference_intents(&mut entities);

        assert_eq!(
            entities[3].relationships,
            vec![(own_uuid, RelationshipType::Calls)]
        );
        assert_eq!(
            entities[4].relationships,
            vec![(other_module_uuid, RelationshipType::Calls)]
        );
    }

    /// Without `enclosing_class_fqn` (e.g. fixtures with no crate root), a
    /// `self` call still resolves through the short type name.
    #[test]
    fn test_resolve_rust_self_call_without_enclosing_fqn() {
        let own = rust_method("helper", "__loose::sample::Foo", "sample.rs");
        let homonym = rust_method("helper", "__loose::sample::Bar", "sample.rs");
        let mut caller = rust_method("run", "__loose::sample::Foo", "sample.rs");
        caller.enclosing_class_fqn = None;
        caller.reference_intents = vec![self_call("helper")];

        let own_uuid = own.uuid;
        let mut entities = vec![homonym, own, caller];
        resolve_reference_intents(&mut entities);

        assert_eq!(
            entities[2].relationships,
            vec![(own_uuid, RelationshipType::Calls)]
        );
    }

    fn receiver_call(method: &str, receiver: &str) -> ReferenceIntent {
        ReferenceIntent::Call {
            method: method.to_string(),
            receiver: Some(receiver.to_string()),
            line: 1,
            arg_count: None,
        }
    }

    /// Resolve `intents` issued by a Rust method of `shop::PriceAdjuster`
    /// against `targets` and return the FQNs it ends up calling.
    fn rust_call_targets(
        targets: Vec<ResolutionEntity>,
        intents: Vec<ReferenceIntent>,
    ) -> Vec<String> {
        let mut caller = rust_method("caller", "shop::PriceAdjuster", "src/lib.rs");
        caller.reference_intents = intents;
        let mut entities = targets;
        entities.push(caller);
        resolve_reference_intents(&mut entities);

        let caller = entities.last().unwrap();
        let mut fqns: Vec<String> = caller
            .relationships
            .iter()
            .filter_map(|(uuid, _)| entities.iter().find(|e| e.uuid == *uuid))
            .map(|e| e.fqn.clone())
            .collect();
        fqns.sort();
        fqns
    }

    /// Issue #36: a std method on a receiver of unknown type
    /// (`skus.iter().collect()`, `self.label.as_str()`) must never bind to
    /// the only user-defined method that happens to share its name.
    #[test]
    fn test_resolve_rust_compound_receiver_skips_name_only_fallback() {
        let targets = vec![
            rust_method("collect", "shop::recall::Pool", "src/recall.rs"),
            rust_method("as_str", "shop::query::Tier", "src/query.rs"),
            mock_resolution_entity_at(
                "find",
                "AlphaSearchService.find",
                Some("AlphaSearchService"),
                "web/alpha.ts",
            ),
        ];
        let intents = vec![
            receiver_call("collect", "skus.iter().map()"),
            receiver_call("as_str", "self.label"),
            receiver_call("find", "self.items.iter()"),
        ];
        assert!(rust_call_targets(targets, intents).is_empty());
    }

    /// `self.m()` with no `m` on the enclosing type (e.g. a derived
    /// `self.clone()`) is terminal: no fallback to a same-named method.
    #[test]
    fn test_resolve_rust_self_call_is_terminal() {
        let targets = vec![rust_method("clone", "shop::Other", "src/lib.rs")];
        let intents = vec![self_call("clone")];
        assert!(rust_call_targets(targets, intents).is_empty());
    }

    /// The last path segment of a compound receiver is evidence of its type:
    /// `self.repo.save()` → `Repo::save`, `self.price_rules().apply()` →
    /// `PriceRules::apply` (snake_case getter/field named after its type).
    #[test]
    fn test_resolve_rust_compound_receiver_by_last_segment_type() {
        let targets = vec![
            rust_method("save", "shop::store::Repo", "src/store.rs"),
            rust_method("save", "shop::Draft", "src/lib.rs"),
            rust_method("apply", "shop::rules::PriceRules", "src/rules.rs"),
        ];
        let intents = vec![
            receiver_call("save", "self.repo"),
            receiver_call("apply", "self.price_rules()"),
        ];
        assert_eq!(
            rust_call_targets(targets, intents),
            vec!["shop::rules::PriceRules::apply", "shop::store::Repo::save"]
        );
    }

    /// A plain identifier receiver keeps the pre-existing resolution ladder
    /// (including the name-only fallback) — only compound receivers, which
    /// the Rust parser used to drop entirely, are evidence-only.
    #[test]
    fn test_resolve_rust_identifier_receiver_unchanged() {
        let targets = vec![rust_method(
            "adjust",
            "shop::pricing::PriceAdjuster",
            "src/pricing.rs",
        )];
        let intents = vec![receiver_call("adjust", "adjuster")];
        assert_eq!(
            rust_call_targets(targets, intents),
            vec!["shop::pricing::PriceAdjuster::adjust"]
        );
    }

    #[test]
    fn test_resolve_fallback_uniqueness_guard() {
        let mut caller = mock_resolution_entity("main", "App.main", None);
        let callee1 =
            mock_resolution_entity_at("new", "Class1.new", Some("Class1"), "test/file1.java");
        let callee2 =
            mock_resolution_entity_at("new", "Class2.new", Some("Class2"), "test/file2.java");

        caller.reference_intents.push(ReferenceIntent::Call {
            method: "new".to_string(),
            receiver: None,
            line: 10,
            arg_count: None,
        });

        let mut entities = vec![caller, callee1, callee2];
        resolve_reference_intents(&mut entities);

        assert_eq!(entities[0].relationships.len(), 0);
    }

    /// Arity is a filter, not evidence (issue #35): two `add` homonyms in
    /// other files, one of them with the call's arity, are still ambiguous.
    /// The repository-wide "unique by arity" pick bound every `x.isEmpty(y)`
    /// to the only one-parameter `isEmpty` of an unrelated class.
    #[test]
    fn test_resolve_fallback_arg_count_alone_is_not_evidence() {
        let mut caller = mock_resolution_entity("main", "App.main", None);
        let mut callee1 =
            mock_resolution_entity_at("add", "Calc.add", Some("Calc"), "test/calc.java");
        callee1.signature = Some("add(int, int)".to_string()); // 2 args

        let mut callee2 =
            mock_resolution_entity_at("add", "List.add", Some("List"), "test/list.java");
        callee2.signature = Some("add(int)".to_string()); // 1 arg

        caller.reference_intents.push(ReferenceIntent::Call {
            method: "add".to_string(),
            receiver: None,
            line: 10,
            arg_count: Some(2),
        });

        let mut entities = vec![caller, callee1, callee2];
        resolve_reference_intents(&mut entities);

        assert!(entities[0].relationships.is_empty());
    }

    /// A same-file homonym is only bound when its arity does not contradict
    /// the call, so a test method `chat()` calling `api().chat(request)` no
    /// longer gets a false self-recursion edge.
    #[test]
    fn test_resolve_fallback_same_file_respects_arity() {
        let mut caller =
            mock_resolution_entity_at("chat", "ApiIT.chat", Some("ApiIT"), "test/ApiIT.java");
        caller.signature = Some("()".to_string());
        caller.reference_intents.push(ReferenceIntent::Call {
            method: "chat".to_string(),
            receiver: None,
            line: 10,
            arg_count: Some(1),
        });
        let mut helper =
            mock_resolution_entity_at("chat", "ApiIT.chat", Some("ApiIT"), "test/ApiIT.java");
        helper.signature = Some("(String prompt)".to_string());
        let mut other = mock_resolution_entity_at("chat", "Api.chat", Some("Api"), "test/Api.java");
        other.signature = Some("(Request request)".to_string());

        let helper_uuid = helper.uuid;
        let mut entities = vec![caller, helper, other];
        resolve_reference_intents(&mut entities);

        assert_eq!(
            entities[0].relationships,
            vec![(helper_uuid, RelationshipType::Calls)]
        );
    }

    /// Resolve the local `batch` against itself plus the dependency
    /// entities `(fqn, name)`, loaded from the graph as in a cross-repo run;
    /// `local_types` are local type names loaded from the graph (declared in
    /// files outside the batch). Returns the FQNs `batch[caller]` calls,
    /// dependency targets prefixed with `dep:`.
    fn cross_repo_targets(
        mut batch: Vec<ResolutionEntity>,
        caller: usize,
        dependencies: &[(&str, &str)],
        local_types: &[&str],
    ) -> Vec<String> {
        let mut mappings = crate::db::graph::EntityMappings::default();
        let mut dep_fqns: HashMap<Uuid, String> = HashMap::new();
        for (fqn, name) in dependencies {
            let uuid = Uuid::new_v4();
            mappings.fqn_to_uuid.insert(fqn.to_string(), uuid);
            mappings
                .name_to_uuids
                .entry(name.to_string())
                .or_default()
                .push(uuid);
            mappings.dependency_uuids.insert(uuid);
            dep_fqns.insert(uuid, format!("dep:{fqn}"));
        }
        mappings
            .local_type_names
            .extend(local_types.iter().map(|t| t.to_string()));
        crate::pipeline::ingest::resolve::resolve_reference_intents_with_mappings(
            &mut batch, mappings,
        );
        let mut targets: Vec<String> = batch[caller]
            .relationships
            .iter()
            .filter_map(|(uuid, _)| {
                dep_fqns.get(uuid).cloned().or_else(|| {
                    batch
                        .iter()
                        .find(|e| e.uuid == *uuid)
                        .map(|e| e.fqn.clone())
                })
            })
            .collect();
        targets.sort();
        targets
    }

    /// Issue #34 (knot-lite): the consumer's own `Tool`, whose `handle` is
    /// generated by a macro and so not indexed, must not bind
    /// `Tool::handle(..)` to the dependency's same-named `Tool::handle`, not
    /// even when `handle` is declared nowhere else. A type only the
    /// dependency declares still resolves across repositories.
    #[test]
    fn test_local_type_shadows_dependency_homonym() {
        let local_tool = mock_resolution_entity_with_kind(
            "Tool",
            "lite::tools::Tool",
            None,
            "src/tools.rs",
            EntityKind::RustStruct,
        );
        let mut dispatch = mock_resolution_entity_with_kind(
            "dispatch",
            "lite::dispatch",
            None,
            "src/main.rs",
            EntityKind::RustFunction,
        );
        dispatch.reference_intents = vec![
            receiver_call("handle", "Tool"),
            receiver_call("ping", "Shared"),
        ];
        let dependencies = [
            ("core::tools::Tool::handle", "handle"),
            ("core::tools::Tool", "Tool"),
            ("core::Shared::ping", "ping"),
        ];
        assert_eq!(
            cross_repo_targets(vec![local_tool, dispatch], 1, &dependencies, &[]),
            vec!["dep:core::Shared::ping"]
        );
    }

    /// The Java variant: the local `AuditLog` is declared in a file outside
    /// the batch (known from the graph). A declared-type receiver
    /// (`AuditLog`), an untyped one (`auditLog`) and a homonym `record` of an
    /// unrelated local class must all stay unresolved.
    #[test]
    fn test_local_type_from_graph_shadows_dependency_homonym() {
        let mut audit = java_method("com.app.AuditController", "audit", "(String event)");
        audit.reference_intents = vec![receiver_call("record", "AuditLog")];
        let mut audit_untyped = java_method("com.app.AuditController", "auditUntyped", "()");
        audit_untyped.reference_intents = vec![receiver_call("record", "auditLog")];
        let other = java_method("com.app.Recorder", "record", "(String event)");
        let dependencies = [("com.lib.AuditLog.record", "record")];
        for caller in [0, 1] {
            let batch = vec![audit.clone(), audit_untyped.clone(), other.clone()];
            assert!(
                cross_repo_targets(batch, caller, &dependencies, &["AuditLog"]).is_empty(),
                "caller {caller}"
            );
        }
    }

    /// Without a local type of that name, the dependency member is the only
    /// sensible target and keeps resolving (the existing cross-repo
    /// behaviour).
    #[test]
    fn test_dependency_type_resolves_without_local_homonym() {
        let mut audit = java_method("com.app.AuditController", "audit", "(String event)");
        audit.reference_intents = vec![receiver_call("record", "AuditLog")];
        let dependencies = [("com.lib.AuditLog.record", "record")];
        assert_eq!(
            cross_repo_targets(vec![audit], 0, &dependencies, &[]),
            vec!["dep:com.lib.AuditLog.record"]
        );
    }

    /// Issue #32: a receiver of a type no indexed repository declares (JDK,
    /// standard library, third-party package) never binds to the only
    /// repository method sharing the callee's name, whether the type is
    /// named statically (`String.valueOf`), through a field's declared type
    /// (`ModelMapper`) or by a qualified name.
    #[test]
    fn test_external_type_receiver_skips_unique_homonym() {
        for (receiver, method) in [
            ("String", "valueOf"),
            ("ModelMapper", "map"),
            ("java.util.Objects", "requireNonNull"),
            ("std::string::String", "from"),
        ] {
            let mut caller = java_method("com.app.ExampleResource", "read", "(Entity e)");
            caller.reference_intents = vec![receiver_call(method, receiver)];
            let homonym = java_method("com.other.SomeEventMapper", method, "(Object value)");
            let mut entities = vec![caller, homonym];
            resolve_reference_intents(&mut entities);
            assert!(entities[0].relationships.is_empty(), "{receiver}.{method}");
        }
    }

    /// A capitalized receiver naming an indexed entity (a C# property) is
    /// not known to be external, and a same-file homonym (`Self::build()`)
    /// still wins: both keep the fallback.
    #[test]
    fn test_indexed_or_same_file_receiver_keeps_homonym_fallback() {
        let mut place = mock_resolution_entity_with_kind(
            "Place",
            "Shop.OrderService.Place",
            Some("OrderService"),
            "src/OrderService.cs",
            EntityKind::CSharpMethod,
        );
        place.reference_intents = vec![receiver_call("Save", "Repository")];
        let property = mock_resolution_entity_with_kind(
            "Repository",
            "Shop.OrderService.Repository",
            Some("OrderService"),
            "src/OrderService.cs",
            EntityKind::CSharpProperty,
        );
        let save = mock_resolution_entity_with_kind(
            "Save",
            "Shop.OrderRepository.Save",
            Some("OrderRepository"),
            "src/OrderRepository.cs",
            EntityKind::CSharpMethod,
        );
        let save_uuid = save.uuid;
        let mut entities = vec![place, property, save];
        resolve_reference_intents(&mut entities);
        assert_eq!(
            entities[0].relationships,
            vec![(save_uuid, RelationshipType::Calls)]
        );

        let mut checkout = mock_resolution_entity_with_kind(
            "checkout",
            "shop::Cart::checkout",
            Some("Cart"),
            "src/cart.rs",
            EntityKind::RustFunction,
        );
        checkout.reference_intents = vec![receiver_call("build", "Self")];
        let build = mock_resolution_entity_with_kind(
            "build",
            "shop::Cart::build",
            Some("Cart"),
            "src/cart.rs",
            EntityKind::RustFunction,
        );
        let build_uuid = build.uuid;
        let other = mock_resolution_entity_with_kind(
            "build",
            "shop::Order::build",
            Some("Order"),
            "src/order.rs",
            EntityKind::RustFunction,
        );
        let mut entities = vec![checkout, build, other];
        resolve_reference_intents(&mut entities);
        assert_eq!(
            entities[0].relationships,
            vec![(build_uuid, RelationshipType::Calls)]
        );
    }

    /// Java reaches a Kotlin `object` through its synthetic `INSTANCE`: the
    /// unindexed last segment is not external because `StringUtils` is
    /// indexed, so the receiver chain still binds the call.
    #[test]
    fn test_indexed_receiver_segment_keeps_receiver_chain() {
        let mut greet = java_method("com.example.java.JavaUser", "greet", "(String name)");
        greet.reference_intents = vec![receiver_call("capitalize", "StringUtils.INSTANCE")];
        let object = mock_resolution_entity_with_kind(
            "StringUtils",
            "com.example.kotlin.StringUtils",
            None,
            "src/StringUtils.kt",
            EntityKind::KotlinObject,
        );
        let capitalize = mock_resolution_entity_at(
            "capitalize",
            "com.example.kotlin.StringUtils.capitalize",
            Some("StringUtils"),
            "src/StringUtils.kt",
        );
        let other = java_method("com.example.text.Words", "capitalize", "(String word)");
        let capitalize_uuid = capitalize.uuid;
        let mut entities = vec![greet, object, capitalize, other];
        resolve_reference_intents(&mut entities);
        assert_eq!(
            entities[0].relationships,
            vec![(capitalize_uuid, RelationshipType::Calls)]
        );
    }

    /// `<package>/<Class>.java` for a JVM FQN `<package>.<Class>[.<member>]`.
    fn jvm_entity(fqn: &str, class_fqn: &str, kind: EntityKind) -> ResolutionEntity {
        let (package, class) = class_fqn.rsplit_once('.').unwrap();
        let name = fqn.rsplit('.').next().unwrap();
        let enclosing = (fqn != class_fqn).then_some(class);
        mock_resolution_entity_with_kind(
            name,
            fqn,
            enclosing,
            &format!("src/{}/{class}.java", package.replace('.', "/")),
            kind,
        )
    }

    /// `com.a.Config` and `com.b.Config` both declare `load`; `reset` is
    /// declared by `com.a.Config` and inherited by `com.b.Config` from
    /// `com.b.Base`.
    fn config_homonyms() -> Vec<ResolutionEntity> {
        let mut b_config = jvm_entity("com.b.Config", "com.b.Config", EntityKind::Class);
        b_config.reference_intents.push(ReferenceIntent::Extends {
            parent: "Base".to_string(),
            line: 1,
        });
        vec![
            jvm_entity("com.a.Config", "com.a.Config", EntityKind::Class),
            jvm_entity("com.a.Config.load", "com.a.Config", EntityKind::Method),
            jvm_entity("com.a.Config.reset", "com.a.Config", EntityKind::Method),
            b_config,
            jvm_entity("com.b.Config.load", "com.b.Config", EntityKind::Method),
            jvm_entity("com.b.Base", "com.b.Base", EntityKind::Class),
            jvm_entity("com.b.Base.reset", "com.b.Base", EntityKind::Method),
        ]
    }

    /// Resolve `intent` issued by `<package>.Caller.run` (whose file has
    /// `scope`) against [`config_homonyms`]; returns the target FQNs.
    fn scoped_targets(package: &str, scope: ImportScope, intent: ReferenceIntent) -> Vec<String> {
        let mut caller = jvm_entity(
            &format!("{package}.Caller.run"),
            &format!("{package}.Caller"),
            EntityKind::Method,
        );
        caller.import_scope = Some(std::sync::Arc::new(scope));
        caller.reference_intents = vec![intent];
        let mut entities = config_homonyms();
        entities.push(caller);
        resolve_reference_intents(&mut entities);
        let caller = entities.last().unwrap();
        caller
            .relationships
            .iter()
            .map(|(uuid, _)| {
                entities
                    .iter()
                    .find(|e| e.uuid == *uuid)
                    .unwrap()
                    .fqn
                    .clone()
            })
            .collect()
    }

    fn import_scope(package: &str, imports: &[(&str, &str)], wildcards: &[&str]) -> ImportScope {
        ImportScope {
            package: Some(package.to_string()),
            imports: imports
                .iter()
                .map(|(local, fqn)| (local.to_string(), fqn.to_string()))
                .collect(),
            wildcards: wildcards.iter().map(|w| w.to_string()).collect(),
            module_bindings: Vec::new(),
        }
    }

    /// Issue #45: a call on a type with homonyms in other packages binds to
    /// the one the caller's file imports (by name or Kotlin alias), else to
    /// its own package's, else to an on-demand import's — for declared-type
    /// receivers (`Config`) and untyped ones named after the type (`config`).
    #[test]
    fn test_imports_pick_the_homonym_type() {
        let b_import = [("Config", "com.b.Config")];
        for (package, scope, receiver) in [
            ("com.app", import_scope("com.app", &b_import, &[]), "Config"),
            ("com.app", import_scope("com.app", &b_import, &[]), "config"),
            (
                "com.app",
                import_scope("com.app", &[("BConfig", "com.b.Config")], &[]),
                "BConfig",
            ),
            ("com.b", import_scope("com.b", &[], &[]), "Config"),
            (
                "com.app",
                import_scope("com.app", &[], &["com.b"]),
                "Config",
            ),
        ] {
            assert_eq!(
                scoped_targets(package, scope, receiver_call("load", receiver)),
                vec!["com.b.Config.load"],
                "{package} {receiver}"
            );
        }
    }

    /// The imported type's own supertypes are walked: `reset` binds to
    /// `com.b.Base.reset`, never to the homonym `com.a.Config.reset`.
    #[test]
    fn test_imported_type_walks_its_own_supertypes() {
        assert_eq!(
            scoped_targets(
                "com.app",
                import_scope("com.app", &[("Config", "com.b.Config")], &[]),
                receiver_call("reset", "Config")
            ),
            vec!["com.b.Base.reset"]
        );
    }

    /// An import of a type no indexed repository declares pins the call to
    /// that external type: no repository homonym is bound.
    #[test]
    fn test_external_import_binds_no_homonym() {
        for receiver in ["Config", "config"] {
            assert!(
                scoped_targets(
                    "com.app",
                    import_scope("com.app", &[("Config", "org.lib.Config")], &[]),
                    receiver_call("reset", receiver)
                )
                .is_empty(),
                "{receiver}"
            );
        }
    }

    /// Type references follow the same imports: `Config` is ambiguous
    /// without them and the imported one with them.
    #[test]
    fn test_imports_pick_the_referenced_homonym_type() {
        let type_ref = || ReferenceIntent::TypeReference {
            type_name: "Config".to_string(),
            line: 1,
        };
        assert_eq!(
            scoped_targets(
                "com.app",
                import_scope("com.app", &[("Config", "com.b.Config")], &[]),
                type_ref()
            ),
            vec!["com.b.Config"]
        );
        assert!(scoped_targets("com.app", ImportScope::default(), type_ref()).is_empty());
    }

    /// A Java method `<class_fqn>.<name><params>` declared in `<Class>.java`.
    fn java_method(class_fqn: &str, name: &str, params: &str) -> ResolutionEntity {
        let class = class_fqn.rsplit('.').next().unwrap();
        let mut entity = mock_resolution_entity_at(
            name,
            &format!("{class_fqn}.{name}"),
            Some(class),
            &format!("src/{class}.java"),
        );
        entity.signature = Some(params.to_string());
        entity
    }

    /// Resolve one call `<receiver>.<method>(<arg_count> args)` issued from
    /// `com.app.App.main` and return the signature + FQN of every target.
    fn overload_targets(
        targets: Vec<ResolutionEntity>,
        receiver: &str,
        method: &str,
        arg_count: usize,
    ) -> Vec<String> {
        let mut caller = java_method("com.app.App", "main", "()");
        caller.reference_intents.push(ReferenceIntent::Call {
            method: method.to_string(),
            receiver: Some(receiver.to_string()),
            line: 1,
            arg_count: Some(arg_count),
        });
        let mut entities = targets;
        entities.push(caller);
        resolve_reference_intents(&mut entities);
        let caller = entities.last().unwrap();
        caller
            .relationships
            .iter()
            .filter_map(|(uuid, _)| entities.iter().find(|e| e.uuid == *uuid))
            .map(|e| format!("{}{}", e.fqn, e.signature.as_deref().unwrap_or("")))
            .collect()
    }

    /// Issue #35: overloads share one FQN and `fqn_to_uuid` keeps only the
    /// last one declared. With the wrong-arity overload last, the call used
    /// to bind to a same-name/same-arity method of an unrelated class (or to
    /// the wrong overload), depending on random UUID order. Both declaration
    /// orders, repeated so a lucky UUID order cannot hide the bug.
    #[test]
    fn test_overload_disambiguation_stays_in_receiver_class() {
        for wrong_arity_last in [true, false] {
            for _ in 0..20 {
                let pair = java_method("com.x.Foo", "bar", "(int a, int b)");
                let single = java_method("com.x.Foo", "bar", "(int a)");
                let mut targets = vec![java_method("com.other.Other", "bar", "(int a, int b)")];
                if wrong_arity_last {
                    targets.extend([pair, single]);
                } else {
                    targets.extend([single, pair]);
                }
                assert_eq!(
                    overload_targets(targets, "Foo", "bar", 2),
                    vec!["com.x.Foo.bar(int a, int b)"],
                    "wrong_arity_last={wrong_arity_last}"
                );
            }
        }
    }

    /// An untyped lowercase receiver (`foo.bar(1, 2)` → `Foo.bar`) matched by
    /// FQN segment still picks the overload with the call's arity.
    #[test]
    fn test_overload_disambiguation_on_untyped_receiver() {
        for _ in 0..20 {
            let targets = vec![
                java_method("com.x.Foo", "bar", "(int a, int b)"),
                java_method("com.x.Foo", "bar", "(int a)"),
            ];
            assert_eq!(
                overload_targets(targets, "foo", "bar", 2),
                vec!["com.x.Foo.bar(int a, int b)"]
            );
        }
    }

    /// An overload with the call's arity inherited from a supertype wins over
    /// the receiver's own different-arity overload, and a same-named method
    /// of an unrelated class never does.
    #[test]
    fn test_overload_disambiguation_walks_supertypes() {
        for _ in 0..20 {
            let mut child_class = mock_resolution_entity_with_kind(
                "Child",
                "com.x.Child",
                None,
                "src/Child.java",
                EntityKind::Class,
            );
            child_class
                .reference_intents
                .push(ReferenceIntent::Extends {
                    parent: "com.x.Base".to_string(),
                    line: 1,
                });
            let targets = vec![
                java_method("com.other.Other", "scale", "(int a, int b)"),
                java_method("com.x.Base", "scale", "(int a, int b)"),
                child_class,
                java_method("com.x.Child", "scale", "(int a)"),
            ];
            assert_eq!(
                overload_targets(targets, "Child", "scale", 2),
                vec!["com.x.Base.scale(int a, int b)"]
            );
        }
    }

    /// Issue #41: `this.url()` in a subclass binds to the method inherited
    /// from its superclass, even when an unrelated class declares a
    /// same-named method — through every supertype level, nearest first.
    #[test]
    fn test_this_call_walks_supertypes() {
        let ts_class = |name: &str, parent: Option<&str>| {
            let mut class = mock_resolution_entity_with_kind(
                name,
                name,
                None,
                &format!("src/{name}.ts"),
                EntityKind::Class,
            );
            if let Some(parent) = parent {
                class.reference_intents.push(ReferenceIntent::Extends {
                    parent: parent.to_string(),
                    line: 1,
                });
            }
            class
        };
        let ts_method = |class: &str, name: &str| {
            mock_resolution_entity_at(
                name,
                &format!("{class}.{name}"),
                Some(class),
                &format!("src/{class}.ts"),
            )
        };
        let mut delete_cookie = ts_method("CdpPage", "deleteCookie");
        delete_cookie.reference_intents = vec![receiver_call("url", "this")];
        let mut reload = ts_method("CdpPage", "reload");
        reload.reference_intents = vec![receiver_call("close", "this")];
        let page_url = ts_method("Page", "url");
        let base_close = ts_method("Base", "close");
        let (page_url_uuid, base_close_uuid) = (page_url.uuid, base_close.uuid);
        let mut entities = vec![
            delete_cookie,
            reload,
            ts_class("CdpPage", Some("Page")),
            ts_class("Page", Some("Base")),
            ts_class("Base", None),
            ts_class("Frame", None),
            page_url,
            base_close,
            ts_method("Frame", "url"),
            ts_method("Frame", "close"),
        ];
        resolve_reference_intents(&mut entities);
        assert_eq!(
            entities[0].relationships,
            vec![(page_url_uuid, RelationshipType::Calls)]
        );
        assert_eq!(
            entities[1].relationships,
            vec![(base_close_uuid, RelationshipType::Calls)]
        );
    }

    /// Overloads span the class hierarchy only in Java, Kotlin, Groovy and C#.
    /// In TypeScript a subclass method replaces the inherited one whatever
    /// its arity, so the receiver's own method is kept (issue #38).
    #[test]
    fn test_overload_disambiguation_typescript_keeps_own_method() {
        for _ in 0..20 {
            let ts_method = |class: &str, params: &str| {
                let mut e = mock_resolution_entity_at(
                    "scale",
                    &format!("{class}.scale"),
                    Some(class),
                    &format!("src/{class}.ts"),
                );
                e.signature = Some(params.to_string());
                e
            };
            let mut child_class = mock_resolution_entity_with_kind(
                "Child",
                "Child",
                None,
                "src/Child.ts",
                EntityKind::Class,
            );
            child_class
                .reference_intents
                .push(ReferenceIntent::Extends {
                    parent: "Base".to_string(),
                    line: 1,
                });
            let targets = vec![
                ts_method("Base", "(a: number, b: number)"),
                child_class,
                ts_method("Child", "(a: number, b?: number)"),
            ];
            assert_eq!(
                overload_targets(targets, "Child", "scale", 2),
                vec!["Child.scale(a: number, b?: number)"]
            );
        }
    }

    /// With no overload of the call's arity in the receiver class or its
    /// supertypes, the receiver's own method is kept instead of jumping to
    /// another class (varargs, defaults and unparsed signatures land here).
    #[test]
    fn test_overload_disambiguation_keeps_candidate_without_match() {
        for _ in 0..20 {
            let targets = vec![
                java_method("com.other.Other", "log", "(String a, String b)"),
                java_method("com.x.Foo", "log", "(String... parts)"),
            ];
            assert_eq!(
                overload_targets(targets, "Foo", "log", 2),
                vec!["com.x.Foo.log(String... parts)"]
            );
        }
    }

    /// A method declared in `file_path` with `params`, called with
    /// `call_arity` arguments from `caller_file` without a receiver: the FQN
    /// of every target it binds to.
    fn bare_call_targets(
        caller_file: &str,
        file_path: &str,
        params: &str,
        call_arity: usize,
    ) -> Vec<String> {
        let mut caller = mock_resolution_entity_at("main", "app.main", None, caller_file);
        if caller_file != file_path {
            caller.import_scope = Some(std::sync::Arc::new(ImportScope {
                module_bindings: vec!["helper".to_string()],
                ..ImportScope::default()
            }));
        }
        caller.reference_intents.push(ReferenceIntent::Call {
            method: "helper".to_string(),
            receiver: None,
            line: 1,
            arg_count: Some(call_arity),
        });
        let mut helper = mock_resolution_entity_at("helper", "lib.helper", None, file_path);
        helper.signature = Some(params.to_string());
        let mut entities = vec![helper, caller];
        resolve_reference_intents(&mut entities);
        entities[1]
            .relationships
            .iter()
            .filter_map(|(uuid, _)| entities.iter().find(|e| e.uuid == *uuid))
            .map(|e| e.fqn.clone())
            .collect()
    }

    /// Issue #38: JavaScript never enforces arity, so neither a JS caller's
    /// argument count nor a JS target's declared parameters rule a candidate
    /// out. TypeScript does enforce it.
    #[test]
    fn test_javascript_arity_is_never_binding() {
        assert_eq!(
            bare_call_targets("src/a.js", "src/a.js", "(a, b)", 1),
            vec!["lib.helper"]
        );
        assert_eq!(
            bare_call_targets("src/a.ts", "src/b.mjs", "(a, b)", 1),
            vec!["lib.helper"]
        );
        assert!(bare_call_targets("src/a.ts", "src/a.ts", "(a: number, b: number)", 1).is_empty());
    }

    /// A trailing array parameter of a Groovy method is a varargs parameter;
    /// in Java it is a plain array.
    #[test]
    fn test_groovy_trailing_array_parameter_is_varargs() {
        assert_eq!(
            bare_call_targets(
                "src/A.groovy",
                "src/A.groovy",
                "void helper(String[] parts)",
                3
            ),
            vec!["lib.helper"]
        );
        assert!(bare_call_targets("src/A.java", "src/A.java", "(String[] parts)", 3).is_empty());
    }

    /// A varargs method accepts any arity: the arity filter of the homonym
    /// fallback never rules it out, and an exact-arity overload beats it.
    #[test]
    fn test_varargs_method_is_never_ruled_out_by_arity() {
        for _ in 0..20 {
            let mut caller = java_method("com.app.App", "main", "()");
            caller.reference_intents.push(ReferenceIntent::Call {
                method: "format".to_string(),
                receiver: None,
                line: 1,
                arg_count: Some(3),
            });
            let varargs = java_method("com.x.Text", "format", "(String fmt, Object... args)");
            let varargs_uuid = varargs.uuid;
            let mut entities = vec![varargs, caller];
            resolve_reference_intents(&mut entities);
            assert_eq!(
                entities[1].relationships,
                vec![(varargs_uuid, RelationshipType::Calls)]
            );

            let targets = vec![
                java_method("com.x.Text", "join", "(String... parts)"),
                java_method("com.x.Text", "join", "(String a, String b)"),
            ];
            assert_eq!(
                overload_targets(targets.clone(), "Text", "join", 2),
                vec!["com.x.Text.join(String a, String b)"]
            );
            assert_eq!(
                overload_targets(targets, "Text", "join", 3),
                vec!["com.x.Text.join(String... parts)"],
                "only the varargs overload accepts three arguments"
            );
        }
    }

    /// Target arity comes from the #30 parameter parser: commas inside
    /// generics and annotation arguments do not count as parameters.
    #[test]
    fn test_overload_disambiguation_counts_generic_parameters() {
        for _ in 0..20 {
            let targets = vec![
                java_method("com.x.Foo", "put", "(int a, int b)"),
                java_method("com.x.Foo", "put", "(Map<String, Integer> values)"),
                java_method(
                    "com.x.Foo",
                    "put",
                    "(@Named(\"a\") int a, @Named(\"b\") int b, int c)",
                ),
            ];
            assert_eq!(
                overload_targets(targets.clone(), "Foo", "put", 1),
                vec!["com.x.Foo.put(Map<String, Integer> values)"]
            );
            assert_eq!(
                overload_targets(targets, "Foo", "put", 3),
                vec!["com.x.Foo.put(@Named(\"a\") int a, @Named(\"b\") int b, int c)"]
            );
        }
    }

    // ============================================================
    // Spec: docs/specs/python_constructor_call_resolution.md
    // ClassName(...) must resolve to ClassName.__init__
    // ============================================================

    #[test]
    fn test_python_class_call_redirects_to_init() {
        let foo_class = mock_resolution_entity_with_kind(
            "Foo",
            "mod.Foo",
            None,
            "test/mod.py",
            EntityKind::Class,
        );
        let foo_init = mock_resolution_entity_with_kind(
            "__init__",
            "mod.Foo.__init__",
            Some("Foo"),
            "test/mod.py",
            EntityKind::Method,
        );

        let mut factory = mock_resolution_entity_with_kind(
            "factory",
            "mod.factory",
            None,
            "test/mod.py",
            EntityKind::Function,
        );
        factory.reference_intents.push(ReferenceIntent::Call {
            method: "Foo".to_string(),
            receiver: None,
            line: 10,
            arg_count: None,
        });

        let foo_class_uuid = foo_class.uuid;
        let foo_init_uuid = foo_init.uuid;

        let mut entities = vec![foo_class, foo_init, factory];
        resolve_reference_intents(&mut entities);

        let factory_entity = entities.iter().find(|e| e.name == "factory").unwrap();
        assert!(
            factory_entity
                .relationships
                .contains(&(foo_init_uuid, RelationshipType::Calls)),
            "factory should call Foo.__init__, not Foo"
        );
        assert!(
            !factory_entity
                .relationships
                .contains(&(foo_class_uuid, RelationshipType::Calls)),
            "factory must NOT call the Foo class entity itself"
        );
    }

    #[test]
    fn test_python_class_call_no_init_keeps_class() {
        let bar_class = mock_resolution_entity_with_kind(
            "Bar",
            "mod.Bar",
            None,
            "test/mod.py",
            EntityKind::Class,
        );

        let mut caller = mock_resolution_entity_with_kind(
            "caller",
            "mod.caller",
            None,
            "test/mod.py",
            EntityKind::Function,
        );
        caller.reference_intents.push(ReferenceIntent::Call {
            method: "Bar".to_string(),
            receiver: None,
            line: 5,
            arg_count: None,
        });

        let bar_class_uuid = bar_class.uuid;

        let mut entities = vec![bar_class, caller];
        resolve_reference_intents(&mut entities);

        let caller_entity = entities.iter().find(|e| e.name == "caller").unwrap();
        assert_eq!(caller_entity.relationships.len(), 1);
        assert_eq!(
            caller_entity.relationships[0],
            (bar_class_uuid, RelationshipType::Calls),
            "Class without __init__ must keep the legacy behavior (call goes to class)"
        );
    }

    #[test]
    fn test_python_class_call_inherited_init() {
        let parent_class = mock_resolution_entity_with_kind(
            "Parent",
            "mod.Parent",
            None,
            "test/mod.py",
            EntityKind::Class,
        );
        let parent_init = mock_resolution_entity_with_kind(
            "__init__",
            "mod.Parent.__init__",
            Some("Parent"),
            "test/mod.py",
            EntityKind::Method,
        );
        let mut child_class = mock_resolution_entity_with_kind(
            "Child",
            "mod.Child",
            None,
            "test/mod.py",
            EntityKind::Class,
        );
        child_class
            .reference_intents
            .push(ReferenceIntent::Extends {
                parent: "Parent".to_string(),
                line: 1,
            });

        let mut caller = mock_resolution_entity_with_kind(
            "caller",
            "mod.caller",
            None,
            "test/mod.py",
            EntityKind::Function,
        );
        caller.reference_intents.push(ReferenceIntent::Call {
            method: "Child".to_string(),
            receiver: None,
            line: 20,
            arg_count: None,
        });

        let parent_init_uuid = parent_init.uuid;

        let mut entities = vec![parent_class, parent_init, child_class, caller];
        resolve_reference_intents(&mut entities);

        let caller_entity = entities.iter().find(|e| e.name == "caller").unwrap();
        assert!(
            caller_entity
                .relationships
                .contains(&(parent_init_uuid, RelationshipType::Calls)),
            "Child() with no own __init__ must redirect to Parent.__init__ via inheritance"
        );
    }

    #[test]
    fn test_function_named_like_class_not_redirected() {
        let foo_function = mock_resolution_entity_with_kind(
            "foo",
            "mod.foo",
            None,
            "test/mod.py",
            EntityKind::Function,
        );

        let mut caller = mock_resolution_entity_with_kind(
            "caller",
            "mod.caller",
            None,
            "test/mod.py",
            EntityKind::Function,
        );
        caller.reference_intents.push(ReferenceIntent::Call {
            method: "foo".to_string(),
            receiver: None,
            line: 5,
            arg_count: None,
        });

        let foo_function_uuid = foo_function.uuid;

        let mut entities = vec![foo_function, caller];
        resolve_reference_intents(&mut entities);

        let caller_entity = entities.iter().find(|e| e.name == "caller").unwrap();
        assert_eq!(caller_entity.relationships.len(), 1);
        assert_eq!(
            caller_entity.relationships[0],
            (foo_function_uuid, RelationshipType::Calls),
            "A function must NOT be redirected — only Class/Struct kinds trigger the redirect"
        );
    }

    // Regression: bug reported on LlamaFactory's webui/chatter.py.
    // `engine.chatter.load_model(args)` was failing to resolve when there were
    // two homonyms in different modules:
    //   - `llamafactory.webui.chatter.WebChatModel.load_model` (method)
    //   - `llamafactory.model.loader.load_model` (top-level function)
    // The receiver chain `engine.chatter` contains the segment `chatter` which
    // uniquely identifies the WebChatModel method via its FQN module path.
    #[test]
    fn test_call_disambiguation_by_receiver_chain_segments() {
        let web_chat_method = mock_resolution_entity_with_kind(
            "load_model",
            "llamafactory.webui.chatter.WebChatModel.load_model",
            Some("WebChatModel"),
            "src/llamafactory/webui/chatter.py",
            EntityKind::Method,
        );
        let loader_fn = mock_resolution_entity_with_kind(
            "load_model",
            "llamafactory.model.loader.load_model",
            None,
            "src/llamafactory/model/loader.py",
            EntityKind::Function,
        );

        let mut caller = mock_resolution_entity_with_kind(
            "build_chat_tab",
            "llamafactory.webui.components.chatbot.build_chat_tab",
            None,
            "src/llamafactory/webui/components/chatbot.py",
            EntityKind::Function,
        );
        caller.reference_intents.push(ReferenceIntent::Call {
            method: "load_model".to_string(),
            receiver: Some("engine.chatter".to_string()),
            line: 42,
            arg_count: Some(1),
        });

        let web_chat_method_uuid = web_chat_method.uuid;
        let loader_fn_uuid = loader_fn.uuid;

        let mut entities = vec![web_chat_method, loader_fn, caller];
        resolve_reference_intents(&mut entities);

        let caller_entity = entities
            .iter()
            .find(|e| e.name == "build_chat_tab")
            .unwrap();
        assert!(
            caller_entity
                .relationships
                .contains(&(web_chat_method_uuid, RelationshipType::Calls)),
            "engine.chatter.load_model(...) must resolve to WebChatModel.load_model"
        );
        assert!(
            !caller_entity
                .relationships
                .iter()
                .any(|(u, _)| *u == loader_fn_uuid),
            "must NOT misattribute to the module-level loader.load_model"
        );
    }

    // Two candidates whose FQNs both contain a single receiver-chain segment
    // (`chatter`) result in a tie and the resolver must NOT pick either one.
    #[test]
    fn test_call_disambiguation_ties_are_dropped() {
        let candidate_a = mock_resolution_entity_with_kind(
            "load_model",
            "mod.chatter.WebChatModel.load_model",
            Some("WebChatModel"),
            "src/a.py",
            EntityKind::Method,
        );
        let candidate_b = mock_resolution_entity_with_kind(
            "load_model",
            "mod.chatter.Other.load_model",
            Some("Other"),
            "src/b.py",
            EntityKind::Method,
        );

        let mut caller = mock_resolution_entity_with_kind(
            "caller",
            "mod.caller",
            None,
            "src/caller.py",
            EntityKind::Function,
        );
        caller.reference_intents.push(ReferenceIntent::Call {
            method: "load_model".to_string(),
            receiver: Some("engine.chatter".to_string()),
            line: 1,
            arg_count: Some(1),
        });

        let candidate_a_uuid = candidate_a.uuid;
        let candidate_b_uuid = candidate_b.uuid;

        let mut entities = vec![candidate_a, candidate_b, caller];
        resolve_reference_intents(&mut entities);

        let caller_entity = entities.iter().find(|e| e.name == "caller").unwrap();
        assert!(
            !caller_entity
                .relationships
                .iter()
                .any(|(u, _)| *u == candidate_a_uuid || *u == candidate_b_uuid),
            "tie on receiver-chain score must drop the call rather than guess"
        );
    }

    // Regression: bug reported on LlamaFactory's webui/chatter.py.
    // `super().__init__()` inside `WebChatModel.__init__` was being resolved to
    // the same-file `WebChatModel.__init__` (the enclosing function itself) via
    // the name_to_uuids fallback, instead of `ChatModel.__init__`.
    #[test]
    fn test_python_super_init_resolves_to_parent_class() {
        let parent_class = mock_resolution_entity_with_kind(
            "ChatModel",
            "llamafactory.chat.ChatModel",
            None,
            "src/llamafactory/chat/chat_model.py",
            EntityKind::PythonClass,
        );
        let parent_init = mock_resolution_entity_with_kind(
            "__init__",
            "llamafactory.chat.ChatModel.__init__",
            Some("ChatModel"),
            "src/llamafactory/chat/chat_model.py",
            EntityKind::Method,
        );
        let mut child_class = mock_resolution_entity_with_kind(
            "WebChatModel",
            "llamafactory.webui.chatter.WebChatModel",
            None,
            "src/llamafactory/webui/chatter.py",
            EntityKind::PythonClass,
        );
        child_class
            .reference_intents
            .push(ReferenceIntent::Extends {
                parent: "ChatModel".to_string(),
                line: 81,
            });

        let mut child_init = mock_resolution_entity_with_kind(
            "__init__",
            "llamafactory.webui.chatter.WebChatModel.__init__",
            Some("WebChatModel"),
            "src/llamafactory/webui/chatter.py",
            EntityKind::Method,
        );
        child_init.reference_intents.push(ReferenceIntent::Call {
            method: "__init__".to_string(),
            receiver: Some("super()".to_string()),
            line: 87,
            arg_count: Some(0),
        });
        child_init.reference_intents.push(ReferenceIntent::Call {
            method: "__init__".to_string(),
            receiver: Some("super()".to_string()),
            line: 93,
            arg_count: Some(1),
        });

        let parent_init_uuid = parent_init.uuid;
        let child_init_uuid = child_init.uuid;

        let mut entities = vec![parent_class, parent_init, child_class, child_init];
        resolve_reference_intents(&mut entities);

        let child_init_entity = entities.iter().find(|e| e.uuid == child_init_uuid).unwrap();

        let calls_to_parent = child_init_entity
            .relationships
            .iter()
            .filter(|(uuid, rel)| *uuid == parent_init_uuid && *rel == RelationshipType::Calls)
            .count();
        assert_eq!(
            calls_to_parent, 1,
            "super().__init__() must resolve to ChatModel.__init__ (deduplicated across the two call sites)"
        );

        let self_calls = child_init_entity
            .relationships
            .iter()
            .filter(|(uuid, rel)| *uuid == child_init_uuid && *rel == RelationshipType::Calls)
            .count();
        assert_eq!(
            self_calls, 0,
            "super().__init__() must NOT be misattributed to the enclosing WebChatModel.__init__"
        );
    }

    // Java/Kotlin style: `super.method()` produces receiver `"super"`.
    #[test]
    fn test_java_super_call_resolves_to_parent_class() {
        let parent_class = mock_resolution_entity_with_kind(
            "Base",
            "com.example.Base",
            None,
            "src/Base.java",
            EntityKind::Class,
        );
        let parent_method = mock_resolution_entity_with_kind(
            "init",
            "com.example.Base.init",
            Some("Base"),
            "src/Base.java",
            EntityKind::Method,
        );
        let mut child_class = mock_resolution_entity_with_kind(
            "Child",
            "com.example.Child",
            None,
            "src/Child.java",
            EntityKind::Class,
        );
        child_class
            .reference_intents
            .push(ReferenceIntent::Extends {
                parent: "Base".to_string(),
                line: 1,
            });

        let mut child_method = mock_resolution_entity_with_kind(
            "init",
            "com.example.Child.init",
            Some("Child"),
            "src/Child.java",
            EntityKind::Method,
        );
        child_method.reference_intents.push(ReferenceIntent::Call {
            method: "init".to_string(),
            receiver: Some("super".to_string()),
            line: 5,
            arg_count: Some(0),
        });

        let parent_method_uuid = parent_method.uuid;
        let child_method_uuid = child_method.uuid;

        let mut entities = vec![parent_class, parent_method, child_class, child_method];
        resolve_reference_intents(&mut entities);

        let child_method_entity = entities
            .iter()
            .find(|e| e.uuid == child_method_uuid)
            .unwrap();
        assert!(
            child_method_entity
                .relationships
                .contains(&(parent_method_uuid, RelationshipType::Calls)),
            "super.init() must resolve to Base.init"
        );
    }

    #[test]
    fn test_python_super_init_no_parent_returns_no_edge() {
        // If the parent class isn't indexed, super().__init__() should drop the
        // call (return None) rather than fall back to the same-file __init__.
        let mut child_class = mock_resolution_entity_with_kind(
            "Orphan",
            "mod.Orphan",
            None,
            "test/mod.py",
            EntityKind::PythonClass,
        );
        child_class
            .reference_intents
            .push(ReferenceIntent::Extends {
                parent: "UnindexedExternal".to_string(),
                line: 1,
            });

        let mut child_init = mock_resolution_entity_with_kind(
            "__init__",
            "mod.Orphan.__init__",
            Some("Orphan"),
            "test/mod.py",
            EntityKind::Method,
        );
        child_init.reference_intents.push(ReferenceIntent::Call {
            method: "__init__".to_string(),
            receiver: Some("super()".to_string()),
            line: 5,
            arg_count: Some(0),
        });

        let child_init_uuid = child_init.uuid;

        let mut entities = vec![child_class, child_init];
        resolve_reference_intents(&mut entities);

        let child_init_entity = entities.iter().find(|e| e.uuid == child_init_uuid).unwrap();
        let self_calls = child_init_entity
            .relationships
            .iter()
            .filter(|(uuid, rel)| *uuid == child_init_uuid && *rel == RelationshipType::Calls)
            .count();
        assert_eq!(
            self_calls, 0,
            "Unresolved super() must not be misattributed to self"
        );
    }

    #[test]
    fn test_python_class_call_with_existing_init_no_duplicate() {
        let foo_class = mock_resolution_entity_with_kind(
            "Foo",
            "mod.Foo",
            None,
            "test/mod.py",
            EntityKind::Class,
        );
        let foo_init = mock_resolution_entity_with_kind(
            "__init__",
            "mod.Foo.__init__",
            Some("Foo"),
            "test/mod.py",
            EntityKind::Method,
        );

        let mut caller = mock_resolution_entity_with_kind(
            "caller",
            "mod.caller",
            None,
            "test/mod.py",
            EntityKind::Function,
        );
        caller.reference_intents.push(ReferenceIntent::Call {
            method: "Foo".to_string(),
            receiver: None,
            line: 10,
            arg_count: None,
        });
        caller.reference_intents.push(ReferenceIntent::Call {
            method: "__init__".to_string(),
            receiver: Some("Foo".to_string()),
            line: 10,
            arg_count: None,
        });

        let foo_init_uuid = foo_init.uuid;

        let mut entities = vec![foo_class, foo_init, caller];
        resolve_reference_intents(&mut entities);

        let caller_entity = entities.iter().find(|e| e.name == "caller").unwrap();
        let init_calls = caller_entity
            .relationships
            .iter()
            .filter(|(uuid, rel)| *uuid == foo_init_uuid && *rel == RelationshipType::Calls)
            .count();
        assert_eq!(
            init_calls, 1,
            "There must be exactly one Calls edge to Foo.__init__ (parser + redirect deduplicated)"
        );
    }

    #[test]
    fn test_type_receiver_prefers_exact_class_over_suffix_homonym() {
        let http_impl = mock_resolution_entity_at(
            "fetchItems",
            "com.a.impl.HttpCatalogProvider.fetchItems",
            Some("HttpCatalogProvider"),
            "src/impl/HttpCatalogProvider.java",
        );
        let iface = mock_resolution_entity_at(
            "fetchItems",
            "com.z.api.CatalogProvider.fetchItems",
            Some("CatalogProvider"),
            "src/api/CatalogProvider.java",
        );
        let mut caller = mock_resolution_entity_at(
            "build",
            "com.web.ReportService.build",
            Some("ReportService"),
            "src/web/ReportService.java",
        );
        caller.reference_intents.push(ReferenceIntent::Call {
            method: "fetchItems".to_string(),
            receiver: Some("CatalogProvider".to_string()),
            line: 10,
            arg_count: None,
        });

        let iface_uuid = iface.uuid;
        let mut entities = vec![http_impl, iface, caller];
        resolve_reference_intents(&mut entities);

        let caller_entity = entities.iter().find(|e| e.name == "build").unwrap();
        assert_eq!(caller_entity.relationships.len(), 1);
        assert_eq!(
            caller_entity.relationships[0],
            (iface_uuid, RelationshipType::Calls),
            "Expected call to resolve to CatalogProvider.fetchItems, not HttpCatalogProvider.fetchItems"
        );
    }

    #[test]
    fn test_contains_on_segment_boundary() {
        let needle = "LookupService.listOptions";
        assert!(contains_on_segment_boundary(
            "com.x.LookupService.listOptions",
            needle
        ));
        assert!(contains_on_segment_boundary(
            "LookupService.listOptions",
            needle
        ));
        assert!(contains_on_segment_boundary(
            "crate::LookupService.listOptions",
            needle
        ));
        assert!(contains_on_segment_boundary(
            "pkg.LookupService.listOptions.<anonymous@3>.run",
            needle
        ));
        assert!(!contains_on_segment_boundary(
            "com.x.DefaultLookupService.listOptions",
            needle
        ));
        assert!(!contains_on_segment_boundary(
            "com.x.LookupService.listOptionsAsync",
            needle
        ));
        // A later whole-segment occurrence still counts.
        assert!(contains_on_segment_boundary(
            "a.DefaultLookupService.listOptions.LookupService.listOptions",
            needle
        ));
    }

    /// Mock a `Call` intent from `LookupController.options` through an
    /// untyped receiver (e.g. a field inherited from another file).
    fn lookup_caller(receiver: &str) -> ResolutionEntity {
        let mut caller = mock_resolution_entity_at(
            "options",
            "com.web.LookupController.options",
            Some("LookupController"),
            "src/web/LookupController.java",
        );
        caller.reference_intents.push(ReferenceIntent::Call {
            method: "listOptions".to_string(),
            receiver: Some(receiver.to_string()),
            line: 12,
            arg_count: None,
        });
        caller
    }

    fn lookup_method(class: &str) -> ResolutionEntity {
        mock_resolution_entity_at(
            "listOptions",
            &format!("com.service.{class}.listOptions"),
            Some(class),
            &format!("src/service/{class}.java"),
        )
    }

    #[test]
    fn test_untyped_receiver_never_binds_to_implementation_containing_interface_name() {
        // Issue #29: `DefaultLookupService` sorts before `LookupService`, so a
        // plain substring match on `LookupService.listOptions` picked the
        // implementation.
        let iface = lookup_method("LookupService");
        let implementation = lookup_method("DefaultLookupService");
        let iface_uuid = iface.uuid;
        let mut entities = vec![implementation, iface, lookup_caller("lookupService")];
        resolve_reference_intents(&mut entities);

        let caller = entities.iter().find(|e| e.name == "options").unwrap();
        assert_eq!(
            caller.relationships,
            vec![(iface_uuid, RelationshipType::Calls)],
            "lookupService.listOptions() must bind to LookupService.listOptions"
        );
    }

    #[test]
    fn test_untyped_receiver_with_this_prefix_binds_to_interface() {
        let iface = lookup_method("LookupService");
        let implementation = lookup_method("DefaultLookupService");
        let iface_uuid = iface.uuid;
        let mut entities = vec![implementation, iface, lookup_caller("this.lookupService")];
        resolve_reference_intents(&mut entities);

        let caller = entities.iter().find(|e| e.name == "options").unwrap();
        assert_eq!(
            caller.relationships,
            vec![(iface_uuid, RelationshipType::Calls)]
        );
    }

    #[test]
    fn test_untyped_receiver_keeps_unique_implementation_as_fallback() {
        // Without the interface in scope the only candidate still wins
        // through the homonym fallback: tightening the match costs no recall.
        let implementation = lookup_method("DefaultLookupService");
        let impl_uuid = implementation.uuid;
        let mut entities = vec![implementation, lookup_caller("lookupService")];
        resolve_reference_intents(&mut entities);

        let caller = entities.iter().find(|e| e.name == "options").unwrap();
        assert_eq!(
            caller.relationships,
            vec![(impl_uuid, RelationshipType::Calls)]
        );
    }

    #[test]
    fn test_untyped_receiver_ignores_method_name_prefix() {
        let mut async_variant = mock_resolution_entity_at(
            "listOptionsAsync",
            "com.service.LookupService.listOptionsAsync",
            Some("LookupService"),
            "src/service/LookupService.java",
        );
        async_variant.name = "listOptionsAsync".to_string();
        let mut entities = vec![async_variant, lookup_caller("lookupService")];
        resolve_reference_intents(&mut entities);

        let caller = entities.iter().find(|e| e.name == "options").unwrap();
        assert!(
            caller.relationships.is_empty(),
            "listOptions() must not bind to listOptionsAsync: {:?}",
            caller.relationships
        );
    }

    #[test]
    fn test_type_receiver_resolves_inherited_interface_method() {
        let base_method = mock_resolution_entity_at(
            "fetchItems",
            "com.api.BaseProvider.fetchItems",
            Some("BaseProvider"),
            "src/api/BaseProvider.java",
        );
        let other_method = mock_resolution_entity_at(
            "fetchItems",
            "com.api.OtherProvider.fetchItems",
            Some("OtherProvider"),
            "src/api/OtherProvider.java",
        );
        let mut child_iface = mock_resolution_entity_with_kind(
            "CatalogProvider",
            "com.api.CatalogProvider",
            None,
            "src/api/CatalogProvider.java",
            EntityKind::Interface,
        );
        child_iface
            .reference_intents
            .push(ReferenceIntent::Extends {
                parent: "BaseProvider".to_string(),
                line: 3,
            });
        let mut caller = mock_resolution_entity_at(
            "build",
            "com.web.ReportService.build",
            Some("ReportService"),
            "src/web/ReportService.java",
        );
        caller.reference_intents.push(ReferenceIntent::Call {
            method: "fetchItems".to_string(),
            receiver: Some("CatalogProvider".to_string()),
            line: 10,
            arg_count: None,
        });

        let base_method_uuid = base_method.uuid;
        let mut entities = vec![base_method, other_method, child_iface, caller];
        resolve_reference_intents(&mut entities);

        let caller_entity = entities.iter().find(|e| e.name == "build").unwrap();
        assert_eq!(caller_entity.relationships.len(), 1);
        assert_eq!(
            caller_entity.relationships[0],
            (base_method_uuid, RelationshipType::Calls),
            "Expected call to resolve to BaseProvider.fetchItems via extends hierarchy"
        );
    }
}
