//! The `max_results` bound contract: single source of truth for the limit
//! the search enforces and the MCP/CLI advertise.

/// Default result count when the caller does not ask for one.
/// Must equal the `default` advertised by the MCP schema
/// ([`crate::mcp_tools::search_hybrid_context`]).
pub const DEFAULT_MAX_RESULTS: usize = 5;

/// Hard ceiling for `max_results`. Must equal the `maximum` advertised by the
/// MCP schema — the drift guard test in `mcp_tools::search_hybrid_context`
/// pins the two together. There is no cursor or pagination: past this bound,
/// callers narrow the search with `kinds` / `path` / `repo_name` or refine
/// the query instead of raising the limit.
pub const MAX_RESULTS_CEILING: usize = 100;

/// A caller-requested result count resolved against the advertised bound.
///
/// Resolved values are always clamped into `1..=MAX_RESULTS_CEILING`:
/// values within the bound pass through unchanged, zero/negative requests
/// floor at 1 (a result count of 0 would mean "return nothing at all"),
/// and requests above the ceiling clamp to it — the caller is told via
/// [`ResolvedLimit::notice`], never silently.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResolvedLimit {
    /// The enforced value to use for the search.
    pub value: usize,
    /// What the caller originally asked for.
    pub requested: usize,
}

impl ResolvedLimit {
    /// Whether the request exceeded the advertised ceiling (or floored).
    pub fn was_clamped(&self) -> bool {
        self.requested != self.value
    }

    /// Caller-facing note when the request had to be adjusted; `None` when
    /// the request was served as asked.
    pub fn notice(&self) -> Option<String> {
        if !self.was_clamped() {
            return None;
        }
        if self.requested == 0 {
            return Some("> Note: `max_results` was floored from 0 to the minimum of 1.\n".into());
        }
        Some(format!(
            "> Note: `max_results` was clamped from {requested} to the advertised maximum of {MAX_RESULTS_CEILING}. \
This tool has no pagination — narrow the search with `kinds` / `path` / `repo_name`, or refine the query.\n",
            requested = self.requested
        ))
    }
}

/// Single source of truth for the `max_results` bound — used by the shared
/// search core, by the MCP tool layer and by the CLI so the enforced value
/// can never drift from the advertised schema.
pub fn resolve_max_results(requested: usize) -> ResolvedLimit {
    ResolvedLimit {
        value: requested.clamp(1, MAX_RESULTS_CEILING),
        requested,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_max_results_clamps_above_ceiling() {
        let resolved = resolve_max_results(10_000);
        assert_eq!(resolved.value, MAX_RESULTS_CEILING);
        assert_eq!(resolved.requested, 10_000);
        assert!(resolved.was_clamped());
    }

    #[test]
    fn resolve_max_results_preserves_values_within_bound() {
        for n in [1, DEFAULT_MAX_RESULTS, 20, MAX_RESULTS_CEILING] {
            let resolved = resolve_max_results(n);
            assert_eq!(resolved.value, n);
            assert!(!resolved.was_clamped());
            assert!(resolved.notice().is_none());
        }
    }

    #[test]
    fn resolve_max_results_floors_at_one() {
        // A result count of 0 would mean "return nothing at all"; floored.
        let resolved = resolve_max_results(0);
        assert_eq!(resolved.value, 1);
        assert!(resolved.was_clamped());
    }

    #[test]
    fn clamp_notice_states_bound_and_refine_rule() {
        let notice = resolve_max_results(500).notice().expect("clamped");
        assert!(notice.contains("500"));
        assert!(notice.contains(MAX_RESULTS_CEILING.to_string().as_str()));
        assert!(notice.contains("kinds"));
        assert!(notice.contains("path"));
        assert!(notice.contains("no pagination"));
    }
}
