//! Fixture crate for the rust_test_module E2E regression suite.
//!
//! Reproduces the bug where `#[cfg(test)] mod tests { ... }` blocks fail
//! to surface their entities and reference edges in the indexed graph.

pub mod helpers;

/// Production-grade entry point. Used by `production_caller` and by every
/// test inside `mod tests`. The graph must report all of them as callers.
pub fn is_supported(ext: &str) -> bool {
    matches!(ext, "rs" | "ts" | "java")
}

/// Production caller — guaranteed to be detected by the current indexer
/// (this is the only caller that knot detects today).
pub fn production_caller(ext: &str) -> bool {
    is_supported(ext)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_supported_rs() {
        assert!(is_supported("rs"));
    }

    #[test]
    fn test_is_supported_ts() {
        assert!(is_supported("ts"));
    }

    #[test]
    fn test_is_supported_rejects_txt() {
        assert!(!is_supported("txt"));
    }

    mod nested {
        use super::*;

        #[test]
        fn test_deeply_nested_is_supported() {
            assert!(is_supported("java"));
        }
    }
}
