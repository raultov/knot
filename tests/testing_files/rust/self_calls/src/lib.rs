//! `self.method()` call resolution (issue #36): methods reached only through
//! a `self` receiver, a `self.field` receiver or a call-chain receiver must
//! still get a CALLS edge from their caller.

pub mod other;

pub trait Sink {
    fn flush(&self) -> u32;
}

pub struct Repo;

impl Repo {
    pub fn save(&self) -> bool {
        true
    }
}

pub struct Builder;

impl Builder {
    pub fn finish(&self) -> u32 {
        0
    }
}

pub struct PriceAdjuster {
    repo: Repo,
    label: String,
}

impl PriceAdjuster {
    pub fn adjust(&self, sku: &str) -> u32 {
        self.compute_value(sku)
    }

    pub fn persist(&self) -> bool {
        self.repo.save()
    }

    pub fn rebuild(&self) -> u32 {
        self.builder().finish()
    }

    fn compute_value(&self, sku: &str) -> u32 {
        sku.len() as u32
    }

    fn builder(&self) -> Builder {
        Builder
    }

    /// `Iterator::collect` and `String::as_str` on receivers of unknown type
    /// must not bind to the user-defined homonyms below.
    pub fn lengths(&self, skus: &[&str]) -> Vec<usize> {
        let _ = self.label.as_str();
        skus.iter().map(|s| s.len()).collect()
    }
}

/// Homonym trap for the std `Iterator::collect`.
pub struct Pool;

impl Pool {
    pub fn collect(&self) -> Vec<u32> {
        Vec::new()
    }
}

/// Homonym trap for the std `String::as_str`.
pub struct Tier;

impl Tier {
    pub fn as_str(&self) -> &'static str {
        "tier"
    }
}

impl Sink for PriceAdjuster {
    fn flush(&self) -> u32 {
        self.compute_value("flush")
    }
}

/// Same-file homonym trap: `self.compute_value()` inside `PriceAdjuster`
/// must never resolve here.
pub struct DiscountAdjuster;

impl DiscountAdjuster {
    fn compute_value(&self, sku: &str) -> u32 {
        sku.len() as u32 / 2
    }
}
