use super::models::{Correlation, CorrelationKind, RepoAsset, Signal};

const STALE_DAYS: i64 = 30;
const HUB_DEPENDENT_THRESHOLD: usize = 3;

pub fn detect_signals(
    assets: &[RepoAsset],
    correlations: &[Correlation],
    dep_edges: &[(String, String)],
) -> Vec<Signal> {
    let mut signals = Vec::new();
    let median_entities = median_i64(assets.iter().map(|a| a.entity_count).collect());
    let call_counts: Vec<i64> = correlations
        .iter()
        .filter(|c| c.kind == CorrelationKind::Calls)
        .map(|c| c.strength)
        .collect();
    let median_calls = median_i64(call_counts);

    for asset in assets {
        match asset.role {
            super::models::RepoRole::Hub => {
                signals.push(Signal {
                    kind: "hub_repo".to_string(),
                    repo: asset.name.clone(),
                    detail: format!("{} dependents (portfolio hub)", asset.dependent_count),
                });
            }
            super::models::RepoRole::Leaf => {
                signals.push(Signal {
                    kind: "leaf_repo".to_string(),
                    repo: asset.name.clone(),
                    detail: "depends on others but nothing depends on it".to_string(),
                });
            }
            super::models::RepoRole::Isolated => {
                signals.push(Signal {
                    kind: "isolated_repo".to_string(),
                    repo: asset.name.clone(),
                    detail: "no DEPENDS_ON edges in or out".to_string(),
                });
            }
            super::models::RepoRole::Balanced => {}
        }

        if is_stale(&asset.indexed_at) {
            signals.push(Signal {
                kind: "stale_index".to_string(),
                repo: asset.name.clone(),
                detail: format!("indexed_at older than {STALE_DAYS} days or missing"),
            });
        }

        if asset.dependent_count >= HUB_DEPENDENT_THRESHOLD
            && (is_stale(&asset.indexed_at) || asset.entity_count < median_entities)
        {
            signals.push(Signal {
                kind: "index_library_first".to_string(),
                repo: asset.name.clone(),
                detail: "shared dependency with stale or small index — re-index before clients"
                    .to_string(),
            });
        }
    }

    for corr in correlations {
        if corr.kind == CorrelationKind::Calls && median_calls > 0 && corr.strength > median_calls {
            signals.push(Signal {
                kind: "high_coupling".to_string(),
                repo: format!("{} -> {}", corr.from, corr.to),
                detail: format!(
                    "{} cross-repo calls (above median {})",
                    corr.strength, median_calls
                ),
            });
        }
    }

    if dep_edges.is_empty() && assets.len() > 1 {
        signals.push(Signal {
            kind: "no_structural_links".to_string(),
            repo: "*".to_string(),
            detail: "multiple repos indexed but no DEPENDS_ON edges — index build files or use KNOT_DEPENDENCIES".to_string(),
        });
    }

    signals
}

fn is_stale(indexed_at: &str) -> bool {
    if indexed_at.is_empty() {
        return true;
    }
    let Some(date_str) = indexed_at.get(0..10) else {
        return true;
    };
    let Some(days) = days_since_ymd(date_str) else {
        return true;
    };
    days > STALE_DAYS
}

/// Days between `YYYY-MM-DD` and today (UTC date from system clock).
fn days_since_ymd(date_str: &str) -> Option<i64> {
    let mut parts = date_str.split('-');
    let y: i32 = parts.next()?.parse().ok()?;
    let m: u32 = parts.next()?.parse().ok()?;
    let d: u32 = parts.next()?.parse().ok()?;
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    let indexed = days_from_civil(y, m, d);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_secs() as i64;
    let today = now / 86_400;
    Some(today - indexed)
}

/// Approximate days since Unix epoch for a civil date (good enough for stale checks).
fn days_from_civil(y: i32, m: u32, d: u32) -> i64 {
    let y = y as i64;
    let m = m as i64;
    let d = d as i64;
    let y = if m <= 2 { y - 1 } else { y };
    let era = y / 400;
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn median_i64(mut values: Vec<i64>) -> i64 {
    if values.is_empty() {
        return 0;
    }
    values.sort_unstable();
    values[values.len() / 2]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::portfolio::correlate::classify_role;
    use crate::portfolio::models::RepoRole;

    fn sample_asset(name: &str, deps: usize, dependents: usize) -> RepoAsset {
        RepoAsset {
            name: name.to_string(),
            entity_count: 100,
            file_count: 10,
            build_system: "maven".to_string(),
            primary_language: "java".to_string(),
            group_id: String::new(),
            artifact_id: String::new(),
            version: String::new(),
            indexed_at: "2020-01-01T00:00:00Z".to_string(),
            role: classify_role(deps, dependents),
            weight_pct: 25.0,
            dependency_count: deps,
            dependent_count: dependents,
            description: String::new(),
            readme_excerpt: String::new(),
            identity: String::new(),
        }
    }

    #[test]
    fn test_detect_hub_signal() {
        let assets = vec![sample_asset("core", 0, 4)];
        let signals = detect_signals(&assets, &[], &[]);
        assert!(signals.iter().any(|s| s.kind == "hub_repo"));
    }

    #[test]
    fn test_detect_stale_index() {
        let assets = vec![sample_asset("old", 0, 0)];
        let signals = detect_signals(&assets, &[], &[]);
        assert!(signals.iter().any(|s| s.kind == "stale_index"));
    }
}
