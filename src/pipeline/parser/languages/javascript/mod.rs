//! JavaScript language support (`.js`, `.mjs`, `.cjs`, `.jsx`).
//!
//! Entity extraction is query-driven via the shared extractor; this module
//! provides the JavaScript-specific reference-intent logic. TypeScript
//! ([`super::typescript`]) reuses several of these extractors, so the shared
//! API is re-exported `pub(crate)` from here.
//!
//! Submodules group the pipeline by responsibility:
//!
//! | Submodule       | Responsibility |
//! |-----------------|----------------|
//! | [`refs`]        | Call / new-expression / JSX / callback reference intents, enum-static member usages, reserved keywords |
//! | [`imports`]     | Module-system intents: `import` clauses, `require` destructuring, require paths, `module.exports` targets |
//! | [`jsx`]         | JSX component invocations and `id` / `className` attribute extraction for cross-language search |
//! | [`inheritance`] | Class `extends` heritage extraction |
//! | [`dom_css`]     | `dom.element_id` / `css.class_name` capture handling |

mod dom_css;
mod imports;
mod inheritance;
mod jsx;
mod refs;

#[cfg(test)]
mod tests;

pub(crate) use dom_css::handle_dom_css_capture;
pub(crate) use imports::{
    collect_import_intents_javascript, extract_require_module_path, scan_module_exports_target,
};
pub(crate) use inheritance::extract_class_inheritance_js;
pub(crate) use jsx::{extract_jsx_component_invocation, extract_jsx_html_attributes};
pub(crate) use refs::{
    collect_all_reference_intents_javascript, extract_enum_usages_javascript,
    extract_reference_intents_javascript, extract_single_call_intent_javascript,
};

// Test-only consumers (unit-test helpers); cfg(test) keeps the lib profile
// free of unused-import warnings.
#[cfg(test)]
pub(crate) use jsx::extract_jsx_attributes;
#[cfg(test)]
pub(crate) use refs::{extract_callback_arguments, is_reserved_keyword};
