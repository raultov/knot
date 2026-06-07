//! Rust language support for entity extraction and reference intent collection.
//!
//! Submodules group the pipeline by responsibility:
//!
//! | Submodule | Responsibility |
//! |-----------|----------------|
//! | [`fqn`]   | FQN qualification + inline `mod` context extraction |
//! | [`capture`] | Tree-sitter capture name → entity mapping |
//! | [`impls`] | Impl block analysis, method reclassification, trait `IMPLEMENTS` edges |
//! | [`macros`] | Macro invocation tracking |
//! | [`types`] | Type reference collection (signatures, fields, `use` statements, macros) |
//! | [`calls`] | Function / method call collection (including macro body scans) |
//! | [`utils`] | Helpers shared across submodules |

mod calls;
mod capture;
mod fqn;
mod impls;
mod macros;
mod types;
mod utils;

pub(crate) use calls::collect_rust_call_references;
pub(crate) use capture::handle_rust_capture;
pub(crate) use fqn::qualify_rust_fqns;
pub(crate) use impls::{
    collect_rust_trait_implementations, extract_impl_self_type, reclassify_methods_in_impl_blocks,
};
pub(crate) use macros::collect_rust_macro_references;
pub use types::collect_rust_type_references;

#[cfg(test)]
mod tests;
