//! Second module in the fixture crate.
//!
//! Adds an extra production function plus its own `#[cfg(test)] mod tests`
//! block. This lets the E2E verify that the inline `tests` module is scoped
//! per-file (each file owns its own `tests::` namespace), which prevents
//! collisions like `test_module_repo::tests::test_helper` clashing across
//! files.

use crate::is_supported;

/// Top-level helper that delegates to the crate root's `is_supported`.
/// This is the production-side caller for this file.
pub fn helper_is_supported(ext: &str) -> bool {
    is_supported(ext)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_helper_is_supported_rs() {
        assert!(helper_is_supported("rs"));
    }

    #[test]
    fn test_helper_calls_is_supported() {
        // Calls both helper_is_supported (same-file) AND is_supported (cross-module).
        // Both edges should appear with this fix.
        assert!(helper_is_supported("ts"));
        assert!(is_supported("java"));
    }
}
