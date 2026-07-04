use std::collections::HashMap;
use uuid::Uuid;

use super::context::ResolutionContext;
use crate::models::{CallIntent, EntityKind};

pub(crate) fn disambiguate_overload(
    fqn_uuid: Uuid,
    intent: &CallIntent,
    ctx: &ResolutionContext,
    expected_enclosing_class: Option<&str>,
) -> Uuid {
    if let Some(ac) = intent.arg_count
        && let Some(ac_map) = ctx.uuid_to_arg_count
        && ac_map.get(&fqn_uuid) != Some(&ac)
        && let Some(uuids) = ctx.name_to_uuids.get(&intent.method)
    {
        if let Some(class_name) = expected_enclosing_class
            && let Some(fqn_map) = ctx.uuid_to_fqn
        {
            let dot_fqn = format!("{}.{}", class_name, intent.method);
            let colon_fqn = format!("{}::{}", class_name, intent.method);

            if let Some(&better) = uuids.iter().find(|&&u| {
                ac_map.get(&u) == Some(&ac)
                    && fqn_map
                        .get(&u)
                        .is_some_and(|fqn| *fqn == dot_fqn || *fqn == colon_fqn)
            }) {
                return better;
            }
        }

        if let Some(&better) = uuids.iter().find(|u| ac_map.get(u) == Some(&ac)) {
            return better;
        }
    }
    fqn_uuid
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

pub(crate) fn find_by_arg_count(
    uuids: &[Uuid],
    arg_count: usize,
    uuid_to_arg_count: &HashMap<Uuid, usize>,
) -> Option<Uuid> {
    let matches: Vec<&Uuid> = uuids
        .iter()
        .filter(|u| uuid_to_arg_count.get(u) == Some(&arg_count))
        .collect();
    if matches.len() == 1 {
        Some(*matches[0])
    } else {
        None
    }
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
pub(crate) fn redirect_class_call_to_constructor(uuid: Uuid, ctx: &ResolutionContext) -> Uuid {
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

    if let Some(init_uuid) = lookup_init_in_class_hierarchy(class_name, ctx) {
        return init_uuid;
    }

    if let Some(parents) = ctx.extends_map.get(class_name) {
        for parent in parents {
            if let Some(init_uuid) = lookup_init_in_class_hierarchy(parent, ctx) {
                return init_uuid;
            }
        }
    }

    uuid
}

/// Locate a class's `__init__` constructor by trying every FQN shape the parser
/// may have produced (`<ClassName>.__init__`, `<mod.ClassName>.__init__`, …).
fn lookup_init_in_class_hierarchy(class_name: &str, ctx: &ResolutionContext) -> Option<Uuid> {
    lookup_method_in_class_hierarchy(class_name, "__init__", ctx)
}

/// Locate `<class_name>.<method>` by trying every FQN shape the parser may
/// have produced. Falls back to suffix matching against the FQN map so that
/// short class names (e.g. `"ChatModel"`) resolve to fully-qualified
/// methods (e.g. `"llamafactory.chat.ChatModel.__init__"`).
fn lookup_method_in_class_hierarchy(
    class_name: &str,
    method: &str,
    ctx: &ResolutionContext,
) -> Option<Uuid> {
    if let Some(uuid) = lookup_fqn(class_name, method, ctx.fqn_to_uuid) {
        return Some(uuid);
    }

    if let Some(uuid_to_fqn) = ctx.uuid_to_fqn {
        for class_fqn in uuid_to_fqn.values() {
            let is_class_match = class_fqn == class_name
                || class_fqn.ends_with(&format!(".{}", class_name))
                || class_fqn.ends_with(&format!("::{}", class_name));
            if is_class_match
                && let Some(uuid) = lookup_fqn_by_fqn(class_fqn, method, ctx.fqn_to_uuid)
            {
                return Some(uuid);
            }
        }
    }

    None
}

/// Lookup `__init__` using a fully-qualified class path (handles `mod.Class.__init__`).
fn lookup_fqn_by_fqn(
    class_fqn: &str,
    method: &str,
    fqn_to_uuid: &HashMap<String, Uuid>,
) -> Option<Uuid> {
    let dot_fqn = format!("{}.{}", class_fqn, method);
    if let Some(&uuid) = fqn_to_uuid.get(&dot_fqn) {
        return Some(uuid);
    }
    let colon_fqn = format!("{}::{}", class_fqn, method);
    fqn_to_uuid.get(&colon_fqn).copied()
}

pub(crate) fn resolve_single_call_intent(
    intent: &CallIntent,
    caller_file_path: &str,
    caller_enclosing_class: Option<&str>,
    ctx: &ResolutionContext,
) -> Option<Uuid> {
    if let Some(receiver) = intent.receiver.as_deref()
        && is_super_receiver(receiver)
    {
        if let Some(enclosing_class) = caller_enclosing_class
            && let Some(parents) = ctx.extends_map.get(enclosing_class)
        {
            for parent in parents {
                if let Some(uuid) = lookup_method_in_class_hierarchy(parent, &intent.method, ctx) {
                    return Some(disambiguate_overload(uuid, intent, ctx, Some(parent)));
                }
            }
        }
        return None;
    }

    if (intent.receiver.is_none()
        || intent.receiver.as_deref() == Some("this")
        || intent.receiver.as_deref() == Some("self"))
        && let Some(enclosing_class) = caller_enclosing_class
    {
        if let Some(uuid) = lookup_fqn(enclosing_class, &intent.method, ctx.fqn_to_uuid) {
            return Some(disambiguate_overload(
                uuid,
                intent,
                ctx,
                Some(enclosing_class),
            ));
        }

        if intent.receiver.as_deref() == Some("self")
            && let Some(parents) = ctx.extends_map.get(enclosing_class)
        {
            for parent in parents {
                if let Some(uuid) = lookup_fqn(parent, &intent.method, ctx.fqn_to_uuid) {
                    return Some(uuid);
                }
            }
        }
    }

    if let Some(receiver) = &intent.receiver
        && receiver.chars().next().is_some_and(|c| c.is_uppercase())
        && receiver != "this"
        && let Some(uuid) = lookup_fqn(receiver, &intent.method, ctx.fqn_to_uuid)
    {
        return Some(disambiguate_overload(uuid, intent, ctx, Some(receiver)));
    }

    if let Some(receiver) = &intent.receiver {
        let receiver_class = if receiver.contains('.') {
            receiver
                .split('.')
                .next_back()
                .map(|s| s.trim())
                .unwrap_or(receiver)
        } else {
            receiver
        };

        if !receiver_class.is_empty() {
            if let Some(uuid) = lookup_fqn(receiver_class, &intent.method, ctx.fqn_to_uuid) {
                return Some(disambiguate_overload(
                    uuid,
                    intent,
                    ctx,
                    Some(receiver_class),
                ));
            }

            let mut chars = receiver_class.chars();
            let capitalized = if let Some(first) = chars.next() {
                first.to_uppercase().to_string() + chars.as_str()
            } else {
                receiver_class.to_string()
            };

            if let Some(uuid) = lookup_fqn(&capitalized, &intent.method, ctx.fqn_to_uuid) {
                return Some(disambiguate_overload(uuid, intent, ctx, Some(&capitalized)));
            }

            let method_dot = format!("{}.{}", receiver_class, intent.method);
            let capitalized_method_dot = format!("{}.{}", capitalized, intent.method);
            let method_colon = format!("{}::{}", receiver_class, intent.method);
            let capitalized_method_colon = format!("{}::{}", capitalized, intent.method);
            for (fqn, uuid) in ctx.fqn_to_uuid.iter() {
                if fqn.contains(&method_dot)
                    || fqn.contains(&capitalized_method_dot)
                    || fqn.contains(&method_colon)
                    || fqn.contains(&capitalized_method_colon)
                {
                    return Some(*uuid);
                }
            }
        }

        if let Some(uuids) = ctx.name_to_uuids.get(&intent.method) {
            if let Some(ac) = intent.arg_count
                && let Some(ac_map) = ctx.uuid_to_arg_count
            {
                if let Some(&same_file_ac_uuid) = uuids.iter().find(|&&u| {
                    ac_map.get(&u) == Some(&ac)
                        && ctx
                            .uuid_to_file
                            .get(&u)
                            .is_some_and(|f| f == caller_file_path)
                }) {
                    return Some(same_file_ac_uuid);
                }
                if let Some(u) = find_by_arg_count(uuids, ac, ac_map) {
                    return Some(u);
                }
            }
            if let Some(same_file_uuid) =
                find_entity_in_same_file(uuids, caller_file_path, ctx.uuid_to_file)
            {
                return Some(same_file_uuid);
            }
            if let Some(u) = disambiguate_by_receiver_chain(uuids, receiver, ctx) {
                return Some(u);
            }
            if uuids.len() == 1 {
                return uuids.first().copied();
            }
        }
    }

    if intent.receiver.is_none()
        && let Some(uuids) = ctx.name_to_uuids.get(&intent.method)
    {
        if let Some(ac) = intent.arg_count
            && let Some(ac_map) = ctx.uuid_to_arg_count
        {
            if let Some(&same_file_ac_uuid) = uuids.iter().find(|&&u| {
                ac_map.get(&u) == Some(&ac)
                    && ctx
                        .uuid_to_file
                        .get(&u)
                        .is_some_and(|f| f == caller_file_path)
            }) {
                return Some(same_file_ac_uuid);
            }
            if let Some(u) = find_by_arg_count(uuids, ac, ac_map) {
                return Some(u);
            }
        }
        if let Some(same_file_uuid) =
            find_entity_in_same_file(uuids, caller_file_path, ctx.uuid_to_file)
        {
            return Some(same_file_uuid);
        }
        if uuids.len() == 1 {
            return uuids.first().copied();
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::super::test_utils::*;
    use super::*;
    use crate::models::{EntityKind, ReferenceIntent, RelationshipType, ResolutionEntity};
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

    #[test]
    fn test_resolve_fallback_arg_count_disambiguation() {
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

        assert_eq!(entities[0].relationships.len(), 1);
        assert_eq!(entities[0].relationships[0].0, entities[1].uuid);
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
}
