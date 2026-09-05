use super::properties::GroovyPropertyDecl;
use super::utils::build_fqn;

use crate::models::{EntityKind, ParsedEntity};

/// Emits Groovy's compiler-generated property accessors as first-class
/// method entities, so name-based OVERRIDES linking can match a subtype
/// property against a supertype getter (see resolve/overrides.rs).
pub(super) fn synthesize_property_accessors(
    entities: &mut Vec<ParsedEntity>,
    package: &Option<String>,
    file_path: &str,
    repo_name: &str,
    prop_decls: &std::collections::HashMap<(String, String), GroovyPropertyDecl>,
) {
    use std::collections::{HashMap, HashSet};

    // Build declared method names per enclosing class.
    let mut declared: HashSet<(String, String)> = HashSet::new(); // (enclosing_class, method_name)
    let mut type_kind: HashMap<String, EntityKind> = HashMap::new();

    for e in entities.iter() {
        match e.kind {
            EntityKind::GroovyClass
            | EntityKind::GroovyInterface
            | EntityKind::GroovyTrait
            | EntityKind::GroovyEnum => {
                type_kind.insert(e.name.clone(), e.kind.clone());
            }
            EntityKind::GroovyMethod => {
                if let Some(ref cls) = e.enclosing_class {
                    declared.insert((cls.clone(), e.name.clone()));
                }
            }
            _ => {}
        }
    }

    let mut synthetic: Vec<ParsedEntity> = Vec::new();

    for e in entities.iter() {
        if e.kind != EntityKind::GroovyProperty {
            continue;
        }
        let Some(ref cls) = e.enclosing_class else {
            continue;
        };
        let Some(kind) = type_kind.get(cls.as_str()) else {
            continue;
        };

        // Interface fields are constants — no accessors generated.
        if *kind == EntityKind::GroovyInterface {
            continue;
        }

        let prop_name = &e.name;
        if prop_name.is_empty()
            || !prop_name
                .as_bytes()
                .first()
                .is_some_and(|b| b.is_ascii_alphabetic() || *b == b'_')
        {
            continue;
        }

        // Skip if the property name already starts with get/set/is (would collide).
        if (prop_name.starts_with("get")
            && prop_name.chars().nth(3).is_some_and(|c| c.is_uppercase()))
            || (prop_name.starts_with("set")
                && prop_name.chars().nth(3).is_some_and(|c| c.is_uppercase()))
            || (prop_name.starts_with("is")
                && prop_name.chars().nth(2).is_some_and(|c| c.is_uppercase()))
        {
            continue;
        }

        let cap = {
            let mut chars = prop_name.chars();
            let first = chars.next().unwrap().to_uppercase().to_string();
            let rest: String = chars.collect();
            format!("{first}{rest}")
        };

        let decl_info = prop_decls.get(&(cls.clone(), e.name.clone()));

        // Emit getter: `get{Cap}`
        let getter_name = format!("get{cap}");
        if !declared.contains(&(cls.clone(), getter_name.clone())) {
            synthetic.push(make_synthetic_accessor(
                &getter_name,
                e,
                package,
                file_path,
                repo_name,
                cls,
            ));
        }

        // Emit `is{Cap}` for boolean / Boolean properties
        if let Some(decl) = decl_info
            && let Some(ref dt) = decl.declared_type
            && (dt == "boolean" || dt == "Boolean")
        {
            let is_name = format!("is{cap}");
            if !declared.contains(&(cls.clone(), is_name.clone())) {
                synthetic.push(make_synthetic_accessor(
                    &is_name, e, package, file_path, repo_name, cls,
                ));
            }
        }

        // Emit setter: `set{Cap}` (suppressed for `final` properties and explicit declarations)
        let is_final = decl_info.is_some_and(|d| d.is_final);
        if !is_final {
            let setter_name = format!("set{cap}");
            if !declared.contains(&(cls.clone(), setter_name.clone())) {
                synthetic.push(make_synthetic_accessor(
                    &setter_name,
                    e,
                    package,
                    file_path,
                    repo_name,
                    cls,
                ));
            }
        }
    }

    entities.append(&mut synthetic);
}
#[expect(
    clippy::too_many_arguments,
    reason = "function is verbose but correct — extraction deferred"
)]
fn make_synthetic_accessor(
    name: &str,
    property: &ParsedEntity,
    package: &Option<String>,
    file_path: &str,
    repo_name: &str,
    enclosing_class: &str,
) -> ParsedEntity {
    let fqn = build_fqn(package, &Some(enclosing_class.to_string()), name);
    ParsedEntity::new(
        name,
        EntityKind::GroovyMethod,
        &fqn,
        Some("<synthetic Groovy property accessor>".to_string()),
        property.docstring.clone(),
        "groovy",
        file_path,
        property.start_line,
        property.end_line,
        Some(enclosing_class.to_string()),
        repo_name,
    )
}
