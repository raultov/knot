//! C# language support (`.cs`).
//!
//! Entity extraction is query-driven (`queries/csharp.scm`) via the shared
//! extractor; this module provides the C#-specific logic the generic
//! pipeline cannot express:
//!
//! - [`capture`] — capture routing for grammar gaps (`field_declaration`
//!   has no `name` field, `record_declaration` covers both record flavours,
//!   indexer/operator declarations have no name at all).
//! - [`fqn`] — FQN construction: file-scoped namespace pre-pass plus an
//!   ancestor walk over block namespaces and containing types
//!   (`<namespace>.<Outer>.<Nested>.<member>`, plan §3.2).
//! - [`refs`] — reference intents: call/collection with receiver-type
//!   substitution, the `base_list` EXTENDS/IMPLEMENTS heuristic (plan §3.3),
//!   attribute references, and type references.
//!
//! FQN shape: `MyApp.Services.UserService.GetUserAsync` — dot-joined,
//! namespace first, then containing types outermost-first, then the member.

mod capture;
mod fqn;
#[cfg(test)]
mod tests;

pub(crate) mod refs;

pub(crate) use capture::handle_csharp_capture;
pub(crate) use fqn::{build_csharp_fqn_prefix, extract_file_scoped_namespace};
pub(crate) use refs::{
    collect_all_reference_intents_csharp, extract_attribute_references,
    extract_class_inheritance_csharp, extract_reference_intents_csharp,
    extract_type_references_csharp,
};
